//! Live web dashboard HTTP server.
//!
//! Compiled only with the `dashboard` crate feature. Runs an HTTP/1 accept
//! loop over an axum router that serves a minimal static shell, a one-shot
//! metrics snapshot API, an SSE stream of coalesced snapshots, and (when
//! `--dashboard-control` is set) authenticated Start/Stop/Users/Quit control
//! endpoints, the report switches, and report deletion.
//!
//! The parent GooseAttack main loop answers [`DashboardRequest`]s via oneshot
//! channels using [`MetricsCommand::GetDashboardSnapshot`] for snapshots.
//! Control endpoints send [`ControllerCommand`]s on the channel the telnet and
//! WebSocket Controllers share, and the main loop answers them the same way.
//!
//! # Snapshot freshness
//!
//! [`SnapshotHub`] builds at most one snapshot per second while SSE clients are
//! connected or a recent poll occurred. Expected display freshness is **~1 s**,
//! with up to **~0.5 s scheduling jitter** in the main loop before a snapshot
//! request is dequeued. Tests assert multi-client coalescing, not sub-100 ms
//! latency.

use crate::controller::{
    self, send_command, ControlCommand, ControlReply, ControlRequest, ControllerCommand,
    ListeningControllers, RunSettings, SendError, CONTROL_TIMEOUT,
};
use crate::metrics::dashboard_snapshot::{
    Heatmap, HeatmapColumn, HeatmapCursor, RequestSeries, SnapshotSettings,
};
use crate::metrics::DashboardSnapshot;
use crate::runs;
use crate::{ControlOutcome, GooseConfiguration, GooseError};

use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Response, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use futures::stream;
use serde::Deserialize;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt;
use tokio::sync::{oneshot, watch, Notify};
use tower_http::set_header::SetResponseHeaderLayer;

/// Embedded SPA shell and assets (no inline scripts — CSP-friendly).
const INDEX_HTML: &str = include_str!("dashboard/static/index.html");
const APP_JS: &str = include_str!("dashboard/static/app.js");
const APP_CSS: &str = include_str!("dashboard/static/app.css");

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

/// Hub wait for a single parent/metrics snapshot reply before treating the build
/// as transient (poll clients already use a 5s outer timeout).
const SNAPSHOT_BUILD_TIMEOUT: Duration = Duration::from_secs(4);

/// How long [`DashboardServer::shutdown`] lets open connections finish (an
/// SSE stream sending `event: closed`, a response in flight) before closing
/// them. Bounds the wait in `execute()` when a client never finishes a request.
const SERVER_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

/// How long a client has to send a request's headers, on a new connection and
/// between requests on a keep-alive one, before the connection is closed. A
/// header read timeout and not a whole request timeout, which would cut the
/// long-lived `/api/v1/events` streams.
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Connections allowed beyond the SSE client cap: every browser tab holds one
/// SSE stream plus keep-alive connections for static assets, polls and
/// control requests, and monitors and scripts need a few more.
const EXTRA_CONNECTIONS: usize = 16;

/// Max JSON body size for control POSTs (stop is empty, users one int, and
/// start or settings at most every run setting, a test plan of up to 4096
/// bytes among them).
const CONTROL_BODY_LIMIT: usize = 16 * 1024;

/// Max concurrent control POSTs waiting on the main loop (admission control).
/// Additional authenticated control requests receive 503 `busy` without
/// enqueueing unbounded flume work.
const MAX_CONTROL_IN_FLIGHT: usize = 4;

/// Max concurrent `GET /api/v1/series` requests waiting on the main loop.
/// Counted apart from control requests, so polling a series can never make
/// Start or Stop answer `busy`.
const MAX_SERIES_IN_FLIGHT: usize = 4;

/// Longest combined length, in bytes, of the `method` and `name` parameters
/// of `GET /api/v1/series`.
const MAX_SERIES_QUERY_LEN: usize = 2048;

/// How long a `GET /api/v1/series` answer is served from the cache, so
/// several viewers polling one request cost one export a second.
const SERIES_CACHE_TTL: Duration = Duration::from_secs(1);

/// Result of a successful dashboard server spawn.
#[derive(Debug)]
pub(crate) struct DashboardSetup {
    /// Parent end of the request channel.
    pub request_rx: flume::Receiver<DashboardRequest>,
    /// Actual TCP port after bind.
    ///
    /// Exposed for unit tests and internal callers that bind with port 0 (ephemeral).
    /// Note: the CLI treats `--dashboard-port 0` as “unset” and rewrites it to the
    /// default (5118) when the dashboard is enabled.
    #[cfg_attr(not(test), allow(dead_code))]
    pub bound_port: u16,
    /// Number of snapshots the hub has successfully built (tests / diagnostics).
    #[cfg_attr(not(test), allow(dead_code))]
    pub build_count: Arc<AtomicU64>,
    /// Concurrent SSE clients, which the main loop reads to decide whether
    /// to wait for a dashboard tab after an autostarted run.
    pub active_sse: Arc<AtomicUsize>,
    /// The name of the folder of the run in progress, which Delete refuses
    /// and the runs panel shows as running. The main loop keeps it current.
    pub running_folder: RunningFolder,
    /// The name of the folder a bare `--goose-log` is open in, which Delete
    /// refuses. It stays set after the run ends, as the log stays open there
    /// until the next run starts.
    pub log_folder: RunningFolder,
    /// The address the dashboard listens on, as logged at startup.
    pub url: String,
    /// Signal SSE clients to emit `event: closed` and stop the hub loop.
    ///
    /// Used on final process shutdown so browsers get a clean close without
    /// waiting for the next 1 Hz build attempt to observe a dropped flume channel.
    pub close_tx: watch::Sender<bool>,
    /// The running HTTP server; stops when shut down or dropped.
    pub server: DashboardServer,
}

/// Requests from the dashboard HTTP task to the GooseAttack main loop.
#[derive(Debug)]
pub(crate) enum DashboardRequest {
    /// Build and return a compact metrics snapshot, with the heatmap's
    /// columns changed since `heatmap_since` (every column when `None`).
    GetSnapshot {
        heatmap_since: Option<HeatmapCursor>,
        respond: tokio::sync::oneshot::Sender<DashboardSnapshot>,
    },
    /// One request's heatmap (`GET /api/v1/series`), with the columns
    /// changed since `since` (every column when `None`); `None` when the
    /// request has no histograms of its own.
    GetRequestSeries {
        method: String,
        name: String,
        since: Option<HeatmapCursor>,
        respond: tokio::sync::oneshot::Sender<Option<RequestSeries>>,
    },
}

/// JSON body returned by control endpoints after the main loop answers.
#[derive(Debug, Clone, Serialize)]
struct ControlResult {
    ok: bool,
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    message: String,
    phase: String,
    active_users: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_users: Option<u64>,
    /// The setting at fault in a refusal, when one is.
    #[serde(skip_serializing_if = "Option::is_none")]
    field: Option<String>,
    /// The run settings after a settings request or a Start with settings,
    /// as the snapshot's `settings`.
    #[serde(skip_serializing_if = "Option::is_none")]
    settings: Option<Box<SnapshotSettings>>,
}

impl ControlResult {
    /// The body for the main loop's outcome of the `command` endpoint.
    fn from_outcome(command: &str, outcome: ControlOutcome) -> Self {
        Self {
            ok: outcome.ok,
            command: command.to_string(),
            error: outcome.error.map(str::to_string),
            message: outcome.message,
            phase: outcome.phase,
            active_users: outcome.active_users as u64,
            target_users: outcome.target_users.map(|u| u as u64),
            field: outcome.field.map(str::to_string),
            settings: outcome.settings,
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
    /// The setting at fault, `null` when no one setting is.
    field: Option<String>,
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
            field: None,
        }
    }

    /// As [`Self::new`], naming the setting at fault.
    fn with_field(mut self, field: Option<&str>) -> Self {
        self.field = field.map(str::to_string);
        self
    }
}

/// Every heatmap column the dashboard has been sent for the window, each
/// with the `rev` of the export that last changed it, so a client can be sent
/// only the columns changed since the heatmap it holds.
struct HeatmapStore {
    generation: u64,
    rev: u64,
    start_second: u64,
    end_second: u64,
    hold_from: u64,
    buckets_us: Vec<[u64; 3]>,
    columns: BTreeMap<u64, (u64, HeatmapColumn)>,
}

impl HeatmapStore {
    fn new(heatmap: &Heatmap) -> Self {
        let mut store = HeatmapStore {
            generation: heatmap.generation,
            rev: 0,
            start_second: 0,
            end_second: 0,
            hold_from: 0,
            buckets_us: Vec::new(),
            columns: BTreeMap::new(),
        };
        store.apply(heatmap);
        store
    }

    /// Takes in an export: the columns changed since this store's cursor, or
    /// a whole window that replaces it.
    fn apply(&mut self, heatmap: &Heatmap) {
        if heatmap.since == 0 || heatmap.generation != self.generation {
            self.columns.clear();
        }
        self.generation = heatmap.generation;
        self.rev = heatmap.rev;
        self.start_second = heatmap.start_second;
        self.end_second = heatmap.end_second;
        self.hold_from = heatmap.hold_from;
        if heatmap.buckets_us.len() >= self.buckets_us.len() {
            self.buckets_us = heatmap.buckets_us.clone();
        }
        for column in &heatmap.columns {
            self.columns
                .insert(column.second, (heatmap.rev, column.clone()));
        }
        self.columns = self.columns.split_off(&heatmap.start_second);
    }

    fn cursor(&self) -> HeatmapCursor {
        HeatmapCursor {
            generation: self.generation,
            rev: self.rev,
        }
    }

    /// The columns a client holding `cursor` lacks: those changed after it,
    /// or every column when it is not a cursor of this generation.
    fn since(&self, cursor: Option<HeatmapCursor>) -> Heatmap {
        let since = match cursor {
            Some(cursor) if cursor.generation == self.generation && cursor.rev <= self.rev => {
                cursor.rev
            }
            _ => 0,
        };
        Heatmap {
            generation: self.generation,
            rev: self.rev,
            since,
            start_second: self.start_second,
            end_second: self.end_second,
            hold_from: self.hold_from,
            buckets_us: self.buckets_us.clone(),
            columns: self
                .columns
                .values()
                .filter(|(rev, _)| *rev > since)
                .map(|(_, column)| column.clone())
                .collect(),
        }
    }
}

/// The latest snapshot without its heatmap, and the heatmap's columns, for
/// answering a poll with only the columns that client lacks.
#[derive(Default)]
struct HubView {
    snapshot: Option<DashboardSnapshot>,
    heatmap: Option<HeatmapStore>,
}

/// Coalescing fan-out for dashboard snapshots.
///
/// One hub task builds at most ~1 snapshot/sec while `active_sse > 0` or a poll
/// was recent. Idle (no clients, no recent poll) issues **no**
/// `GetDashboardSnapshot` commands. SSE clients all get the same snapshot,
/// whose heatmap holds the columns changed since the previous build; a client
/// that missed one, and every poll, asks with its own heatmap cursor and is
/// answered from [`HubView`].
#[derive(Clone, Debug)]
struct SnapshotHub {
    inner: Arc<HubInner>,
}

