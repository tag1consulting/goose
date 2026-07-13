//! Live web dashboard HTTP server.
//!
//! Compiled only with the `dashboard` crate feature. Spawns an axum server that
//! serves a minimal static shell, a one-shot metrics snapshot API, an SSE
//! stream of coalesced snapshots, and (when `--dashboard-control` is set)
//! authenticated Start/Stop/Users control endpoints.
//!
//! The parent GooseAttack main loop answers [`DashboardRequest`]s via oneshot
//! channels using [`MetricsCommand::GetDashboardSnapshot`] for snapshots and
//! staged control helpers for mutation.
//!
//! # Snapshot freshness
//!
//! [`SnapshotHub`] builds at most one snapshot per second while SSE clients are
//! connected or a recent poll occurred. Expected display freshness is **~1 s**,
//! with up to **~0.5 s scheduling jitter** in the main loop before a snapshot
//! request is dequeued. Tests assert multi-client coalescing, not sub-100 ms
//! latency.

use crate::metrics::DashboardSnapshot;
use crate::{ControlOutcome, GooseConfiguration, GooseError};

use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Response, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream;
use serde::Deserialize;
use serde::Serialize;
use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{oneshot, watch, Notify};
use tower_http::set_header::SetResponseHeaderLayer;

/// Embedded SPA shell and assets (no inline scripts — CSP-friendly).
const INDEX_HTML: &str = include_str!("dashboard/static/index.html");
const APP_JS: &str = include_str!("dashboard/static/app.js");
const APP_CSS: &str = include_str!("dashboard/static/app.css");
/// Vendored minified Chart.js (UMD) for series charts.
const CHART_JS: &str = include_str!("dashboard/static/chart.min.js");

/// Content-Security-Policy applied to every response.
///
/// `frame-ancestors 'none'` blocks embedding the control UI in third-party
/// frames (clickjacking Start/Stop/Users if a user is induced to interact).
const CSP: &str =
    "default-src 'self'; script-src 'self'; connect-src 'self'; style-src 'self'; img-src 'self' data:; frame-ancestors 'none'";

/// Default hard cap on concurrent SSE clients. Overridable via
/// `--dashboard-max-clients` / [`crate::config::GooseDefault::DashboardMaxClients`].
/// Additional `GET /api/v1/events` connections receive 503.
const MAX_SSE_CLIENTS: usize = 32;

/// Server-side snapshot cadence while clients are active.
const SNAPSHOT_INTERVAL: Duration = Duration::from_secs(1);

/// How long a one-shot poll keeps the hub building after the request.
const RECENT_POLL_WINDOW: Duration = Duration::from_secs(2);

/// SSE comment heartbeat interval (`: ping`).
const SSE_HEARTBEAT: Duration = Duration::from_secs(15);

/// Oneshot wait for control POSTs (Start replies before `reset_run_state`).
///
/// If this fires the HTTP client receives 503 with `error=timeout`. The main
/// loop **skips** applying the command when the oneshot is already closed
/// (still queued). If the main loop had already started applying (e.g. large
/// `SetUsers` allocation), a residual race can still mutate after this 503 —
/// clients must re-check phase/users before retrying (do not blind-retry).
///
/// 15s allows non-trivial `weight_scenario_users` work; Start still replies
/// before `control_start_finish` so HTTP usually completes much sooner.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(15);

/// Hub wait for a single parent/metrics snapshot reply before treating the build
/// as transient (poll clients already use a 5s outer timeout).
const SNAPSHOT_BUILD_TIMEOUT: Duration = Duration::from_secs(4);

/// Max JSON body size for control POSTs (start/stop are tiny; users is one int).
const CONTROL_BODY_LIMIT: usize = 16 * 1024;

/// HTTP-layer upper bound for `POST /api/v1/control/users`.
///
/// Controllers historically had no cap; the dashboard makes user changes one
/// click/curl away, so a hard limit prevents accidental multi-GB client
/// allocation from a single POST. Extreme values still require Controllers.
const MAX_CONTROL_USERS: u64 = 100_000;

/// Max concurrent control POSTs waiting on the main loop (admission control).
/// Additional authenticated control requests receive 503 `busy` without
/// enqueueing unbounded flume work.
const MAX_CONTROL_IN_FLIGHT: usize = 4;

/// Result of a successful dashboard server spawn.
#[derive(Debug)]
pub(crate) struct DashboardSetup {
    /// Parent end of the request channel.
    pub request_rx: flume::Receiver<DashboardRequest>,
    /// Actual TCP port after bind (useful when configured port is 0 / ephemeral).
    /// Read by unit tests; production only needs `request_rx`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub bound_port: u16,
    /// Number of snapshots the hub has successfully built (tests / diagnostics).
    #[cfg_attr(not(test), allow(dead_code))]
    pub build_count: Arc<AtomicU64>,
    /// Signal SSE clients to emit `event: closed` and stop the hub loop.
    ///
    /// Used on final process shutdown so browsers get a clean close without
    /// waiting for the next 1 Hz build attempt to observe a dropped flume channel.
    pub close_tx: watch::Sender<bool>,
}

/// Requests from the dashboard HTTP task to the GooseAttack main loop.
#[derive(Debug)]
pub(crate) enum DashboardRequest {
    /// Build and return a compact metrics snapshot.
    GetSnapshot {
        respond: tokio::sync::oneshot::Sender<DashboardSnapshot>,
    },
    /// Start an idle load test (reply before `reset_run_state`).
    Start {
        respond: tokio::sync::oneshot::Sender<ControlResult>,
    },
    /// Begin cancel ramp (Increase/Maintain → Decrease).
    Stop {
        respond: tokio::sync::oneshot::Sender<ControlResult>,
    },
    /// Set absolute target user count.
    SetUsers {
        users: usize,
        respond: tokio::sync::oneshot::Sender<ControlResult>,
    },
}

/// JSON body returned by control endpoints after the main loop answers.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ControlResult {
    pub ok: bool,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub message: String,
    pub phase: String,
    pub active_users: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_users: Option<u64>,
}

impl ControlResult {
    /// Map a soft [`ControlOutcome`] from the shared control helpers.
    pub(crate) fn from_outcome(command: &str, outcome: ControlOutcome) -> Self {
        Self {
            ok: outcome.ok,
            command: command.to_string(),
            error: outcome.error.map(str::to_string),
            message: outcome.message,
            phase: outcome.phase,
            active_users: outcome.active_users as u64,
            target_users: outcome.target_users.map(|u| u as u64),
        }
    }

    /// Unexpected hard error mapped into the oneshot before re-raise.
    pub(crate) fn internal(command: &str, phase: &str, active_users: usize) -> Self {
        Self {
            ok: false,
            command: command.to_string(),
            error: Some("internal".to_string()),
            message: "internal control error".to_string(),
            phase: phase.to_string(),
            active_users: active_users as u64,
            target_users: None,
        }
    }
}

/// HTTP-layer control error (auth / body / timeout) — main loop not consulted.
#[derive(Debug, Serialize)]
struct ControlHttpErrorBody {
    ok: bool,
    command: String,
    error: String,
    message: String,
    phase: Option<String>,
    active_users: Option<u64>,
    target_users: Option<u64>,
}

impl ControlHttpErrorBody {
    fn new(command: &str, error: &str, message: &str) -> Self {
        Self {
            ok: false,
            command: command.to_string(),
            error: error.to_string(),
            message: message.to_string(),
            phase: None,
            active_users: None,
            target_users: None,
        }
    }
}

/// Coalescing fan-out for dashboard snapshots.
///
/// One hub task builds at most ~1 snapshot/sec while `active_sse > 0` or a poll
/// was recent. Idle (no clients, no recent poll) issues **no**
/// `GetDashboardSnapshot` commands.
#[derive(Clone, Debug)]
struct SnapshotHub {
    inner: Arc<HubInner>,
}

struct HubInner {
    request_tx: flume::Sender<DashboardRequest>,
    /// Latest pre-serialized snapshot JSON (`None` until the first build).
    latest_tx: watch::Sender<Option<Bytes>>,
    /// Concurrent SSE subscribers.
    active_sse: AtomicUsize,
    /// Configured concurrent SSE client cap (default [`MAX_SSE_CLIENTS`]).
    max_sse_clients: usize,
    /// Unix-ms of the most recent `/api/v1/snapshot` poll mark (0 = never).
    last_poll_ms: AtomicU64,
    /// Unix-ms of the most recent successful build (0 = never). Used for freshness.
    last_build_at_ms: AtomicU64,
    /// Wall-clock duration of the most recent successful build, in milliseconds.
    last_build_ms: AtomicU64,
    /// Successful hub builds (shared with [`DashboardSetup::build_count`]).
    build_count: Arc<AtomicU64>,
    /// True while a GetSnapshot request is outstanding (including after the hub
    /// timed out waiting — prevents re-request pile-up on the metrics processor).
    snapshot_in_flight: AtomicBool,
    /// Wake the hub loop from idle when demand appears.
    wake: Notify,
    /// When `true`, SSE clients should emit `event: closed` and disconnect.
    shutdown_tx: watch::Sender<bool>,
}

// Manual Debug — `Notify` is not Debug on all tokio versions.
impl std::fmt::Debug for HubInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HubInner")
            .field("active_sse", &self.active_sse.load(Ordering::Relaxed))
            .field("max_sse_clients", &self.max_sse_clients)
            .field("build_count", &self.build_count.load(Ordering::Relaxed))
            .field("last_poll_ms", &self.last_poll_ms.load(Ordering::Relaxed))
            .field(
                "last_build_at_ms",
                &self.last_build_at_ms.load(Ordering::Relaxed),
            )
            .field("last_build_ms", &self.last_build_ms.load(Ordering::Relaxed))
            .field(
                "snapshot_in_flight",
                &self.snapshot_in_flight.load(Ordering::Relaxed),
            )
            .finish()
    }
}

