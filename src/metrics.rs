//! Optional metrics collected and aggregated during load tests.
//!
//! By default, Goose collects a large number of metrics while performing a load test.
//! When [`GooseAttack::execute()`](../struct.GooseAttack.html#method.execute) completes
//! it returns a [`GooseMetrics`] object.
//!
//! When the [`GooseMetrics`] object is viewed with [`std::fmt::Display`], the
//! contained [`TransactionMetrics`], [`GooseRequestMetrics`], and
//! [`GooseErrorMetrics`] are displayed in tables.

pub(crate) mod buckets;
pub(crate) mod common;
pub mod coordinated_omission;
pub(crate) mod dashboard_snapshot;
mod delta;
pub(crate) mod millis;
mod nullable;

#[cfg(test)]
mod microsecond_tests;

pub use buckets::TimeBuckets;
#[cfg(feature = "dashboard")]
pub(crate) use common::compare_reports;
#[cfg(test)]
pub(crate) use common::json_report_path;
#[cfg(any(test, feature = "dashboard"))]
pub(crate) use common::read_report_text;
pub(crate) use common::{load_baseline_file, LoadedReport, ReportData};
pub use coordinated_omission::{CadenceCalculator, CoMetricsSummary, CoordinatedOmissionMetrics};
pub(crate) use dashboard_snapshot::DashboardSnapshot;
pub(crate) use delta::{ApplyBaseline, DeltaTo, Value};
pub(crate) use millis::{format_micros, format_ms, Millis, MillisFloat};
pub(crate) use nullable::NullableFloat;

use crate::config::GooseDefaults;
use crate::goose::{get_base_url, GooseMethod, Scenario, TransactionName};
use crate::graph::GraphData;
use crate::logger::{GooseLog, GooseLoggerTx};
use crate::metrics::common::ReportOptions;
use crate::report;
use crate::runs::RunRecord;
use crate::test_plan::{TestPlanHistory, TestPlanStepAction};
use crate::threshold::{self, ThresholdResult};
use crate::util;
use crate::{GooseAttack, GooseAttackRunState, GooseConfiguration, GooseError};
use chrono::prelude::*;
/// The report data builder, for the whole run audit in `goose.rs`.
#[cfg(test)]
pub(crate) use common::{prepare_data_with_baseline, ReportOptions as AuditReportOptions};
use itertools::Itertools;
use num_format::{Locale, ToFormattedString};
use regex::RegexSet;
use reqwest::StatusCode;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Serde helper for `Arc<str>` fields, enabling `#[serde(with = "arc_str_serde")]`.
pub(crate) mod arc_str_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::sync::Arc;

    pub fn serialize<S: Serializer>(value: &Arc<str>, serializer: S) -> Result<S::Ok, S::Error> {
        value.as_ref().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Arc<str>, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Arc::from(s.as_str()))
    }
}
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::fmt;
use std::fmt::Write;
use std::io::{BufWriter, Write as _};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::fs::File;

/// Used to send metrics from [`GooseUser`](../goose/struct.GooseUser.html) threads
/// to the parent Goose process.
///
/// [`GooseUser`](../goose/struct.GooseUser.html) threads send these metrics to the
/// Goose parent process using an
/// [`unbounded Flume channel`](https://docs.rs/flume/*/flume/fn.unbounded.html).
///
/// The parent process will spend up to 80% of its time receiving and aggregating
/// these metrics. The parent process aggregates [`GooseRequestMetric`]s into
/// [`GooseRequestMetricAggregate`], [`TransactionMetric`]s into [`TransactionMetricAggregate`],
/// and [`GooseErrorMetric`]s into [`GooseErrorMetricAggregate`]. Aggregation happens in the
/// parent process so the individual [`GooseUser`](../goose/struct.GooseUser.html) threads
/// can spend all their time generating and validating load.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GooseMetric {
    Request(Box<GooseRequestMetric>),
    Transaction(TransactionMetric),
    Scenario(ScenarioMetric),
    /// Combined metric carrying all three payloads in a single channel message.
    ///
    /// Emitted when a scenario iteration completes with exactly one pending request
    /// metric and one pending transaction metric, avoiding three separate channel sends.
    ///
    /// The request and the scenario are each boxed, so the variant, and with
    /// it every message, stays small.
    All {
        request: Box<GooseRequestMetric>,
        transaction: TransactionMetric,
        scenario: Box<ScenarioMetric>,
    },
    /// A batch of pre-aggregated metrics from a single GooseUser thread.
    ///
    /// Reduces channel pressure from `O(RPS)` to `O(RPS / batch_size)` by
    /// accumulating metrics locally in each user thread and flushing them as a
    /// single channel message. The metrics processor merges batch entries
    /// into global aggregates using the same logic as individual metrics.
    Batch(Box<GooseMetricBatch>),
}

/// Maximum number of request metrics to accumulate before flushing a batch.
pub(crate) const METRICS_BATCH_SIZE: usize = 100;
/// Maximum number of metrics channel messages the processor handles in one
/// turn before it checks for a command, and that a dashboard snapshot or
/// series request drains before it is built.
///
/// Each message can be a [`GooseMetric::Batch`] of up to [`METRICS_BATCH_SIZE`]
/// requests. Measured with a release build on an Apple Silicon Mac, a batch
/// of 100 successful requests over 20 request names takes about 4.5µs to
/// process, and a batch of 100 failed requests, which are processed one by
/// one, about 86µs. A turn of 256 messages therefore takes about 1.1ms, and
/// about 22ms in the worst case, so a command waits at most tens of
/// milliseconds even while user threads keep the channel full.
pub(crate) const METRICS_DRAIN_LIMIT: usize = 256;
/// Maximum age of a batch before it is flushed, regardless of size.
pub(crate) const METRICS_BATCH_MAX_AGE: std::time::Duration = std::time::Duration::from_millis(250);

/// Shared epoch counter for coordinating batch validity across metrics resets.
///
/// When a metrics reset occurs, the processor increments the epoch. Batches
/// stamped with a previous epoch are discarded, preventing stale pre-reset
/// data from leaking into post-reset metrics.
pub(crate) type MetricsEpoch = Arc<AtomicU64>;

/// A batch of pre-aggregated metrics from a single
/// [`GooseUser`](crate::goose::GooseUser) thread.
///
/// Instead of sending one channel message per request, each user thread
/// accumulates metrics locally into this struct and flushes it when the batch
/// reaches 100 requests or 250 milliseconds have elapsed. The metrics
/// processor merges pre-aggregated entries into the global [`GooseMetrics`]
/// using the same aggregation logic.
///
/// Successful, non-CO requests are pre-aggregated (timing data, status codes).
/// Failed requests and CO-affected requests are buffered individually since
/// they require per-request processing (error logging, CO backfill).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GooseMetricBatch {
    /// The metrics epoch when this batch was created. Batches with an epoch
    /// older than the processor's current epoch are discarded during
    /// metrics resets.
    pub(crate) epoch: u64,
    /// Pre-aggregated request timing and status-code data for successful,
    /// non-CO-mitigated requests, keyed by `"METHOD path"`.
    pub(crate) requests: HashMap<String, RequestBatchEntry>,
    /// Individual request metrics that need per-request processing:
    /// failed requests (for error logging) and CO-affected requests (for backfill).
    pub(crate) individual_requests: Vec<GooseRequestMetric>,
    /// Pre-aggregated transaction timing data, keyed by `(scenario_index, transaction_index)`.
    pub(crate) transactions: HashMap<(usize, usize), TransactionBatchEntry>,
    /// Pre-aggregated scenario timing data, keyed by `scenario_index`.
    pub(crate) scenarios: HashMap<usize, ScenarioBatchEntry>,
    /// Per-second request graph data (RPS count and average response time),
    /// keyed by `(request_key, second)`. Populated when per second graphs are
    /// kept and the heatmap is off; with the heatmap on, the batch's
    /// [`Self::second`] and [`Self::next_second`] stand for it.
    pub(crate) graph_request_data: HashMap<(String, usize), RequestGraphBatchData>,
    /// When the dashboard's heatmap keeps response times by second:
    /// timing data of the requests that started in the second after
    /// [`Self::second`], keyed like `requests`, which then hold only those
    /// that started in that second. Empty otherwise, so a batch without
    /// the heatmap is laid out and built as it always was.
    pub(crate) next_second: HashMap<String, GooseRequestMetricTimingData>,
    /// The second, from the start of the test, that the first request in
    /// `requests` started in (`None` before it), when the heatmap is on. A request from a second other than this one or the next goes
    /// in a new batch.
    pub(crate) second: Option<usize>,
    /// Per-second transaction counters for graph data.
    /// Only populated when `--report-file` is configured.
    pub(crate) graph_tps: HashMap<usize, u32>,
    /// Per-second scenario counters for graph data.
    /// Only populated when `--report-file` is configured.
    pub(crate) graph_sps: HashMap<usize, u32>,
}

impl GooseMetricBatch {
    /// Create a new empty batch stamped with the given epoch.
    pub(crate) fn new(epoch: u64) -> Self {
        GooseMetricBatch {
            epoch,
            requests: HashMap::new(),
            individual_requests: Vec::new(),
            transactions: HashMap::new(),
            scenarios: HashMap::new(),
            graph_request_data: HashMap::new(),
            next_second: HashMap::new(),
            second: None,
            graph_tps: HashMap::new(),
            graph_sps: HashMap::new(),
        }
    }
}

/// Per-second graph data for a single request endpoint within a batch,
/// combining RPS count and average response time accumulator.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RequestGraphBatchData {
    /// Number of requests in this second.
    pub(crate) count: u32,
    /// Sum of response times in this second (for computing weighted average).
    pub(crate) total_time: f64,
}

/// Pre-aggregated request timing and status-code data for a single
/// `"METHOD path"` key within a [`GooseMetricBatch`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RequestBatchEntry {
    /// The request path (e.g., `"/api/users"`).
    pub(crate) path: String,
    /// The HTTP method.
    pub(crate) method: GooseMethod,
    /// Custom method label for non-HTTP protocols (empty for standard HTTP).
    pub(crate) custom_method: String,
    /// Pre-aggregated timing data (bucketed response times, min, max, total,
    /// count). When per second graphs are kept, only of requests that started
    /// in the batch's second ([`GooseMetricBatch::second`]).
    pub(crate) raw_data: GooseRequestMetricTimingData,
    /// Per-status-code occurrence counts.
    pub(crate) status_code_counts: HashMap<u16, usize>,
    /// Per-status-code timing summaries (count, total, min, max).
    pub(crate) status_code_timings: HashMap<u16, StatusCodeTimingSummary>,
}

/// Pre-aggregated transaction timing data for a single
/// `(scenario_index, transaction_index)` pair within a batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TransactionBatchEntry {
    /// Run times in this batch, in microseconds, and how many.
    pub(crate) run_times: GooseRequestMetricTimingData,
    /// Successful transaction count.
    pub(crate) success_count: usize,
    /// Failed transaction count.
    pub(crate) fail_count: usize,
}

/// Pre-aggregated scenario timing data for a single scenario index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ScenarioBatchEntry {
    /// Run times in this batch, in microseconds, and how many.
    pub(crate) run_times: GooseRequestMetricTimingData,
    /// The user index (single user per batch, since each GooseUser has its own batch).
    pub(crate) user: usize,
}

/// Lock-free shared counters for request success/failure counts.
///
/// Incremented directly by [`GooseUser`](../goose/struct.GooseUser.html) threads using
/// atomic operations, and read by the parent process when computing running metrics or
/// building the final report. This moves counter bookkeeping off the parent's
/// per-message processing path for the two most frequently updated fields
/// (`success_count` and `fail_count`). The full `GooseRequestMetric` is still sent
/// via channel for timing, status-code, and error aggregation.
///
/// The primary benefit is **timeliness**: counters are visible to the parent immediately
/// (not delayed by channel buffering under heavy load), which improves running-metrics
/// accuracy. It also establishes the atomic-sharing pattern for potential future
/// optimizations that move additional data off the channel.
///
/// `Ordering::Relaxed` is sufficient because these counters are only read at display/report
/// time, not used for synchronization between threads.
#[derive(Debug)]
pub struct GooseRequestCounters {
    /// Number of successful requests.
    pub success: AtomicU64,
    /// Number of failed requests.
    pub failure: AtomicU64,
}

impl GooseRequestCounters {
    /// Create a new zeroed counter pair.
    pub fn new() -> Self {
        GooseRequestCounters {
            success: AtomicU64::new(0),
            failure: AtomicU64::new(0),
        }
    }

    /// Reset both counters to zero.
    pub fn reset(&self) {
        self.success.store(0, AtomicOrdering::Relaxed);
        self.failure.store(0, AtomicOrdering::Relaxed);
    }
}

impl Default for GooseRequestCounters {
    fn default() -> Self {
        Self::new()
    }
}

/// Type alias for the shared registry of atomic request counters.
///
/// Keyed the same way as [`GooseMetrics::requests`]: `"METHOD path"` (e.g., `"GET /api/users"`).
/// User threads lazily insert entries on first use of each request key, then cache the `Arc`
/// locally to avoid repeated mutex lookups.
pub type GooseRequestCounterRegistry = Arc<Mutex<HashMap<String, Arc<GooseRequestCounters>>>>;

/// Commands sent from the main control loop to the dedicated metrics processor task.
pub(crate) enum MetricsCommand {
    /// Display running metrics (fire-and-forget).
    ///
    /// The processor drains pending metrics, syncs atomic counters, and prints.
    DisplayRunning {
        duration: usize,
        total_users: usize,
        maximum_users: usize,
    },
    /// Check the `--threshold` rules against the metrics so far (fire-and-forget),
    /// triggering the killswitch on the first that fails.
    ///
    /// Like the dashboard commands, the processor drains at most
    /// [`METRICS_DRAIN_LIMIT`] pending metrics first, so a busy run can not hold it.
    CheckThresholds {
        /// The run's duration in whole seconds.
        duration: usize,
        /// The run time in seconds, to the microsecond, for `rps` rules.
        seconds: f64,
    },
    /// The run enters Maintain, holding its users steady, `seconds` into the run
    /// (fire-and-forget): the hold `rps` rules count starts here. It drains at most
    /// [`METRICS_DRAIN_LIMIT`] pending metrics first, as
    /// [`MetricsCommand::CheckThresholds`] does.
    EnterMaintain { seconds: f64 },
    /// The run leaves Maintain `seconds` into the run (fire-and-forget): the hold
    /// `rps` rules count ends here. It drains as [`MetricsCommand::EnterMaintain`]
    /// does.
    LeaveMaintain { seconds: f64 },
    /// Record the current user count for the users-per-second graph.
    RecordUsers {
        active_users: usize,
        elapsed_secs: usize,
    },
    /// Flush all pending metrics, print running metrics, then reset rate metrics.
    ///
    /// Active-users graph history is **not** cleared (see
    /// [`GraphData::reset_rate_series`]). Sends an ack when complete so the main
    /// loop can safely continue.
    Reset {
        duration: usize,
        total_users: usize,
        maximum_users: usize,
        /// Live user count at reset (for logging continuity; users series is kept).
        active_users: usize,
        /// Seconds on the continuous series clock at the reset, where the
        /// report heatmap's columns start again.
        series_elapsed_secs: usize,
        ack: tokio::sync::oneshot::Sender<()>,
    },
    /// Return a snapshot of the current metrics for the controller.
    GetMetrics {
        duration: usize,
        total_users: usize,
        maximum_users: usize,
        history: Vec<TestPlanHistory>,
        respond: tokio::sync::oneshot::Sender<GooseMetrics>,
    },
    /// Keep the graph data for the whole run, or only the dashboard's window,
    /// as Save report is turned on or off. `now` is the current second on
    /// the series clock.
    #[cfg_attr(not(feature = "dashboard"), allow(dead_code))]
    KeepGraph { keep: bool, now: usize },
    /// Copies of the metrics and graph data so far, for Save report now.
    #[cfg_attr(not(feature = "dashboard"), allow(dead_code))]
    GetReportData {
        duration: usize,
        total_users: usize,
        maximum_users: usize,
        history: Vec<TestPlanHistory>,
        respond: tokio::sync::oneshot::Sender<(GooseMetrics, GraphData)>,
    },
    /// Flush all pending metrics and return the final state for reporting.
    Shutdown {
        duration: usize,
        total_users: usize,
        maximum_users: usize,
        respond: tokio::sync::oneshot::Sender<(GooseMetrics, GraphData)>,
    },
    /// Build a compact dashboard snapshot (always compiled; used by the HTTP dashboard).
    ///
    /// Same drain + atomic sync path as [`MetricsCommand::GetMetrics`]. Phase and
    /// `active_users` are supplied by the main loop (processor does not own them).
    ///
    /// Constructed by the dashboard HTTP server; unit-tested here.
    #[cfg_attr(not(feature = "dashboard"), allow(dead_code))]
    GetDashboardSnapshot {
        /// Same path Controllers use: main loop calls `update_duration()` first,
        /// then passes `self.metrics.duration` (processor overwrites snapshot field).
        duration: usize,
        total_users: usize,
        maximum_users: usize,
        /// Live count from GooseAttackRunState.active_users — NOT on GooseMetrics.
        active_users: usize,
        /// Current plan-step / configured target (users ramping toward).
        target_users: usize,
        /// Seconds since the continuous series clock started (not restarted on
        /// metrics reset). Used to pin the active-users series at "now".
        series_elapsed_secs: usize,
        /// String form of AttackPhase: "idle"|"increase"|"maintain"|"decrease"|"shutdown".
        phase: String,
        /// `GooseAttackRunState.stopping`: a cancel is ramping the run down.
        stopping: bool,
        series_window_secs: u32,
        /// The report the run writes, which only the main loop knows.
        report: dashboard_snapshot::SnapshotReport,
        /// The run settings, which only the main loop knows.
        settings: Box<dashboard_snapshot::SnapshotSettings>,
        /// What the dashboard holds of the heatmap: only columns changed
        /// since are exported (every column when `None`).
        heatmap_since: Option<dashboard_snapshot::HeatmapCursor>,
        respond: tokio::sync::oneshot::Sender<DashboardSnapshot>,
    },
    /// One request's heatmap for the dashboard (`GET /api/v1/series`), or
    /// `None` when the request has no histograms of its own.
    #[cfg_attr(not(feature = "dashboard"), allow(dead_code))]
    GetRequestSeries {
        /// The request's method label, as in the snapshot's request rows.
        method: String,
        /// The request's name.
        name: String,
        /// Seconds since the continuous series clock started, as for
        /// [`MetricsCommand::GetDashboardSnapshot`].
        series_elapsed_secs: usize,
        /// What the client holds of this heatmap, as for
        /// [`MetricsCommand::GetDashboardSnapshot`].
        since: Option<dashboard_snapshot::HeatmapCursor>,
        respond: tokio::sync::oneshot::Sender<Option<dashboard_snapshot::RequestSeries>>,
    },
}

/// A dedicated task that processes metrics from user threads independently of
/// the main control loop.
///
/// This decouples metric volume from control-loop responsiveness: the main loop
/// stays fast regardless of RPS, while the processor gets a full Tokio worker
/// thread for metric aggregation.
pub(crate) struct MetricsProcessor {
    /// Channel receiver for raw metrics from user threads.
    metrics_rx: flume::Receiver<GooseMetric>,
    /// Channel receiver for commands from the main loop.
    cmd_rx: flume::Receiver<MetricsCommand>,
    /// Aggregated metrics state.
    metrics: GooseMetrics,
    /// Graph data for HTML reports.
    graph_data: GraphData,
    /// Read-only configuration.
    configuration: GooseConfiguration,
    /// Scenario definitions, needed for re-initialization on reset.
    scenarios: Vec<Scenario>,
    /// Defaults, needed for re-initialization on reset.
    defaults: GooseDefaults,
    /// Shared atomic request counter registry.
    request_counter_registry: GooseRequestCounterRegistry,
    /// Logger channel for error logging.
    logger_tx: GooseLoggerTx,
    /// Shared epoch counter for batch validity across metrics resets.
    metrics_epoch: MetricsEpoch,
    /// Keep the graph data for the whole run, as a report at the end of the
    /// run needs; otherwise only the dashboard's window. Changes with Save
    /// report.
    keep_full_graph: bool,
}

/// THIS IS AN EXPERIMENTAL FEATURE, DISABLED BY DEFAULT. Optionally mitigate the loss of data
/// (coordinated omission) due to stalls on the upstream server.
///
/// Stalling can happen for many reasons, for example: garbage collection, a cache stampede,
/// even unrelated load on the same server. Without any mitigation, Goose loses
/// statistically relevant information as [`GooseUser`](../goose/struct.GooseUser.html)
/// threads are unable to make additional requests while they are blocked by an upstream stall.
/// Goose mitigates this by backfilling the requests that would have been made during that time.
/// Backfilled requests show up in the `--request-file` if enabled, though they were not actually
/// sent to the server.
///
/// Goose can be configured to backfill requests based on the expected
/// [`user_cadence`](struct.GooseRequestMetric.html#structfield.user_cadence). The expected
/// cadence can be automatically calculated with any of the following configuration options.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum GooseCoordinatedOmissionMitigation {
    /// Backfill based on the average
    /// [`user_cadence`](struct.GooseRequestMetric.html#structfield.user_cadence) for this
    /// [`GooseUser`](../goose/struct.GooseUser.html).
    Average,
    /// Backfill based on the maximum
    /// [`user_cadence`](struct.GooseRequestMetric.html#structfield.user_cadence) for this
    /// [`GooseUser`](../goose/struct.GooseUser.html).
    Maximum,
    /// Backfill based on the minimum
    /// [`user_cadence`](struct.GooseRequestMetric.html#structfield.user_cadence) for this
    /// [`GooseUser`](../goose/struct.GooseUser.html).
    Minimum,
    /// Completely disable coordinated omission mitigation (default).
    Disabled,
}
/// Allow `--co-mitigation` from the command line using text variations on supported
/// `GooseCoordinatedOmissionMitigation`s by implementing [`FromStr`].
impl FromStr for GooseCoordinatedOmissionMitigation {
    type Err = GooseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Use a [`RegexSet`] to match string representations of `GooseCoordinatedOmissionMitigation`,
        // returning the appropriate enum value. Also match a wide range of abbreviations and synonyms.
        let co_mitigation = RegexSet::new([
            r"(?i)^(average|ave|aver|avg|mean)$",
            r"(?i)^(maximum|ma|max|maxi)$",
            r"(?i)^(minimum|mi|min|mini)$",
            r"(?i)^(disabled|di|dis|disable|none|no)$",
        ])
        .expect("failed to compile co_mitigation RegexSet");
        let matches = co_mitigation.matches(s);
        if matches.matched(0) {
            Ok(GooseCoordinatedOmissionMitigation::Average)
        } else if matches.matched(1) {
            Ok(GooseCoordinatedOmissionMitigation::Maximum)
        } else if matches.matched(2) {
            Ok(GooseCoordinatedOmissionMitigation::Minimum)
        } else if matches.matched(3) {
            Ok(GooseCoordinatedOmissionMitigation::Disabled)
        } else {
            Err(GooseError::InvalidOption {
                option: format!("GooseCoordinatedOmissionMitigation::{s:?}"),
                value: s.to_string(),
                detail:
                    "Invalid co_mitigation, expected: average, disabled, maximum, median, or minimum"
                        .to_string(),
            })
        }
    }
}

/// All requests made during a load test.
///
/// Goose optionally tracks metrics about requests made during a load test. The
/// metrics can be disabled with the `--no-metrics` run-time option, or with
/// [`GooseDefault::NoMetrics`](../config/enum.GooseDefault.html#variant.NoMetrics).
///
/// Aggregated requests ([`GooseRequestMetricAggregate`]) are stored in a HashMap
/// with they key `method request-name`, for example `GET /`.
///
/// # Example
/// When viewed with [`std::fmt::Display`], [`GooseRequestMetrics`] are displayed in
/// a table:
/// ```text
/// === PER REQUEST METRICS ===
/// ------------------------------------------------------------------------------
/// Name                     |        # reqs |        # fails |    req/s |  fail/s
/// ------------------------------------------------------------------------------
/// GET (Anon) front page    |           438 |         0 (0%) |    43.80 |    0.00
/// GET (Anon) node page     |           296 |         0 (0%) |    29.60 |    0.00
/// GET (Anon) user page     |            90 |         0 (0%) |     9.00 |    0.00
/// GET (Auth) comment form  |            19 |         0 (0%) |     1.90 |    0.00
/// GET (Auth) front page    |           108 |         0 (0%) |    10.80 |    0.00
/// GET (Auth) node page     |            74 |         0 (0%) |     7.40 |    0.00
/// GET (Auth) user page     |            19 |         0 (0%) |     1.90 |    0.00
/// GET static asset         |         3,288 |         0 (0%) |   328.80 |    0.00
/// POST (Auth) comment form |            20 |         0 (0%) |     2.00 |    0.00
/// -------------------------+---------------+----------------+----------+--------
/// Aggregated               |         4,352 |         0 (0%) |   435.20 |    0.00
/// ------------------------------------------------------------------------------
/// Name                     |    Avg (ms) |        Min |        Max |      Median
/// ------------------------------------------------------------------------------
/// GET (Anon) front page    |       14.22 |          2 |         211 |         14
/// GET (Anon) node page     |       53.26 |          3 |          96 |         53
/// GET (Anon) user page     |       32.97 |         17 |         221 |         30
/// GET (Auth) comment form  |       54.32 |         36 |          88 |         50
/// GET (Auth) front page    |       39.02 |         25 |         232 |         38
/// GET (Auth) node page     |       52.08 |         36 |          81 |         51
/// GET (Auth) user page     |       31.21 |         25 |          40 |         31
/// GET static asset         |       11.55 |          3 |         217 |          8
/// POST (Auth) comment form |       54.30 |         41 |          73 |         52
/// -------------------------+-------------+------------+-------------+-----------
/// Aggregated               |       16.94 |          2 |         232 |         10
/// ------------------------------------------------------------------------------
/// Slowest page load within specified percentile of requests (in ms):
/// ------------------------------------------------------------------------------
/// Name                     |    50% |    75% |    98% |    99% |  99.9% | 99.99%
/// ------------------------------------------------------------------------------
/// GET (Anon) front page    |     14 |     18 |     30 |     43 |    210 |    210
/// GET (Anon) node page     |     53 |     62 |     78 |     86 |     96 |     96
/// GET (Anon) user page     |     30 |     33 |     43 |     53 |    220 |    220
/// GET (Auth) comment form  |     50 |     65 |     88 |     88 |     88 |     88
/// GET (Auth) front page    |     38 |     43 |     58 |     59 |    230 |    230
/// GET (Auth) node page     |     51 |     58 |     72 |     72 |     81 |     81
/// GET (Auth) user page     |     31 |     33 |     40 |     40 |     40 |     40
/// GET static asset         |      8 |     16 |     30 |     36 |    210 |    210
/// POST (Auth) comment form |     52 |     59 |     73 |     73 |     73 |     73
/// -------------------------+--------+--------+--------+--------+--------+-------
/// Aggregated               |     10 |     20 |     64 |     71 |    210 |    230
/// ```
pub type GooseRequestMetrics = HashMap<String, GooseRequestMetricAggregate>;

/// All transactions executed during a load test.
///
/// Goose optionally tracks metrics about transactions executed during a load test. The
/// metrics can be disabled with either the `--no-transaction-metrics` or the `--no-metrics`
/// run-time option, or with either
/// [`GooseDefault::NoTransactionMetrics`](../config/enum.GooseDefault.html#variant.NoTransactionMetrics) or
/// [`GooseDefault::NoMetrics`](../config/enum.GooseDefault.html#variant.NoMetrics).
///
/// Aggregated transactions ([`TransactionMetricAggregate`]) are stored in a Vector of Vectors
/// keyed to the order the transaction is created in the load test.
///
/// # Example
/// When viewed with [`std::fmt::Display`], [`TransactionMetrics`] are displayed in
/// a table:
/// ```text
///  === PER TRANSACTION METRICS ===
/// ------------------------------------------------------------------------------
/// Name                     |   # times run |        # fails |  trans/s |  fail/s
/// ------------------------------------------------------------------------------
/// 1: AnonBrowsingUser      |
///   1: (Anon) front page   |           440 |         0 (0%) |    44.00 |    0.00
///   2: (Anon) node page    |           296 |         0 (0%) |    29.60 |    0.00
///   3: (Anon) user page    |            90 |         0 (0%) |     9.00 |    0.00
/// 2: AuthBrowsingUser      |
///   1: (Auth) login        |             0 |         0 (0%) |     0.00 |    0.00
///   2: (Auth) front page   |           109 |         0 (0%) |    10.90 |    0.00
///   3: (Auth) node page    |            74 |         0 (0%) |     7.40 |    0.00
///   4: (Auth) user page    |            19 |         0 (0%) |     1.90 |    0.00
///   5: (Auth) comment form |            20 |         0 (0%) |     2.00 |    0.00
/// -------------------------+---------------+----------------+----------+--------
/// Aggregated               |         1,048 |         0 (0%) |   104.80 |    0.00
/// ------------------------------------------------------------------------------
/// Name                     |    Avg (ms) |        Min |         Max |     Median
/// ------------------------------------------------------------------------------
/// 1: AnonBrowsingUser      |
///   1: (Anon) front page   |       94.41 |         59 |         294 |         88
///   2: (Anon) node page    |       53.29 |          3 |          96 |         53
///   3: (Anon) user page    |       33.02 |         17 |         221 |         30
/// 2: AuthBrowsingUser      |
///   1: (Auth) login        |        0.00 |          0 |           0 |          0
///   2: (Auth) front page   |      119.45 |         84 |         307 |        110
///   3: (Auth) node page    |       52.16 |         37 |          81 |         51
///   4: (Auth) user page    |       31.21 |         25 |          40 |         31
///   5: (Auth) comment form |      135.10 |        107 |         175 |        130
/// -------------------------+-------------+------------+-------------+-----------
/// Aggregated               |       76.78 |          3 |         307 |         74
/// ```
pub type TransactionMetrics = Vec<Vec<TransactionMetricAggregate>>;

/// All scenarios executed during a load test.
///
/// Goose optionally tracks metrics about scenarios executed during a load test. The
/// metrics can be disabled with either the `--no-scenario-metrics` or the `--no-metrics`
/// run-time option, or with either
/// [`GooseDefault::NoScenarioMetrics`](../config/enum.GooseDefault.html#variant.NoScenarioMetrics) or
/// [`GooseDefault::NoMetrics`](../config/enum.GooseDefault.html#variant.NoMetrics).
///
/// Aggregated scenarios ([`ScenarioMetricAggregate`]) are stored in a Vector keyed to the order the
/// scenario is created in the load test.
///
/// # Example
/// When viewed with [`std::fmt::Display`], [`TransactionMetrics`] are displayed in
/// a table:
/// ```text
///  === PER SCENARIO METRICS ===
/// ------------------------------------------------------------------------------
/// Name                     |  # users |  # times run | scenarios/s | iterations
/// ------------------------------------------------------------------------------
/// 1: AnonBrowsingUser      |        9 |          224 |        6.40 |      24.89
/// 2: AuthBrowsingUser      |        3 |           51 |        1.46 |      17.00
/// -------------------------+----------+--------------+-------------+------------
/// Aggregated               |       12 |          275 |        7.86 |      22.92
/// ------------------------------------------------------------------------------
/// Name                     |    Avg (ms) |        Min |         Max |     Median
/// ------------------------------------------------------------------------------
/// 1: AnonBrowsingUser    |        1293 |      1,067 |       1,672 |      1,067
/// 2: AuthBrowsingUser    |        1895 |      1,505 |       2,235 |      1,505
/// -------------------------+-------------+------------+-------------+-----------
/// Aggregated               |        1405 |      1,067 |       2,235 |      1,067
/// ```
pub type ScenarioMetrics = Vec<ScenarioMetricAggregate>;

/// All errors detected during a load test.
///
/// By default Goose tracks all errors detected during the load test. Each error is stored
/// as a [`GooseErrorMetricAggregate`](./struct.GooseErrorMetricAggregate.html), and they
/// are all stored together within a `BTreeMap` which is returned by
/// [`GooseAttack::execute()`](../struct.GooseAttack.html#method.execute) when a load test
/// completes.
///
/// `GooseErrorMetrics` can be disabled with the `--no-error-summary` run-time option, or with
/// [GooseDefault::NoErrorSummary](../config/enum.GooseDefault.html#variant.NoErrorSummary).
///
/// # Example
/// When viewed with [`std::fmt::Display`], [`GooseErrorMetrics`] are displayed in
/// a table:
/// ```text
///  === ERRORS ===
/// ------------------------------------------------------------------------------
/// Count       | Error
/// ------------------------------------------------------------------------------
/// 924           GET (Auth) front page: 503 Service Unavailable: /
/// 715           POST (Auth) front page: 503 Service Unavailable: /user
/// 36            GET (Anon) front page: error sending request for url (http://example.com/): connection closed before message completed
/// ```
pub type GooseErrorMetrics = BTreeMap<String, GooseErrorMetricAggregate>;

/// For tracking and logging requests made during a load test.
///
/// The raw request that the GooseClient is making. Is included in the [`GooseRequestMetric`]
/// when metrics are enabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GooseRawRequest {
    /// The method being used (ie, Get, Post, etc).
    pub method: GooseMethod,
    /// The full URL that was requested.
    pub url: String,
    /// Headers set on this individual request, for example by `.header()`, `.bearer_auth()`,
    /// `.json()` or `.form()`.
    ///
    /// Headers that Reqwest adds when it sends the request are not included: the client's
    /// default headers (those set with `.default_headers()`, the `User-Agent` and
    /// `Accept: */*`), the `Cookie` header from the cookie store, `Accept-Encoding` when
    /// compression is enabled, and the `Host` and `Content-Length` headers. Reqwest offers
    /// no way to read them, see
    /// [issue #336](https://github.com/tag1consulting/goose/issues/336).
    ///
    /// Only captured when the request log, the debug log or the error log is enabled,
    /// as these are the only consumers. Empty otherwise.
    pub headers: Vec<String>,
    /// The body of the request made, if `--request-body` is enabled.
    pub body: String,
    /// User-defined method label for non-HTTP protocols (e.g. "TCP", "GRPC", "WS").
    ///
    /// When non-empty this label is used in metrics output instead of the
    /// [`GooseMethod`] display. Empty for standard HTTP requests.
    #[serde(default)]
    pub custom_method: String,
}
/// Maps a standard HTTP [`GooseMethod`] variant to its canonical uppercase string.
///
/// Centralises the variant-to-string mapping that [`GooseRawRequest::method_label`],
/// [`GooseRequestMetricAggregate::method_label`], and
/// [`GooseErrorMetricAggregate::method_label`] all need. Adding a new `GooseMethod`
/// variant only requires updating this one function.
fn http_method_str(method: GooseMethod) -> &'static str {
    match method {
        GooseMethod::Delete => "DELETE",
        GooseMethod::Get => "GET",
        GooseMethod::Head => "HEAD",
        GooseMethod::Patch => "PATCH",
        GooseMethod::Post => "POST",
        GooseMethod::Put => "PUT",
        GooseMethod::Custom => "CUSTOM",
    }
}

impl GooseRawRequest {
    pub(crate) fn new(
        method: GooseMethod,
        url: &str,
        headers: Vec<String>,
        body: &str,
    ) -> GooseRawRequest {
        GooseRawRequest {
            method,
            url: url.to_string(),
            headers,
            body: body.to_string(),
            custom_method: String::new(),
        }
    }

    /// Create a `GooseRawRequest` for a non-HTTP custom-protocol operation.
    ///
    /// Unlike [`GooseRawRequest::new`], this constructor does not require headers or a
    /// body — neither concept applies to custom protocols.
    pub(crate) fn new_custom(name: &str, custom_method: &str) -> GooseRawRequest {
        GooseRawRequest {
            method: GooseMethod::Custom,
            url: name.to_string(),
            headers: vec![],
            body: String::new(),
            custom_method: custom_method.to_string(),
        }
    }