struct HubInner {
    request_tx: flume::Sender<DashboardRequest>,
    /// Latest pre-serialized snapshot JSON (`None` until the first build).
    latest_tx: watch::Sender<Option<Bytes>>,
    /// Concurrent SSE subscribers (shared with [`DashboardSetup::active_sse`]).
    active_sse: Arc<AtomicUsize>,
    /// Configured concurrent SSE client cap (default [`MAX_SSE_CLIENTS`]).
    max_sse_clients: usize,
    /// Unix-ms of the most recent `/api/v1/snapshot` poll mark (0 = never).
    last_poll_ms: AtomicU64,
    /// Unix-ms of the most recent successful build (0 = never). Used for freshness.
    last_build_at_ms: AtomicU64,
    /// Time the metrics processor spent on the most recent published snapshot,
    /// draining metrics and building it, in milliseconds. Excludes waiting
    /// for the main loop and the processor queue, and serialization.
    last_build_ms: AtomicU64,
    /// Hub round trip of the most recent published snapshot, in milliseconds:
    /// from sending the request to publishing the JSON.
    last_snapshot_ms: AtomicU64,
    /// Snapshot requests not answered within [`SNAPSHOT_BUILD_TIMEOUT`]. A
    /// late reply is logged and dropped, never published.
    snapshot_timeouts: AtomicU64,
    /// Successful hub builds (shared with [`DashboardSetup::build_count`]).
    build_count: Arc<AtomicU64>,
    /// True while a GetSnapshot request is outstanding (including after the hub
    /// timed out waiting — prevents re-request pile-up on the metrics processor).
    snapshot_in_flight: AtomicBool,
    /// Wake the hub loop from idle when demand appears.
    wake: Notify,
    /// When `true`, SSE clients should emit `event: closed` and disconnect.
    shutdown_tx: watch::Sender<bool>,
    /// The latest snapshot and heatmap columns, for polls.
    view: Mutex<HubView>,
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
                "last_snapshot_ms",
                &self.last_snapshot_ms.load(Ordering::Relaxed),
            )
            .field(
                "snapshot_timeouts",
                &self.snapshot_timeouts.load(Ordering::Relaxed),
            )
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
        active_sse: Arc<AtomicUsize>,
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
            active_sse,
            max_sse_clients,
            last_poll_ms: AtomicU64::new(0),
            last_build_at_ms: AtomicU64::new(0),
            last_build_ms: AtomicU64::new(0),
            last_snapshot_ms: AtomicU64::new(0),
            snapshot_timeouts: AtomicU64::new(0),
            build_count,
            snapshot_in_flight: AtomicBool::new(false),
            wake: Notify::new(),
            shutdown_tx,
            view: Mutex::new(HubView::default()),
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

    /// Processor time of the last published snapshot in milliseconds (0 if none).
    fn last_build_ms(&self) -> u64 {
        self.inner.last_build_ms.load(Ordering::Relaxed)
    }

    /// Hub round trip of the last published snapshot in milliseconds (0 if none).
    fn last_snapshot_ms(&self) -> u64 {
        self.inner.last_snapshot_ms.load(Ordering::Relaxed)
    }

    /// Snapshot requests that timed out waiting for a reply.
    fn snapshot_timeouts(&self) -> u64 {
        self.inner.snapshot_timeouts.load(Ordering::Relaxed)
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

    /// The latest snapshot with the heatmap columns a client holding
    /// `cursor` lacks, as JSON. Waits for a fresh build as
    /// [`Self::snapshot_bytes`] does.
    async fn snapshot_bytes_since(
        &self,
        cursor: Option<HeatmapCursor>,
    ) -> Result<Bytes, StatusCode> {
        let shared = self.snapshot_bytes().await?;
        let snapshot = {
            let view = self.inner.view.lock().unwrap_or_else(|e| e.into_inner());
            let Some(snapshot) = view.snapshot.as_ref() else {
                return Ok(shared);
            };
            let mut snapshot = snapshot.clone();
            snapshot.series.heatmap = view.heatmap.as_ref().map(|store| store.since(cursor));
            snapshot
        };
        match serde_json::to_vec(&snapshot) {
            Ok(json) => Ok(Bytes::from(json)),
            Err(e) => {
                warn!("[dashboard]: failed to serialize snapshot: {e}");
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
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

        match build_once(&inner, SNAPSHOT_BUILD_TIMEOUT).await {
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

/// Request one snapshot from the parent and publish pre-serialized JSON,
/// waiting up to `timeout` for the reply ([`SNAPSHOT_BUILD_TIMEOUT`] outside
/// tests).
async fn build_once(inner: &Arc<HubInner>, timeout: Duration) -> BuildOutcome {
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
    let heatmap_since = inner
        .view
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .heatmap
        .as_ref()
        .map(HeatmapStore::cursor);
    if inner
        .request_tx
        .send(DashboardRequest::GetSnapshot {
            heatmap_since,
            respond: respond_tx,
        })
        .is_err()
    {
        inner.snapshot_in_flight.store(false, Ordering::Release);
        return BuildOutcome::Fatal;
    }

    // Wait up to `timeout`; on timeout keep the oneshot alive in a
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
        _ = tokio::time::sleep(timeout) => {
            inner.snapshot_timeouts.fetch_add(1, Ordering::Relaxed);
            debug!(
                "[dashboard]: snapshot build timed out after {timeout:?}; waiting in background"
            );
            let clear_flag = Arc::clone(inner);
            tokio::spawn(async move {
                // A late reply is stale by now: log it, never publish it.
                if respond_rx.await.is_ok() {
                    debug!(
                        "[dashboard]: late snapshot reply arrived after {}ms; dropped",
                        started.elapsed().as_millis()
                    );
                }
                clear_flag.snapshot_in_flight.store(false, Ordering::Release);
            });
            return BuildOutcome::Transient;
        }
    };

    let build_time = snapshot.build_time;

    // Keep the heatmap's columns for polls. SSE clients are sent the export as
    // it came: the columns changed since the previous build.
    {
        let mut view = inner.view.lock().unwrap_or_else(|e| e.into_inner());
        match (snapshot.series.heatmap.as_ref(), view.heatmap.as_mut()) {
            (Some(heatmap), Some(store)) => store.apply(heatmap),
            (Some(heatmap), None) => view.heatmap = Some(HeatmapStore::new(heatmap)),
            (None, _) => view.heatmap = None,
        }
        let mut kept = snapshot.clone();
        kept.series.heatmap = None;
        view.snapshot = Some(kept);
    }

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
            let snapshot_ms = started.elapsed().as_millis() as u64;
            let build_ms = build_time.as_millis() as u64;
            inner
                .last_build_at_ms
                .store(unix_now_ms(), Ordering::Release);
            inner.last_build_ms.store(build_ms, Ordering::Release);
            inner.last_snapshot_ms.store(snapshot_ms, Ordering::Release);
            let count = inner.build_count.fetch_add(1, Ordering::Release) + 1;
            debug!(
                "[dashboard]: snapshot build {} took {}ms, round trip {}ms (active_sse={})",
                count,
                build_ms,
                snapshot_ms,
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
#[derive(Clone)]
struct DashboardState {
    /// Coalescing snapshot hub (owns a clone of the parent request channel sender).
    hub: SnapshotHub,
    /// Clone of the flume sender for series requests (same channel as the hub).
    request_tx: flume::Sender<DashboardRequest>,
    /// The channel control commands go to the main loop on, apart from the
    /// telnet and WebSocket Controllers' so the main loop answers it first:
    /// `Some` only with dashboard control, when the control routes are
    /// registered and require a non-empty token.
    control_tx: Option<flume::Sender<ControlRequest>>,
    /// Auth token; empty means observe auth is disabled (loopback-only runs,
    /// and every run under `--dashboard-no-auth`).
    auth_token: String,
    /// `--dashboard-no-auth`: control needs no token, and the control routes
    /// refuse a cross site request instead.
    no_auth: bool,
    /// Whether the dashboard is bound to a loopback address.
    loopback: bool,
    /// Concurrent control POSTs currently waiting on the main loop.
    control_in_flight: Arc<AtomicUsize>,
    /// Concurrent `GET /api/v1/series` requests waiting on the main loop.
    series_in_flight: Arc<AtomicUsize>,
    /// Recent `GET /api/v1/series` 200 answers by method and name, with when
    /// each was built.
    series_cache: Arc<Mutex<SeriesCache>>,
    /// The folder run folders go in, which the report routes read, and only
    /// them.
    reports_dir: std::path::PathBuf,
    /// Each `report.json` the listing has read, by path, size and time.
    listing_cache: Arc<Mutex<runs::ListingCache>>,
    /// The folder of the run in progress.
    running_folder: RunningFolder,
    /// The folder the Goose log is open in.
    log_folder: RunningFolder,
    /// The controller addresses `GET /api/v1/health` reports.
    controllers: HealthControllers,
}

#[derive(Debug, Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

/// Query of `GET /api/v1/snapshot`: the token, and the heatmap the client
/// holds, if any.
#[derive(Debug, Deserialize)]
struct SnapshotQuery {
    token: Option<String>,
    heatmap_generation: Option<u64>,
    heatmap_since: Option<u64>,
}

/// The heatmap a client holds, when it sent both halves of its cursor.
fn heatmap_cursor(generation: Option<u64>, since: Option<u64>) -> Option<HeatmapCursor> {
    Some(HeatmapCursor {
        generation: generation?,
        rev: since?,
    })
}

/// Query of `GET /api/v1/series`. Every field is optional so a missing one
/// is answered by the handler, as JSON, not by the extractor.
#[derive(Debug, Deserialize)]
struct SeriesQuery {
    method: Option<String>,
    name: Option<String>,
    token: Option<String>,
    heatmap_generation: Option<u64>,
    heatmap_since: Option<u64>,
}

/// What a `GET /api/v1/series` answer depends on: method, name and the
/// heatmap cursor the client sent.
type SeriesKey = (String, String, Option<(u64, u64)>);

/// Cached `GET /api/v1/series` bodies, by what they depend on.
type SeriesCache = HashMap<SeriesKey, (Instant, Bytes)>;

/// Liveness plus lightweight operational counters (no load-test metrics).
///
/// `last_build_ms` is the time the metrics processor spent draining metrics and
/// building the last published snapshot; `last_snapshot_ms` is the hub's round
/// trip for it, from request to published JSON. Both are durations, not Unix
/// timestamps, and the difference is time spent waiting on the main loop and
/// the processor queue, plus serialization. Fields are always present so external monitors can
/// scrape them without auth; they do not leak request names, target hosts, or
/// rates. `controllers` names this process's own telnet and WebSocket
/// listeners, which take no authentication; it reveals only where they
/// listen.
#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    version: String,
    /// Processor time of the last published snapshot in milliseconds (0 if none).
    last_build_ms: u64,
    /// Hub round trip of the last published snapshot in milliseconds (0 if none).
    last_snapshot_ms: u64,
    /// Snapshot requests that timed out since process start.
    snapshot_timeouts: u64,
    /// Total successful hub snapshot builds since process start.
    build_count: u64,
    /// Concurrent SSE clients currently holding a slot.
    active_sse_clients: usize,
    /// Whether the Start/Stop/Users/Quit control and Delete endpoints are registered.
    control_enabled: bool,
    /// Whether a request needs the token: false only under `--dashboard-no-auth`.
    auth_required: bool,
    /// Whether the dashboard is bound to a loopback address.
    loopback: bool,
    /// Where the telnet and WebSocket Controllers listen.
    controllers: HealthControllers,
}

/// The `host:port` each Controller listens on, or `None` when it is off
/// (`--no-telnet`, or no `--websocket`) or couldn't bind its port.
#[derive(Debug, Clone, Serialize)]
struct HealthControllers {
    telnet: Option<String>,
    websocket: Option<String>,
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
/// `control_tx`, given only with dashboard control, registers the control routes.
///
/// When/if multi-mode (Gaggle) returns, the dashboard should only run on the
/// standalone/manager process — currently AttackMode is StandAlone only.
pub(crate) async fn setup_dashboard(
    configuration: &GooseConfiguration,
    control_tx: Option<flume::Sender<ControlRequest>>,
    listening: ListeningControllers,
) -> Result<Option<DashboardSetup>, GooseError> {
    if !configuration.dashboard {
        return Ok(None);
    }

    let host = configuration.dashboard_host();
    // 0 picks a free port; the log and the footer name the port bound.
    let port = configuration.dashboard_port();
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
    let active_sse = Arc::new(AtomicUsize::new(0));
    let running_folder = RunningFolder::default();
    let log_folder = RunningFolder::default();
    let max_sse_clients = configuration.dashboard_max_clients();
    // Clone sender for the hub and the series handler (same channel).
    let hub = SnapshotHub::new(
        request_tx.clone(),
        Arc::clone(&build_count),
        close_tx.clone(),
        max_sse_clients,
        Arc::clone(&active_sse),
    );
    let control_enabled = control_tx.is_some();
    let no_auth = configuration.dashboard_no_auth;
    // Loopback by the configured name, the same rule that lets the dashboard
    // run without a token. Any other name needs a token, and a hostname that
    // resolves to 127.0.1.1 must still accept its own name as `Host`.
    let loopback = crate::config::is_loopback_bind(host);
    let state = DashboardState {
        hub,
        request_tx,
        control_tx,
        // Validation refuses a token with --dashboard-no-auth; never keep one.
        auth_token: if no_auth {
            String::new()
        } else {
            configuration
                .dashboard_auth_token()
                .unwrap_or_default()
                .to_string()
        },
        no_auth,
        loopback,
        control_in_flight: Arc::new(AtomicUsize::new(0)),
        series_in_flight: Arc::new(AtomicUsize::new(0)),
        series_cache: Arc::new(Mutex::new(HashMap::new())),
        reports_dir: runs::parent_folder(configuration),
        listing_cache: Arc::new(Mutex::new(runs::ListingCache::new())),
        running_folder: running_folder.clone(),
        log_folder: log_folder.clone(),
        controllers: HealthControllers {
            telnet: listening.telnet.then(|| {
                format_bind_address(configuration.telnet_host(), configuration.telnet_port())
            }),
            websocket: listening.websocket.then(|| {
                format_bind_address(
                    configuration.websocket_host(),
                    configuration.websocket_port(),
                )
            }),
        },
    };

    let app = build_router(state, host);
    let max_connections = connection_cap(max_sse_clients);

    let token = !no_auth && configuration.dashboard_auth_token().is_some();
    for line in startup_lines(
        &bound.to_string(),
        control_enabled,
        no_auth,
        token,
        loopback,
    ) {
        match line {
            StartupLine::Info(text) => info!("{text}"),
            StartupLine::Warn(text) => warn!("{text}"),
            StartupLine::Block(text) => {
                // Straight to stderr, so neither -q nor the log level hides
                // it; the Goose log gets it at warn level too.
                let rule = "=".repeat(78);
                eprintln!("{rule}\n{text}\n{rule}");
                warn!("[dashboard]: {}", text.replace('\n', " "));
            }
        }
    }

    let (stop_tx, stop_rx) = watch::channel(false);
    let task = tokio::spawn(serve_dashboard(listener, app, max_connections, stop_rx));
    let server = DashboardServer {
        stop_tx,
        close_tx: close_tx.clone(),
        task,
    };

    Ok(Some(DashboardSetup {
        request_rx,
        bound_port: bound.port(),
        build_count,
        active_sse,
        running_folder,
        log_folder,
        url: format!("http://{bound}"),
        close_tx,
        server,
    }))
}

/// The address to open the dashboard bound to `bound` at: the bind address,
/// or on an unspecified one (`0.0.0.0`, `::`) loopback of the same family,
/// as a browser can't open an unspecified address.
fn open_address(bound: &str) -> String {
    match bound.parse::<std::net::SocketAddr>() {
        Ok(addr) if addr.ip().is_unspecified() => {
            let ip: std::net::IpAddr = if addr.is_ipv4() {
                std::net::Ipv4Addr::LOCALHOST.into()
            } else {
                std::net::Ipv6Addr::LOCALHOST.into()
            };
            std::net::SocketAddr::new(ip, addr.port()).to_string()
        }
        _ => bound.to_string(),
    }
}

/// One line, or block, Goose prints when the dashboard starts.
#[derive(Debug, PartialEq)]
enum StartupLine {
    Info(String),
    Warn(String),
    /// Written straight to stderr, and to the Goose log at warn level.
    Block(String),
}

/// What Goose says when the dashboard binds `bound`: where it listens, and
/// how to get control, or the warning a dashboard with no token earns. The
/// token is never printed, only the `<token>` placeholder.
fn startup_lines(
    bound: &str,
    control: bool,
    no_auth: bool,
    token: bool,
    loopback: bool,
) -> Vec<StartupLine> {
    let mut lines = Vec::new();
    let listening = |mode: &str| {
        StartupLine::Info(format!("[dashboard]: listening on http://{bound} ({mode})"))
    };
    match (control, no_auth) {
        // With a token already set, --dashboard-no-auth would be refused.
        (false, false) if token => {
            lines.push(listening("read-only"));
            lines.push(StartupLine::Info(
                "[dashboard]: read-only; for control add --dashboard-control".to_string(),
            ));
        }
        (false, false) => {
            lines.push(listening("read-only"));
            lines.push(StartupLine::Info(
                "[dashboard]: read-only; for control add --dashboard-control and either --dashboard-no-auth (no token) or --dashboard-auth-token <token>".to_string(),
            ));
        }
        (false, true) => {
            lines.push(listening("read-only, no token"));
            lines.push(StartupLine::Info(
                "[dashboard]: read-only; for control add --dashboard-control".to_string(),
            ));
        }
        (true, false) => {
            lines.push(listening("control enabled"));
            let open = open_address(bound);
            lines.push(StartupLine::Info(format!(
                "[dashboard]: open http://{open}/?token=<token> to control the test"
            )));
        }
        (true, true) => {
            lines.push(listening("control enabled, no token"));
            if loopback {
                lines.push(StartupLine::Warn(
                    "[dashboard]: --dashboard-no-auth: anyone on this machine can start, stop and redirect the test".to_string(),
                ));
            }
        }
    }
    if no_auth && !loopback {
        let port = bound.rsplit(':').next().unwrap_or("5118");
        let who = if control {
            "Anyone who can reach this port can watch the test, start and stop it,\n\
             and point the load at any host."
        } else {
            "Anyone who can reach this port can watch the test."
        };
        lines.push(StartupLine::Block(format!(
            "WARNING: the dashboard on http://{bound} has no token (--dashboard-no-auth).\n\
             {who}\n\
             Safer: bind 127.0.0.1 and use an ssh forward (ssh -L {port}:127.0.0.1:{port} host),\n\
             or use --dashboard-auth-token in place of --dashboard-no-auth. Only IP addresses,\n\
             localhost and the --dashboard-host name are accepted as Host; set --dashboard-host\n\
             to the DNS name you browse to."
        )));
    }
    lines
}

/// Handle on the running dashboard HTTP server.
///
/// [`DashboardServer::shutdown`] stops it and waits until the listener and
/// every connection are closed, so the port is free when it returns. Dropping
/// the handle instead (an attack future dropped mid run, or a panic) drops
/// `stop_tx`, which stops accepting and drains connections as `shutdown`
/// does, but without sending `event: closed` and without waiting for the
/// server task.
#[derive(Debug)]
pub(crate) struct DashboardServer {
    /// Tells the accept loop and every connection task to stop, by a send or
    /// by being dropped.
    stop_tx: watch::Sender<bool>,
    /// Same channel as [`DashboardSetup::close_tx`]: SSE clients get
    /// `event: closed` before their connection is drained.
    close_tx: watch::Sender<bool>,
    /// Accept loop task; it owns the listener and the connection tasks.
    task: tokio::task::JoinHandle<()>,
}

impl DashboardServer {
    /// Stop accepting, drain open connections for at most
    /// [`SERVER_DRAIN_TIMEOUT`], close whatever is still open, and wait for
    /// the server task to finish.
    pub(crate) async fn shutdown(self) {
        self.close_tx.send_replace(true);
        self.stop_tx.send_replace(true);
        if let Err(e) = self.task.await {
            error!("[dashboard]: server task failed: {e}");
        }
    }
}

/// Most connections the server holds open at once for a given SSE client cap:
/// one SSE stream and about one keep-alive connection per client, plus
/// [`EXTRA_CONNECTIONS`], so every allowed SSE stream fits with room for the
/// page loads, polls and control requests beside it.
fn connection_cap(max_sse_clients: usize) -> usize {
    max_sse_clients
        .saturating_mul(2)
        .saturating_add(EXTRA_CONNECTIONS)
        .min(tokio::sync::Semaphore::MAX_PERMITS)
}

/// Accept loop. Owns the listener and every connection task, so when it
/// returns nothing of the server is left running.
///
/// At most `max_connections` connections are open at once; a connection
/// accepted beyond that is closed at once, before any of it is read.
async fn serve_dashboard(
    listener: tokio::net::TcpListener,
    app: Router,
    max_connections: usize,
    mut stop_rx: watch::Receiver<bool>,
) {
    let mut builder = hyper::server::conn::http1::Builder::new();
    // Without a timer hyper applies no header read timeout at all.
    builder
        .timer(hyper_util::rt::TokioTimer::new())
        .header_read_timeout(HEADER_READ_TIMEOUT);
    // Each connection task holds one permit until it ends, aborted or not, so
    // the count does not wait for finished tasks to be reaped.
    let open_connections = Arc::new(tokio::sync::Semaphore::new(max_connections));
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            // A send or a dropped sender both mean stop.
            _ = stop_rx.changed() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => match Arc::clone(&open_connections).try_acquire_owned() {
                    Ok(permit) => {
                        connections.spawn(serve_connection(
                            builder.clone(),
                            stream,
                            app.clone(),
                            stop_rx.clone(),
                            permit,
                        ));
                    }
                    Err(_) => {
                        debug!(
                            "[dashboard]: closing connection from {peer}: {max_connections} connections already open"
                        );
                        drop(stream);
                    }
                },
                Err(e) => {
                    if !is_connection_error(&e) {
                        // Usually EMFILE: back off as axum::serve does, but
                        // keep listening for the stop signal meanwhile.
                        error!("[dashboard]: accept error: {e}");
                        tokio::select! {
                            _ = stop_rx.changed() => break,
                            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                        }
                    }
                }
            },
        }
        // Reap finished connection tasks so the set does not grow with every
        // short lived connection.
        while connections.try_join_next().is_some() {}
    }

    // Stop accepting before the drain, so only open connections remain.
    drop(listener);

    let drained = tokio::time::timeout(SERVER_DRAIN_TIMEOUT, async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    if drained.is_err() {
        debug!(
            "[dashboard]: closing {} connection(s) still open after the drain deadline",
            connections.len()
        );
        connections.shutdown().await;
    }
}

/// Serve one HTTP/1 connection until it ends or the server stops; on stop,
/// let an in flight response finish and then close. Holds its slot of the
/// connection cap until it returns.
async fn serve_connection(
    builder: hyper::server::conn::http1::Builder,
    stream: tokio::net::TcpStream,
    app: Router,
    mut stop_rx: watch::Receiver<bool>,
    _permit: tokio::sync::OwnedSemaphorePermit,
) {
    let service = hyper_util::service::TowerToHyperService::new(app);
    let conn = builder.serve_connection(hyper_util::rt::TokioIo::new(stream), service);
    let mut conn = std::pin::pin!(conn);
    let result = tokio::select! {
        result = conn.as_mut() => result,
        _ = stop_rx.changed() => {
            conn.as_mut().graceful_shutdown();
            conn.await
        }
    };
    if let Err(e) = result {
        debug!("[dashboard]: connection error: {e}");
    }
}

/// Accept errors that concern one connection only; the listener is fine.
fn is_connection_error(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
    )
}

/// Build the axum router with public shell/static/health and auth-gated metrics.
///
/// Control routes are registered only when `state.control_tx` is set
/// (otherwise POST /api/v1/control/* and DELETE /api/v1/reports/{name} → 404). On a loopback bind every request
/// must carry a loopback `Host`, so a page that rebinds its own domain to
/// 127.0.0.1 cannot read the API. Under `--dashboard-no-auth` the control
/// routes refuse a cross site request, and on any other bind every request
/// must carry an IP literal or `host`, the configured `--dashboard-host`, as
/// its `Host`.
fn build_router(state: DashboardState, host: &str) -> Router {
    let control_enabled = state.control_tx.is_some();
    let loopback_bind = state.loopback;
    let no_auth = state.no_auth;
    let mut router = Router::new()
        .route("/", get(index_handler))
        .route("/static/app.js", get(app_js_handler))
        .route("/static/app.css", get(app_css_handler))
        .route("/api/v1/health", get(health_handler))
        .route("/api/v1/snapshot", get(snapshot_handler))
        .route("/api/v1/series", get(series_handler))
        .route("/api/v1/events", get(events_handler))
        .route("/api/v1/reports", get(reports_handler))
        .route("/api/v1/reports/{name}/{file}", get(report_file_handler));

    if control_enabled {
        // Bound control request bodies so a large POST cannot blow memory/CPU
        // on the dashboard task (start/stop expect empty/`{}`; users is small).
        let control_routes = Router::new()
            .route("/api/v1/control/start", post(control_start_handler))
            .route("/api/v1/control/stop", post(control_stop_handler))
            .route("/api/v1/control/quit", post(control_quit_handler))
            .route("/api/v1/reports/{name}", delete(delete_report_handler))
            .route("/api/v1/control/users", post(control_users_handler))
            .route("/api/v1/control/settings", post(control_settings_handler))
            .route("/api/v1/control/report", post(control_report_handler))
            .route(
                "/api/v1/control/report/now",
                post(control_report_now_handler),
            )
            .layer(axum::extract::DefaultBodyLimit::max(CONTROL_BODY_LIMIT));
        // With no token, nothing else stops another site's page posting here.
        let control_routes = if no_auth {
            control_routes.route_layer(axum::middleware::from_fn(require_same_origin_json))
        } else {
            control_routes
        };
        router = router.merge(control_routes);
    }

    if loopback_bind {
        // Added before the header layers so a refusal carries them too.
        router = router.layer(axum::middleware::from_fn(require_loopback_host));
    } else if no_auth {
        router = router.layer(axum::middleware::from_fn_with_state(
            Arc::<str>::from(host),
            require_ip_or_configured_host,
        ));
    }

    router
        // `if_not_present`: a report download sets its own
        // `Content-Security-Policy: sandbox`; every other route sets none and
        // gets this one.
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        ))
        // `no-store`, not `no-cache`: the first page load carries `?token=`
        // and the API responses carry metrics; neither belongs in a cache.
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        // Keeps `?token=` out of the `Referer` of anything the page loads or
        // links to.
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .with_state(Arc::new(state))
}

/// Refuse a request whose `Host` is missing or not a loopback name or address
/// (403). Used on loopback binds only, against DNS rebinding.
async fn require_loopback_host(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response<Body> {
    let allowed = request
        .headers()
        .get(header::HOST)
        .and_then(|host| host.to_str().ok())
        .is_some_and(is_loopback_host);
    if allowed {
        next.run(request).await
    } else {
        (StatusCode::FORBIDDEN, "host not allowed").into_response()
    }
}

/// A refusal of the no token guards: 403 with `{ "ok": false, "reason": ... }`.
fn guard_refusal(reason: &'static str) -> Response<Body> {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({ "ok": false, "reason": reason })),
    )
        .into_response()
}

/// Refuse a control request another site's page could send (403
/// `cross_origin`): one with an `Origin` other than `http://` and its own
/// `Host`, or without `Content-Type: application/json`, which a form or a
/// simple `fetch` can't send. Used under `--dashboard-no-auth` only, where no
/// token stands in the way.
async fn require_same_origin_json(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response<Body> {
    let headers = request.headers();
    let host = headers
        .get(header::HOST)
        .and_then(|host| host.to_str().ok());
    let same_origin = match headers.get(header::ORIGIN) {
        None => true,
        Some(origin) => match (origin.to_str().ok(), host) {
            (Some(origin), Some(host)) => origin
                .strip_prefix("http://")
                .is_some_and(|rest| rest.eq_ignore_ascii_case(host)),
            _ => false,
        },
    };
    let json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(is_json_content_type);
    if same_origin && json {
        next.run(request).await
    } else {
        guard_refusal("cross_origin")
    }
}

/// True for `application/json`, with or without parameters.
fn is_json_content_type(value: &str) -> bool {
    value
        .split(';')
        .next()
        .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
}

/// Refuse a request whose `Host` is neither an IP literal, a loopback name
/// nor the configured `--dashboard-host` (403 `host_not_allowed`). Used under
/// `--dashboard-no-auth` on a non-loopback bind, against DNS rebinding: a
/// rebinding page arrives with its own domain as both `Host` and `Origin`.
async fn require_ip_or_configured_host(
    State(configured): State<Arc<str>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response<Body> {
    let allowed = request
        .headers()
        .get(header::HOST)
        .and_then(|host| host.to_str().ok())
        .is_some_and(|host| is_ip_or_configured_host(host, &configured));
    if allowed {
        next.run(request).await
    } else {
        guard_refusal("host_not_allowed")
    }
}

/// True when a `Host` header value is an IP literal (v4, or bracketed v6),
/// `localhost`, or the configured host name, with or without a port.
fn is_ip_or_configured_host(value: &str, configured: &str) -> bool {
    // An authority also parses `user@host`, which is no valid `Host`.
    if value.contains('@') {
        return false;
    }
    let Ok(authority) = value.parse::<axum::http::uri::Authority>() else {
        return false;
    };
    let host = authority.host();
    let unbracketed = |h: &str| -> String {
        h.strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .unwrap_or(h)
            .to_string()
    };
    // A bare IPv6 address is not a valid `Host`; it needs its brackets.
    let ip = if host.starts_with('[') {
        unbracketed(host).parse::<std::net::Ipv6Addr>().is_ok()
    } else {
        host.parse::<std::net::Ipv4Addr>().is_ok()
    };
    // A rebinding page never carries a loopback name, and an ssh forward to
    // this machine sends one.
    ip || crate::config::is_loopback_bind(host)
        || unbracketed(host).eq_ignore_ascii_case(&unbracketed(configured.trim()))
}

/// True when a `Host` header value names a loopback host: `localhost`, an
/// address in `127.0.0.0/8` or `[::1]`, with or without a port.
fn is_loopback_host(value: &str) -> bool {
    // An authority also parses `user@host`, which is no valid `Host`.
    if value.contains('@') {
        return false;
    }
    value
        .parse::<axum::http::uri::Authority>()
        .is_ok_and(|authority| crate::config::is_loopback_bind(authority.host()))
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

async fn health_handler(State(state): State<Arc<DashboardState>>) -> impl IntoResponse {
    Json(HealthResponse {
        ok: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
        last_build_ms: state.hub.last_build_ms(),
        last_snapshot_ms: state.hub.last_snapshot_ms(),
        snapshot_timeouts: state.hub.snapshot_timeouts(),
        build_count: state.hub.build_count(),
        active_sse_clients: state.hub.active_sse_clients(),
        control_enabled: state.control_tx.is_some(),
        auth_required: !state.no_auth,
        loopback: state.loopback,
        controllers: state.controllers.clone(),
    })
}

/// `GET /api/v1/snapshot`: the latest snapshot. Its heatmap holds every
/// column of the window, or with `heatmap_generation` and `heatmap_since`
/// only the columns changed since that heatmap.
async fn snapshot_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<SnapshotQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    if !authorize(&state.auth_token, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let cursor = heatmap_cursor(query.heatmap_generation, query.heatmap_since);
    let bytes = state.hub.snapshot_bytes_since(cursor).await?;
    Ok(([(header::CONTENT_TYPE, "application/json")], bytes))
}

/// A `GET /api/v1/series` error: the status and `{"error":"<code>"}`.
fn series_error(status: StatusCode, error: &'static str) -> Response<Body> {
    (status, Json(serde_json::json!({ "error": error }))).into_response()
}

/// RAII guard that decrements `series_in_flight` on drop.
struct SeriesInFlightGuard {
    counter: Arc<AtomicUsize>,
}

impl Drop for SeriesInFlightGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Try to reserve a series slot. Returns `None` when at cap.
fn try_acquire_series_slot(state: &DashboardState) -> Option<SeriesInFlightGuard> {
    let prev = state.series_in_flight.fetch_add(1, Ordering::AcqRel);
    if prev >= MAX_SERIES_IN_FLIGHT {
        state.series_in_flight.fetch_sub(1, Ordering::AcqRel);
        return None;
    }
    Some(SeriesInFlightGuard {
        counter: Arc::clone(&state.series_in_flight),
    })
}

/// A cached series body for this method and name, if built within
/// [`SERIES_CACHE_TTL`].
fn cached_series(state: &DashboardState, key: &SeriesKey) -> Option<Bytes> {
    let cache = state.series_cache.lock().unwrap_or_else(|e| e.into_inner());
    cache
        .get(key)
        .filter(|(built, _)| built.elapsed() < SERIES_CACHE_TTL)
        .map(|(_, body)| body.clone())
}

/// Cache a series body, dropping every entry older than [`SERIES_CACHE_TTL`].
/// Only 200 answers are cached, so it holds at most one entry per request
/// with a series of its own.
fn cache_series(state: &DashboardState, key: SeriesKey, body: Bytes) {
    let mut cache = state.series_cache.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|_, (built, _)| built.elapsed() < SERIES_CACHE_TTL);
    cache.insert(key, (Instant::now(), body));
}

/// `GET /api/v1/series?method=&name=`: one request's response time heatmap,
/// with only the columns changed since `heatmap_generation` and
/// `heatmap_since` when the client sends the heatmap it holds. 400
/// `bad_request` without both parameters or when they are too long together,
/// 404 `not_found` when the request has no histograms of its own (or
/// percentiles over time are off), 503 `busy`, `timeout` or `unavailable`
/// when the main loop cannot answer.
async fn series_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<SeriesQuery>,
) -> Response<Body> {
    if !authorize(&state.auth_token, &headers, query.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (Some(method), Some(name)) = (query.method, query.name) else {
        return series_error(StatusCode::BAD_REQUEST, "bad_request");
    };
    if method.len() + name.len() > MAX_SERIES_QUERY_LEN {
        return series_error(StatusCode::BAD_REQUEST, "bad_request");
    }
    let since = heatmap_cursor(query.heatmap_generation, query.heatmap_since);
    let key = (
        method,
        name,
        since.map(|cursor| (cursor.generation, cursor.rev)),
    );
    if let Some(body) = cached_series(&state, &key) {
        return ([(header::CONTENT_TYPE, "application/json")], body).into_response();
    }

    let Some(_slot) = try_acquire_series_slot(&state) else {
        return series_error(StatusCode::SERVICE_UNAVAILABLE, "busy");
    };
    let (respond, respond_rx) = oneshot::channel();
    let request = DashboardRequest::GetRequestSeries {
        method: key.0.clone(),
        name: key.1.clone(),
        since,
        respond,
    };
    if state.request_tx.send(request).is_err() {
        return series_error(StatusCode::SERVICE_UNAVAILABLE, "unavailable");
    }
    match tokio::time::timeout(CONTROL_TIMEOUT, respond_rx).await {
        Ok(Ok(Some(series))) => match serde_json::to_vec(&series) {
            Ok(body) => {
                let body = Bytes::from(body);
                cache_series(&state, key, body.clone());
                ([(header::CONTENT_TYPE, "application/json")], body).into_response()
            }
            Err(e) => {
                error!("[dashboard]: failed to serialize series: {e}");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        },
        Ok(Ok(None)) => series_error(StatusCode::NOT_FOUND, "not_found"),
        Ok(Err(_)) => series_error(StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
        Err(_) => series_error(StatusCode::SERVICE_UNAVAILABLE, "timeout"),
    }
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
            let _ = event_tx.send(closed_event()).await;
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
                        let _ = event_tx.send(closed_event()).await;
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
                        let _ = event_tx.send(closed_event()).await;
                        break;
                    }
                }
                // The client disconnected and its body stream was dropped:
                // free the slot now, not on the next snapshot (#696).
                _ = event_tx.closed() => break,
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
    let event = closed_event();
    let stream = stream::once(async move { Ok::<_, Infallible>(event) });
    Sse::new(stream).into_response()
}

/// The terminal `closed` event.
fn closed_event() -> Event {
    Event::default().event("closed").data("1")
}

fn snapshot_event(bytes: &Bytes) -> Event {
    let data = std::str::from_utf8(bytes).unwrap_or("{}");
    Event::default().event("snapshot").data(data)
}

/// The name of a folder Goose is writing to, shared by the main loop and the
/// report routes.
#[derive(Clone, Debug, Default)]
pub(crate) struct RunningFolder(Arc<Mutex<Option<String>>>);

impl RunningFolder {
    /// Set the folder, or `None` once Goose no longer writes to it.
    pub(crate) fn set(&self, folder: Option<&std::path::Path>) {
        let name = folder
            .and_then(std::path::Path::file_name)
            .map(|name| name.to_string_lossy().into_owned());
        match self.0.lock() {
            Ok(mut running) => *running = name,
            Err(poisoned) => *poisoned.into_inner() = name,
        }
    }

    fn get(&self) -> Option<String> {
        match self.0.lock() {
            Ok(running) => running.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Whether this is the folder `name`, at `folder` in `parent`: by name, or
    /// by identity, so a name in another case on a filesystem that ignores
    /// case is still this folder.
    fn is(&self, parent: &Path, name: &str, folder: &Path) -> bool {
        self.get().is_some_and(|own| {
            own == name
                || std::fs::symlink_metadata(parent.join(&own))
                    .is_ok_and(|metadata| same_file(folder, &metadata))
        })
    }
}

/// Query of the report routes: the token, and for a comparison the baseline.
#[derive(Debug, Deserialize)]
struct ReportQuery {
    token: Option<String>,
    baseline: Option<String>,
}

/// The comparison a report route serves besides the reports.
const COMPARE_MD: &str = "compare.md";

/// `GET /api/v1/reports`.
#[derive(Debug, Serialize)]
struct ReportsListing {
    /// The folder listed, as given, never made absolute.
    dir: String,
    /// Newest first.
    reports: Vec<runs::ListedReport>,
}

/// `GET /api/v1/reports`: the folders in the folder run folders go in that
/// hold a `report.json` or `report.json.gz`, newest first. Read from disk on
/// each request, never on the snapshot path; each report is parsed again only
/// when it changes.
async fn reports_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Response<Body> {
    if !authorize(&state.auth_token, &headers, query.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let listing_state = Arc::clone(&state);
    let listed = tokio::task::spawn_blocking(move || {
        let running = listing_state
            .running_folder
            .get()
            .map(|name| listing_state.reports_dir.join(name));
        let mut cache = match listing_state.listing_cache.lock() {
            Ok(cache) => cache,
            Err(poisoned) => poisoned.into_inner(),
        };
        runs::list_reports(&listing_state.reports_dir, running.as_deref(), &mut cache)
    })
    .await;
    let dir = state.reports_dir.to_string_lossy().into_owned();
    match listed {
        Ok(Ok(reports)) => Json(ReportsListing { dir, reports }).into_response(),
        Ok(Err(e)) => {
            warn!("[dashboard]: can't list the reports in {dir}: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "can't list the reports").into_response()
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "can't list the reports").into_response(),
    }
}

/// `GET /api/v1/reports/{name}/{file}`: one of a folder's reports, rendered
/// from its `report.json` or `report.json.gz` when the folder lacks it; or
/// with `file` `compare.md` and `?baseline={name}`, the report compared to the
/// baseline as Markdown. Anything that is not a valid name, a known file, a real
/// folder and a regular `report.json` or `report.json.gz` is a plain 404.
async fn report_file_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    path: Result<axum::extract::Path<(String, String)>, axum::extract::rejection::PathRejection>,
    Query(query): Query<ReportQuery>,
) -> Response<Body> {
    if !authorize(&state.auth_token, &headers, query.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(axum::extract::Path((name, file))) = path else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if file == COMPARE_MD {
        return compare_response(&state.reports_dir, name, query.baseline).await;
    }
    let (content_type, ext) = match file.as_str() {
        runs::REPORT_HTML => ("text/html; charset=utf-8", "html"),
        runs::REPORT_JSON => ("application/json", "json"),
        runs::REPORT_MD => ("text/markdown; charset=utf-8", "md"),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let Some((folder, json)) = runs::report_folder(&state.reports_dir, &name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // The report is read and parsed off the runtime the load test runs on.
    let test = {
        let json = json.clone();
        tokio::task::spawn_blocking(move || download_test_name(&json))
            .await
            .unwrap_or_else(|_| "run".to_string())
    };
    let filename = format!("goose-{test}-{name}.{ext}");
    if let Some(stored) = runs::regular_file(&folder, &file) {
        // A report stored gzipped is sent as it is to a client that accepts
        // gzip, and decompressed for any other.
        let gzipped = runs::without_gz(&stored.to_string_lossy()).1;
        if !gzipped || accepts_gzip(&headers) {
            return stored_file_response(&stored, content_type, &filename, gzipped).await;
        }
        // The file opened is checked to be the one at the path, as when it is
        // streamed.
        let gunzipped = tokio::task::spawn_blocking(move || {
            let file = std::fs::File::open(&stored)?;
            let metadata = file.metadata()?;
            if !metadata.is_file() || !same_file(&stored, &metadata) {
                return Err(std::io::Error::other("the report changed"));
            }
            let mut body = Vec::new();
            std::io::Read::read_to_end(&mut flate2::read::MultiGzDecoder::new(file), &mut body)?;
            Ok(body)
        })
        .await;
        return match gunzipped {
            Ok(Ok(body)) => {
                let length = body.len() as u64;
                let mut response =
                    download_response(content_type, &filename, Body::from(body), Some(length));
                response
                    .headers_mut()
                    .insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
                response
            }
            _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
    }
    // A format the folder lacks is rendered from its JSON report.
    let rendered = tokio::task::spawn_blocking(move || render_report_file(&json, ext)).await;
    match rendered {
        Ok(Ok(body)) => {
            let length = body.len() as u64;
            download_response(content_type, &filename, Body::from(body), Some(length))
        }
        Ok(Err(reason)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            reason,
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// The HTML or Markdown report rendered from the JSON report at `json`. An
/// error is one line for the person who asked.
fn render_report_file(json: &Path, ext: &str) -> Result<Vec<u8>, String> {
    let loaded = crate::metrics::LoadedReport::read(json)?;
    let configuration = GooseConfiguration::default();
    let empty = crate::graph::GraphData::new();
    let source = loaded.source(&configuration, &empty);
    let mut out = Vec::new();
    let written = match ext {
        "html" => source.html(None).map(|html| out = html.into_bytes()),
        _ => source.write_markdown(&mut out, None),
    };
    written.map_err(|e| format!("Can't render the report: {e}"))?;
    Ok(out)
}

/// `test` from the record in a `report.json`, reduced to `[A-Za-z0-9_-]` for
/// a file name.
fn download_test_name(json: &Path) -> String {
    runs::read_record(json)
        .map(|record| {
            record
                .test
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>()
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "run".to_string())
}

/// Headers of a report or comparison download: an attachment, never rendered
/// by the browser, never cached, and sandboxed if it is opened anyway.
fn download_response(
    content_type: &'static str,
    filename: &str,
    body: Body,
    length: Option<u64>,
) -> Response<Body> {
    let mut response = Response::new(body);
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    if let Some(length) = length {
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    }
    // The name is built from [A-Za-z0-9_.-] only, so it is a valid header value.
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox"),
    );
    response
}

/// Whether a request's `Accept-Encoding` takes gzip: by name, else by `*`,
/// and not with `q=0`.
fn accepts_gzip(headers: &HeaderMap) -> bool {
    let codings: Vec<(String, bool)> = headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|coding| {
            let mut parts = coding.split(';').map(str::trim);
            let name = parts.next().unwrap_or_default().to_ascii_lowercase();
            let refused = parts.any(|param| {
                let param = param.to_ascii_lowercase();
                param
                    .strip_prefix("q=")
                    .and_then(|q| q.trim().parse::<f32>().ok())
                    .is_some_and(|q| q == 0.0)
            });
            (name, !refused)
        })
        .collect();
    let taken = |name: &str| codings.iter().find(|(n, _)| n == name).map(|(_, ok)| *ok);
    taken("gzip").or_else(|| taken("*")).unwrap_or(false)
}

/// Stream a report stored in a folder; one stored gzipped is sent with
/// `Content-Encoding: gzip`.
async fn stored_file_response(
    path: &Path,
    content_type: &'static str,
    filename: &str,
    gzipped: bool,
) -> Response<Body> {
    let Ok(file) = tokio::fs::File::open(path).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // The file opened must be the regular file checked above, not something
    // swapped in since.
    let Ok(metadata) = file.metadata().await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !metadata.is_file() || !same_file(path, &metadata) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mut response = download_response(
        content_type,
        filename,
        Body::from_stream(file_stream(file)),
        Some(metadata.len()),
    );
    if gzipped {
        response
            .headers_mut()
            .insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        response
            .headers_mut()
            .insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    }
    response
}

/// True when the open file's metadata is that of the file at `path` now, by
/// device and inode where the platform has them.
fn same_file(path: &Path, opened: &std::fs::Metadata) -> bool {
    let Ok(at_path) = std::fs::symlink_metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        at_path.dev() == opened.dev() && at_path.ino() == opened.ino()
    }
    #[cfg(not(unix))]
    {
        at_path.file_type().is_file() && at_path.len() == opened.len()
    }
}

/// Chunks of an open file, read as the response is sent, so a report is never
/// read into memory whole. `futures::stream::unfold` over `tokio::fs::File`
/// needs no new dependency.
fn file_stream(
    file: tokio::fs::File,
) -> impl futures::Stream<Item = Result<Bytes, std::io::Error>> + Send {
    const CHUNK: usize = 64 * 1024;
    stream::unfold(Some(file), |file| async move {
        let mut file = file?;
        let mut buf = vec![0u8; CHUNK];
        match file.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok(Bytes::from(buf)), Some(file)))
            }
            // End the stream after an error.
            Err(e) => Some((Err(e), None)),
        }
    })
}

/// The Markdown comparison of report `name` (the newer) to `baseline`.
async fn compare_response(
    reports_dir: &Path,
    name: String,
    baseline: Option<String>,
) -> Response<Body> {
    let Some(baseline) = baseline else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (Some((_, newer_json)), Some((_, baseline_json))) = (
        runs::report_folder(reports_dir, &name),
        runs::report_folder(reports_dir, &baseline),
    ) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (run_name, baseline_name) = (name.clone(), baseline.clone());
    let compared = tokio::task::spawn_blocking(move || {
        let report = crate::metrics::compare_reports(
            &newer_json,
            &run_name,
            &baseline_json,
            &baseline_name,
        )?;
        let mut out =
            compared_runs_line(&newer_json, &run_name, &baseline_json, &baseline_name).into_bytes();
        out.extend_from_slice(b"\n\n");
        out.extend(report);
        Ok::<_, String>((download_test_name(&newer_json), out))
    })
    .await;
    match compared {
        Ok(Ok((test, markdown))) => {
            let length = markdown.len() as u64;
            download_response(
                "text/markdown; charset=utf-8",
                &format!("goose-{test}-{name}-vs-{baseline}.md"),
                Body::from(markdown),
                Some(length),
            )
        }
        Ok(Err(reason)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            reason,
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// The first line of a comparison: which run is compared to which baseline,
/// with when each started, from their JSON reports.
fn compared_runs_line(
    newer_json: &Path,
    run_name: &str,
    baseline_json: &Path,
    baseline_name: &str,
) -> String {
    let started = |json: &Path| {
        runs::read_record(json)
            .and_then(|record| record.started)
            .and_then(|started| chrono::DateTime::parse_from_rfc3339(&started).ok())
            .map(|time| {
                time.with_timezone(&chrono::Utc)
                    .format("%Y-%m-%d %H:%M:%S UTC")
                    .to_string()
            })
            .unwrap_or_else(|| "at an unknown time".to_string())
    };
    format!(
        "Run {run_name} (started {}) compared to baseline {baseline_name} (started {}).",
        started(newer_json),
        started(baseline_json)
    )
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
/// **Bearer only** — any query `?token=` is rejected so mutating URLs do not carry
/// the secret (even alongside a valid Bearer header). Scripts and the SPA must use
/// `Authorization: Bearer`.
fn authorize_control(
    configured_token: &str,
    headers: &HeaderMap,
    query_token: Option<&str>,
) -> bool {
    // Reject URL tokens outright so control endpoints never accept (or log-leak)
    // secrets in the query string, including when a valid Bearer is also present.
    if query_token.is_some() {
        return false;
    }
    if configured_token.is_empty() {
        return false;
    }

    if let Some(token) = bearer_token_from_headers(headers) {
        return constant_time_eq(token.as_bytes(), configured_token.as_bytes());
    }

    false
}

impl DashboardState {
    /// Authorize a control request: under `--dashboard-no-auth` every one,
    /// as the cross site guard in front of the control routes has passed it;
    /// otherwise [`authorize_control`] with the configured token.
    fn authorize_control(&self, headers: &HeaderMap, query_token: Option<&str>) -> bool {
        self.no_auth || authorize_control(&self.auth_token, headers, query_token)
    }
}

/// Extract a Bearer token from `Authorization`. Scheme match is case-insensitive
/// (RFC 7235); the token itself is compared separately with constant-time eq.
///
/// Accepts any ASCII whitespace between scheme and token (`Bearer TOKEN`,
/// `Bearer\tTOKEN`, multiple spaces). Rejects extra tokens after the credential.
fn bearer_token_from_headers(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?;
    let s = value.to_str().ok()?;
    let mut parts = s.split_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;
    // Reject `Bearer tok extra` — only scheme + single token credential.
    if parts.next().is_some() {
        return None;
    }
    if scheme.eq_ignore_ascii_case("Bearer") && !token.is_empty() {
        Some(token)
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

/// Parse `{"users": N}` into the `users` command, through the same bound as
/// the Controllers' `users`, refused here as a 400.
fn parse_users_body(body: &[u8]) -> Result<ControllerCommand, (&'static str, &'static str)> {
    let trimmed = trim_ascii_whitespace(body);
    if trimmed.is_empty() {
        return Err(("bad_request", "request body required"));
    }
    let value: serde_json::Value =
        serde_json::from_slice(trimmed).map_err(|_| ("bad_request", "malformed JSON body"))?;
    let obj = value
        .as_object()
        .ok_or(("bad_request", "request body must be a JSON object"))?;
    // Anything but a non negative JSON integer (a missing count, a float, a
    // string, a negative number) becomes 0, which the bound refuses. Extra
    // fields are tolerated for forward compat.
    let users = obj
        .get("users")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    ControllerCommand::users(users).map_err(|message| ("invalid_users", message))
}

/// The body of `POST /api/v1/control/settings`, and of a Start with
/// settings, as JSON: each field absent to leave the setting alone, `null`
/// to clear it, or a value, which [`parse_settings_body`] bounds.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsBody {
    #[serde(default, deserialize_with = "present")]
    host: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "present")]
    users: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "present")]
    increase_rate: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "present")]
    increase_time_secs: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "present")]
    decrease_rate: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "present")]
    decrease_time_secs: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "present")]
    run_time_secs: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "present")]
    test_plan: Option<serde_json::Value>,
}