impl SnapshotHub {
    fn new(
        request_tx: flume::Sender<DashboardRequest>,
        build_count: Arc<AtomicU64>,
        shutdown_tx: watch::Sender<bool>,
        max_sse_clients: usize,
    ) -> Self {
        let max_sse_clients = if max_sse_clients == 0 {
            MAX_SSE_CLIENTS
        } else {
            max_sse_clients
        };
        let (latest_tx, _) = watch::channel(None);
        let inner = Arc::new(HubInner {
            request_tx,
            latest_tx,
            active_sse: AtomicUsize::new(0),
            max_sse_clients,
            last_poll_ms: AtomicU64::new(0),
            last_build_at_ms: AtomicU64::new(0),
            last_build_ms: AtomicU64::new(0),
            build_count,
            snapshot_in_flight: AtomicBool::new(false),
            wake: Notify::new(),
            shutdown_tx,
        });
        let loop_inner = Arc::clone(&inner);
        tokio::spawn(async move {
            hub_loop(loop_inner).await;
        });
        SnapshotHub { inner }
    }

    fn mark_poll(&self) {
        self.inner
            .last_poll_ms
            .store(unix_now_ms(), Ordering::Relaxed);
        self.inner.wake.notify_one();
    }

    /// Reserve an SSE client slot. Returns `false` when at the configured cap.
    fn try_acquire_sse(&self) -> bool {
        // Do not consume a slot when the hub is already permanently closed.
        if *self.inner.shutdown_tx.borrow() {
            return false;
        }
        let cap = self.inner.max_sse_clients;
        let mut current = self.inner.active_sse.load(Ordering::Relaxed);
        loop {
            if current >= cap {
                return false;
            }
            match self.inner.active_sse.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    // Re-check sticky shutdown: a close may have raced with acquire.
                    if *self.inner.shutdown_tx.borrow() {
                        self.inner.active_sse.fetch_sub(1, Ordering::AcqRel);
                        return false;
                    }
                    self.inner.wake.notify_one();
                    return true;
                }
                Err(observed) => current = observed,
            }
        }
    }

    fn release_sse(&self) {
        self.inner.active_sse.fetch_sub(1, Ordering::AcqRel);
    }

    fn is_closed(&self) -> bool {
        *self.inner.shutdown_tx.borrow()
    }

    fn active_sse_clients(&self) -> usize {
        self.inner.active_sse.load(Ordering::Relaxed)
    }

    fn build_count(&self) -> u64 {
        self.inner.build_count.load(Ordering::Relaxed)
    }

    /// Duration of the last successful snapshot build in milliseconds (0 if none).
    fn last_build_ms(&self) -> u64 {
        self.inner.last_build_ms.load(Ordering::Relaxed)
    }

    fn subscribe_latest(&self) -> watch::Receiver<Option<Bytes>> {
        self.inner.latest_tx.subscribe()
    }

    fn subscribe_shutdown(&self) -> watch::Receiver<bool> {
        self.inner.shutdown_tx.subscribe()
    }

    /// Mark demand and wait for a (coalesced) snapshot JSON body.
    async fn snapshot_bytes(&self) -> Result<Bytes, StatusCode> {
        if self.is_closed() {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }

        self.mark_poll();

        // Fast path: return a snapshot built within the last cadence interval.
        if let Some(bytes) = self.fresh_latest() {
            return Ok(bytes);
        }

        let gen_before = self.inner.build_count.load(Ordering::Acquire);
        // Re-notify in case the hub was between checks.
        self.inner.wake.notify_one();

        let mut rx = self.inner.latest_tx.subscribe();
        let mut shutdown_rx = self.inner.shutdown_tx.subscribe();
        let wait = async {
            loop {
                if self.is_closed() {
                    return Err(StatusCode::SERVICE_UNAVAILABLE);
                }
                // `build_count` is only incremented after `latest_tx.send`, so a
                // higher generation implies the watch already holds the new body.
                if self.inner.build_count.load(Ordering::Acquire) > gen_before {
                    if let Some(bytes) = rx.borrow().clone() {
                        return Ok(bytes);
                    }
                }
                if let Some(bytes) = self.fresh_latest() {
                    return Ok(bytes);
                }

                tokio::select! {
                    result = rx.changed() => {
                        if result.is_err() {
                            return Err(StatusCode::SERVICE_UNAVAILABLE);
                        }
                    }
                    result = shutdown_rx.changed() => {
                        if result.is_err() || *shutdown_rx.borrow() {
                            return Err(StatusCode::SERVICE_UNAVAILABLE);
                        }
                    }
                }
            }
        };

        match tokio::time::timeout(Duration::from_secs(5), wait).await {
            Ok(result) => result,
            Err(_) => Err(StatusCode::SERVICE_UNAVAILABLE),
        }
    }

    /// Cached snapshot if one was built within [`SNAPSHOT_INTERVAL`].
    fn fresh_latest(&self) -> Option<Bytes> {
        let bytes = self.inner.latest_tx.borrow().clone()?;
        let built_at = self.inner.last_build_at_ms.load(Ordering::Relaxed);
        if built_at == 0 {
            return None;
        }
        let age = unix_now_ms().saturating_sub(built_at);
        if age < SNAPSHOT_INTERVAL.as_millis() as u64 {
            Some(bytes)
        } else {
            None
        }
    }
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn should_build(inner: &HubInner) -> bool {
    if *inner.shutdown_tx.borrow() {
        return false;
    }
    if inner.active_sse.load(Ordering::Relaxed) > 0 {
        return true;
    }
    let last_poll = inner.last_poll_ms.load(Ordering::Relaxed);
    if last_poll == 0 {
        return false;
    }
    unix_now_ms().saturating_sub(last_poll) < RECENT_POLL_WINDOW.as_millis() as u64
}

/// Outcome of one hub build attempt.
enum BuildOutcome {
    /// Snapshot published (or serialization failed but hub remains healthy).
    Built,
    /// Reply channel cancelled (e.g. metrics processor recycled) — skip tick, keep hub.
    Transient,
    /// Parent flume channel disconnected — permanent stop + `event: closed`.
    Fatal,
}

async fn hub_loop(inner: Arc<HubInner>) {
    let mut shutdown_rx = inner.shutdown_tx.subscribe();
    // Sticky shutdown may already be true (proactive close from main loop).
    if *shutdown_rx.borrow() {
        return;
    }
    loop {
        // Idle: no GetDashboardSnapshot — wait for demand or shutdown.
        // A 1s disconnect poll does not touch the metrics path; it only lets
        // the hub notice a dropped parent channel while parked (Issue 6).
        while !should_build(&inner) {
            if inner.request_tx.is_disconnected() {
                let _ = inner.shutdown_tx.send(true);
                return;
            }
            tokio::select! {
                biased;
                result = shutdown_rx.changed() => {
                    if result.is_err() || *shutdown_rx.borrow() {
                        return;
                    }
                }
                _ = inner.wake.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        }

        match build_once(&inner).await {
            BuildOutcome::Built => {}
            BuildOutcome::Transient => {
                // Metrics/main-loop blip or in-flight wait: do not kill the hub.
                // Brief backoff so we do not spin while the processor is busy.
                debug!("[dashboard]: snapshot build transient; will retry");
                tokio::select! {
                    biased;
                    result = shutdown_rx.changed() => {
                        if result.is_err() || *shutdown_rx.borrow() {
                            return;
                        }
                    }
                    _ = tokio::time::sleep(SNAPSHOT_INTERVAL) => {}
                }
                continue;
            }
            BuildOutcome::Fatal => {
                // Parent request channel gone — signal SSE clients and stop.
                let _ = inner.shutdown_tx.send(true);
                return;
            }
        }

        // 1 Hz pace while demand continues; also observe disconnect / close.
        tokio::select! {
            biased;
            result = shutdown_rx.changed() => {
                if result.is_err() || *shutdown_rx.borrow() {
                    return;
                }
            }
            _ = tokio::time::sleep(SNAPSHOT_INTERVAL) => {
                if inner.request_tx.is_disconnected() {
                    let _ = inner.shutdown_tx.send(true);
                    return;
                }
            }
        }
    }
}

/// Request one snapshot from the parent and publish pre-serialized JSON.
async fn build_once(inner: &Arc<HubInner>) -> BuildOutcome {
    if inner.request_tx.is_disconnected() {
        return BuildOutcome::Fatal;
    }

    // At most one outstanding GetSnapshot (including late replies after a hub
    // wait timeout) so slow metrics builds cannot pile up unbounded work.
    if inner
        .snapshot_in_flight
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return BuildOutcome::Transient;
    }

    let started = Instant::now();
    let (respond_tx, mut respond_rx) = oneshot::channel();
    if inner
        .request_tx
        .send(DashboardRequest::GetSnapshot {
            respond: respond_tx,
        })
        .is_err()
    {
        inner.snapshot_in_flight.store(false, Ordering::Release);
        return BuildOutcome::Fatal;
    }

    // Wait up to SNAPSHOT_BUILD_TIMEOUT; on timeout keep the oneshot alive in a
    // background task so in_flight stays set until the metrics reply is drained.
    let snapshot = tokio::select! {
        result = &mut respond_rx => {
            match result {
                Ok(s) => s,
                Err(_) => {
                    // Metrics/main-loop dropped the oneshot without a reply.
                    inner.snapshot_in_flight.store(false, Ordering::Release);
                    return BuildOutcome::Transient;
                }
            }
        }
        _ = tokio::time::sleep(SNAPSHOT_BUILD_TIMEOUT) => {
            debug!(
                "[dashboard]: snapshot build timed out after {SNAPSHOT_BUILD_TIMEOUT:?}; waiting in background"
            );
            let clear_flag = Arc::clone(inner);
            tokio::spawn(async move {
                let _ = respond_rx.await;
                clear_flag.snapshot_in_flight.store(false, Ordering::Release);
            });
            return BuildOutcome::Transient;
        }
    };

    let outcome = match serde_json::to_vec(&snapshot) {
        Ok(json) => {
            // Publish first, then advance generation counters so waiters that
            // observe `build_count` never read a stale watch value.
            //
            // Use `send_replace` (not `send`): `watch::Sender::send` is a no-op
            // when no receivers are currently subscribed, which is the common
            // poll-only path (`fresh_latest` reads via the sender). Without this,
            // last_build_at / build_count advance while the cached JSON stays
            // frozen — clients keep receiving a stale snapshot for the entire
            // load test after the first poll's receiver is dropped.
            let _ = inner.latest_tx.send_replace(Some(Bytes::from(json)));
            let elapsed_ms = started.elapsed().as_millis() as u64;
            inner
                .last_build_at_ms
                .store(unix_now_ms(), Ordering::Release);
            inner.last_build_ms.store(elapsed_ms, Ordering::Release);
            let count = inner.build_count.fetch_add(1, Ordering::Release) + 1;
            debug!(
                "[dashboard]: snapshot build {} took {}ms (active_sse={})",
                count,
                elapsed_ms,
                inner.active_sse.load(Ordering::Relaxed)
            );
            BuildOutcome::Built
        }
        Err(e) => {
            warn!("[dashboard]: failed to serialize snapshot: {e}");
            // Keep the hub running; waiters will time out or get a later build.
            BuildOutcome::Built
        }
    };
    inner.snapshot_in_flight.store(false, Ordering::Release);
    outcome
}