    /// Returns the display label for this request's method.
    ///
    /// For standard HTTP requests this returns the uppercase HTTP method name
    /// (e.g. "GET", "POST"). For custom protocol requests with a non-empty
    /// `custom_method`, returns that label instead.
    pub fn method_label(&self) -> &str {
        if !self.custom_method.is_empty() {
            &self.custom_method
        } else {
            http_method_str(self.method)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionDetail {
    /// An index into [`GooseAttack`]`.scenarios`, indicating which scenario this is.
    pub scenario_index: usize,
    /// The scenario name.
    #[serde(with = "arc_str_serde")]
    pub scenario_name: Arc<str>,
    /// An optional index into [`Scenario`]`.transaction`, indicating which transaction this is.
    /// `None` indicates no transaction, `Some(0)` is the first transaction.
    pub transaction_index: Option<usize>,
    /// An optional name for the transaction.
    pub transaction_name: TransactionName,
}

/// How many milliseconds the load test has been running.
/// For tracking and counting requests made during a load test.
///
/// The request that Goose is making. User threads send this data to the parent thread
/// when metrics are enabled. This request object must be provided to calls to
/// [`set_success`](../goose/struct.GooseUser.html#method.set_success) or
/// [`set_failure`](../goose/struct.GooseUser.html#method.set_failure) so Goose
/// knows which request is being updated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GooseRequestMetric {
    /// How many milliseconds the load test has been running.
    pub elapsed: u64,
    /// An index into [`GooseAttack`]`.scenarios`, indicating which scenario this is.
    pub scenario_index: usize,
    /// The scenario name.
    #[serde(with = "arc_str_serde")]
    pub scenario_name: Arc<str>,
    /// An optional index into [`Scenario`]`.transaction`, indicating which transaction this is.
    /// Stored as usize, `usize::MAX` indicates no transaction, while `0` is the first `Scenario.transaction`.
    pub transaction_index: usize,
    /// The optional transaction name.
    pub transaction_name: TransactionName,
    /// The raw request that the GooseClient made.
    pub raw: GooseRawRequest,
    /// The optional name of the request.
    #[serde(with = "arc_str_serde")]
    pub name: Arc<str>,
    /// The final full URL that was requested, after redirects.
    pub final_url: String,
    /// Whether or not the request was redirected.
    pub redirected: bool,
    /// How many milliseconds the request took (Time to First Byte),
    /// truncated. [`response_time_fraction_us`](Self::response_time_fraction_us)
    /// holds the microseconds past them.
    pub response_time: u64,
    /// The HTTP response code (optional).
    pub status_code: u16,
    /// Whether or not the request was successful.
    pub success: bool,
    /// Whether or not we're updating a previous request, modifies how the parent thread records it.
    pub update: bool,
    /// Which [`GooseUser`](../goose/struct.GooseUser.html) thread processed the request.
    pub user: usize,
    /// The optional error caused by this request.
    pub error: String,
    /// If non-zero, Coordinated Omission Mitigation detected an abnormally long response time on
    /// the upstream server, blocking requests from being made.
    pub coordinated_omission_elapsed: u64,
    /// If non-zero, the calculated cadence of looping through all
    /// [`Transaction`](../goose/struct.Transaction.html)s by this
    /// [`GooseUser`](../goose/struct.GooseUser.html) thread.
    pub user_cadence: u64,
    /// The microseconds past [`response_time`](Self::response_time)'s whole
    /// milliseconds, from 0 to 999, so a request under a millisecond keeps
    /// its time. [`response_time_micros`](Self::response_time_micros) gives
    /// both together. 0 when read from a log written before Goose kept them.
    #[serde(default)]
    pub response_time_fraction_us: u16,
}

impl GooseRequestMetric {
    pub(crate) fn new(
        raw: GooseRawRequest,
        transaction_detail: TransactionDetail,
        name: impl Into<Arc<str>>,
        elapsed: u128,
        user: usize,
    ) -> Self {
        GooseRequestMetric {
            elapsed: elapsed as u64,
            scenario_index: transaction_detail.scenario_index,
            scenario_name: transaction_detail.scenario_name,
            transaction_index: transaction_detail.transaction_index.unwrap_or(usize::MAX),
            transaction_name: transaction_detail.transaction_name,
            raw,
            name: name.into(),
            final_url: "".to_string(),
            redirected: false,
            response_time: 0,
            status_code: 0,
            success: true,
            update: false,
            user,
            error: "".to_string(),
            coordinated_omission_elapsed: 0,
            user_cadence: 0,
            response_time_fraction_us: 0,
        }
    }

    /// How long the request took, in microseconds.
    pub fn response_time_micros(&self) -> u64 {
        micros(self.response_time, self.response_time_fraction_us)
    }

    /// Record how long the request took: whole milliseconds in
    /// `response_time`, the microseconds past them in
    /// `response_time_fraction_us`.
    pub(crate) fn set_response_duration(&mut self, response_time: Duration) {
        self.set_response_micros(duration_micros(response_time));
    }

    /// [`Self::set_response_duration`] from microseconds.
    pub(crate) fn set_response_micros(&mut self, us: u64) {
        self.response_time = us / 1000;
        self.response_time_fraction_us = (us % 1000) as u16;
    }

    /// Record how long the request took in whole milliseconds, with no
    /// microseconds past them.
    #[cfg(test)]
    pub(crate) fn set_response_time(&mut self, response_time: u128) {
        self.set_response_micros((response_time as u64).saturating_mul(1000));
    }

    // Record the final URL returned.
    pub(crate) fn set_final_url(&mut self, final_url: &str) {
        self.final_url = final_url.to_string();
        if self.final_url != self.raw.url {
            self.redirected = true;
        }
    }

    // Record the returned `status_code`.
    pub(crate) fn set_status_code(&mut self, status_code: Option<StatusCode>) {
        self.status_code = match status_code {
            Some(status_code) => status_code.as_u16(),
            None => 0,
        };
    }
}

/// Metrics collected about a method-path pair, (for example `GET /index`).
///
/// [`GooseRequestMetric`]s are sent by [`GooseUser`](../goose/struct.GooseUser.html)
/// threads to the Goose parent process where they are aggregated together into this
/// structure, and stored in [`GooseMetrics::requests`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GooseRequestMetricAggregate {
    /// The request path for which metrics are being collected.
    ///
    /// For example: "/".
    pub path: String,
    /// The method for which metrics are being collected.
    ///
    /// For example: [`GooseMethod::Get`].
    pub method: GooseMethod,
    /// The raw data seen from actual requests.
    pub raw_data: GooseRequestMetricTimingData,
    /// Combines the raw data with statistically generated Coordinated Omission Metrics.
    pub coordinated_omission_data: Option<GooseRequestMetricTimingData>,
    /// Per-status-code counters, tracking how often each response code was returned for this request.
    pub status_code_counts: HashMap<u16, usize>,
    /// Per-status-code timing summaries for response time breakdowns.
    ///
    /// Only tracks aggregate statistics (count, total, min, max) rather than full
    /// distributions, keeping memory overhead minimal. Only populated for non-CO-mitigated
    /// requests when status code tracking is enabled.
    ///
    /// Written to the JSON report, so a report rendered from it with
    /// `--render-report` rebuilds the per status code breakdown rows.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub status_code_timings: HashMap<u16, StatusCodeTimingSummary>,
    /// Total number of times this path-method request resulted in a successful (2xx) status code.
    ///
    /// A count of how many requests resulted in a 2xx status code.
    pub success_count: usize,
    /// Total number of times this path-method request resulted in a non-successful (non-2xx) status code.
    ///
    /// A count of how many requests resulted in a non-2xx status code.
    pub fail_count: usize,
    /// Load test hash.
    ///
    /// The hash is primarily used when running a distributed Gaggle, allowing the Manager to confirm
    /// that all Workers are running the same load test plan.
    pub load_test_hash: u64,
    /// User-defined method label for non-HTTP protocols (e.g. "TCP", "GRPC").
    ///
    /// Empty for standard HTTP requests. When non-empty, used in metrics output
    /// instead of the [`GooseMethod`] display.
    #[serde(default)]
    pub custom_method: String,
}
impl GooseRequestMetricAggregate {
    /// Create a new GooseRequestMetricAggregate object.
    pub(crate) fn new(
        path: &str,
        method: GooseMethod,
        load_test_hash: u64,
        custom_method: &str,
    ) -> Self {
        trace!("[metrics]: new request");
        GooseRequestMetricAggregate {
            path: path.to_string(),
            method,
            raw_data: GooseRequestMetricTimingData::new(None),
            coordinated_omission_data: None,
            status_code_counts: HashMap::new(),
            status_code_timings: HashMap::new(),
            success_count: 0,
            fail_count: 0,
            load_test_hash,
            custom_method: custom_method.to_string(),
        }
    }

    /// Returns the display label for this aggregate's method.
    pub fn method_label(&self) -> &str {
        if !self.custom_method.is_empty() {
            &self.custom_method
        } else {
            http_method_str(self.method)
        }
    }

    /// Record a response time in whole milliseconds (unit tests).
    #[cfg(test)]
    pub(crate) fn record_time(&mut self, time_elapsed: u64, coordinated_omission_mitigation: bool) {
        self.record_time_us(
            time_elapsed.saturating_mul(1000),
            coordinated_omission_mitigation,
        );
    }

    /// Record a response time in microseconds.
    pub(crate) fn record_time_us(&mut self, us: u64, coordinated_omission_mitigation: bool) {
        // Only add the time to raw_data if it wasn't generated by Coordinated
        // Omission Mitigation.
        if !coordinated_omission_mitigation {
            self.raw_data.record_time_us(us);
        }

        // A Coordinated Omission data object already exists, add a new time into the data.
        if let Some(coordinated_omission_data) = self.coordinated_omission_data.as_mut() {
            coordinated_omission_data.record_time_us(us);
        }
        // Create a new Coordinated Omission data object by cloning the raw data.
        else if coordinated_omission_mitigation {
            // If this time was generated by Coordinated Omission Mitigation, it doesn't
            // exist in the raw_data, so add it.
            let mut coordinated_omission_data = self.raw_data.clone();
            coordinated_omission_data.record_time_us(us);
            self.coordinated_omission_data = Some(coordinated_omission_data);
        }
    }

    /// Increment counter for status code, creating new counter if first time seeing status code.
    pub(crate) fn set_status_code(&mut self, status_code: u16) {
        let counter = match self.status_code_counts.get(&status_code) {
            // We've seen this status code before, increment counter.
            Some(c) => {
                debug!("[metrics]: got {status_code:?} counter: {c}");
                *c + 1
            }
            // First time we've seen this status code, initialize counter.
            None => {
                debug!("[metrics]: no match for counter: {status_code}");
                1
            }
        };
        self.status_code_counts.insert(status_code, counter);
        debug!("[metrics]: incremented {status_code} counter: {counter}");
    }

    /// Record a response time in whole milliseconds for a specific status
    /// code.
    ///
    /// Only tracks aggregate statistics (count, total, min, max) to keep memory
    /// overhead minimal compared to the full [`GooseRequestMetricTimingData`].
    pub fn record_status_code_time(&mut self, status_code: u16, time_elapsed: u64) {
        self.record_status_code_time_us(status_code, time_elapsed.saturating_mul(1000));
    }

    /// [`Self::record_status_code_time`] with the time in microseconds.
    pub fn record_status_code_time_us(&mut self, status_code: u16, us: u64) {
        let summary = self.status_code_timings.entry(status_code).or_default();
        summary.record_us(us);
    }

    /// Returns per-status-code breakdown data if multiple status codes were recorded.
    ///
    /// Returns `None` if only one (or zero) status codes were seen, since a breakdown
    /// would not provide additional insight beyond the aggregate row.
    pub fn status_code_breakdowns(&self) -> Option<Vec<StatusCodeBreakdown>> {
        if self.status_code_timings.len() <= 1 {
            return None;
        }
        let total: usize = self.status_code_timings.values().map(|s| s.count).sum();
        let mut breakdowns: Vec<StatusCodeBreakdown> = self
            .status_code_timings
            .iter()
            .map(|(&code, summary)| StatusCodeBreakdown {
                status_code: code,
                percentage: if total > 0 {
                    summary.count as f32 / total as f32 * 100.0
                } else {
                    0.0
                },
                average: if summary.count > 0 {
                    (summary.total_time_us as f64 / summary.count as f64 / 1000.0) as f32
                } else {
                    0.0
                },
                min_time_us: summary.min_time_us,
                max_time_us: summary.max_time_us,
                count: summary.count,
            })
            .collect();
        breakdowns.sort_by_key(|b| b.status_code);
        Some(breakdowns)
    }
}

/// Lightweight per-status-code timing summary for response time breakdowns.
///
/// Unlike [`GooseRequestMetricTimingData`], which counts every time in buckets
/// for percentiles, this keeps only the count, total, fastest and slowest, to
/// keep memory low. Times are in microseconds.
#[derive(Debug, Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct StatusCodeTimingSummary {
    pub count: usize,
    /// Sum of the response times, in microseconds.
    pub total_time_us: u64,
    /// The fastest response in microseconds, 0 before any.
    pub min_time_us: u64,
    /// The slowest response in microseconds.
    pub max_time_us: u64,
}

impl StatusCodeTimingSummary {
    /// Count one response that took `us` microseconds. The count, not a
    /// minimum of 0, says whether there is a time yet: a response can take
    /// 0 µs.
    pub(crate) fn record_us(&mut self, us: u64) {
        if self.count == 0 || us < self.min_time_us {
            self.min_time_us = us;
        }
        if us > self.max_time_us {
            self.max_time_us = us;
        }
        self.count += 1;
        self.total_time_us += us;
    }

    /// Add another summary of the same status code's responses.
    pub(crate) fn merge(&mut self, other: &StatusCodeTimingSummary) {
        if other.count > 0 && (self.count == 0 || other.min_time_us < self.min_time_us) {
            self.min_time_us = other.min_time_us;
        }
        if other.max_time_us > self.max_time_us {
            self.max_time_us = other.max_time_us;
        }
        self.count += other.count;
        self.total_time_us += other.total_time_us;
    }
}

/// Computed breakdown data for a single status code, used for display.
pub struct StatusCodeBreakdown {
    pub status_code: u16,
    pub percentage: f32,
    /// The average response time, in milliseconds.
    pub average: f32,
    /// The fastest response, in microseconds.
    pub min_time_us: u64,
    /// The slowest response, in microseconds.
    pub max_time_us: u64,
    pub count: usize,
}

/// Implement equality for GooseRequestMetricAggregate. We can't simply derive since
/// we have floats in the struct.
///
/// `Eq` trait has no functions on it - it is there only to inform the compiler
/// that this is an equivalence rather than a partial equivalence.
///
/// See <https://doc.rust-lang.org/std/cmp/trait.Eq.html> for more information.
impl Eq for GooseRequestMetricAggregate {}

/// Implement ordering for GooseRequestMetricAggregate.
impl Ord for GooseRequestMetricAggregate {
    fn cmp(&self, other: &Self) -> Ordering {
        (&self.method, &self.custom_method, &self.path).cmp(&(
            &other.method,
            &other.custom_method,
            &other.path,
        ))
    }
}
/// Implement partial-ordering for GooseRequestMetricAggregate.
impl PartialOrd for GooseRequestMetricAggregate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Response times, in microseconds: how many fell in each bucket of a
/// [`TimeBuckets`], and the fastest, slowest and total. Only the buckets
/// that hold a time are kept.
///
/// It serializes, in a JSON report, with each bucket once, in
/// `times_us`, beside the fastest, slowest and total in microseconds and in
/// whole milliseconds; an earlier Goose, which reads whole millisecond
/// `times`, can not read it. A report from an earlier Goose, with only
/// whole millisecond `times`, reads as whole milliseconds.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct GooseRequestMetricTimingData {
    /// How many times fell in each bucket, keyed by the bucket's fastest
    /// time in microseconds.
    pub times_us: TimeBuckets,
    /// The fastest time, in microseconds, not rounded. 0 before any.
    pub minimum_time_us: u64,
    /// The slowest time, in microseconds, not rounded.
    pub maximum_time_us: u64,
    /// All times added together, in microseconds.
    pub total_time_us: u64,
    /// How many times are counted.
    pub counter: usize,
}
impl GooseRequestMetricTimingData {
    /// Create a new GooseRequestMetricAggregate object.
    pub(crate) fn new(metric_data: Option<&GooseRequestMetricTimingData>) -> Self {
        trace!("new GooseRequestMetricTimingData");
        metric_data.cloned().unwrap_or_default()
    }

    /// [`Self::merge`], also handing each of `other`'s buckets, as its
    /// fastest time in microseconds and its count, to `also`, so a caller
    /// keeping the same times elsewhere walks them once.
    pub(crate) fn merge_also(
        &mut self,
        other: &GooseRequestMetricTimingData,
        mut also: impl FnMut(u64, usize),
    ) {
        self.times_us.merge(&other.times_us, &mut also);
        self.merge_totals(other);
    }

    /// Merge another set of timing data into this one: combine the buckets and
    /// update the minimum, maximum, total and counter.
    pub(crate) fn merge(&mut self, other: &GooseRequestMetricTimingData) {
        self.times_us.merge(&other.times_us, |_, _| {});
        self.merge_totals(other);
    }

    /// The fastest, slowest, total and counter of [`Self::merge`].
    fn merge_totals(&mut self, other: &GooseRequestMetricTimingData) {
        // The counters, not a minimum of 0, say whether there are times yet:
        // a time can be 0 µs.
        if other.counter > 0 && (self.counter == 0 || other.minimum_time_us < self.minimum_time_us)
        {
            self.minimum_time_us = other.minimum_time_us;
        }
        self.counter += other.counter;
        self.total_time_us += other.total_time_us;
        if other.maximum_time_us > self.maximum_time_us {
            self.maximum_time_us = other.maximum_time_us;
        }
    }

    /// Record a new time, in whole milliseconds (unit tests).
    #[cfg(test)]
    pub(crate) fn record_time(&mut self, time_elapsed: u64) {
        self.record_time_us(time_elapsed.saturating_mul(1000));
    }

    /// Record a new time, in microseconds.
    pub(crate) fn record_time_us(&mut self, us: u64) {
        // The counter, not a minimum of 0, says whether there is a time yet.
        if self.counter == 0 || us < self.minimum_time_us {
            self.minimum_time_us = us;
        }
        if us > self.maximum_time_us {
            self.maximum_time_us = us;
        }
        self.total_time_us += us;
        self.counter += 1;
        self.times_us.add(buckets::bucket_floor(us), 1);
    }

    /// [`Self::record_time_us`] for a user's batch: the buckets stay in the
    /// order the times came, which a merge takes.
    pub(crate) fn record_time_us_unsorted(&mut self, us: u64) {
        if self.counter == 0 || us < self.minimum_time_us {
            self.minimum_time_us = us;
        }
        if us > self.maximum_time_us {
            self.maximum_time_us = us;
        }
        self.total_time_us += us;
        self.counter += 1;
        self.times_us.push(buckets::bucket_floor(us), 1);
    }

    /// The time `percent` (0.0 to 1.0) of these times finished within, in
    /// microseconds: the middle of its bucket, kept between the fastest and
    /// slowest. 0 without times.
    pub fn percentile_us(&self, percent: f32) -> u64 {
        buckets::percentile(
            self.times_us
                .iter()
                .map(|(&bucket, &count)| (buckets::bucket_value(bucket), count)),
            self.counter,
            self.minimum_time_us,
            self.maximum_time_us,
            percent,
        )
    }

    /// The fastest time, in whole milliseconds, truncated.
    pub fn minimum_time_ms(&self) -> usize {
        (self.minimum_time_us / 1000) as usize
    }

    /// The slowest time, in whole milliseconds, truncated.
    pub fn maximum_time_ms(&self) -> usize {
        (self.maximum_time_us / 1000) as usize
    }

    /// The mean time in milliseconds, to the microsecond; NaN without times.
    pub fn average_ms(&self) -> f64 {
        self.total_time_us as f64 / self.counter as f64 / 1000.0
    }

    /// [`Self::average_ms`], or 0 without times.
    pub(crate) fn average_ms_or_zero(&self) -> f64 {
        match self.counter {
            0 => 0.0,
            _ => self.average_ms(),
        }
    }
}

/// The buckets of whole millisecond `times`, as a file without microseconds
/// holds them: each in the bucket of its time.
fn microsecond_times(times: &BTreeMap<usize, usize>) -> TimeBuckets {
    times
        .iter()
        .map(|(&ms, &count)| {
            (
                buckets::bucket_floor((ms as u64).saturating_mul(1000)),
                count,
            )
        })
        .collect()
}

/// Whole milliseconds as microseconds.
fn ms_to_us(ms: usize) -> u64 {
    (ms as u64).saturating_mul(1000)
}

/// Microseconds as whole milliseconds, truncated.
fn us_to_ms(us: u64) -> usize {
    (us / 1000) as usize
}

/// [`GooseRequestMetricTimingData`] as it is written: the fastest, slowest
/// and total in whole milliseconds and the count, then the buckets and the
/// same times in microseconds.
#[derive(Serialize)]
struct TimingDataOut<'a> {
    minimum_time: usize,
    maximum_time: usize,
    total_time: usize,
    counter: usize,
    times_us: &'a TimeBuckets,
    minimum_time_us: u64,
    maximum_time_us: u64,
    total_time_us: u64,
}

/// [`GooseRequestMetricTimingData`] as it is read: with microseconds, or
/// only the whole milliseconds of a file written before them.
#[derive(Deserialize)]
struct TimingDataIn {
    #[serde(default)]
    times: BTreeMap<usize, usize>,
    #[serde(default)]
    minimum_time: usize,
    #[serde(default)]
    maximum_time: usize,
    #[serde(default)]
    total_time: usize,
    #[serde(default)]
    counter: usize,
    times_us: Option<TimeBuckets>,
    #[serde(default)]
    minimum_time_us: Option<u64>,
    #[serde(default)]
    maximum_time_us: Option<u64>,
    #[serde(default)]
    total_time_us: Option<u64>,
}

impl Serialize for GooseRequestMetricTimingData {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        TimingDataOut {
            minimum_time: self.minimum_time_ms(),
            maximum_time: self.maximum_time_ms(),
            total_time: us_to_ms(self.total_time_us),
            counter: self.counter,
            times_us: &self.times_us,
            minimum_time_us: self.minimum_time_us,
            maximum_time_us: self.maximum_time_us,
            total_time_us: self.total_time_us,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for GooseRequestMetricTimingData {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let data = TimingDataIn::deserialize(deserializer)?;
        Ok(run_times(
            data.times,
            [data.minimum_time, data.maximum_time, data.total_time],
            data.counter,
            data.times_us,
            [
                data.minimum_time_us,
                data.maximum_time_us,
                data.total_time_us,
            ],
        ))
    }
}
/// The per-scenario metrics collected each time a scenario is run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioMetric {
    /// How many milliseconds the load test has been running.
    pub elapsed: u64,
    /// The name of the scenario.
    #[serde(with = "arc_str_serde")]
    pub name: Arc<str>,
    /// An index into [`GooseAttack`]`.scenarios`, indicating which scenario this is.
    pub index: usize,
    /// How many milliseconds the scenario ran, truncated.
    pub run_time: u64,
    /// Which GooseUser thread processed the request.
    pub user: usize,
    /// The microseconds past [`run_time`](Self::run_time)'s whole
    /// milliseconds, from 0 to 999. [`run_time_micros`](Self::run_time_micros)
    /// gives both together. 0 when read from a log written before Goose kept
    /// them.
    #[serde(default)]
    pub run_time_fraction_us: u16,
}
impl ScenarioMetric {
    /// Create a new Scenario metric for a scenario that ran `run_time`.
    pub(crate) fn new(
        elapsed: u128,
        scenario_name: Arc<str>,
        index: usize,
        run_time: Duration,
        user: usize,
    ) -> Self {
        let us = duration_micros(run_time);
        ScenarioMetric {
            elapsed: elapsed as u64,
            name: scenario_name,
            index,
            run_time: us / 1000,
            user,
            run_time_fraction_us: (us % 1000) as u16,
        }
    }

    /// How long the scenario ran, in microseconds.
    pub fn run_time_micros(&self) -> u64 {
        micros(self.run_time, self.run_time_fraction_us)
    }
}

/// `duration` in whole microseconds, at most [`u64::MAX`].
pub(crate) fn duration_micros(duration: Duration) -> u64 {
    duration.as_micros().min(u64::MAX as u128) as u64
}

/// Whole milliseconds and the microseconds past them, as microseconds.
fn micros(ms: u64, fraction_us: u16) -> u64 {
    ms.saturating_mul(1000)
        .saturating_add(u64::from(fraction_us))
}

/// The per-transaction metrics collected each time a transaction is invoked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionMetric {
    /// How many milliseconds the load test has been running.
    pub elapsed: u64,
    /// An index into [`GooseAttack`]`.scenarios`, indicating which transaction set this is.
    pub scenario_index: usize,
    /// An index into [`Scenario`]`.transaction`, indicating which transaction this is.
    pub transaction_index: usize,
    /// The optional name of the transaction.
    #[serde(with = "arc_str_serde")]
    pub name: Arc<str>,
    /// How many milliseconds the transaction ran, truncated.
    pub run_time: u64,
    /// Whether or not the request was successful.
    pub success: bool,
    /// Which GooseUser thread processed the request.
    pub user: usize,
    /// The microseconds past [`run_time`](Self::run_time)'s whole
    /// milliseconds, from 0 to 999. [`run_time_micros`](Self::run_time_micros)
    /// gives both together. 0 when read from a log written before Goose kept
    /// them.
    #[serde(default)]
    pub run_time_fraction_us: u16,
}
impl TransactionMetric {
    /// Create a new TransactionMetric metric.
    pub(crate) fn new(
        elapsed: u128,
        scenario_index: usize,
        transaction_index: usize,
        name: Arc<str>,
        user: usize,
    ) -> Self {
        TransactionMetric {
            elapsed: elapsed as u64,
            scenario_index,
            transaction_index,
            name,
            run_time: 0,
            success: true,
            user,
            run_time_fraction_us: 0,
        }
    }

    /// Record how long the transaction ran and whether it succeeded.
    pub(crate) fn set_duration(&mut self, run_time: Duration, success: bool) {
        let us = duration_micros(run_time);
        self.run_time = us / 1000;
        self.run_time_fraction_us = (us % 1000) as u16;
        self.success = success;
    }

    /// How long the transaction ran, in microseconds.
    pub fn run_time_micros(&self) -> u64 {
        micros(self.run_time, self.run_time_fraction_us)
    }
}

/// Aggregated per-transaction metrics updated each time a transaction is invoked.
///
/// [`TransactionMetric`]s are sent by [`GooseUser`](../goose/struct.GooseUser.html)
/// threads to the Goose parent process where they are aggregated together into this
/// structure, and stored in [`GooseMetrics::transactions`].
///
/// It serializes its [`run_times`](Self::run_times) flat: `min_time`,
/// `max_time` and `total_time` in whole milliseconds, `counter`, and
/// `times_us`, `min_time_us`, `max_time_us` and `total_time_us`; an
/// earlier Goose, which reads whole millisecond `times`, can not read it.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct TransactionMetricAggregate {
    /// An index into [`GooseAttack`](../struct.GooseAttack.html)`.scenarios`,
    /// indicating which scenario this is.
    pub scenario_index: usize,
    /// The scenario name.
    pub scenario_name: Arc<str>,
    /// An index into [`Scenario`](../goose/struct.Scenario.html)`.transaction`,
    /// indicating which transaction this is.
    pub transaction_index: usize,
    /// An optional name for the transaction.
    pub transaction_name: TransactionName,
    /// The transaction's run times, in microseconds, and how many times it
    /// ran.
    pub run_times: GooseRequestMetricTimingData,
    /// Total number of times transaction has run successfully.
    pub success_count: usize,
    /// Total number of times transaction has failed.
    pub fail_count: usize,
}
impl TransactionMetricAggregate {
    /// Create a new TransactionMetricAggregate.
    pub(crate) fn new(
        scenario_index: usize,
        scenario_name: Arc<str>,
        transaction_index: usize,
        transaction_name: TransactionName,
    ) -> Self {
        TransactionMetricAggregate {
            scenario_index,
            scenario_name,
            transaction_index,
            transaction_name,
            run_times: GooseRequestMetricTimingData::default(),
            success_count: 0,
            fail_count: 0,
        }
    }

    /// Track a transaction's run time in whole milliseconds.
    #[cfg(test)]
    pub(crate) fn set_time(&mut self, time: u64, success: bool) {
        self.set_time_us(time.saturating_mul(1000), success);
    }

    /// Track a transaction's run time in microseconds.
    pub(crate) fn set_time_us(&mut self, us: u64, success: bool) {
        self.run_times.record_time_us(us);
        if success {
            self.success_count += 1;
        } else {
            self.fail_count += 1;
        }
    }
}

/// [`TransactionMetricAggregate`] as it is written.
#[derive(Serialize)]
struct TransactionAggregateOut<'a> {
    scenario_index: usize,
    scenario_name: &'a str,
    transaction_index: usize,
    transaction_name: &'a TransactionName,
    #[serde(flatten)]
    times: RunTimesOut<'a>,
    success_count: usize,
    fail_count: usize,
}

/// [`TransactionMetricAggregate`] as it is read.
#[derive(Deserialize)]
struct TransactionAggregateIn {
    scenario_index: usize,
    #[serde(with = "arc_str_serde")]
    scenario_name: Arc<str>,
    transaction_index: usize,
    transaction_name: TransactionName,
    #[serde(default)]
    times: BTreeMap<usize, usize>,
    #[serde(default)]
    min_time: usize,
    #[serde(default)]
    max_time: usize,
    #[serde(default)]
    total_time: usize,
    #[serde(default)]
    counter: usize,
    times_us: Option<TimeBuckets>,
    #[serde(default)]
    min_time_us: Option<u64>,
    #[serde(default)]
    max_time_us: Option<u64>,
    #[serde(default)]
    total_time_us: Option<u64>,
    success_count: usize,
    fail_count: usize,
}

impl Serialize for TransactionMetricAggregate {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        TransactionAggregateOut {
            scenario_index: self.scenario_index,
            scenario_name: &self.scenario_name,
            transaction_index: self.transaction_index,
            transaction_name: &self.transaction_name,
            times: RunTimesOut::of(&self.run_times),
            success_count: self.success_count,
            fail_count: self.fail_count,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TransactionMetricAggregate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let data = TransactionAggregateIn::deserialize(deserializer)?;
        Ok(TransactionMetricAggregate {
            scenario_index: data.scenario_index,
            scenario_name: data.scenario_name,
            transaction_index: data.transaction_index,
            transaction_name: data.transaction_name,
            run_times: run_times(
                data.times,
                [data.min_time, data.max_time, data.total_time],
                data.counter,
                data.times_us,
                [data.min_time_us, data.max_time_us, data.total_time_us],
            ),
            success_count: data.success_count,
            fail_count: data.fail_count,
        })
    }
}

/// Run times as transaction and scenario aggregates write them: the fastest,
/// slowest and total in whole milliseconds and the count, then the buckets
/// and the same times in microseconds.
#[derive(Serialize)]
struct RunTimesOut<'a> {
    min_time: usize,
    max_time: usize,
    total_time: usize,
    counter: usize,
    times_us: &'a TimeBuckets,
    min_time_us: u64,
    max_time_us: u64,
    total_time_us: u64,
}

impl<'a> RunTimesOut<'a> {
    fn of(data: &'a GooseRequestMetricTimingData) -> Self {
        RunTimesOut {
            min_time: data.minimum_time_ms(),
            max_time: data.maximum_time_ms(),
            total_time: us_to_ms(data.total_time_us),
            counter: data.counter,
            times_us: &data.times_us,
            min_time_us: data.minimum_time_us,
            max_time_us: data.maximum_time_us,
            total_time_us: data.total_time_us,
        }
    }
}

/// A request's, transaction's or scenario's timing data as read from a file:
/// with microseconds (`times_us` and the fastest, slowest and total), or only
/// the whole milliseconds of a file from an earlier Goose.
fn run_times(
    times: BTreeMap<usize, usize>,
    [min, max, total]: [usize; 3],
    counter: usize,
    times_us: Option<TimeBuckets>,
    [min_us, max_us, total_us]: [Option<u64>; 3],
) -> GooseRequestMetricTimingData {
    match times_us {
        Some(times_us) => GooseRequestMetricTimingData {
            times_us,
            minimum_time_us: min_us.unwrap_or(0),
            maximum_time_us: max_us.unwrap_or(0),
            total_time_us: total_us.unwrap_or(0),
            counter,
        },
        None => GooseRequestMetricTimingData {
            times_us: microsecond_times(&times),
            minimum_time_us: ms_to_us(min),
            maximum_time_us: ms_to_us(max),
            total_time_us: ms_to_us(total),
            counter,
        },
    }
}

/// Aggregated per-scenario metrics updated each time a scenario is run.
///
/// [`ScenarioMetric`]s are sent by [`GooseUser`](../goose/struct.GooseUser.html)
/// threads to the Goose parent process where they are aggregated together into this
/// structure, and stored in [`GooseMetrics::transactions`].
///
/// It serializes its [`run_times`](Self::run_times) flat: `min_time`,
/// `max_time` and `total_time` in whole milliseconds, `counter`, and
/// `times_us`, `min_time_us`, `max_time_us` and `total_time_us`; an
/// earlier Goose, which reads whole millisecond `times`, can not read it.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ScenarioMetricAggregate {
    /// An index into [`GooseAttack`](../struct.GooseAttack.html)`.scenarios`,
    /// indicating which scenario this is.
    pub index: usize,
    /// The scenario name.
    pub name: Arc<str>,
    /// List of users running this scenario.
    pub users: HashSet<usize>,
    /// The scenario's run times, in microseconds, and how many times it ran.
    pub run_times: GooseRequestMetricTimingData,
}
impl ScenarioMetricAggregate {
    /// Create a new ScenarioMetricAggregate.
    pub(crate) fn new(index: usize, name: Arc<str>) -> Self {
        ScenarioMetricAggregate {
            index,
            name,
            users: HashSet::new(),
            run_times: GooseRequestMetricTimingData::default(),
        }
    }

    /// Track a scenario's run time in whole milliseconds.
    #[cfg(test)]
    pub(crate) fn update(&mut self, time: u64, user: usize) {
        self.update_us(time.saturating_mul(1000), user);
    }

    /// Track a scenario's run time in microseconds, and the user that ran it.
    pub(crate) fn update_us(&mut self, us: u64, user: usize) {
        // Record each different user running this scenario.
        self.users.insert(user);
        self.run_times.record_time_us(us);
    }
}

/// [`ScenarioMetricAggregate`] as it is written.
#[derive(Serialize)]
struct ScenarioAggregateOut<'a> {
    index: usize,
    name: &'a str,
    users: &'a HashSet<usize>,
    #[serde(flatten)]
    times: RunTimesOut<'a>,
}

/// [`ScenarioMetricAggregate`] as it is read.
#[derive(Deserialize)]
struct ScenarioAggregateIn {
    index: usize,
    #[serde(with = "arc_str_serde")]
    name: Arc<str>,
    users: HashSet<usize>,
    #[serde(default)]
    times: BTreeMap<usize, usize>,
    #[serde(default)]
    min_time: usize,
    #[serde(default)]
    max_time: usize,
    #[serde(default)]
    total_time: usize,
    #[serde(default)]
    counter: usize,
    times_us: Option<TimeBuckets>,
    #[serde(default)]
    min_time_us: Option<u64>,
    #[serde(default)]
    max_time_us: Option<u64>,
    #[serde(default)]
    total_time_us: Option<u64>,
}