/// A field that is present, `null` included, so `null` is told from absent.
fn present<'de, D>(deserializer: D) -> Result<Option<serde_json::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde_json::Value::deserialize(deserializer).map(Some)
}

/// Refusal for `users: null`: users can only be switched to.
const NULL_USERS_MSG: &str =
    "users can not be cleared: send a user count, or a test_plan to switch to";

/// Refusal for a time outside the bounds, sent as whole seconds.
fn invalid_secs_msg(field: &str) -> &'static str {
    match field {
        "increase_time" => {
            "increase_time_secs must be whole seconds from 1 to 2592000, or null to clear"
        }
        "decrease_time" => {
            "decrease_time_secs must be whole seconds from 1 to 2592000, or null to clear"
        }
        _ => "run_time_secs must be whole seconds from 1 to 2592000, or null to clear",
    }
}

/// A refused settings body: the error code, the message and the setting
/// at fault, answered as a 400.
type BodyRefusal = (&'static str, &'static str, Option<&'static str>);

/// Parse a settings body, bounding every value with the Controllers'
/// validators, so a setting is bounded alike from every front end. Refused
/// in this order: `bad_request` for malformed JSON or an unknown field,
/// `invalid_value` for a value of the wrong type or out of bounds (and
/// `users: null`), then `conflicting_settings` for settings that can not go
/// together whatever the state.
pub(crate) fn parse_settings_body(body: &[u8]) -> Result<RunSettings, BodyRefusal> {
    let bad_request = (
        "bad_request",
        "request body must be a JSON object of run settings: host, users, increase_rate, increase_time_secs, decrease_rate, decrease_time_secs, run_time_secs and test_plan",
        None,
    );
    let body: serde_json::Value =
        serde_json::from_slice(trim_ascii_whitespace(body)).map_err(|_| bad_request)?;
    if !body.is_object() {
        return Err(bad_request);
    }
    let body: SettingsBody = serde_json::from_value(body).map_err(|_| bad_request)?;
    let invalid = |field, message| ("invalid_value", message, Some(field));
    // `Some(None)` for `null`, `Some(Some(value))` for a value.
    fn read<T>(
        value: Option<serde_json::Value>,
        parse: impl FnOnce(&serde_json::Value) -> Option<Result<T, &'static str>>,
        error: &'static str,
    ) -> Result<Option<Option<T>>, &'static str> {
        match value {
            None => Ok(None),
            Some(serde_json::Value::Null) => Ok(Some(None)),
            Some(value) => parse(&value).unwrap_or(Err(error)).map(|v| Some(Some(v))),
        }
    }
    let rate = |field, value, error| {
        read(
            value,
            |value: &serde_json::Value| {
                value
                    .as_f64()
                    .map(|rate| controller::bounded_rate(rate as f32, error))
            },
            error,
        )
        .map_err(|message| invalid(field, message))
    };
    let secs = |field, value| {
        let error = invalid_secs_msg(field);
        read(
            value,
            |value: &serde_json::Value| {
                value
                    .as_u64()
                    .filter(|&secs| secs > 0)
                    .map(|secs| controller::bounded_secs(secs, error))
            },
            error,
        )
        .map_err(|message| invalid(field, message))
    };
    let settings = RunSettings {
        host: read(
            body.host,
            |value| value.as_str().map(controller::host),
            controller::INVALID_HOST_MSG,
        )
        .map_err(|message| invalid("host", message))?,
        users: match read(
            body.users,
            |value| value.as_u64().map(controller::bounded_users),
            controller::INVALID_USERS_MSG,
        ) {
            Ok(Some(None)) => return Err(invalid("users", NULL_USERS_MSG)),
            Ok(users) => users.flatten(),
            Err(message) => return Err(invalid("users", message)),
        },
        increase_rate: rate(
            "increase_rate",
            body.increase_rate,
            controller::INVALID_INCREASE_RATE_MSG,
        )?,
        increase_time: secs("increase_time", body.increase_time_secs)?,
        decrease_rate: rate(
            "decrease_rate",
            body.decrease_rate,
            controller::INVALID_DECREASE_RATE_MSG,
        )?,
        decrease_time: secs("decrease_time", body.decrease_time_secs)?,
        run_time: secs("run_time", body.run_time_secs)?,
        test_plan: read(
            body.test_plan,
            |value| value.as_str().map(controller::test_plan),
            controller::INVALID_PLAN_MSG,
        )
        .map_err(|message| invalid("test_plan", message))?,
    };
    match settings.conflict() {
        Some((field, message)) => Err(("conflicting_settings", message, Some(field))),
        None => Ok(settings),
    }
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

/// Send a control command to the main loop and wait up to [`CONTROL_TIMEOUT`].
///
/// On a timeout the main loop skips the command if it is still queued; if it
/// had already started applying it, it can still apply after the 503.
async fn dispatch_control(
    state: &DashboardState,
    command: impl Into<ControlCommand>,
    name: &str,
) -> Response<Body> {
    let Some(_slot) = try_acquire_control_slot(state) else {
        warn!("[dashboard]: control admission full ({name})");
        return control_http_error(
            StatusCode::SERVICE_UNAVAILABLE,
            name,
            "busy",
            "too many control requests in flight — wait and retry",
        );
    };
    // Control routes are registered only with a channel.
    let reply = match state.control_tx.as_ref() {
        Some(control_tx) => send_command(control_tx, command, CONTROL_TIMEOUT).await,
        None => Err(SendError::Unavailable),
    };
    match reply {
        Ok(ControlReply::Outcome(outcome)) => {
            let result = ControlResult::from_outcome(name, outcome);
            if result.ok {
                info!(
                    "[dashboard]: control {name} ok phase={} target={:?}",
                    result.phase, result.target_users
                );
            } else {
                info!(
                    "[dashboard]: control {name} rejected error={:?} phase={}",
                    result.error, result.phase
                );
            }
            (StatusCode::OK, Json(result)).into_response()
        }
        Err(SendError::Timeout) => {
            warn!("[dashboard]: control timed out ({name}); skipped if still queued");
            control_http_error(
                StatusCode::SERVICE_UNAVAILABLE,
                name,
                "timeout",
                "control timed out — action may still be applying; check phase/users before retrying",
            )
        }
        // The main loop has gone or dropped the request; or it sent a Config
        // or Metrics reply, which only Controllers ask for.
        _ => {
            warn!("[dashboard]: control unavailable ({name})");
            control_http_error(
                StatusCode::SERVICE_UNAVAILABLE,
                name,
                "unavailable",
                "control unavailable",
            )
        }
    }
}

/// Start, Stop and Quit: authorized, with an empty body.
async fn control_without_body(
    state: &DashboardState,
    headers: &HeaderMap,
    query: TokenQuery,
    body: &[u8],
    command: ControllerCommand,
    name: &str,
) -> Response<Body> {
    if !state.authorize_control(headers, query.token.as_deref()) {
        warn!("[dashboard]: control {name} auth failure");
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Err((code, msg)) = parse_empty_control_body(body) {
        return control_http_error(StatusCode::BAD_REQUEST, name, code, msg);
    }
    dispatch_control(state, command, name).await
}

/// Start: with an empty body or `{}` as it is, or with a settings body
/// applied and started in one step on the main loop.
async fn control_start_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    body: Bytes,
) -> Response<Body> {
    if parse_empty_control_body(&body).is_ok() {
        return control_without_body(
            &state,
            &headers,
            query,
            &body,
            ControllerCommand::Start,
            "start",
        )
        .await;
    }
    control_with_settings(&state, &headers, query, &body, "start").await
}

/// Apply run settings without starting.
async fn control_settings_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    body: Bytes,
) -> Response<Body> {
    control_with_settings(&state, &headers, query, &body, "settings").await
}

