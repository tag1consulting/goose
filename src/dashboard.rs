//! Live web dashboard HTTP server.
//!
//! Compiled only with the `dashboard` crate feature. Runs an HTTP/1 accept
//! loop over an axum router that serves a minimal static shell, a one-shot
//! metrics snapshot API, an SSE stream of coalesced snapshots, and (when
//! `--dashboard-control` is set) authenticated Start/Stop/Users/Quit control
//! endpoints and saved run deletion.
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
    RunSettings, SendError, CONTROL_TIMEOUT,
};
use crate::metrics::dashboard_snapshot::{
    Heatmap, HeatmapColumn, HeatmapCursor, RequestSeries, SnapshotSave, SnapshotSettings,
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
use std::sync::{Arc, Mutex, RwLock};
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
    /// The data of every `closed` event: the save state as JSON. The main
    /// loop keeps it current, so it is right before the close signal fires.
    pub closed_data: ClosedData,
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

/// The data of the SSE `closed` event, shared by the hub and the main loop.
#[derive(Clone, Debug)]
pub(crate) struct ClosedData(Arc<RwLock<String>>);

impl ClosedData {
    fn new(save: &SnapshotSave) -> Self {
        let data = ClosedData(Arc::new(RwLock::new(String::new())));
        data.set(save);
        data
    }

    /// Replace the save state sent with `closed`.
    pub(crate) fn set(&self, save: &SnapshotSave) {
        let json = serde_json::to_string(save).unwrap_or_else(|_| "{}".to_string());
        match self.0.write() {
            Ok(mut data) => *data = json,
            Err(poisoned) => *poisoned.into_inner() = json,
        }
    }

    fn get(&self) -> String {
        match self.0.read() {
            Ok(data) => data.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
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
    buckets: Vec<[u64; 3]>,
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
            buckets: Vec::new(),
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
        if heatmap.buckets.len() >= self.buckets.len() {
            self.buckets = heatmap.buckets.clone();
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
            buckets: self.buckets.clone(),
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
    /// The data of the `closed` event.
    closed_data: ClosedData,
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
        closed_data: ClosedData,
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
            closed_data,
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

    /// The data to send with `closed`.
    fn closed_data(&self) -> String {
        self.inner.closed_data.get()
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
#[derive(Clone, Debug)]
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
    /// Auth token; empty means observe auth is disabled (loopback-only runs).
    auth_token: String,
    /// Concurrent control POSTs currently waiting on the main loop.
    control_in_flight: Arc<AtomicUsize>,
    /// Concurrent `GET /api/v1/series` requests waiting on the main loop.
    series_in_flight: Arc<AtomicUsize>,
    /// Recent `GET /api/v1/series` 200 answers by method and name, with when
    /// each was built.
    series_cache: Arc<Mutex<SeriesCache>>,
    /// The runs directory as given, or the default: the saved run routes
    /// read it, and only them.
    runs_dir: String,
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
    /// Where the telnet and WebSocket Controllers listen.
    controllers: HealthControllers,
}

/// The `host:port` each Controller is configured to listen on, or `None` when
/// it is off (`--no-telnet`, `--no-websocket`).
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
    // Until the main loop sets it: what the configuration says.
    let closed_data = ClosedData::new(&SnapshotSave {
        state: if configuration.saves_runs() {
            "on"
        } else {
            "off"
        }
        .to_string(),
        reason: None,
        dir: configuration.runs_dir_or_default().to_string(),
        last_run: None,
    });
    let max_sse_clients = configuration.dashboard_max_clients();
    // Clone sender for the hub and the series handler (same channel).
    let hub = SnapshotHub::new(
        request_tx.clone(),
        Arc::clone(&build_count),
        close_tx.clone(),
        max_sse_clients,
        Arc::clone(&active_sse),
        closed_data.clone(),
    );
    let control_enabled = control_tx.is_some();
    let state = DashboardState {
        hub,
        request_tx,
        control_tx,
        auth_token: configuration
            .dashboard_auth_token()
            .unwrap_or_default()
            .to_string(),
        control_in_flight: Arc::new(AtomicUsize::new(0)),
        series_in_flight: Arc::new(AtomicUsize::new(0)),
        series_cache: Arc::new(Mutex::new(HashMap::new())),
        runs_dir: configuration.runs_dir_or_default().to_string(),
        controllers: HealthControllers {
            telnet: (!configuration.no_telnet).then(|| {
                format_bind_address(configuration.telnet_host(), configuration.telnet_port())
            }),
            websocket: (!configuration.no_websocket).then(|| {
                format_bind_address(
                    configuration.websocket_host(),
                    configuration.websocket_port(),
                )
            }),
        },
    };

    // Loopback by the configured name, the same rule that lets the dashboard
    // run without a token. Any other name needs a token, and a hostname that
    // resolves to 127.0.1.1 must still accept its own name as `Host`.
    let app = build_router(state, crate::config::is_loopback_bind(host));
    let max_connections = connection_cap(max_sse_clients);

    if control_enabled {
        info!("[dashboard]: listening on http://{bound} (control enabled)");
        info!("[dashboard]: control POST /api/v1/control/* requires Authorization: Bearer <token>");
    } else {
        info!("[dashboard]: listening on http://{bound} (read-only)");
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
        closed_data,
        url: format!("http://{bound}"),
        close_tx,
        server,
    }))
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
/// (otherwise POST /api/v1/control/* and DELETE /api/v1/runs/{id} → 404). On a loopback bind every request
/// must carry a loopback `Host`, so a page that rebinds its own domain to
/// 127.0.0.1 cannot read the API.
fn build_router(state: DashboardState, loopback_bind: bool) -> Router {
    let control_enabled = state.control_tx.is_some();
    let mut router = Router::new()
        .route("/", get(index_handler))
        .route("/static/app.js", get(app_js_handler))
        .route("/static/app.css", get(app_css_handler))
        .route("/api/v1/health", get(health_handler))
        .route("/api/v1/snapshot", get(snapshot_handler))
        .route("/api/v1/series", get(series_handler))
        .route("/api/v1/events", get(events_handler))
        .route("/api/v1/runs", get(runs_handler))
        .route("/api/v1/runs/{id}/{file}", get(run_file_handler));

    if control_enabled {
        // Bound control request bodies so a large POST cannot blow memory/CPU
        // on the dashboard task (start/stop expect empty/`{}`; users is small).
        let control_routes = Router::new()
            .route("/api/v1/control/start", post(control_start_handler))
            .route("/api/v1/control/stop", post(control_stop_handler))
            .route("/api/v1/control/quit", post(control_quit_handler))
            .route("/api/v1/runs/{id}", delete(delete_run_handler))
            .route("/api/v1/control/users", post(control_users_handler))
            .route("/api/v1/control/settings", post(control_settings_handler))
            .layer(axum::extract::DefaultBodyLimit::max(CONTROL_BODY_LIMIT));
        router = router.merge(control_routes);
    }

    if loopback_bind {
        // Added before the header layers so a refusal carries them too.
        router = router.layer(axum::middleware::from_fn(require_loopback_host));
    }

    router
        // `if_not_present`: a saved run download sets its own
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
        return closed_sse_response(&state.hub);
    }

    if !state.hub.try_acquire_sse() {
        // Distinguish full cap vs raced into shutdown.
        if state.hub.is_closed() {
            return closed_sse_response(&state.hub);
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
        let guard = SseGuard(hub);
        let hub = &guard.0;

        // watch does not notify for the value present at subscribe time — check
        // sticky shutdown before pushing a stale snapshot or blocking forever.
        if *shutdown_rx.borrow() {
            let _ = event_tx.send(closed_event(hub)).await;
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
                        let _ = event_tx.send(closed_event(hub)).await;
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
                        let _ = event_tx.send(closed_event(hub)).await;
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
fn closed_sse_response(hub: &SnapshotHub) -> Response<Body> {
    let event = closed_event(hub);
    let stream = stream::once(async move { Ok::<_, Infallible>(event) });
    Sse::new(stream).into_response()
}

/// The terminal `closed` event; its data is the save state as JSON.
fn closed_event(hub: &SnapshotHub) -> Event {
    Event::default().event("closed").data(hub.closed_data())
}

fn snapshot_event(bytes: &Bytes) -> Event {
    let data = std::str::from_utf8(bytes).unwrap_or("{}");
    Event::default().event("snapshot").data(data)
}

/// Query of the saved run routes: the token, and for a comparison the
/// baseline run.
#[derive(Debug, Deserialize)]
struct RunQuery {
    token: Option<String>,
    baseline: Option<String>,
}

/// The comparison a run route serves besides its report files.
const COMPARE_MD: &str = "compare.md";

/// `GET /api/v1/runs`: every complete saved run, newest first. Read from disk
/// on each request, never on the snapshot path.
async fn runs_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Response<Body> {
    if !authorize(&state.auth_token, &headers, query.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let dir = state.runs_dir.clone();
    let listed = tokio::task::spawn_blocking(move || runs::list_runs(Path::new(&dir), &dir)).await;
    match listed {
        Ok(Ok(listing)) => Json(listing).into_response(),
        Ok(Err(e)) => {
            warn!(
                "[dashboard]: can't list saved runs in {}: {e}",
                state.runs_dir
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "can't list saved runs").into_response()
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "can't list saved runs").into_response(),
    }
}

/// `GET /api/v1/runs/{id}/{file}`: one of a saved run's reports, or with
/// `file` `compare.md` and `?baseline={id}`, the run compared to the baseline
/// run as Markdown. Anything that is not a valid run id, a known file, a real
/// directory and a regular file is a plain 404.
async fn run_file_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    path: Result<axum::extract::Path<(String, String)>, axum::extract::rejection::PathRejection>,
    Query(query): Query<RunQuery>,
) -> Response<Body> {
    if !authorize(&state.auth_token, &headers, query.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(axum::extract::Path((id, file))) = path else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !runs::is_valid_run_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if file == COMPARE_MD {
        return compare_response(&state.runs_dir, id, query.baseline).await;
    }
    let Some(&name) = runs::REPORT_FILES.iter().find(|&&name| name == file) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    report_file_response(&state.runs_dir, &id, name).await
}

/// The run directory for `id`, if it is a real directory holding `file` and
/// `run.json` as regular files: a run still being written, or whose save
/// failed, has no `run.json` and is not served. `id` must already be valid
/// and `file` one of the fixed names.
fn run_file_path(
    runs_dir: &str,
    id: &str,
    file: &str,
) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let run_dir = Path::new(runs_dir).join(id);
    let path = run_dir.join(file);
    if runs::is_real_dir(&run_dir)
        && runs::is_regular_file(&run_dir.join(runs::RUN_JSON))
        && runs::is_regular_file(&path)
    {
        Some((run_dir, path))
    } else {
        None
    }
}

/// `test` from a run's `run.json`, reduced to `[A-Za-z0-9_-]` for a file name.
fn download_test_name(run_dir: &Path) -> String {
    runs::read_run_info(run_dir)
        .map(|info| {
            info.test
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

/// Stream a report file from disk.
async fn report_file_response(runs_dir: &str, id: &str, name: &'static str) -> Response<Body> {
    let Some((run_dir, path)) = run_file_path(runs_dir, id, name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(file) = tokio::fs::File::open(&path).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // The file opened must be the regular file checked above, not something
    // swapped in since.
    let Ok(metadata) = file.metadata().await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !metadata.is_file() || !same_file(&path, &metadata) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let (content_type, ext) = match name {
        runs::REPORT_HTML => ("text/html; charset=utf-8", "html"),
        runs::REPORT_JSON => ("application/json", "json"),
        _ => ("text/markdown; charset=utf-8", "md"),
    };
    let test = download_test_name(&run_dir);
    download_response(
        content_type,
        &format!("goose-{test}-{id}.{ext}"),
        Body::from_stream(file_stream(file)),
        Some(metadata.len()),
    )
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

/// The Markdown comparison of run `id` (the newer) to `baseline`.
async fn compare_response(runs_dir: &str, id: String, baseline: Option<String>) -> Response<Body> {
    let Some(baseline) = baseline.filter(|b| runs::is_valid_run_id(b)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (Some((run_dir, run_json)), Some((baseline_dir, baseline_json))) = (
        run_file_path(runs_dir, &id, runs::REPORT_JSON),
        run_file_path(runs_dir, &baseline, runs::REPORT_JSON),
    ) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (run_id, baseline_id) = (id.clone(), baseline.clone());
    let run_dir_for_names = run_dir.clone();
    let compared = tokio::task::spawn_blocking(move || {
        let report =
            crate::metrics::compare_reports(&run_json, &run_id, &baseline_json, &baseline_id)?;
        let mut out = compared_runs_line(&run_dir_for_names, &run_id, &baseline_dir, &baseline_id)
            .into_bytes();
        out.extend_from_slice(b"\n\n");
        out.extend(report);
        Ok::<_, String>(out)
    })
    .await;
    match compared {
        Ok(Ok(markdown)) => {
            let test = download_test_name(&run_dir);
            let length = markdown.len() as u64;
            download_response(
                "text/markdown; charset=utf-8",
                &format!("goose-{test}-{id}-vs-{baseline}.md"),
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
/// with when each started, from their `run.json`.
fn compared_runs_line(
    run_dir: &Path,
    run_id: &str,
    baseline_dir: &Path,
    baseline_id: &str,
) -> String {
    let started = |dir: &Path| {
        runs::read_run_info(dir)
            .map(|info| {
                chrono::DateTime::parse_from_rfc3339(&info.started)
                    .map(|time| {
                        time.with_timezone(&chrono::Utc)
                            .format("%Y-%m-%d %H:%M:%S UTC")
                            .to_string()
                    })
                    .unwrap_or(info.started)
            })
            .unwrap_or_else(|| "at an unknown time".to_string())
    };
    format!(
        "Run {run_id} (started {}) compared to baseline {baseline_id} (started {}).",
        started(run_dir),
        started(baseline_dir)
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
    if !authorize_control(&state.auth_token, headers, query.token.as_deref()) {
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
    if !authorize_control(&state.auth_token, headers, query.token.as_deref()) {
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

/// `DELETE /api/v1/runs/{id}`: remove a complete saved run from disk.
/// Registered only with `--dashboard-control`, and authorized like the
/// control routes. 204 when removed; a plain 404 for anything that is not a
/// valid run id naming a real directory that holds a regular `run.json`,
/// which includes the run being written now: it has no `run.json` until it
/// ends.
async fn delete_run_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    path: Result<axum::extract::Path<String>, axum::extract::rejection::PathRejection>,
    Query(query): Query<TokenQuery>,
) -> Response<Body> {
    if !authorize_control(&state.auth_token, &headers, query.token.as_deref()) {
        warn!("[dashboard]: delete run auth failure");
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(axum::extract::Path(id)) = path else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !runs::is_valid_run_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let run_dir = Path::new(&state.runs_dir).join(&id);
    // `remove_dir_all` removes a symlink inside the run directory, never
    // what it points to.
    let removed = tokio::task::spawn_blocking(move || {
        if !runs::is_real_dir(&run_dir) || !runs::is_regular_file(&run_dir.join(runs::RUN_JSON)) {
            return Ok(false);
        }
        std::fs::remove_dir_all(&run_dir).map(|()| true)
    })
    .await;
    match removed {
        Ok(Ok(true)) => {
            info!("[dashboard]: deleted saved run {id}");
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(Ok(false)) => StatusCode::NOT_FOUND.into_response(),
        Ok(Err(e)) => {
            warn!("[dashboard]: can't delete saved run {id}: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "can't delete the run").into_response()
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
    if !authorize_control(&state.auth_token, &headers, query.token.as_deref()) {
        warn!("[dashboard]: control users auth failure");
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match parse_users_body(&body) {
        Ok(command) => dispatch_control(&state, command, "users").await,
        Err((code, msg)) => control_http_error(StatusCode::BAD_REQUEST, "users", code, msg),
    }
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
                co_active: false,
                co_adjusted: None,
            },
            requests: vec![],
            errors: vec![],
            scenarios: vec![],
            transactions: vec![],
            series: SeriesWindow::empty(),
            flags: SnapshotFlags {
                metrics_disabled: false,
                transaction_metrics_disabled: false,
                scenario_metrics_disabled: false,
                requests_truncated: false,
                errors_truncated: false,
                series_seconds: 300,
                percentiles_over_time: false,
            },
            save: SnapshotSave::off("goose-runs"),
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
        bind_dashboard_with(token, 0, None).await
    }

    async fn bind_dashboard_with(
        token: &str,
        max_clients: u32,
        control: Option<flume::Sender<ControlRequest>>,
    ) -> (DashboardSetup, String) {
        let config = GooseConfiguration {
            dashboard: true,
            dashboard_control: control.is_some(),
            dashboard_host: Some("127.0.0.1".to_string()),
            dashboard_port: Some(0),
            dashboard_auth_token: Some(token.to_string()),
            // 0 leaves the cap at its default.
            dashboard_max_clients: (max_clients > 0).then_some(max_clients),
            ..Default::default()
        };
        let setup = setup_dashboard(&config, control)
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
        let (setup, base) = bind_dashboard_with(token, max_clients, None).await;
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
        let (setup, base) = bind_dashboard_with("s3cret", 0, Some(control_tx)).await;
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
        let setup = setup_dashboard(&config, None)
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
        match setup_dashboard(&config, None).await {
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
        let err = setup_dashboard(&config, None)
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
            ClosedData::new(&SnapshotSave::off("goose-runs")),
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
        // (no_telnet, no_websocket, expected controllers). Hosts and ports are
        // set as setup_controllers leaves them; an IPv6 host is bracketed.
        let cases = [
            (
                false,
                false,
                serde_json::json!({"telnet": "0.0.0.0:5116", "websocket": "[::1]:6117"}),
            ),
            (
                true,
                false,
                serde_json::json!({"telnet": null, "websocket": "[::1]:6117"}),
            ),
            (
                true,
                true,
                serde_json::json!({"telnet": null, "websocket": null}),
            ),
        ];
        let client = reqwest::Client::new();
        for (no_telnet, no_websocket, expected) in cases {
            let config = GooseConfiguration {
                dashboard: true,
                dashboard_host: Some("127.0.0.1".to_string()),
                dashboard_port: Some(0),
                no_telnet,
                telnet_host: Some("0.0.0.0".to_string()),
                telnet_port: Some(5116),
                no_websocket,
                websocket_host: Some("::1".to_string()),
                websocket_port: Some(6117),
                ..Default::default()
            };
            let setup = setup_dashboard(&config, None)
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
                "no_telnet {}, no_websocket {}",
                no_telnet, no_websocket
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
            ClosedData::new(&SnapshotSave::off("goose-runs")),
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn control_disabled_returns_404_and_health_false() {
        let (setup, base) = bind_dashboard_with("s3cret", 0, None).await;
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
        let (control_tx, control_rx) = flume::unbounded();
        let (setup, base) = bind_dashboard_with("s3cret", 0, Some(control_tx)).await;
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
            buckets: vec![[0, 0, 0]; buckets],
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
        assert_eq!(whole.buckets.len(), 206);
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
        let (setup, base) = bind_dashboard_with("s3cret", 0, Some(control_tx)).await;
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
        let (_setup, base) = bind_dashboard_with("s3cret", 0, Some(control_tx)).await;
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
        let (_setup, base) = bind_dashboard_with("s3cret", 0, Some(control_tx)).await;
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
            let (setup, base) = bind_dashboard_with("s3cret", 0, Some(control_tx)).await;
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
            let (setup, base) = bind_dashboard_with("s3cret", 0, Some(control_tx)).await;
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
            let (setup, base) = bind_dashboard_with("s3cret", 0, Some(control_tx)).await;
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
            let (setup, base) = bind_dashboard_with("s3cret", 0, Some(control_tx)).await;
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
        let (setup, _base) = bind_dashboard_with("s3cret", 0, Some(control_tx)).await;
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
            let setup = setup_dashboard(&config, None)
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
        let (_setup, base) = bind_dashboard_with("", cap, None).await;
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