impl Serialize for ScenarioMetricAggregate {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ScenarioAggregateOut {
            index: self.index,
            name: &self.name,
            users: &self.users,
            times: RunTimesOut::of(&self.run_times),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ScenarioMetricAggregate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let data = ScenarioAggregateIn::deserialize(deserializer)?;
        Ok(ScenarioMetricAggregate {
            index: data.index,
            name: data.name,
            users: data.users,
            run_times: run_times(
                data.times,
                [data.min_time, data.max_time, data.total_time],
                data.counter,
                data.times_us,
                [data.min_time_us, data.max_time_us, data.total_time_us],
            ),
        })
    }
}
/// All metrics optionally collected during a Goose load test.
///
/// By default, Goose collects metrics during a load test in a `GooseMetrics` object
/// that is returned by
/// [`GooseAttack::execute()`](../struct.GooseAttack.html#method.execute) when a load
/// test finishes.
///
/// # Example
/// ```rust
/// use goose::prelude::*;
///
/// #[tokio::main]
/// async fn main() -> Result<(), GooseError> {
///     let goose_metrics: GooseMetrics = GooseAttack::initialize()?
///         .register_scenario(scenario!("ExampleUsers")
///             .register_transaction(transaction!(example_transaction))
///         )
///         // Set a default host so the load test will start.
///         .set_default(GooseDefault::Host, "http://localhost/")?
///         // Set a default run time so this test runs to completion.
///         .set_default(GooseDefault::RunTime, 1)?
///         .execute()
///         .await?;
///
///     // It is now possible to do something with the metrics collected by Goose.
///     // For now, we'll just pretty-print the entire object.
///     println!("{:#?}", goose_metrics);
///
///     /**
///     // For example:
///     $ cargo run -- -H http://example.com -u1 -t1
///     GooseMetrics {
///         hash: 0,
///         started: Some(
///             2021-06-15T09:32:49.888147+02:00,
///         ),
///         duration: 1,
///         users: 1,
///         requests: {
///             "GET /": GooseRequestMetricAggregate {
///                 path: "/",
///                 method: Get,
///                 response_times: {
///                     3: 14,
///                     4: 163,
///                     5: 36,
///                     6: 8,
///                 },
///                 min_response_time: 3,
///                 max_response_time: 6,
///                 total_response_time: 922,
///                 response_time_counter: 221,
///                 status_code_counts: {},
///                 success_count: 0,
///                 fail_count: 221,
///                 load_test_hash: 0,
///             },
///         },
///         transactions: [
///             [
///                 TransactionMetricAggregate {
///                     scenario_index: 0,
///                     scenario_name: "ExampleUsers",
///                     transaction_index: 0,
///                     transaction_name: "",
///                     times: {
///                         3: 14,
///                         4: 161,
///                         5: 38,
///                         6: 8,
///                     },
///                     min_time: 3,
///                     max_time: 6,
///                     total_time: 924,
///                     counter: 221,
///                     success_count: 221,
///                     fail_count: 0,
///                 },
///             ],
///         ],
///         errors: {
///             "503 Service Unavailable: /.GET./": GooseErrorMetric {
///                 method: Get,
///                 name: "/",
///                 error: "503 Service Unavailable: /",
///                 occurrences: 221,
///             },
///         },
///         final_metrics: true,
///         display_status_codes: false,
///         display_metrics: true,
///     }
///     **/
///
///     Ok(())
/// }
///
/// async fn example_transaction(user: &mut GooseUser) -> TransactionResult {
///     let _goose = user.get("/").await?;
///
///     Ok(())
/// }
/// ```
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GooseMetrics {
    /// A hash of the load test, primarily used to validate all Workers in a Gaggle
    /// are running the same load test.
    pub hash: u64,
    /// A vector recording the history of each load test step.
    #[serde(default)]
    pub history: Vec<TestPlanHistory>,
    /// Total number of seconds the load test ran.
    pub duration: usize,
    /// Maximum number of users simulated during this load test.
    ///
    /// This value may be smaller than what was configured at start time if the test
    /// didn't run long enough for all configured users to start.
    pub maximum_users: usize,
    /// Total number of users simulated during this load test.
    pub total_users: usize,
    /// Tracks details about each request made during the load test.
    ///
    /// Can be disabled with the `--no-metrics` run-time option, or with
    /// [GooseDefault::NoMetrics](../config/enum.GooseDefault.html#variant.NoMetrics).
    pub requests: GooseRequestMetrics,
    /// Transactions details about each transaction that is invoked during the load test.
    ///
    /// Can be disabled with either the `--no-transaction-metrics` or `--no-metrics` run-time options,
    /// or with either the
    /// [GooseDefault::NoTransactionMetrics](../config/enum.GooseDefault.html#variant.NoTransactionMetrics) or
    /// [GooseDefault::NoMetrics](../config/enum.GooseDefault.html#variant.NoMetrics).
    pub transactions: TransactionMetrics,
    /// Transactions details about each scenario that is invoked during the load test.
    ///
    /// Can be disabled with either the `--no-scenario-metrics` or `--no-metrics` run-time options,
    /// or with either the
    /// [GooseDefault::NoTransactionMetrics](../config/enum.GooseDefault.html#variant.NoTransactionMetrics) or
    /// [GooseDefault::NoMetrics](../config/enum.GooseDefault.html#variant.NoMetrics).
    pub scenarios: ScenarioMetrics,
    /// Tracks and counts each time an error is detected during the load test.
    ///
    /// Can be disabled with either the `--no-error-summary` or `--no-metrics` run-time options,
    /// or with either the
    /// [GooseDefault::NoErrorSummary](../config/enum.GooseDefault.html#variant.NoErrorSummary) or
    /// [GooseDefault::NoMetrics](../config/enum.GooseDefault.html#variant.NoMetrics).
    pub errors: GooseErrorMetrics,
    /// Tracks all hosts that the load test is run against.
    pub hosts: HashSet<String>,
    /// Enhanced Coordinated Omission metrics tracking.
    ///
    /// Tracks CO events, synthetic vs actual requests, and provides detailed
    /// analysis of coordinated omission impact on the load test.
    pub coordinated_omission_metrics: Option<CoordinatedOmissionMetrics>,
    /// The result of each `--threshold` rule, in the order given, judged when the
    /// run ended; empty without thresholds.
    ///
    /// Set with `--threshold` or
    /// [GooseDefault::Threshold](../config/enum.GooseDefault.html#variant.Threshold).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thresholds: Vec<ThresholdResult>,
    /// The index of the threshold that stopped the run while it was going, and what
    /// it observed then, if one did.
    #[serde(skip)]
    pub(crate) threshold_abort: Option<(usize, String)>,
    /// What `rps` thresholds count: the time the run held its users steady.
    #[serde(skip)]
    pub(crate) rps_window: crate::threshold::RpsWindow,
    /// Flag indicating whether or not these are the final metrics, used to determine
    /// which metrics should be displayed. Defaults to false.
    pub(crate) final_metrics: bool,
    /// Flag indicating whether or not to display status_codes. Defaults to false.
    pub(crate) display_status_codes: bool,
    /// Flag indicating whether or not to display metrics. This defaults to false on
    /// Workers, otherwise true.
    pub(crate) display_metrics: bool,
}
impl GooseMetrics {
    /// Initialize the transaction_metrics vector, and determine which hosts are being
    /// load tested to display when printing metrics.
    pub(crate) fn initialize_transaction_metrics(
        &mut self,
        scenarios: &[Scenario],
        config: &GooseConfiguration,
        defaults: &GooseDefaults,
    ) -> Result<(), GooseError> {
        self.transactions = Vec::new();
        // Don't initialize transaction metrics if metrics or transaction_metrics are disabled.
        if !config.no_metrics {
            for scenario in scenarios {
                if !config.no_transaction_metrics {
                    let mut transaction_vector = Vec::new();
                    for transaction in &scenario.transactions {
                        transaction_vector.push(TransactionMetricAggregate::new(
                            scenario.scenarios_index,
                            scenario.name.clone(),
                            transaction.transactions_index,
                            transaction.name.clone(),
                        ));
                    }
                    self.transactions.push(transaction_vector);
                }

                // Determine the base_url for this transaction based on which of the following
                // are configured so metrics can be printed.
                self.hosts.insert(
                    get_base_url(
                        // Determine if --host was configured.
                        config.host().map(str::to_string),
                        // Determine if the scenario defines a host.
                        scenario.host.clone(),
                        // Determine if there is a default host.
                        defaults.host.clone(),
                    )?
                    .to_string(),
                );
            }
        }

        Ok(())
    }

    /// Initialize the scenario_metrics vector.
    pub(crate) fn initialize_scenario_metrics(
        &mut self,
        scenarios: &[Scenario],
        config: &GooseConfiguration,
    ) {
        if !config.no_metrics && !config.no_scenario_metrics {
            self.scenarios = Vec::new();
            for scenario in scenarios {
                self.scenarios.push(ScenarioMetricAggregate::new(
                    scenario.scenarios_index,
                    scenario.name.clone(),
                ));
            }
        }
    }

    /// Displays metrics while a load test is running.
    ///
    /// This function is invoked one time immediately after all GooseUsers are
    /// started, unless the `--no-reset-metrics` run-time option is enabled. It
    /// is invoked at regular intervals if the `--running-metrics` run-time
    /// option is enabled.
    pub(crate) fn print_running(&self) {
        if self.display_metrics {
            info!(
                "printing running metrics after {} seconds...",
                self.duration
            );

            // Include a blank line after printing running metrics.
            println!("{self}");
        }
    }

    /// Optionally prepares a table of requests and fails.
    ///
    /// This function is invoked by `GooseMetrics::print()` and
    /// `GooseMetrics::print_running()`.
    pub(crate) fn fmt_requests(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if self.requests.is_empty() {
            return Ok(());
        }

        // Display metrics from merged HashMap
        writeln!(
            fmt,
            "\n === PER REQUEST METRICS ===\n ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<24} | {:>13} | {:>14} | {:>8} | {:>7}",
            "Name", "# reqs", "# fails", "req/s", "fail/s"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        let mut aggregate_fail_count = 0;
        let mut aggregate_total_count = 0;
        for (request_key, request) in self.requests.iter().sorted() {
            let total_count = request.success_count + request.fail_count;
            let fail_percent = if request.fail_count > 0 {
                request.fail_count as f32 / total_count as f32 * 100.0
            } else {
                0.0
            };
            let (reqs, fails) =
                per_second_calculations(self.duration, total_count, request.fail_count);
            let reqs_precision = determine_precision(reqs);
            let fails_precision = determine_precision(fails);
            // Compress 100.0 and 0.0 to 100 and 0 respectively to save width.
            if fail_percent as usize == 100 || fail_percent as usize == 0 {
                let fail_and_percent = format!(
                    "{} ({}%)",
                    request.fail_count.to_formatted_string(&Locale::en),
                    fail_percent as usize
                );
                writeln!(
                    fmt,
                    " {:<24} | {:>13} | {:>14} | {:>8.reqs_p$} | {:>7.fails_p$}",
                    util::truncate_string(request_key, 24),
                    total_count.to_formatted_string(&Locale::en),
                    fail_and_percent,
                    reqs,
                    fails,
                    reqs_p = reqs_precision,
                    fails_p = fails_precision,
                )?;
            } else {
                let fail_and_percent = format!(
                    "{} ({:.1}%)",
                    request.fail_count.to_formatted_string(&Locale::en),
                    fail_percent
                );
                writeln!(
                    fmt,
                    " {:<24} | {:>13} | {:>14} | {:>8.reqs_p$} | {:>7.fails_p$}",
                    util::truncate_string(request_key, 24),
                    total_count.to_formatted_string(&Locale::en),
                    fail_and_percent,
                    reqs,
                    fails,
                    reqs_p = reqs_precision,
                    fails_p = fails_precision,
                )?;
            }
            aggregate_total_count += total_count;
            aggregate_fail_count += request.fail_count;
        }
        if self.requests.len() > 1 {
            let aggregate_fail_percent = if aggregate_fail_count > 0 {
                aggregate_fail_count as f32 / aggregate_total_count as f32 * 100.0
            } else {
                0.0
            };
            writeln!(
                fmt,
                " -------------------------+---------------+----------------+----------+--------"
            )?;
            let (reqs, fails) =
                per_second_calculations(self.duration, aggregate_total_count, aggregate_fail_count);
            let reqs_precision = determine_precision(reqs);
            let fails_precision = determine_precision(fails);
            // Compress 100.0 and 0.0 to 100 and 0 respectively to save width.
            if aggregate_fail_percent as usize == 100 || aggregate_fail_percent as usize == 0 {
                let fail_and_percent = format!(
                    "{} ({}%)",
                    aggregate_fail_count.to_formatted_string(&Locale::en),
                    aggregate_fail_percent as usize
                );
                writeln!(
                    fmt,
                    " {:<24} | {:>13} | {:>14} | {:>8.reqs_p$} | {:>7.fails_p$}",
                    "Aggregated",
                    aggregate_total_count.to_formatted_string(&Locale::en),
                    fail_and_percent,
                    reqs,
                    fails,
                    reqs_p = reqs_precision,
                    fails_p = fails_precision,
                )?;
            } else {
                let fail_and_percent = format!(
                    "{} ({:.1}%)",
                    aggregate_fail_count.to_formatted_string(&Locale::en),
                    aggregate_fail_percent
                );
                writeln!(
                    fmt,
                    " {:<24} | {:>13} | {:>14} | {:>8.reqs_p$} | {:>7.fails_p$}",
                    "Aggregated",
                    aggregate_total_count.to_formatted_string(&Locale::en),
                    fail_and_percent,
                    reqs,
                    fails,
                    reqs_p = reqs_precision,
                    fails_p = fails_precision,
                )?;
            }
        }

        Ok(())
    }

    /// Optionally prepares a table of transactions.
    ///
    /// This function is invoked by `GooseMetrics::print()` and
    /// `GooseMetrics::print_running()`.
    pub(crate) fn fmt_transactions(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if self.transactions.is_empty() || !self.display_metrics {
            return Ok(());
        }

        // Display metrics from transactions Vector
        writeln!(
            fmt,
            "\n === PER TRANSACTION METRICS ===\n ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<24} | {:>13} | {:>14} | {:>8} | {:>7}",
            "Name", "# times run", "# fails", "trans/s", "fail/s"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        let mut aggregate_fail_count = 0;
        let mut aggregate_total_count = 0;
        let mut transaction_count = 0;
        for scenario in &self.transactions {
            let mut displayed_scenario = false;
            for transaction in scenario {
                transaction_count += 1;
                let total_count = transaction.success_count + transaction.fail_count;
                let fail_percent = if transaction.fail_count > 0 {
                    transaction.fail_count as f32 / total_count as f32 * 100.0
                } else {
                    0.0
                };
                let (runs, fails) =
                    per_second_calculations(self.duration, total_count, transaction.fail_count);
                let runs_precision = determine_precision(runs);
                let fails_precision = determine_precision(fails);

                // First time through display name of scenario.
                if !displayed_scenario {
                    writeln!(
                        fmt,
                        " {:24 }",
                        util::truncate_string(
                            &format!(
                                "{}: {}",
                                transaction.scenario_index + 1,
                                transaction.scenario_name
                            ),
                            60
                        ),
                    )?;
                    displayed_scenario = true;
                }

                if fail_percent as usize == 100 || fail_percent as usize == 0 {
                    let fail_and_percent = format!(
                        "{} ({}%)",
                        transaction.fail_count.to_formatted_string(&Locale::en),
                        fail_percent as usize
                    );
                    writeln!(
                        fmt,
                        " {:<24} | {:>13} | {:>14} | {:>8.runs_p$} | {:>7.fails_p$}",
                        util::truncate_string(
                            &format!(
                                "  {}: {}",
                                transaction.transaction_index + 1,
                                transaction.transaction_name.name_for_transaction()
                            ),
                            24
                        ),
                        total_count.to_formatted_string(&Locale::en),
                        fail_and_percent,
                        runs,
                        fails,
                        runs_p = runs_precision,
                        fails_p = fails_precision,
                    )?;
                } else {
                    let fail_and_percent = format!(
                        "{} ({:.1}%)",
                        transaction.fail_count.to_formatted_string(&Locale::en),
                        fail_percent
                    );
                    writeln!(
                        fmt,
                        " {:<24} | {:>13} | {:>14} | {:>8.runs_p$} | {:>7.fails_p$}",
                        util::truncate_string(
                            &format!(
                                "  {}: {}",
                                transaction.transaction_index + 1,
                                transaction.transaction_name.name_for_transaction()
                            ),
                            24
                        ),
                        total_count.to_formatted_string(&Locale::en),
                        fail_and_percent,
                        runs,
                        fails,
                        runs_p = runs_precision,
                        fails_p = fails_precision,
                    )?;
                }
                aggregate_total_count += total_count;
                aggregate_fail_count += transaction.fail_count;
            }
        }
        if transaction_count > 1 {
            let aggregate_fail_percent = if aggregate_fail_count > 0 {
                aggregate_fail_count as f32 / aggregate_total_count as f32 * 100.0
            } else {
                0.0
            };
            writeln!(
                fmt,
                " -------------------------+---------------+----------------+----------+--------"
            )?;
            let (runs, fails) =
                per_second_calculations(self.duration, aggregate_total_count, aggregate_fail_count);
            let runs_precision = determine_precision(runs);
            let fails_precision = determine_precision(fails);

            // Compress 100.0 and 0.0 to 100 and 0 respectively to save width.
            if aggregate_fail_percent as usize == 100 || aggregate_fail_percent as usize == 0 {
                let fail_and_percent = format!(
                    "{} ({}%)",
                    aggregate_fail_count.to_formatted_string(&Locale::en),
                    aggregate_fail_percent as usize
                );
                writeln!(
                    fmt,
                    " {:<24} | {:>13} | {:>14} | {:>8.runs_p$} | {:>7.fails_p$}",
                    "Aggregated",
                    aggregate_total_count.to_formatted_string(&Locale::en),
                    fail_and_percent,
                    runs,
                    fails,
                    runs_p = runs_precision,
                    fails_p = fails_precision,
                )?;
            } else {
                let fail_and_percent = format!(
                    "{} ({:.1}%)",
                    aggregate_fail_count.to_formatted_string(&Locale::en),
                    aggregate_fail_percent
                );
                writeln!(
                    fmt,
                    " {:<24} | {:>13} | {:>14} | {:>8.runs_p$} | {:>7.fails_p$}",
                    "Aggregated",
                    aggregate_total_count.to_formatted_string(&Locale::en),
                    fail_and_percent,
                    runs,
                    fails,
                    runs_p = runs_precision,
                    fails_p = fails_precision,
                )?;
            }
        }

        Ok(())
    }

    /// Optionally prepares a table of transaction times.
    ///
    /// This function is invoked by `GooseMetrics::print()` and
    /// `GooseMetrics::print_running()`.
    pub(crate) fn fmt_transaction_times(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if self.transactions.is_empty() || !self.display_metrics {
            return Ok(());
        }

        // Every transaction's times together.
        let mut aggregate = GooseRequestMetricTimingData::default();
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<24} | {:>11} | {:>10} | {:>11} | {:>10}",
            "Name", "Avg (ms)", "Min", "Max", "Median"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        let mut transaction_count = 0;
        for scenario in &self.transactions {
            let mut displayed_scenario = false;
            for transaction in scenario {
                transaction_count += 1;
                // First time through display name of scenario.
                if !displayed_scenario {
                    writeln!(
                        fmt,
                        " {:24}",
                        util::truncate_string(
                            &format!(
                                "{}: {}",
                                transaction.scenario_index + 1,
                                transaction.scenario_name
                            ),
                            60
                        ),
                    )?;
                    displayed_scenario = true;
                }

                aggregate.merge(&transaction.run_times);
                write_times_row(
                    fmt,
                    &util::truncate_string(
                        &format!(
                            "  {}: {}",
                            transaction.transaction_index + 1,
                            transaction.transaction_name.name_for_transaction()
                        ),
                        24,
                    ),
                    &transaction.run_times,
                )?;
            }
        }
        if transaction_count > 1 {
            writeln!(
                fmt,
                " -------------------------+-------------+------------+-------------+-----------"
            )?;
            write_times_row(fmt, "Aggregated", &aggregate)?;
        }

        Ok(())
    }

    /// Optionally prepares a table of scenarios.
    ///
    /// This function is invoked by `GooseMetrics::print()` and
    /// `GooseMetrics::print_running()`.
    pub(crate) fn fmt_scenarios(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if self.scenarios.is_empty() || !self.display_metrics {
            return Ok(());
        }

        // Display metrics from scenarios Vector
        writeln!(
            fmt,
            "\n === PER SCENARIO METRICS ===\n ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<24} | {:>8} | {:>12} | {:>11} | {:10}",
            "Name", "# users", "# times run", "scenarios/s", "iterations"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        let mut aggregate_users = 0;
        let mut aggregate_counter = 0;
        for scenario in &self.scenarios {
            aggregate_users += scenario.users.len();
            aggregate_counter += scenario.run_times.counter;
            let (runs, _fails) =
                per_second_calculations(self.duration, scenario.run_times.counter, 0);
            let runs_precision = determine_precision(runs);
            let iterations = scenario.run_times.counter as f32 / scenario.users.len() as f32;
            let iterations_precision = determine_precision(iterations);
            writeln!(
                fmt,
                " {:24 } | {:>8} | {:>12} | {:>11.runs_p$} | {:>10.iterations_p$}",
                util::truncate_string(&format!("{}: {}", scenario.index + 1, scenario.name,), 24),
                scenario.users.len(),
                scenario.run_times.counter,
                runs,
                iterations,
                runs_p = runs_precision,
                iterations_p = iterations_precision,
            )?;
        }
        // If more than 1 scenario is defined, also display aggregated metrics.
        if self.scenarios.len() > 1 {
            let (aggregate_runs, _fails) =
                per_second_calculations(self.duration, aggregate_counter, 0);
            let aggregate_runs_precision = determine_precision(aggregate_runs);
            let aggregate_iterations = aggregate_counter as f32 / aggregate_users as f32;
            let aggregate_iterations_precision = determine_precision(aggregate_iterations);
            writeln!(
                fmt,
                " -------------------------+----------+--------------+-------------+------------"
            )?;
            writeln!(
                fmt,
                " {:24 } | {:>8} | {:>12} | {:>11.runs_p$} | {:>10.iterations_p$}",
                "Aggregated",
                aggregate_users,
                aggregate_counter,
                aggregate_runs,
                aggregate_iterations,
                runs_p = aggregate_runs_precision,
                iterations_p = aggregate_iterations_precision,
            )?;
        }

        Ok(())
    }

    /// Optionally prepares a table of scenario times.
    ///
    /// This function is invoked by `GooseMetrics::print()` and
    /// `GooseMetrics::print_running()`.
    pub(crate) fn fmt_scenario_times(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if self.scenarios.is_empty() || !self.display_metrics {
            return Ok(());
        }

        // Every scenario's times together.
        let mut aggregate = GooseRequestMetricTimingData::default();
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<24} | {:>11} | {:>10} | {:>11} | {:>10}",
            "Name", "Avg (ms)", "Min", "Max", "Median"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        for scenario in &self.scenarios {
            aggregate.merge(&scenario.run_times);
            write_times_row(
                fmt,
                &util::truncate_string(&format!("  {}: {}", scenario.index + 1, scenario.name), 24),
                &scenario.run_times,
            )?;
        }
        if self.scenarios.len() > 1 {
            writeln!(
                fmt,
                " -------------------------+-------------+------------+-------------+-----------"
            )?;
            write_times_row(fmt, "Aggregated", &aggregate)?;
        }

        Ok(())
    }

    /// Optionally prepares a table of response times.
    ///
    /// This function is invoked by `GooseMetrics::print()` and
    /// `GooseMetrics::print_running()`.
    pub(crate) fn fmt_response_times(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if self.requests.is_empty() {
            return Ok(());
        }

        // Every request's measured times together.
        let mut aggregate_raw = GooseRequestMetricTimingData::new(None);
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<24} | {:>11} | {:>10} | {:>11} | {:>10}",
            "Name", "Avg (ms)", "Min", "Max", "Median"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;

        // First display the raw data, as it always exists.
        let mut co_data = false;
        for (request_key, request) in self.requests.iter().sorted() {
            if !co_data && request.coordinated_omission_data.is_some() {
                co_data = true;
            }

            // Merge in all times from this request into an aggregate.
            aggregate_raw.merge(&request.raw_data);

            write_times_row(
                fmt,
                &util::truncate_string(request_key, 24),
                &request.raw_data,
            )?;

            // Display per-status-code breakdown when multiple status codes exist.
            if let Some(breakdowns) = request.status_code_breakdowns() {
                for b in breakdowns {
                    writeln!(
                        fmt,
                        " {:<24} | {:>11} | {:>10} | {:>11} | {:>10}",
                        format!(
                            "  \u{2514}\u{2500} {} ({:.1}%)",
                            b.status_code, b.percentage
                        ),
                        format_ms(NullableFloat::from(b.average).0, true),
                        format_micros(b.min_time_us, true),
                        format_micros(b.max_time_us, true),
                        "-",
                    )?;
                }
            }
        }

        let raw_average = aggregate_raw.average_ms_or_zero();

        // Display aggregated data if there was more than one request.
        if self.requests.len() > 1 {
            writeln!(
                fmt,
                " -------------------------+-------------+------------+-------------+-----------"
            )?;
            write_times_row(fmt, "Aggregated", &aggregate_raw)?;
        }

        // Nothing more to display if there was no Coordinated Omission data collected.
        if !co_data {
            return Ok(());
        }

        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(fmt, " Adjusted for Coordinated Omission:")?;

        // Every request's adjusted times together: its measured times plus
        // the synthetic ones, or only its measured times when mitigation
        // added none to it.
        let mut aggregate_co = GooseRequestMetricTimingData::new(None);
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<24} | {:>11} | {:>10} | {:>11} | {:>10}",
            "Name", "Avg (ms)", "Std Dev", "Max", "Median"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;

        // Now display Coordinated Omission data.
        for (request_key, request) in self.requests.iter().sorted() {
            aggregate_co.merge(
                request
                    .coordinated_omission_data
                    .as_ref()
                    .unwrap_or(&request.raw_data),
            );
            // Coordinated Omission Mitigation was enabled for this request, display the extra data:
            if let Some(co_data) = request.coordinated_omission_data.as_ref() {
                let co_average = co_data.average_ms_or_zero();
                let standard_deviation = util::standard_deviation(
                    request.raw_data.average_ms_or_zero() as f32,
                    co_average as f32,
                );
                writeln!(
                    fmt,
                    " {:<24} | {:>11} | {:>10} | {:>11} | {:>10}",
                    util::truncate_string(request_key, 24),
                    format_ms(co_average, true),
                    format_ms(NullableFloat::from(standard_deviation).0, true),
                    format_micros(co_data.maximum_time_us, true),
                    format_micros(co_data.percentile_us(0.5), true),
                )?;
            } else {
                writeln!(
                    fmt,
                    " {:<24} | {:>11} | {:>10} | {:>11} | {:>10}",
                    util::truncate_string(request_key, 24),
                    "-",
                    "-",
                    "-",
                    "-",
                )?;
            }
        }

        // Display aggregated Coordinate Omission data if there was more than one request.
        if self.requests.len() > 1 {
            let co_average = aggregate_co.average_ms_or_zero();
            let standard_deviation =
                util::standard_deviation(raw_average as f32, co_average as f32);

            writeln!(
                fmt,
                " -------------------------+-------------+------------+-------------+-----------"
            )?;

            writeln!(
                fmt,
                " {:<24} | {:>11} | {:>10} | {:>11} | {:>10}",
                "Aggregated",
                format_ms(co_average, true),
                format_ms(NullableFloat::from(standard_deviation).0, true),
                format_micros(aggregate_co.maximum_time_us, true),
                format_micros(aggregate_co.percentile_us(0.5), true),
            )?;
        }

        Ok(())
    }

    /// Optionally prepares a table of slowest response times within several percentiles.
    ///
    /// This function is invoked by `GooseMetrics::print()` and
    /// `GooseMetrics::print_running()`.
    pub(crate) fn fmt_percentiles(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Only include percentiles when displaying the final metrics report.
        if !self.final_metrics {
            return Ok(());
        }

        /// The table's percentiles of these times.
        fn row(data: &GooseRequestMetricTimingData) -> [u64; 6] {
            [0.5, 0.75, 0.98, 0.99, 0.999, 0.9999].map(|percent| data.percentile_us(percent))
        }

        fn write_row(fmt: &mut fmt::Formatter<'_>, name: &str, cells: [String; 6]) -> fmt::Result {
            writeln!(
                fmt,
                " {:<24} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6}",
                name, cells[0], cells[1], cells[2], cells[3], cells[4], cells[5]
            )
        }

        // Every request's measured times together.
        let mut raw_aggregate = GooseRequestMetricTimingData::new(None);
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " Slowest page load within specified percentile of requests (in ms):"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<24} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6}",
            "Name", "50%", "75%", "98%", "99%", "99.9%", "99.99%"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        // Track whether or not Coordinated Omission Mitigation kicked in.
        let mut co_data = false;
        for (request_key, request) in self.requests.iter().sorted() {
            if !co_data && request.coordinated_omission_data.is_some() {
                co_data = true;
            }
            raw_aggregate.merge(&request.raw_data);
            write_row(
                fmt,
                &util::truncate_string(request_key, 24),
                row(&request.raw_data).map(|us| format_micros(us, false)),
            )?;
        }
        if self.requests.len() > 1 {
            writeln!(
                fmt,
                " -------------------------+--------+--------+--------+--------+--------+-------"
            )?;
            write_row(
                fmt,
                "Aggregated",
                row(&raw_aggregate).map(|us| format_micros(us, false)),
            )?;
        }

        // If there's no Coordinated Omission Mitigation data to display, exit.
        if !co_data {
            return Ok(());
        }

        // Every request's adjusted times together: its measured times plus
        // the synthetic ones, or only its measured times when mitigation
        // added none to it.
        let mut co_aggregate = GooseRequestMetricTimingData::new(None);

        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(fmt, " Adjusted for Coordinated Omission:")?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<24} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6}",
            "Name", "50%", "75%", "98%", "99%", "99.9%", "99.99%"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        for (request_key, request) in self.requests.iter().sorted() {
            co_aggregate.merge(
                request
                    .coordinated_omission_data
                    .as_ref()
                    .unwrap_or(&request.raw_data),
            );
            let name = util::truncate_string(request_key, 24);
            match request.coordinated_omission_data.as_ref() {
                Some(coordinated_omission_data) => write_row(
                    fmt,
                    &name,
                    row(coordinated_omission_data).map(|us| format_micros(us, false)),
                )?,
                None => write_row(fmt, &name, ["-"; 6].map(String::from))?,
            }
        }
        if self.requests.len() > 1 {
            writeln!(
                fmt,
                " -------------------------+--------+--------+--------+--------+--------+-------"
            )?;
            write_row(
                fmt,
                "Aggregated",
                row(&co_aggregate).map(|us| format_micros(us, false)),
            )?;
        }

        Ok(())
    }

    /// Optionally prepares a table of response status codes.
    ///
    /// This function is invoked by `GooseMetrics::print()` and
    /// `GooseMetrics::print_running()`.
    pub(crate) fn fmt_status_codes(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if !self.display_status_codes {
            return Ok(());
        }

        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(fmt, " {:<24} | {:>51} ", "Name", "Status codes")?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        let mut aggregated_status_code_counts: HashMap<u16, usize> = HashMap::new();
        for (request_key, request) in self.requests.iter().sorted() {
            let codes = prepare_status_codes(
                &request.status_code_counts,
                &mut Some(&mut aggregated_status_code_counts),
            );

            writeln!(
                fmt,
                " {:<24} | {:>51}",
                util::truncate_string(request_key, 24),
                codes,
            )?;
        }
        writeln!(
            fmt,
            " -------------------------+----------------------------------------------------"
        )?;
        let codes = prepare_status_codes(&aggregated_status_code_counts, &mut None);
        writeln!(fmt, " {:<24} | {:>51} ", "Aggregated", codes)?;

        Ok(())
    }

    /// Optionally prepares a table of errors.
    ///
    /// This function is invoked by `GooseMetrics::print()` and
    /// `GooseMetrics::print_running()`.
    pub(crate) fn fmt_errors(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Only include errors when displaying the final metrics report, and if there are
        // errors to display.
        if !self.final_metrics || self.errors.is_empty() {
            return Ok(());
        }

        // Write the errors into a vector which can then be sorted by occurrences.
        let mut errors: Vec<(usize, String)> = Vec::new();
        for error in self.errors.values() {
            errors.push((
                error.occurrences,
                format!("{} {}: {}", error.method_label(), error.name, error.error),
            ));
        }

        writeln!(
            fmt,
            "\n === ERRORS ===\n ------------------------------------------------------------------------------"
        )?;
        writeln!(fmt, " {:<11} | Error", "Count")?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;

        // Reverse sort errors to display the error occuring the most first.
        for (occurrences, error) in errors.iter().sorted().rev() {
            writeln!(fmt, " {:<12}  {}", format_number(*occurrences), error)?;
        }

        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;

        Ok(())
    }

    /// Optionally prepares a table of the thresholds and their results.
    ///
    /// This function is invoked by `GooseMetrics::print()`.
    pub(crate) fn fmt_thresholds(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Thresholds are judged only when the run ends, so there are results only in
        // the final metrics.
        if self.thresholds.is_empty() {
            return Ok(());
        }

        writeln!(
            fmt,
            "\n === THRESHOLDS ===\n ------------------------------------------------------------------------------"
        )?;
        // The Observed column widens for a long value, such as the two values of
        // the rule that stopped the run.
        let width = self
            .thresholds
            .iter()
            .map(|result| result.observed.chars().count())
            .max()
            .unwrap_or(0)
            .max(14);
        writeln!(fmt, " {:<44} | {:>width$} | Result", "Rule", "Observed")?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        for result in &self.thresholds {
            writeln!(
                fmt,
                " {:<44} | {:>width$} | {}",
                util::truncate_string(&result.rule, 44),
                result.observed,
                result.verdict(),
            )?;
        }
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;

        Ok(())
    }

    // Determine the seconds, minutes and hours between two chrono:DateTimes.
    pub(crate) fn get_seconds_minutes_hours(
        &self,
        start: &chrono::DateTime<chrono::Utc>,
        end: &chrono::DateTime<chrono::Utc>,
    ) -> (i64, i64, i64) {
        let duration = end.timestamp() - start.timestamp();
        let seconds = duration % 60;
        let minutes = (duration / 60) % 60;
        let hours = duration / 60 / 60;
        (seconds, minutes, hours)
    }

    /// Optionally prepares an overview table.
    ///
    /// This function is invoked by [`GooseMetrics::print()`].
    pub(crate) fn fmt_overview(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Only display overview in the final metrics.
        if !self.final_metrics || self.history.is_empty() {
            return Ok(());
        }

        writeln!(
            fmt,
            "\n === OVERVIEW ===\n ------------------------------------------------------------------------------"
        )?;

        writeln!(
            fmt,
            " {:<12} {:<21} {:<19} {:<10} Users",
            "Action", "Started", "Stopped", "Elapsed",
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;

        // Step through history looking at current step and next step.
        for step in self.history.windows(2) {
            let (seconds, minutes, hours) =
                self.get_seconds_minutes_hours(&step[0].timestamp, &step[1].timestamp);
            let started = Local
                .timestamp_opt(step[0].timestamp.timestamp(), 0)
                // @TODO: error handling
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            let stopped = Local
                .timestamp_opt(step[1].timestamp.timestamp(), 0)
                // @TODO: error handling
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            match &step[0].action {
                // For maintaining just show the current number of users.
                TestPlanStepAction::Maintaining => {
                    writeln!(
                        fmt,
                        " {:<12} {} - {} ({:02}:{:02}:{:02}, {})",
                        format!("{:?}:", step[0].action),
                        started,
                        stopped,
                        hours,
                        minutes,
                        seconds,
                        step[0].users,
                    )?;
                }
                // For increasing show the current number of users to the new number of users.
                TestPlanStepAction::Increasing => {
                    writeln!(
                        fmt,
                        " {:<12} {} - {} ({:02}:{:02}:{:02}, {} -> {})",
                        format!("{:?}:", step[0].action),
                        started,
                        stopped,
                        hours,
                        minutes,
                        seconds,
                        step[0].users,
                        step[1].users,
                    )?;
                }
                // For decreasing show the new number of users from the current number of users.
                TestPlanStepAction::Decreasing | TestPlanStepAction::Canceling => {
                    writeln!(
                        fmt,
                        " {:<12} {} - {} ({:02}:{:02}:{:02}, {} <- {})",
                        format!("{:?}:", step[0].action),
                        started,
                        stopped,
                        hours,
                        minutes,
                        seconds,
                        step[1].users,
                        step[0].users,
                    )?;
                }
                TestPlanStepAction::Finished => {
                    unreachable!("there shouldn't be a step after finished");
                }
            }
        }

        match self.hosts.len() {
            0 => {
                // A host is required to run a load test.
                writeln!(fmt, "\n Target host: undefined")?;
            }
            1 => {
                for host in &self.hosts {
                    writeln!(fmt, "\n Target host: {host}")?;
                }
            }
            _ => {
                writeln!(fmt, "\n Target hosts: ")?;
                for host in &self.hosts {
                    writeln!(fmt, " - {host}",)?;
                }
            }
        }
        writeln!(
            fmt,
            " {} v{}",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
        )?;

        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;

        Ok(())
    }
}