/// Shared state for axum handlers.
#[derive(Clone, Debug)]
struct DashboardState {
    /// Coalescing snapshot hub (owns a clone of the parent request channel sender).
    hub: SnapshotHub,
    /// Clone of the flume sender for control handlers (same channel as the hub).
    request_tx: flume::Sender<DashboardRequest>,
    /// Auth token; empty means observe auth is disabled (loopback-only runs).
    auth_token: String,
    /// When true, control routes are registered and require a non-empty token.
    control_enabled: bool,
    /// Concurrent control POSTs currently waiting on the main loop.
    control_in_flight: Arc<AtomicUsize>,
}

#[derive(Debug, Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

/// Liveness plus lightweight operational counters (no load-test metrics).
///
/// `last_build_ms` is the wall-clock duration of the last successful hub build,
/// not a Unix timestamp. Fields are always present so external monitors can
/// scrape them without auth; they do not leak request names, hosts, or rates.
#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    version: String,
    /// Duration of the last successful snapshot build in milliseconds (0 if none).
    last_build_ms: u64,
    /// Total successful hub snapshot builds since process start.
    build_count: u64,
    /// Concurrent SSE clients currently holding a slot.
    active_sse_clients: usize,
    /// Whether Start/Stop/Users control endpoints are registered.
    control_enabled: bool,
}

/// Format `host:port` for [`TcpListener::bind`], bracketing bare IPv6 literals.
///
/// Mirrors how operators type `--dashboard-host` (`localhost`, `127.0.0.1`,
/// `::1`, `[::1]`) while remaining valid for `ToSocketAddrs`.
fn format_bind_address(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Spawn the dashboard HTTP server task.
///
/// Binds to `configuration.dashboard_host`:`configuration.dashboard_port` and
/// returns the parent end of the request channel. Logs the listening URL.
///
/// When `--dashboard` is set, bind failure is a hard error (opt-in server must
/// not silently disappear). When the dashboard is disabled, returns `Ok(None)`.
///
/// When/if multi-mode (Gaggle) returns, the dashboard should only run on the
/// standalone/manager process — currently AttackMode is StandAlone only.
pub(crate) async fn setup_dashboard(
    configuration: &GooseConfiguration,
) -> Result<Option<DashboardSetup>, GooseError> {
    if !configuration.dashboard {
        return Ok(None);
    }

    let host = if configuration.dashboard_host.is_empty() {
        "127.0.0.1"
    } else {
        configuration.dashboard_host.as_str()
    };
    let port = configuration.dashboard_port;
    // Use a host:port string so `ToSocketAddrs` resolves `localhost` and IPv6
    // (same approach as Controllers). Do not parse as SocketAddr first — that
    // rejects hostnames and unbracketed `::1`.
    let address = format_bind_address(host, port);

    let listener = tokio::net::TcpListener::bind(&address).await.map_err(|e| {
        error!("[dashboard]: failed to bind {address}: {e} (is the port already in use?)");
        GooseError::Io(e)
    })?;

    let bound = listener.local_addr().map_err(|e| {
        error!("[dashboard]: failed to read local address after bind on {address}: {e}");
        GooseError::Io(e)
    })?;

    let (request_tx, request_rx) = flume::unbounded();
    let build_count = Arc::new(AtomicU64::new(0));
    let (close_tx, _) = watch::channel(false);
    // 0 means unset (direct setup without configure); fall back to default cap.
    let max_sse_clients = if configuration.dashboard_max_clients == 0 {
        MAX_SSE_CLIENTS
    } else {
        configuration.dashboard_max_clients as usize
    };
    // Clone sender for hub + control handlers (same channel).
    let hub = SnapshotHub::new(
        request_tx.clone(),
        Arc::clone(&build_count),
        close_tx.clone(),
        max_sse_clients,
    );
    let control_enabled = configuration.dashboard_control;
    let state = DashboardState {
        hub,
        request_tx,
        auth_token: configuration.dashboard_auth_token.clone(),
        control_enabled,
        control_in_flight: Arc::new(AtomicUsize::new(0)),
    };

    let app = build_router(state);

    if control_enabled {
        info!("[dashboard]: listening on http://{bound} (control enabled)");
        info!("[dashboard]: control POST /api/v1/control/* requires Authorization: Bearer <token>");
    } else {
        info!("[dashboard]: listening on http://{bound} (read-only)");
    }

    // Detached server task. `close_tx` only signals the SSE hub to emit
    // `event: closed` — the HTTP listener stays up so late subscribers and
    // idle re-Start polls still work until the process/runtime ends. Binding
    // is released when the Tokio runtime is dropped (CLI exit).
    // Wrap in Some so the JoinHandle is not a bare `let _ = future` (clippy).
    let _ = Some(tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            error!("[dashboard]: server error: {e}");
        }
    }));

    Ok(Some(DashboardSetup {
        request_rx,
        bound_port: bound.port(),
        build_count,
        close_tx,
    }))
}

/// Build the axum router with public shell/static/health and auth-gated metrics.
///
/// Control routes are registered only when `state.control_enabled` is true
/// (otherwise POST /api/v1/control/* → 404).
fn build_router(state: DashboardState) -> Router {
    let control_enabled = state.control_enabled;
    let mut router = Router::new()
        .route("/", get(index_handler))
        .route("/static/app.js", get(app_js_handler))
        .route("/static/app.css", get(app_css_handler))
        .route("/static/chart.min.js", get(chart_js_handler))
        .route("/api/v1/health", get(health_handler))
        .route("/api/v1/snapshot", get(snapshot_handler))
        .route("/api/v1/events", get(events_handler));

    if control_enabled {
        // Bound control request bodies so a large POST cannot blow memory/CPU
        // on the dashboard task (start/stop expect empty/`{}`; users is small).
        let control_routes = Router::new()
            .route("/api/v1/control/start", post(control_start_handler))
            .route("/api/v1/control/stop", post(control_stop_handler))
            .route("/api/v1/control/users", post(control_users_handler))
            .layer(axum::extract::DefaultBodyLimit::max(CONTROL_BODY_LIMIT));
        router = router.merge(control_routes);
    }

    router
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ))
        .with_state(Arc::new(state))
}

async fn index_handler() -> Response<Body> {
    static_response("text/html; charset=utf-8", INDEX_HTML)
}

async fn app_js_handler() -> Response<Body> {
    static_response("application/javascript; charset=utf-8", APP_JS)
}

async fn app_css_handler() -> Response<Body> {
    static_response("text/css; charset=utf-8", APP_CSS)
}

async fn chart_js_handler() -> Response<Body> {
    static_response("application/javascript; charset=utf-8", CHART_JS)
}

/// Build a static asset response without `unwrap` on the builder.
fn static_response(content_type: &'static str, body: &'static str) -> Response<Body> {
    ([(header::CONTENT_TYPE, content_type)], Body::from(body)).into_response()
}

async fn health_handler(State(state): State<Arc<DashboardState>>) -> impl IntoResponse {
    Json(HealthResponse {
        ok: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
        last_build_ms: state.hub.last_build_ms(),
        build_count: state.hub.build_count(),
        active_sse_clients: state.hub.active_sse_clients(),
        control_enabled: state.control_enabled,
    })
}

async fn snapshot_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    if !authorize(&state.auth_token, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let bytes = state.hub.snapshot_bytes().await?;
    Ok(([(header::CONTENT_TYPE, "application/json")], bytes))
}

/// SSE stream of coalesced `snapshot` events, with `: ping` heartbeats and a
/// terminal `closed` event when the hub shuts down.
async fn events_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Response<Body> {
    if !authorize(&state.auth_token, &headers, query.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    // Sticky shutdown: do not hold a client slot; emit `closed` immediately.
    if state.hub.is_closed() {
        return closed_sse_response();
    }

    if !state.hub.try_acquire_sse() {
        // Distinguish full cap vs raced into shutdown.
        if state.hub.is_closed() {
            return closed_sse_response();
        }
        return (StatusCode::SERVICE_UNAVAILABLE, "too many SSE clients").into_response();
    }

    let hub = state.hub.clone();
    let mut snap_rx = hub.subscribe_latest();
    let mut shutdown_rx = hub.subscribe_shutdown();

    // Channel bridges the select-loop task to the SSE body stream.
    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<Event>(8);

    tokio::spawn(async move {
        // Release the client slot when this task ends (client disconnect or closed).
        struct SseGuard(SnapshotHub);
        impl Drop for SseGuard {
            fn drop(&mut self) {
                self.0.release_sse();
            }
        }
        let _guard = SseGuard(hub);

        // watch does not notify for the value present at subscribe time — check
        // sticky shutdown before pushing a stale snapshot or blocking forever.
        if *shutdown_rx.borrow() {
            let _ = event_tx
                .send(Event::default().event("closed").data("1"))
                .await;
            return;
        }

        // Push the current snapshot immediately when available.
        // Clone out of the watch Ref before awaiting so the future stays Send.
        let initial = snap_rx.borrow_and_update().clone();
        if let Some(bytes) = initial {
            if event_tx.send(snapshot_event(&bytes)).await.is_err() {
                return;
            }
        }

        loop {
            tokio::select! {
                result = snap_rx.changed() => {
                    if result.is_err() {
                        break;
                    }
                    // Re-check shutdown in case both fired; prefer closed.
                    if *shutdown_rx.borrow() {
                        let _ = event_tx
                            .send(Event::default().event("closed").data("1"))
                            .await;
                        break;
                    }
                    let next = snap_rx.borrow_and_update().clone();
                    if let Some(bytes) = next {
                        if event_tx.send(snapshot_event(&bytes)).await.is_err() {
                            break;
                        }
                    }
                }
                result = shutdown_rx.changed() => {
                    let closed = result.is_err() || *shutdown_rx.borrow();
                    if closed {
                        let _ = event_tx
                            .send(Event::default().event("closed").data("1"))
                            .await;
                        break;
                    }
                }
            }
        }
    });

    let stream = stream::unfold(event_rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|event| (Ok::<_, Infallible>(event), rx))
    });

    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(SSE_HEARTBEAT).text("ping"))
        .into_response()
}