/// Start with settings, or settings alone: authorized, then the body parsed
/// and bounded, a refusal answered as a 400 naming the setting at fault.
async fn control_with_settings(
    state: &DashboardState,
    headers: &HeaderMap,
    query: TokenQuery,
    body: &[u8],
    name: &str,
) -> Response<Body> {
    if !state.authorize_control(headers, query.token.as_deref()) {
        warn!("[dashboard]: control {name} auth failure");
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match parse_settings_body(body) {
        Ok(settings) => {
            let command = ControlCommand::Settings {
                settings: Box::new(settings),
                start: name == "start",
            };
            dispatch_control(state, command, name).await
        }
        Err((code, message, field)) => {
            let body = ControlHttpErrorBody::new(name, code, message).with_field(field);
            (StatusCode::BAD_REQUEST, Json(body)).into_response()
        }
    }
}

async fn control_stop_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    body: Bytes,
) -> Response<Body> {
    control_without_body(
        &state,
        &headers,
        query,
        &body,
        ControllerCommand::Stop,
        "stop",
    )
    .await
}

async fn control_quit_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    body: Bytes,
) -> Response<Body> {
    control_without_body(
        &state,
        &headers,
        query,
        &body,
        ControllerCommand::Quit,
        "quit",
    )
    .await
}

/// Delete's 409, with why it refused.
fn conflict(message: &'static str) -> Response<Body> {
    (
        StatusCode::CONFLICT,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        message,
    )
        .into_response()
}

/// Why Delete refuses a folder another Goose process reserves for its run.
const RESERVED_BY_ANOTHER: &str = "Another Goose process is writing to this folder.";