/// Implement format trait to allow displaying metrics.
impl fmt::Display for GooseMetrics {
    // Implement display of metrics with `{}` marker.
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        // Formats metrics data in tables, depending on what data is contained and which
        // flags are set.
        self.fmt_scenarios(fmt)?;
        self.fmt_scenario_times(fmt)?;
        self.fmt_transactions(fmt)?;
        self.fmt_transaction_times(fmt)?;
        self.fmt_requests(fmt)?;
        self.fmt_response_times(fmt)?;
        self.fmt_percentiles(fmt)?;
        self.fmt_status_codes(fmt)?;
        self.fmt_errors(fmt)?;
        self.fmt_thresholds(fmt)?;

        // Display enhanced coordinated omission metrics if available (before overview)
        if let Some(co_metrics) = &self.coordinated_omission_metrics {
            write!(fmt, "{co_metrics}")?;
        }

        self.fmt_overview(fmt)?;

        Ok(())
    }
}

/// For tracking and counting requests made during a load test.
///
/// The request that Goose is making. User threads send this data to the parent thread
/// when metrics are enabled. This request object must be provided to calls to
/// [`set_success`](../goose/struct.GooseUser.html#method.set_success) or
/// [`set_failure`](../goose/struct.GooseUser.html#method.set_failure) so Goose knows
/// which request is being updated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GooseErrorMetric {
    /// How many milliseconds the load test has been running.
    pub elapsed: u64,
    /// The raw request that the GooseClient made.
    pub raw: GooseRawRequest,
    /// The optional name of the request.
    pub name: String,
    /// The final full URL that was requested, after redirects.
    pub final_url: String,
    /// Whether or not the request was redirected.
    pub redirected: bool,
    /// How many milliseconds the request took.
    pub response_time: u64,
    /// The HTTP response code (optional).
    pub status_code: u16,
    /// Which GooseUser thread processed the request.
    pub user: usize,
    /// The error caused by this request.
    pub error: String,
}

/// For tracking and counting errors detected during a load test.
///
/// When a load test completes, by default it will include a summary of all errors that
/// were detected during the load test. Multiple errors that share the same request method,
/// the same request name, and the same error text are contained within a single
/// GooseErrorMetric object, with `occurrences` indicating how many times this error was
/// seen.
///
/// Individual `GooseErrorMetric`s are stored within a
/// [`GooseErrorMetrics`](./type.GooseErrorMetrics.html) `BTreeMap` with a string key of
/// `error.method.name`. The `BTreeMap` is found in the `errors` field of what is returned
/// by [`GooseAttack::execute()`](../struct.GooseAttack.html#method.execute) when a load
/// test finishes.
///
/// Can be disabled with the `--no-error-summary` run-time option, or with
/// [GooseDefault::NoErrorSummary](../config/enum.GooseDefault.html#variant.NoErrorSummary).
///
/// # Example
/// In this example, requests to load the front page are failing:
/// ```text
/// GooseErrorMetric {
///     method: Get,
///     name: "(Anon) front page",
///     error: "503 Service Unavailable: /",
///     occurrences: 4588,
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct GooseErrorMetricAggregate {
    /// The method that resulted in an error.
    pub method: GooseMethod,
    /// The optional name of the request.
    pub name: String,
    /// The error string.
    pub error: String,
    /// A counter reflecting how many times this error occurred.
    pub occurrences: usize,
    /// User-defined method label for non-HTTP protocols.
    #[serde(default)]
    pub custom_method: String,
}
impl GooseErrorMetricAggregate {
    pub(crate) fn new(
        method: GooseMethod,
        name: String,
        error: String,
        custom_method: &str,
    ) -> Self {
        GooseErrorMetricAggregate {
            method,
            name,
            error,
            occurrences: 0,
            custom_method: custom_method.to_string(),
        }
    }

    /// Returns the display label for this error's method.
    pub fn method_label(&self) -> &str {
        if !self.custom_method.is_empty() {
            &self.custom_method
        } else {
            http_method_str(self.method)
        }
    }
}

impl MetricsProcessor {
    /// Create a new MetricsProcessor.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        metrics_rx: flume::Receiver<GooseMetric>,
        cmd_rx: flume::Receiver<MetricsCommand>,
        metrics: GooseMetrics,
        graph_data: GraphData,
        configuration: GooseConfiguration,
        scenarios: Vec<Scenario>,
        defaults: GooseDefaults,
        request_counter_registry: GooseRequestCounterRegistry,
        logger_tx: GooseLoggerTx,
        metrics_epoch: MetricsEpoch,
        keep_full_graph: bool,
    ) -> Self {
        MetricsProcessor {
            metrics_rx,
            cmd_rx,
            metrics,
            graph_data,
            configuration,
            scenarios,
            defaults,
            request_counter_registry,
            logger_tx,
            metrics_epoch,
            keep_full_graph,
        }
    }

    /// Main task loop. Processes metrics from user threads and handles commands
    /// from the main loop using `tokio::select!`.
    pub(crate) async fn run(mut self) {
        let mut metrics_open = true;

        loop {
            if !metrics_open {
                // Metrics channel closed; only wait for commands (e.g. Shutdown).
                match self.cmd_rx.recv_async().await {
                    Ok(cmd) => {
                        if self.handle_command(cmd) {
                            return;
                        }
                    }
                    Err(_) => return,
                }
                continue;
            }

            // Commands first: a waiting command is answered after at most one
            // capped turn of metrics. Commands are sparse, so they cannot
            // starve metrics.
            tokio::select! {
                biased;
                cmd = self.cmd_rx.recv_async() => {
                    match cmd {
                        Ok(cmd) => {
                            if self.handle_command(cmd) {
                                return;
                            }
                        }
                        Err(_) => return,
                    }
                }
                msg = self.metrics_rx.recv_async() => {
                    match msg {
                        Ok(metric) => {
                            self.process_metric(metric);
                            // Batch-drain pending metrics for throughput, but
                            // never so many that a command waits behind them.
                            self.drain_up_to(METRICS_DRAIN_LIMIT);
                        }
                        Err(_) => {
                            metrics_open = false;
                        }
                    }
                }
            }
        }
    }

    /// Dispatch a single GooseMetric to the appropriate processing method.
    fn process_metric(&mut self, metric: GooseMetric) {
        match metric {
            GooseMetric::Request(request_metric) => {
                self.process_request_metric(&request_metric);
            }
            GooseMetric::Transaction(raw_transaction) => {
                self.process_transaction_metric(&raw_transaction);
            }
            GooseMetric::Scenario(raw_scenario) => {
                self.process_scenario_metric(&raw_scenario);
            }
            GooseMetric::All {
                request,
                transaction,
                scenario,
            } => {
                self.process_request_metric(&request);
                self.process_transaction_metric(&transaction);
                self.process_scenario_metric(&scenario);
            }
            GooseMetric::Batch(batch) => {
                self.process_batch(*batch);
            }
        }
    }

    /// Drain all pending metrics from the channel without blocking.
    ///
    /// Shutdown, Reset, GetMetrics and DisplayRunning need every metric sent
    /// before the command counted, so they drain everything; the dashboard
    /// commands use [`Self::drain_up_to`], since a busy run would never let
    /// this return.
    fn drain_pending(&mut self) {
        while let Ok(metric) = self.metrics_rx.try_recv() {
            self.process_metric(metric);
        }
    }

    /// Process at most `limit` pending metrics messages without blocking.
    fn drain_up_to(&mut self, limit: usize) {
        for _ in 0..limit {
            match self.metrics_rx.try_recv() {
                Ok(metric) => self.process_metric(metric),
                Err(_) => break,
            }
        }
    }

    /// Handle a command from the main loop. Returns `true` if shutdown was processed.
    fn handle_command(&mut self, cmd: MetricsCommand) -> bool {
        match cmd {
            MetricsCommand::DisplayRunning {
                duration,
                total_users,
                maximum_users,
            } => {
                self.drain_pending();
                self.sync_atomic_counters();
                self.metrics.duration = duration;
                self.metrics.total_users = total_users;
                self.metrics.maximum_users = maximum_users;
                self.metrics.print_running();
                false
            }
            MetricsCommand::CheckThresholds { duration, seconds } => {
                // Once the killswitch is triggered, by a threshold or anything
                // else, the run is stopping and there is nothing more to judge.
                if self.metrics.threshold_abort.is_none() && !crate::is_killswitch_triggered() {
                    // Capped, as for a dashboard snapshot: draining until empty
                    // could take as long as a busy run keeps sending.
                    self.drain_up_to(METRICS_DRAIN_LIMIT);
                    self.sync_atomic_counters();
                    self.metrics.duration = duration;
                    if let Some(failure) = threshold::first_failure(
                        &self.configuration.thresholds,
                        &self.metrics,
                        seconds,
                    ) {
                        crate::trigger_killswitch(&failure.reason);
                        // Only the first reason is kept: the rule stopped the run
                        // only if nothing else triggered the killswitch first.
                        if crate::killswitch_reason().as_deref() == Some(failure.reason.as_str()) {
                            self.metrics.threshold_abort = Some((failure.index, failure.observed));
                        }
                    }
                }
                false
            }
            MetricsCommand::EnterMaintain { seconds } => {
                let mark = self.threshold_mark(seconds);
                self.metrics.rps_window.enter(mark);
                false
            }
            MetricsCommand::LeaveMaintain { seconds } => {
                let mark = self.threshold_mark(seconds);
                self.metrics.rps_window.leave(mark);
                false
            }
            MetricsCommand::RecordUsers {
                active_users,
                elapsed_secs,
            } => {
                self.graph_data
                    .record_users_per_second(active_users, elapsed_secs);
                // Bound per-second history on long dashboard-only soaks.
                self.maybe_prune_dashboard_graph();
                false
            }
            MetricsCommand::Reset {
                duration,
                total_users,
                maximum_users,
                active_users,
                series_elapsed_secs,
                ack,
            } => {
                // Drain all pending metrics before reset.
                self.drain_pending();
                self.sync_atomic_counters();
                // Display pre-reset running metrics.
                self.metrics.duration = duration;
                self.metrics.total_users = total_users;
                self.metrics.maximum_users = maximum_users;
                self.metrics.print_running();
                // Reset all metrics state.
                self.metrics.requests = HashMap::new();
                self.metrics
                    .initialize_scenario_metrics(&self.scenarios, &self.configuration);
                self.metrics
                    .initialize_transaction_metrics(
                        &self.scenarios,
                        &self.configuration,
                        &self.defaults,
                    )
                    .expect("initialize_transaction_metrics failed during reset; defaults were validated at startup so this should not happen");
                // Reset atomic counters.
                {
                    let registry = self
                        .request_counter_registry
                        .lock()
                        .expect("request_counter_registry mutex poisoned");
                    for counters in registry.values() {
                        counters.reset();
                    }
                }
                // A reset restarts the run's clock and its counts, so what `rps`
                // rules count, as when a Controller changed the users before the
                // ramp up ended, starts again with them.
                self.metrics.rps_window.reset();
                // Increment the batch epoch so user-side batches accumulated
                // before the reset are discarded when they arrive.
                self.metrics_epoch.fetch_add(1, AtomicOrdering::Relaxed);
                // Clear rate series only. Active-users history (ramp + later
                // changes) is kept on a continuous series clock so the dashboard
                // still shows how load evolved over the full run.
                self.graph_data.reset_rate_series(series_elapsed_secs);
                // Ensure the users series reflects the live count at the last
                // known second (or second 0 if empty) so the chart does not dip
                // to zero when the next sample is slightly delayed.
                if active_users > 0 {
                    let second = self.graph_data.users_last_absolute_second().unwrap_or(0);
                    self.graph_data.record_users_monotonic(active_users, second);
                }
                let _ = ack.send(());
                false
            }
            MetricsCommand::GetMetrics {
                duration,
                total_users,
                maximum_users,
                history,
                respond,
            } => {
                self.drain_pending();
                self.sync_atomic_counters();
                let mut snapshot = self.metrics.clone();
                snapshot.duration = duration;
                snapshot.total_users = total_users;
                snapshot.maximum_users = maximum_users;
                snapshot.history = history;
                let _ = respond.send(snapshot);
                false
            }
            MetricsCommand::KeepGraph { keep, now } => {
                self.keep_full_graph = keep;
                self.graph_data.keep_latency_columns(keep, now);
                false
            }
            MetricsCommand::GetReportData {
                duration,
                total_users,
                maximum_users,
                history,
                respond,
            } => {
                self.drain_pending();
                self.sync_atomic_counters();
                let mut metrics = self.metrics.clone();
                metrics.duration = duration;
                metrics.total_users = total_users;
                metrics.maximum_users = maximum_users;
                metrics.history = history;
                let mut graph_data = self.graph_data.clone();
                // Fold every response time held into the copy's columns.
                graph_data.finish_latency_columns();
                let _ = respond.send((metrics, graph_data));
                false
            }
            MetricsCommand::Shutdown {
                duration,
                total_users,
                maximum_users,
                respond,
            } => {
                // Drain all remaining metrics from the channel.
                self.drain_pending();
                // Fold every response time still held into the report
                // heatmap's columns.
                self.graph_data.finish_latency_columns();
                self.sync_atomic_counters();
                self.metrics.duration = duration;
                self.metrics.total_users = total_users;
                self.metrics.maximum_users = maximum_users;
                let metrics = std::mem::take(&mut self.metrics);
                let graph_data = std::mem::replace(&mut self.graph_data, GraphData::new());
                let _ = respond.send((metrics, graph_data));
                true
            }
            MetricsCommand::GetDashboardSnapshot {
                duration,
                total_users,
                maximum_users,
                active_users,
                target_users,
                series_elapsed_secs,
                phase,
                stopping,
                series_window_secs,
                report,
                settings,
                heatmap_since,
                respond,
            } => {
                let started = std::time::Instant::now();
                // Capped: a snapshot is a live view, and draining until empty
                // could hold it past the hub's build timeout under load.
                self.drain_up_to(METRICS_DRAIN_LIMIT);
                self.sync_atomic_counters();
                self.metrics.duration = duration;
                self.metrics.total_users = total_users;
                self.metrics.maximum_users = maximum_users;
                // Pin the live user count at the continuous series clock "now".
                // Monotonic-only: never rewrite past samples (avoids mid-ramp
                // V-glitches from gap-fill + out-of-order overwrites). Extends
                // the right edge so the chart matches the KPI active_users.
                self.graph_data
                    .record_users_monotonic(active_users, series_elapsed_secs);
                let series = self.graph_data.export_series_window_with_heatmap(
                    series_window_secs,
                    Some(series_elapsed_secs),
                    self.co_active(),
                    heatmap_since,
                );
                let mut snapshot = dashboard_snapshot::build_dashboard_snapshot(
                    dashboard_snapshot::DashboardSnapshotInput {
                        metrics: &self.metrics,
                        series,
                        active_users,
                        maximum_users,
                        target_users,
                        total_users,
                        phase,
                        stopping,
                        series_window_secs,
                        no_status_codes: self.configuration.no_status_codes,
                        metrics_disabled: self.configuration.no_metrics,
                        no_transaction_metrics: self.configuration.no_transaction_metrics,
                        no_scenario_metrics: self.configuration.no_scenario_metrics,
                        report,
                        settings: *settings,
                        percentiles_over_time: self.configuration.records_latency_series(),
                        series_keys: self.graph_data.latency_series_keys(),
                    },
                );
                snapshot.build_time = started.elapsed();
                let _ = respond.send(snapshot);
                // After export, drop series older than the dashboard window so
                // memory stays bounded when --report-file is not also set.
                self.maybe_prune_dashboard_graph();
                false
            }
            MetricsCommand::GetRequestSeries {
                method,
                name,
                series_elapsed_secs,
                since,
                respond,
            } => {
                // Capped, as for GetDashboardSnapshot.
                self.drain_up_to(METRICS_DRAIN_LIMIT);
                let series = self.graph_data.export_request_series(
                    method,
                    name,
                    dashboard_snapshot::SERIES_WINDOW_SECS,
                    Some(series_elapsed_secs),
                    self.co_active(),
                    since,
                );
                let _ = respond.send(series);
                false
            }
        }
    }

    /// [`Self::process_request_metric`], for unit tests in other modules.
    #[cfg(test)]
    pub(crate) fn process_request_metric_for_test(&mut self, request_metric: &GooseRequestMetric) {
        self.process_request_metric(request_metric);
    }

    /// The processor's graph data, for unit tests in other modules.
    #[cfg(test)]
    pub(crate) fn graph_data_for_test(&mut self) -> &mut GraphData {
        &mut self.graph_data
    }

    /// Whether coordinated omission mitigation has recorded an event, so the
    /// dashboard shows adjusted values.
    fn co_active(&self) -> bool {
        self.metrics
            .coordinated_omission_metrics
            .as_ref()
            .is_some_and(|m| m.has_events())
    }

    /// Prune GraphData to a trailing window when only the live dashboard needs series.
    ///
    /// The HTML and Markdown reports graph the whole run, so nothing is pruned while a
    /// report will be written at the end of it. Dashboard export uses at most [`dashboard_snapshot::SERIES_WINDOW_SECS`];
    /// we keep 2× that as a small buffer against edge flicker at the window boundary.
    ///
    /// Call only from low-frequency paths (`RecordUsers`, `GetDashboardSnapshot`),
    /// not from per-request/transaction/scenario handlers.
    fn maybe_prune_dashboard_graph(&mut self) {
        if self.configuration.dashboard && !self.keep_full_graph {
            let keep = dashboard_snapshot::SERIES_WINDOW_SECS.saturating_mul(2);
            self.graph_data.prune_to_window(keep);
        }
    }

    /// Increment the atomic counters in the shared registry.
    ///
    /// Used for Coordinated Omission synthetic metrics which are generated on the
    /// processor side and therefore not counted by user threads.
    fn increment_registry_counter(
        registry: &GooseRequestCounterRegistry,
        key: &str,
        success: bool,
    ) {
        let counter = {
            let mut map = registry
                .lock()
                .expect("request_counter_registry mutex poisoned");
            map.entry(key.to_string())
                .or_insert_with(|| Arc::new(GooseRequestCounters::new()))
                .clone()
        };
        if success {
            counter.success.fetch_add(1, AtomicOrdering::Relaxed);
        } else {
            counter.failure.fetch_add(1, AtomicOrdering::Relaxed);
        }
    }

    /// Copy the atomic counter values from the shared registry into the
    /// `GooseRequestMetricAggregate` structs.
    fn sync_atomic_counters(&mut self) {
        let registry = self
            .request_counter_registry
            .lock()
            .expect("request_counter_registry mutex poisoned");
        for (key, counters) in registry.iter() {
            if let Some(aggregate) = self.metrics.requests.get_mut(key) {
                aggregate.success_count = counters.success.load(AtomicOrdering::Relaxed) as usize;
                aggregate.fail_count = counters.failure.load(AtomicOrdering::Relaxed) as usize;
            }
            // The aggregate may not exist yet if a user thread incremented an atomic
            // counter but the corresponding channel message hasn't been processed by
            // record_request_metric() yet. This is benign: atomic values are cumulative
            // and will be picked up on the next sync once the aggregate is created.
            // The Shutdown handler always calls drain_pending() before sync_atomic_counters(),
            // so the final report is always accurate.
        }
    }

    /// The moment `seconds` into the run, with each request's count from its atomic
    /// counters, which hold every request made even before its first metric is
    /// drained. The capped drain first keeps the aggregates close to them.
    fn threshold_mark(&mut self, seconds: f64) -> threshold::ThresholdMark {
        self.drain_up_to(METRICS_DRAIN_LIMIT);
        self.sync_atomic_counters();
        let registry = self
            .request_counter_registry
            .lock()
            .expect("request_counter_registry mutex poisoned");
        let requests = registry
            .iter()
            .map(|(key, counters)| {
                let made = counters.success.load(AtomicOrdering::Relaxed)
                    + counters.failure.load(AtomicOrdering::Relaxed);
                (key.clone(), made as usize)
            })
            .collect();
        threshold::ThresholdMark::new(seconds, requests)
    }

    /// Store `GooseRequestMetric` in a `GooseRequestMetricAggregate`, merging if
    /// already existing, or creating new.
    fn record_request_metric(&mut self, request_metric: &GooseRequestMetric) {
        let method_label = request_metric.raw.method_label();
        let key = format!("{} {}", method_label, request_metric.name);
        // Updated in place: a copy of the aggregate, with every microsecond
        // bucket, for each request recorded here was most of its cost.
        let merge_request = self.metrics.requests.entry(key).or_insert_with(|| {
            GooseRequestMetricAggregate::new(
                &request_metric.name,
                request_metric.raw.method,
                0,
                &request_metric.raw.custom_method,
            )
        });

        if !request_metric.update {
            merge_request.record_time_us(
                request_metric.response_time_micros(),
                request_metric.coordinated_omission_elapsed > 0,
            );
            if !self.configuration.no_status_codes {
                merge_request.set_status_code(request_metric.status_code);
                if request_metric.coordinated_omission_elapsed == 0 {
                    merge_request.record_status_code_time_us(
                        request_metric.status_code,
                        request_metric.response_time_micros(),
                    );
                }
            }
        }
    }

    /// [`Self::process_batch`], for unit tests in other modules.
    #[cfg(test)]
    pub(crate) fn process_batch_for_test(&mut self, batch: GooseMetricBatch) {
        self.process_batch(batch);
    }

    /// The processor's aggregated metrics, for unit tests in other modules.
    #[cfg(test)]
    pub(crate) fn metrics_for_test(&self) -> &GooseMetrics {
        &self.metrics
    }

    /// [`Self::sync_atomic_counters`], for unit tests in other modules.
    #[cfg(test)]
    pub(crate) fn sync_atomic_counters_for_test(&mut self) {
        self.sync_atomic_counters();
    }

    /// Process a single request metric: record errors, handle CO backfill, update
    /// aggregates and graph data.
    fn process_request_metric(&mut self, request_metric: &GooseRequestMetric) {
        if !request_metric.error.is_empty() {
            self.record_error(request_metric);
        }

        if request_metric.coordinated_omission_elapsed > 0 && request_metric.user_cadence > 0 {
            if let Some(co_metrics) = &mut self.metrics.coordinated_omission_metrics {
                let synthetic_count = (request_metric.coordinated_omission_elapsed as i64
                    - request_metric.user_cadence as i64
                    - request_metric.response_time as i64)
                    / request_metric.user_cadence as i64;

                if synthetic_count > 0 {
                    co_metrics.record_co_event(
                        std::time::Duration::from_millis(request_metric.user_cadence),
                        std::time::Duration::from_millis(
                            request_metric.coordinated_omission_elapsed,
                        ),
                        synthetic_count as u32,
                        request_metric.user,
                        request_metric.scenario_name.to_string(),
                    );
                }
            }

            let mut co_metric = request_metric.clone();
            let mut response_time = request_metric.coordinated_omission_elapsed as i64
                - request_metric.user_cadence as i64
                - request_metric.response_time as i64;

            // Synthetic times also go to the dashboard's heatmap, each in
            // the second its synthetic request would have started. This copy
            // was sent because the previous scenario iteration took
            // `coordinated_omission_elapsed` (E) ms, more than twice the
            // cadence (c). That iteration is the stall, and it ended at or
            // just before this request started, at `elapsed` (s) ms since the
            // start. The loop below records t_k = E - k*c - r for k = 1, 2, ...
            // while t_k > r, where r is this request's own response time; a
            // request that took t_k and ended at s started at s - t_k. This
            // is exact for the first request of an iteration and slightly late
            // for later ones, and with wait times the seconds compress toward
            // the end of the stall.
            let latency_key = self.graph_data.records_synthetic_latency().then(|| {
                format!(
                    "{} {}",
                    request_metric.raw.method_label(),
                    request_metric.name
                )
            });

            loop {
                if response_time > request_metric.response_time as i64 {
                    // Synthetic times are whole milliseconds, as the
                    // cadence they come from.
                    co_metric.response_time = response_time as u64;
                    co_metric.response_time_fraction_us = 0;
                    self.record_request_metric(&co_metric);
                    if let Some(key) = latency_key.as_deref() {
                        let second = (request_metric
                            .elapsed
                            .saturating_sub(co_metric.response_time)
                            / 1000) as usize;
                        self.graph_data.record_synthetic_latency(
                            key,
                            second,
                            co_metric.response_time,
                        );
                    }
                    let co_key = format!("{} {}", co_metric.raw.method, co_metric.name);
                    Self::increment_registry_counter(
                        &self.request_counter_registry,
                        &co_key,
                        co_metric.success,
                    );
                    response_time -= request_metric.user_cadence as i64;
                } else {
                    break;
                }
            }
        } else {
            if let Some(co_metrics) = &mut self.metrics.coordinated_omission_metrics {
                co_metrics.record_actual_request();
            }

            self.record_request_metric(request_metric);

            if self.configuration.records_graph_data() {
                let seconds_since_start = (request_metric.elapsed / 1000) as usize;

                let key = format!(
                    "{} {}",
                    request_metric.raw.method_label(),
                    request_metric.name
                );
                // An update from set_success() or set_failure() names a
                // request already counted in its second: only whether it
                // failed changes.
                if !request_metric.update {
                    // Coordinated omission copies are not new measurements,
                    // so they stay out of the heatmap.
                    let latency = (request_metric.coordinated_omission_elapsed == 0)
                        .then_some(request_metric.response_time_micros());
                    self.graph_data.record_request_per_second_and_latency_us(
                        &key,
                        seconds_since_start,
                        latency,
                    );
                    self.graph_data.record_average_response_time_per_second_us(
                        key.clone(),
                        seconds_since_start,
                        request_metric.response_time_micros(),
                    );
                }

                if !request_metric.success {
                    self.graph_data
                        .record_errors_per_second(&key, seconds_since_start);
                } else if request_metric.update {
                    self.graph_data
                        .remove_error_per_second(&key, seconds_since_start);
                }
                // Do not prune here: per-request pruning is O(unique keys) and
                // would dominate at high RPS. Pruning runs from low-frequency
                // RecordUsers (~1 Hz) and GetDashboardSnapshot instead.
            }
        }
    }

    /// Process a single transaction metric: update aggregate and graph data.
    fn process_transaction_metric(&mut self, raw_transaction: &TransactionMetric) {
        self.metrics.transactions[raw_transaction.scenario_index]
            [raw_transaction.transaction_index]
            .set_time_us(raw_transaction.run_time_micros(), raw_transaction.success);

        if self.configuration.records_graph_data() {
            self.graph_data
                .record_transactions_per_second((raw_transaction.elapsed / 1000) as usize);
        }
    }

    /// Process a single scenario metric: update aggregate and graph data.
    fn process_scenario_metric(&mut self, raw_scenario: &ScenarioMetric) {
        self.metrics.scenarios[raw_scenario.index]
            .update_us(raw_scenario.run_time_micros(), raw_scenario.user);

        if self.configuration.records_graph_data() {
            self.graph_data
                .record_scenarios_per_second((raw_scenario.elapsed / 1000) as usize);
        }
    }

    /// Update error metrics and optionally send to the error logger.
    fn record_error(&mut self, raw_request: &GooseRequestMetric) {
        if self.configuration.error_log().is_some() {
            if let Some(logger) = self.logger_tx.as_ref() {
                if let Err(e) = logger.send(Some(GooseLog::Error(GooseErrorMetric {
                    elapsed: raw_request.elapsed,
                    raw: raw_request.raw.clone(),
                    name: raw_request.name.to_string(),
                    final_url: raw_request.final_url.clone(),
                    redirected: raw_request.redirected,
                    response_time: raw_request.response_time,
                    status_code: raw_request.status_code,
                    user: raw_request.user,
                    error: raw_request.error.clone(),
                }))) {
                    if let flume::SendError(Some(ref message)) = e {
                        info!("Failed to write to error log (receiver dropped?): flume::SendError({message:?})");
                    } else {
                        info!("Failed to write to error log: (receiver dropped?) {e:?}");
                    }
                }
            }
        }

        if self.configuration.no_error_summary {
            return;
        }

        let method_label = raw_request.raw.method_label();
        let error_string = format!(
            "{}.{}.{}",
            raw_request.error, method_label, raw_request.name
        );

        let mut error_metrics = match self.metrics.errors.get(&error_string) {
            Some(m) => m.clone(),
            None => GooseErrorMetricAggregate::new(
                raw_request.raw.method,
                raw_request.name.to_string(),
                raw_request.error.to_string(),
                &raw_request.raw.custom_method,
            ),
        };
        error_metrics.occurrences += 1;
        self.metrics.errors.insert(error_string, error_metrics);
    }

    /// Process a pre-aggregated batch of metrics from a single GooseUser thread.
    ///
    /// Individual request metrics (failed / CO-affected) are processed one by one
    /// using the normal per-request path. Pre-aggregated entries are merged directly
    /// into the global aggregates, avoiding the per-request overhead.
    /// Merge a batched request's coordinated omission and status code
    /// figures into its whole run `aggregate`; its timing data is merged by
    /// the caller.
    fn merge_batch_entry(
        no_status_codes: bool,
        aggregate: &mut GooseRequestMetricAggregate,
        entry: &RequestBatchEntry,
    ) {
        // Once a request has coordinated omission adjusted times they must
        // include every later real time too, batched ones included, or the
        // adjusted values drift toward the synthetic times (#739).
        if let Some(co_data) = aggregate.coordinated_omission_data.as_mut() {
            co_data.merge(&entry.raw_data);
        }

        // Merge status-code counts.
        if !no_status_codes {
            for (code, count) in &entry.status_code_counts {
                *aggregate.status_code_counts.entry(*code).or_insert(0) += count;
            }

            // Merge status-code timing summaries.
            for (code, summary) in &entry.status_code_timings {
                aggregate
                    .status_code_timings
                    .entry(*code)
                    .or_default()
                    .merge(summary);
            }
        }
    }

    fn process_batch(&mut self, batch: GooseMetricBatch) {
        // Discard stale batches from before a metrics reset.
        let current_epoch = self.metrics_epoch.load(AtomicOrdering::Relaxed);
        if batch.epoch < current_epoch {
            return;
        }

        // 1. Process individual request metrics (errors, CO) normally.
        for request_metric in &batch.individual_requests {
            self.process_request_metric(request_metric);
        }

        // With response times kept by second, step 2 merges each request's
        // timing data into the whole run's as well.
        let latency_merged = self.configuration.records_graph_data()
            && batch.second.is_some()
            && self.graph_data.records_latency();

        // 2. Apply graph data if a report file is configured or the dashboard
        //    is on, before step 3 consumes the request entries.
        if self.configuration.records_graph_data() {
            // A request's `raw_data` in the batch holds its requests that
            // started in the batch's second, and the batch's `next_second`
            // those that started in the second after.
            if let (true, Some(second)) = (latency_merged, batch.second) {
                // The batch holds this second and the next.
                self.graph_data.fit_latency_columns(second + 1);
                // Each request's whole run timing data and its pending counts
                // in one walk of the batch's buckets, with the rest of the
                // request's whole run figures, so step 3 has nothing left
                // to merge for it.
                for (key, entry) in &batch.requests {
                    let aggregate = match self.metrics.requests.get_mut(key) {
                        Some(aggregate) => aggregate,
                        None => self.metrics.requests.entry(key.clone()).or_insert_with(|| {
                            GooseRequestMetricAggregate::new(
                                &entry.path,
                                entry.method,
                                0,
                                &entry.custom_method,
                            )
                        }),
                    };
                    self.graph_data.record_requests_per_second_merging_latency(
                        key,
                        second,
                        &entry.raw_data,
                        &mut aggregate.raw_data,
                    );
                    Self::merge_batch_entry(self.configuration.no_status_codes, aggregate, entry);
                    self.graph_data
                        .record_average_response_time_per_second_batch(
                            key,
                            second,
                            entry.raw_data.total_time_us as f64 / 1000.0,
                            entry.raw_data.counter as u32,
                        );
                }
                // Times from the second after the batch's: every key here
                // also has an entry in `requests`, merged above.
                for (key, next) in &batch.next_second {
                    if let Some(aggregate) = self.metrics.requests.get_mut(key) {
                        self.graph_data.record_requests_per_second_merging_latency(
                            key,
                            second + 1,
                            next,
                            &mut aggregate.raw_data,
                        );
                        if let Some(co_data) = aggregate.coordinated_omission_data.as_mut() {
                            co_data.merge(next);
                        }
                        self.graph_data
                            .record_average_response_time_per_second_batch(
                                key,
                                second + 1,
                                next.total_time_us as f64 / 1000.0,
                                next.counter as u32,
                            );
                    }
                }
            } else if let Some(second) = batch.second {
                let seconds = batch
                    .requests
                    .iter()
                    .map(|(key, entry)| (key, second, &entry.raw_data))
                    .chain(
                        batch
                            .next_second
                            .iter()
                            .map(|(key, data)| (key, second + 1, data)),
                    );
                for (key, second, data) in seconds {
                    self.graph_data
                        .record_requests_per_second_and_latency_batch(
                            key,
                            second,
                            data.counter as u32,
                            None,
                        );
                    self.graph_data
                        .record_average_response_time_per_second_batch(
                            key,
                            second,
                            data.total_time_us as f64 / 1000.0,
                            data.counter as u32,
                        );
                }
            }
            // Without the heatmap, the batch counts its requests by second
            // as it always did.
            for ((key, second), entry) in &batch.graph_request_data {
                self.graph_data
                    .record_requests_per_second_and_latency_batch(key, *second, entry.count, None);
                if entry.count > 0 {
                    self.graph_data
                        .record_average_response_time_per_second_batch(
                            key,
                            *second,
                            entry.total_time,
                            entry.count,
                        );
                }
            }
            for (&second, &count) in &batch.graph_tps {
                self.graph_data
                    .record_transactions_per_second_batch(second, count as usize);
            }
            for (&second, &count) in &batch.graph_sps {
                self.graph_data
                    .record_scenarios_per_second_batch(second, count as usize);
            }
        }

        // 3. Merge pre-aggregated request timing and status-code data.
        //    Pre-aggregated requests are all non-CO successful requests, so they
        //    count as actual requests for coordinated omission tracking.
        if let Some(co_metrics) = &mut self.metrics.coordinated_omission_metrics {
            let total_actual: u64 = batch
                .requests
                .values()
                .map(|e| e.raw_data.counter as u64)
                .chain(batch.next_second.values().map(|next| next.counter as u64))
                .sum();
            co_metrics.actual_requests += total_actual;
            co_metrics.update_synthetic_percentage();
        }
        if !latency_merged {
            for (key, entry) in batch.requests {
                let aggregate = self.metrics.requests.entry(key).or_insert_with(|| {
                    GooseRequestMetricAggregate::new(
                        &entry.path,
                        entry.method,
                        0,
                        &entry.custom_method,
                    )
                });

                // Merge timing data: combine BTreeMaps, update min/max/total/counter.
                aggregate.raw_data.merge(&entry.raw_data);
                Self::merge_batch_entry(self.configuration.no_status_codes, aggregate, &entry);
            }

            // Times from the second after the batch's: every key here also
            // has an entry in `requests`, merged above.
            for (key, next) in batch.next_second {
                if let Some(aggregate) = self.metrics.requests.get_mut(&key) {
                    aggregate.raw_data.merge(&next);
                    if let Some(co_data) = aggregate.coordinated_omission_data.as_mut() {
                        co_data.merge(&next);
                    }
                }
            }
        }

        // 4. Merge pre-aggregated transaction timing data.
        for ((si, ti), entry) in batch.transactions {
            let Some(scenario_txns) = self.metrics.transactions.get_mut(si) else {
                // Invalid scenario index (possible in Gaggle mode with mismatched configs).
                continue;
            };
            let Some(agg) = scenario_txns.get_mut(ti) else {
                // Invalid transaction index within this scenario.
                continue;
            };
            agg.run_times.merge(&entry.run_times);
            agg.success_count += entry.success_count;
            agg.fail_count += entry.fail_count;
        }

        // 5. Merge pre-aggregated scenario timing data.
        for (index, entry) in batch.scenarios {
            let Some(agg) = self.metrics.scenarios.get_mut(index) else {
                // Invalid scenario index (possible in Gaggle mode with mismatched configs).
                continue;
            };
            agg.users.insert(entry.user);
            agg.run_times.merge(&entry.run_times);
        }
    }
}