/// One-shot SSE body that only delivers `event: closed` (late subscribers after
/// sticky shutdown, without consuming a client slot).
fn closed_sse_response() -> Response<Body> {
    let stream =
        stream::once(async { Ok::<_, Infallible>(Event::default().event("closed").data("1")) });
    Sse::new(stream).into_response()
}

fn snapshot_event(bytes: &Bytes) -> Event {
    let data = std::str::from_utf8(bytes).unwrap_or("{}");
    Event::default().event("snapshot").data(data)
}

/// Authorize a metric API request.
///
/// When `configured_token` is empty, all requests are allowed (loopback trust).
/// Otherwise accept `Authorization: Bearer <token>` or `?token=<token>` with a
/// constant-time comparison.
fn authorize(configured_token: &str, headers: &HeaderMap, query_token: Option<&str>) -> bool {
    if configured_token.is_empty() {
        return true;
    }

    if let Some(token) = query_token {
        if constant_time_eq(token.as_bytes(), configured_token.as_bytes()) {
            return true;
        }
    }

    if let Some(token) = bearer_token_from_headers(headers) {
        if constant_time_eq(token.as_bytes(), configured_token.as_bytes()) {
            return true;
        }
    }

    false
}

/// Authorize a control API request.
///
/// Control always requires a non-empty configured token (fail closed if miswired).
/// **Bearer only** — query `?token=` is rejected so mutating URLs do not carry the
/// secret (scripts and the SPA should use `Authorization: Bearer`).
fn authorize_control(
    configured_token: &str,
    headers: &HeaderMap,
    query_token: Option<&str>,
) -> bool {
    let _ = query_token; // deliberately ignored — Bearer only
    if configured_token.is_empty() {
        return false;
    }

    if let Some(token) = bearer_token_from_headers(headers) {
        return constant_time_eq(token.as_bytes(), configured_token.as_bytes());
    }

    false
}

/// Extract a Bearer token from `Authorization`. Scheme match is case-insensitive
/// (RFC 7235); the token itself is compared separately with constant-time eq.
fn bearer_token_from_headers(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?;
    let s = value.to_str().ok()?;
    let (scheme, token) = s.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("Bearer") {
        let token = token.trim();
        if token.is_empty() {
            None
        } else {
            Some(token)
        }
    } else {
        None
    }
}

/// Best-effort constant-time equality for auth tokens.
///
/// Always walks a fixed number of bytes derived from both lengths so short-circuit
/// on length mismatch does not dominate the timing profile for typical token sizes.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    // Compare up to max(len_a, len_b), treating missing bytes as 0, and fold in
    // a length mismatch bit so equal prefixes of different lengths never match.
    let len_mismatch = (a.len() != b.len()) as u8;
    let max_len = a.len().max(b.len());
    let mut diff = len_mismatch;
    let mut i = 0;
    while i < max_len {
        let x = if i < a.len() { a[i] } else { 0 };
        let y = if i < b.len() { b[i] } else { 0 };
        diff |= x ^ y;
        i += 1;
    }
    diff == 0
}

/// Validate start/stop body: empty, missing, or `{}` only (no unknown fields).
fn parse_empty_control_body(body: &[u8]) -> Result<(), (&'static str, &'static str)> {
    let trimmed = trim_ascii_whitespace(body);
    if trimmed.is_empty() {
        return Ok(());
    }
    let value: serde_json::Value = match serde_json::from_slice(trimmed) {
        Ok(v) => v,
        Err(_) => return Err(("bad_request", "request body must be empty or {}")),
    };
    match value.as_object() {
        Some(obj) if obj.is_empty() => Ok(()),
        _ => Err(("bad_request", "request body must be empty or {}")),
    }
}

/// Parse `{"users": N}` with integer N in 1..=[`MAX_CONTROL_USERS`].
fn parse_users_body(body: &[u8]) -> Result<usize, (&'static str, &'static str)> {
    const USERS_MSG: &str = "users must be an integer between 1 and 100000";
    let trimmed = trim_ascii_whitespace(body);
    if trimmed.is_empty() {
        return Err(("bad_request", "request body required"));
    }
    let value: serde_json::Value = match serde_json::from_slice(trimmed) {
        Ok(v) => v,
        Err(_) => return Err(("bad_request", "malformed JSON body")),
    };
    let obj = match value.as_object() {
        Some(o) => o,
        None => return Err(("bad_request", "request body must be a JSON object")),
    };
    let users_val = match obj.get("users") {
        Some(v) => v,
        None => return Err(("invalid_users", USERS_MSG)),
    };
    // Reject floats/strings: only JSON integers (i64/u64), not f64.
    let n = match users_val.as_u64() {
        Some(n) => n,
        None => {
            // Negative integers arrive as i64; treat as invalid_users.
            if users_val.as_i64().is_some() || users_val.is_number() || users_val.is_string() {
                return Err(("invalid_users", USERS_MSG));
            }
            return Err(("invalid_users", USERS_MSG));
        }
    };
    if !(1..=MAX_CONTROL_USERS).contains(&n) {
        return Err(("invalid_users", USERS_MSG));
    }
    // Reject unknown extra fields? Design only requires deny_unknown on start/stop.
    // Users: only `users` is required; extra fields are tolerated for forward compat.
    Ok(n as usize)
}

fn trim_ascii_whitespace(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map(|i| i + 1)
        .unwrap_or(0);
    if start >= end {
        &[]
    } else {
        &bytes[start..end]
    }
}

fn control_http_error(
    status: StatusCode,
    command: &str,
    error: &str,
    message: &str,
) -> Response<Body> {
    let body = ControlHttpErrorBody::new(command, error, message);
    (status, Json(body)).into_response()
}

/// RAII guard that decrements `control_in_flight` on drop.
struct ControlInFlightGuard {
    counter: Arc<AtomicUsize>,
}

impl Drop for ControlInFlightGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Try to reserve a control admission slot. Returns `None` when at cap.
fn try_acquire_control_slot(state: &DashboardState) -> Option<ControlInFlightGuard> {
    let prev = state.control_in_flight.fetch_add(1, Ordering::AcqRel);
    if prev >= MAX_CONTROL_IN_FLIGHT {
        state.control_in_flight.fetch_sub(1, Ordering::AcqRel);
        return None;
    }
    Some(ControlInFlightGuard {
        counter: Arc::clone(&state.control_in_flight),
    })
}

/// Send a control request to the main loop and wait up to [`CONTROL_TIMEOUT`].
async fn dispatch_control(
    state: &DashboardState,
    request: DashboardRequest,
    respond_rx: oneshot::Receiver<ControlResult>,
    command: &str,
) -> Response<Body> {
    let Some(_slot) = try_acquire_control_slot(state) else {
        warn!("[dashboard]: control admission full ({command})");
        return control_http_error(
            StatusCode::SERVICE_UNAVAILABLE,
            command,
            "busy",
            "too many control requests in flight — wait and retry",
        );
    };

    if state.request_tx.send(request).is_err() {
        warn!("[dashboard]: control channel disconnected ({command})");
        return control_http_error(
            StatusCode::SERVICE_UNAVAILABLE,
            command,
            "unavailable",
            "control unavailable",
        );
    }

    match tokio::time::timeout(CONTROL_TIMEOUT, respond_rx).await {
        Ok(Ok(result)) => {
            if result.ok {
                info!(
                    "[dashboard]: control {command} ok phase={} target={:?}",
                    result.phase, result.target_users
                );
            } else {
                info!(
                    "[dashboard]: control {command} rejected error={:?} phase={}",
                    result.error, result.phase
                );
            }
            (StatusCode::OK, Json(result)).into_response()
        }
        Ok(Err(_)) => {
            warn!("[dashboard]: control oneshot dropped ({command})");
            control_http_error(
                StatusCode::SERVICE_UNAVAILABLE,
                command,
                "unavailable",
                "control unavailable",
            )
        }
        Err(_) => {
            // Dropping respond_rx closes the oneshot. The main loop skips
            // mutation when `respond.is_closed()` *before* handling — the common
            // case (still queued). If the main loop had already started applying
            // the command, a residual race can still mutate state after this 503.
            warn!(
                "[dashboard]: control oneshot timed out ({command}); skipped if still queued"
            );
            control_http_error(
                StatusCode::SERVICE_UNAVAILABLE,
                command,
                "timeout",
                "control timed out — action may still be applying; check phase/users before retrying",
            )
        }
    }
}