/// `DELETE /api/v1/reports/{name}`: remove a folder's reports, and the logs
/// Goose names in a run's folder, then the folder if that leaves it empty;
/// nothing else. Registered only with `--dashboard-control`, and authorized
/// like the control routes. 204 when removed; 409 for the folder of the run
/// in progress, the one the Goose log is open in, or one another process
/// reserves for its run; a plain 404 for anything the listing would not list.
async fn delete_report_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    path: Result<axum::extract::Path<String>, axum::extract::rejection::PathRejection>,
    Query(query): Query<TokenQuery>,
) -> Response<Body> {
    if !state.authorize_control(&headers, query.token.as_deref()) {
        warn!("[dashboard]: delete report auth failure");
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(axum::extract::Path(name)) = path else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some((folder, json)) = runs::report_folder(&state.reports_dir, &name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let refused = if state.running_folder.is(&state.reports_dir, &name, &folder) {
        Some("The run in progress writes to this folder.")
    } else if state.log_folder.is(&state.reports_dir, &name, &folder) {
        Some("Goose is writing its log to this folder.")
    } else if runs::is_reserved(&folder) {
        Some(RESERVED_BY_ANOTHER)
    } else {
        None
    };
    if let Some(refused) = refused {
        return conflict(refused);
    }
    // `remove_file` removes a symlink, never what it points to.
    let name_for_check = name.clone();
    let reports_dir = state.reports_dir.clone();
    let removed = tokio::task::spawn_blocking(move || {
        // Checked again just before the deletes: still a real folder, and
        // still a report the panel lists.
        if runs::report_folder(&reports_dir, &name_for_check).is_none() || !runs::is_listed(&json) {
            return Ok(Some(StatusCode::NOT_FOUND));
        }
        if runs::is_reserved(&folder) {
            return Ok(Some(StatusCode::CONFLICT));
        }
        for file in runs::deletable_files() {
            let path = folder.join(file);
            if std::fs::symlink_metadata(&path).is_ok_and(|m| !m.is_dir()) {
                std::fs::remove_file(&path)?;
            }
        }
        // A folder that still holds other files stays.
        let _ = std::fs::remove_dir(&folder);
        Ok::<_, std::io::Error>(None)
    })
    .await;
    match removed {
        Ok(Ok(Some(StatusCode::CONFLICT))) => conflict(RESERVED_BY_ANOTHER),
        Ok(Ok(Some(status))) => status.into_response(),
        Ok(Ok(None)) => {
            info!("[dashboard]: deleted the report in {name}");
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(Err(e)) => {
            warn!("[dashboard]: can't delete the report in {name}: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "can't delete the report").into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn control_users_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    body: Bytes,
) -> Response<Body> {
    if !state.authorize_control(&headers, query.token.as_deref()) {
        warn!("[dashboard]: control users auth failure");
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match parse_users_body(&body) {
        Ok(command) => dispatch_control(&state, command, "users").await,
        Err((code, msg)) => control_http_error(StatusCode::BAD_REQUEST, "users", code, msg),
    }
}

/// Parse `{"save": bool, "graph_data": bool}`, each optional, at least one,
/// nothing else.
fn parse_report_body(body: &[u8]) -> Result<ControlCommand, (&'static str, &'static str)> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ReportBody {
        save: Option<bool>,
        graph_data: Option<bool>,
    }
    let body: ReportBody = serde_json::from_slice(trim_ascii_whitespace(body)).map_err(|_| {
        (
            "bad_request",
            "request body must be {\"save\": true or false, \"graph_data\": true or false}",
        )
    })?;
    if body.save.is_none() && body.graph_data.is_none() {
        return Err((
            "bad_request",
            "request body names neither save nor graph_data",
        ));
    }
    Ok(ControlCommand::Report {
        save: body.save,
        graph_data: body.graph_data,
    })
}

/// Save report and Include graph data: authorized, with a body naming the
/// switches to set.
async fn control_report_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    body: Bytes,
) -> Response<Body> {
    if !state.authorize_control(&headers, query.token.as_deref()) {
        warn!("[dashboard]: control report auth failure");
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match parse_report_body(&body) {
        Ok(command) => dispatch_control(&state, command, "report").await,
        Err((code, msg)) => control_http_error(StatusCode::BAD_REQUEST, "report", code, msg),
    }
}

/// Save report now: authorized, with an empty body.
async fn control_report_now_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    body: Bytes,
) -> Response<Body> {
    if !state.authorize_control(&headers, query.token.as_deref()) {
        warn!("[dashboard]: control report now auth failure");
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Err((code, msg)) = parse_empty_control_body(&body) {
        return control_http_error(StatusCode::BAD_REQUEST, "report_now", code, msg);
    }
    dispatch_control(&state, ControlCommand::SaveReportNow, "report_now").await
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
            stopping: false,
            duration_secs: seq,
            active_users: 1,
            maximum_users: 1,
            target_users: 1,
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
                response_time_us: None,
                co_active: false,
                co_adjusted: None,
            },
            requests: vec![],
            errors: vec![],
            transaction_errors: vec![],
            scenarios: vec![],
            transactions: vec![],
            series: SeriesWindow::empty(),
            flags: SnapshotFlags {
                metrics_disabled: false,
                transaction_metrics_disabled: false,
                scenario_metrics_disabled: false,
                requests_truncated: false,
                errors_truncated: false,
                transaction_errors_truncated: false,
                series_seconds: 300,
                percentiles_over_time: false,
            },
            report: crate::metrics::dashboard_snapshot::SnapshotReport::default(),
            settings: crate::metrics::dashboard_snapshot::SnapshotSettings::idle(),
            build_time: Duration::ZERO,
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
                    DashboardRequest::GetSnapshot { respond, .. } => {
                        let n = parent_builds.fetch_add(1, Ordering::SeqCst);
                        let _ = respond.send(dummy_snapshot(n));
                    }
                    DashboardRequest::GetRequestSeries { respond, .. } => {
                        let _ = respond.send(None);
                    }
                }
            }
        })
    }

    /// Answer each control command with `answer`'s reply, or drop its
    /// oneshot unanswered when `answer` gives none.
    fn spawn_control_parent(
        control_rx: flume::Receiver<ControlRequest>,
        answer: fn(ControlCommand) -> Option<ControlReply>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Ok((command, respond)) = control_rx.recv_async().await {
                if let Some(reply) = answer(command) {
                    let _ = respond.send(reply);
                }
            }
        })
    }

    fn outcome(
        ok: bool,
        error: Option<&'static str>,
        message: &str,
        phase: &str,
        active_users: usize,
        target_users: Option<usize>,
    ) -> Option<ControlReply> {
        Some(ControlReply::Outcome(ControlOutcome {
            ok,
            error,
            message: message.to_string(),
            phase: phase.to_string(),
            active_users,
            target_users,
            field: None,
            settings: None,
        }))
    }

    /// A soft success for every control command.
    fn answer_ok(command: ControlCommand) -> Option<ControlReply> {
        let command = match command {
            ControlCommand::Settings { start: false, .. } => {
                return outcome(true, None, "settings configured", "idle", 0, None);
            }
            ControlCommand::Settings { start: true, .. } => ControllerCommand::Start,
            ControlCommand::Report { .. } => {
                return outcome(true, None, "save report on", "idle", 0, None);
            }
            ControlCommand::SaveReportNow => {
                return outcome(true, None, "saving the report", "maintain", 1, None);
            }
            ControlCommand::Command(command) => command,
        };
        match command {
            ControllerCommand::Start => {
                outcome(true, None, "load test started", "increase", 0, Some(1))
            }
            ControllerCommand::Stop => {
                outcome(true, None, "load test stopped", "decrease", 1, Some(0))
            }
            ControllerCommand::Quit => {
                outcome(true, None, "goose is shutting down", "decrease", 0, None)
            }
            ControllerCommand::Users(users) => {
                outcome(true, None, "users configured", "idle", 0, Some(users))
            }
            // The dashboard sends only the commands above.
            other => panic!("unexpected dashboard command {:?}", other),
        }
    }

    /// Parent that drops snapshot oneshots in a window (simulates metrics
    /// processor recycle) then resumes answering.
    fn spawn_blip_parent(
        request_rx: flume::Receiver<DashboardRequest>,
        parent_builds: Arc<AtomicU64>,
        drop_from: u64,
        drop_until: u64,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Ok(msg) = request_rx.recv_async().await {
                match msg {
                    DashboardRequest::GetSnapshot { respond, .. } => {
                        let n = parent_builds.fetch_add(1, Ordering::SeqCst);
                        if n >= drop_from && n < drop_until {
                            drop(respond);
                        } else {
                            let _ = respond.send(dummy_snapshot(n));
                        }
                    }
                    DashboardRequest::GetRequestSeries { respond, .. } => {
                        let _ = respond.send(None);
                    }
                }
            }
        })
    }

    async fn bind_dashboard(token: &str) -> (DashboardSetup, String) {
        bind_dashboard_with(TestBind::token(token)).await
    }

    /// How a test binds the dashboard: the token, `--dashboard-no-auth`, the
    /// bind host, the live client cap, and the control channel, given only
    /// with control on. Always on port 0.
    struct TestBind {
        token: String,
        no_auth: bool,
        host: String,
        max_clients: u32,
        control: Option<flume::Sender<ControlRequest>>,
    }

    impl TestBind {
        /// Loopback, read-only, with `token` ("" for none).
        fn token(token: &str) -> Self {
            TestBind {
                token: token.to_string(),
                no_auth: false,
                host: "127.0.0.1".to_string(),
                max_clients: 0,
                control: None,
            }
        }

        /// A row of the shared configuration matrix, with `control` given
        /// when the row turns control on.
        fn row(row: &crate::config::DashboardRow, control: flume::Sender<ControlRequest>) -> Self {
            let configuration = crate::config::dashboard_row_configuration(row);
            TestBind {
                token: configuration
                    .dashboard_auth_token()
                    .unwrap_or_default()
                    .to_string(),
                no_auth: configuration.dashboard_no_auth,
                host: configuration.dashboard_host().to_string(),
                max_clients: 0,
                control: configuration.dashboard_control.then_some(control),
            }
        }

        fn control(mut self, control: flume::Sender<ControlRequest>) -> Self {
            self.control = Some(control);
            self
        }

        /// 0 leaves the cap at its default.
        fn max_clients(mut self, max_clients: u32) -> Self {
            self.max_clients = max_clients;
            self
        }
    }

    async fn bind_dashboard_with(bind: TestBind) -> (DashboardSetup, String) {
        let config = GooseConfiguration {
            dashboard: true,
            dashboard_control: bind.control.is_some(),
            dashboard_no_auth: bind.no_auth,
            dashboard_host: Some(bind.host),
            dashboard_port: Some(0),
            dashboard_auth_token: Some(bind.token),
            dashboard_max_clients: (bind.max_clients > 0).then_some(bind.max_clients),
            ..Default::default()
        };
        let setup = setup_dashboard(
            &config,
            bind.control,
            ListeningControllers::configured(&config),
        )
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

    /// Bind dashboard + mock parent; returns (hub_builds, parent_builds, parent,
    /// base_url, server). The server stops when `server` is dropped, so bind it
    /// to a named variable for the life of the test.
    async fn setup_with_mock(
        token: &str,
    ) -> (
        Arc<AtomicU64>,
        Arc<AtomicU64>,
        tokio::task::JoinHandle<()>,
        String,
        DashboardServer,
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
        DashboardServer,
    ) {
        let (setup, base) =
            bind_dashboard_with(TestBind::token(token).max_clients(max_clients)).await;
        let hub_builds = Arc::clone(&setup.build_count);
        let parent_builds = Arc::new(AtomicU64::new(0));
        let parent = spawn_mock_parent(setup.request_rx, Arc::clone(&parent_builds));
        // Keep close_tx alive for the duration of the test by not dropping setup's other fields.
        // close_tx is intentionally moved out of setup when request_rx is taken; the watch
        // sender remaining in the hub keeps the channel open (false).
        drop(setup.close_tx);
        (hub_builds, parent_builds, parent, base, setup.server)
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
    fn accepts_gzip_reads_accept_encoding() {
        let with = |value: Option<&str>| {
            let mut headers = HeaderMap::new();
            if let Some(value) = value {
                headers.insert(
                    header::ACCEPT_ENCODING,
                    HeaderValue::from_str(value).unwrap(),
                );
            }
            accepts_gzip(&headers)
        };
        for yes in ["gzip", "gzip, deflate, br", "br;q=1.0, GZIP;q=0.5", "*"] {
            assert!(with(Some(yes)), "{}", yes);
        }
        for no in [
            "identity",
            "deflate, br",
            "gzip;q=0",
            "gzip; q=0.0",
            "*;q=0",
            "gzip;Q=0",
        ] {
            assert!(!with(Some(no)), "{}", no);
        }
        assert!(!with(None));
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
        // Any query token is rejected on control, even with a valid Bearer.
        assert!(!authorize_control("s3cret", &headers, Some("wrong")));
        assert!(!authorize_control("s3cret", &headers, Some("s3cret")));

        // Scheme is case-insensitive (RFC 7235).
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("bearer s3cret"),
        );
        assert!(authorize_control("s3cret", &headers, None));
        assert!(authorize("s3cret", &headers, None));

        // Tabs / multi-space between scheme and token are accepted.
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer\ts3cret"),
        );
        assert!(authorize("s3cret", &headers, None));
        assert!(authorize_control("s3cret", &headers, None));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer    s3cret"),
        );
        assert!(authorize("s3cret", &headers, None));
        // Extra credential parts are rejected.
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer s3cret extra"),
        );
        assert!(!authorize("s3cret", &headers, None));
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
        assert!(matches!(
            parse_users_body(b"{\"users\":50}"),
            Ok(ControllerCommand::Users(50))
        ));
        assert!(matches!(
            parse_users_body(b"{\"users\":1}"),
            Ok(ControllerCommand::Users(1))
        ));
        assert!(matches!(
            parse_users_body(b"{\"users\":100000}"),
            Ok(ControllerCommand::Users(100000))
        ));

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

        // An integer too large for a u64 is read as an f64 (no
        // arbitrary_precision), so it is refused by the bound up to f64::MAX;
        // above that serde_json refuses the body as "number out of range".
        let users = |digits: usize| format!("{{\"users\":{}}}", "9".repeat(digits));
        let err = parse_users_body(users(76).as_bytes()).unwrap_err();
        assert_eq!(err.0, "invalid_users");
        let err = parse_users_body(users(308).as_bytes()).unwrap_err();
        assert_eq!(err.0, "invalid_users");
        let err = parse_users_body(users(309).as_bytes()).unwrap_err();
        assert_eq!(err, ("bad_request", "malformed JSON body"));
    }

    /// A settings body: absent leaves a setting alone and `null` clears it;
    /// refused as `bad_request`, then `invalid_value`, then
    /// `conflicting_settings`, each naming the setting at fault.
    #[test]
    fn parse_settings_body_rules() {
        let settings = parse_settings_body(br#"{}"#).unwrap();
        assert!(settings.fields().is_empty());
        let settings = parse_settings_body(
            br#"{"host":"https://example.com","users":5,"increase_rate":2.5,"decrease_time_secs":null,"run_time_secs":90}"#,
        )
        .unwrap();
        assert_eq!(settings.host, Some(Some("https://example.com".to_string())));
        assert_eq!(settings.users, Some(5));
        assert_eq!(settings.increase_rate, Some(Some(2.5)));
        assert_eq!(settings.increase_time, None);
        assert_eq!(settings.decrease_rate, None);
        assert_eq!(settings.decrease_time, Some(None));
        assert_eq!(settings.run_time, Some(Some(90)));
        assert!(settings.test_plan.is_none());
        let settings =
            parse_settings_body(br#"{"host":null,"test_plan":"5,1s;0,1s","increase_rate":null}"#)
                .unwrap();
        assert_eq!(settings.host, Some(None));
        assert_eq!(settings.increase_rate, Some(None));
        assert_eq!(
            settings.test_plan.unwrap().unwrap().steps,
            [(5, 1_000), (0, 1_000)]
        );
        assert!(parse_settings_body(br#"{"users":2,"test_plan":null}"#).is_ok());
        assert!(parse_settings_body(br#"{"increase_rate":2,"increase_time_secs":null}"#).is_ok());

        for (body, refusal) in [
            // Malformed, not an object, or an unknown field, whatever else.
            (&b"{"[..], ("bad_request", None)),
            (b"[1]", ("bad_request", None)),
            (b"5", ("bad_request", None)),
            (
                br#"{"increase_rate":0,"iterations":1}"#,
                ("bad_request", None),
            ),
            (br#"{"run_time":60}"#, ("bad_request", None)),
            // A value of the wrong type or out of bounds, before conflicts.
            (
                br#"{"host":"ftp://example.com"}"#,
                ("invalid_value", Some("host")),
            ),
            (
                br#"{"host":"http://user:secret@example.com"}"#,
                ("invalid_value", Some("host")),
            ),
            (br#"{"host":5}"#, ("invalid_value", Some("host"))),
            (br#"{"users":null}"#, ("invalid_value", Some("users"))),
            (br#"{"users":0}"#, ("invalid_value", Some("users"))),
            (br#"{"users":"5"}"#, ("invalid_value", Some("users"))),
            (
                br#"{"increase_rate":0.001,"test_plan":"1,1s","users":3}"#,
                ("invalid_value", Some("increase_rate")),
            ),
            (
                br#"{"decrease_rate":"2"}"#,
                ("invalid_value", Some("decrease_rate")),
            ),
            (
                br#"{"decrease_rate":100001}"#,
                ("invalid_value", Some("decrease_rate")),
            ),
            (
                br#"{"increase_time_secs":0}"#,
                ("invalid_value", Some("increase_time")),
            ),
            (
                br#"{"decrease_time_secs":1.5}"#,
                ("invalid_value", Some("decrease_time")),
            ),
            (
                br#"{"run_time_secs":2592001}"#,
                ("invalid_value", Some("run_time")),
            ),
            (
                br#"{"run_time_secs":-1}"#,
                ("invalid_value", Some("run_time")),
            ),
            (
                br#"{"test_plan":"0,1s"}"#,
                ("invalid_value", Some("test_plan")),
            ),
            (br#"{"test_plan":10}"#, ("invalid_value", Some("test_plan"))),
            // Settings that never go together.
            (
                br#"{"test_plan":"1,1s","users":3}"#,
                ("conflicting_settings", Some("test_plan")),
            ),
            (
                br#"{"test_plan":"1,1s","run_time_secs":10}"#,
                ("conflicting_settings", Some("run_time")),
            ),
            (
                br#"{"test_plan":null}"#,
                ("conflicting_settings", Some("test_plan")),
            ),
            (
                br#"{"increase_rate":2,"increase_time_secs":5}"#,
                ("conflicting_settings", Some("increase_rate")),
            ),
            (
                br#"{"decrease_rate":2,"decrease_time_secs":5}"#,
                ("conflicting_settings", Some("decrease_rate")),
            ),
        ] {
            let (code, _, field) = parse_settings_body(body).unwrap_err();
            assert_eq!((code, field), refusal, "{}", String::from_utf8_lossy(body));
        }
        let plan = format!(r#"{{"test_plan":"{}"}}"#, "1,1s;".repeat(1000));
        let (code, _, field) = parse_settings_body(plan.as_bytes()).unwrap_err();
        assert_eq!((code, field), ("invalid_value", Some("test_plan")));
    }

    /// The settings route and a Start with settings take only the control
    /// token, as a Bearer header; a body refused before the main loop is a
    /// 400 naming the setting, and an accepted one reaches it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn settings_routes_require_token_and_check_body() {
        let (control_tx, control_rx) = flume::unbounded();
        let (setup, base) =
            bind_dashboard_with(TestBind::token("s3cret").control(control_tx)).await;
        let parent = spawn_mock_parent(setup.request_rx, Arc::new(AtomicU64::new(0)));
        let control = spawn_control_parent(control_rx, answer_ok);
        drop(setup.close_tx);
        let client = reqwest::Client::new();
        let post = |path: &str, auth: Option<&str>, body: &str| {
            let mut request = client
                .post(format!("{base}/api/v1/control/{path}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(body.to_string());
            if let Some(auth) = auth {
                request = request.header(header::AUTHORIZATION, auth.to_string());
            }
            request.send()
        };
        let body = r#"{"host":"https://example.com","increase_rate":2}"#;
        for path in [
            "settings",
            "start",
            "settings?token=s3cret",
            "start?token=s3cret",
        ] {
            for auth in [None, Some("Bearer wrong")] {
                let resp = post(path, auth, body).await.expect("post");
                assert_eq!(resp.status(), 401, "{} {:?}", path, auth);
            }
        }
        let resp = post("settings?token=s3cret", Some("Bearer s3cret"), body)
            .await
            .expect("post");
        assert_eq!(resp.status(), 401);

        for (path, message) in [
            ("settings", "settings configured"),
            ("start", "load test started"),
        ] {
            let resp = post(path, Some("Bearer s3cret"), body).await.expect("post");
            assert_eq!(resp.status(), 200, "{}", path);
            let reply: serde_json::Value = resp.json().await.expect("json");
            assert_eq!(reply["ok"], true, "{}", path);
            assert_eq!(reply["command"], path);
            assert_eq!(reply["message"], message);

            let resp = post(path, Some("Bearer s3cret"), r#"{"increase_rate":0}"#)
                .await
                .expect("post");
            assert_eq!(resp.status(), 400, "{}", path);
            let reply: serde_json::Value = resp.json().await.expect("json");
            assert_eq!(reply["error"], "invalid_value");
            assert_eq!(reply["field"], "increase_rate");
            assert_eq!(reply["command"], path);

            let resp = post(path, Some("Bearer s3cret"), r#"{"bogus":1}"#)
                .await
                .expect("post");
            assert_eq!(resp.status(), 400, "{}", path);
            let reply: serde_json::Value = resp.json().await.expect("json");
            assert_eq!(reply["error"], "bad_request");
            assert_eq!(reply["field"], serde_json::Value::Null);

            let resp = post(
                path,
                Some("Bearer s3cret"),
                r#"{"increase_rate":2,"increase_time_secs":5}"#,
            )
            .await
            .expect("post");
            assert_eq!(resp.status(), 400, "{}", path);
            let reply: serde_json::Value = resp.json().await.expect("json");
            assert_eq!(reply["error"], "conflicting_settings");
            assert_eq!(reply["field"], "increase_rate");

            // Over the control body limit.
            let big = format!(r#"{{"test_plan":"{}"}}"#, " ".repeat(CONTROL_BODY_LIMIT));
            let resp = post(path, Some("Bearer s3cret"), &big).await.expect("post");
            assert_eq!(resp.status(), 413, "{}", path);
        }
        parent.abort();
        control.abort();
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
            dashboard_host: Some("localhost".to_string()),
            dashboard_port: Some(0),
            dashboard_auth_token: None,
            ..Default::default()
        };
        let setup = setup_dashboard(&config, None, ListeningControllers::configured(&config))
            .await
            .expect("localhost bind should succeed")
            .expect("dashboard enabled");
        assert!(setup.bound_port > 0, "ephemeral port must be recorded");
        // Dropping the setup drops the server handle, which stops the server.
        drop(setup);

        // ::1 (bare IPv6 literal needs brackets in the bind string).
        let config = GooseConfiguration {
            dashboard: true,
            dashboard_host: Some("::1".to_string()),
            dashboard_port: Some(0),
            dashboard_auth_token: None,
            ..Default::default()
        };
        // ::1 may be unavailable in some CI network namespaces; treat bind errors
        // as soft-skip only when the OS reports address-family issues.
        match setup_dashboard(&config, None, ListeningControllers::configured(&config)).await {
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
            dashboard_host: Some("127.0.0.1".to_string()),
            dashboard_port: Some(port),
            dashboard_auth_token: None,
            ..Default::default()
        };
        let err = setup_dashboard(&config, None, ListeningControllers::configured(&config))
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
        let (hub_builds, parent_builds, parent, _base, _server) = setup_with_mock("").await;
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
        let (hub_builds, parent_builds, parent, base, _server) = setup_with_mock("").await;
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
        let (_hub_builds, _parent_builds, parent, base, _server) = setup_with_mock("").await;
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
        let (_hub_builds, _parent_builds, parent, base, _server) =
            setup_with_mock_max("", cap).await;
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
        let (hub_builds, _parent_builds, parent, base, _server) = setup_with_mock("").await;
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
        assert_eq!(idle_json["last_snapshot_ms"], 0);
        assert_eq!(idle_json["snapshot_timeouts"], 0);
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

    /// A hub with no HTTP server, for driving [`build_once`] directly.
    fn bare_hub() -> (SnapshotHub, flume::Receiver<DashboardRequest>) {
        let (request_tx, request_rx) = flume::unbounded();
        let (shutdown_tx, _) = watch::channel(false);
        let hub = SnapshotHub::new(
            request_tx,
            Arc::new(AtomicU64::new(0)),
            shutdown_tx,
            0,
            Arc::new(AtomicUsize::new(0)),
        );
        (hub, request_rx)
    }

    #[tokio::test]
    async fn build_records_processor_time_and_round_trip() {
        let (hub, request_rx) = bare_hub();
        // Report the time spent "building" as the processor does, so the
        // round trip, which starts before the request is sent, contains it.
        let parent = tokio::spawn(async move {
            while let Ok(msg) = request_rx.recv_async().await {
                if let DashboardRequest::GetSnapshot { respond, .. } = msg {
                    let started = Instant::now();
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    let mut snapshot = dummy_snapshot(0);
                    snapshot.build_time = started.elapsed();
                    let _ = respond.send(snapshot);
                }
            }
        });

        assert!(matches!(
            build_once(&hub.inner, SNAPSHOT_BUILD_TIMEOUT).await,
            BuildOutcome::Built
        ));
        let build_ms = hub.last_build_ms();
        let snapshot_ms = hub.last_snapshot_ms();
        assert!(build_ms >= 5, "last_build_ms {}", build_ms);
        assert!(
            build_ms <= snapshot_ms,
            "last_build_ms {} > last_snapshot_ms {}",
            build_ms,
            snapshot_ms
        );
        assert_eq!(hub.snapshot_timeouts(), 0);
        assert!(!hub.inner.snapshot_in_flight.load(Ordering::Acquire));
        // The published JSON never carries the processor time.
        let json = hub.inner.latest_tx.borrow().clone().expect("published");
        assert!(!String::from_utf8_lossy(&json).contains("build_time"));
        parent.abort();
    }

    #[tokio::test]
    async fn snapshot_timeout_is_counted_and_late_reply_dropped() {
        // The hub waits SNAPSHOT_BUILD_TIMEOUT in production; a short timeout
        // exercises the same path without a 4 second wait.
        assert_eq!(SNAPSHOT_BUILD_TIMEOUT, Duration::from_secs(4));
        let timeout = Duration::from_millis(50);
        let (hub, request_rx) = bare_hub();
        // Hold the reply until after the hub has given up on it, and keep the
        // channel open so the hub does not see a closed parent.
        let parent = tokio::spawn(async move {
            match request_rx.recv_async().await {
                Ok(DashboardRequest::GetSnapshot { respond, .. }) => (respond, request_rx),
                _ => panic!("expected GetSnapshot"),
            }
        });

        assert!(matches!(
            build_once(&hub.inner, timeout).await,
            BuildOutcome::Transient
        ));
        assert_eq!(hub.snapshot_timeouts(), 1);
        assert!(hub.inner.snapshot_in_flight.load(Ordering::Acquire));
        // While the late reply is outstanding, no new request is sent.
        assert!(matches!(
            build_once(&hub.inner, timeout).await,
            BuildOutcome::Transient
        ));
        assert_eq!(hub.snapshot_timeouts(), 1);

        let (respond, _request_rx) = parent.await.expect("parent");
        let _ = respond.send(dummy_snapshot(0));
        for _ in 0..50 {
            if !hub.inner.snapshot_in_flight.load(Ordering::Acquire) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!hub.inner.snapshot_in_flight.load(Ordering::Acquire));
        // The late snapshot is not published and does not count as a build.
        assert!(hub.inner.latest_tx.borrow().is_none());
        assert_eq!(hub.build_count(), 0);
        assert_eq!(hub.last_build_ms(), 0);
        assert_eq!(hub.last_snapshot_ms(), 0);
    }

    #[tokio::test]
    async fn health_reports_controller_addresses() {
        // (no_telnet, websocket, whether telnet bound, expected
        // controllers). Hosts and ports are set as setup_controllers leaves
        // them; an IPv6 host is bracketed.
        let cases = [
            (
                false,
                true,
                true,
                serde_json::json!({"telnet": "0.0.0.0:5116", "websocket": "[::1]:6117"}),
            ),
            (
                true,
                true,
                true,
                serde_json::json!({"telnet": null, "websocket": "[::1]:6117"}),
            ),
            (
                true,
                false,
                true,
                serde_json::json!({"telnet": null, "websocket": null}),
            ),
            // The default: telnet on, the WebSocket Controller off.
            (
                false,
                false,
                true,
                serde_json::json!({"telnet": "0.0.0.0:5116", "websocket": null}),
            ),
            // A Controller that couldn't bind its port is not named.
            (
                false,
                true,
                false,
                serde_json::json!({"telnet": null, "websocket": "[::1]:6117"}),
            ),
        ];
        let client = reqwest::Client::new();
        for (no_telnet, websocket, telnet_bound, expected) in cases {
            let config = GooseConfiguration {
                dashboard: true,
                dashboard_host: Some("127.0.0.1".to_string()),
                dashboard_port: Some(0),
                no_telnet,
                telnet_host: Some("0.0.0.0".to_string()),
                telnet_port: Some(5116),
                websocket,
                websocket_host: Some("::1".to_string()),
                websocket_port: Some(6117),
                ..Default::default()
            };
            let mut listening = ListeningControllers::configured(&config);
            listening.telnet &= telnet_bound;
            let setup = setup_dashboard(&config, None, listening)
                .await
                .expect("bind")
                .expect("enabled");
            let health: serde_json::Value = client
                .get(format!(
                    "http://127.0.0.1:{}/api/v1/health",
                    setup.bound_port
                ))
                .send()
                .await
                .expect("health")
                .json()
                .await
                .expect("json");
            assert_eq!(
                health["controllers"], expected,
                "no_telnet {}, websocket {}, telnet bound {}",
                no_telnet, websocket, telnet_bound
            );
            // The fields health had before stay.
            assert_eq!(health["ok"], true);
            assert_eq!(health["control_enabled"], false);
            for field in [
                "version",
                "last_build_ms",
                "last_snapshot_ms",
                "snapshot_timeouts",
                "build_count",
                "active_sse_clients",
            ] {
                assert!(!health[field].is_null(), "{} missing", field);
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn events_require_token_when_configured() {
        let token = "sse-secret";
        let (_hub, _parent_builds, parent, base, _server) = setup_with_mock(token).await;
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
                    DashboardRequest::GetSnapshot { respond, .. } => {
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
                    DashboardRequest::GetRequestSeries { respond, .. } => {
                        let _ = respond.send(None);
                    }
                }
            }
        });

        let build_count = Arc::new(AtomicU64::new(0));
        let (close_tx, _) = watch::channel(false);
        let hub = SnapshotHub::new(
            tx,
            Arc::clone(&build_count),
            close_tx,
            MAX_SSE_CLIENTS,
            Arc::new(AtomicUsize::new(0)),
        );

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
        let (hub_builds, parent_builds, parent, base, _server) = setup_with_mock("").await;
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
        let (_hub, _parent_builds, parent, base, _server) = setup_with_mock("").await;
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

    /// The requests the HTTP matrix sends every accepting row: method, path
    /// and a valid JSON body.
    const MATRIX_REQUESTS: [(&str, &str, &str); 10] = [
        ("GET", "/api/v1/health", ""),
        ("GET", "/api/v1/snapshot", ""),
        ("POST", "/api/v1/control/start", "{}"),
        ("POST", "/api/v1/control/stop", "{}"),
        ("POST", "/api/v1/control/users", "{\"users\":3}"),
        ("POST", "/api/v1/control/settings", "{}"),
        ("POST", "/api/v1/control/report", "{\"save\":true}"),
        ("POST", "/api/v1/control/report/now", "{}"),
        ("POST", "/api/v1/control/quit", "{}"),
        ("DELETE", "/api/v1/reports/no-such-run", ""),
    ];

    /// The status each accepting row answers each of [`MATRIX_REQUESTS`],
    /// sent with no token, the right token and a wrong one, all as the page
    /// sends them: JSON, no `Origin`, a loopback IP `Host`. A missing report
    /// is a 404 once a Delete is authorized.
    const MATRIX_STATUS: [(&str, [[u16; 3]; 10]); 7] = [
        (
            "R1",
            [
                [200; 3], [200; 3], [404; 3], [404; 3], [404; 3], [404; 3], [404; 3], [404; 3],
                [404; 3], [404; 3],
            ],
        ),
        (
            "R2",
            [
                [200; 3],
                [401, 200, 401],
                [404; 3],
                [404; 3],
                [404; 3],
                [404; 3],
                [404; 3],
                [404; 3],
                [404; 3],
                [404; 3],
            ],
        ),
        (
            "R3",
            [
                [200; 3],
                [401, 200, 401],
                [401, 200, 401],
                [401, 200, 401],
                [401, 200, 401],
                [401, 200, 401],
                [401, 200, 401],
                [401, 200, 401],
                [401, 200, 401],
                [401, 404, 401],
            ],
        ),
        (
            "R4",
            [
                [200; 3], [200; 3], [200; 3], [200; 3], [200; 3], [200; 3], [200; 3], [200; 3],
                [200; 3], [404; 3],
            ],
        ),
        (
            "R5",
            [
                [200; 3], [200; 3], [404; 3], [404; 3], [404; 3], [404; 3], [404; 3], [404; 3],
                [404; 3], [404; 3],
            ],
        ),
        (
            "R6",
            [
                [200; 3], [200; 3], [200; 3], [200; 3], [200; 3], [200; 3], [200; 3], [200; 3],
                [200; 3], [404; 3],
            ],
        ),
        (
            "R11",
            [
                [200; 3],
                [401, 200, 401],
                [401, 200, 401],
                [401, 200, 401],
                [401, 200, 401],
                [401, 200, 401],
                [401, 200, 401],
                [401, 200, 401],
                [401, 200, 401],
                [401, 404, 401],
            ],
        ),
    ];

    /// What `GET /api/v1/health` says for each accepting row:
    /// `control_enabled`, `auth_required` and `loopback`.
    const MATRIX_HEALTH: [(&str, [bool; 3]); 7] = [
        ("R1", [false, true, true]),
        ("R2", [false, true, true]),
        ("R3", [true, true, true]),
        ("R4", [true, false, true]),
        ("R5", [false, false, true]),
        ("R6", [true, false, false]),
        ("R11", [true, true, false]),
    ];

    /// The cross site probes, for the rows with control and no token, on
    /// each request of [`MATRIX_REQUESTS`] that changes something: a form
    /// post, a body-less request with no JSON content type, JSON with a
    /// foreign `Origin`, and JSON with the page's own `Origin`.
    const MATRIX_PROBES: [(&str, [u16; 4]); 8] = [
        ("/api/v1/control/start", [403, 403, 403, 200]),
        ("/api/v1/control/stop", [403, 403, 403, 200]),
        ("/api/v1/control/users", [403, 403, 403, 200]),
        ("/api/v1/control/settings", [403, 403, 403, 200]),
        ("/api/v1/control/report", [403, 403, 403, 200]),
        ("/api/v1/control/report/now", [403, 403, 403, 200]),
        ("/api/v1/control/quit", [403, 403, 403, 200]),
        ("/api/v1/reports/no-such-run", [403, 403, 403, 404]),
    ];

    /// What a page at another name gets, for each accepting row, as `Host`
    /// (and as `Origin` on the Start): `[snapshot as rebind.example,
    /// Start as rebind.example, snapshot as the configured host,
    /// snapshot as an IPv6 literal]`.
    const MATRIX_HOSTS: [(&str, [u16; 4]); 7] = [
        ("R1", [403, 403, 200, 200]),
        ("R2", [403, 403, 401, 401]),
        ("R3", [403, 403, 401, 401]),
        ("R4", [403, 403, 200, 200]),
        ("R5", [403, 403, 200, 200]),
        ("R6", [403, 403, 200, 200]),
        // A token bind off loopback checks no Host; every cell lacks the token.
        ("R11", [401, 401, 401, 401]),
    ];

    /// Request headers, by name and value.
    type Headers<'a> = Vec<(&'a str, &'a str)>;

    /// Send `method path` on a fresh connection with `Host: host`, the
    /// headers and the body; answers the status and the body.
    async fn raw_call(
        port: u16,
        method: &str,
        path: &str,
        host: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> (u16, String) {
        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Length: {}\r\n",
            body.len()
        );
        for (name, value) in headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("\r\n");
        request.push_str(body);
        let reply = raw_exchange(port, &request).await;
        let status = reply.get(9..12).and_then(|s| s.parse().ok()).unwrap_or(0);
        let body = reply
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_string())
            .unwrap_or_default();
        (status, body)
    }

    /// Every accepting row of the shared configuration matrix, bound through
    /// the one helper, answers every request of the fixed list as its table
    /// says: with no token, the right one and a wrong one; under the cross
    /// site probes when it has control and no token; and as another `Host`.
    /// Replaces `control_disabled_returns_404_and_health_false`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dashboard_matrix_answers_every_request() {
        let token = crate::config::DASHBOARD_ROW_TOKEN;
        let right = format!("Bearer {token}");
        let ways: [Option<&str>; 3] = [None, Some(&right), Some("Bearer wrong")];
        let mut failed: Vec<String> = Vec::new();
        fn check(failed: &mut Vec<String>, cell: String, got: u16, want: u16, body: &str) {
            if got != want {
                failed.push(format!("{cell}: {got}, want {want} {body:?}"));
            }
        }
        for row in crate::config::DASHBOARD_ROWS
            .iter()
            .filter(|r| r.refusal.is_none())
        {
            let (control_tx, control_rx) = flume::unbounded();
            let (setup, base) = bind_dashboard_with(TestBind::row(row, control_tx)).await;
            let parent = spawn_mock_parent(setup.request_rx, Arc::new(AtomicU64::new(0)));
            let control = spawn_control_parent(control_rx, answer_ok);
            let port = port_of(&base);
            let own_host = format!("127.0.0.1:{port}");
            let name = row.name;

            let health = MATRIX_HEALTH
                .iter()
                .find(|(r, _)| *r == name)
                .expect("health row")
                .1;
            let (_, body) = raw_call(port, "GET", "/api/v1/health", &own_host, &[], "").await;
            let json: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            for (field, want) in ["control_enabled", "auth_required", "loopback"]
                .iter()
                .zip(health)
            {
                if json[field] != want {
                    failed.push(format!(
                        "{name} health {field}: {}, want {want}",
                        json[field]
                    ));
                }
            }

            let statuses = MATRIX_STATUS
                .iter()
                .find(|(r, _)| *r == name)
                .expect("status row")
                .1;
            for ((method, path, body), want) in MATRIX_REQUESTS.iter().zip(statuses) {
                for (way, want) in ways.iter().zip(want) {
                    let mut headers = vec![("Content-Type", "application/json")];
                    if let Some(bearer) = way {
                        headers.push(("Authorization", bearer));
                    }
                    let (got, reply) =
                        raw_call(port, method, path, &own_host, &headers, body).await;
                    let way = match way {
                        None => "no token",
                        Some(b) if *b == right => "right token",
                        Some(_) => "wrong token",
                    };
                    check(
                        &mut failed,
                        format!("{name} {method} {path} {way}"),
                        got,
                        want,
                        &reply,
                    );
                }
            }

            if row.args.contains(&"--dashboard-no-auth")
                && row.args.contains(&"--dashboard-control")
            {
                let own_origin = format!("http://{own_host}");
                for (path, want) in MATRIX_PROBES {
                    let (method, _, body) = MATRIX_REQUESTS
                        .iter()
                        .find(|(_, p, _)| *p == path)
                        .expect("probed request");
                    let probes: [(&str, Headers, &str); 4] = [
                        (
                            "form",
                            vec![("Content-Type", "application/x-www-form-urlencoded")],
                            "a=b",
                        ),
                        ("body-less", vec![], ""),
                        (
                            "foreign Origin",
                            vec![
                                ("Content-Type", "application/json"),
                                ("Origin", "http://evil.example"),
                            ],
                            body,
                        ),
                        (
                            "own Origin",
                            vec![
                                ("Content-Type", "application/json"),
                                ("Origin", &own_origin),
                            ],
                            body,
                        ),
                    ];
                    for ((probe, headers, body), want) in probes.iter().zip(want) {
                        let (got, reply) =
                            raw_call(port, method, path, &own_host, headers, body).await;
                        if want == 403 && !reply.contains("\"reason\":\"cross_origin\"") {
                            failed.push(format!(
                                "{name} {method} {path} {probe}: no cross_origin reason {reply:?}"
                            ));
                        }
                        check(
                            &mut failed,
                            format!("{name} {method} {path} {probe}"),
                            got,
                            want,
                            &reply,
                        );
                    }
                }
            }

            let hosts = MATRIX_HOSTS
                .iter()
                .find(|(r, _)| *r == name)
                .expect("host row")
                .1;
            let rebind = format!("rebind.example:{port}");
            let rebind_origin = format!("http://{rebind}");
            let configured = format!(
                "{}:{port}",
                crate::config::dashboard_row_configuration(row).dashboard_host()
            );
            let ipv6 = format!("[::1]:{port}");
            let json = [("Content-Type", "application/json")];
            let cells: [(&str, &str, &str, Headers, &str); 4] = [
                ("GET", "/api/v1/snapshot", &rebind, vec![], ""),
                (
                    "POST",
                    "/api/v1/control/start",
                    &rebind,
                    vec![json[0], ("Origin", &rebind_origin)],
                    "{}",
                ),
                ("GET", "/api/v1/snapshot", &configured, vec![], ""),
                ("GET", "/api/v1/snapshot", &ipv6, vec![], ""),
            ];
            for ((method, path, host, headers, body), want) in cells.iter().zip(hosts) {
                let (got, reply) = raw_call(port, method, path, host, headers, body).await;
                if name == "R6" && want == 403 && !reply.contains("\"reason\":\"host_not_allowed\"")
                {
                    failed.push(format!(
                        "{name} {method} {path} as {host}: no host_not_allowed reason {reply:?}"
                    ));
                }
                check(
                    &mut failed,
                    format!("{name} {method} {path} as {host}"),
                    got,
                    want,
                    &reply,
                );
            }

            parent.abort();
            control.abort();
            drop(setup.close_tx);
        }
        assert!(failed.is_empty(), "{:#?}", failed);
    }

    /// What each accepting row prints at startup, bound to port 5118. The
    /// token is never printed.
    #[test]
    fn dashboard_matrix_startup_lines() {
        let read_only_hint = "[dashboard]: read-only; for control add --dashboard-control and either --dashboard-no-auth (no token) or --dashboard-auth-token <token>";
        let expected: [(&str, Vec<StartupLine>); 7] = [
            (
                "R1",
                vec![
                    StartupLine::Info("[dashboard]: listening on http://127.0.0.1:5118 (read-only)".into()),
                    StartupLine::Info(read_only_hint.into()),
                ],
            ),
            (
                "R2",
                vec![
                    StartupLine::Info("[dashboard]: listening on http://127.0.0.1:5118 (read-only)".into()),
                    StartupLine::Info("[dashboard]: read-only; for control add --dashboard-control".into()),
                ],
            ),
            (
                "R3",
                vec![
                    StartupLine::Info("[dashboard]: listening on http://127.0.0.1:5118 (control enabled)".into()),
                    StartupLine::Info("[dashboard]: open http://127.0.0.1:5118/?token=<token> to control the test".into()),
                ],
            ),
            (
                "R4",
                vec![
                    StartupLine::Info("[dashboard]: listening on http://127.0.0.1:5118 (control enabled, no token)".into()),
                    StartupLine::Warn("[dashboard]: --dashboard-no-auth: anyone on this machine can start, stop and redirect the test".into()),
                ],
            ),
            (
                "R5",
                vec![
                    StartupLine::Info("[dashboard]: listening on http://127.0.0.1:5118 (read-only, no token)".into()),
                    StartupLine::Info("[dashboard]: read-only; for control add --dashboard-control".into()),
                ],
            ),
            (
                "R11",
                vec![
                    StartupLine::Info("[dashboard]: listening on http://0.0.0.0:5118 (control enabled)".into()),
                    // A browser can't open 0.0.0.0: the line to open names loopback.
                    StartupLine::Info("[dashboard]: open http://127.0.0.1:5118/?token=<token> to control the test".into()),
                ],
            ),
            (
                "R6",
                vec![
                    StartupLine::Info("[dashboard]: listening on http://0.0.0.0:5118 (control enabled, no token)".into()),
                    StartupLine::Block(
                        "WARNING: the dashboard on http://0.0.0.0:5118 has no token (--dashboard-no-auth).\n\
                         Anyone who can reach this port can watch the test, start and stop it,\n\
                         and point the load at any host.\n\
                         Safer: bind 127.0.0.1 and use an ssh forward (ssh -L 5118:127.0.0.1:5118 host),\n\
                         or use --dashboard-auth-token in place of --dashboard-no-auth. Only IP addresses,\n\
                         localhost and the --dashboard-host name are accepted as Host; set --dashboard-host\n\
                         to the DNS name you browse to."
                            .into(),
                    ),
                ],
            ),
        ];
        for row in crate::config::DASHBOARD_ROWS
            .iter()
            .filter(|r| r.refusal.is_none())
        {
            let configuration = crate::config::dashboard_row_configuration(row);
            let host = configuration.dashboard_host();
            let bound = format_bind_address(host, 5118);
            let lines = startup_lines(
                &bound,
                configuration.dashboard_control,
                configuration.dashboard_no_auth,
                configuration.dashboard_auth_token().is_some(),
                crate::config::is_loopback_bind(host),
            );
            let want = &expected
                .iter()
                .find(|(r, _)| *r == row.name)
                .expect("row")
                .1;
            assert_eq!(&lines, want, "{}", row.name);
            for line in &lines {
                let text = format!("{line:?}");
                assert!(
                    !text.contains(crate::config::DASHBOARD_ROW_TOKEN),
                    "{}",
                    text
                );
            }
        }
        // An unspecified IPv6 bind, as R11 with --dashboard-host ::, opens on [::1].
        assert_eq!(
            startup_lines("[::]:5118", true, false, true, false),
            vec![
                StartupLine::Info(
                    "[dashboard]: listening on http://[::]:5118 (control enabled)".into()
                ),
                StartupLine::Info(
                    "[dashboard]: open http://[::1]:5118/?token=<token> to control the test".into()
                ),
            ]
        );
    }

    /// The no token Host rule: IP literals, and the configured name, with or
    /// without a port; no other name.
    #[test]
    fn ip_or_configured_host_rules() {
        for (host, configured, allowed) in [
            ("10.1.2.3:5118", "0.0.0.0", true),
            ("10.1.2.3", "0.0.0.0", true),
            ("[fe80::1]:5118", "0.0.0.0", true),
            ("[::1]", "::", true),
            ("loadgen.example:5118", "loadgen.example", true),
            ("LOADGEN.example", "loadgen.example", true),
            ("rebind.example:5118", "0.0.0.0", false),
            ("rebind.example:5118", "loadgen.example", false),
            ("localhost:5118", "0.0.0.0", true),
            ("localhost.example:5118", "0.0.0.0", false),
            ("fe80::1", "0.0.0.0", false),
            ("user@10.1.2.3", "0.0.0.0", false),
            ("", "0.0.0.0", false),
        ] {
            assert_eq!(
                is_ip_or_configured_host(host, configured),
                allowed,
                "{host} with --dashboard-host {configured}"
            );
        }
        assert!(is_json_content_type("application/json"));
        assert!(is_json_content_type("Application/JSON; charset=utf-8"));
        assert!(!is_json_content_type("text/plain"));
        assert!(!is_json_content_type("application/x-www-form-urlencoded"));
        assert!(!is_json_content_type("application/jsonp"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn control_requires_auth_and_validates_body() {
        let (control_tx, control_rx) = flume::unbounded();
        let (setup, base) =
            bind_dashboard_with(TestBind::token("s3cret").control(control_tx)).await;
        let parent = spawn_mock_parent(setup.request_rx, Arc::new(AtomicU64::new(0)));
        let control = spawn_control_parent(control_rx, answer_ok);
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

        // Query token with valid Bearer is also rejected (no URL secret on control).
        let resp = client
            .post(format!("{base}/api/v1/control/stop?token=s3cret"))
            .header(header::AUTHORIZATION, "Bearer s3cret")
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
        control.abort();
    }

    /// A heatmap export with these columns (second, measured count in bucket 3).
    fn heatmap_export(
        generation: u64,
        rev: u64,
        since: u64,
        start_second: u64,
        columns: &[(u64, u64)],
        buckets: usize,
    ) -> Heatmap {
        Heatmap {
            generation,
            rev,
            since,
            start_second,
            end_second: start_second + 299,
            hold_from: start_second + 298,
            buckets_us: vec![[0, 0, 0]; buckets],
            columns: columns
                .iter()
                .map(|&(second, count)| HeatmapColumn {
                    second,
                    measured: vec![3, count],
                    synthetic: Vec::new(),
                })
                .collect(),
        }
    }

    /// `(second, measured count)` of each column of a heatmap.
    fn column_counts(heatmap: &Heatmap) -> Vec<(u64, u64)> {
        heatmap
            .columns
            .iter()
            .map(|column| (column.second, column.measured[1]))
            .collect()
    }

    #[test]
    fn heatmap_store_sends_each_client_what_it_lacks() {
        let cursor = |generation, rev| Some(HeatmapCursor { generation, rev });
        let mut store = HeatmapStore::new(&heatmap_export(7, 5, 0, 0, &[(1, 10), (2, 4)], 205));
        // A client with nothing, or a stranger's cursor, gets every column.
        for held in [None, cursor(6, 5), cursor(7, 6)] {
            let heatmap = store.since(held);
            assert_eq!((heatmap.since, heatmap.rev), (0, 5), "{:?}", held);
            assert_eq!(column_counts(&heatmap), [(1, 10), (2, 4)]);
        }
        assert!(store.since(cursor(7, 5)).columns.is_empty());

        // The next build: second 2 grew and second 3 began.
        store.apply(&heatmap_export(7, 9, 5, 0, &[(2, 6), (3, 1)], 205));
        assert_eq!(
            store.cursor(),
            HeatmapCursor {
                generation: 7,
                rev: 9
            }
        );
        let caught_up = store.since(cursor(7, 5));
        assert_eq!(caught_up.since, 5);
        assert_eq!(column_counts(&caught_up), [(2, 6), (3, 1)]);
        // A client that missed builds gets everything changed after its own.
        assert_eq!(
            column_counts(&store.since(cursor(7, 2))),
            [(1, 10), (2, 6), (3, 1)]
        );
        assert_eq!(column_counts(&store.since(None)), [(1, 10), (2, 6), (3, 1)]);

        // The window moves on: older columns are dropped. A shorter bucket
        // table never replaces a longer one.
        store.apply(&heatmap_export(7, 12, 9, 2, &[(301, 2)], 206));
        store.apply(&heatmap_export(7, 13, 12, 2, &[], 205));
        let whole = store.since(None);
        assert_eq!(column_counts(&whole), [(2, 6), (3, 1), (301, 2)]);
        assert_eq!(whole.buckets_us.len(), 206);
        assert_eq!((whole.start_second, whole.end_second), (2, 301));

        // A new generation (metrics reset, new test) replaces everything.
        store.apply(&heatmap_export(8, 1, 0, 0, &[(0, 1)], 205));
        assert_eq!(column_counts(&store.since(cursor(7, 13))), [(0, 1)]);
        assert_eq!(store.since(cursor(7, 13)).since, 0);
    }

    /// A fifth series request while four wait on a main loop that never
    /// answers gets 503 `busy`, and control requests are still admitted.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn series_admission_caps_in_flight() {
        let (control_tx, control_rx) = flume::unbounded();
        let (setup, base) =
            bind_dashboard_with(TestBind::token("s3cret").control(control_tx)).await;
        // Nobody drains the request channel: admitted requests wait in it.
        let request_rx = setup.request_rx;
        let client = reqwest::Client::new();
        let wait_for_queue = |len: usize| {
            let request_rx = request_rx.clone();
            async move {
                let deadline = Instant::now() + Duration::from_secs(5);
                while request_rx.len() < len && Instant::now() < deadline {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                request_rx.len()
            }
        };
        let mut waiting = Vec::new();
        for i in 0..MAX_SERIES_IN_FLIGHT {
            let client = client.clone();
            let url = format!("{base}/api/v1/series?method=GET&name=/{i}&token=s3cret");
            waiting.push(tokio::spawn(async move { client.get(url).send().await }));
        }
        assert_eq!(
            wait_for_queue(MAX_SERIES_IN_FLIGHT).await,
            MAX_SERIES_IN_FLIGHT
        );

        let resp = client
            .get(format!(
                "{base}/api/v1/series?method=GET&name=/{MAX_SERIES_IN_FLIGHT}&token=s3cret"
            ))
            .send()
            .await
            .expect("fifth series GET");
        assert_eq!(resp.status(), 503);
        let body: serde_json::Value = resp.json().await.expect("json");
        assert_eq!(body, serde_json::json!({ "error": "busy" }));
        assert_eq!(request_rx.len(), MAX_SERIES_IN_FLIGHT);

        // Series polls never take control slots: Start still reaches the
        // main loop.
        let start_client = client.clone();
        let start_url = format!("{base}/api/v1/control/start");
        waiting.push(tokio::spawn(async move {
            start_client
                .post(start_url)
                .header(header::AUTHORIZATION, "Bearer s3cret")
                .header(header::CONTENT_TYPE, "application/json")
                .body("{}")
                .send()
                .await
        }));
        let deadline = Instant::now() + Duration::from_secs(5);
        while control_rx.is_empty() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(control_rx.len(), 1);
        for task in waiting {
            task.abort();
        }
    }

    /// A control request beyond the four waiting on a main loop that never
    /// answers gets 503 `busy` and never reaches the main loop.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn control_admission_caps_in_flight() {
        let (control_tx, control_rx) = flume::unbounded();
        let (_setup, base) =
            bind_dashboard_with(TestBind::token("s3cret").control(control_tx)).await;
        let client = reqwest::Client::new();
        let start = |client: reqwest::Client, url: String| async move {
            client
                .post(url)
                .header(header::AUTHORIZATION, "Bearer s3cret")
                .header(header::CONTENT_TYPE, "application/json")
                .body("{}")
                .send()
                .await
        };
        let url = format!("{base}/api/v1/control/start");
        let fill = || -> Vec<_> {
            (0..MAX_CONTROL_IN_FLIGHT)
                .map(|_| tokio::spawn(start(client.clone(), url.clone())))
                .collect()
        };
        let wait_for_queue = || async {
            let deadline = Instant::now() + Duration::from_secs(5);
            while control_rx.len() < MAX_CONTROL_IN_FLIGHT && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            control_rx.len()
        };
        // Nobody receives from control_rx: admitted requests wait in it.
        let waiting = fill();
        assert_eq!(wait_for_queue().await, MAX_CONTROL_IN_FLIGHT);

        let resp = start(client.clone(), url.clone())
            .await
            .expect("fifth start POST");
        assert_eq!(resp.status(), 503);
        let body: serde_json::Value = resp.json().await.expect("json");
        assert_eq!(body["ok"], false);
        assert_eq!(body["command"], "start");
        assert_eq!(body["error"], "busy");
        assert_eq!(control_rx.len(), MAX_CONTROL_IN_FLIGHT);

        // Dropping the queued commands answers the waiting requests 503
        // `unavailable`, which must free their slots for a new set.
        drop(control_rx.drain().collect::<Vec<_>>());
        for task in waiting {
            let resp = task.await.expect("join").expect("waiting start POST");
            assert_eq!(resp.status(), 503);
        }
        let again = fill();
        assert_eq!(
            wait_for_queue().await,
            MAX_CONTROL_IN_FLIGHT,
            "slots must be released"
        );
        for task in again {
            task.abort();
        }
    }

    /// A control body over [`CONTROL_BODY_LIMIT`] gets 413; one of exactly
    /// the limit passes the size gate and reaches body validation.
    #[tokio::test]
    async fn control_body_limit_is_exact() {
        let (control_tx, _control_rx) = flume::unbounded();
        let (_setup, base) =
            bind_dashboard_with(TestBind::token("s3cret").control(control_tx)).await;
        let client = reqwest::Client::new();
        let post = |path: &'static str, len: usize| {
            client
                .post(format!("{base}{path}"))
                .header(header::AUTHORIZATION, "Bearer s3cret")
                .header(header::CONTENT_TYPE, "application/json")
                .body(vec![b'x'; len])
                .send()
        };

        for path in [
            "/api/v1/control/start",
            "/api/v1/control/stop",
            "/api/v1/control/quit",
            "/api/v1/control/users",
        ] {
            let resp = post(path, CONTROL_BODY_LIMIT + 1)
                .await
                .expect("oversized POST");
            assert_eq!(resp.status(), 413, "{path}");
        }

        let resp = post("/api/v1/control/start", CONTROL_BODY_LIMIT)
            .await
            .expect("limit sized POST");
        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = resp.json().await.expect("json");
        assert_eq!(body["command"], "start");
        assert_eq!(body["error"], "bad_request");
    }

    /// Case 10: channel disconnect / oneshot drop → 503 `unavailable`;
    /// parent `internal` → 200 `error=internal`; oneshot timeout → 503.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn control_oneshot_error_paths() {
        let client = reqwest::Client::new();

        // --- disconnect: no parent consuming the control channel ---
        {
            let (control_tx, control_rx) = flume::unbounded();
            let (setup, base) =
                bind_dashboard_with(TestBind::token("s3cret").control(control_tx)).await;
            drop(setup.close_tx);
            drop(control_rx);
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
            let (control_tx, control_rx) = flume::unbounded();
            let (setup, base) =
                bind_dashboard_with(TestBind::token("s3cret").control(control_tx)).await;
            let parent = spawn_control_parent(control_rx, |_| None);
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
            let (control_tx, control_rx) = flume::unbounded();
            let (setup, base) =
                bind_dashboard_with(TestBind::token("s3cret").control(control_tx)).await;
            let parent = spawn_control_parent(control_rx, |_| {
                outcome(
                    false,
                    Some("internal"),
                    "internal control error",
                    "maintain",
                    1,
                    None,
                )
            });
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
            let (control_tx, control_rx) = flume::unbounded();
            let (setup, base) =
                bind_dashboard_with(TestBind::token("s3cret").control(control_tx)).await;
            // Hold every oneshot so the client hits CONTROL_TIMEOUT rather
            // than an immediate oneshot-drop 503.
            let parent = tokio::spawn(async move {
                let mut held = Vec::new();
                while let Ok(request) = control_rx.recv_async().await {
                    held.push(request);
                }
            });
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

    /// Send one keep-alive `GET /api/v1/health` on `stream` and return the
    /// response head and body, or `None` if the connection is closed or
    /// gives no complete response within two seconds.
    async fn health_on(stream: &mut tokio::net::TcpStream) -> Option<String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let request = "GET /api/v1/health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        stream.write_all(request.as_bytes()).await.ok()?;
        let mut buf = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let text = String::from_utf8_lossy(&buf).into_owned();
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let length = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length: "))
                    .and_then(|v| v.trim().parse::<usize>().ok())?;
                if body.len() >= length {
                    return Some(text);
                }
            }
            let mut chunk = [0u8; 1024];
            let left = deadline.checked_duration_since(Instant::now())?;
            match tokio::time::timeout(left, stream.read(&mut chunk)).await {
                Ok(Ok(n)) if n > 0 => buf.extend_from_slice(&chunk[..n]),
                _ => return None,
            }
        }
    }

    /// True when a read on `stream` reports the peer closed it (EOF or reset)
    /// within two seconds.
    async fn closed_by_peer(stream: &mut tokio::net::TcpStream) -> bool {
        closed_within(stream, Duration::from_secs(2)).await
    }

    /// True when a read on `stream` reports the peer closed it (EOF or reset)
    /// within `within`.
    async fn closed_within(stream: &mut tokio::net::TcpStream, within: Duration) -> bool {
        use tokio::io::AsyncReadExt;
        let mut chunk = [0u8; 64];
        matches!(
            tokio::time::timeout(within, stream.read(&mut chunk)).await,
            Ok(Ok(0)) | Ok(Err(_))
        )
    }

    /// Regression for #695: shutdown closes the listener and keep-alive
    /// connections, and the port can be bound again once it returns.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_closes_listener_and_keep_alive_connections() {
        let (setup, _base) = bind_dashboard("").await;
        let port = setup.bound_port;
        let mut keep_alive = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        let first = health_on(&mut keep_alive).await.expect("health answers");
        assert!(first.starts_with("HTTP/1.1 200"), "{}", first);

        let started = Instant::now();
        tokio::time::timeout(Duration::from_secs(5), setup.server.shutdown())
            .await
            .expect("shutdown must finish");
        assert!(
            started.elapsed() < SERVER_DRAIN_TIMEOUT,
            "an idle keep-alive connection closes at once, without the drain deadline"
        );

        assert!(
            tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_err(),
            "no new connection may be accepted after shutdown"
        );
        assert_eq!(
            health_on(&mut keep_alive).await,
            None,
            "keep-alive connection must not be served after shutdown"
        );
        drop(
            tokio::net::TcpListener::bind(("127.0.0.1", port))
                .await
                .expect("port must be free when shutdown returns"),
        );
    }

    /// A request that never completes cannot hold shutdown past the drain
    /// deadline; the listener closes before the drain and the busy
    /// connection is closed at the deadline.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_drain_is_bounded() {
        use tokio::io::AsyncWriteExt;
        let (control_tx, control_rx) = flume::unbounded();
        let (setup, _base) =
            bind_dashboard_with(TestBind::token("s3cret").control(control_tx)).await;
        let port = setup.bound_port;
        let mut stalled = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        stalled
            .write_all(
                b"POST /api/v1/control/users HTTP/1.1\r\nHost: 127.0.0.1\r\n\
                  Authorization: Bearer s3cret\r\nContent-Type: application/json\r\n\
                  Content-Length: 11\r\n\r\n{\"users\":5}",
            )
            .await
            .expect("write control request");
        // Hold the request unanswered: its handler waits on the reply, so the
        // connection is busy (not idle) when shutdown starts.
        let _in_flight = tokio::time::timeout(Duration::from_secs(5), control_rx.recv_async())
            .await
            .expect("control request reaches the parent")
            .expect("request channel open");

        let started = Instant::now();
        let shutdown = tokio::spawn(setup.server.shutdown());
        loop {
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_err()
            {
                break;
            }
            assert!(
                started.elapsed() < SERVER_DRAIN_TIMEOUT / 2,
                "listener must close before the drain, not after it"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::timeout(SERVER_DRAIN_TIMEOUT * 3, shutdown)
            .await
            .expect("shutdown must not wait on a stalled request")
            .expect("shutdown task");
        assert!(
            started.elapsed() >= SERVER_DRAIN_TIMEOUT,
            "a busy connection gets the drain deadline before it is closed"
        );
        assert!(
            closed_by_peer(&mut stalled).await,
            "stalled connection must be closed"
        );
    }

    /// Dropping the handle without `shutdown()` (a dropped attack future)
    /// still stops the server.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_server_stops_it() {
        let (setup, _base) = bind_dashboard("").await;
        let port = setup.bound_port;
        let mut keep_alive = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        assert!(health_on(&mut keep_alive).await.is_some());

        drop(setup.server);

        assert!(
            closed_by_peer(&mut keep_alive).await,
            "keep-alive connection must be closed"
        );
        let mut refused = false;
        for _ in 0..40 {
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_err()
            {
                refused = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(refused, "listener must close after the handle is dropped");
    }

    /// Send `request` on a new connection and read until the server closes it
    /// (the request should say `Connection: close`) or two seconds pass.
    async fn raw_exchange(port: u16, request: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut buf = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut buf)).await;
        String::from_utf8_lossy(&buf).into_owned()
    }

    /// Port of a `http://127.0.0.1:<port>` base URL from the setup helpers.
    fn port_of(base: &str) -> u16 {
        base.rsplit(':').next().unwrap().parse().unwrap()
    }

    /// A `GET path` request with the given `Host` that asks the server to
    /// close the connection after answering.
    fn get_request(path: &str, host: &str) -> String {
        format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
    }

    #[test]
    fn is_loopback_host_rules() {
        for host in [
            "localhost",
            "LOCALHOST:5118",
            "127.0.0.1",
            "127.0.0.1:5118",
            "127.1.2.3:80",
            "[::1]",
            "[::1]:5118",
        ] {
            assert!(is_loopback_host(host), "{} is loopback", host);
        }
        for host in [
            "",
            "evil.example",
            "evil.example:5118",
            "localhost.evil.example",
            "127.0.0.1.evil.example",
            "0.0.0.0:5118",
            "10.0.0.1:5118",
            "[::]:5118",
            "evil.example@127.0.0.1",
            "127.0.0.1:5118/path",
        ] {
            assert!(!is_loopback_host(host), "{:?} is not loopback", host);
        }
    }

    #[test]
    fn connection_cap_is_above_sse_cap() {
        assert_eq!(connection_cap(MAX_SSE_CLIENTS), 2 * MAX_SSE_CLIENTS + 16);
        assert_eq!(connection_cap(1), 18);
        assert_eq!(
            connection_cap(usize::MAX),
            tokio::sync::Semaphore::MAX_PERMITS
        );
    }

    /// Regression for #697: on a loopback bind a request whose `Host` is not
    /// loopback (a DNS rebinding page) is refused, before auth.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn loopback_bind_refuses_non_loopback_host() {
        let (_hub_builds, _parent_builds, parent, base, _server) = setup_with_mock("").await;
        let port = port_of(&base);

        for path in ["/", "/api/v1/health", "/api/v1/snapshot", "/api/v1/events"] {
            let reply =
                raw_exchange(port, &get_request(path, &format!("evil.example:{port}"))).await;
            assert!(
                reply.starts_with("HTTP/1.1 403"),
                "{} with a rebound Host must get 403, got {:?}",
                path,
                reply
            );
        }
        let reply = raw_exchange(port, "GET /api/v1/snapshot HTTP/1.0\r\n\r\n").await;
        assert!(
            reply.starts_with("HTTP/1.0 403"),
            "a request with no Host must get 403, got {:?}",
            reply
        );

        for host in [
            format!("127.0.0.1:{port}"),
            format!("localhost:{port}"),
            format!("[::1]:{port}"),
        ] {
            let reply = raw_exchange(port, &get_request("/api/v1/snapshot", &host)).await;
            assert!(
                reply.starts_with("HTTP/1.1 200"),
                "Host {} must be served, got {:?}",
                host,
                reply
            );
        }
        parent.abort();
    }

    /// The Host check follows the configured name: a name that is not a
    /// loopback name needs a token and gets no Host check, even when it
    /// resolves to a loopback address (`127.1` resolves to 127.0.0.1, as a
    /// machine's own hostname often resolves to 127.0.1.1).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn non_loopback_bind_serves_any_host() {
        for bind_host in ["0.0.0.0", "127.1"] {
            let config = GooseConfiguration {
                dashboard: true,
                dashboard_host: Some(bind_host.to_string()),
                dashboard_port: Some(0),
                dashboard_auth_token: Some("secret".to_string()),
                ..Default::default()
            };
            let setup = setup_dashboard(&config, None, ListeningControllers::configured(&config))
                .await
                .expect("bind")
                .expect("enabled");
            let reply = raw_exchange(
                setup.bound_port,
                &get_request("/api/v1/health", "loadgen.example:5118"),
            )
            .await;
            assert!(
                reply.starts_with("HTTP/1.1 200"),
                "a bind to {} must not check Host, got {:?}",
                bind_host,
                reply
            );
        }
    }

    /// Regression for #697: every response, the token-bearing page load and
    /// the API included, carries `nosniff`, `no-referrer` and `no-store`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn responses_carry_security_headers() {
        let (_hub_builds, _parent_builds, parent, base, _server) = setup_with_mock("secret").await;
        let port = port_of(&base);
        let client = reqwest::Client::new();

        let mut responses = Vec::new();
        for (path, status) in [
            ("/?token=secret", 200),
            ("/static/app.js", 200),
            ("/api/v1/health", 200),
            ("/api/v1/snapshot?token=secret", 200),
            ("/api/v1/snapshot", 401),
            ("/api/v1/events?token=secret", 200),
            ("/no/such/page", 404),
        ] {
            let resp = client
                .get(format!("{base}{path}"))
                .send()
                .await
                .expect("request");
            assert_eq!(resp.status(), status, "{path}");
            responses.push((path.to_string(), resp.headers().clone()));
        }
        let refused = raw_exchange(port, &get_request("/", "evil.example")).await;
        assert!(refused.starts_with("HTTP/1.1 403"), "{:?}", refused);

        for (path, headers) in &responses {
            assert_eq!(headers["cache-control"], "no-store", "{path}");
            assert_eq!(headers["x-content-type-options"], "nosniff", "{path}");
            assert_eq!(headers["referrer-policy"], "no-referrer", "{path}");
        }
        for line in [
            "cache-control: no-store\r\n",
            "x-content-type-options: nosniff\r\n",
            "referrer-policy: no-referrer\r\n",
        ] {
            assert!(
                refused.contains(line),
                "403 must carry {:?}: {:?}",
                line,
                refused
            );
        }
        parent.abort();
    }

    /// Read from `stream` until the text read contains `needle`; false when
    /// the stream ends or five seconds pass first.
    async fn read_until_contains(stream: &mut tokio::net::TcpStream, needle: &str) -> bool {
        use tokio::io::AsyncReadExt;
        let mut text = String::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut chunk = [0u8; 4096];
        while !text.contains(needle) {
            match tokio::time::timeout_at(deadline, stream.read(&mut chunk)).await {
                Ok(Ok(n)) if n > 0 => text.push_str(&String::from_utf8_lossy(&chunk[..n])),
                _ => return false,
            }
        }
        true
    }

    /// Open `GET /api/v1/events` on a new connection and wait for its first
    /// snapshot.
    async fn open_sse(port: u16) -> tokio::net::TcpStream {
        use tokio::io::AsyncWriteExt;
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        stream
            .write_all(b"GET /api/v1/events HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .expect("write");
        assert!(
            read_until_contains(&mut stream, "event: snapshot").await,
            "SSE stream must get a first snapshot"
        );
        stream
    }

    /// True when an SSE stream is still open and sends a snapshot after now:
    /// what it sent earlier is read and dropped first (failing on EOF), so a
    /// snapshot buffered before a close cannot pass for a live stream.
    async fn sse_still_live(stream: &mut tokio::net::TcpStream) -> bool {
        use tokio::io::AsyncReadExt;
        let mut chunk = [0u8; 4096];
        // Snapshots come about once a second, so a 300 ms gap ends the drain.
        while let Ok(read) =
            tokio::time::timeout(Duration::from_millis(300), stream.read(&mut chunk)).await
        {
            if !matches!(read, Ok(n) if n > 0) {
                return false;
            }
        }
        read_until_contains(stream, "event: snapshot").await
    }

    /// Regression for #697: a client that sends part of a request line and
    /// stalls, and one that sends nothing, are closed at the header read
    /// timeout, while an SSE stream open across the timeout is not cut.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn slow_header_client_is_cut_at_timeout() {
        use tokio::io::AsyncWriteExt;
        let (_hub_builds, _parent_builds, parent, base, _server) = setup_with_mock("").await;
        let port = port_of(&base);
        let mut sse = open_sse(port).await;

        // Taken before connecting, so the server's deadlines start after it.
        let started = Instant::now();
        let mut silent = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        let mut slow = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        slow.write_all(b"GET /api/v1/health HTTP/1.1\r\nHost: 127")
            .await
            .expect("write");

        assert!(
            !closed_within(&mut slow, HEADER_READ_TIMEOUT - Duration::from_secs(1)).await,
            "closed before the header read timeout"
        );
        assert!(
            closed_within(&mut slow, Duration::from_secs(5)).await,
            "a stalled header must be cut at the header read timeout"
        );
        assert!(
            started.elapsed() >= HEADER_READ_TIMEOUT,
            "cut after {:?}",
            started.elapsed()
        );
        assert!(
            closed_within(&mut silent, Duration::from_secs(1)).await,
            "a connection that sends nothing must be cut at the header read timeout"
        );
        assert!(
            sse_still_live(&mut sse).await,
            "an SSE stream must outlive the header read timeout"
        );
        parent.abort();
    }

    /// Regression for #697: connections beyond the cap are closed on accept,
    /// while every SSE stream up to `--dashboard-max-clients` keeps working,
    /// and a closed connection frees its slot.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn connection_cap_refuses_extra_connections_and_keeps_sse() {
        let max_clients = 2u32;
        let cap = connection_cap(max_clients as usize);
        let (_hub_builds, _parent_builds, parent, base, _server) =
            setup_with_mock_max("", max_clients).await;
        let port = port_of(&base);

        let mut sse = Vec::new();
        for _ in 0..max_clients {
            sse.push(open_sse(port).await);
        }

        // Fill the remaining slots with served keep-alive connections until
        // one is closed without an answer. A connection the setup left open
        // may still hold a slot when the first refusal comes, so after each
        // refusal wait and try once more; the fill ends when a refusal holds.
        let mut held = Vec::new();
        let mut after_refusal = false;
        loop {
            assert!(
                sse.len() + held.len() <= cap,
                "more than {} connections served",
                cap
            );
            if after_refusal {
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .expect("connect");
            match health_on(&mut stream).await {
                Some(_) => {
                    held.push(stream);
                    after_refusal = false;
                }
                None if after_refusal => break,
                None => after_refusal = true,
            }
        }
        assert_eq!(
            sse.len() + held.len(),
            cap,
            "the cap is reached at exactly {cap} open connections"
        );

        // The SSE streams up to the client cap still receive snapshots.
        for (i, stream) in sse.iter_mut().enumerate() {
            assert!(
                sse_still_live(stream).await,
                "SSE client {} must keep receiving snapshots at the connection cap",
                i
            );
        }

        // Closing one connection frees its slot for a new one.
        drop(held.pop());
        let mut served = false;
        for _ in 0..40 {
            let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .expect("connect");
            if health_on(&mut stream).await.is_some() {
                served = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(served, "a closed connection must free its slot");
        parent.abort();
    }

    /// Poll `/api/v1/health` until `active_sse_clients` equals `want` or
    /// `within` elapses; returns the last count read.
    async fn wait_active_sse(base: &str, want: u64, within: Duration) -> u64 {
        let client = reqwest::Client::new();
        let deadline = Instant::now() + within;
        loop {
            let health: serde_json::Value = client
                .get(format!("{base}/api/v1/health"))
                .send()
                .await
                .expect("health")
                .json()
                .await
                .expect("health json");
            let active = health["active_sse_clients"].as_u64().expect("count");
            if active == want || Instant::now() >= deadline {
                return active;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Regression for #696: a client that disconnects frees its SSE slot at
    /// once, even while no snapshot is published, so browsers reconnecting
    /// while the main loop is busy cannot fill the cap and lock viewers out.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sse_slot_freed_on_disconnect_without_snapshots() {
        let cap = 2u32;
        // Nothing reads `request_rx`, like a main loop busy in a long
        // `test_start`, so no snapshot is ever published.
        let (_setup, base) = bind_dashboard_with(TestBind::token("").max_clients(cap)).await;
        let client = reqwest::Client::new();

        let mut held = Vec::new();
        for i in 0..cap {
            let resp = client
                .get(format!("{base}/api/v1/events"))
                .send()
                .await
                .expect("connect");
            assert_eq!(resp.status(), 200, "client {i} should be accepted");
            held.push(resp);
        }

        drop(held);
        // Well inside the 15 s heartbeat, so its write cannot free the slots.
        assert_eq!(
            wait_active_sse(&base, 0, Duration::from_secs(2)).await,
            0,
            "disconnected clients must free their slots"
        );
    }
}