impl GooseAttack {
    /// Check if it's time to display running metrics, and if so, send a
    /// DisplayRunning command to the metrics processor (fire-and-forget).
    pub(crate) fn sync_metrics(&mut self, goose_attack_run_state: &mut GooseAttackRunState) {
        if !self.configuration.no_metrics {
            if let Some(running_metrics) = self.configuration.running_metrics {
                if util::timer_expired(
                    goose_attack_run_state.running_metrics_timer,
                    running_metrics,
                ) {
                    goose_attack_run_state.running_metrics_timer = std::time::Instant::now();
                    self.update_duration();
                    let _ = goose_attack_run_state.metrics_cmd_tx.send(
                        MetricsCommand::DisplayRunning {
                            duration: self.metrics.duration,
                            total_users: self.metrics.total_users,
                            maximum_users: self.metrics.maximum_users,
                        },
                    );
                }
            }
        }
    }

    /// When going from the Increasing phase to the Maintaining phase, optionally
    /// flush and reset metrics via the dedicated processor task.
    pub(crate) async fn reset_metrics(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        if !goose_attack_run_state.all_users_spawned {
            goose_attack_run_state.all_users_spawned = true;
            // Only reset metrics on startup if not using `--test-plan` or
            // `--iterations`. A users ramp always has users; with none, as
            // when a ramp setting reached a running test plan before
            // Controllers refused it, the run is treated as a test plan
            // rather than panicking.
            let users = match self.configuration.users {
                Some(users)
                    if self.configuration.test_plan.is_none()
                        && self.configuration.iterations() == 0 =>
                {
                    Some(users)
                }
                _ => None,
            };
            if let Some(users) = users {
                if !self.configuration.no_reset_metrics {
                    self.update_duration();

                    // Send Reset command to the processor: it will drain pending metrics,
                    // sync atomic counters, display running metrics, and reset all state.
                    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
                    goose_attack_run_state
                        .metrics_cmd_tx
                        .send(MetricsCommand::Reset {
                            duration: self.metrics.duration,
                            total_users: self.metrics.total_users,
                            maximum_users: self.metrics.maximum_users,
                            active_users: goose_attack_run_state.active_users,
                            series_elapsed_secs: self.series_elapsed_secs(),
                            ack: ack_tx,
                        })
                        .expect("metrics processor channel closed unexpectedly");
                    ack_rx.await.expect("metrics processor dropped ack sender");

                    // Reset running_metrics_timer.
                    goose_attack_run_state.running_metrics_timer = std::time::Instant::now();

                    if self.metrics.display_metrics {
                        if goose_attack_run_state.active_users < users {
                            println!(
                                "{} of {} users launched, timer expired, resetting metrics (disable with --no-reset-metrics).\n", goose_attack_run_state.active_users, users
                            );
                        } else {
                            println!(
                                "All {users} users launched, resetting metrics (disable with --no-reset-metrics).\n"
                            );
                        }
                    }

                    // Restart the timer now that all threads are launched.
                    self.started = Some(std::time::Instant::now());
                } else if goose_attack_run_state.active_users < users {
                    println!(
                        "{} of {} users launched, timer expired.\n",
                        goose_attack_run_state.active_users, users
                    );
                } else {
                    println!(
                        "All {} users launched.\n",
                        goose_attack_run_state.active_users
                    );
                }
            } else {
                println!("{} users launched.", goose_attack_run_state.active_users);
            }
        }

        Ok(())
    }

    // Update metrics showing how long the load test has been running.
    // 1.2 seconds will round down to 1 second. 1.6 seconds will round up to 2 seconds.
    pub(crate) fn update_duration(&mut self) {
        self.elapsed = self
            .started
            .map_or(0.0, |started| started.elapsed().as_secs_f64());
        self.metrics.duration = self.elapsed.round() as usize;
    }

    /// Process the `--report-file` reports.
    ///
    /// With a `source`, write each report from it. Without one, create the files, so a
    /// path Goose can't write to fails before the load test. `html` caches the HTML report so it is generated once
    /// however many reports name an HTML file.
    ///
    /// Creating stops at the first error. Writing tries every report, logs each
    /// failure, and returns the first.
    pub(crate) async fn process_reports(
        &self,
        reports: &[String],
        source: Option<&ReportSource<'_>>,
        baseline: Option<&ReportData<'static>>,
        html: &mut Option<String>,
    ) -> Result<(), GooseError> {
        let mut first_error = None;
        // Like the HTML report, a PDF report is rendered once for every name.
        let mut pdf = None;
        for report in reports {
            if let Err(e) = self
                .process_report(report, source, baseline, html, &mut pdf)
                .await
            {
                if source.is_none() {
                    return Err(e);
                }
                match &e {
                    GooseError::InvalidOption { value, detail, .. } => {
                        error!("couldn't write the report {report}: {value}. {detail}")
                    }
                    other => error!("couldn't write the report {report}: {other:?}"),
                }
                first_error.get_or_insert(e);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Create one report file, and with a `source`, write it. A name that ends in
    /// `.gz` is written gzipped, in the format the rest of the name gives.
    async fn process_report(
        &self,
        report: &str,
        source: Option<&ReportSource<'_>>,
        baseline: Option<&ReportData<'static>>,
        html: &mut Option<String>,
        pdf: &mut Option<Vec<u8>>,
    ) -> Result<(), GooseError> {
        let (inner, gzip) = crate::runs::without_gz(report);
        let kind = match std::path::Path::new(inner)
            .extension()
            .map(OsStr::to_string_lossy)
            .as_deref()
        {
            Some("html" | "htm") => "html",
            Some("json") => "json",
            Some("md") => "markdown",
            #[cfg(feature = "pdf-reports")]
            Some("pdf") => "pdf",
            #[cfg(not(feature = "pdf-reports"))]
            Some("pdf") => {
                return Err(GooseError::InvalidOption {
                    option: "--report-file".to_string(),
                    value: report.to_string(),
                    detail: "PDF reports require compiling with the 'pdf-reports' feature flag"
                        .to_string(),
                })
            }
            None => {
                return Err(GooseError::InvalidOption {
                    option: "--report-file".to_string(),
                    value: report.to_string(),
                    detail: "Missing file extension for report".to_string(),
                })
            }
            Some(ext) => {
                return Err(GooseError::InvalidOption {
                    option: "--report-file".to_string(),
                    value: report.to_string(),
                    detail: format!("Unknown report file type: {ext}"),
                })
            }
        };
        let path = PathBuf::from(report);
        let file = File::create(&path)
            .await
            .map_err(|err| report_io_error(report, err))?;
        let Some(source) = source else {
            return Ok(());
        };
        let mut out = ReportOut::new(
            file.into_std().await,
            gzip,
            self.configuration.compression_level(),
        );
        let written = match kind {
            "html" => {
                let content = match html {
                    Some(content) => content,
                    None => html.insert(source.html(baseline)?),
                };
                out.write_all(content.as_bytes())
                    .map_err(|err| report_io_error(report, err))
            }
            "json" => source.write_json(&mut out, baseline),
            "markdown" => source.write_markdown(&mut out, baseline),
            _ => {
                let bytes = match pdf {
                    Some(bytes) => bytes,
                    None => match self.pdf_report(source, baseline).await {
                        Ok(bytes) => pdf.insert(bytes),
                        Err(e) => {
                            // Dropping a gzip writer writes an empty member,
                            // so the file goes by name: Goose created it above.
                            drop(out);
                            let _ = tokio::fs::remove_file(&path).await;
                            return Err(e);
                        }
                    },
                };
                out.write_all(bytes)
                    .map_err(|err| report_io_error(report, err))
            }
        };
        // Finish the file, gzip trailer and all, here: an error on drop would be
        // lost.
        written.and_then(|()| out.finish().map_err(|err| report_io_error(report, err)))?;
        info!("{kind} report file written to: {report}");
        Ok(())
    }

    /// Create all requested reports, to ensure we have access. When any is a
    /// PDF, also find Chrome and confirm it renders one, so a Chrome problem
    /// stops the run before any user starts.
    ///
    /// The Chrome check runs first, so a run it stops leaves no report file behind.
    pub(crate) async fn create_reports(&self) -> Result<(), GooseError> {
        let reports = self.configuration.report_paths();
        #[cfg(feature = "pdf-reports")]
        if reports.iter().any(|report| {
            std::path::Path::new(crate::runs::without_gz(report).0)
                .extension()
                .is_some_and(|ext| ext == "pdf")
        }) {
            self.preflight_pdf().await?;
        }

        self.process_reports(&reports, None, None, &mut None).await
    }

    /// The Chrome binary PDF reports run, found (and if need be downloaded)
    /// the first time it is asked for.
    #[cfg(feature = "pdf-reports")]
    async fn pdf_chrome(&self) -> Result<&report::chrome::ResolvedChrome, GooseError> {
        self.chrome
            .get_or_try_init(report::chrome::resolve_chrome)
            .await
    }

    /// Finds Chrome and prints a small page to PDF with it.
    #[cfg(feature = "pdf-reports")]
    async fn preflight_pdf(&self) -> Result<(), GooseError> {
        let started = std::time::Instant::now();
        let chrome = self.pdf_chrome().await?.clone();
        let scale = self.configuration.pdf_scale_or_default();
        let verbose = self.configuration.verbose > 0;
        let timeout = self.configuration.pdf_timeout_or_default();
        let launched = chrome.clone();
        let version = tokio::task::spawn_blocking(move || {
            report::pdf::preflight_pdf(scale, verbose, timeout, &launched)
        })
        .await
        .map_err(|e| chrome_task_error("the check that Chrome can render a PDF", e))??;
        info!(
            "PDF reports will use Chrome {} at {} ({}); checked in {:.1}s",
            version
                .or(chrome.version)
                .as_deref()
                .unwrap_or("(version unknown)"),
            chrome.path.display(),
            chrome.source.describe(),
            started.elapsed().as_secs_f64()
        );
        Ok(())
    }

    /// Write all `--report-file` reports from `source`, and `--pdf-print-html` if set,
    /// even when a report failed. Returns the first error.
    pub(crate) async fn write_report_files(
        &self,
        source: &ReportSource<'_>,
        baseline: Option<&ReportData<'static>>,
        html: &mut Option<String>,
    ) -> Result<(), GooseError> {
        let reports = self
            .process_reports(
                &self.configuration.report_paths(),
                Some(source),
                baseline,
                html,
            )
            .await;

        // Write PDF-optimized HTML if configured (always available)
        let print_html = self.write_print_html_if_configured(source, baseline).await;

        first_error(reports, print_html)
    }

    /// Write PDF-optimized HTML if the --pdf-print-html option is configured.
    /// This functionality is always available regardless of feature flags.
    async fn write_print_html_if_configured(
        &self,
        source: &ReportSource<'_>,
        baseline: Option<&ReportData<'static>>,
    ) -> Result<(), GooseError> {
        if let Some(pdf_print_html) = self.configuration.pdf_print_html() {
            use crate::report::print::generate_print_optimized_html_content;
            use tokio::fs as async_fs;

            // Generate HTML report content
            let html_report = source.html(baseline)?;

            // Add print-optimized CSS using the shared function
            let print_optimized_html = generate_print_optimized_html_content(&html_report);

            // Write the PDF-optimized HTML to file
            if let Err(e) = async_fs::write(pdf_print_html, &print_optimized_html).await {
                return Err(GooseError::InvalidOption {
                    option: "--pdf-print-html".to_string(),
                    value: pdf_print_html.to_string(),
                    detail: format!("Failed to write PDF-optimized HTML: {e}"),
                });
            }

            info!("PDF-optimized HTML saved to: {pdf_print_html}");
        }

        Ok(())
    }

    /// The reports of the run in progress or just ended, from its metrics and
    /// graph data, recorded as `record` says.
    pub(crate) fn report_source<'a>(&'a self, record: &'a RunRecord) -> ReportSource<'a> {
        ReportSource {
            configuration: &self.configuration,
            metrics: &self.metrics,
            graph_data: &self.graph_data,
            graphs: true,
            heatmap: None,
            loaded: None,
            record,
            graph_data_in_json: self.output.graph_data,
        }
    }

    /// A PDF report, rendered by Chrome from the print ready HTML report.
    #[cfg(feature = "pdf-reports")]
    async fn pdf_report(
        &self,
        source: &ReportSource<'_>,
        baseline: Option<&ReportData<'static>>,
    ) -> Result<Vec<u8>, GooseError> {
        use crate::report::pdf::generate_pdf_from_html;
        use crate::report::print::generate_print_optimized_html_content;
        use tokio::fs as async_fs;

        // Generate HTML report content using the shared function
        let html_report = source.html(baseline)?;

        // Add print-optimized CSS for better PDF output using the shared function
        let print_optimized_html = generate_print_optimized_html_content(&html_report);

        // If pdf_print_html is configured, save the PDF-optimized HTML for debugging
        if let Some(pdf_print_html) = self.configuration.pdf_print_html() {
            if let Err(e) = async_fs::write(pdf_print_html, &print_optimized_html).await {
                return Err(GooseError::InvalidOption {
                    option: "--pdf-print-html".to_string(),
                    value: pdf_print_html.to_string(),
                    detail: format!("Failed to write PDF-optimized HTML: {e}"),
                });
            }
            info!("PDF-optimized HTML saved to: {pdf_print_html}");
        }

        // The Chrome found at startup, by create_reports, unless it was a
        // download that another Goose run's refresh has since removed.
        let startup = self.pdf_chrome().await?;
        let found_again;
        let chrome = if startup.needs_new_lookup() {
            info!(
                "the downloaded Chrome at {} was removed, most likely by another Goose run's refresh; looking for Chrome again",
                startup.path.display()
            );
            found_again = report::chrome::resolve_chrome().await?;
            &found_again
        } else {
            startup
        };

        // Generate PDF from HTML using the configurable timeout and scale.
        // Launching Chrome and rendering block, so they run off the runtime.
        let scale = self.configuration.pdf_scale_or_default();
        let verbose = self.configuration.verbose > 0;
        let timeout = self.configuration.pdf_timeout_or_default();
        let chrome = chrome.clone();
        tokio::task::spawn_blocking(move || {
            generate_pdf_from_html(&print_optimized_html, scale, verbose, timeout, &chrome)
        })
        .await
        .map_err(|e| chrome_task_error("writing the PDF report", e))?
    }

    /// A PDF report needs the pdf-reports feature, which `process_report`
    /// checks first.
    #[cfg(not(feature = "pdf-reports"))]
    async fn pdf_report(
        &self,
        _source: &ReportSource<'_>,
        _baseline: Option<&ReportData<'static>>,
    ) -> Result<Vec<u8>, GooseError> {
        unreachable!("process_report refuses a PDF report without the pdf-reports feature")
    }
}

/// What a report is written from: a run's metrics and graph data, the run's
/// own or copies of them, or a JSON report read back, and the record of the
/// run that goes with them.
pub(crate) struct ReportSource<'a> {
    pub configuration: &'a GooseConfiguration,
    pub metrics: &'a GooseMetrics,
    pub graph_data: &'a GraphData,
    /// Graphs were recorded. A JSON report read back without graph data has none.
    pub graphs: bool,
    /// The heatmap of a JSON report read back, which keeps no histograms.
    pub heatmap: Option<&'a serde_json::Value>,
    /// The tables of a JSON report read back.
    pub loaded: Option<&'a ReportData<'static>>,
    pub record: &'a RunRecord,
    /// The JSON report includes the graph data.
    pub graph_data_in_json: bool,
}

/// The JSON report: the record of the run, its metrics, and, when asked for,
/// the per second data the HTML report graphs.
#[derive(Serialize)]
struct JsonReport<'a> {
    #[serde(flatten)]
    record: &'a RunRecord,
    #[serde(flatten)]
    data: ReportData<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    graphs: Option<crate::graph::ReportGraphs<'a>>,
}

impl ReportSource<'_> {
    /// The report's tables, with any baseline's deltas.
    pub(crate) fn data(&self, baseline: Option<&ReportData<'static>>) -> ReportData<'_> {
        match self.loaded {
            Some(loaded) => {
                let mut data = loaded.clone();
                if let Some(baseline) = baseline {
                    common::apply_baseline_deltas(&mut data, baseline);
                }
                data
            }
            None => common::prepare_data_with_baseline(
                ReportOptions {
                    no_transaction_metrics: self.configuration.no_transaction_metrics,
                    no_scenario_metrics: self.configuration.no_scenario_metrics,
                    no_status_codes: self.configuration.no_status_codes,
                },
                self.metrics,
                baseline,
            ),
        }
    }

    /// Write the JSON report to `writer`.
    pub(crate) fn write_json<W: std::io::Write>(
        &self,
        writer: W,
        baseline: Option<&ReportData<'static>>,
    ) -> Result<(), GooseError> {
        let co_active = self.co_active();
        let report = JsonReport {
            record: self.record,
            data: self.data(baseline),
            graphs: (self.graph_data_in_json && self.graphs).then(|| {
                self.graph_data
                    .report_graphs(co_active, self.started(), self.heatmap)
            }),
        };
        serde_json::to_writer_pretty(writer, &report)?;
        Ok(())
    }

    /// Write the Markdown report to `writer`.
    pub(crate) fn write_markdown<W: std::io::Write>(
        &self,
        writer: &mut W,
        baseline: Option<&ReportData<'static>>,
    ) -> Result<(), GooseError> {
        report::write_markdown_report(writer, self.data(baseline), &self.record.intro())
    }

    /// When the run started, as the graphs count it.
    fn started(&self) -> DateTime<Utc> {
        self.metrics
            .history
            .first()
            .map_or_else(Utc::now, |step| step.timestamp)
    }

    /// The run's final second, counted as the graphs count seconds, when the
    /// run ended partway through it: the per second count graphs leave it
    /// out. `None` for a run still in progress, which has no end yet.
    fn partial_final_second(&self) -> Option<usize> {
        let first = self.metrics.history.first()?;
        let last = self.metrics.history.last()?;
        if !matches!(last.action, TestPlanStepAction::Finished) {
            return None;
        }
        let ran = (last.timestamp - first.timestamp).num_microseconds()?;
        (ran > 0 && ran % 1_000_000 != 0).then_some((ran / 1_000_000) as usize)
    }

    /// Whether coordinated omission mitigation recorded synthetic times.
    fn co_active(&self) -> bool {
        self.metrics
            .coordinated_omission_metrics
            .as_ref()
            .is_some_and(|metrics| metrics.has_events())
    }

    /// The heatmap card: from the histograms, or from the JSON read back.
    fn heatmap_markup(&self, started: DateTime<Utc>) -> String {
        match self.heatmap {
            Some(heatmap) => crate::graph::heatmap_card(heatmap),
            None => self
                .graph_data
                .get_latency_heatmap_markup(self.co_active(), started),
        }
    }

    /// The paragraphs at the top of the HTML report: the notes, and what the
    /// graphs cover.
    fn notes_markup(&self) -> String {
        let mut markup = String::new();
        for paragraph in self.record.intro() {
            let _ = write!(
                markup,
                "<p class=\"notes\">{}</p>",
                report::escape_html(&paragraph)
            );
        }
        if !self.graphs {
            let _ = write!(
                markup,
                "<p class=\"notes\">{}</p>",
                report::escape_html(GRAPHS_NOT_RECORDED)
            );
        } else if let Some(start) = &self.record.graphs_start {
            let _ = write!(
                markup,
                "<p class=\"notes\">{}</p>",
                report::escape_html(&start.describe())
            );
        }
        markup
    }

    /// The HTML report, which the PDF report is also made from.
    pub(crate) fn html(
        &self,
        baseline: Option<&ReportData<'static>>,
    ) -> Result<String, GooseError> {
        let test_start_time = self.started();
        let partial_second = self.partial_final_second();

        // Prepare report summary variables.
        let users = self.metrics.maximum_users.to_string();

        let mut steps_overview = String::new();
        for step in self.metrics.history.windows(2) {
            let (seconds, minutes, hours) = self
                .metrics
                .get_seconds_minutes_hours(&step[0].timestamp, &step[1].timestamp);
            let started = Local
                .timestamp_opt(step[0].timestamp.timestamp(), 0)
                // @TODO: error handling
                .unwrap()
                .format("%y-%m-%d %H:%M:%S");
            let stopped = Local
                .timestamp_opt(step[1].timestamp.timestamp(), 0)
                // @TODO: error handling
                .unwrap()
                .format("%y-%m-%d %H:%M:%S");
            match &step[0].action {
                // For maintaining just show the current number of users.
                TestPlanStepAction::Maintaining => {
                    let _ = write!(steps_overview,
                            "<tr><td>{:?}</td><td>{}</td><td>{}</td><td>{:02}:{:02}:{:02}</td><td>{}</td></tr>",
                            step[0].action,
                            started,
                            stopped,
                            hours,
                            minutes,
                            seconds,
                            step[0].users,
                        );
                }
                // For increasing show the current number of users to the new number of users.
                TestPlanStepAction::Increasing => {
                    let _ = write!(steps_overview,
                            "<tr><td>{:?}</td><td>{}</td><td>{}</td><td>{:02}:{:02}:{:02}</td><td>{} &rarr; {}</td></tr>",
                            step[0].action,
                            started,
                            stopped,
                            hours,
                            minutes,
                            seconds,
                            step[0].users,
                            step[1].users,
                        );
                }
                // For decreasing show the new number of users from the current number of users.
                TestPlanStepAction::Decreasing | TestPlanStepAction::Canceling => {
                    let _ = write!(steps_overview,
                            "<tr><td>{:?}</td><td>{}</td><td>{}</td><td>{:02}:{:02}:{:02}</td><td>{} &larr; {}</td></tr>",
                            step[0].action,
                            started,
                            stopped,
                            hours,
                            minutes,
                            seconds,
                            step[1].users,
                            step[0].users,
                        );
                }
                TestPlanStepAction::Finished => {
                    unreachable!("there shouldn't be a step after finished");
                }
            }
        }

        // Build a comma separated list of hosts.
        let hosts = &self.metrics.hosts.clone().into_iter().join(", ");

        let ReportData {
            raw_metrics: _,
            raw_request_metrics,
            raw_response_metrics,
            co_request_metrics,
            co_response_metrics,
            scenario_metrics,
            transaction_metrics,
            errors,
            status_code_metrics,
            coordinated_omission_metrics: _,
        } = self.data(baseline);

        // Compile the request metrics template.
        let mut raw_requests_rows = Vec::new();
        for metric in raw_request_metrics {
            raw_requests_rows.push(report::raw_request_metrics_row(metric));
        }

        // Compile the response metrics template.
        let mut raw_responses_rows = Vec::new();
        for metric in raw_response_metrics {
            raw_responses_rows.push(report::response_metrics_row(metric));
        }

        let co_requests_template = co_request_metrics
            .map(|co_request_metrics| {
                let co_request_rows = co_request_metrics
                    .into_iter()
                    .map(report::coordinated_omission_request_metrics_row)
                    .join("\n");

                report::coordinated_omission_request_metrics_template(&co_request_rows)
            })
            .unwrap_or_default();

        let co_responses_template = co_response_metrics
            .map(|co_response_metrics| {
                let co_response_rows = co_response_metrics
                    .into_iter()
                    .map(report::coordinated_omission_response_metrics_row)
                    .join("\n");

                report::coordinated_omission_response_metrics_template(&co_response_rows)
            })
            .unwrap_or_default();

        let co_metrics_template =
            if let Some(co_metrics) = &self.metrics.coordinated_omission_metrics {
                let co_summary = co_metrics.get_summary();
                report::coordinated_omission_metrics_template(&co_summary)
            } else {
                String::new()
            };

        let scenarios_template = scenario_metrics
            .map(|scenario_metric| {
                let scenarios_rows = scenario_metric
                    .into_iter()
                    .map(report::scenario_metrics_row)
                    .join("\n");

                report::scenario_metrics_template(
                    &scenarios_rows,
                    self.graph_data
                        .get_scenarios_per_second_graph(!self.configuration.no_granular_report)
                        .without_partial_second(partial_second)
                        .get_markup(&self.metrics.history, test_start_time),
                )
            })
            .unwrap_or_default();

        let transactions_template = transaction_metrics
            .map(|transaction_metrics| {
                let transactions_rows = transaction_metrics
                    .into_iter()
                    .map(report::transaction_metrics_row)
                    .join("\n");

                report::transaction_metrics_template(
                    &transactions_rows,
                    self.graph_data
                        .get_transactions_per_second_graph(!self.configuration.no_granular_report)
                        .without_partial_second(partial_second)
                        .get_markup(&self.metrics.history, test_start_time),
                )
            })
            .unwrap_or_default();

        let errors_template = errors
            .map(|errors| {
                let error_rows = errors.iter().map(report::error_row).join("\n");

                report::errors_template(
                    &error_rows,
                    self.graph_data
                        .get_errors_per_second_graph(!self.configuration.no_granular_report)
                        .without_partial_second(partial_second)
                        .get_markup(&self.metrics.history, test_start_time),
                )
            })
            .unwrap_or_default();

        let status_code_template = status_code_metrics
            .map(|status_code_metrics| {
                // Compile the status_code metrics rows.
                let mut status_code_rows = Vec::new();
                for metric in status_code_metrics {
                    status_code_rows.push(report::status_code_metrics_row(metric));
                }

                // Compile the status_code metrics template.
                report::status_code_metrics_template(&status_code_rows.join("\n"))
            })
            .unwrap_or_default();

        let thresholds_template = if self.metrics.thresholds.is_empty() {
            String::new()
        } else {
            let threshold_rows = self
                .metrics
                .thresholds
                .iter()
                .map(report::threshold_row)
                .join("\n");
            report::thresholds_template(&threshold_rows)
        };

        // Compile and return the report template.
        Ok(report::build_report(
            &users,
            &steps_overview,
            hosts,
            report::GooseReportTemplates {
                notes_template: &self.notes_markup(),
                raw_requests_template: &raw_requests_rows.join("\n"),
                raw_responses_template: &raw_responses_rows.join("\n"),
                co_requests_template: &co_requests_template,
                co_responses_template: &co_responses_template,
                transactions_template: &transactions_template,
                scenarios_template: &scenarios_template,
                status_codes_template: &status_code_template,
                errors_template: &errors_template,
                thresholds_template: &thresholds_template,
                graph_rps_template: &self
                    .graph_data
                    .get_requests_per_second_graph(!self.configuration.no_granular_report)
                    .without_partial_second(partial_second)
                    .get_markup(&self.metrics.history, test_start_time),
                graph_average_response_time_template: &self
                    .graph_data
                    .get_average_response_time_graph(!self.configuration.no_granular_report)
                    .get_markup(&self.metrics.history, test_start_time),
                graph_latency_heatmap_template: &self.heatmap_markup(test_start_time),
                graph_users_per_second: &self
                    .graph_data
                    .get_active_users_graph(!self.configuration.no_granular_report)
                    .get_markup(&self.metrics.history, test_start_time),
                co_metrics_template: &co_metrics_template,
            },
        ))
    }
}

/// What a report rendered from a JSON report without graph data says.
pub(crate) const GRAPHS_NOT_RECORDED: &str =
    "The graphs were not recorded: the JSON report this was rendered from has no graph data (--report-graph-data).";

/// A report file being written, gzipped when its name ends in `.gz`. Every
/// format goes through it, so a report is gzipped in this one place.
pub(crate) enum ReportOut {
    Plain(BufWriter<std::fs::File>),
    /// Buffered above the encoder, so it compresses a buffer at a time
    /// rather than each small write the serializers make.
    Gzip(BufWriter<flate2::write::GzEncoder<std::fs::File>>),
}

impl ReportOut {
    /// Write to `file`, gzipped at `level` when `gzip`.
    pub(crate) fn new(file: std::fs::File, gzip: bool, level: u32) -> Self {
        if gzip {
            ReportOut::Gzip(BufWriter::new(flate2::write::GzEncoder::new(
                file,
                flate2::Compression::new(level),
            )))
        } else {
            ReportOut::Plain(BufWriter::new(file))
        }
    }

    /// Write the gzip trailer, if any, and flush, returning any error.
    pub(crate) fn finish(self) -> std::io::Result<()> {
        match self {
            ReportOut::Plain(mut file) => file.flush(),
            ReportOut::Gzip(gzip) => gzip
                .into_inner()
                .map_err(|e| e.into_error())?
                .finish()?
                .flush(),
        }
    }
}

impl std::io::Write for ReportOut {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            ReportOut::Plain(file) => file.write(buf),
            ReportOut::Gzip(gzip) => gzip.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            ReportOut::Plain(file) => file.flush(),
            ReportOut::Gzip(gzip) => gzip.flush(),
        }
    }
}

/// Write a run's folder report from `source`: `report.html`, then
/// `report.json` last, each through a temporary file and a rename, so a
/// folder never holds half a report. When `compress` holds a level, each is
/// gzipped at that level and named with `.gz` added. Returns the files
/// written.
pub(crate) fn write_folder_report(
    folder: &std::path::Path,
    source: &ReportSource<'_>,
    baseline: Option<&ReportData<'static>>,
    html: &mut Option<String>,
    compress: Option<u32>,
) -> Result<Vec<PathBuf>, GooseError> {
    use std::fs;
    let gz = if compress.is_some() { ".gz" } else { "" };
    let level = compress.unwrap_or_default();
    let content = match html {
        Some(content) => content,
        None => html.insert(source.html(baseline)?),
    };
    let write = |name: &str, body: &dyn Fn(&mut ReportOut) -> std::io::Result<()>| {
        let name = format!("{name}{gz}");
        let path = folder.join(&name);
        let tmp = folder.join(format!(".{name}.tmp"));
        let written = fs::File::create(&tmp).and_then(|file| {
            let mut out = ReportOut::new(file.try_clone()?, compress.is_some(), level);
            body(&mut out)?;
            out.finish()?;
            file.sync_all()
        });
        match written.and_then(|()| fs::rename(&tmp, &path)) {
            Ok(()) => Ok(path),
            Err(err) => {
                let _ = fs::remove_file(&tmp);
                Err(crate::runs::folder_error(&path, err))
            }
        }
    };
    let html_path = write(crate::runs::REPORT_HTML, &|out| {
        out.write_all(content.as_bytes())
    })?;
    let json_path = write(crate::runs::REPORT_JSON, &|out| {
        source
            .write_json(out, baseline)
            .map_err(|e| std::io::Error::other(e.to_string()))
    })?;
    Ok(vec![html_path, json_path])
}

/// The error for a report that can't be written to `path`.
fn report_io_error(path: &str, err: std::io::Error) -> GooseError {
    GooseError::InvalidOption {
        option: "--report-file".to_string(),
        value: path.to_string(),
        detail: format!("Failed to create report file: {err}"),
    }
}

/// The error for a blocking Chrome task, such as `task`, that panicked or was
/// cancelled.
#[cfg(feature = "pdf-reports")]
fn chrome_task_error(task: &str, e: tokio::task::JoinError) -> GooseError {
    GooseError::InvalidOption {
        option: "--report-file".to_string(),
        value: format!("{task} did not finish: {e}"),
        detail: "The thread running Chrome panicked or was cancelled.".to_string(),
    }
}

/// Combines the results of writers that all ran, returning the first error
/// and logging any later one, which would otherwise be lost.
fn first_error(
    first: Result<(), GooseError>,
    second: Result<(), GooseError>,
) -> Result<(), GooseError> {
    match (first, second) {
        (Err(first), Err(second)) => {
            error!("a later report also failed: {second:?}");
            Err(first)
        }
        (first, second) => first.and(second),
    }
}

/// Helper to calculate requests and fails per seconds.
pub(crate) fn per_second_calculations(duration: usize, total: usize, fail: usize) -> (f32, f32) {
    let (requests_per_second, fails_per_second) = if duration == 0 {
        (0.0, 0.0)
    } else {
        (
            total as f32 / duration as f32,
            fail as f32 / duration as f32,
        )
    };
    (requests_per_second, fails_per_second)
}

/// One row of a console table of times: `name`, then the average, fastest,
/// slowest and median of `data` in milliseconds, to the hundredth under
/// 10 ms and whole from there, rounded half up.
fn write_times_row(
    fmt: &mut fmt::Formatter<'_>,
    name: &str,
    data: &GooseRequestMetricTimingData,
) -> fmt::Result {
    writeln!(
        fmt,
        " {:<24} | {:>11} | {:>10} | {:>11} | {:>10}",
        name,
        format_ms(data.average_ms_or_zero(), true),
        format_micros(data.minimum_time_us, true),
        format_micros(data.maximum_time_us, true),
        format_micros(data.percentile_us(0.5), true),
    )
}

fn determine_precision(value: f32) -> usize {
    if value < 1000.0 {
        2
    } else {
        0
    }
}

/// Format large number in locale appropriate style.
pub(crate) fn format_number(number: usize) -> String {
    (number).to_formatted_string(&Locale::en)
}

/// Format a `Value` for display with locale-formatted numbers (e.g. `1,234`),
/// showing both the value and delta if present.
pub(crate) fn format_value<T: FormatValue>(value: &Value<T>) -> String {
    match value {
        Value::Plain(v) => v.formatted(),
        Value::Delta { value: v, delta } => {
            let formatted_value = v.formatted();
            let formatted_delta = T::formatted_delta(*delta);
            if T::is_delta_non_negative(*delta) {
                format!("{formatted_value} (+{formatted_delta})")
            } else {
                format!("{formatted_value} ({formatted_delta})")
            }
        }
    }
}

/// A value [`format_value`] formats with thousands separators.
pub(crate) trait FormatValue: delta::DeltaValue {
    /// The value, with thousands separators.
    fn formatted(&self) -> String;
    /// A delta, with thousands separators and a `-` when negative.
    fn formatted_delta(delta: Self::Delta) -> String;
}