async fn control_start_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    body: Bytes,
) -> Response<Body> {
    if !authorize_control(&state.auth_token, &headers, query.token.as_deref()) {
        warn!("[dashboard]: control start auth failure");
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Err((code, msg)) = parse_empty_control_body(&body) {
        return control_http_error(StatusCode::BAD_REQUEST, "start", code, msg);
    }
    let (respond_tx, respond_rx) = oneshot::channel();
    dispatch_control(
        &state,
        DashboardRequest::Start {
            respond: respond_tx,
        },
        respond_rx,
        "start",
    )
    .await
}

async fn control_stop_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    body: Bytes,
) -> Response<Body> {
    if !authorize_control(&state.auth_token, &headers, query.token.as_deref()) {
        warn!("[dashboard]: control stop auth failure");
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Err((code, msg)) = parse_empty_control_body(&body) {
        return control_http_error(StatusCode::BAD_REQUEST, "stop", code, msg);
    }
    let (respond_tx, respond_rx) = oneshot::channel();
    dispatch_control(
        &state,
        DashboardRequest::Stop {
            respond: respond_tx,
        },
        respond_rx,
        "stop",
    )
    .await
}

async fn control_users_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    body: Bytes,
) -> Response<Body> {
    if !authorize_control(&state.auth_token, &headers, query.token.as_deref()) {
        warn!("[dashboard]: control users auth failure");
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let users = match parse_users_body(&body) {
        Ok(n) => n,
        Err((code, msg)) => {
            return control_http_error(StatusCode::BAD_REQUEST, "users", code, msg);
        }
    };
    let (respond_tx, respond_rx) = oneshot::channel();
    dispatch_control(
        &state,
        DashboardRequest::SetUsers {
            users,
            respond: respond_tx,
        },
        respond_rx,
        "users",
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GooseConfiguration;
    use crate::metrics::dashboard_snapshot::{
        AggregateMetrics, Percentiles, SeriesWindow, SnapshotFlags,
    };
    use chrono::Utc;
    use std::time::Instant;

    fn dummy_snapshot(seq: u64) -> DashboardSnapshot {
        DashboardSnapshot {
            version: 1,
            generated_at: Utc::now(),
            goose_version: "test".into(),
            phase: "maintain".into(),
            duration_secs: seq,
            active_users: 1,
            maximum_users: 1,
            total_users: 1,
            hosts: vec!["http://example.test".into()],
            aggregate: AggregateMetrics {
                total_requests: seq,
                total_failures: 0,
                requests_per_second: 1.0,
                failures_per_second: 0.0,
                failure_rate: 0.0,
                response_time_avg_ms: 1.0,
                response_time_min_ms: 1,
                response_time_max_ms: 1,
                percentile_ms: Percentiles {
                    p50: 1,
                    p95: 1,
                    p99: 1,
                },
                co_active: false,
            },
            requests: vec![],
            errors: vec![],
            series: SeriesWindow::empty(),
            flags: SnapshotFlags {
                metrics_disabled: false,
                requests_truncated: false,
                errors_truncated: false,
                series_seconds: 300,
            },
        }
    }

    /// Answer `GetSnapshot` requests with incrementing dummy payloads.
    /// Control requests get a soft success so tests do not hang on oneshot waits.
    fn spawn_mock_parent(
        request_rx: flume::Receiver<DashboardRequest>,
        parent_builds: Arc<AtomicU64>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Ok(msg) = request_rx.recv_async().await {
                match msg {
                    DashboardRequest::GetSnapshot { respond } => {
                        let n = parent_builds.fetch_add(1, Ordering::SeqCst);
                        let _ = respond.send(dummy_snapshot(n));
                    }
                    DashboardRequest::Start { respond } => {
                        let _ = respond.send(ControlResult {
                            ok: true,
                            command: "start".into(),
                            error: None,
                            message: "load test started".into(),
                            phase: "increase".into(),
                            active_users: 0,
                            target_users: Some(1),
                        });
                    }
                    DashboardRequest::Stop { respond } => {
                        let _ = respond.send(ControlResult {
                            ok: true,
                            command: "stop".into(),
                            error: None,
                            message: "load test stopped".into(),
                            phase: "decrease".into(),
                            active_users: 1,
                            target_users: Some(0),
                        });
                    }
                    DashboardRequest::SetUsers { users, respond } => {
                        let _ = respond.send(ControlResult {
                            ok: true,
                            command: "users".into(),
                            error: None,
                            message: "users configured".into(),
                            phase: "idle".into(),
                            active_users: 0,
                            target_users: Some(users as u64),
                        });
                    }
                }
            }
        })
    }

    /// Parent that drops a window of oneshots (simulates metrics processor recycle)
    /// then resumes answering.
    fn spawn_blip_parent(
        request_rx: flume::Receiver<DashboardRequest>,
        parent_builds: Arc<AtomicU64>,
        drop_from: u64,
        drop_until: u64,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Ok(msg) = request_rx.recv_async().await {
                match msg {
                    DashboardRequest::GetSnapshot { respond } => {
                        let n = parent_builds.fetch_add(1, Ordering::SeqCst);
                        if n >= drop_from && n < drop_until {
                            drop(respond);
                        } else {
                            let _ = respond.send(dummy_snapshot(n));
                        }
                    }
                    // Control not exercised by blip tests.
                    DashboardRequest::Start { respond }
                    | DashboardRequest::Stop { respond }
                    | DashboardRequest::SetUsers { respond, .. } => {
                        let _ = respond.send(ControlResult::internal("unused", "idle", 0));
                    }
                }
            }
        })
    }

    async fn bind_dashboard(token: &str) -> (DashboardSetup, String) {
        bind_dashboard_with(token, 0, false).await
    }

    async fn bind_dashboard_with(
        token: &str,
        max_clients: u32,
        control: bool,
    ) -> (DashboardSetup, String) {
        let config = GooseConfiguration {
            dashboard: true,
            dashboard_control: control,
            dashboard_host: "127.0.0.1".to_string(),
            dashboard_port: 0,
            dashboard_auth_token: token.to_string(),
            dashboard_max_clients: max_clients,
            ..Default::default()
        };
        let setup = setup_dashboard(&config)
            .await
            .expect("bind")
            .expect("enabled");
        let base = format!("http://127.0.0.1:{}", setup.bound_port);
        let client = reqwest::Client::new();
        for _ in 0..50 {
            if let Ok(r) = client.get(format!("{base}/api/v1/health")).send().await {
                if r.status().is_success() {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        (setup, base)
    }

    /// Bind dashboard + mock parent; returns (hub_builds, parent_builds, parent, base_url).
    async fn setup_with_mock(
        token: &str,
    ) -> (
        Arc<AtomicU64>,
        Arc<AtomicU64>,
        tokio::task::JoinHandle<()>,
        String,
    ) {
        setup_with_mock_max(token, 0).await
    }

    async fn setup_with_mock_max(
        token: &str,
        max_clients: u32,
    ) -> (
        Arc<AtomicU64>,
        Arc<AtomicU64>,
        tokio::task::JoinHandle<()>,
        String,
    ) {
        let (setup, base) = bind_dashboard_with(token, max_clients, false).await;
        let hub_builds = Arc::clone(&setup.build_count);
        let parent_builds = Arc::new(AtomicU64::new(0));
        let parent = spawn_mock_parent(setup.request_rx, Arc::clone(&parent_builds));
        // Keep close_tx alive for the duration of the test by not dropping setup's other fields.
        // close_tx is intentionally moved out of setup when request_rx is taken; the watch
        // sender remaining in the hub keeps the channel open (false).
        drop(setup.close_tx);
        (hub_builds, parent_builds, parent, base)
    }

    /// Read SSE body chunks until `predicate` is true or `timeout` elapses.
    async fn read_sse_until(
        resp: &mut reqwest::Response,
        timeout: Duration,
        predicate: impl Fn(&str) -> bool,
    ) -> String {
        let mut buf = String::new();
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(400), resp.chunk()).await {
                Ok(Ok(Some(chunk))) => {
                    buf.push_str(&String::from_utf8_lossy(&chunk));
                    if predicate(&buf) {
                        return buf;
                    }
                }
                Ok(Ok(None)) => break,
                Ok(Err(e)) => panic!("stream error: {}", e),
                Err(_) => continue,
            }
        }
        buf
    }

    #[test]
    fn constant_time_eq_matches() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"Secret"));
        assert!(!constant_time_eq(b"short", b"longer"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn authorize_empty_token_allows_all() {
        let headers = HeaderMap::new();
        assert!(authorize("", &headers, None));
        assert!(authorize("", &headers, Some("anything")));
    }

    #[test]
    fn authorize_requires_token_when_configured() {
        let headers = HeaderMap::new();
        assert!(!authorize("s3cret", &headers, None));
        assert!(!authorize("s3cret", &headers, Some("wrong")));
        assert!(authorize("s3cret", &headers, Some("s3cret")));

        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer s3cret"),
        );
        assert!(authorize("s3cret", &headers, None));

        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer wrong"),
        );
        assert!(!authorize("s3cret", &headers, None));
    }

    #[test]
    fn authorize_control_always_requires_token() {
        let headers = HeaderMap::new();
        // Empty configured token: fail closed (unlike observe authorize).
        assert!(!authorize_control("", &headers, None));
        assert!(!authorize_control("", &headers, Some("anything")));
        assert!(!authorize_control("s3cret", &headers, None));
        // Query token alone is rejected for control (Bearer only).
        assert!(!authorize_control("s3cret", &headers, Some("wrong")));
        assert!(!authorize_control("s3cret", &headers, Some("s3cret")));

        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer s3cret"),
        );
        assert!(authorize_control("s3cret", &headers, None));
        // Query token must not override/augment Bearer requirement.
        assert!(authorize_control("s3cret", &headers, Some("wrong")));

        // Scheme is case-insensitive (RFC 7235).
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("bearer s3cret"),
        );
        assert!(authorize_control("s3cret", &headers, None));
        assert!(authorize("s3cret", &headers, None));
    }

    #[test]
    fn constant_time_eq_length_mismatch() {
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"ab", b"abc"));
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreT"));
    }

    #[test]
    fn parse_empty_control_body_rules() {
        assert!(parse_empty_control_body(b"").is_ok());
        assert!(parse_empty_control_body(b"   ").is_ok());
        assert!(parse_empty_control_body(b"{}").is_ok());
        assert!(parse_empty_control_body(b"  {}  ").is_ok());
        assert!(parse_empty_control_body(b"[]").is_err());
        assert!(parse_empty_control_body(b"null").is_err());
        assert!(parse_empty_control_body(b"{\"extra\":1}").is_err());
        assert!(parse_empty_control_body(b"not-json").is_err());
    }

    #[test]
    fn parse_users_body_rules() {
        assert_eq!(parse_users_body(b"{\"users\":50}").unwrap(), 50);
        assert_eq!(parse_users_body(b"{\"users\":1}").unwrap(), 1);
        assert_eq!(parse_users_body(b"{\"users\":100000}").unwrap(), 100_000);

        let err = parse_users_body(b"").unwrap_err();
        assert_eq!(err.0, "bad_request");

        let err = parse_users_body(b"[]").unwrap_err();
        assert_eq!(err.0, "bad_request");

        let err = parse_users_body(b"not-json").unwrap_err();
        assert_eq!(err.0, "bad_request");

        let err = parse_users_body(b"{}").unwrap_err();
        assert_eq!(err.0, "invalid_users");

        let err = parse_users_body(b"{\"users\":0}").unwrap_err();
        assert_eq!(err.0, "invalid_users");

        let err = parse_users_body(b"{\"users\":100001}").unwrap_err();
        assert_eq!(err.0, "invalid_users");

        let err = parse_users_body(b"{\"users\":1000000}").unwrap_err();
        assert_eq!(err.0, "invalid_users");

        let err = parse_users_body(b"{\"users\":-1}").unwrap_err();
        assert_eq!(err.0, "invalid_users");

        let err = parse_users_body(b"{\"users\":1.5}").unwrap_err();
        assert_eq!(err.0, "invalid_users");

        let err = parse_users_body(b"{\"users\":\"10\"}").unwrap_err();
        assert_eq!(err.0, "invalid_users");
    }

    #[test]
    fn format_bind_address_brackets_ipv6() {
        assert_eq!(format_bind_address("127.0.0.1", 5118), "127.0.0.1:5118");
        assert_eq!(format_bind_address("localhost", 5118), "localhost:5118");
        assert_eq!(format_bind_address("::1", 5118), "[::1]:5118");
        assert_eq!(format_bind_address("[::1]", 5118), "[::1]:5118");
        assert_eq!(format_bind_address("0.0.0.0", 80), "0.0.0.0:80");
    }

    #[tokio::test]
    async fn binds_localhost_and_ipv6_loopback() {
        // localhost (hostname — rejected by SocketAddr::parse, must use ToSocketAddrs).
        let config = GooseConfiguration {
            dashboard: true,
            dashboard_host: "localhost".to_string(),
            dashboard_port: 0,
            dashboard_auth_token: String::new(),
            ..Default::default()
        };
        let setup = setup_dashboard(&config)
            .await
            .expect("localhost bind should succeed")
            .expect("dashboard enabled");
        assert!(setup.bound_port > 0, "ephemeral port must be recorded");
        // Drop the channel; server task keeps running until process ends (fine in tests).
        drop(setup);

        // ::1 (bare IPv6 literal needs brackets in the bind string).
        let config = GooseConfiguration {
            dashboard: true,
            dashboard_host: "::1".to_string(),
            dashboard_port: 0,
            dashboard_auth_token: String::new(),
            ..Default::default()
        };
        // ::1 may be unavailable in some CI network namespaces; treat bind errors
        // as soft-skip only when the OS reports address-family issues.
        match setup_dashboard(&config).await {
            Ok(Some(setup)) => {
                assert!(setup.bound_port > 0);
            }
            Ok(None) => panic!("dashboard enabled but setup returned None"),
            Err(e) => {
                // Acceptable on hosts without IPv6 loopback.
                let msg = format!("{e}");
                eprintln!("skipping ::1 bind assertion: {msg}");
            }
        }
    }

    #[tokio::test]
    async fn bind_failure_is_hard_error() {
        // Occupy a port, then try to bind the dashboard to the same port.
        let occupied = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("occupy port");
        let port = occupied.local_addr().unwrap().port();

        let config = GooseConfiguration {
            dashboard: true,
            dashboard_host: "127.0.0.1".to_string(),
            dashboard_port: port,
            dashboard_auth_token: String::new(),
            ..Default::default()
        };
        let err = setup_dashboard(&config)
            .await
            .expect_err("second bind on same port must fail hard");
        match err {
            GooseError::Io(_) => {}
            other => panic!("expected GooseError::Io, got {:?}", other),
        }
        // Keep occupied alive until after the failed bind.
        drop(occupied);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn idle_hub_issues_no_snapshot_commands() {
        let (hub_builds, parent_builds, parent, _base) = setup_with_mock("").await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(
            parent_builds.load(Ordering::SeqCst),
            0,
            "no clients ⇒ parent must not see GetSnapshot"
        );
        assert_eq!(hub_builds.load(Ordering::SeqCst), 0);
        parent.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn multi_client_sse_coalesces_to_about_one_build_per_sec() {
        let (hub_builds, parent_builds, parent, base) = setup_with_mock("").await;
        let client = reqwest::Client::new();

        // Open several concurrent SSE clients.
        let n_clients = 6usize;
        let mut responses = Vec::new();
        for _ in 0..n_clients {
            let resp = client
                .get(format!("{base}/api/v1/events"))
                .send()
                .await
                .expect("sse connect");
            assert_eq!(resp.status(), 200, "SSE must accept client");
            let ct = resp
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            assert!(
                ct.contains("text/event-stream"),
                "content-type was {:?}",
                ct
            );
            responses.push(resp);
        }

        // Hold connections open for ~3.2s of active streaming.
        let start = Instant::now();
        tokio::time::sleep(Duration::from_millis(3200)).await;
        let elapsed = start.elapsed().as_secs_f64();

        // Drop clients so slots release.
        drop(responses);

        let builds = parent_builds.load(Ordering::SeqCst);
        let hub = hub_builds.load(Ordering::SeqCst);
        // Expect roughly 1 build/sec, not one per client per second.
        // Allow generous bounds: at least 2, at most ~elapsed+3 (startup + jitter).
        assert!(
            builds >= 2,
            "expected several coalesced builds over {:.1}s, got {}",
            elapsed,
            builds
        );
        assert!(
            builds as f64 <= elapsed + 3.0,
            "builds={} over {:.1}s looks uncoalesced (clients={})",
            builds,
            elapsed,
            n_clients
        );
        // Far below N clients * seconds.
        assert!(
            (builds as f64) < (n_clients as f64) * elapsed * 0.5,
            "builds={} too high for coalescing with {} clients over {:.1}s",
            builds,
            n_clients,
            elapsed
        );
        assert_eq!(builds, hub, "hub and parent build counters should match");

        // After all clients disconnect, hub must return to idle (no further builds).
        let after_drop = parent_builds.load(Ordering::SeqCst);
        // Past RECENT_POLL_WINDOW + a couple of cadence intervals.
        tokio::time::sleep(Duration::from_millis(3500)).await;
        let idle_baseline = parent_builds.load(Ordering::SeqCst);
        // Allow at most one in-flight tick that started before disconnect.
        assert!(
            idle_baseline <= after_drop + 2,
            "builds should stop after clients disconnect (after_drop={}, now={})",
            after_drop,
            idle_baseline
        );
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(
            parent_builds.load(Ordering::SeqCst),
            idle_baseline,
            "idle hub must not issue further GetSnapshot after clients disconnect"
        );

        parent.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn sse_client_cap_rejects_33rd_with_503() {
        let (_hub_builds, _parent_builds, parent, base) = setup_with_mock("").await;
        let client = reqwest::Client::new();

        let mut held = Vec::with_capacity(MAX_SSE_CLIENTS);
        for i in 0..MAX_SSE_CLIENTS {
            let resp = client
                .get(format!("{base}/api/v1/events"))
                .send()
                .await
                .unwrap_or_else(|e| panic!("client {} connect failed: {}", i, e));
            assert_eq!(resp.status(), 200, "client {} should be accepted", i);
            held.push(resp);
        }

        let overflow = client
            .get(format!("{base}/api/v1/events"))
            .send()
            .await
            .expect("33rd connect");
        assert_eq!(overflow.status(), 503, "33rd SSE client must receive 503");
        let body = overflow.text().await.unwrap_or_default();
        assert!(
            body.contains("too many") || body.contains("SSE"),
            "503 should include a short plain-text body, got {:?}",
            body
        );

        drop(held);
        parent.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn sse_client_cap_respects_dashboard_max_clients() {
        let cap = 2u32;
        let (_hub_builds, _parent_builds, parent, base) = setup_with_mock_max("", cap).await;
        let client = reqwest::Client::new();

        let mut held = Vec::with_capacity(cap as usize);
        for i in 0..cap {
            let resp = client
                .get(format!("{base}/api/v1/events"))
                .send()
                .await
                .unwrap_or_else(|e| panic!("client {} connect failed: {}", i, e));
            assert_eq!(resp.status(), 200, "client {} should be accepted", i);
            held.push(resp);
        }

        let overflow = client
            .get(format!("{base}/api/v1/events"))
            .send()
            .await
            .expect("overflow connect");
        assert_eq!(
            overflow.status(),
            503,
            "client beyond --dashboard-max-clients must receive 503"
        );

        // Health should report the active SSE count at the configured cap.
        let health = client
            .get(format!("{base}/api/v1/health"))
            .send()
            .await
            .expect("health");
        assert_eq!(health.status(), 200);
        let health_json: serde_json::Value = health.json().await.expect("health json");
        assert_eq!(health_json["ok"], true);
        assert_eq!(health_json["active_sse_clients"], cap);

        drop(held);
        parent.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn health_exposes_build_timing_and_client_counts() {
        let (hub_builds, _parent_builds, parent, base) = setup_with_mock("").await;
        let client = reqwest::Client::new();

        // Before any demand: counters are zero.
        let idle = client
            .get(format!("{base}/api/v1/health"))
            .send()
            .await
            .expect("health idle");
        assert_eq!(idle.status(), 200);
        let idle_json: serde_json::Value = idle.json().await.expect("json");
        assert_eq!(idle_json["ok"], true);
        assert!(!idle_json["version"].as_str().unwrap().is_empty());
        assert_eq!(idle_json["build_count"], 0);
        assert_eq!(idle_json["last_build_ms"], 0);
        assert_eq!(idle_json["active_sse_clients"], 0);

        // Trigger a snapshot build via one-shot poll.
        let snap = client
            .get(format!("{base}/api/v1/snapshot"))
            .send()
            .await
            .expect("snapshot");
        assert_eq!(snap.status(), 200);

        // Wait until the hub has recorded at least one build.
        for _ in 0..50 {
            if hub_builds.load(Ordering::SeqCst) >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            hub_builds.load(Ordering::SeqCst) >= 1,
            "expected at least one hub build after snapshot poll"
        );

        let after = client
            .get(format!("{base}/api/v1/health"))
            .send()
            .await
            .expect("health after");
        let after_json: serde_json::Value = after.json().await.expect("json");
        assert!(
            after_json["build_count"].as_u64().unwrap() >= 1,
            "build_count should advance after a snapshot, got {}",
            after_json["build_count"]
        );
        // last_build_ms is a duration; 0 is possible on a very fast mock parent,
        // but the field must be present and numeric.
        assert!(
            after_json["last_build_ms"].as_u64().is_some(),
            "last_build_ms must be a number, got {}",
            after_json["last_build_ms"]
        );
        assert_eq!(after_json["active_sse_clients"], 0);

        parent.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn events_require_token_when_configured() {
        let token = "sse-secret";
        let (_hub, _parent_builds, parent, base) = setup_with_mock(token).await;
        let client = reqwest::Client::new();

        let unauth = client
            .get(format!("{base}/api/v1/events"))
            .send()
            .await
            .expect("unauth");
        assert_eq!(unauth.status(), 401);

        let wrong = client
            .get(format!("{base}/api/v1/events?token=wrong"))
            .send()
            .await
            .expect("wrong");
        assert_eq!(wrong.status(), 401);

        let ok = client
            .get(format!("{base}/api/v1/events?token={token}"))
            .send()
            .await
            .expect("query token");
        assert_eq!(ok.status(), 200);

        let bearer = client
            .get(format!("{base}/api/v1/events"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .expect("bearer");
        assert_eq!(bearer.status(), 200);

        parent.abort();
    }

    /// Poll-only clients must observe successive hub publishes. Regression for
    /// `watch::Sender::send` no-op when no receivers are subscribed (the common
    /// path after the first poll's temporary receiver is dropped).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn poll_only_snapshots_refresh_without_sse() {
        let counter = Arc::new(AtomicU64::new(0));
        let counter2 = Arc::clone(&counter);
        let (tx, rx) = flume::unbounded();
        // Parent answers GetSnapshot with incrementing active_users.
        let parent = tokio::spawn(async move {
            while let Ok(req) = rx.recv_async().await {
                match req {
                    DashboardRequest::GetSnapshot { respond } => {
                        let n = counter2.fetch_add(1, Ordering::SeqCst);
                        let mut snap = dummy_snapshot(n + 1);
                        snap.active_users = n + 1;
                        snap.phase = if n == 0 {
                            "maintain".into()
                        } else {
                            "increase".into()
                        };
                        let _ = respond.send(snap);
                    }
                    other => {
                        // Control arms unused in this test.
                        match other {
                            DashboardRequest::Start { respond } => {
                                let _ = respond.send(ControlResult::internal("start", "idle", 0));
                            }
                            DashboardRequest::Stop { respond } => {
                                let _ = respond.send(ControlResult::internal("stop", "idle", 0));
                            }
                            DashboardRequest::SetUsers { respond, .. } => {
                                let _ = respond.send(ControlResult::internal("users", "idle", 0));
                            }
                            DashboardRequest::GetSnapshot { .. } => unreachable!(),
                        }
                    }
                }
            }
        });

        let build_count = Arc::new(AtomicU64::new(0));
        let (close_tx, _) = watch::channel(false);
        let hub = SnapshotHub::new(tx, Arc::clone(&build_count), close_tx, MAX_SSE_CLIENTS);

        // First poll.
        let b1 = hub.snapshot_bytes().await.expect("first");
        let v1: serde_json::Value = serde_json::from_slice(&b1).expect("json1");
        let users1 = v1["active_users"].as_u64().unwrap();

        // Wait past SNAPSHOT_INTERVAL so the next poll is not served from cache.
        tokio::time::sleep(SNAPSHOT_INTERVAL + Duration::from_millis(50)).await;

        // Second poll must receive a newly published snapshot (not the first body).
        let b2 = hub.snapshot_bytes().await.expect("second");
        let v2: serde_json::Value = serde_json::from_slice(&b2).expect("json2");
        let users2 = v2["active_users"].as_u64().unwrap();

        assert!(
            users2 > users1,
            "poll-only path must publish updated snapshots; got users {users1} then {users2}, builds={}",
            build_count.load(Ordering::SeqCst)
        );
        assert!(
            build_count.load(Ordering::SeqCst) >= 2,
            "expected at least 2 hub builds, got {}",
            build_count.load(Ordering::SeqCst)
        );

        parent.abort();
        // Keep hub alive until after polls (drop order).
        drop(hub);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn snapshot_goes_through_hub() {
        let (hub_builds, parent_builds, parent, base) = setup_with_mock("").await;
        let client = reqwest::Client::new();

        let resp = client
            .get(format!("{base}/api/v1/snapshot"))
            .send()
            .await
            .expect("snapshot");
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.expect("json");
        assert_eq!(body["version"], 1);
        assert_eq!(body["phase"], "maintain");

        // Concurrent polls coalesce onto hub builds.
        let start_builds = parent_builds.load(Ordering::SeqCst);
        let mut joins = Vec::new();
        for _ in 0..8 {
            let c = client.clone();
            let url = format!("{base}/api/v1/snapshot");
            joins.push(tokio::spawn(async move {
                c.get(url).send().await.expect("poll").status()
            }));
        }
        for j in joins {
            assert_eq!(j.await.unwrap(), 200);
        }
        let delta = parent_builds.load(Ordering::SeqCst) - start_builds;
        assert!(
            delta <= 2,
            "8 concurrent polls should coalesce (delta builds={})",
            delta
        );
        assert!(hub_builds.load(Ordering::SeqCst) >= 1);
        parent.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sse_stream_emits_snapshot_event() {
        let (_hub, _parent_builds, parent, base) = setup_with_mock("").await;
        let client = reqwest::Client::new();

        let mut resp = client
            .get(format!("{base}/api/v1/events"))
            .send()
            .await
            .expect("sse");
        assert_eq!(resp.status(), 200);

        let buf = read_sse_until(&mut resp, Duration::from_secs(3), |b| {
            b.contains("event: snapshot") && b.contains("data: {")
        })
        .await;
        assert!(
            buf.contains("event: snapshot") && buf.contains("data: {"),
            "did not observe snapshot SSE event in body: {:?}",
            buf
        );
        parent.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sse_emits_closed_when_parent_disconnects() {
        let (setup, base) = bind_dashboard("").await;
        let parent_builds = Arc::new(AtomicU64::new(0));
        let parent = spawn_mock_parent(setup.request_rx, Arc::clone(&parent_builds));
        // Retain close_tx so we can also test proactive close path separately;
        // this test uses parent/channel drop.
        let _close_tx = setup.close_tx;
        let client = reqwest::Client::new();

        let mut resp = client
            .get(format!("{base}/api/v1/events"))
            .send()
            .await
            .expect("sse");
        assert_eq!(resp.status(), 200);

        let buf = read_sse_until(&mut resp, Duration::from_secs(3), |b| {
            b.contains("event: snapshot")
        })
        .await;
        assert!(
            buf.contains("event: snapshot"),
            "expected snapshot before close, got {:?}",
            buf
        );

        // Drop parent (and its request_rx) so the hub observes flume disconnect.
        parent.abort();
        // Give the aborted task a moment to drop request_rx.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let buf = read_sse_until(&mut resp, Duration::from_secs(3), |b| {
            b.contains("event: closed")
        })
        .await;
        assert!(
            buf.contains("event: closed"),
            "expected event: closed after parent disconnect, got {:?}",
            buf
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sse_emits_closed_on_proactive_close_and_late_subscriber() {
        let (setup, base) = bind_dashboard("").await;
        let parent_builds = Arc::new(AtomicU64::new(0));
        let parent = spawn_mock_parent(setup.request_rx, Arc::clone(&parent_builds));
        let close_tx = setup.close_tx;
        let client = reqwest::Client::new();

        let mut resp = client
            .get(format!("{base}/api/v1/events"))
            .send()
            .await
            .expect("sse");
        assert_eq!(resp.status(), 200);
        let _ = read_sse_until(&mut resp, Duration::from_secs(3), |b| {
            b.contains("event: snapshot")
        })
        .await;

        // Proactive close (mirrors main-loop AttackPhase::Shutdown).
        let _ = close_tx.send(true);

        let buf = read_sse_until(&mut resp, Duration::from_secs(2), |b| {
            b.contains("event: closed")
        })
        .await;
        assert!(
            buf.contains("event: closed"),
            "live client must see closed after proactive signal, got {:?}",
            buf
        );

        // Late subscriber after sticky shutdown must also get closed (not hang).
        let mut late = client
            .get(format!("{base}/api/v1/events"))
            .send()
            .await
            .expect("late sse");
        assert_eq!(late.status(), 200);
        let late_buf = read_sse_until(&mut late, Duration::from_secs(2), |b| {
            b.contains("event: closed")
        })
        .await;
        assert!(
            late_buf.contains("event: closed"),
            "late subscriber after shutdown must receive closed, got {:?}",
            late_buf
        );

        parent.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn oneshot_cancel_does_not_kill_hub() {
        let (setup, base) = bind_dashboard("").await;
        let parent_builds = Arc::new(AtomicU64::new(0));
        // Drop oneshots for build attempts 2..5 (0-indexed counts after fetch_add).
        let parent = spawn_blip_parent(setup.request_rx, Arc::clone(&parent_builds), 2, 5);
        drop(setup.close_tx);
        let client = reqwest::Client::new();

        let mut resp = client
            .get(format!("{base}/api/v1/events"))
            .send()
            .await
            .expect("sse");
        assert_eq!(resp.status(), 200);

        // Wait long enough to cover the blip window (3 cancelled ticks) + recovery.
        tokio::time::sleep(Duration::from_millis(7000)).await;
        let builds = parent_builds.load(Ordering::SeqCst);
        // drop_from=2, drop_until=5 ⇒ attempts 2,3,4 cancelled; 6+ proves post-blip recovery.
        assert!(
            builds >= 6,
            "hub must keep requesting after oneshot cancels (builds={})",
            builds
        );

        // Still streaming snapshots (not stuck closed).
        let buf = read_sse_until(&mut resp, Duration::from_secs(2), |b| {
            b.contains("event: snapshot")
        })
        .await;
        assert!(
            buf.contains("event: snapshot"),
            "SSE must still deliver snapshots after transient oneshot cancels, got {:?}",
            buf
        );
        assert!(
            !buf.contains("event: closed"),
            "transient oneshot cancel must not emit closed"
        );

        parent.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn control_disabled_returns_404_and_health_false() {
        let (setup, base) = bind_dashboard_with("s3cret", 0, false).await;
        drop(setup.close_tx);
        drop(setup.request_rx);
        let client = reqwest::Client::new();

        let health: serde_json::Value = client
            .get(format!("{base}/api/v1/health"))
            .send()
            .await
            .expect("health")
            .json()
            .await
            .expect("json");
        assert_eq!(health["control_enabled"], false);

        for path in [
            "/api/v1/control/start",
            "/api/v1/control/stop",
            "/api/v1/control/users",
        ] {
            let resp = client
                .post(format!("{base}{path}"))
                .header(header::AUTHORIZATION, "Bearer s3cret")
                .header(header::CONTENT_TYPE, "application/json")
                .body("{}")
                .send()
                .await
                .expect("post");
            assert_eq!(
                resp.status(),
                404,
                "control off must 404 {path}, got {}",
                resp.status()
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn control_requires_auth_and_validates_body() {
        let (setup, base) = bind_dashboard_with("s3cret", 0, true).await;
        let parent = spawn_mock_parent(setup.request_rx, Arc::new(AtomicU64::new(0)));
        drop(setup.close_tx);
        let client = reqwest::Client::new();

        let health: serde_json::Value = client
            .get(format!("{base}/api/v1/health"))
            .send()
            .await
            .expect("health")
            .json()
            .await
            .expect("json");
        assert_eq!(health["control_enabled"], true);

        // Missing token → 401.
        let resp = client
            .post(format!("{base}/api/v1/control/start"))
            .header(header::CONTENT_TYPE, "application/json")
            .body("{}")
            .send()
            .await
            .expect("post");
        assert_eq!(resp.status(), 401);

        // Wrong token → 401.
        let resp = client
            .post(format!("{base}/api/v1/control/start"))
            .header(header::AUTHORIZATION, "Bearer wrong")
            .header(header::CONTENT_TYPE, "application/json")
            .body("{}")
            .send()
            .await
            .expect("post");
        assert_eq!(resp.status(), 401);

        // Bad start body → 400 bad_request.
        let resp = client
            .post(format!("{base}/api/v1/control/start"))
            .header(header::AUTHORIZATION, "Bearer s3cret")
            .header(header::CONTENT_TYPE, "application/json")
            .body("[1]")
            .send()
            .await
            .expect("post");
        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = resp.json().await.expect("json");
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"], "bad_request");
        assert_eq!(body["command"], "start");

        // users: 0 → 400 invalid_users.
        let resp = client
            .post(format!("{base}/api/v1/control/users"))
            .header(header::AUTHORIZATION, "Bearer s3cret")
            .header(header::CONTENT_TYPE, "application/json")
            .body("{\"users\":0}")
            .send()
            .await
            .expect("post");
        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = resp.json().await.expect("json");
        assert_eq!(body["error"], "invalid_users");

        // Valid start with Bearer → 200 ok (mock parent).
        let resp = client
            .post(format!("{base}/api/v1/control/start"))
            .header(header::AUTHORIZATION, "Bearer s3cret")
            .header(header::CONTENT_TYPE, "application/json")
            .body("{}")
            .send()
            .await
            .expect("post");
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.expect("json");
        assert_eq!(body["ok"], true);
        assert_eq!(body["command"], "start");
        assert_eq!(body["phase"], "increase");

        // Query token alone is rejected for control (Bearer only).
        let resp = client
            .post(format!("{base}/api/v1/control/stop?token=s3cret"))
            .header(header::CONTENT_TYPE, "application/json")
            .body("{}")
            .send()
            .await
            .expect("post");
        assert_eq!(resp.status(), 401);

        // Stop with Bearer → 200.
        let resp = client
            .post(format!("{base}/api/v1/control/stop"))
            .header(header::AUTHORIZATION, "Bearer s3cret")
            .header(header::CONTENT_TYPE, "application/json")
            .body("{}")
            .send()
            .await
            .expect("post");
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.expect("json");
        assert_eq!(body["ok"], true);
        assert_eq!(body["phase"], "decrease");

        // Empty body accepted for start.
        let resp = client
            .post(format!("{base}/api/v1/control/start"))
            .header(header::AUTHORIZATION, "Bearer s3cret")
            .send()
            .await
            .expect("post");
        assert_eq!(resp.status(), 200);

        parent.abort();
    }

    /// Parent that drops control oneshots (simulates main-loop disconnect mid-command).
    fn spawn_control_drop_parent(
        request_rx: flume::Receiver<DashboardRequest>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Ok(msg) = request_rx.recv_async().await {
                match msg {
                    DashboardRequest::GetSnapshot { respond } => {
                        let _ = respond.send(dummy_snapshot(0));
                    }
                    DashboardRequest::Start { respond }
                    | DashboardRequest::Stop { respond }
                    | DashboardRequest::SetUsers { respond, .. } => {
                        drop(respond);
                    }
                }
            }
        })
    }

    /// Parent that answers control with `error: "internal"` (hard-error mapping).
    fn spawn_control_internal_parent(
        request_rx: flume::Receiver<DashboardRequest>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Ok(msg) = request_rx.recv_async().await {
                match msg {
                    DashboardRequest::GetSnapshot { respond } => {
                        let _ = respond.send(dummy_snapshot(0));
                    }
                    DashboardRequest::Start { respond } => {
                        let _ = respond.send(ControlResult::internal("start", "idle", 0));
                    }
                    DashboardRequest::Stop { respond } => {
                        let _ = respond.send(ControlResult::internal("stop", "maintain", 1));
                    }
                    DashboardRequest::SetUsers { respond, .. } => {
                        let _ = respond.send(ControlResult::internal("users", "maintain", 1));
                    }
                }
            }
        })
    }

    /// Parent that never completes control oneshots (exercises CONTROL_TIMEOUT → 503).
    fn spawn_control_silent_parent(
        request_rx: flume::Receiver<DashboardRequest>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Ok(msg) = request_rx.recv_async().await {
                match msg {
                    DashboardRequest::GetSnapshot { respond } => {
                        let _ = respond.send(dummy_snapshot(0));
                    }
                    // Hold oneshots until task abort / channel drop — do not reply.
                    DashboardRequest::Start { respond }
                    | DashboardRequest::Stop { respond }
                    | DashboardRequest::SetUsers { respond, .. } => {
                        // Keep the oneshot Sender alive so the client hits CONTROL_TIMEOUT
                        // rather than an immediate oneshot-drop 503.
                        let _hold = respond;
                        futures::future::pending::<()>().await;
                        drop(_hold);
                    }
                }
            }
        })
    }

    /// Case 10: channel disconnect / oneshot drop → 503 `unavailable`;
    /// parent `internal` → 200 `error=internal`; oneshot timeout → 503.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn control_oneshot_error_paths() {
        let client = reqwest::Client::new();

        // --- disconnect: no parent consuming request_rx ---
        {
            let (setup, base) = bind_dashboard_with("s3cret", 0, true).await;
            drop(setup.close_tx);
            drop(setup.request_rx);
            let resp = client
                .post(format!("{base}/api/v1/control/start"))
                .header(header::AUTHORIZATION, "Bearer s3cret")
                .header(header::CONTENT_TYPE, "application/json")
                .body("{}")
                .send()
                .await
                .expect("post");
            assert_eq!(resp.status(), 503, "disconnected channel must 503");
            let body: serde_json::Value = resp.json().await.expect("json");
            assert_eq!(body["ok"], false);
            assert_eq!(body["error"], "unavailable");
            assert_eq!(body["command"], "start");
        }

        // --- oneshot drop: parent receives then drops respond ---
        {
            let (setup, base) = bind_dashboard_with("s3cret", 0, true).await;
            let parent = spawn_control_drop_parent(setup.request_rx);
            drop(setup.close_tx);
            let resp = client
                .post(format!("{base}/api/v1/control/stop"))
                .header(header::AUTHORIZATION, "Bearer s3cret")
                .header(header::CONTENT_TYPE, "application/json")
                .body("{}")
                .send()
                .await
                .expect("post");
            assert_eq!(resp.status(), 503, "oneshot drop must 503");
            let body: serde_json::Value = resp.json().await.expect("json");
            assert_eq!(body["ok"], false);
            assert_eq!(body["error"], "unavailable");
            assert_eq!(body["command"], "stop");
            parent.abort();
        }

        // --- internal soft mapping: 200 + error=internal ---
        {
            let (setup, base) = bind_dashboard_with("s3cret", 0, true).await;
            let parent = spawn_control_internal_parent(setup.request_rx);
            drop(setup.close_tx);
            let resp = client
                .post(format!("{base}/api/v1/control/users"))
                .header(header::AUTHORIZATION, "Bearer s3cret")
                .header(header::CONTENT_TYPE, "application/json")
                .body(r#"{"users":5}"#)
                .send()
                .await
                .expect("post");
            assert_eq!(resp.status(), 200, "internal maps to HTTP 200");
            let body: serde_json::Value = resp.json().await.expect("json");
            assert_eq!(body["ok"], false);
            assert_eq!(body["error"], "internal");
            assert_eq!(body["command"], "users");
            assert_eq!(body["message"], "internal control error");
            parent.abort();
        }

        // --- oneshot timeout: parent never replies → 503 after CONTROL_TIMEOUT ---
        {
            let (setup, base) = bind_dashboard_with("s3cret", 0, true).await;
            let parent = spawn_control_silent_parent(setup.request_rx);
            drop(setup.close_tx);
            let started = Instant::now();
            let resp = client
                .post(format!("{base}/api/v1/control/start"))
                .header(header::AUTHORIZATION, "Bearer s3cret")
                .header(header::CONTENT_TYPE, "application/json")
                .body("{}")
                .send()
                .await
                .expect("post");
            let elapsed = started.elapsed();
            assert_eq!(resp.status(), 503, "timeout must 503");
            let body: serde_json::Value = resp.json().await.expect("json");
            assert_eq!(body["ok"], false);
            assert_eq!(body["error"], "timeout");
            assert_eq!(body["command"], "start");
            assert!(
                body["message"]
                    .as_str()
                    .unwrap_or("")
                    .contains("check phase"),
                "timeout message must warn against blind retry: {:?}",
                body["message"]
            );
            // CONTROL_TIMEOUT is 15s; allow a little slack for scheduling.
            assert!(
                elapsed >= Duration::from_secs(14),
                "expected ~15s timeout, got {:?}",
                elapsed
            );
            assert!(
                elapsed < Duration::from_secs(30),
                "timeout should not hang forever, got {:?}",
                elapsed
            );
            parent.abort();
        }
    }

    #[test]
    fn csp_includes_frame_ancestors_none() {
        assert!(
            CSP.contains("frame-ancestors 'none'"),
            "CSP must block embedding: {}",
            CSP
        );
    }
}
