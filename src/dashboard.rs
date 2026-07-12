//! Read-only live web dashboard HTTP server.
//!
//! Compiled only with the `dashboard` crate feature. Spawns an axum server that
//! serves a minimal static shell, a one-shot metrics snapshot API, and an SSE
//! stream of coalesced snapshots.
//!
//! The parent GooseAttack main loop answers [`DashboardRequest`]s via oneshot
//! channels using [`MetricsCommand::GetDashboardSnapshot`].
//!
//! # Snapshot freshness
//!
//! [`SnapshotHub`] builds at most one snapshot per second while SSE clients are
//! connected or a recent poll occurred. Expected display freshness is **~1 s**,
//! with up to **~0.5 s scheduling jitter** in the main loop before a snapshot
//! request is dequeued. Tests assert multi-client coalescing, not sub-100 ms
//! latency.

use crate::metrics::DashboardSnapshot;
use crate::{GooseConfiguration, GooseError};

use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Response, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use futures::stream;
use serde::Deserialize;
use serde::Serialize;
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{oneshot, watch, Notify};
use tower_http::set_header::SetResponseHeaderLayer;

/// Embedded SPA shell and assets (no inline scripts — CSP-friendly).
const INDEX_HTML: &str = include_str!("dashboard/static/index.html");
const APP_JS: &str = include_str!("dashboard/static/app.js");
const APP_CSS: &str = include_str!("dashboard/static/app.css");

/// Content-Security-Policy applied to every response.
const CSP: &str =
    "default-src 'self'; script-src 'self'; connect-src 'self'; style-src 'self'; img-src 'self' data:";

/// Hard cap on concurrent SSE clients (v1). The 33rd connection receives 503.
const MAX_SSE_CLIENTS: usize = 32;

/// Server-side snapshot cadence while clients are active.
const SNAPSHOT_INTERVAL: Duration = Duration::from_secs(1);

/// How long a one-shot poll keeps the hub building after the request.
const RECENT_POLL_WINDOW: Duration = Duration::from_secs(2);

/// SSE comment heartbeat interval (`: ping`).
const SSE_HEARTBEAT: Duration = Duration::from_secs(15);

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
    /// Unix-ms of the most recent `/api/v1/snapshot` poll mark (0 = never).
    last_poll_ms: AtomicU64,
    /// Unix-ms of the most recent successful build (0 = never).
    last_build_ms: AtomicU64,
    /// Successful hub builds (shared with [`DashboardSetup::build_count`]).
    build_count: Arc<AtomicU64>,
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
            .field("build_count", &self.build_count.load(Ordering::Relaxed))
            .field("last_poll_ms", &self.last_poll_ms.load(Ordering::Relaxed))
            .field("last_build_ms", &self.last_build_ms.load(Ordering::Relaxed))
            .finish()
    }
}

impl SnapshotHub {
    fn new(
        request_tx: flume::Sender<DashboardRequest>,
        build_count: Arc<AtomicU64>,
        shutdown_tx: watch::Sender<bool>,
    ) -> Self {
        let (latest_tx, _) = watch::channel(None);
        let inner = Arc::new(HubInner {
            request_tx,
            latest_tx,
            active_sse: AtomicUsize::new(0),
            last_poll_ms: AtomicU64::new(0),
            last_build_ms: AtomicU64::new(0),
            build_count,
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

    /// Reserve an SSE client slot. Returns `false` when at [`MAX_SSE_CLIENTS`].
    fn try_acquire_sse(&self) -> bool {
        // Do not consume a slot when the hub is already permanently closed.
        if *self.inner.shutdown_tx.borrow() {
            return false;
        }
        let mut current = self.inner.active_sse.load(Ordering::Relaxed);
        loop {
            if current >= MAX_SSE_CLIENTS {
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
        let built_at = self.inner.last_build_ms.load(Ordering::Relaxed);
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
                // Metrics/main-loop blip: do not kill the hub. Brief backoff so
                // we do not spin while the processor is recycling.
                debug!("[dashboard]: snapshot reply cancelled; will retry");
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
async fn build_once(inner: &HubInner) -> BuildOutcome {
    if inner.request_tx.is_disconnected() {
        return BuildOutcome::Fatal;
    }

    let (respond_tx, respond_rx) = oneshot::channel();
    if inner
        .request_tx
        .send(DashboardRequest::GetSnapshot {
            respond: respond_tx,
        })
        .is_err()
    {
        return BuildOutcome::Fatal;
    }

    let snapshot = match respond_rx.await {
        Ok(s) => s,
        Err(_) => {
            // Metrics processor / main-loop dropped this oneshot without a
            // reply. Recoverable (e.g. Idle after controller stop then restart).
            return BuildOutcome::Transient;
        }
    };

    match serde_json::to_vec(&snapshot) {
        Ok(json) => {
            // Publish first, then advance generation counters so waiters that
            // observe `build_count` never read a stale watch value.
            let _ = inner.latest_tx.send(Some(Bytes::from(json)));
            inner.last_build_ms.store(unix_now_ms(), Ordering::Release);
            inner.build_count.fetch_add(1, Ordering::Release);
            BuildOutcome::Built
        }
        Err(e) => {
            warn!("[dashboard]: failed to serialize snapshot: {e}");
            // Keep the hub running; waiters will time out or get a later build.
            BuildOutcome::Built
        }
    }
}

/// Shared state for axum handlers.
#[derive(Clone, Debug)]
struct DashboardState {
    /// Coalescing snapshot hub (owns the parent request channel sender).
    hub: SnapshotHub,
    /// Auth token; empty means auth is disabled (loopback-only runs).
    auth_token: String,
}

#[derive(Debug, Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    version: String,
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
    let hub = SnapshotHub::new(request_tx, Arc::clone(&build_count), close_tx.clone());
    let state = DashboardState {
        hub,
        auth_token: configuration.dashboard_auth_token.clone(),
    };

    let app = build_router(state);

    info!("[dashboard]: listening on http://{bound} (read-only)");

    // Detached server task — no need to rejoin when the load test ends.
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
fn build_router(state: DashboardState) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/static/app.js", get(app_js_handler))
        .route("/static/app.css", get(app_css_handler))
        .route("/api/v1/health", get(health_handler))
        .route("/api/v1/snapshot", get(snapshot_handler))
        .route("/api/v1/events", get(events_handler))
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

/// Build a static asset response without `unwrap` on the builder.
fn static_response(content_type: &'static str, body: &'static str) -> Response<Body> {
    ([(header::CONTENT_TYPE, content_type)], Body::from(body)).into_response()
}

async fn health_handler() -> impl IntoResponse {
    Json(HealthResponse {
        ok: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
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

    if let Some(value) = headers.get(header::AUTHORIZATION) {
        if let Ok(s) = value.to_str() {
            if let Some(token) = s.strip_prefix("Bearer ") {
                if constant_time_eq(token.as_bytes(), configured_token.as_bytes()) {
                    return true;
                }
            }
        }
    }

    false
}

/// Best-effort constant-time equality for auth tokens.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
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
                }
            }
        })
    }

    async fn bind_dashboard(token: &str) -> (DashboardSetup, String) {
        let config = GooseConfiguration {
            dashboard: true,
            dashboard_host: "127.0.0.1".to_string(),
            dashboard_port: 0,
            dashboard_auth_token: token.to_string(),
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
        let (setup, base) = bind_dashboard(token).await;
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
}