impl FormatValue for usize {
    fn formatted(&self) -> String {
        self.to_formatted_string(&Locale::en)
    }

    fn formatted_delta(delta: isize) -> String {
        delta.to_formatted_string(&Locale::en)
    }
}

impl FormatValue for MillisFloat {
    fn formatted(&self) -> String {
        MillisFloat::formatted(*self)
    }

    fn formatted_delta(delta: millis::MillisFloatDelta) -> String {
        delta.formatted()
    }
}

impl FormatValue for Millis {
    fn formatted(&self) -> String {
        format_micros(self.0, true)
    }

    fn formatted_delta(delta: millis::MillisDelta) -> String {
        delta.formatted()
    }
}

/// `percent` as the decimal it was written as, to a millionth, in f64.
pub(crate) fn percent_as_written(percent: f32) -> f64 {
    (f64::from(percent) * 1_000_000.0).round() / 1_000_000.0
}

/// The response time `percent` of the requests finished within, from
/// `(response time, count)` buckets in ascending order: the time of the
/// bucket that holds the request at that rank, kept between `min` and `max`.
/// The dashboard's per second percentiles follow the same rule.
pub(crate) fn response_time_percentile<T: Copy + Default + PartialOrd>(
    response_times: impl IntoIterator<Item = (T, usize)>,
    total_requests: usize,
    min: T,
    max: T,
    percent: f32,
) -> T {
    // In f64, which holds every count exactly (an f32 rounds counts past
    // 2^24), from the percent as written: 0.95 rather than the f32's
    // 0.94999999, so 10 times it is 9.5 and rounds up.
    let percentile_request = (total_requests as f64 * percent_as_written(percent)).round() as usize;
    debug!("percentile: {percent}, request {percentile_request} of total {total_requests}");

    let mut total_count: usize = 0;

    for (value, counter) in response_times {
        // A baseline file's counts are not checked, so they may sum past
        // `usize::MAX`.
        total_count = total_count.saturating_add(counter);
        if total_count >= percentile_request {
            if value < min {
                return min;
            } else if value > max {
                return max;
            } else {
                return value;
            }
        }
    }

    T::default()
}

/// Helper to count and aggregate seen status codes.
pub(crate) fn prepare_status_codes(
    status_code_counts: &HashMap<u16, usize>,
    aggregate_counts: &mut Option<&mut HashMap<u16, usize>>,
) -> String {
    let mut codes: String = "".to_string();
    for (status_code, count) in status_code_counts {
        if codes.is_empty() {
            codes = format!(
                "{} [{}]",
                count.to_formatted_string(&Locale::en),
                status_code
            );
        } else {
            codes = format!(
                "{}, {} [{}]",
                codes.clone(),
                count.to_formatted_string(&Locale::en),
                status_code
            );
        }
        if let Some(aggregate_status_code_counts) = aggregate_counts.as_mut() {
            let new_count = if let Some(existing_status_code_count) =
                aggregate_status_code_counts.get(status_code)
            {
                *existing_status_code_count + *count
            } else {
                *count
            };
            aggregate_status_code_counts.insert(*status_code, new_count);
        }
    }
    codes
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod test {
    use std::sync::Arc;

    use super::*;

    /// A response under a millisecond is a time of 0 ms, and the minimum:
    /// the counter, not a minimum of 0, says whether there is a time yet.
    #[test]
    fn timing_data_minimum_includes_zero() {
        let record = |times: &[u64]| {
            let mut data = GooseRequestMetricTimingData::new(None);
            for &time in times {
                data.record_time(time);
            }
            data
        };
        assert_eq!(record(&[5, 0, 3]).minimum_time_us, 0);
        assert_eq!(record(&[0, 5]).minimum_time_us, 0);
        assert_eq!(record(&[5, 3]).minimum_time_us, 3_000);
        assert_eq!(record(&[]).minimum_time_us, 0);

        // Merging: an empty side never sets the minimum; a 0 always does.
        let mut merged = record(&[5]);
        merged.merge(&record(&[]));
        assert_eq!(merged.minimum_time_us, 5_000);
        merged.merge(&record(&[0, 9]));
        assert_eq!(merged.minimum_time_us, 0);
        merged.merge(&record(&[4]));
        assert_eq!(merged.minimum_time_us, 0);
        let mut empty = record(&[]);
        empty.merge(&record(&[7]));
        assert_eq!(empty.minimum_time_us, 7_000);
        let mut from_zero = record(&[0]);
        from_zero.merge(&record(&[2]));
        assert_eq!(from_zero.minimum_time_us, 0);

        // Percentiles are no longer clamped up to the fastest nonzero time.
        let data = record(&[0, 0, 0, 5]);
        assert_eq!(data.percentile_us(0.5), 0);

        // Status code summaries too.
        let mut summary = StatusCodeTimingSummary::default();
        summary.record_us(4_000);
        summary.record_us(0);
        assert_eq!(
            (summary.min_time_us, summary.max_time_us, summary.count),
            (0, 4_000, 2)
        );
        let mut other = StatusCodeTimingSummary::default();
        other.merge(&StatusCodeTimingSummary::default());
        assert_eq!(other.count, 0);
        other.merge(&summary);
        assert_eq!(other.min_time_us, 0);
        let mut later = StatusCodeTimingSummary::default();
        later.record_us(9_000);
        later.merge(&summary);
        assert_eq!(later.min_time_us, 0);
    }

    /// A request with these measured times, counted `counted` times (with
    /// coordinated omission mitigation, synthetic requests are counted too).
    fn request_with_times(
        path: &str,
        times: &[u64],
        counted: usize,
    ) -> GooseRequestMetricAggregate {
        let mut request = GooseRequestMetricAggregate::new(path, GooseMethod::Get, 0, "");
        for &time in times {
            request.record_time(time, false);
        }
        request.success_count = counted;
        request
    }

    fn report_of(metrics: &GooseMetrics) -> common::ReportData<'_> {
        common::prepare_data_with_baseline(
            ReportOptions {
                no_transaction_metrics: true,
                no_scenario_metrics: true,
                no_status_codes: true,
            },
            metrics,
            None,
        )
    }

    /// The console's response time and percentile tables, as printed.
    struct ConsoleTables<'a>(&'a GooseMetrics);

    impl fmt::Display for ConsoleTables<'_> {
        fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.fmt_response_times(fmt)?;
            self.0.fmt_percentiles(fmt)
        }
    }

    /// The cells of the printed rows named `name`, measured then adjusted.
    fn printed_rows(text: &str, name: &str) -> Vec<Vec<String>> {
        text.lines()
            .filter(|line| line.split('|').next().map(str::trim) == Some(name))
            .map(|line| {
                line.split('|')
                    .skip(1)
                    .map(|cell| cell.trim().to_string())
                    .collect()
            })
            .collect()
    }

    /// The minimum over several requests counts a 0 ms minimum.
    #[test]
    fn aggregate_minimum_includes_zero() {
        let mut metrics = GooseMetrics::default();
        metrics.final_metrics = true;
        metrics
            .requests
            .insert("GET /a".to_string(), request_with_times("/a", &[0, 8], 2));
        metrics
            .requests
            .insert("GET /b".to_string(), request_with_times("/b", &[3], 1));
        let report = report_of(&metrics);
        let aggregated = report.raw_request_metrics.last().unwrap();
        assert_eq!(aggregated.name, "Aggregated");
        assert_eq!(aggregated.response_time_minimum.value(), 0);
        let printed = ConsoleTables(&metrics).to_string();
        // Avg, Min, Max, Median.
        assert_eq!(printed_rows(&printed, "Aggregated")[0][1], "0");
    }

    /// The console's transaction times table, as printed.
    struct ConsoleTransactionTimes<'a>(&'a GooseMetrics);

    impl fmt::Display for ConsoleTransactionTimes<'_> {
        fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.fmt_transaction_times(fmt)
        }
    }

    /// The report's transaction Aggregated average is the transactions'
    /// total time over their count, as the console prints it, not the
    /// requests' total time over the transaction count.
    #[test]
    fn report_transaction_aggregate_average_uses_transaction_times() {
        let mut metrics = GooseMetrics::default();
        metrics.final_metrics = true;
        metrics.display_metrics = true;
        let mut first = TransactionMetricAggregate::new(
            0,
            "Scenario".into(),
            0,
            TransactionName::InheritNameByRequests(Arc::from("first")),
        );
        first.set_time(10, true);
        first.set_time(13, true);
        let mut second = TransactionMetricAggregate::new(
            0,
            "Scenario".into(),
            1,
            TransactionName::InheritNameByRequests(Arc::from("second")),
        );
        second.set_time(31, true);
        metrics.transactions = vec![vec![first, second]];
        metrics.requests.insert(
            "GET /a".to_string(),
            request_with_times("/a", &[100, 300], 2),
        );
        let report = common::prepare_data_with_baseline(
            ReportOptions {
                no_transaction_metrics: false,
                no_scenario_metrics: true,
                no_status_codes: true,
            },
            &metrics,
            None,
        );
        let transactions = report.transaction_metrics.unwrap();
        let aggregated = transactions.last().unwrap();
        assert_eq!(aggregated.name, "Aggregated");
        // (10 + 13 + 31) / 3 = 18; the old figure was (100 + 300) / 3.
        let average = *aggregated.response_time_average.as_ref().unwrap().value();
        assert_eq!(average, 18.0);
        let printed = ConsoleTransactionTimes(&metrics).to_string();
        assert_eq!(printed_rows(&printed, "Aggregated")[0][0], "18");
    }

    /// Whether `value` is the bucket of `ms` milliseconds: within 1/128.
    fn near(value: Millis, ms: u64) -> bool {
        value.0.abs_diff(ms * 1000) * 128 <= ms * 1000
    }

    /// With coordinated omission mitigation every request is counted, the
    /// synthetic ones too, but only measured times have a time: the
    /// report's measured aggregate takes its rank and its average over those.
    #[test]
    fn report_aggregate_uses_measured_times() {
        let mut metrics = GooseMetrics::default();
        let times: Vec<u64> = (1..=10).map(|i| i * 10).collect();
        // Ten measured times, counted 40 times.
        metrics
            .requests
            .insert("GET /a".to_string(), request_with_times("/a", &times, 40));
        metrics
            .requests
            .insert("GET /b".to_string(), request_with_times("/b", &[5], 1));
        let report = report_of(&metrics);
        let aggregated = report.raw_response_metrics.last().unwrap();
        assert_eq!(aggregated.name, "Aggregated");
        // Eleven times: 5, 10, 20, ..., 100.
        assert!(near(aggregated.percentile_50.value(), 50));
        assert_eq!(aggregated.percentile_99.value(), 100);
        assert_eq!(aggregated.percentile_100.value(), 100);
        let request = report.raw_request_metrics.last().unwrap();
        assert_eq!(request.number_of_requests.value(), 41);
        assert_eq!(
            format!("{:.2}", request.response_time_average.value().0),
            "50.45"
        );
    }

    /// The adjusted aggregate covers every request: one with synthetic times
    /// contributes its adjusted times, one without its measured times. The
    /// console's adjusted maximum is the largest of all of them.
    #[test]
    fn adjusted_aggregate_includes_every_request() {
        let mut metrics = GooseMetrics::default();
        metrics.final_metrics = true;
        // /a: measured 10 and 20, then a synthetic 900.
        let mut a = request_with_times("/a", &[10, 20], 3);
        a.record_time(900, true);
        // /b: measured 30 and 40, then a synthetic 500.
        let mut b = request_with_times("/b", &[30, 40], 3);
        b.record_time(500, true);
        // /c: measured 50 and 60, no synthetic times.
        let c = request_with_times("/c", &[50, 60], 2);
        metrics.requests.insert("GET /a".to_string(), a);
        metrics.requests.insert("GET /b".to_string(), b);
        metrics.requests.insert("GET /c".to_string(), c);

        // Adjusted times: 10, 20, 30, 40, 50, 60, 500, 900.
        let report = report_of(&metrics);
        let co = report.co_response_metrics.as_ref().unwrap();
        let aggregated = co.last().unwrap();
        assert_eq!(aggregated.name, "Aggregated");
        assert!(near(aggregated.percentile_50.value(), 40));
        assert!(near(aggregated.percentile_90.value(), 500));
        assert_eq!(aggregated.percentile_100.value(), 900);
        let co_requests = report.co_request_metrics.as_ref().unwrap();
        assert_eq!(
            co_requests.last().unwrap().response_time_maximum.value(),
            900
        );
        assert_eq!(
            format!(
                "{:.2}",
                co_requests.last().unwrap().response_time_average.value().0
            ),
            "201.25"
        );

        let printed = ConsoleTables(&metrics).to_string();
        let rows = printed_rows(&printed, "Aggregated");
        // Response times: measured, then adjusted (Avg, Std Dev, Max, Median).
        assert_eq!(rows[1][0], "201");
        // The standard deviation of the measured and adjusted averages, 35
        // and 201.25 ms.
        assert_eq!(rows[1][1], "118");
        // And /a's, of 15 and 310 ms.
        assert_eq!(printed_rows(&printed, "GET /a")[1][1], "209");
        // /a's adjusted average, 310 ms, as a time.
        assert_eq!(printed_rows(&printed, "GET /a")[1][0], "310");
        assert_eq!(rows[1][2], "900");
        assert_eq!(rows[1][3], "40");
        // Percentiles: measured, then adjusted (50%, 75%, 98%, ...).
        assert_eq!(rows[3][0], "40");
        assert_eq!(rows[3][1], "60");
        assert_eq!(rows[3][2], "900");
    }

    /// The reports keep averages as `f64`, so a large mean prints exactly to
    /// the hundredth; as an `f32` this one printed 29609.38.
    #[test]
    fn report_average_prints_exactly() {
        let mut metrics = GooseMetrics::default();
        let mut request = GooseRequestMetricAggregate::new("/a", GooseMethod::Get, 0, "");
        request.raw_data.total_time_us = 2_960_937_350_000;
        request.raw_data.counter = 100_000;
        request
            .raw_data
            .times_us
            .insert(buckets::bucket_floor(30_000_000), 100_000);
        request.raw_data.minimum_time_us = 29_000_000;
        request.raw_data.maximum_time_us = 30_000_000;
        request.success_count = 100_000;
        metrics.requests.insert("GET /a".to_string(), request);
        let report = report_of(&metrics);
        for row in &report.raw_request_metrics {
            assert_eq!(
                format!("{:.2}", row.response_time_average.value().0),
                "29609.37"
            );
        }
        // A value still computed as an f32 keeps the digits it prints with.
        assert_eq!(NullableFloat::from(0.1f32).0, 0.1f64);
    }

    /// The histogram `CROSS_CHECK` in src/dashboard/static/heatmap.test.mjs
    /// checks the dashboard's per second percentiles against: both use this
    /// rule, so both must give these answers (#713).
    #[test]
    fn response_time_percentile_matches_the_dashboard_heatmap() {
        let histogram: [(usize, usize); 24] = [
            (0, 40),
            (1, 60),
            (2, 80),
            (3, 100),
            (5, 120),
            (8, 110),
            (12, 100),
            (18, 90),
            (25, 70),
            (40, 60),
            (60, 50),
            (90, 40),
            (100, 30),
            (110, 25),
            (150, 20),
            (200, 15),
            (300, 12),
            (400, 10),
            (500, 8),
            (700, 6),
            (1000, 5),
            (2000, 3),
            (5000, 2),
            (10000, 1),
        ];
        let total: usize = histogram.iter().map(|(_, count)| count).sum();
        assert_eq!(total, 1057);
        let percentile =
            |percent| response_time_percentile(histogram.iter().copied(), total, 0, 10000, percent);
        assert_eq!(percentile(0.5), 12);
        assert_eq!(percentile(0.95), 200);
        assert_eq!(percentile(0.99), 700);
    }

    #[test]
    fn max_response_time_percentile() {
        let percentile = |times: &BTreeMap<usize, usize>, total, min, max, percent| {
            response_time_percentile(
                times.iter().map(|(&time, &count)| (time, count)),
                total,
                min,
                max,
                percent,
            )
        };
        let mut response_times: BTreeMap<usize, usize> = BTreeMap::new();
        response_times.insert(1, 1);
        response_times.insert(2, 1);
        response_times.insert(3, 1);
        // 3 * .5 = 1.5, rounds to 2.
        assert_eq!(percentile(&response_times, 3, 1, 3, 0.5), 2);
        response_times.insert(3, 2);
        // 4 * .5 = 2
        assert_eq!(percentile(&response_times, 4, 1, 3, 0.5), 2);
        // 4 * .25 = 1
        assert_eq!(percentile(&response_times, 4, 1, 3, 0.25), 1);
        // 4 * .75 = 3
        assert_eq!(percentile(&response_times, 4, 1, 3, 0.75), 3);
        // 4 * 1 = 4 (and the 4th response time is also 3)
        assert_eq!(percentile(&response_times, 4, 1, 3, 1.0), 3);

        // 4 * .5 = 2, but uses specified minimum of 2
        assert_eq!(percentile(&response_times, 4, 2, 3, 0.25), 2);
        // 4 * .75 = 3, but uses specified maximum of 2
        assert_eq!(percentile(&response_times, 4, 1, 2, 0.75), 2);

        response_times.insert(10, 25);
        response_times.insert(20, 25);
        response_times.insert(30, 25);
        response_times.insert(50, 25);
        response_times.insert(100, 10);
        response_times.insert(200, 1);
        assert_eq!(percentile(&response_times, 115, 1, 200, 0.9), 50);
        assert_eq!(percentile(&response_times, 115, 1, 200, 0.99), 100);
        assert_eq!(percentile(&response_times, 115, 1, 200, 0.999), 200);
    }

    #[test]
    fn calculate_per_second() {
        // With duration of 0, requests and fails per second is always 0.
        let mut duration = 0;
        let mut total = 10;
        let fail = 10;
        let (requests_per_second, fails_per_second) =
            per_second_calculations(duration, total, fail);
        assert!(requests_per_second == 0.0);
        assert!(fails_per_second == 0.0);
        // Changing total doesn't affect requests and fails as duration is still 0.
        total = 100;
        let (requests_per_second, fails_per_second) =
            per_second_calculations(duration, total, fail);
        assert!(requests_per_second == 0.0);
        assert!(fails_per_second == 0.0);

        // With non-zero duration, requests and fails per second return properly.
        duration = 10;
        let (requests_per_second, fails_per_second) =
            per_second_calculations(duration, total, fail);
        assert!((requests_per_second - 10.0).abs() < f32::EPSILON);
        assert!((fails_per_second - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn goose_raw_request() {
        const PATH: &str = "http://127.0.0.1/";
        let raw_request = GooseRawRequest::new(GooseMethod::Get, PATH, vec![], "");
        let mut request_metric = GooseRequestMetric::new(
            raw_request,
            TransactionDetail {
                scenario_index: 0,
                scenario_name: Arc::from("LoadTestUser"),
                transaction_index: Some(5),
                transaction_name: TransactionName::InheritNameByRequests(Arc::from("front page")),
            },
            "/",
            0,
            0,
        );
        assert_eq!(request_metric.raw.method, GooseMethod::Get);
        assert_eq!(request_metric.scenario_index, 0);
        assert_eq!(request_metric.scenario_name.as_ref(), "LoadTestUser");
        assert_eq!(request_metric.transaction_index, 5);
        assert_eq!(
            request_metric.transaction_name.name_for_transaction(),
            "front page"
        );
        assert_eq!(request_metric.raw.url, PATH.to_string());
        assert_eq!(request_metric.name.as_ref(), "/");
        assert_eq!(request_metric.response_time, 0);
        assert_eq!(request_metric.status_code, 0);
        assert!(request_metric.success);
        assert!(!request_metric.update);

        let response_time = 123;
        request_metric.set_response_time(response_time);
        assert_eq!(request_metric.raw.method, GooseMethod::Get);
        assert_eq!(request_metric.name.as_ref(), "/");
        assert_eq!(request_metric.raw.url, PATH.to_string());
        assert_eq!(request_metric.response_time, response_time as u64);
        assert_eq!(request_metric.status_code, 0);
        assert!(request_metric.success);
        assert!(!request_metric.update);

        let status_code = reqwest::StatusCode::OK;
        request_metric.set_status_code(Some(status_code));
        assert_eq!(request_metric.raw.method, GooseMethod::Get);
        assert_eq!(request_metric.name.as_ref(), "/");
        assert_eq!(request_metric.raw.url, PATH.to_string());
        assert_eq!(request_metric.response_time, response_time as u64);
        assert_eq!(request_metric.status_code, 200);
        assert!(request_metric.success);
        assert!(!request_metric.update);
    }

    #[test]
    fn goose_request() {
        let mut request = GooseRequestMetricAggregate::new("/", GooseMethod::Get, 0, "");
        assert_eq!(request.path, "/".to_string());
        assert_eq!(request.method, GooseMethod::Get);
        assert_eq!(request.raw_data.times_us.len(), 0);
        assert_eq!(request.raw_data.minimum_time_us, 0);
        assert_eq!(request.raw_data.maximum_time_us, 0);
        assert_eq!(request.raw_data.total_time_us, 0);
        assert_eq!(request.raw_data.counter, 0);
        assert_eq!(request.status_code_counts.len(), 0);
        assert_eq!(request.success_count, 0);
        assert_eq!(request.fail_count, 0);

        // Tracking a response time updates several fields.
        request.record_time(1, false);
        // We've seen only one response time so far.
        assert_eq!(request.raw_data.times_us.len(), 1);
        // We've seen one response time of length 1 ms, in its bucket.
        assert_eq!(request.raw_data.times_us[&buckets::bucket_floor(1_000)], 1);
        // The minimum response time seen so far is 1.
        assert_eq!(request.raw_data.minimum_time_us, 1_000);
        // The maximum response time seen so far is 1.
        assert_eq!(request.raw_data.maximum_time_us, 1_000);
        // We've seen a total of 1 ms of response time so far.
        assert_eq!(request.raw_data.total_time_us, 1_000);
        // We've seen a total of 2 response times so far.
        assert_eq!(request.raw_data.counter, 1);
        // Nothing else changes.
        assert_eq!(request.path, "/".to_string());
        assert_eq!(request.method, GooseMethod::Get);
        assert_eq!(request.status_code_counts.len(), 0);
        assert_eq!(request.success_count, 0);
        assert_eq!(request.fail_count, 0);

        // Tracking another response time updates all related fields.
        request.record_time(10, false);
        // We've added a new unique response time.
        assert_eq!(request.raw_data.times_us.len(), 2);
        // We've seen the 10 ms response time 1 time.
        assert_eq!(request.raw_data.times_us[&buckets::bucket_floor(10_000)], 1);
        // Minimum doesn't change.
        assert_eq!(request.raw_data.minimum_time_us, 1_000);
        // Maximum is new response time.
        assert_eq!(request.raw_data.maximum_time_us, 10_000);
        // Total combined response times is now 11 ms.
        assert_eq!(request.raw_data.total_time_us, 11_000);
        // We've seen two response times so far.
        assert_eq!(request.raw_data.counter, 2);
        // Nothing else changes.
        assert_eq!(request.path, "/".to_string());
        assert_eq!(request.method, GooseMethod::Get);
        assert_eq!(request.status_code_counts.len(), 0);
        assert_eq!(request.success_count, 0);
        assert_eq!(request.fail_count, 0);

        // Tracking another response time updates all related fields.
        request.record_time(10, false);
        // We've incremented the counter of an existing response time.
        assert_eq!(request.raw_data.times_us.len(), 2);
        // We've seen the 10 ms response time 2 times.
        assert_eq!(request.raw_data.times_us[&buckets::bucket_floor(10_000)], 2);
        // Minimum doesn't change.
        assert_eq!(request.raw_data.minimum_time_us, 1_000);
        // Maximum doesn't change.
        assert_eq!(request.raw_data.maximum_time_us, 10_000);
        // Total combined response times is now 21 ms.
        assert_eq!(request.raw_data.total_time_us, 21_000);
        // We've seen three response times so far.
        assert_eq!(request.raw_data.counter, 3);

        // Tracking another response time updates all related fields.
        request.record_time(101, false);
        // We've added a new response time for the first time.
        assert_eq!(request.raw_data.times_us.len(), 3);
        // The response time is kept in its microsecond bucket, seen once.
        assert_eq!(
            request.raw_data.times_us[&buckets::bucket_floor(101_000)],
            1
        );
        // Minimum doesn't change.
        assert_eq!(request.raw_data.minimum_time_us, 1_000);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.raw_data.maximum_time_us, 101_000);
        // Total combined response times is now 122 ms.
        assert_eq!(request.raw_data.total_time_us, 122_000);
        // We've seen four response times so far.
        assert_eq!(request.raw_data.counter, 4);

        // Tracking another response time updates all related fields.
        request.record_time(102, false);
        // A bucket is at most 1/64 of its time wide, so 102 ms is not in 101 ms's.
        assert_eq!(request.raw_data.times_us.len(), 4);
        assert_eq!(
            request.raw_data.times_us[&buckets::bucket_floor(102_000)],
            1
        );
        // Minimum doesn't change.
        assert_eq!(request.raw_data.minimum_time_us, 1_000);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.raw_data.maximum_time_us, 102_000);
        // Add 102 to the total response time so far.
        assert_eq!(request.raw_data.total_time_us, 224_000);
        // We've seen five response times so far.
        assert_eq!(request.raw_data.counter, 5);

        // Tracking another response time updates all related fields.
        request.record_time(155, false);
        // Adds a new response time.
        assert_eq!(request.raw_data.times_us.len(), 5);
        // The response time is kept in its microsecond bucket, seen once.
        assert_eq!(
            request.raw_data.times_us[&buckets::bucket_floor(155_000)],
            1
        );
        // Minimum doesn't change.
        assert_eq!(request.raw_data.minimum_time_us, 1_000);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.raw_data.maximum_time_us, 155_000);
        // Add 155 to the total response time so far.
        assert_eq!(request.raw_data.total_time_us, 379_000);
        // We've seen six response times so far.
        assert_eq!(request.raw_data.counter, 6);

        // Tracking another response time updates all related fields.
        request.record_time(2345, false);
        // Adds a new response time.
        assert_eq!(request.raw_data.times_us.len(), 6);
        // The response time is kept in its microsecond bucket, seen once.
        assert_eq!(
            request.raw_data.times_us[&buckets::bucket_floor(2_345_000)],
            1
        );
        // Minimum doesn't change.
        assert_eq!(request.raw_data.minimum_time_us, 1_000);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.raw_data.maximum_time_us, 2_345_000);
        // Add 2345 to the total response time so far.
        assert_eq!(request.raw_data.total_time_us, 2_724_000);
        // We've seen seven response times so far.
        assert_eq!(request.raw_data.counter, 7);

        // Tracking another response time updates all related fields.
        request.record_time(987654321, false);
        // Adds a new response time.
        assert_eq!(request.raw_data.times_us.len(), 7);
        // The response time is kept in its microsecond bucket, seen once.
        assert_eq!(
            request.raw_data.times_us[&buckets::bucket_floor(987_654_321_000)],
            1
        );
        // Minimum doesn't change.
        assert_eq!(request.raw_data.minimum_time_us, 1_000);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.raw_data.maximum_time_us, 987_654_321_000);
        // Add 987654321 to the total response time so far.
        assert_eq!(request.raw_data.total_time_us, 987_657_045_000);
        // We've seen eight response times so far.
        assert_eq!(request.raw_data.counter, 8);

        // Tracking status code updates all related fields.
        request.set_status_code(200);
        // We've seen only one status code.
        assert_eq!(request.status_code_counts.len(), 1);
        // First time seeing this status code.
        assert_eq!(request.status_code_counts[&200], 1);
        // As status code tracking is optional, we don't track success/fail here.
        assert_eq!(request.success_count, 0);
        assert_eq!(request.fail_count, 0);
        // Nothing else changes.
        assert_eq!(request.raw_data.times_us.len(), 7);
        assert_eq!(request.raw_data.minimum_time_us, 1_000);
        assert_eq!(request.raw_data.maximum_time_us, 987_654_321_000);
        assert_eq!(request.raw_data.total_time_us, 987_657_045_000);
        assert_eq!(request.raw_data.counter, 8);

        // Tracking status code updates all related fields.
        request.set_status_code(200);
        // We've seen only one unique status code.
        assert_eq!(request.status_code_counts.len(), 1);
        // Second time seeing this status code.
        assert_eq!(request.status_code_counts[&200], 2);

        // Tracking status code updates all related fields.
        request.set_status_code(0);
        // We've seen two unique status codes.
        assert_eq!(request.status_code_counts.len(), 2);
        // First time seeing a client-side error.
        assert_eq!(request.status_code_counts[&0], 1);

        // Tracking status code updates all related fields.
        request.set_status_code(500);
        // We've seen three unique status codes.
        assert_eq!(request.status_code_counts.len(), 3);
        // First time seeing an internal server error.
        assert_eq!(request.status_code_counts[&500], 1);

        // Tracking status code updates all related fields.
        request.set_status_code(308);
        // We've seen four unique status codes.
        assert_eq!(request.status_code_counts.len(), 4);
        // First time seeing an internal server error.
        assert_eq!(request.status_code_counts[&308], 1);

        // Tracking status code updates all related fields.
        request.set_status_code(200);
        // We've seen four unique status codes.
        assert_eq!(request.status_code_counts.len(), 4);
        // Third time seeing this status code.
        assert_eq!(request.status_code_counts[&200], 3);
        // Nothing else changes.
        assert_eq!(request.success_count, 0);
        assert_eq!(request.fail_count, 0);
        assert_eq!(request.raw_data.times_us.len(), 7);
        assert_eq!(request.raw_data.minimum_time_us, 1_000);
        assert_eq!(request.raw_data.maximum_time_us, 987_654_321_000);
        assert_eq!(request.raw_data.total_time_us, 987_657_045_000);
        assert_eq!(request.raw_data.counter, 8);
    }

    #[test]
    fn record_status_code_time() {
        let mut request = GooseRequestMetricAggregate::new("/", GooseMethod::Get, 0, "");
        assert!(request.status_code_timings.is_empty());

        // First recording for status 200.
        request.record_status_code_time(200, 50);
        assert_eq!(request.status_code_timings.len(), 1);
        let s = &request.status_code_timings[&200];
        assert_eq!(s.count, 1);
        assert_eq!(s.total_time_us, 50_000);
        assert_eq!(s.min_time_us, 50_000);
        assert_eq!(s.max_time_us, 50_000);

        // Second recording for 200 with a lower time updates min.
        request.record_status_code_time(200, 10);
        let s = &request.status_code_timings[&200];
        assert_eq!(s.count, 2);
        assert_eq!(s.total_time_us, 60_000);
        assert_eq!(s.min_time_us, 10_000);
        assert_eq!(s.max_time_us, 50_000);

        // Third recording for 200 with a higher time updates max.
        request.record_status_code_time(200, 100);
        let s = &request.status_code_timings[&200];
        assert_eq!(s.count, 3);
        assert_eq!(s.total_time_us, 160_000);
        assert_eq!(s.min_time_us, 10_000);
        assert_eq!(s.max_time_us, 100_000);

        // Recording for a different status code is tracked separately.
        request.record_status_code_time(404, 75);
        assert_eq!(request.status_code_timings.len(), 2);
        let s = &request.status_code_timings[&404];
        assert_eq!(s.count, 1);
        assert_eq!(s.total_time_us, 75_000);
        assert_eq!(s.min_time_us, 75_000);
        assert_eq!(s.max_time_us, 75_000);
        // 200 entry is unchanged.
        assert_eq!(request.status_code_timings[&200].count, 3);
    }

    #[test]
    fn status_code_breakdowns_none_when_single() {
        let mut request = GooseRequestMetricAggregate::new("/", GooseMethod::Get, 0, "");
        // Empty timings returns None.
        assert!(request.status_code_breakdowns().is_none());

        // Single status code returns None.
        request.record_status_code_time(200, 50);
        assert!(request.status_code_breakdowns().is_none());
    }

    #[test]
    fn status_code_breakdowns_sorted_with_percentages() {
        let mut request = GooseRequestMetricAggregate::new("/", GooseMethod::Get, 0, "");
        // 3 requests at 200, 1 request at 500.
        request.record_status_code_time(500, 200);
        request.record_status_code_time(200, 10);
        request.record_status_code_time(200, 20);
        request.record_status_code_time(200, 30);

        let breakdowns = request
            .status_code_breakdowns()
            .expect("should have breakdowns");
        assert_eq!(breakdowns.len(), 2);

        // Sorted by status code.
        assert_eq!(breakdowns[0].status_code, 200);
        assert_eq!(breakdowns[1].status_code, 500);

        // Counts.
        assert_eq!(breakdowns[0].count, 3);
        assert_eq!(breakdowns[1].count, 1);

        // Percentages: 3/4 = 75%, 1/4 = 25%.
        assert!((breakdowns[0].percentage - 75.0).abs() < 0.1);
        assert!((breakdowns[1].percentage - 25.0).abs() < 0.1);

        // Average for 200: (10+20+30)/3 = 20.
        assert!((breakdowns[0].average - 20.0).abs() < 0.1);
        // Average for 500: 200/1 = 200.
        assert!((breakdowns[1].average - 200.0).abs() < 0.1);

        // Min/max for 200.
        assert_eq!(breakdowns[0].min_time_us, 10_000);
        assert_eq!(breakdowns[0].max_time_us, 30_000);

        // Min/max for 500.
        assert_eq!(breakdowns[1].min_time_us, 200_000);
        assert_eq!(breakdowns[1].max_time_us, 200_000);
    }

    #[test]
    fn format_value_output() {
        // Plain value with locale formatting
        assert_eq!(format_value(&Value::Plain(1000usize)), "1,000");
        assert_eq!(format_value(&Value::Plain(42usize)), "42");

        // Positive delta with locale formatting
        assert_eq!(
            format_value(&Value::Delta {
                value: 1000usize,
                delta: 200isize
            }),
            "1,000 (+200)"
        );

        // Large positive delta
        assert_eq!(
            format_value(&Value::Delta {
                value: 10000usize,
                delta: 5000isize
            }),
            "10,000 (+5,000)"
        );

        // Negative delta
        assert_eq!(
            format_value(&Value::Delta {
                value: 800usize,
                delta: -200isize
            }),
            "800 (-200)"
        );

        // Large negative delta
        assert_eq!(
            format_value(&Value::Delta {
                value: 1000usize,
                delta: -5000isize
            }),
            "1,000 (-5,000)"
        );

        // Zero delta
        assert_eq!(
            format_value(&Value::Delta {
                value: 500usize,
                delta: 0isize
            }),
            "500 (+0)"
        );

        // Times and means, as the HTML report shows them against a
        // baseline: the difference of the values shown, with separators.
        let mut mean = Value::Plain(MillisFloat(1_436.69));
        mean.diff(MillisFloat(436.2));
        assert_eq!(format_value(&mean), "1,437 (+1,001)");
        let mut time = Value::Plain(Millis(2_341));
        time.diff(Millis(12_000));
        assert_eq!(format_value(&time), "2.34 (-9.66)");
    }

    /// Build a minimal GooseRequestMetric for testing.
    fn make_test_request_metric() -> GooseRequestMetric {
        let raw = GooseRawRequest::new(GooseMethod::Get, "http://localhost/", vec![], "");
        GooseRequestMetric::new(
            raw,
            TransactionDetail {
                scenario_index: 0,
                scenario_name: Arc::from("TestScenario"),
                transaction_index: Some(0),
                transaction_name: TransactionName::InheritNameByRequests(Arc::from("tx")),
            },
            "/",
            0,
            0,
        )
    }

    #[test]
    fn goose_metric_all_variant_round_trips() {
        let request = make_test_request_metric();
        let transaction = TransactionMetric::new(0, 0, 0, Arc::from("tx"), 0);
        let scenario = ScenarioMetric::new(
            0,
            Arc::from("TestScenario"),
            0,
            Duration::from_millis(100),
            0,
        );

        let metric = GooseMetric::All {
            request: Box::new(request.clone()),
            transaction: transaction.clone(),
            scenario: Box::new(scenario.clone()),
        };

        // Pattern-match to extract all three payloads.
        match metric {
            GooseMetric::All {
                request: r,
                transaction: t,
                scenario: s,
            } => {
                assert_eq!(r.name.as_ref(), "/");
                assert_eq!(&*t.name, "tx");
                assert_eq!(&*s.name, "TestScenario");
                assert_eq!(s.run_time, 100);
            }
            _ => panic!("expected GooseMetric::All"),
        }
    }

    /// The microsecond fractions fit in padding, so a request metric is no
    /// larger than before them, and a channel message stays small.
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn metric_sizes_stay_small() {
        assert_eq!(std::mem::size_of::<GooseRequestMetric>(), 272);
        assert_eq!(std::mem::size_of::<TransactionMetric>(), 64);
        assert!(std::mem::size_of::<GooseMetric>() <= 120);
    }

    #[test]
    fn goose_metric_all_sent_on_channel() {
        // Verify that GooseMetric::All can be sent and received on a flume channel
        // just like the individual variants.
        let (tx, rx) = flume::unbounded::<GooseMetric>();

        let request = make_test_request_metric();
        let transaction = TransactionMetric::new(0, 0, 0, Arc::from("tx"), 0);
        let scenario = ScenarioMetric::new(
            0,
            Arc::from("TestScenario"),
            0,
            Duration::from_millis(100),
            0,
        );

        // Send one All message (replaces what would have been 3 individual sends).
        tx.send(GooseMetric::All {
            request: Box::new(request),
            transaction,
            scenario: Box::new(scenario),
        })
        .unwrap();

        // Only one message on the channel.
        let msg = rx.try_recv().unwrap();
        assert!(matches!(msg, GooseMetric::All { .. }));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn goose_metric_all_serialization() {
        // Verify GooseMetric::All round-trips through serde (metrics are serialized
        // for logging and Gaggle communication).
        let request = make_test_request_metric();
        let transaction = TransactionMetric::new(0, 0, 0, Arc::from("tx"), 0);
        let scenario = ScenarioMetric::new(
            0,
            Arc::from("TestScenario"),
            0,
            Duration::from_millis(100),
            0,
        );

        let metric = GooseMetric::All {
            request: Box::new(request),
            transaction,
            scenario: Box::new(scenario),
        };

        let json = serde_json::to_string(&metric).expect("serialize");
        let deserialized: GooseMetric = serde_json::from_str(&json).expect("deserialize");
        assert!(matches!(deserialized, GooseMetric::All { .. }));
    }

    /// Build a minimal MetricsProcessor for unit tests of process_* paths.
    fn make_dashboard_test_processor(configuration: GooseConfiguration) -> MetricsProcessor {
        let keep_full_graph = configuration.keeps_full_graph(configuration.save_report_at_start());
        let (_metrics_tx, metrics_rx) = flume::unbounded();
        let (_cmd_tx, cmd_rx) = flume::unbounded();
        MetricsProcessor::new(
            metrics_rx,
            cmd_rx,
            GooseMetrics::default(),
            GraphData::new(),
            configuration,
            Vec::new(),
            GooseDefaults::default(),
            Arc::new(Mutex::new(HashMap::new())),
            None,
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
            keep_full_graph,
        )
    }

    #[test]
    fn graph_data_recorded_when_dashboard_enabled_without_report_file() {
        // Regression: --dashboard alone must collect GraphData series so charts
        // are non-empty even when --report-file is not set.
        let configuration = GooseConfiguration {
            dashboard: true,
            report_file: Vec::new(),
            ..Default::default()
        };
        assert!(configuration.report_file.is_empty());

        let mut processor = make_dashboard_test_processor(configuration);

        let mut request = make_test_request_metric();
        request.elapsed = 2_500; // second 2
        request.response_time = 42;
        request.success = true;
        processor.process_request_metric(&request);

        let mut request2 = make_test_request_metric();
        request2.elapsed = 3_100; // second 3
        request2.response_time = 50;
        request2.success = false;
        request2.error = "boom".to_string();
        processor.process_request_metric(&request2);

        let window = processor.graph_data.export_series_window(300);
        assert!(
            !window.rps.is_empty(),
            "series export must be non-empty when dashboard is on"
        );
        assert!(
            window.rps.iter().any(|&v| v > 0.0),
            "expected at least one non-zero RPS bucket, got {:?}",
            window.rps
        );
        assert!(
            window.fps.iter().any(|&v| v > 0.0),
            "expected at least one non-zero error bucket from failed request"
        );
        assert!(
            window.avg_latency_ms.iter().any(|&v| v > 0.0),
            "expected non-zero latency samples"
        );
    }

    #[test]
    fn graph_data_not_recorded_when_dashboard_and_report_file_off() {
        let configuration = GooseConfiguration {
            dashboard: false,
            report_file: Vec::new(),
            ..Default::default()
        };

        let mut processor = make_dashboard_test_processor(configuration);
        let mut request = make_test_request_metric();
        request.elapsed = 1_000;
        request.response_time = 10;
        processor.process_request_metric(&request);

        let window = processor.graph_data.export_series_window(300);
        assert!(
            window.rps.is_empty(),
            "series must stay empty when neither report_file nor dashboard is set"
        );
    }

    #[test]
    fn graph_data_pruned_when_dashboard_only_without_report_file() {
        // Multi-hour soaks with --dashboard (no --report-file) must not retain
        // unbounded per-second series; keep 2× SERIES_WINDOW_SECS max.
        // Pruning runs from low-frequency RecordUsers / snapshot, not per request.
        // Save report keeps the full series, so this turns it off.
        let configuration = GooseConfiguration {
            dashboard: true,
            report_file: Vec::new(),
            ..Default::default()
        };
        let mut processor = make_dashboard_test_processor(configuration);
        processor.keep_full_graph = false;

        let keep = dashboard_snapshot::SERIES_WINDOW_SECS.saturating_mul(2) as usize;
        // Record past the keep window so pruning must drop early samples.
        let far_second = keep + 120;
        for sec in 0..=far_second {
            let mut request = make_test_request_metric();
            request.elapsed = (sec as u64) * 1000;
            request.response_time = 10;
            request.success = true;
            processor.process_request_metric(&request);
        }

        // Without a prune trigger, series may still be long (hot path no longer prunes).
        assert!(
            processor.graph_data.max_series_len_for_test() > keep,
            "expected growth before low-frequency prune"
        );

        // RecordUsers (~1 Hz in production) is the prune cadence.
        let shutdown = processor.handle_command(MetricsCommand::RecordUsers {
            active_users: 1,
            elapsed_secs: far_second,
        });
        assert!(!shutdown);

        // Export still sees recent absolute seconds (origin advanced, data bounded).
        let window = processor.graph_data.export_series_window(300);
        assert!(!window.rps.is_empty());
        assert!(
            window.start_second as usize >= far_second.saturating_sub(299),
            "export should cover the trailing window near second {far_second}, got start {}",
            window.start_second
        );
        assert!(
            processor.graph_data.max_series_len_for_test() <= keep,
            "dashboard-only GraphData must stay within keep window ({keep}), got {}",
            processor.graph_data.max_series_len_for_test()
        );
        // Early absolute second must no longer hold data (pruned away).
        let early = processor.graph_data.export_series_window(300);
        // With end near far_second, a 300s export starts near far_second-299, not 0.
        assert!(early.start_second > 0);
    }

    #[test]
    fn graph_data_not_pruned_when_report_file_set() {
        let configuration = GooseConfiguration {
            dashboard: true,
            report_file: vec!["report.html".to_string()],
            ..Default::default()
        };
        let mut processor = make_dashboard_test_processor(configuration);

        let far_second = dashboard_snapshot::SERIES_WINDOW_SECS as usize * 2 + 200;
        for sec in 0..=far_second {
            let mut request = make_test_request_metric();
            request.elapsed = (sec as u64) * 1000;
            request.response_time = 5;
            processor.process_request_metric(&request);
        }
        let shutdown = processor.handle_command(MetricsCommand::RecordUsers {
            active_users: 1,
            elapsed_secs: far_second,
        });
        assert!(!shutdown);

        // Full history retained for HTML report graphs.
        assert!(
            processor.graph_data.max_series_len_for_test() > far_second,
            "report_file path must retain full series, got len {}",
            processor.graph_data.max_series_len_for_test()
        );
    }

    #[test]
    fn graph_data_not_pruned_with_save_report_on() {
        // The dashboard's default: Save report is on, so the report needs every
        // second even without --report-file.
        let configuration = GooseConfiguration {
            dashboard: true,
            report_file: Vec::new(),
            ..Default::default()
        };
        assert!(configuration.save_report_at_start());
        let mut processor = make_dashboard_test_processor(configuration);

        let far_second = dashboard_snapshot::SERIES_WINDOW_SECS as usize * 2 + 200;
        for sec in 0..=far_second {
            let mut request = make_test_request_metric();
            request.elapsed = (sec as u64) * 1000;
            request.response_time = 5;
            processor.process_request_metric(&request);
        }
        let shutdown = processor.handle_command(MetricsCommand::RecordUsers {
            active_users: 1,
            elapsed_secs: far_second,
        });
        assert!(!shutdown);
        assert!(
            processor.graph_data.max_series_len_for_test() > far_second,
            "Save report must retain the full series, got len {}",
            processor.graph_data.max_series_len_for_test()
        );
    }

    /// Save report turned on during a run: the totals still cover the whole
    /// run, the graphs keep from then on plus the dashboard's window before
    /// it, and the heatmap gets whole run columns; turned off, the window
    /// alone again. Save report now's copy is the same, and leaves the
    /// processor's own data alone.
    #[tokio::test]
    async fn save_report_turned_on_during_a_run() {
        let configuration = GooseConfiguration {
            dashboard: true,
            ..Default::default()
        };
        let mut processor = make_graph_test_processor(&configuration);
        processor.keep_full_graph = false;
        processor.graph_data.keep_latency_columns(false, 0);
        let keep = dashboard_snapshot::SERIES_WINDOW_SECS as usize * 2;
        let record = |processor: &mut MetricsProcessor, from: usize, to: usize| {
            for sec in from..to {
                let mut request = make_test_request_metric();
                request.elapsed = sec as u64 * 1000 + 500;
                request.response_time = 5;
                processor.process_request_metric(&request);
                let _ = processor.handle_command(MetricsCommand::RecordUsers {
                    active_users: 1,
                    elapsed_secs: sec,
                });
            }
        };
        record(&mut processor, 0, keep + 300);
        let pruned_to = processor.graph_data.first_second();
        assert!(pruned_to > 0, "Save report off keeps only the window");

        // On: nothing more is dropped from here.
        processor.handle_command(MetricsCommand::KeepGraph {
            keep: true,
            now: keep + 300,
        });
        assert!(processor.graph_data.has_latency_columns_for_test());
        // The heatmap takes the seconds the rings held before the switch.
        assert!(!processor
            .graph_data
            .get_latency_heatmap_markup(false, chrono::Utc::now())
            .is_empty());
        record(&mut processor, keep + 300, 2 * keep + 300);
        assert_eq!(processor.graph_data.first_second(), pruned_to);

        let (respond, rx) = tokio::sync::oneshot::channel();
        processor.handle_command(MetricsCommand::GetReportData {
            duration: 2 * keep + 300,
            total_users: 1,
            maximum_users: 1,
            history: Vec::new(),
            respond,
        });
        let (metrics, graph_data) = rx.await.unwrap();
        let timed: usize = metrics.requests.values().map(|r| r.raw_data.counter).sum();
        assert_eq!(timed, 2 * keep + 300, "the totals cover the whole run");
        assert_eq!(graph_data.first_second(), pruned_to);
        assert!(!graph_data
            .get_latency_heatmap_markup(false, chrono::Utc::now())
            .is_empty());

        // Off again: back to the window, and no heatmap columns.
        processor.handle_command(MetricsCommand::KeepGraph {
            keep: false,
            now: 2 * keep + 300,
        });
        assert!(!processor.graph_data.has_latency_columns_for_test());
        record(&mut processor, 2 * keep + 300, 2 * keep + 400);
        assert!(processor.graph_data.first_second() > pruned_to);
    }

    #[tokio::test]
    async fn get_dashboard_snapshot_command_builds_snapshot() {
        let configuration = GooseConfiguration {
            dashboard: true,
            ..Default::default()
        };

        let mut processor = make_dashboard_test_processor(configuration);
        let mut request = make_test_request_metric();
        request.elapsed = 1_500;
        request.response_time = 25;
        request.success = true;
        processor.process_request_metric(&request);
        // Simulate atomic success counter so aggregate totals are non-zero.
        {
            let key = format!("{} {}", request.raw.method_label(), request.name);
            if let Some(agg) = processor.metrics.requests.get_mut(&key) {
                agg.success_count = 1;
            }
        }
        processor
            .metrics
            .hosts
            .insert("http://localhost/".to_string());

        let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();
        let shutdown = processor.handle_command(MetricsCommand::GetDashboardSnapshot {
            duration: 10,
            total_users: 5,
            maximum_users: 10,
            active_users: 3,
            target_users: 8,
            // series clock "now" past the request sample at second 1
            series_elapsed_secs: 5,
            phase: "maintain".to_string(),
            stopping: true,
            series_window_secs: dashboard_snapshot::SERIES_WINDOW_SECS,
            report: dashboard_snapshot::SnapshotReport::default(),
            settings: Box::new(dashboard_snapshot::SnapshotSettings::idle()),
            heatmap_since: None,
            respond: respond_tx,
        });
        assert!(!shutdown);

        let snapshot = respond_rx.await.expect("snapshot response");
        assert_eq!(snapshot.version, 1);
        assert_eq!(snapshot.phase, "maintain");
        assert!(snapshot.stopping);
        assert_eq!(snapshot.duration_secs, 10);
        assert_eq!(snapshot.active_users, 3);
        assert_eq!(snapshot.maximum_users, 10);
        assert_eq!(snapshot.target_users, 8);
        assert_eq!(snapshot.total_users, 5);
        assert_eq!(snapshot.aggregate.total_requests, 1);
        assert!(!snapshot.series.rps.is_empty());
        // Users series must extend to series_elapsed and hold live active_users.
        assert!(
            snapshot.series.users.len() >= 6,
            "expected users through second 5, got len {}",
            snapshot.series.users.len()
        );
        assert_eq!(
            *snapshot.series.users.last().unwrap(),
            3,
            "right edge must match live active_users"
        );
        assert_eq!(
            snapshot.flags.series_seconds,
            dashboard_snapshot::SERIES_WINDOW_SECS
        );
        assert_eq!(snapshot.hosts, vec!["http://localhost/".to_string()]);
    }

    /// A metrics reset restarts the report heatmap's columns at the reset's
    /// second on the series clock: what came before is gone, and a request
    /// that started in the second before the reset and finished after it is
    /// counted once, in column 0, as the report's totals count it (#746).
    #[test]
    fn reset_restarts_the_report_heatmap_columns() {
        let configuration = GooseConfiguration {
            report_file: vec!["report.html".to_string()],
            ..Default::default()
        };
        assert!(configuration.records_latency_columns());
        let mut processor = make_dashboard_test_processor(configuration.clone());
        processor.graph_data = GraphData::for_metrics_processor(
            configuration.records_latency_series(),
            configuration.records_synthetic_latency(),
            configuration.records_latency_columns(),
        );
        for elapsed in [1_200, 4_500, 8_000] {
            processor.process_request_metric(&make_request_metric("/", 10, 200, elapsed));
        }
        let (ack, _ack_rx) = tokio::sync::oneshot::channel();
        // The run's duration rounds on another clock: the columns start from
        // the series second, not from it.
        processor.handle_command(MetricsCommand::Reset {
            duration: 7,
            total_users: 1,
            maximum_users: 1,
            active_users: 1,
            series_elapsed_secs: 10,
            ack,
        });
        // In flight at the reset: started at 9.8 s, finished after it.
        processor.process_request_metric(&make_request_metric("/", 400, 200, 9_800));
        processor.process_request_metric(&make_request_metric("/", 10, 200, 12_100));
        let (respond, mut respond_rx) = tokio::sync::oneshot::channel();
        processor.handle_command(MetricsCommand::Shutdown {
            duration: 13,
            total_users: 1,
            maximum_users: 1,
            respond,
        });
        let (metrics, graph_data) = respond_rx.try_recv().unwrap();
        assert_eq!(metrics.requests["GET /"].raw_data.counter, 2);
        for key in [None, Some("GET /")] {
            assert_eq!(
                graph_data.latency_columns_for_test(key),
                Some((1, vec![(0, 1), (2, 1)])),
                "{:?}",
                key
            );
        }
    }

    #[test]
    fn reset_preserves_active_users_ramp_history() {
        // Regression: metrics reset used to wipe users and reseed second 0 with
        // the final count, so the dashboard looked like the test started at full
        // load. Ramp history must survive reset_rate_series.
        let configuration = GooseConfiguration {
            dashboard: true,
            ..Default::default()
        };
        let mut processor = make_dashboard_test_processor(configuration);

        // Simulated ramp 1 → 5 over seconds 0..4.
        for (s, users) in (0..=4).zip(1..=5) {
            processor.graph_data.record_users_per_second(users, s);
        }
        // Steady at 5 through second 10.
        for s in 5..=10 {
            processor.graph_data.record_users_per_second(5, s);
        }
        // Some RPS during ramp (will be cleared on reset).
        for s in 0..=10 {
            processor.graph_data.record_requests_per_second("GET /", s);
        }

        let (ack_tx, _ack_rx) = tokio::sync::oneshot::channel();
        let shutdown = processor.handle_command(MetricsCommand::Reset {
            duration: 10,
            total_users: 5,
            maximum_users: 5,
            active_users: 5,
            series_elapsed_secs: 10,
            ack: ack_tx,
        });
        assert!(!shutdown);

        // Rate series cleared.
        let after_reset = processor.graph_data.export_series_window(300);
        assert!(
            after_reset.rps.iter().all(|&v| v == 0.0),
            "RPS must be cleared on metrics reset, got {:?}",
            after_reset.rps
        );

        // Users ramp still present.
        assert_eq!(processor.graph_data.users_last_absolute_second(), Some(10));
        let users_window = processor.graph_data.export_series_window(300);
        assert_eq!(users_window.users.len(), 11);
        assert_eq!(users_window.users[0], 1);
        assert_eq!(users_window.users[1], 2);
        assert_eq!(users_window.users[2], 3);
        assert_eq!(users_window.users[3], 4);
        assert_eq!(users_window.users[4], 5);
        for s in 5..=10 {
            assert_eq!(users_window.users[s], 5, "steady users[{s}]");
        }

        // Post-reset recording continues on the same clock (no restart at 0).
        processor.graph_data.record_users_per_second(8, 15);
        let continued = processor.graph_data.export_series_window(300);
        assert_eq!(continued.users[10], 5);
        // Gap 11..14 held at 5, second 15 = 8.
        assert_eq!(continued.users[14], 5);
        assert_eq!(continued.users[15], 8);
    }

    // --- Batch metrics tests ---

    /// Build a minimal GooseRequestMetric with the given response time and status code.
    fn make_request_metric(
        path: &str,
        response_time: u64,
        status_code: u16,
        elapsed: u64,
    ) -> GooseRequestMetric {
        let raw = GooseRawRequest::new(GooseMethod::Get, "http://localhost/", vec![], "");
        let mut metric = GooseRequestMetric::new(
            raw,
            TransactionDetail {
                scenario_index: 0,
                scenario_name: Arc::from("TestScenario"),
                transaction_index: Some(0),
                transaction_name: TransactionName::InheritNameByRequests(Arc::from("tx")),
            },
            path,
            0,
            0,
        );
        metric.response_time = response_time;
        metric.status_code = status_code;
        metric.elapsed = elapsed;
        metric.success = true;
        metric
    }

    /// Build a MetricsProcessor suitable for unit tests.
    ///
    /// Sets up the minimum required state: one scenario with one transaction,
    /// empty configuration, and the provided epoch.
    fn make_test_processor(epoch: u64) -> MetricsProcessor {
        let (_metrics_tx, metrics_rx) = flume::unbounded::<GooseMetric>();
        let (_cmd_tx, cmd_rx) = flume::unbounded::<MetricsCommand>();

        let mut metrics = GooseMetrics::default();
        // Set up one scenario with one transaction so batch indexing works.
        metrics.transactions = vec![vec![TransactionMetricAggregate::new(
            0,
            "TestScenario".into(),
            0,
            TransactionName::InheritNameByRequests(Arc::from("tx")),
        )]];
        metrics.scenarios = vec![ScenarioMetricAggregate::new(0, "TestScenario".into())];

        let metrics_epoch: MetricsEpoch = Arc::new(std::sync::atomic::AtomicU64::new(epoch));

        MetricsProcessor::new(
            metrics_rx,
            cmd_rx,
            metrics,
            crate::graph::GraphData::new(),
            GooseConfiguration::default(),
            vec![],
            GooseDefaults::default(),
            Arc::new(Mutex::new(HashMap::new())),
            None,
            metrics_epoch,
            false,
        )
    }

    #[test]
    fn batch_new_has_correct_epoch() {
        let batch = GooseMetricBatch::new(42);
        assert_eq!(batch.epoch, 42);
        assert!(batch.requests.is_empty());
        assert!(batch.individual_requests.is_empty());
        assert!(batch.transactions.is_empty());
        assert!(batch.scenarios.is_empty());
        assert!(batch.second.is_none());
        assert!(batch.graph_tps.is_empty());
        assert!(batch.graph_sps.is_empty());
    }

    #[test]
    fn batch_serialization_round_trip() {
        let mut batch = GooseMetricBatch::new(1);
        batch.requests.insert(
            "GET /".to_string(),
            RequestBatchEntry {
                path: "/".to_string(),
                method: GooseMethod::Get,
                custom_method: String::new(),
                raw_data: GooseRequestMetricTimingData::new(None),
                status_code_counts: HashMap::new(),
                status_code_timings: HashMap::new(),
            },
        );
        batch
            .individual_requests
            .push(make_request_metric("/err", 50, 500, 1000));

        let json = serde_json::to_string(&GooseMetric::Batch(Box::new(batch))).expect("serialize");
        let deserialized: GooseMetric = serde_json::from_str(&json).expect("deserialize");
        assert!(matches!(deserialized, GooseMetric::Batch(_)));
    }

    #[test]
    fn batch_sent_on_channel() {
        let (tx, rx) = flume::unbounded::<GooseMetric>();
        let batch = GooseMetricBatch::new(0);
        tx.send(GooseMetric::Batch(Box::new(batch))).unwrap();

        let msg = rx.try_recv().unwrap();
        assert!(matches!(msg, GooseMetric::Batch(_)));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn process_batch_merges_request_timing() {
        let mut processor = make_test_processor(0);

        // Build a batch with pre-aggregated request data.
        let mut batch = GooseMetricBatch::new(0);
        let mut raw_data = GooseRequestMetricTimingData::new(None);
        raw_data.record_time(5);
        raw_data.record_time(10);
        raw_data.record_time(15);

        batch.requests.insert(
            "GET /".to_string(),
            RequestBatchEntry {
                path: "/".to_string(),
                method: GooseMethod::Get,
                custom_method: String::new(),
                raw_data,
                status_code_counts: HashMap::from([(200, 3)]),
                status_code_timings: HashMap::from([(
                    200,
                    StatusCodeTimingSummary {
                        count: 3,
                        total_time_us: 30_000,
                        min_time_us: 5_000,
                        max_time_us: 15_000,
                    },
                )]),
            },
        );

        processor.process_batch(batch);

        let agg = &processor.metrics.requests["GET /"];
        assert_eq!(agg.raw_data.counter, 3);
        assert_eq!(agg.raw_data.total_time_us, 30_000);
        assert_eq!(agg.raw_data.minimum_time_us, 5_000);
        assert_eq!(agg.raw_data.maximum_time_us, 15_000);
        assert_eq!(agg.status_code_counts[&200], 3);
        assert_eq!(agg.status_code_timings[&200].count, 3);
        assert_eq!(agg.status_code_timings[&200].total_time_us, 30_000);
        assert_eq!(agg.status_code_timings[&200].min_time_us, 5_000);
        assert_eq!(agg.status_code_timings[&200].max_time_us, 15_000);
    }

    #[test]
    fn process_batch_merges_batched_requests_into_co_adjusted_times() {
        let mut processor = make_test_processor(0);

        // Ten real times of 10 ms, then one synthetic time of 5000 ms, which
        // starts the coordinated omission adjusted data.
        let mut aggregate = GooseRequestMetricAggregate::new("/", GooseMethod::Get, 0, "");
        for _ in 0..10 {
            aggregate.record_time(10, false);
        }
        aggregate.record_time(5000, true);
        processor
            .metrics
            .requests
            .insert("GET /".to_string(), aggregate);

        // Then a batch of 100 successful requests at 10 ms.
        let mut raw_data = GooseRequestMetricTimingData::new(None);
        for _ in 0..100 {
            raw_data.record_time(10);
        }
        let mut batch = GooseMetricBatch::new(0);
        batch.requests.insert(
            "GET /".to_string(),
            RequestBatchEntry {
                path: "/".to_string(),
                method: GooseMethod::Get,
                custom_method: String::new(),
                raw_data,
                status_code_counts: HashMap::from([(200, 100)]),
                status_code_timings: HashMap::new(),
            },
        );
        processor.process_batch(batch);

        let agg = &processor.metrics.requests["GET /"];
        assert_eq!(agg.raw_data.counter, 110);
        let co_data = agg.coordinated_omission_data.as_ref().unwrap();
        // Before #739 was fixed the batch never reached the adjusted data: 11.
        assert_eq!(co_data.counter, 111);
        assert_eq!(co_data.total_time_us, (110 * 10 + 5000) * 1000);
        assert_eq!(co_data.minimum_time_us, 10_000);
        assert_eq!(co_data.maximum_time_us, 5_000_000);
        // Before the fix the adjusted p99 was the synthetic 5000 ms.
        let p99 = co_data.percentile_us(0.99) / 1000;
        assert_eq!(p99, 10);
    }

    #[test]
    fn process_batch_merges_multiple_batches() {
        let mut processor = make_test_processor(0);

        // First batch: 2 requests at 10ms and 20ms.
        let mut batch1 = GooseMetricBatch::new(0);
        let mut raw1 = GooseRequestMetricTimingData::new(None);
        raw1.record_time(10);
        raw1.record_time(20);
        batch1.requests.insert(
            "GET /api".to_string(),
            RequestBatchEntry {
                path: "/api".to_string(),
                method: GooseMethod::Get,
                custom_method: String::new(),
                raw_data: raw1,
                status_code_counts: HashMap::from([(200, 2)]),
                status_code_timings: HashMap::new(),
            },
        );
        processor.process_batch(batch1);

        // Second batch: 1 request at 5ms (new minimum).
        let mut batch2 = GooseMetricBatch::new(0);
        let mut raw2 = GooseRequestMetricTimingData::new(None);
        raw2.record_time(5);
        batch2.requests.insert(
            "GET /api".to_string(),
            RequestBatchEntry {
                path: "/api".to_string(),
                method: GooseMethod::Get,
                custom_method: String::new(),
                raw_data: raw2,
                status_code_counts: HashMap::from([(200, 1)]),
                status_code_timings: HashMap::new(),
            },
        );
        processor.process_batch(batch2);

        let agg = &processor.metrics.requests["GET /api"];
        assert_eq!(agg.raw_data.counter, 3);
        assert_eq!(agg.raw_data.total_time_us, 35_000);
        assert_eq!(agg.raw_data.minimum_time_us, 5_000);
        assert_eq!(agg.raw_data.maximum_time_us, 20_000);
        assert_eq!(agg.status_code_counts[&200], 3);
    }

    #[test]
    fn process_batch_handles_individual_requests() {
        let mut processor = make_test_processor(0);

        let mut batch = GooseMetricBatch::new(0);
        let mut req = make_request_metric("/fail", 100, 500, 5000);
        req.error = "server error".to_string();
        req.success = false;
        batch.individual_requests.push(req);

        processor.process_batch(batch);

        // Individual requests go through process_request_metric, which records
        // them into the aggregate just like non-batched metrics.
        assert!(processor.metrics.requests.contains_key("GET /fail"));
        let agg = &processor.metrics.requests["GET /fail"];
        assert_eq!(agg.raw_data.counter, 1);
        assert_eq!(agg.raw_data.total_time_us, 100_000);
        // Error should be recorded.
        assert!(!processor.metrics.errors.is_empty());
    }

    /// Timing data holding these whole millisecond times.
    fn timing_data(times: &[u64]) -> GooseRequestMetricTimingData {
        let mut data = GooseRequestMetricTimingData::default();
        for &time in times {
            data.record_time(time);
        }
        data
    }

    #[test]
    fn process_batch_merges_transaction_timing() {
        let mut processor = make_test_processor(0);

        let mut batch = GooseMetricBatch::new(0);
        batch.transactions.insert(
            (0, 0),
            TransactionBatchEntry {
                run_times: timing_data(&[10, 10, 20]),
                success_count: 2,
                fail_count: 1,
            },
        );

        processor.process_batch(batch);

        let agg = &processor.metrics.transactions[0][0];
        assert_eq!(agg.run_times.counter, 3);
        assert_eq!(agg.run_times.total_time_us, 40_000);
        assert_eq!(agg.run_times.minimum_time_us, 10_000);
        assert_eq!(agg.run_times.maximum_time_us, 20_000);
        assert_eq!(agg.success_count, 2);
        assert_eq!(agg.fail_count, 1);
        assert_eq!(agg.run_times.times_us[&buckets::bucket_floor(10_000)], 2);
        assert_eq!(agg.run_times.times_us[&buckets::bucket_floor(20_000)], 1);
    }

    #[test]
    fn process_batch_merges_scenario_timing() {
        let mut processor = make_test_processor(0);

        let mut batch = GooseMetricBatch::new(0);
        batch.scenarios.insert(
            0,
            ScenarioBatchEntry {
                run_times: timing_data(&[50, 75, 75]),
                user: 0,
            },
        );

        processor.process_batch(batch);

        let agg = &processor.metrics.scenarios[0];
        assert_eq!(agg.run_times.counter, 3);
        assert_eq!(agg.run_times.total_time_us, 200_000);
        assert_eq!(agg.run_times.minimum_time_us, 50_000);
        assert_eq!(agg.run_times.maximum_time_us, 75_000);
        assert_eq!(agg.run_times.times_us[&buckets::bucket_floor(50_000)], 1);
        assert_eq!(agg.run_times.times_us[&buckets::bucket_floor(75_000)], 2);
        assert!(agg.users.contains(&0));
    }

    #[test]
    fn process_batch_discards_stale_epoch() {
        // Processor at epoch 2; batch stamped with epoch 1 should be discarded.
        let mut processor = make_test_processor(2);

        let mut batch = GooseMetricBatch::new(1); // stale epoch
        let mut raw_data = GooseRequestMetricTimingData::new(None);
        raw_data.record_time(99);
        batch.requests.insert(
            "GET /stale".to_string(),
            RequestBatchEntry {
                path: "/stale".to_string(),
                method: GooseMethod::Get,
                custom_method: String::new(),
                raw_data,
                status_code_counts: HashMap::new(),
                status_code_timings: HashMap::new(),
            },
        );

        processor.process_batch(batch);

        // Nothing should be recorded because the batch was stale.
        assert!(
            !processor.metrics.requests.contains_key("GET /stale"),
            "stale batch data should be discarded"
        );
    }

    #[test]
    fn process_batch_accepts_current_epoch() {
        let mut processor = make_test_processor(3);

        let mut batch = GooseMetricBatch::new(3); // matches current epoch
        let mut raw_data = GooseRequestMetricTimingData::new(None);
        raw_data.record_time(42);
        batch.requests.insert(
            "GET /current".to_string(),
            RequestBatchEntry {
                path: "/current".to_string(),
                method: GooseMethod::Get,
                custom_method: String::new(),
                raw_data,
                status_code_counts: HashMap::new(),
                status_code_timings: HashMap::new(),
            },
        );

        processor.process_batch(batch);

        assert!(processor.metrics.requests.contains_key("GET /current"));
        assert_eq!(
            processor.metrics.requests["GET /current"].raw_data.counter,
            1
        );
    }

    #[test]
    fn process_batch_produces_same_result_as_individual() {
        // Process 3 request metrics individually, then process the same 3 as a
        // pre-aggregated batch. The resulting aggregates should be identical.
        let requests = vec![
            make_request_metric("/", 5, 200, 1000),
            make_request_metric("/", 15, 200, 2000),
            make_request_metric("/", 10, 200, 3000),
        ];

        // --- Individual path ---
        let mut proc_individual = make_test_processor(0);
        for req in &requests {
            proc_individual.process_request_metric(req);
        }

        // --- Batch path ---
        let mut proc_batch = make_test_processor(0);
        let mut batch = GooseMetricBatch::new(0);
        let mut raw_data = GooseRequestMetricTimingData::new(None);
        for req in &requests {
            raw_data.record_time(req.response_time);
        }
        let mut status_code_counts = HashMap::new();
        let mut status_code_timings = HashMap::new();
        for req in &requests {
            *status_code_counts.entry(req.status_code).or_insert(0) += 1;
            let summary = status_code_timings
                .entry(req.status_code)
                .or_insert_with(StatusCodeTimingSummary::default);
            let time = req.response_time_micros();
            summary.count += 1;
            summary.total_time_us += time;
            if time > 0 && (summary.min_time_us == 0 || time < summary.min_time_us) {
                summary.min_time_us = time;
            }
            if time > summary.max_time_us {
                summary.max_time_us = time;
            }
        }
        batch.requests.insert(
            "GET /".to_string(),
            RequestBatchEntry {
                path: "/".to_string(),
                method: GooseMethod::Get,
                custom_method: String::new(),
                raw_data,
                status_code_counts,
                status_code_timings,
            },
        );
        proc_batch.process_batch(batch);

        // Compare aggregates.
        let ind = &proc_individual.metrics.requests["GET /"];
        let bat = &proc_batch.metrics.requests["GET /"];
        assert_eq!(ind.raw_data.counter, bat.raw_data.counter);
        assert_eq!(ind.raw_data.total_time_us, bat.raw_data.total_time_us);
        assert_eq!(ind.raw_data.minimum_time_us, bat.raw_data.minimum_time_us);
        assert_eq!(ind.raw_data.maximum_time_us, bat.raw_data.maximum_time_us);
        assert_eq!(ind.raw_data.times_us, bat.raw_data.times_us);
        assert_eq!(ind.status_code_counts, bat.status_code_counts);
        assert_eq!(
            ind.status_code_timings[&200].count,
            bat.status_code_timings[&200].count
        );
        assert_eq!(
            ind.status_code_timings[&200].total_time_us,
            bat.status_code_timings[&200].total_time_us
        );
        assert_eq!(
            ind.status_code_timings[&200].min_time_us,
            bat.status_code_timings[&200].min_time_us
        );
        assert_eq!(
            ind.status_code_timings[&200].max_time_us,
            bat.status_code_timings[&200].max_time_us
        );
    }

    #[test]
    fn process_batch_graph_data_with_report_file() {
        // Enable report file so graph data is recorded.
        // Compare the batch path to individual processing to verify they produce
        // the same graph data.
        let response_times: Vec<u64> = vec![10, 20, 30];

        // --- Individual path: feed each request one-by-one ---
        let (_metrics_tx1, metrics_rx1) = flume::unbounded::<GooseMetric>();
        let (_cmd_tx1, cmd_rx1) = flume::unbounded::<MetricsCommand>();
        let mut config1 = GooseConfiguration::default();
        config1.report_file = vec!["test.html".to_string()];
        let mut metrics1 = GooseMetrics::default();
        metrics1.transactions = vec![vec![TransactionMetricAggregate::new(
            0,
            "TestScenario".into(),
            0,
            TransactionName::InheritNameByRequests(Arc::from("tx")),
        )]];
        metrics1.scenarios = vec![ScenarioMetricAggregate::new(0, "TestScenario".into())];
        let mut proc_individual = MetricsProcessor::new(
            metrics_rx1,
            cmd_rx1,
            metrics1,
            crate::graph::GraphData::new(),
            config1,
            vec![],
            GooseDefaults::default(),
            Arc::new(Mutex::new(HashMap::new())),
            None,
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
            false,
        );
        for &rt in &response_times {
            let req = make_request_metric("/", rt, 200, 5000);
            proc_individual.process_request_metric(&req);
        }

        // --- Batch path: same data in a single batch ---
        let (_metrics_tx2, metrics_rx2) = flume::unbounded::<GooseMetric>();
        let (_cmd_tx2, cmd_rx2) = flume::unbounded::<MetricsCommand>();
        let mut config2 = GooseConfiguration::default();
        config2.report_file = vec!["test.html".to_string()];
        let mut metrics2 = GooseMetrics::default();
        metrics2.transactions = vec![vec![TransactionMetricAggregate::new(
            0,
            "TestScenario".into(),
            0,
            TransactionName::InheritNameByRequests(Arc::from("tx")),
        )]];
        metrics2.scenarios = vec![ScenarioMetricAggregate::new(0, "TestScenario".into())];
        let mut proc_batch = MetricsProcessor::new(
            metrics_rx2,
            cmd_rx2,
            metrics2,
            crate::graph::GraphData::new(),
            config2,
            vec![],
            GooseDefaults::default(),
            Arc::new(Mutex::new(HashMap::new())),
            None,
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
            false,
        );
        let mut batch = GooseMetricBatch::new(0);
        batch.second = Some(5);
        // Pre-aggregate request timing data too, so both paths match.
        let mut raw_data = GooseRequestMetricTimingData::new(None);
        for &rt in &response_times {
            raw_data.record_time(rt);
        }
        let mut sc_counts = HashMap::new();
        *sc_counts.entry(200u16).or_insert(0usize) += response_times.len();
        batch.requests.insert(
            "GET /".to_string(),
            RequestBatchEntry {
                path: "/".to_string(),
                method: GooseMethod::Get,
                custom_method: String::new(),
                raw_data,
                status_code_counts: sc_counts,
                status_code_timings: HashMap::new(),
            },
        );
        proc_batch.process_batch(batch);

        // Compare request aggregates (timing data should match exactly).
        let ind_agg = &proc_individual.metrics.requests["GET /"];
        let bat_agg = &proc_batch.metrics.requests["GET /"];
        assert_eq!(ind_agg.raw_data.counter, bat_agg.raw_data.counter);
        assert_eq!(
            ind_agg.raw_data.total_time_us,
            bat_agg.raw_data.total_time_us
        );
        assert_eq!(
            ind_agg.raw_data.minimum_time_us,
            bat_agg.raw_data.minimum_time_us
        );
        assert_eq!(
            ind_agg.raw_data.maximum_time_us,
            bat_agg.raw_data.maximum_time_us
        );
    }

    /// Batched requests must feed the dashboard charts like the per-request
    /// path: recorded with `--dashboard` and no report file, and stored relative
    /// to the series origin after dashboard pruning has advanced it.
    #[test]
    fn process_batch_graph_data_dashboard_after_prune() {
        // Accumulate (elapsed ms, response time ms) requests through the user
        // side batch path, then apply the flushed batch on the parent side.
        fn apply_batch(
            user: &mut crate::goose::GooseUser,
            rx: &flume::Receiver<GooseMetric>,
            processor: &mut MetricsProcessor,
            requests: &[(u64, u64)],
        ) {
            for &(elapsed, response_time) in requests {
                user.pending_request = Some(make_request_metric("/", response_time, 200, elapsed));
                user.flush_pending_request().unwrap();
            }
            user.flush_batch().unwrap();
            while let Ok(metric) = rx.try_recv() {
                match metric {
                    GooseMetric::Batch(batch) => processor.process_batch(*batch),
                    other => panic!("expected GooseMetric::Batch, got {:?}", other),
                }
            }
        }

        let mut configuration = GooseConfiguration::default();
        configuration.dashboard = true;
        let mut user =
            crate::goose::GooseUser::single("http://localhost/".parse().unwrap(), &configuration)
                .unwrap();
        let (metrics_tx, metrics_rx) = flume::unbounded::<GooseMetric>();
        user.metrics_channel = Some(metrics_tx);
        user.metrics_batch = Some(GooseMetricBatch::new(0));

        let mut processor = make_test_processor(0);
        processor.configuration = configuration;

        // Second 5: two requests averaging 200 ms.
        apply_batch(
            &mut user,
            &metrics_rx,
            &mut processor,
            &[(5000, 100), (5500, 300)],
        );
        processor.graph_data.record_users_per_second(1, 5);

        // Keep the last 2 seconds (4 and 5) so every series origin moves to 4.
        processor.graph_data.prune_to_window(2);

        // Second 7: two requests averaging 100 ms, stored relative to origin 4.
        apply_batch(
            &mut user,
            &metrics_rx,
            &mut processor,
            &[(7000, 50), (7200, 150)],
        );
        // Second 2 is older than the origin: dropped, not a panic.
        apply_batch(&mut user, &metrics_rx, &mut processor, &[(2000, 999)]);

        let window = processor.graph_data.export_series_window(300);
        assert_eq!(window.start_second, 0);
        assert_eq!(window.rps.len(), 8);
        assert_eq!(window.rps[2], 0.0);
        assert_eq!(window.rps[5], 2.0);
        assert_eq!(window.rps[7], 2.0);
        assert_eq!(window.avg_latency_ms[2], 0.0);
        assert_eq!(window.avg_latency_ms[5], 200.0);
        assert_eq!(window.avg_latency_ms[7], 100.0);
    }

    /// A test processor fed through channels the test holds, with `queued`
    /// request metrics already waiting.
    fn make_channel_processor(
        queued: usize,
    ) -> (
        MetricsProcessor,
        flume::Sender<GooseMetric>,
        flume::Sender<MetricsCommand>,
    ) {
        let (metrics_tx, metrics_rx) = flume::unbounded();
        let (cmd_tx, cmd_rx) = flume::unbounded();
        let mut processor = make_test_processor(0);
        processor.metrics_rx = metrics_rx;
        processor.cmd_rx = cmd_rx;
        for _ in 0..queued {
            metrics_tx
                .send(GooseMetric::Request(Box::new(make_request_metric(
                    "/", 10, 200, 1_000,
                ))))
                .unwrap();
        }
        (processor, metrics_tx, cmd_tx)
    }

    #[tokio::test]
    async fn dashboard_snapshot_is_answered_before_the_queue_drains() {
        let queued = 2 * METRICS_DRAIN_LIMIT + 10;
        let (processor, metrics_tx, cmd_tx) = make_channel_processor(queued);
        let (respond, respond_rx) = tokio::sync::oneshot::channel();
        // Queued before the processor starts, so its first turn is this command.
        cmd_tx
            .send(MetricsCommand::GetDashboardSnapshot {
                duration: 2,
                total_users: 1,
                maximum_users: 1,
                active_users: 1,
                target_users: 1,
                series_elapsed_secs: 2,
                phase: "maintain".to_string(),
                stopping: false,
                series_window_secs: dashboard_snapshot::SERIES_WINDOW_SECS,
                report: dashboard_snapshot::SnapshotReport::default(),
                settings: Box::new(dashboard_snapshot::SnapshotSettings::idle()),
                heatmap_since: None,
                respond,
            })
            .unwrap();
        let task = tokio::spawn(processor.run());

        // The snapshot counts exactly one capped drain: the rest of the queue
        // was still waiting when it was built.
        let snapshot = respond_rx.await.unwrap();
        let row = snapshot.requests.iter().find(|r| r.name == "/").unwrap();
        assert_eq!(row.status_codes, vec![(200, METRICS_DRAIN_LIMIT as u64)]);

        // Shutdown still drains everything.
        let (respond, respond_rx) = tokio::sync::oneshot::channel();
        cmd_tx
            .send(MetricsCommand::Shutdown {
                duration: 2,
                total_users: 1,
                maximum_users: 1,
                respond,
            })
            .unwrap();
        let (metrics, _graph_data) = respond_rx.await.unwrap();
        assert_eq!(metrics.requests["GET /"].status_code_counts[&200], queued);
        assert_eq!(metrics_tx.len(), 0);
        task.await.unwrap();
    }

    /// A threshold check drains at most one capped turn of metrics, triggers the
    /// killswitch with the rule and what it observed, and after that judges
    /// nothing more.
    #[test]
    #[serial_test::serial]
    fn check_thresholds_drains_capped_and_triggers_the_killswitch() {
        crate::reset_killswitch();
        let queued = 2 * METRICS_DRAIN_LIMIT + 10;
        let (mut processor, metrics_tx, _cmd_tx) = make_channel_processor(queued);
        processor.configuration.thresholds = vec!["max<5ms".parse().unwrap()];

        assert!(!processor.handle_command(MetricsCommand::CheckThresholds {
            duration: 2,
            seconds: 2.0
        }));
        assert_eq!(
            processor.metrics.requests["GET /"].raw_data.counter,
            METRICS_DRAIN_LIMIT
        );
        assert_eq!(
            crate::killswitch_reason().as_deref(),
            Some("threshold max<5ms failed: observed 10ms")
        );
        assert_eq!(
            processor.metrics.threshold_abort,
            Some((0, "10ms".to_string()))
        );

        // The run is stopping: a later check neither drains nor judges.
        let waiting = metrics_tx.len();
        assert!(!processor.handle_command(MetricsCommand::CheckThresholds {
            duration: 3,
            seconds: 3.0
        }));
        assert_eq!(metrics_tx.len(), waiting);

        // A killswitch triggered first, by anything else, is not the threshold's.
        crate::reset_killswitch();
        let (mut processor, _metrics_tx, _cmd_tx) = make_channel_processor(1);
        processor.configuration.thresholds = vec!["max<5ms".parse().unwrap()];
        crate::trigger_killswitch("user code");
        processor.handle_command(MetricsCommand::CheckThresholds {
            duration: 2,
            seconds: 2.0,
        });
        assert_eq!(processor.metrics.threshold_abort, None);
        assert_eq!(crate::killswitch_reason().as_deref(), Some("user code"));

        // A rule that passes, or has no samples yet, stops nothing.
        crate::reset_killswitch();
        let (mut processor, _metrics_tx, _cmd_tx) = make_channel_processor(1);
        processor.configuration.thresholds = vec![
            "max<1s".parse().unwrap(),
            "max<5ms for GET /other".parse().unwrap(),
        ];
        processor.handle_command(MetricsCommand::CheckThresholds {
            duration: 2,
            seconds: 2.0,
        });
        assert!(!crate::is_killswitch_triggered());
        assert_eq!(processor.metrics.threshold_abort, None);
    }

    /// The holds `rps` thresholds count: each entry to Maintain starts one, with
    /// each request's count from its atomic counters, each exit ends it and adds
    /// its seconds and requests to the holds before, and a metrics reset starts
    /// over, reopening a hold that was going.
    #[test]
    fn rps_window_counts_holds_and_resets() {
        let (mut processor, _metrics_tx, _cmd_tx) = make_channel_processor(0);
        let counters = Arc::new(GooseRequestCounters::new());
        processor
            .request_counter_registry
            .lock()
            .unwrap()
            .insert("GET /".to_string(), counters.clone());
        let shown = |processor: &MetricsProcessor| format!("{:?}", processor.metrics.rps_window);

        // Leaving Maintain before entering it counts nothing.
        processor.handle_command(MetricsCommand::LeaveMaintain { seconds: 1.0 });
        assert_eq!(
            shown(&processor),
            "RpsWindow { seconds: 0.0, requests: {}, open: None }"
        );

        counters.success.store(7, AtomicOrdering::Relaxed);
        counters.failure.store(3, AtomicOrdering::Relaxed);
        processor.handle_command(MetricsCommand::EnterMaintain { seconds: 2.5 });
        assert_eq!(
            shown(&processor),
            "RpsWindow { seconds: 0.0, requests: {}, open: Some(ThresholdMark { seconds: 2.5, requests: {\"GET /\": 10} }) }"
        );

        counters.success.store(27, AtomicOrdering::Relaxed);
        processor.handle_command(MetricsCommand::LeaveMaintain { seconds: 6.0 });
        assert_eq!(
            shown(&processor),
            "RpsWindow { seconds: 3.5, requests: {\"GET /\": 20}, open: None }"
        );

        // A ramp between holds adds nothing; the next hold adds to the first.
        counters.success.store(57, AtomicOrdering::Relaxed);
        processor.handle_command(MetricsCommand::EnterMaintain { seconds: 8.0 });
        counters.success.store(67, AtomicOrdering::Relaxed);
        processor.handle_command(MetricsCommand::LeaveMaintain { seconds: 10.0 });
        assert_eq!(
            shown(&processor),
            "RpsWindow { seconds: 5.5, requests: {\"GET /\": 30}, open: None }"
        );

        // A reset restarts the clock and the counts, and a hold going starts
        // again from it.
        processor.handle_command(MetricsCommand::EnterMaintain { seconds: 11.0 });
        let (ack, _ack_rx) = tokio::sync::oneshot::channel();
        processor.handle_command(MetricsCommand::Reset {
            duration: 12,
            total_users: 1,
            maximum_users: 1,
            active_users: 1,
            series_elapsed_secs: 12,
            ack,
        });
        assert_eq!(
            shown(&processor),
            "RpsWindow { seconds: 0.0, requests: {}, open: Some(ThresholdMark { seconds: 0.0, requests: {} }) }"
        );
    }

    #[test]
    fn batch_constants_are_sensible() {
        const { assert!(METRICS_BATCH_SIZE > 0) };
        const { assert!(METRICS_BATCH_SIZE <= 1000) };
        const { assert!(METRICS_DRAIN_LIMIT > 0) };
        const { assert!(METRICS_DRAIN_LIMIT <= 1024) };
        const { assert!(METRICS_BATCH_MAX_AGE.as_millis() > 0) };
        const { assert!(METRICS_BATCH_MAX_AGE.as_millis() <= 1000) };
    }

    #[test]
    fn aggregate_time_buckets_over_500ms() {
        const GUARD: &str = "guards the time bucket bug fixed in pull request 688";

        let mut transaction = TransactionMetricAggregate::new(
            0,
            Arc::from("scenario"),
            0,
            TransactionName::TransactionOnly(Arc::from("transaction")),
        );
        transaction.set_time(750, true);
        transaction.set_time(1499, true);
        let expected = vec![
            buckets::bucket_floor(750_000),
            buckets::bucket_floor(1_499_000),
        ];
        assert_eq!(
            transaction
                .run_times
                .times_us
                .iter()
                .map(|(&bucket, _)| bucket)
                .collect::<Vec<_>>(),
            expected,
            "transaction times must bucket 750ms and 1499ms in their own microsecond buckets, not 80 and 10: {GUARD}"
        );

        let mut scenario = ScenarioMetricAggregate::new(0, Arc::from("scenario"));
        scenario.update(750, 0);
        scenario.update(1499, 0);
        assert_eq!(
            scenario
                .run_times
                .times_us
                .iter()
                .map(|(&bucket, _)| bucket)
                .collect::<Vec<_>>(),
            expected,
            "scenario times must bucket 750ms and 1499ms in their own microsecond buckets, not 80 and 10: {GUARD}"
        );
    }

    // --- Latency by second (#713) ---

    /// A dashboard processor with the latency histograms on, as the attack
    /// creates it for `mitigation`.
    fn make_latency_test_processor(
        mitigation: GooseCoordinatedOmissionMitigation,
    ) -> MetricsProcessor {
        make_graph_test_processor(&GooseConfiguration {
            dashboard: true,
            co_mitigation: Some(mitigation),
            ..Default::default()
        })
    }

    /// A processor with the graph data and coordinated omission metrics the
    /// attack creates for `configuration`.
    fn make_graph_test_processor(configuration: &GooseConfiguration) -> MetricsProcessor {
        let mut processor = make_dashboard_test_processor(configuration.clone());
        processor.graph_data = GraphData::for_metrics_processor(
            configuration.records_latency_series(),
            configuration.records_synthetic_latency(),
            configuration.records_latency_columns(),
        );
        if let Some(mitigation) = configuration
            .co_mitigation
            .clone()
            .filter(|m| m != &GooseCoordinatedOmissionMitigation::Disabled)
        {
            processor.metrics.coordinated_omission_metrics =
                Some(CoordinatedOmissionMetrics::new(mitigation));
        }
        processor
    }

    /// [`make_graph_test_processor`] with Save report off, keeping only the
    /// dashboard's window.
    fn make_window_test_processor(configuration: &GooseConfiguration) -> MetricsProcessor {
        let mut processor = make_graph_test_processor(configuration);
        processor.keep_full_graph = false;
        processor.graph_data.keep_latency_columns(false, 0);
        processor
    }

    /// Count of measured times in second `second` of the all requests
    /// heatmap.
    fn measured_count(processor: &mut MetricsProcessor, second: usize) -> u64 {
        let window = processor
            .graph_data
            .export_series_window_until(dashboard_snapshot::SERIES_WINDOW_SECS, Some(second + 2));
        window.heatmap.expect("heatmap").counts(second as u64).0
    }

    fn request_series(
        processor: &mut MetricsProcessor,
        name: &str,
        co_active: bool,
    ) -> Option<dashboard_snapshot::RequestSeries> {
        processor.graph_data.export_request_series(
            "GET".to_string(),
            name.to_string(),
            dashboard_snapshot::SERIES_WINDOW_SECS,
            Some(10),
            co_active,
            None,
        )
    }

    /// The measured `(bucket index, count)` pairs of `second` in a heatmap.
    fn measured_buckets(heatmap: &dashboard_snapshot::Heatmap, second: u64) -> Vec<(u64, u64)> {
        heatmap
            .column(second)
            .map(|column| dashboard_snapshot::HeatmapColumn::pairs(&column.measured))
            .unwrap_or_default()
    }

    #[test]
    fn latency_records_failures_but_not_updates_or_co_copies() {
        let mut processor =
            make_latency_test_processor(GooseCoordinatedOmissionMitigation::Average);

        // A failed request lands in all requests and in its own series.
        let mut failed = make_request_metric("/fail", 250, 500, 2_100);
        failed.success = false;
        failed.error = "500 Internal Server Error".to_string();
        processor.process_request_metric(&failed);
        assert_eq!(measured_count(&mut processor, 2), 1);
        let series = request_series(&mut processor, "/fail", false).expect("series");
        // 250 ms is in the 250 to 260 ms row, index 106.
        assert_eq!(measured_buckets(&series.heatmap, 2), [(106, 1)]);

        // An update is not a new measurement.
        let mut update = make_request_metric("/fail", 300, 200, 2_200);
        update.update = true;
        processor.process_request_metric(&update);
        assert_eq!(measured_count(&mut processor, 2), 1);

        // Coordinated omission copies never reach the measured series, with
        // or without a cadence.
        for user_cadence in [0, 100] {
            let mut copy = make_request_metric("/fail", 20, 200, 2_300);
            copy.coordinated_omission_elapsed = 1_000;
            copy.user_cadence = user_cadence;
            processor.process_request_metric(&copy);
            assert_eq!(
                measured_count(&mut processor, 2),
                1,
                "cadence {user_cadence}"
            );
            let series = request_series(&mut processor, "/fail", false).unwrap();
            assert_eq!(series.heatmap.counts(2), (1, 0), "cadence {user_cadence}");
        }
    }

    /// Send `original` through a user thread built from `configuration`, mark
    /// it with `set_failure()` when `mark_failure` is set and `set_success()`
    /// otherwise, and process what the thread sends: the original, then its
    /// update.
    fn process_original_and_update(
        processor: &mut MetricsProcessor,
        configuration: &GooseConfiguration,
        original: GooseRequestMetric,
        mark_failure: bool,
    ) {
        let mut user =
            crate::goose::GooseUser::single("http://localhost/".parse().unwrap(), configuration)
                .unwrap();
        let (metrics_tx, metrics_rx) = flume::unbounded::<GooseMetric>();
        user.metrics_channel = Some(metrics_tx);
        user.metrics_batch = Some(GooseMetricBatch::new(0));
        let mut request = original.clone();
        user.pending_request = Some(original);
        if mark_failure {
            assert!(user
                .set_failure("string missing", &mut request, None, None)
                .is_err());
        } else {
            user.set_success(&mut request).unwrap();
        }
        let mut updates = 0;
        while let Ok(metric) = metrics_rx.try_recv() {
            if matches!(&metric, GooseMetric::Request(request) if request.update) {
                updates += 1;
            }
            processor.process_metric(metric);
        }
        assert_eq!(updates, 1);
    }

    /// A `GET /` request from second 3 that took 40 ms, a success or a 404.
    fn update_test_request(success: bool) -> GooseRequestMetric {
        let mut request = make_request_metric("/", 40, if success { 200 } else { 404 }, 3_400);
        if !success {
            request.success = false;
            request.error = "404 Not Found".to_string();
        }
        request
    }

    /// The configurations the update tests run under: the dashboard with its
    /// heatmap, and a report without one.
    fn update_test_configurations() -> [GooseConfiguration; 2] {
        [
            GooseConfiguration {
                dashboard: true,
                co_mitigation: Some(GooseCoordinatedOmissionMitigation::Average),
                ..Default::default()
            },
            GooseConfiguration {
                report_file: vec!["report.html".to_string()],
                no_percentiles_over_time: true,
                co_mitigation: Some(GooseCoordinatedOmissionMitigation::Average),
                ..Default::default()
            },
        ]
    }

    /// set_failure() on a success leaves one request and one failure in its
    /// second, set_success() on a failure one request and no failure. A
    /// failed original goes through the batch's individual requests with
    /// the heatmap on or off; a successful one takes the by second path with
    /// it on and the summed path with it off.
    #[test]
    fn update_counts_the_request_once() {
        for configuration in update_test_configurations() {
            let heatmap = configuration.records_latency_series();
            for mark_failure in [true, false] {
                let mut processor = make_graph_test_processor(&configuration);
                process_original_and_update(
                    &mut processor,
                    &configuration,
                    update_test_request(mark_failure),
                    mark_failure,
                );
                let graph = &processor.graph_data;
                // The update carries the original's time, so only the count
                // shows a second sample in the average.
                assert_eq!(
                    (
                        graph.requests_in_second_for_test("GET /", 3),
                        graph.errors_in_second_for_test("GET /", 3),
                        graph.average_in_second_for_test("GET /", 3),
                        graph.average_count_in_second_for_test("GET /", 3),
                    ),
                    (1, u32::from(mark_failure), Some(40.0), 1),
                    "heatmap {heatmap}, set_failure {mark_failure}"
                );
            }
        }
    }

    /// The dashboard prunes its series to a trailing window, so an update can
    /// arrive after the original's second is gone: it must not panic or
    /// reach another second, while an update in the window still counts its
    /// request once.
    #[test]
    fn update_after_its_second_was_pruned_changes_nothing() {
        // The dashboard with Save report off, which keeps only its window.
        let configuration = &update_test_configurations()[0];
        let far_second = dashboard_snapshot::SERIES_WINDOW_SECS as usize * 2 + 120;
        let update_of = |original: &GooseRequestMetric, mark_failure: bool| {
            let mut update = original.clone();
            update.update = true;
            update.success = !mark_failure;
            update
        };
        for mark_failure in [true, false] {
            let mut processor = make_window_test_processor(configuration);
            let early = update_test_request(mark_failure);
            let mut late = update_test_request(mark_failure);
            late.elapsed = far_second as u64 * 1_000;
            processor.process_request_metric(&early);
            processor.process_request_metric(&late);
            assert!(!processor.handle_command(MetricsCommand::RecordUsers {
                active_users: 1,
                elapsed_secs: far_second,
            }));
            assert_eq!(
                processor.graph_data.requests_in_second_for_test("GET /", 3),
                0
            );

            processor.process_request_metric(&update_of(&early, mark_failure));
            processor.process_request_metric(&update_of(&late, mark_failure));

            let graph = &processor.graph_data;
            let failures = u32::from(mark_failure);
            for (second, requests, errors) in [(3, 0, 0), (far_second, 1, failures)] {
                assert_eq!(
                    graph.requests_in_second_for_test("GET /", second),
                    requests,
                    "set_failure {mark_failure}, second {second}"
                );
                assert_eq!(
                    graph.errors_in_second_for_test("GET /", second),
                    errors,
                    "set_failure {mark_failure}, second {second}"
                );
                assert_eq!(
                    graph.average_count_in_second_for_test("GET /", second),
                    requests,
                    "set_failure {mark_failure}, second {second}"
                );
            }
        }
    }

    /// A batch of successful `GET /` requests, all from `second`, as a user
    /// thread builds it.
    fn latency_batch(second: usize, times: &[u64]) -> GooseMetricBatch {
        let mut batch = GooseMetricBatch::new(0);
        batch.second = Some(second);
        let mut raw_data = GooseRequestMetricTimingData::new(None);
        for &time in times {
            raw_data.record_time(time);
        }
        batch.requests.insert(
            "GET /".to_string(),
            RequestBatchEntry {
                path: "/".to_string(),
                method: GooseMethod::Get,
                custom_method: String::new(),
                raw_data,
                status_code_counts: HashMap::from([(200, times.len())]),
                status_code_timings: HashMap::new(),
            },
        );
        batch
    }

    #[test]
    fn latency_batch_times_reach_the_rings() {
        let mut processor =
            make_latency_test_processor(GooseCoordinatedOmissionMitigation::Disabled);
        processor.process_batch(latency_batch(3, &[10, 30]));
        processor.process_batch(latency_batch(4, &[1400]));
        assert_eq!(measured_count(&mut processor, 3), 2);
        assert_eq!(measured_count(&mut processor, 4), 1);
        let series = request_series(&mut processor, "/", false).unwrap();
        assert_eq!(measured_buckets(&series.heatmap, 3), [(19, 1), (39, 1)]);
        // 1400 ms is in the 1.4 to 1.5 second row, index 145.
        assert_eq!(measured_buckets(&series.heatmap, 4), [(145, 1)]);
        let graph = &processor.graph_data;
        assert_eq!(graph.requests_in_second_for_test("GET /", 3), 2);
        assert_eq!(graph.requests_in_second_for_test("GET /", 4), 1);
    }

    #[tokio::test]
    async fn latency_has_series_marks_first_hundred_names() {
        let mut processor =
            make_latency_test_processor(GooseCoordinatedOmissionMitigation::Disabled);
        for i in 0..dashboard_snapshot::MAX_REQUEST_ROWS {
            processor.process_request_metric(&make_request_metric(
                &format!("/{i}"),
                10,
                200,
                1_000,
            ));
        }
        // The 101st name, requested most so its row is in the snapshot.
        for _ in 0..5 {
            processor.process_request_metric(&make_request_metric("/late", 10, 200, 1_000));
        }
        // Counts come from the atomic counters, which this test has none of.
        for request in processor.metrics.requests.values_mut() {
            request.success_count = request.raw_data.counter;
        }
        let (respond, respond_rx) = tokio::sync::oneshot::channel();
        processor.handle_command(MetricsCommand::GetDashboardSnapshot {
            duration: 2,
            total_users: 1,
            maximum_users: 1,
            active_users: 1,
            target_users: 1,
            series_elapsed_secs: 2,
            phase: "maintain".to_string(),
            stopping: false,
            series_window_secs: dashboard_snapshot::SERIES_WINDOW_SECS,
            report: dashboard_snapshot::SnapshotReport::default(),
            settings: Box::new(dashboard_snapshot::SnapshotSettings::idle()),
            heatmap_since: None,
            respond,
        });
        let snapshot = respond_rx.await.unwrap();
        assert!(snapshot.flags.percentiles_over_time);
        let late = snapshot
            .requests
            .iter()
            .find(|r| r.name == "/late")
            .unwrap();
        assert!(!late.has_series);
        assert!(snapshot
            .requests
            .iter()
            .filter(|r| r.name != "/late")
            .all(|r| r.has_series));
        // Its times still count toward all requests.
        assert_eq!(snapshot.series.heatmap.unwrap().counts(1), (105, 0));
    }

    #[tokio::test]
    async fn latency_off_snapshot_is_empty() {
        let configuration = GooseConfiguration {
            dashboard: true,
            no_percentiles_over_time: true,
            ..Default::default()
        };
        assert!(!configuration.records_latency_series());
        let mut processor = make_dashboard_test_processor(configuration.clone());
        processor.graph_data = GraphData::for_metrics_processor(
            configuration.records_latency_series(),
            configuration.records_synthetic_latency(),
            configuration.records_latency_columns(),
        );
        processor.process_request_metric(&make_request_metric("/", 10, 200, 1_000));
        let (respond, respond_rx) = tokio::sync::oneshot::channel();
        processor.handle_command(MetricsCommand::GetDashboardSnapshot {
            duration: 2,
            total_users: 1,
            maximum_users: 1,
            active_users: 1,
            target_users: 1,
            series_elapsed_secs: 2,
            phase: "maintain".to_string(),
            stopping: false,
            series_window_secs: dashboard_snapshot::SERIES_WINDOW_SECS,
            report: dashboard_snapshot::SnapshotReport::default(),
            settings: Box::new(dashboard_snapshot::SnapshotSettings::idle()),
            heatmap_since: None,
            respond,
        });
        let snapshot = respond_rx.await.unwrap();
        assert!(!snapshot.flags.percentiles_over_time);
        assert!(!snapshot.series.rps.is_empty());
        assert_eq!(snapshot.series.heatmap, None);
        assert!(snapshot.requests.iter().all(|r| !r.has_series));

        let (respond, respond_rx) = tokio::sync::oneshot::channel();
        processor.handle_command(MetricsCommand::GetRequestSeries {
            method: "GET".to_string(),
            name: "/".to_string(),
            series_elapsed_secs: 2,
            since: None,
            respond,
        });
        assert!(respond_rx.await.unwrap().is_none());
    }

    #[test]
    fn latency_gate() {
        for on in [
            GooseConfiguration {
                dashboard: true,
                ..Default::default()
            },
            // A report, for its heatmap.
            GooseConfiguration {
                report_file: vec!["report.html".to_string()],
                ..Default::default()
            },
        ] {
            assert!(on.records_latency_series());
        }
        for off in [
            GooseConfiguration::default(),
            GooseConfiguration {
                no_percentiles_over_time: true,
                ..Default::default()
            },
            GooseConfiguration {
                dashboard: true,
                no_percentiles_over_time: true,
                ..Default::default()
            },
            GooseConfiguration {
                dashboard: true,
                no_metrics: true,
                ..Default::default()
            },
        ] {
            assert!(!off.records_latency_series());
            assert!(!off.records_synthetic_latency());
        }
        let mitigated = GooseConfiguration {
            dashboard: true,
            co_mitigation: Some(GooseCoordinatedOmissionMitigation::Average),
            ..Default::default()
        };
        assert!(mitigated.records_synthetic_latency());
        let disabled = GooseConfiguration {
            dashboard: true,
            co_mitigation: Some(GooseCoordinatedOmissionMitigation::Disabled),
            ..Default::default()
        };
        assert!(!disabled.records_synthetic_latency());
    }

    #[test]
    fn latency_synthetic_off_with_mitigation_disabled() {
        let mut processor =
            make_latency_test_processor(GooseCoordinatedOmissionMitigation::Disabled);
        assert!(!processor.graph_data.records_synthetic_latency());
        processor.process_request_metric(&make_request_metric("/", 10, 200, 3_000));
        processor
            .graph_data
            .record_synthetic_latency("GET /", 3, 900);
        let window =
            processor
                .graph_data
                .export_series_window_with_heatmap(300, Some(5), true, None);
        assert_eq!(window.heatmap.expect("heatmap").counts(3), (1, 0));
    }

    /// The points each line of the HTML report's graph `id` draws, by name.
    fn report_graph_points(html: &str, id: &str) -> Vec<(String, usize)> {
        let start = html
            .find(&format!("getElementById('{id}')"))
            .unwrap_or_else(|| panic!("no graph {}", id));
        let script = &html[start..start + html[start..].find("</script>").unwrap()];
        let line =
            regex::Regex::new(r#"(?s)name: '([^']*)',\s*type: 'line',.*?data: (\[\[".*?\]\]),"#)
                .unwrap();
        line.captures_iter(script)
            .map(|line| {
                let points: Vec<(String, f64)> = serde_json::from_str(&line[2]).unwrap();
                (line[1].to_string(), points.len())
            })
            .collect()
    }

    /// A run that ends 2.5 s in: the per second count graphs leave out its
    /// final, partial second, every line the same one, while the active users
    /// and average response time graphs keep it.
    #[test]
    fn report_count_graphs_leave_out_the_final_partial_second() {
        let processor = make_test_processor(0);
        let mut metrics = processor.metrics.clone();
        let started = Utc.with_ymd_and_hms(2026, 10, 22, 10, 0, 0).unwrap();
        metrics.history = vec![
            TestPlanHistory {
                action: TestPlanStepAction::Increasing,
                timestamp: started,
                users: 0,
            },
            TestPlanHistory {
                action: TestPlanStepAction::Finished,
                timestamp: started + chrono::Duration::milliseconds(2_500),
                users: 0,
            },
        ];
        metrics.errors.insert(
            "GET /".to_string(),
            GooseErrorMetricAggregate::new(
                GooseMethod::Get,
                "/".to_string(),
                "500".to_string(),
                "",
            ),
        );
        let mut graph_data = GraphData::new();
        // Seconds 0 and 1 are whole; second 2 holds half a second's.
        for (second, count) in [(0, 10), (1, 10), (2, 5)] {
            for _ in 0..count {
                graph_data.record_requests_per_second("GET /", second);
                graph_data.record_errors_per_second("GET /", second);
                graph_data.record_transactions_per_second(second);
                graph_data.record_scenarios_per_second(second);
            }
            graph_data.record_users_per_second(3, second);
            graph_data.record_average_response_time_per_second("GET /".to_string(), second, 20);
        }
        // A request whose last one came in second 1 keeps it.
        graph_data.record_requests_per_second("GET /early", 0);
        graph_data.record_requests_per_second("GET /early", 1);
        let configuration = GooseConfiguration::default();
        let record = RunRecord::default();
        let source = ReportSource {
            configuration: &configuration,
            metrics: &metrics,
            graph_data: &graph_data,
            graphs: true,
            heatmap: None,
            loaded: None,
            record: &record,
            graph_data_in_json: false,
        };
        let html = source.html(None).unwrap();

        let mut rps = report_graph_points(&html, "graph-rps");
        rps.sort();
        assert_eq!(
            rps,
            [
                ("GET /".to_string(), 2),
                ("GET /early".to_string(), 2),
                ("Total".to_string(), 2)
            ]
        );
        for id in ["graph-eps", "graph-tps", "graph-sps"] {
            let lines = report_graph_points(&html, id);
            assert!(!lines.is_empty(), "{} draws a line", id);
            for (line, points) in lines {
                assert_eq!(points, 2, "{id} {line} ends at second 1");
            }
        }
        assert_eq!(
            report_graph_points(&html, "graph-active-users"),
            [("Total".to_string(), 3)]
        );
        let lines = report_graph_points(&html, "graph-avg-response-time");
        assert!(!lines.is_empty(), "graph-avg-response-time draws a line");
        for (line, points) in lines {
            assert_eq!(points, 3, "graph-avg-response-time {line} ends at second 2");
        }
    }
}
