//! # Goose
//!
//! Have you ever been attacked by a goose?
//!
//! Goose is a load testing framework inspired by [Locust](https://locust.io/).
//! User behavior is defined with standard Rust code.
//!
//! Goose load tests, called Goose Attacks, are built by creating an application
//! with Cargo, and declaring a dependency on the Goose library.
//!
//! Goose uses [`reqwest`](https://docs.rs/reqwest/) to provide a convenient HTTP
//! client.
//!
//! ## Documentation
//!
//! - [The Goose Book](https://book.goose.rs)
//! - [Developer documentation](https://docs.rs/goose/)
//! - [Blogs and more](https://tag1.com/goose/)
//!   - [Goose vs Locust and jMeter](https://www.tag1consulting.com/blog/jmeter-vs-locust-vs-goose)
//!   - [Real-life load testing with Goose](https://www.tag1consulting.com/blog/real-life-goose-load-testing)
//!   - [Optimizing Goose performance](https://www.tag1consulting.com/blog/golden-goose-egg-compile-time-adventure)
//!
//! ## License
//!
//! Copyright 2020-2023 Jeremy Andrews
//!
//! Licensed under the Apache License, Version 2.0 (the "License");
//! you may not use this file except in compliance with the License.
//! You may obtain a copy of the License at
//!
//! <http://www.apache.org/licenses/LICENSE-2.0>
//!
//! Unless required by applicable law or agreed to in writing, software
//! distributed under the License is distributed on an "AS IS" BASIS,
//! WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//! See the License for the specific language governing permissions and
//! limitations under the License.

#[macro_use]
extern crate log;

pub mod client;
pub mod config;
pub mod controller;
#[cfg(feature = "dashboard")]
mod dashboard;
#[cfg(feature = "gaggle")]
mod gaggle;
pub mod goose;
mod graph;
pub mod logger;
pub mod metrics;
pub mod prelude;
mod report;
mod run_shape;
mod runs;
pub mod test_plan;
pub mod threshold;
mod throttle;
mod user;
pub mod util;

use lazy_static::lazy_static;
use rand::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{self, Duration};
use std::{fmt, io};

use crate::config::{GooseConfiguration, GooseDefaults};
use crate::controller::{ControlRequest, ControllerProtocol};
use crate::goose::{GooseUser, GooseUserCommand, Scenario, Transaction, TransactionName};
use crate::graph::GraphData;
use crate::logger::{GooseLoggerJoinHandle, GooseLoggerTx};
use crate::metrics::{
    GooseMetric, GooseMetrics, GooseRequestCounterRegistry, MetricsCommand, MetricsProcessor,
};
use crate::test_plan::{TestPlan, TestPlanHistory, TestPlanStepAction};

lazy_static! {
    // The reason the killswitch was first triggered (by Ctrl-C or by
    // `trigger_killswitch`), or None while it is not triggered. Reset each
    // time a load test starts.
    static ref CANCELED: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
}

/// Trigger the killswitch to stop the load test run.
///
/// This function records the reason and marks the killswitch as triggered,
/// which will cause the test to stop gracefully at the next check point. Only
/// the first reason is kept; later calls change nothing until the killswitch
/// is reset when the next load test starts.
///
/// # Arguments
///
/// * `reason` - A string describing why the test is being stopped
///
/// # Example
///
/// ```
/// use goose::trigger_killswitch;
///
/// // Stop the test due to high error rate
/// trigger_killswitch("Error rate exceeded 50% threshold");
/// ```
pub fn trigger_killswitch(reason: &str) {
    let mut canceled = match CANCELED.write() {
        Ok(canceled) => canceled,
        // If the lock is poisoned, we can still write to it
        Err(poisoned) => poisoned.into_inner(),
    };
    if canceled.is_none() {
        info!("Killswitch triggered: {reason}");
        *canceled = Some(reason.to_string());
    }
}

/// Check if the killswitch has been triggered.
pub fn is_killswitch_triggered() -> bool {
    killswitch_reason().is_some()
}

/// The reason the killswitch was first triggered, if it has been.
pub(crate) fn killswitch_reason() -> Option<String> {
    match CANCELED.read() {
        Ok(canceled) => canceled.clone(),
        // If the lock is poisoned, we can still read from it
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// Clear the killswitch.
///
/// Goose clears it itself each time a load test starts, so a new run does not
/// inherit the previous run's cancel. A test that triggers the killswitch can
/// call this when it is done, so tests running after it in the same process
/// are not canceled.
pub fn reset_killswitch() {
    match CANCELED.write() {
        Ok(mut canceled) => *canceled = None,
        Err(poisoned) => *poisoned.into_inner() = None,
    }
}

/// Internal representation of a weighted transaction list.
type WeightedTransactions = Vec<(usize, TransactionName)>;

/// Internal representation of unsequenced transactions.
type UnsequencedTransactions = Vec<Transaction>;
/// Internal representation of sequenced transactions.
type SequencedTransactions = BTreeMap<usize, Vec<Transaction>>;

/// An enumeration of all errors a [`GooseAttack`](./struct.GooseAttack.html) can return.
#[derive(Debug)]
pub enum GooseError {
    /// Wraps a [`std::io::Error`](https://doc.rust-lang.org/std/io/struct.Error.html).
    Io(io::Error),
    /// Wraps a [`reqwest::Error`](https://docs.rs/reqwest/*/reqwest/struct.Error.html).
    Reqwest(reqwest::Error),
    /// Wraps a ['tokio::task::JoinError'](https://tokio-rs.github.io/tokio/doc/tokio/task/struct.JoinError.html).
    TokioJoin(tokio::task::JoinError),
    /// Wraps a ['serde_json::Error'](https://docs.rs/serde_json/*/serde_json/struct.Error.html).
    Serde(serde_json::Error),
    /// Failed attempt to use code that requires a compile-time feature be enabled.
    FeatureNotEnabled {
        /// The missing compile-time feature.
        feature: String,
        /// An optional explanation of the error.
        detail: String,
    },
    /// Failed to parse a hostname.
    InvalidHost {
        /// The invalid hostname that caused this error.
        host: String,
        /// An optional explanation of the error.
        detail: String,
        /// Wraps a [`url::ParseError`](https://docs.rs/url/*/url/enum.ParseError.html).
        parse_error: url::ParseError,
    },
    /// Invalid option or value specified, may only be invalid in context.
    InvalidOption {
        /// The invalid option that caused this error, may be only invalid in context.
        option: String,
        /// The invalid value that caused this error, may be only invalid in context.
        value: String,
        /// An optional explanation of the error.
        detail: String,
    },
    /// Invalid wait time specified.
    InvalidWaitTime {
        // The specified minimum wait time.
        min_wait: Duration,
        // The specified maximum wait time.
        max_wait: Duration,
        /// An optional explanation of the error.
        detail: String,
    },
    /// Invalid weight specified.
    InvalidWeight {
        // The specified weight.
        weight: usize,
        /// An optional explanation of the error.
        detail: String,
    },
    /// Invalid controller command. No longer returned: a Controller answers a
    /// command it refuses with a message, and the variant is kept for callers.
    InvalidControllerCommand {
        /// An optional explanation of the error.
        detail: String,
    },
    /// [`GooseAttack`](./struct.GooseAttack.html) has no [`Scenario`](./goose/struct.Scenario.html) defined.
    NoScenarios {
        /// An optional explanation of the error.
        detail: String,
    },
    /// The load test ran, and at least one `--threshold` failed. Every report was
    /// written first.
    /// A Gaggle manager or worker could not run its part: a worker was turned
    /// away or lost its manager, or a manager's workers did not all join or
    /// prepare.
    Gaggle {
        /// What went wrong.
        detail: String,
    },
    ThresholdsFailed {
        /// The rules that failed, in the order they were given.
        failed: Vec<threshold::ThresholdResult>,
        /// The metrics of the run, as `execute()` returns them when every rule passes.
        metrics: ThresholdMetrics,
    },
}

/// The metrics of a run that failed a threshold, held by
/// [`GooseError::ThresholdsFailed`]. It reads as the [`GooseMetrics`] it holds, as in
/// `metrics.duration`, and its `Debug` leaves them out: a `main` that returns the
/// error prints it, and the metrics of a whole run would bury the rules that failed.
#[derive(Default)]
pub struct ThresholdMetrics(pub Box<GooseMetrics>);

impl std::ops::Deref for ThresholdMetrics {
    type Target = GooseMetrics;

    fn deref(&self) -> &GooseMetrics {
        &self.0
    }
}

impl fmt::Debug for ThresholdMetrics {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("ThresholdMetrics { .. }")
    }
}
/// Implement a helper to provide a text description of all possible types of errors.
impl GooseError {
    fn describe(&self) -> &str {
        match *self {
            GooseError::Io(_) => "io::Error",
            GooseError::Reqwest(_) => "reqwest::Error",
            GooseError::Serde(_) => "serde_json::Error",
            GooseError::TokioJoin(_) => "tokio::task::JoinError",
            GooseError::FeatureNotEnabled { .. } => "required compile-time feature not enabled",
            GooseError::InvalidHost { .. } => "failed to parse hostname",
            GooseError::InvalidOption { .. } => "invalid option or value specified",
            GooseError::InvalidWaitTime { .. } => "invalid wait_time specified",
            GooseError::InvalidWeight { .. } => "invalid weight specified",
            GooseError::InvalidControllerCommand { .. } => "invalid controller command",
            GooseError::NoScenarios { .. } => "no scenarios defined",
            GooseError::ThresholdsFailed { .. } => "one or more thresholds failed",
            GooseError::Gaggle { .. } => "gaggle error",
        }
    }
}

/// Implement format trait to allow displaying errors.
impl fmt::Display for GooseError {
    // Implement display of error with `{}` marker.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            GooseError::Io(ref source) => write!(f, "GooseError: {} ({})", self.describe(), source),
            GooseError::Reqwest(ref source) => {
                write!(f, "GooseError: {} ({})", self.describe(), source)
            }
            GooseError::TokioJoin(ref source) => {
                write!(f, "GooseError: {} ({})", self.describe(), source)
            }
            GooseError::InvalidHost {
                ref parse_error, ..
            } => write!(f, "GooseError: {} ({})", self.describe(), parse_error),
            GooseError::Gaggle { ref detail } => {
                write!(f, "GooseError: {} ({})", self.describe(), detail)
            }
            GooseError::ThresholdsFailed { ref failed, .. } => {
                let failed = failed
                    .iter()
                    .map(|result| format!("{}, observed {}", result.rule, result.observed))
                    .collect::<Vec<_>>()
                    .join("; ");
                write!(f, "GooseError: {} ({})", self.describe(), failed)
            }
            _ => write!(f, "GooseError: {}", self.describe()),
        }
    }
}

// Define the lower level source of this error, if any.
impl std::error::Error for GooseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            GooseError::Io(ref source) => Some(source),
            GooseError::Reqwest(ref source) => Some(source),
            GooseError::TokioJoin(ref source) => Some(source),
            GooseError::InvalidHost {
                ref parse_error, ..
            } => Some(parse_error),
            _ => None,
        }
    }
}

/// Auto-convert Reqwest errors.
impl From<reqwest::Error> for GooseError {
    fn from(err: reqwest::Error) -> GooseError {
        GooseError::Reqwest(err)
    }
}

/// Auto-convert IO errors.
impl From<io::Error> for GooseError {
    fn from(err: io::Error) -> GooseError {
        GooseError::Io(err)
    }
}

/// Auto-convert TokioJoin errors.
impl From<tokio::task::JoinError> for GooseError {
    fn from(err: tokio::task::JoinError) -> GooseError {
        GooseError::TokioJoin(err)
    }
}

/// Auto-convert serde_json errors.
impl From<serde_json::Error> for GooseError {
    fn from(err: serde_json::Error) -> GooseError {
        GooseError::Serde(err)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A [`GooseAttack`](./struct.GooseAttack.html) load test operates in one (and only one)
/// of the following modes.
pub enum AttackMode {
    /// During early startup before one of the following modes gets assigned.
    Undefined,
    /// A single standalone process performing a load test.
    StandAlone,
    /// A Gaggle manager (`--manager`): it runs no users, and splits the load test
    /// between its workers. Needs the `gaggle` feature.
    Manager,
    /// A Gaggle worker (`--worker`): it runs the share of the load test its
    /// manager hands it. Needs the `gaggle` feature.
    Worker,
}

/// What a [`GooseAttack::set_worker_data`] hook is told about the process it
/// builds data for: a Gaggle worker, or a run of one process as worker 0 of 1.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct WorkerShare {
    /// The worker's index, from 0.
    pub index: usize,
    /// How many workers share the load test: 1 for a run of one process.
    pub workers: usize,
    /// The users the worker starts over the run.
    pub users: usize,
    /// The global ordinal of the worker's first user: its users are
    /// `user_offset..user_offset + users` of the whole load test's.
    pub user_offset: usize,
}

/// A [`GooseAttack::set_worker_data`] hook.
type WorkerDataHook = dyn Fn(&WorkerShare) -> Vec<u8> + Send + Sync;

/// The most bytes a worker data hook may return for one worker.
pub const MAX_WORKER_DATA: usize = 16 * 1024 * 1024;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
/// A [`GooseAttack`](./struct.GooseAttack.html) load test moves through each of the following
/// phases during a complete load test.
pub enum AttackPhase {
    /// No load test is running, configuration can be changed by a Controller.
    Idle,
    /// [`GooseUser`](./goose/struct.GooseUser.html)s are launching.
    Increase,
    /// [`GooseUser`](./goose/struct.GooseUser.html)s have been launched and are generating load.
    Maintain,
    /// [`GooseUser`](./goose/struct.GooseUser.html)s are stopping.
    Decrease,
    /// Exiting the load test.
    Shutdown,
}

/// Lowercase string for an [`AttackPhase`] (dashboard / control protocol).
pub(crate) fn attack_phase_str(phase: AttackPhase) -> &'static str {
    match phase {
        AttackPhase::Idle => "idle",
        AttackPhase::Increase => "increase",
        AttackPhase::Maintain => "maintain",
        AttackPhase::Decrease => "decrease",
        AttackPhase::Shutdown => "shutdown",
    }
}

/// The option a [`GooseError::InvalidOption`] names when no host is
/// configured; a refused Start reads its cause from it.
const NO_HOST_OPTION: &str = "--host";

/// The transactions that run with no scenario host, so need a default host.
const HOST_TRANSACTIONS: [&str; 2] = ["test_start", "test_stop"];

/// The detail of the [`NO_HOST_OPTION`] error refusing a run whose
/// `transaction`, one of [`HOST_TRANSACTIONS`], has no host; a refused Start
/// reads the transaction back from it.
fn no_transaction_host_detail(transaction: &str) -> String {
    format!(
        "A host must be defined via the --host option or the GooseAttack.set_default() function (no host defined for the {transaction} transaction)."
    )
}

/// The option a [`GooseError::InvalidOption`] names when there are no users
/// to start; a refused Start reads its cause from it.
const NO_USERS_OPTION: &str = "`configuration.users`";

/// Machine-readable outcome of a control action (soft success/failure).
///
/// Soft failures (expected rejections such as wrong phase or prepare failure)
/// are returned as `ok: false` with a stable `error` code. Hard failures (e.g.
/// `weight_scenario_users`, `reset_run_state` after a successful Start reply)
/// remain `Result::Err`.
///
/// Telnet and WebSocket show `ok` and `message`; the dashboard returns every
/// field.
#[derive(Debug, Clone)]
pub(crate) struct ControlOutcome {
    pub ok: bool,
    /// Stable code when !ok, for soft failures only: `invalid_phase`,
    /// `prepare_failed` or `conflicting_settings`, for example, and
    /// `invalid_report`, `report_busy` or `report_failed` from the report
    /// control routes.
    pub error: Option<&'static str>,
    /// Human message, the reply every front end shows.
    pub message: String,
    /// Attack phase *after* the soft attempt (lowercase).
    pub phase: String,
    /// From `GooseAttackRunState.active_users` at reply time.
    pub active_users: usize,
    /// Command-specific target user count when applicable.
    pub target_users: Option<usize>,
    /// The setting at fault in a refusal, when one is: `host`, `users`,
    /// `increase_rate`, `increase_time`, `decrease_rate`, `decrease_time`,
    /// `run_time` or `test_plan`.
    pub field: Option<&'static str>,
    /// The run settings after a dashboard settings command, applied or
    /// not, so the page that sent it has them before the next snapshot.
    #[cfg(feature = "dashboard")]
    pub settings: Option<Box<metrics::dashboard_snapshot::SnapshotSettings>>,
}

impl ControlOutcome {
    /// This outcome, naming `field` as the setting at fault.
    pub(crate) fn with_field(mut self, field: &'static str) -> Self {
        self.field = Some(field);
        self
    }
}

/// Result of the pre-reply portion of Start (Controller reply point).
///
/// Callers must reply to the client on either variant, then call
/// [`GooseAttack::control_start_finish`] only on [`ControlStartBegin::Accepted`].
pub(crate) enum ControlStartBegin {
    /// Soft rejection — do not change phase; reply false / ok:false.
    Rejected(ControlOutcome),
    /// Phase is already `Increase`; caller must reply success NOW, then call finish.
    Accepted(ControlOutcome),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Used to define the order [`Scenario`](./goose/struct.Scenario.html)s and
/// [`Transaction`](./goose/struct.Transaction.html)s are allocated.
///
/// In order to configure the scheduler, and to see examples of the different scheduler
/// variants, review the
/// [`GooseAttack::set_scheduler`](./struct.GooseAttack.html#method.set_scheduler)
/// documentation.
pub enum GooseScheduler {
    /// Allocate one of each available type at a time (default).
    RoundRobin,
    /// Allocate in the order and weighting defined.
    Serial,
    /// Allocate in a random order.
    Random,
}

#[derive(Debug)]
/// Internal global run state for load test.
struct GooseAttackRunState {
    /// When the next [`GooseUser`](./goose/struct.GooseUser.html) is due to be
    /// increased or decreased: the time the first of the step was, plus one
    /// interval for each since.
    adjust_user_timer: std::time::Instant,
    /// How many milliseconds apart [`GooseUser`](./goose/struct.GooseUser.html)s
    /// are increased or decreased, at least 1 once a step has adjusted one; 0
    /// until then, so the step's first user is adjusted at once.
    adjust_user_in_ms: usize,
    /// A counter tracking how many [`GooseUser`](./goose/struct.GooseUser.html)s are running.
    active_users: usize,
    /// A counter tracking many users have been stopped.
    completed_users: usize,
    /// This variable accounts for time spent doing things which is then subtracted from
    /// the time sleeping to avoid an unintentional drift in events that are supposed to
    /// happen regularly.
    drift_timer: tokio::time::Instant,
    /// Unbounded sender used by all [`GooseUser`](./goose/struct.GooseUser.html)
    /// threads to send metrics to parent.
    all_threads_metrics_tx: flume::Sender<GooseMetric>,
    /// Sender for commands to the dedicated metrics processor task.
    metrics_cmd_tx: flume::Sender<MetricsCommand>,
    /// Handle for the dedicated metrics processor task.
    metrics_processor_handle: Option<tokio::task::JoinHandle<()>>,
    /// Unbounded sender used by all [`GooseUser`](./goose/struct.GooseUser.html)
    /// threads to alert the parent when they shutdown.
    all_threads_shutdown_tx: flume::Sender<usize>,
    /// Unbounded receiver used by [`GooseUser`](./goose.GooseUser.html) threads to notify
    /// the parent if they shut themselves down (for example if `--iterations` is reached).
    shutdown_rx: flume::Receiver<usize>,
    /// Optional unbounded receiver for logger thread, if enabled.
    logger_handle: GooseLoggerJoinHandle,
    /// Optional unbounded sender from all [`GooseUser`](./goose/struct.GooseUser.html)s
    /// to logger thread, if enabled.
    all_threads_logger_tx: GooseLoggerTx,
    /// Optional receiver for all [`GooseUser`](./goose/struct.GooseUser.html)s from
    /// throttle thread, if enabled.
    throttle_threads_tx: Option<flume::Sender<bool>>,
    /// Optional sender for throttle thread, if enabled.
    parent_to_throttle_tx: Option<flume::Sender<bool>>,
    /// The channel telnet and WebSocket send commands on, when either is
    /// enabled.
    controller_channel_rx: Option<flume::Receiver<ControlRequest>>,
    /// The channel dashboard control sends the same commands on, answered
    /// before the Controllers' so they can not delay it.
    dashboard_control_rx: Option<flume::Receiver<ControlRequest>>,
    /// Optional channel allowing the dashboard HTTP task to request snapshots.
    #[cfg(feature = "dashboard")]
    dashboard_channel_rx: Option<flume::Receiver<dashboard::DashboardRequest>>,
    /// Signals the dashboard SnapshotHub to emit SSE `event: closed` on final exit.
    #[cfg(feature = "dashboard")]
    dashboard_close_tx: Option<tokio::sync::watch::Sender<bool>>,
    /// The series second of the last `--gaggle-shadow-merge` delta.
    #[cfg(feature = "gaggle")]
    gaggle_shadow_second: Option<usize>,
    /// Dashboard tabs connected to the event stream.
    #[cfg(feature = "dashboard")]
    dashboard_active_sse: Option<Arc<std::sync::atomic::AtomicUsize>>,
    /// The folder of the run in progress, as the dashboard's report routes see it.
    #[cfg(feature = "dashboard")]
    dashboard_running_folder: Option<dashboard::RunningFolder>,
    /// The folder a bare `--goose-log` is open in, as the dashboard's report
    /// routes see it.
    #[cfg(feature = "dashboard")]
    dashboard_log_folder: Option<dashboard::RunningFolder>,
    /// The address the dashboard listens on, as logged at startup.
    #[cfg(feature = "dashboard")]
    dashboard_url: Option<String>,
    /// The dashboard HTTP server, stopped when the attack loop returns.
    #[cfg(feature = "dashboard")]
    dashboard_server: Option<dashboard::DashboardServer>,
    /// A flag tracking whether or not the header has been written when the metrics
    /// log is enabled.
    metrics_header_displayed: bool,
    /// When entering the idle phase use this flag to only display a message one time.
    idle_status_displayed: bool,
    /// Collection of all [`GooseUser`](./goose/struct.GooseUser.html) threads so they
    /// can be stopped later.
    users: Vec<tokio::task::JoinHandle<()>>,
    /// All unbounded senders to allow communication with
    /// [`GooseUser`](./goose/struct.GooseUser.html) threads.
    user_channels: Vec<flume::Sender<GooseUserCommand>>,
    /// The `(scenario index, user)` of the user each of `user_channels`
    /// reaches, so a metrics reset counts the users still running.
    running_users: Vec<(usize, usize)>,
    /// Timer tracking when to display running metrics, if enabled.
    running_metrics_timer: std::time::Instant,
    /// When the next `--threshold` check is due, or `None` while the thresholds are
    /// not checked: without any, under `--no-threshold-abort`, and before the run
    /// first enters Maintain.
    threshold_check_at: Option<std::time::Instant>,
    /// Boolean flag indicating if all [`GooseUser`](./goose/struct.GooseUser.html)s
    /// have been spawned.
    all_users_spawned: bool,
    /// Set of users that have shut themselves down.
    users_shutdown: HashSet<usize>,
    /// The telnet and WebSocket Controllers that bound their ports.
    listening_controllers: controller::ListeningControllers,
    /// Boolean flag indicating of Goose should shutdown after stopping a running load test.
    shutdown_after_stop: bool,
    /// Whether Ctrl-C has already started a cancel, so the main loop starts
    /// only one. Set once and never cleared: Ctrl-C always ends in Shutdown.
    canceling: bool,
    /// Whether a cancel (Stop, `shutdown`, Ctrl-C) is ramping this run down.
    /// Set in `cancel_attack` and cleared when the run reaches Idle, so a new
    /// Start begins without it; `canceling` above is set only on the Ctrl-C
    /// path and never cleared.
    stopping: bool,
    /// Shared registry of atomic request counters, keyed by `"METHOD path"`.
    /// User threads increment these directly; the parent reads them at report time.
    request_counter_registry: GooseRequestCounterRegistry,
    /// Shared epoch counter for coordinating batch validity across metrics resets.
    metrics_epoch: crate::metrics::MetricsEpoch,
}

impl GooseAttackRunState {
    /// Schedule the next user increase or decrease one `interval` after the
    /// last was due, or after now for the step's first, so a step adjusts its
    /// users at a steady rate from a fixed origin: a late wake up is caught up,
    /// never carried into every later user.
    fn schedule_next_adjust(&mut self, interval: Duration) {
        self.adjust_user_timer = if self.adjust_user_in_ms == 0 {
            std::time::Instant::now() + interval
        } else {
            self.adjust_user_timer + interval
        };
        self.adjust_user_in_ms = (interval.as_millis() as usize).max(1);
    }

    /// Whether the next user increase or decrease is due.
    fn adjust_due(&self) -> bool {
        self.adjust_user_in_ms == 0 || std::time::Instant::now() >= self.adjust_user_timer
    }

    /// Sleep until the next user increase or decrease is due, but never more
    /// than 500 ms, waking up at least twice a second to handle messages and
    /// allow for a quick shutdown if the load test is canceled.
    async fn sleep_until_adjust(&mut self) {
        let maximum_sleep = Duration::from_millis(500);
        let until_due = self
            .adjust_user_timer
            .saturating_duration_since(std::time::Instant::now());
        if until_due >= maximum_sleep {
            self.drift_timer = util::sleep_minus_drift(maximum_sleep, self.drift_timer).await;
        } else {
            debug!("sleeping {until_due:?}...");
            tokio::time::sleep(until_due).await;
            self.drift_timer = tokio::time::Instant::now();
        }
    }
}

/// Global internal state for the load test.
pub struct GooseAttack {
    /// An optional transaction that is run one time before starting GooseUsers and running Scenarios.
    test_start_transaction: Option<Transaction>,
    /// An optional transaction that is run one time after all GooseUsers have finished.
    test_stop_transaction: Option<Transaction>,
    /// A vector containing one copy of each Scenario defined by this load test.
    scenarios: Vec<Scenario>,
    /// A set of all registered scenario names.
    scenario_machine_names: HashSet<String>,
    /// A weighted vector containing a GooseUser object for each GooseUser that will run during this load test.
    weighted_users: Vec<GooseUser>,
    /// How the `--gaggle-shadow-merge` self check ended, once it has.
    #[cfg(feature = "gaggle")]
    gaggle_shadow: Option<Arc<Mutex<Option<gaggle::shadow::ShadowOutcome>>>>,
    /// The global ordinal of this process's first user: 0, unless it runs a
    /// share of a Gaggle's users. User indexes, the users each scenario
    /// counts and coordinated omission events all count from it.
    #[cfg(feature = "gaggle")]
    user_offset: usize,
    /// The index in `scenarios` of each of this process's users, in ordinal
    /// order, when it runs a share of a Gaggle's users; `None` allocates
    /// every user here.
    #[cfg(feature = "gaggle")]
    user_slice: Option<Vec<usize>>,
    /// The hook [`GooseAttack::set_worker_data`] registered.
    worker_data_hook: Option<Arc<WorkerDataHook>>,
    /// What the hook returned for this process, given to every user.
    worker_data: Option<Arc<[u8]>>,
    /// The manager or worker side of a Gaggle, with `--manager` or `--worker`.
    #[cfg(feature = "gaggle")]
    gaggle: Option<gaggle::run::GaggleRole>,
    /// Where a manager reports the address it listens on (unit tests).
    #[cfg(all(test, feature = "gaggle"))]
    gaggle_bound: Option<tokio::sync::oneshot::Sender<std::net::SocketAddr>>,
    /// Optional default values for Goose run-time options.
    defaults: GooseDefaults,
    /// Configuration object holding options set when launching the load test.
    configuration: GooseConfiguration,
    /// The load test operates in only one of the following modes: StandAlone, Manager, or Worker.
    attack_mode: AttackMode,
    /// Which phase the load test is currently operating in.
    attack_phase: AttackPhase,
    /// Defines the order [`Scenario`](./goose/struct.Scenario.html)s and
    /// [`Transaction`](./goose/struct.Transaction.html)s are allocated.
    scheduler: GooseScheduler,
    /// When the load test started.
    ///
    /// Restarted after the increase→maintain metrics reset so running duration
    /// and report rate windows exclude ramp-up. Do **not** use this for the
    /// active-users series clock — see [`Self::series_started`].
    started: Option<time::Instant>,
    /// The seconds since `started`, to the microsecond, when `metrics.duration` was
    /// last set from them.
    elapsed: f64,
    /// Continuous clock for per-second graph series (especially active users).
    ///
    /// Set once when a run starts (`reset_run_state`) and **not** restarted on
    /// metrics reset, so ramp-up and later user changes stay on one timeline.
    series_started: Option<time::Instant>,
    /// Internal Goose test plan representation.
    test_plan: TestPlan,
    /// The shape of a running users ramp, `None` for a test plan and when
    /// no run is in progress.
    run_shape: Option<run_shape::RunShape>,
    /// The snapshot's run settings, but for the fields computed per
    /// snapshot; built when first asked for after a control command.
    #[cfg(feature = "dashboard")]
    settings: Option<metrics::dashboard_snapshot::SnapshotSettings>,
    /// When the current test plan step started.
    step_started: Option<time::Instant>,
    /// All metrics merged together.
    metrics: GooseMetrics,
    /// All data for report graphs.
    graph_data: GraphData,
    /// The client builder from the last `set_client_builder_with_cookies` or
    /// `set_client_builder_without_cookies` call; `None` builds each user's client
    /// from the configuration alone.
    client_builder: Option<client::ClientBuilderSettings>,
    /// How users get their clients when a client builder is set, built once when
    /// the load test starts, so a shared client is one client for the whole run.
    client_strategy: Option<client::ClientStrategy>,
    /// Set when a run starts and cleared once its reports are written, so a
    /// shutdown with no run since the last one writes nothing.
    run_in_progress: bool,
    /// How the current run ended, and the killswitch reason when canceled.
    /// The first cause recorded wins; unset means the plan completed.
    ended_by: Option<(runs::EndedBy, Option<String>)>,
    /// The run's folder and what Goose writes into it.
    output: runs::Output,
    /// The Chrome binary PDF reports run, found once at startup.
    #[cfg(feature = "pdf-reports")]
    chrome: tokio::sync::OnceCell<report::chrome::ResolvedChrome>,
}

/// Whether `--scenarios` pattern `pattern` selects the scenario with machine
/// name `machine_name`: exactly, or with `*` wildcards.
fn scenario_pattern_matches(pattern: &str, machine_name: &str) -> bool {
    if pattern.contains('*') {
        wildcard_match(pattern, machine_name)
    } else {
        machine_name == pattern
    }
}

/// Match a pattern containing `*` wildcards against text.
/// Each `*` matches zero or more characters.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p = pattern.as_bytes();
    let t = text.as_bytes();
    let (mut pi, mut ti) = (0, 0);
    let (mut star_pi, mut star_ti) = (None::<usize>, 0);

    while ti < t.len() {
        if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(s) = star_pi {
            pi = s + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }

    // Consume trailing stars.
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }

    pi == p.len()
}

/// Goose's internal global state.
impl GooseAttack {
    /// Load configuration and initialize a [`GooseAttack`](./struct.GooseAttack.html).
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut goose_attack = GooseAttack::initialize();
    /// ```
    pub fn initialize() -> Result<GooseAttack, GooseError> {
        let configuration = GooseConfiguration::parse_args_default_or_exit();
        Ok(GooseAttack {
            test_start_transaction: None,
            test_stop_transaction: None,
            scenarios: Vec::new(),
            scenario_machine_names: HashSet::new(),
            weighted_users: Vec::new(),
            #[cfg(feature = "gaggle")]
            gaggle_shadow: None,
            #[cfg(feature = "gaggle")]
            user_offset: 0,
            #[cfg(feature = "gaggle")]
            user_slice: None,
            worker_data_hook: None,
            worker_data: None,
            #[cfg(feature = "gaggle")]
            gaggle: None,
            #[cfg(all(test, feature = "gaggle"))]
            gaggle_bound: None,
            defaults: GooseDefaults::default(),
            configuration,
            attack_mode: AttackMode::Undefined,
            attack_phase: AttackPhase::Idle,
            scheduler: GooseScheduler::RoundRobin,
            started: None,
            elapsed: 0.0,
            series_started: None,
            test_plan: TestPlan::new(),
            run_shape: None,
            #[cfg(feature = "dashboard")]
            settings: None,
            step_started: None,
            metrics: GooseMetrics::default(),
            graph_data: GraphData::new(),
            client_builder: None,
            client_strategy: None,
            run_in_progress: false,
            ended_by: None,
            output: runs::Output::default(),
            #[cfg(feature = "pdf-reports")]
            chrome: tokio::sync::OnceCell::new(),
        })
    }

    /// Initialize a [`GooseAttack`](./struct.GooseAttack.html) with an already loaded
    /// configuration.
    ///
    /// This is generally used by Worker instances and tests.
    ///
    /// # Example
    /// ```rust
    /// use goose::GooseAttack;
    /// use goose::config::GooseConfiguration;
    ///
    /// let configuration = GooseConfiguration::parse_args_default_or_exit();
    /// let mut goose_attack = GooseAttack::initialize_with_config(configuration);
    /// ```
    pub fn initialize_with_config(
        configuration: GooseConfiguration,
    ) -> Result<GooseAttack, GooseError> {
        Ok(GooseAttack {
            test_start_transaction: None,
            test_stop_transaction: None,
            scenarios: Vec::new(),
            scenario_machine_names: HashSet::new(),
            weighted_users: Vec::new(),
            #[cfg(feature = "gaggle")]
            gaggle_shadow: None,
            #[cfg(feature = "gaggle")]
            user_offset: 0,
            #[cfg(feature = "gaggle")]
            user_slice: None,
            worker_data_hook: None,
            worker_data: None,
            #[cfg(feature = "gaggle")]
            gaggle: None,
            #[cfg(all(test, feature = "gaggle"))]
            gaggle_bound: None,
            defaults: GooseDefaults::default(),
            configuration,
            attack_mode: AttackMode::Undefined,
            attack_phase: AttackPhase::Idle,
            scheduler: GooseScheduler::RoundRobin,
            started: None,
            elapsed: 0.0,
            series_started: None,
            test_plan: TestPlan::new(),
            run_shape: None,
            #[cfg(feature = "dashboard")]
            settings: None,
            step_started: None,
            metrics: GooseMetrics::default(),
            graph_data: GraphData::new(),
            client_builder: None,
            client_strategy: None,
            run_in_progress: false,
            ended_by: None,
            output: runs::Output::default(),
            #[cfg(feature = "pdf-reports")]
            chrome: tokio::sync::OnceCell::new(),
        })
    }

    /// Define the order [`Scenario`](./goose/struct.Scenario.html)s are
    /// allocated to new [`GooseUser`](./goose/struct.GooseUser.html)s as they are
    /// launched.
    ///
    /// By default, [`Scenario`](./goose/struct.Scenario.html)s are allocated
    /// to new [`GooseUser`](./goose/struct.GooseUser.html)s in a round robin style.
    /// For example, if Scenario A has a weight of 5, Scenario B has a weight of 3, and
    /// you launch 20 users, they will be launched in the following order:
    ///  A, B, A, B, A, B, A, A, A, B, A, B, A, B, A, A, A, B, A, B
    ///
    /// Note that the following pattern is repeated:
    ///  A, B, A, B, A, B, A, A
    ///
    /// If reconfigured to schedule serially, then they will instead be allocated in
    /// the following order:
    ///  A, A, A, A, A, B, B, B, A, A, A, A, A, B, B, B, A, A, A, A
    ///
    /// In the serial case, the following pattern is repeated:
    ///  A, A, A, A, A, B, B, B
    ///
    /// In the following example, [`Scenario`](./goose/struct.Scenario.html)s
    /// are allocated to launching [`GooseUser`](./goose/struct.GooseUser.html)s in a
    /// random order. This means running the test multiple times can generate
    /// different amounts of load, as depending on your weighting rules you may
    /// have a different number of [`GooseUser`](./goose/struct.GooseUser.html)s
    /// running each [`Scenario`](./goose/struct.Scenario.html) each time.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), GooseError> {
    ///     GooseAttack::initialize()?
    ///         .set_scheduler(GooseScheduler::Random)
    ///         .register_scenario(scenario!("A Scenario")
    ///             .set_weight(5)?
    ///             .register_transaction(transaction!(a_transaction))
    ///         )
    ///         .register_scenario(scenario!("B Scenario")
    ///             .set_weight(3)?
    ///             .register_transaction(transaction!(b_transaction))
    ///         );
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn a_transaction(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.get("/foo").await?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn b_transaction(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.get("/bar").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_scheduler(mut self, scheduler: GooseScheduler) -> Self {
        self.scheduler = scheduler;
        self
    }

    /// A load test must contain one or more [`Scenario`](./goose/struct.Scenario.html)s
    /// be registered into Goose's global state with this method for it to run.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), GooseError> {
    ///     GooseAttack::initialize()?
    ///         .register_scenario(scenario!("ExampleScenario")
    ///             .register_transaction(transaction!(example_transaction))
    ///         )
    ///         .register_scenario(scenario!("OtherScenario")
    ///             .register_transaction(transaction!(other_transaction))
    ///         );
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn example_transaction(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.get("/foo").await?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn other_transaction(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.get("/bar").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn register_scenario(mut self, mut scenario: Scenario) -> Self {
        scenario.scenarios_index = self.scenarios.len();
        // Machine names must be unique. If this machine name has already been seen, add an
        // integer at the end to differentiate.
        let mut conflicts: u32 = 0;
        let mut machine_name = scenario.machine_name.to_string();
        // Inserting into the scenario_machine_names hashset will fail if this is name was
        // already seen.
        while !self.scenario_machine_names.insert(machine_name) {
            // For each conflict increase the counter and try again.
            conflicts += 1;
            machine_name = format!("{}_{}", scenario.machine_name, conflicts);
        }
        // If there was a conflict, also update the scenario itself.
        if conflicts > 0 {
            scenario.machine_name = Arc::from(format!("{}_{}", scenario.machine_name, conflicts));
        }
        // Finally, register the scenario.
        self.scenarios.push(scenario);
        self
    }

    /// Optionally define a transaction to run before users are started and all transactions
    /// start running. This is would generally be used to set up anything required
    /// for the load test.
    ///
    /// The [`GooseUser`](./goose/struct.GooseUser.html) used to run the `test_start`
    /// transactions is not preserved and does not otherwise affect the subsequent
    /// [`GooseUser`](./goose/struct.GooseUser.html)s that run the rest of the load
    /// test. For example, if the [`GooseUser`](./goose/struct.GooseUser.html)
    /// logs in during `test_start`, subsequent [`GooseUser`](./goose/struct.GooseUser.html)
    /// do not retain this session and are therefor not already logged in.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), GooseError> {
    ///     GooseAttack::initialize()?
    ///         .test_start(transaction!(setup));
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn setup(user: &mut GooseUser) -> TransactionResult {
    ///     // do stuff to set up load test ...
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn test_start(mut self, transaction: Transaction) -> Self {
        self.test_start_transaction = Some(transaction);
        self
    }

    /// Optionally define a transaction to run after all users have finished running
    /// all defined transactions. This would generally be used to clean up anything
    /// that was specifically set up for the load test.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), GooseError> {
    ///     GooseAttack::initialize()?
    ///         .test_stop(transaction!(teardown));
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn teardown(user: &mut GooseUser) -> TransactionResult {
    ///     // do stuff to tear down the load test ...
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn test_stop(mut self, transaction: Transaction) -> Self {
        self.test_stop_transaction = Some(transaction);
        self
    }

    /// Give each Gaggle worker data of its own, such as its share of a pool of
    /// credentials, so no two workers use the same accounts.
    ///
    /// The hook is called once for each worker, by the manager, after
    /// `test_start` and before any worker starts, and is told the worker's
    /// index, how many workers there are, how many users the worker starts and
    /// the global ordinal of its first user. What it returns is sent to that
    /// worker, and every user there reads it with
    /// [`GooseUser::worker_data`](./goose/struct.GooseUser.html#method.worker_data):
    /// one copy, shared, not one per user. At most [`MAX_WORKER_DATA`] bytes
    /// for each worker; more is refused with an error. The data is not part of
    /// the load test's hash, so it may differ from run to run.
    ///
    /// A run of one process calls the hook once, as worker 0 of 1 holding every
    /// user, after `test_start`, so a load test behaves the same with or
    /// without a Gaggle. Without a hook, `worker_data` is empty.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// /// 300 accounts, `user0` to `user299`.
    /// fn accounts() -> Vec<String> {
    ///     (0..300).map(|n| format!("user{n}")).collect()
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), GooseError> {
    ///     GooseAttack::initialize()?
    ///         // Each worker gets its first user's ordinal, then the accounts
    ///         // for its own users, one per line.
    ///         .set_worker_data(|share: &WorkerShare| {
    ///             let pool = accounts();
    ///             let start = share.user_offset.min(pool.len());
    ///             let end = (share.user_offset + share.users).min(pool.len());
    ///             let mut lines = vec![share.user_offset.to_string()];
    ///             lines.extend(pool[start..end].iter().cloned());
    ///             lines.join("\n").into_bytes()
    ///         })
    ///         .register_scenario(scenario!("LoggedIn").register_transaction(transaction!(log_in)));
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn log_in(user: &mut GooseUser) -> TransactionResult {
    ///     let data = String::from_utf8_lossy(user.worker_data()).into_owned();
    ///     let mut lines = data.lines();
    ///     let first: usize = lines.next().and_then(|line| line.parse().ok()).unwrap_or(0);
    ///     let accounts: Vec<&str> = lines.collect();
    ///     // `weighted_users_index` counts users across the whole load test, from
    ///     // the worker's first user's ordinal: each user takes its own account.
    ///     let index = user.weighted_users_index.saturating_sub(first);
    ///     if let Some(account) = accounts.get(index) {
    ///         let _goose = user.post("/login", account.to_string()).await?;
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_worker_data<F>(mut self, hook: F) -> Self
    where
        F: Fn(&WorkerShare) -> Vec<u8> + Send + Sync + 'static,
    {
        self.worker_data_hook = Some(Arc::new(hook));
        self
    }

    /// The bytes the worker data hook returns for `share`, refused past
    /// [`MAX_WORKER_DATA`]; `None` without a hook.
    pub(crate) fn call_worker_data_hook(
        &self,
        share: &WorkerShare,
    ) -> Result<Option<Vec<u8>>, GooseError> {
        let Some(hook) = self.worker_data_hook.as_ref() else {
            return Ok(None);
        };
        let data = hook(share);
        if data.len() > MAX_WORKER_DATA {
            return Err(GooseError::InvalidOption {
                option: "GooseAttack::set_worker_data".to_string(),
                value: data.len().to_string(),
                detail: format!(
                    "the worker data hook returned {} bytes for worker {}, more than the {} allowed.",
                    data.len(),
                    share.index,
                    MAX_WORKER_DATA
                ),
            });
        }
        Ok(Some(data))
    }

    /// Build every user's HTTP client from `builder`, each user with its own client
    /// and its own cookie jar. The users that run the
    /// [`test_start`](#method.test_start) and [`test_stop`](#method.test_stop)
    /// transactions get one too.
    ///
    /// The builder's user agent replaces Goose's `goose/<version>`. The timeout,
    /// gzip and invalid certificate settings also have a configuration option
    /// (`--timeout`, `--no-gzip`, `--accept-invalid-certs`), and each comes from the
    /// first of:
    ///
    /// 1. the option, when it was given on the command line;
    /// 2. the builder, when it was given that setting;
    /// 3. a [`GooseDefault`](./config/enum.GooseDefault.html) for the option;
    /// 4. Goose's built in default: a 60 second timeout, gzip on, certificates
    ///    checked.
    ///
    /// A transaction can still give its user a different client with
    /// [`GooseUser::set_client_builder`](./goose/struct.GooseUser.html#method.set_client_builder).
    ///
    /// This and
    /// [`set_client_builder_without_cookies`](#method.set_client_builder_without_cookies)
    /// replace each other: calling either one again, or the other, means the last call
    /// wins.
    ///
    /// # Errors
    ///
    /// [`GooseError::FeatureNotEnabled`] when Goose is built without the `cookies`
    /// feature, as the clients could not store cookies.
    ///
    /// # Example
    /// ```rust
    /// # #[cfg(feature = "cookies")]
    /// # {
    /// use goose::prelude::*;
    /// use goose::client::GooseClientBuilder;
    /// use std::time::Duration;
    ///
    /// let custom_client = GooseClientBuilder::new()
    ///     .timeout(Duration::from_secs(30))
    ///     .user_agent("custom-agent");
    ///
    /// let _goose_attack = GooseAttack::initialize()?
    ///     .set_client_builder_with_cookies(custom_client)?;
    /// # }
    /// # Ok::<(), goose::GooseError>(())
    /// ```
    pub fn set_client_builder_with_cookies(
        mut self,
        builder: client::GooseClientBuilder<client::CookiesEnabled>,
    ) -> Result<Self, GooseError> {
        if !cfg!(feature = "cookies") {
            return Err(GooseError::FeatureNotEnabled {
                feature: "cookies".to_string(),
                detail: "`set_client_builder_with_cookies` needs the `cookies` feature to give each user a cookie jar (rebuild with `--features cookies`), or use `set_client_builder_without_cookies`".to_string(),
            });
        }
        self.client_builder = Some(builder.into_settings());
        Ok(self)
    }

    /// Build one HTTP client from `builder`, without a cookie store, and share it
    /// across every user.
    ///
    /// The client is built once, before any user starts, and each user gets a clone
    /// of it; clones of a [`reqwest::Client`] share one connection pool, so this suits
    /// a load test with many users that needs no cookies. Users a Controller adds
    /// later, and the users that run the [`test_start`](#method.test_start) and
    /// [`test_stop`](#method.test_stop) transactions, share it too. When building
    /// the client fails, the load test fails before any user starts, with that error.
    ///
    /// The builder's settings, and the command line options that win over them, are
    /// as for
    /// [`set_client_builder_with_cookies`](#method.set_client_builder_with_cookies).
    /// A transaction that calls
    /// [`GooseUser::set_client_builder`](./goose/struct.GooseUser.html#method.set_client_builder)
    /// moves its user off the shared client and onto a client of its own.
    ///
    /// This and
    /// [`set_client_builder_with_cookies`](#method.set_client_builder_with_cookies)
    /// replace each other: calling either one again, or the other, means the last call
    /// wins.
    ///
    /// # Errors
    ///
    /// None at present: the client is built when the load test starts, and an error
    /// building it is returned from
    /// [`execute`](#method.execute).
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    /// use goose::client::GooseClientBuilder;
    /// use std::time::Duration;
    ///
    /// let custom_client = GooseClientBuilder::new()
    ///     .without_cookies()
    ///     .timeout(Duration::from_secs(30))
    ///     .user_agent("custom-agent");
    ///
    /// let _goose_attack = GooseAttack::initialize()?
    ///     .set_client_builder_without_cookies(custom_client)?;
    /// # Ok::<(), goose::GooseError>(())
    /// ```
    pub fn set_client_builder_without_cookies(
        mut self,
        builder: client::GooseClientBuilder<client::CookiesDisabled>,
    ) -> Result<Self, GooseError> {
        self.client_builder = Some(builder.into_settings());
        Ok(self)
    }

    /// Resolve the client builder's settings against the configured
    /// configuration and build how users get their clients: a shared client is
    /// built here.
    fn build_client_strategy(
        &mut self,
        command_line_client_options: client::CommandLineClientOptions,
    ) -> Result<(), GooseError> {
        self.client_strategy = match &self.client_builder {
            Some(builder) => Some(
                builder
                    .resolve(command_line_client_options, &self.configuration)
                    .into_strategy()?,
            ),
            None => None,
        };
        Ok(())
    }

    /// The `--scenarios` patterns that match no scenario's machine name.
    fn unmatched_scenario_patterns(&self) -> Vec<&str> {
        self.configuration
            .scenarios()
            .iter()
            .filter(|pattern| {
                !self
                    .scenarios
                    .iter()
                    .any(|scenario| scenario_pattern_matches(pattern, &scenario.machine_name))
            })
            .map(String::as_str)
            .collect()
    }

    /// Internal helper to determine if the scenario is currently active.
    fn scenario_is_active(&self, scenario: &Scenario) -> bool {
        let patterns = self.configuration.scenarios();
        if patterns.is_empty() {
            return true;
        }
        patterns
            .iter()
            .any(|pattern| scenario_pattern_matches(pattern, &scenario.machine_name))
    }

    /// How the `--gaggle-shadow-merge` self check ended: one line on a
    /// match; otherwise every difference written to
    /// `gaggle-shadow-diff.txt` in the run's folder, or the working
    /// directory without one, and an error.
    #[cfg(feature = "gaggle")]
    fn gaggle_shadow_report(&mut self) -> Result<(), GooseError> {
        let Some(slot) = self.gaggle_shadow.take() else {
            return Ok(());
        };
        let outcome = slot.lock().ok().and_then(|mut outcome| outcome.take());
        #[cfg(test)]
        if let Ok(mut last) = GAGGLE_SHADOW_LAST.lock() {
            last.clone_from(&outcome);
        }
        let differences = match outcome {
            Some(outcome) if outcome.differences.is_empty() => {
                println!("{}", outcome.summary());
                return Ok(());
            }
            Some(outcome) => outcome.differences,
            None => vec!["the shadow merge did not finish".to_string()],
        };
        let folder = self.output.folder.as_ref().map_or_else(
            || std::path::PathBuf::from("."),
            |folder| folder.path.clone(),
        );
        let path = folder.join(gaggle::shadow::DIFF_FILE);
        let mut text = differences.join("\n");
        text.push('\n');
        std::fs::write(&path, text)?;
        Err(GooseError::Io(io::Error::other(format!(
            "gaggle shadow merge: {} differences, written to {}",
            differences.len(),
            path.display()
        ))))
    }

    /// Use configured GooseScheduler to build out a properly weighted list of
    /// [`Scenario`](./goose/struct.Scenario.html)s to be assigned to
    /// [`GooseUser`](./goose/struct.GooseUser.html)s
    fn allocate_scenarios(&mut self) -> Vec<usize> {
        trace!("allocate_scenarios");

        let mut u: usize = 0;
        let mut v: usize;
        for scenario in &self.scenarios {
            if self.scenario_is_active(scenario) {
                if u == 0 {
                    u = scenario.weight;
                } else {
                    v = scenario.weight;
                    trace!("calculating greatest common denominator of {u} and {v}");
                    u = util::gcd(u, v);
                    trace!("inner gcd: {u}");
                }
            }
        }
        // 'u' will always be the greatest common divisor
        debug!("gcd: {u}");

        // Build a vector of vectors to be used to schedule users.
        let mut available_scenarios = Vec::with_capacity(self.scenarios.len());
        let mut total_scenarios = 0;
        for (index, scenario) in self.scenarios.iter().enumerate() {
            if self.scenario_is_active(scenario) {
                // divide by greatest common divisor so vector is as short as possible
                let weight = scenario.weight / u;
                trace!(
                    "{}: {} has weight of {} (reduced with gcd to {})",
                    index,
                    scenario.name,
                    scenario.weight,
                    weight
                );
                let weighted_sets = vec![index; weight];
                total_scenarios += weight;
                available_scenarios.push(weighted_sets);
            }
        }

        info!(
            "[scheduler]: allocating transactions and scenarios with {:?} scheduler",
            self.scheduler
        );

        // Now build the weighted list with the appropriate scheduler.
        let mut weighted_scenarios = Vec::new();
        match self.scheduler {
            GooseScheduler::RoundRobin => {
                // Allocate scenarios round robin.
                let scenarios_len = available_scenarios.len();
                loop {
                    for (scenario_index, scenarios) in available_scenarios
                        .iter_mut()
                        .enumerate()
                        .take(scenarios_len)
                    {
                        if let Some(scenario) = scenarios.pop() {
                            debug!("[scheduler]: allocating 1 user from Scenario {scenario_index}");
                            weighted_scenarios.push(scenario);
                        }
                    }
                    if weighted_scenarios.len() >= total_scenarios {
                        break;
                    }
                }
            }
            GooseScheduler::Serial => {
                // Allocate scenarios serially in the weighted order defined.
                for (scenario_index, scenarios) in available_scenarios.iter().enumerate() {
                    debug!(
                        "allocating all {} users from Scenario {}",
                        scenarios.len(),
                        scenario_index,
                    );
                    weighted_scenarios.append(&mut scenarios.clone());
                }
            }
            GooseScheduler::Random => {
                // Allocate scenarios randomly.
                loop {
                    let scenario = available_scenarios.choose_mut(&mut rand::rng());
                    match scenario {
                        Some(set) => {
                            if let Some(s) = set.pop() {
                                weighted_scenarios.push(s);
                            }
                        }
                        None => warn!("randomly allocating a Scenario failed, trying again"),
                    }
                    if weighted_scenarios.len() >= total_scenarios {
                        break;
                    }
                }
            }
        }
        weighted_scenarios
    }

    /// The index in `scenarios` of each of `total_users` users, in the order
    /// they are built; they are popped, so the last one built starts first.
    /// The weighted scenarios repeat until there are enough, or, for a share
    /// of a Gaggle's users, they are its slice, which must hold exactly
    /// `total_users`.
    fn user_scenario_order(&mut self, total_users: usize) -> Result<Vec<usize>, GooseError> {
        #[cfg(feature = "gaggle")]
        if let Some(slice) = self.user_slice.as_ref() {
            if slice.len() != total_users {
                return Err(GooseError::InvalidOption {
                    option: "--users".to_string(),
                    value: total_users.to_string(),
                    detail: format!(
                        "this Gaggle worker's share is {} users, not {}",
                        slice.len(),
                        total_users
                    ),
                });
            }
            return Ok(slice.iter().rev().copied().collect());
        }
        let weighted_scenarios = self.allocate_scenarios();
        if weighted_scenarios.is_empty() {
            return Ok(Vec::new());
        }
        Ok(weighted_scenarios
            .iter()
            .copied()
            .cycle()
            .take(total_users)
            .collect())
    }

    /// The index in `scenarios` of each of `total_users` users one process
    /// running them all would start, by global ordinal: the first user
    /// started is ordinal 0. Under the `Random` scheduler each call draws
    /// again, so a Gaggle draws once and hands each worker its slice.
    // Called by the Gaggle manager in step 2; tested here.
    #[cfg(feature = "gaggle")]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn user_scenarios(&mut self, total_users: usize) -> Result<Vec<usize>, GooseError> {
        let saved = self.user_slice.take();
        let order = self.user_scenario_order(total_users);
        self.user_slice = saved;
        let mut order = order?;
        order.reverse();
        Ok(order)
    }

    /// Run the users with global ordinals `offset` on, whose index in
    /// `scenarios` is each of `slice` in turn: a Gaggle worker's share of a
    /// load test, for any scheduler.
    // Called by a Gaggle worker in step 2; tested here.
    #[cfg(feature = "gaggle")]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn set_user_slice(&mut self, offset: usize, slice: Vec<usize>) {
        self.user_offset = offset;
        self.user_slice = Some(slice);
    }

    /// Run `users` users from global ordinal `offset` of a load test of
    /// `total` users, as one process running them all would assign them
    /// scenarios: for the `RoundRobin` and `Serial` schedulers, which
    /// allocate the same way every time. A share past the last user is an
    /// error.
    // Called by a Gaggle worker in step 2; tested here.
    #[cfg(feature = "gaggle")]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn set_user_ordinals(
        &mut self,
        offset: usize,
        users: usize,
        total: usize,
    ) -> Result<(), GooseError> {
        let end = offset.checked_add(users).filter(|&end| end <= total);
        let Some(end) = end else {
            return Err(GooseError::InvalidOption {
                option: "--users".to_string(),
                value: users.to_string(),
                detail: format!(
                    "a Gaggle worker's {users} users from ordinal {offset} run past the last of {total}"
                ),
            });
        };
        let all = self.user_scenarios(total)?;
        let slice = all[offset.min(all.len())..end.min(all.len())].to_vec();
        self.set_user_slice(offset, slice);
        Ok(())
    }

    /// Pre-allocate a vector of weighted [`GooseUser`](./goose/struct.GooseUser.html)s.
    fn weight_scenario_users(&mut self, total_users: usize) -> Result<Vec<GooseUser>, GooseError> {
        trace!("weight_scenario_users");

        let weighted_scenarios = self.user_scenario_order(total_users)?;

        // Allocate a state for each user that will be launched.
        info!(
            "[scheduler]: initializing {} user states...",
            self.test_plan.total_users()
        );

        // Gather the parameters needed for each user. This is cheap and must be
        // sequential to preserve the round-robin scenario assignment order (users
        // are later .pop()'d so ordering matters).
        let config_host = self.get_configuration_host();
        let defaults_host = self.defaults.host.clone();
        let mut user_params: Vec<(usize, Arc<str>, url::Url)> = Vec::with_capacity(total_users);
        for scenarios_index in &weighted_scenarios {
            let base_url = goose::get_base_url(
                config_host.clone(),
                self.scenarios[*scenarios_index].host.clone(),
                defaults_host.clone(),
            )?;
            user_params.push((
                self.scenarios[*scenarios_index].scenarios_index,
                self.scenarios[*scenarios_index].machine_name.clone(),
                base_url,
            ));
        }

        // Create GooseUser objects in parallel across available CPUs. Each user
        // gets its own reqwest client unless one is shared, and building it is
        // the expensive part. Chunks are collected back in order to preserve the
        // scenario assignment sequence.
        let configuration = &self.configuration;
        let client_strategy = &self.client_strategy;
        let metrics_hash = self.metrics.hash;
        let num_cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        let chunk_size = total_users.div_ceil(num_cpus);

        let results: Vec<Result<Vec<GooseUser>, GooseError>> = std::thread::scope(|s| {
            let mut handles = Vec::with_capacity(num_cpus);
            for (chunk_idx, chunk) in user_params.chunks(chunk_size).enumerate() {
                let base = chunk_idx * chunk_size;
                handles.push(s.spawn(move || {
                    let mut users = Vec::with_capacity(chunk.len());
                    for (i, (scenarios_index, machine_name, base_url)) in chunk.iter().enumerate() {
                        debug!("creating user state: {} ({})", base + i, scenarios_index);
                        let reqwest_client = match client_strategy {
                            None => goose::create_reqwest_client(configuration)?,
                            Some(strategy) => strategy.user_client()?,
                        };
                        users.push(GooseUser::new(
                            *scenarios_index,
                            machine_name.clone(),
                            base_url.clone(),
                            configuration,
                            metrics_hash,
                            Some(reqwest_client),
                        )?);
                    }
                    Ok::<Vec<GooseUser>, GooseError>(users)
                }));
            }
            handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or_else(|_| {
                        Err(GooseError::Io(io::Error::other(
                            "user initialization thread panicked",
                        )))
                    })
                })
                .collect()
        });

        let mut weighted_users = Vec::with_capacity(total_users);
        for result in results {
            weighted_users.extend(result?);
        }
        debug!(
            "[scheduler]: created {} weighted_users",
            weighted_users.len()
        );
        Ok(weighted_users)
    }

    // Change from one attack_phase to another.
    fn set_attack_phase(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        phase: AttackPhase,
    ) {
        // There's nothing to do if already in the specified phase.
        if self.attack_phase == phase {
            return;
        }

        // The drift timer starts at 0 any time the phase is changed.
        goose_attack_run_state.drift_timer = tokio::time::Instant::now();

        // Reaching Idle ends any cancel in progress, and the run's shape.
        if phase == AttackPhase::Idle {
            goose_attack_run_state.stopping = false;
            self.run_shape = None;
        }

        // Thresholds are checked from when the run first holds its users steady,
        // so a ramp up never fails a rule while the run goes.
        if goose_attack_run_state.threshold_check_at.is_none() && phase == AttackPhase::Maintain {
            self.start_threshold_checks(goose_attack_run_state);
        }

        // `rps` rules count only the time the run holds its users steady, in every
        // mode, so each entry to Maintain starts a hold and each exit ends it.
        let holds = |phase| phase == AttackPhase::Maintain;
        if holds(phase) != holds(self.attack_phase)
            && self
                .configuration
                .thresholds
                .iter()
                .any(threshold::Threshold::is_rps)
        {
            self.update_duration();
            let seconds = self.elapsed;
            let _ = goose_attack_run_state.metrics_cmd_tx.send(if holds(phase) {
                MetricsCommand::EnterMaintain { seconds }
            } else {
                MetricsCommand::LeaveMaintain { seconds }
            });
        }

        // Optional debug output.
        info!("entering GooseAttack phase: {:?}", phase);

        // Update the current phase.
        self.attack_phase = phase;
    }

    // Display all scenarios (sorted by machine name).
    fn print_scenarios(&self) {
        print!("{}", self.scenarios_list());
    }

    /// What `--scenarios-list` prints: each scenario's machine name, which
    /// `--scenarios` selects it by, and its name.
    fn scenarios_list(&self) -> String {
        let mut scenarios = BTreeMap::new();
        for scenario in &self.scenarios {
            scenarios.insert(scenario.machine_name.clone(), scenario.name.clone());
        }
        // Display sorted by machine_name.
        let mut list = "Scenarios:\n".to_string();
        for (key, name) in scenarios {
            list.push_str(&format!(" - {}: (\"{}\")\n", key, name));
        }
        list
    }

    /// Execute the [`GooseAttack`](./struct.GooseAttack.html) load test.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), GooseError> {
    ///     let _goose_metrics = GooseAttack::initialize()?
    ///         .register_scenario(scenario!("ExampleTransaction")
    ///             .register_transaction(transaction!(example_transaction).set_weight(2)?)
    ///             .register_transaction(transaction!(another_example_transaction).set_weight(3)?)
    ///             // Goose must run against a host, point to localhost so test starts.
    ///             .set_host("http://localhost")
    ///         )
    ///         // Exit after one second so test doesn't run forever.
    ///         .set_default(GooseDefault::RunTime, 1)?
    ///         .execute()
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn example_transaction(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.get("/foo").await?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn another_example_transaction(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.get("/bar").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn execute(self) -> Result<GooseMetrics, GooseError> {
        /// Ends a gzipped Goose log's member on every way out of `execute()`:
        /// a return, a panic unwinding through it, or its future dropped.
        struct FinishGooseLog;
        impl Drop for FinishGooseLog {
            fn drop(&mut self) {
                logger::finish_goose_log();
            }
        }
        logger::start_goose_log();
        let _finish = FinishGooseLog;
        self.execute_load_test().await
    }

    /// [`Self::execute`], before the Goose log is finished.
    async fn execute_load_test(mut self) -> Result<GooseMetrics, GooseError> {
        // If version flag is set, display package name and version and exit.
        if self.configuration.version {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            logger::finish_goose_log();
            std::process::exit(0);
        }

        // At least one scenario is required.
        if self.scenarios.is_empty() {
            return Err(GooseError::NoScenarios {
                detail: "No scenarios are defined.".to_string(),
            });
        }

        // Display scenarios and transactions, then exit.
        if self.configuration.list {
            println!("Available transactions:");
            for scenario in self.scenarios {
                println!(" - {} (weight: {})", scenario.name, scenario.weight);
                for transaction in scenario.transactions {
                    println!(
                        "    o {} (weight: {})",
                        transaction.name.name_for_transaction(),
                        transaction.weight
                    );
                }
            }
            logger::finish_goose_log();
            std::process::exit(0);
        }

        // Note which client options the command line gave before the defaults
        // are merged in: those win over a client builder, the defaults do not.
        let command_line_client_options =
            client::CommandLineClientOptions::from_command_line(&self.configuration);

        // A worker takes every option that shapes the load from its manager.
        #[cfg(feature = "gaggle")]
        self.configuration.worker_refusals()?;

        // Configure GooseConfiguration.
        self.configuration.configure(&self.defaults)?;

        // Validate GooseConfiguration.
        self.configuration.validate()?;
        self.output.save_report = self.configuration.save_report_at_start();
        self.output.graph_data = self.configuration.report_graph_data;

        // Write reports from a JSON report, then exit.
        if let Some(path) = self.configuration.render_report().map(str::to_string) {
            return self.render_report(&path).await;
        }

        // Display scenarios, then exit.
        if self.configuration.scenarios_list {
            self.print_scenarios();
            logger::finish_goose_log();
            std::process::exit(0);
        }

        // A pattern that names no scenario is most likely a mistyped name, as
        // `user_login` for the scenario `userlogin`.
        for pattern in self.unmatched_scenario_patterns() {
            warn!("--scenarios {pattern:?} matches no scenario; --scenarios-list prints the names");
        }

        // At least one scenario must be active.
        let mut active_scenario: bool = false;
        for scenario in &self.scenarios {
            if self.scenario_is_active(scenario) {
                active_scenario = true;
                break;
            }
        }
        if !active_scenario {
            self.print_scenarios();
            return Err(GooseError::NoScenarios {
                detail: "No scenarios are enabled.".to_string(),
            });
        }

        // A worker joins its manager, and runs the share it is handed.
        #[cfg(feature = "gaggle")]
        if self.configuration.is_worker() {
            self.gaggle_worker_join().await?;
        }

        // Build TestPlan, and the shape of a users ramp.
        self.test_plan = TestPlan::build(&self.configuration)?;
        self.run_shape = run_shape::RunShape::for_configuration(&self.configuration);

        // A client builder gives users clients built from its settings. Build
        // them, or the one shared client, now: before any user starts, and once
        // for the whole run, so users a Controller adds later share it too.
        self.build_client_strategy(command_line_client_options)?;

        // With a validated GooseConfiguration, enter a run mode.
        #[cfg(feature = "gaggle")]
        if self.configuration.is_manager() {
            self.attack_mode = AttackMode::Manager;
        } else if !self.configuration.is_worker() {
            self.attack_mode = AttackMode::StandAlone;
        }
        #[cfg(not(feature = "gaggle"))]
        {
            self.attack_mode = AttackMode::StandAlone;
        }

        // When --no-autostart is set, defer prepare_load_test() until the Controller
        // sends a Start command. Otherwise user states are initialized here and then
        // redundantly re-initialized when the Start command calls prepare_load_test().
        if self.configuration.no_autostart {
            if self.validate_host().is_err() {
                info!("host must be configured via Controller before starting load test");
            } else if let Some(host) = self.configuration.host() {
                info!("global host configured: {host}");
            }
        } else {
            // If configuration.host is empty, then it will fall back to per-scenario
            // defaults if set.
            if let Some(host) = self.configuration.host() {
                info!("global host configured: {host}");
            }
            let prepared = self.prepare_load_test();
            // A worker tells its manager whether its users are ready.
            #[cfg(feature = "gaggle")]
            self.gaggle_worker_prepared(prepared.as_ref().err());
            prepared?;
        }

        // Calculate a unique hash for the current load test.
        self.metrics.hash = plan_hash(&self.scenarios, &self.scheduler);
        debug!("hash: {}", self.metrics.hash);

        // A manager waits for its workers, prepares them, and starts its own
        // run with theirs.
        #[cfg(feature = "gaggle")]
        if self.gaggle_manager() {
            self.gaggle_manager_start().await?;
        }

        self = self.start_attack().await?;

        #[cfg(feature = "gaggle")]
        self.gaggle_shadow_report()?;

        if self.metrics.display_metrics {
            info!(
                "printing final metrics after {} seconds...",
                self.metrics.duration
            );
            print!("{}", self.metrics);
        }

        // Every report is written: fail the run when a threshold failed.
        let failed: Vec<threshold::ThresholdResult> = self
            .metrics
            .thresholds
            .iter()
            .filter(|result| !result.passed)
            .cloned()
            .collect();
        if !failed.is_empty() {
            for result in &failed {
                error!(
                    "threshold {} failed: observed {}",
                    result.rule, result.observed
                );
            }
            return Err(GooseError::ThresholdsFailed {
                failed,
                metrics: ThresholdMetrics(Box::new(self.metrics)),
            });
        }

        Ok(self.metrics)
    }

    /// Start checking the thresholds, the first check `--threshold-abort-delay`
    /// from now, when a failing one stops the run.
    pub(crate) fn start_threshold_checks(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) {
        if self.configuration.aborts_on_thresholds() {
            let delay = self.configuration.threshold_abort_delay_secs() as u64;
            goose_attack_run_state.threshold_check_at =
                std::time::Instant::now().checked_add(Duration::from_secs(delay));
        }
    }

    /// Once a second while the thresholds are checked, and only while users start
    /// or run, ask the metrics processor to check them (fire-and-forget). Nothing is
    /// sent without thresholds or under `--no-threshold-abort`.
    fn check_thresholds(&mut self, goose_attack_run_state: &mut GooseAttackRunState) {
        let Some(due) = goose_attack_run_state.threshold_check_at else {
            return;
        };
        if !matches!(
            self.attack_phase,
            AttackPhase::Increase | AttackPhase::Maintain
        ) {
            return;
        }
        let now = std::time::Instant::now();
        if now < due {
            return;
        }
        goose_attack_run_state.threshold_check_at = Some(now + Duration::from_secs(1));
        self.update_duration();
        let _ = goose_attack_run_state
            .metrics_cmd_tx
            .send(MetricsCommand::CheckThresholds {
                duration: self.metrics.duration,
                seconds: self.elapsed,
            });
    }

    // Returns OK(()) if there's a valid host, GooseError with details if not.
    // Every Start checks it before it replies: a scenario, and a test_start
    // or test_stop transaction, which run with no scenario host, each need
    // one.
    fn validate_host(&mut self) -> Result<(), GooseError> {
        if self.configuration.host().is_none() && self.defaults.host.is_none() {
            let transaction = if self.test_start_transaction.is_some() {
                Some(HOST_TRANSACTIONS[0])
            } else if self.test_stop_transaction.is_some() {
                Some(HOST_TRANSACTIONS[1])
            } else {
                None
            };
            if let Some(transaction) = transaction {
                return Err(GooseError::InvalidOption {
                    option: NO_HOST_OPTION.to_string(),
                    value: "".to_string(),
                    detail: no_transaction_host_detail(transaction),
                });
            }
        }
        if self.configuration.host().is_none() {
            for scenario in &self.scenarios {
                match &scenario.host {
                    Some(h) => {
                        if util::is_valid_host(h).is_ok() {
                            info!("host for {} configured: {}", scenario.name, h);
                        }
                    }
                    None => match &self.defaults.host {
                        Some(h) => {
                            if util::is_valid_host(h).is_ok() {
                                info!("host for {} configured: {}", scenario.name, h);
                            }
                        }
                        None => {
                            return Err(GooseError::InvalidOption {
                                option: NO_HOST_OPTION.to_string(),
                                value: "".to_string(),
                                detail: format!(
                                    "A host must be defined via the --host option, the GooseAttack.set_default() function, or the Scenario.set_host() function (no host defined for {}).",
                                    scenario.name
                                ),
                            });
                        }
                    },
                }
            }
        }
        Ok(())
    }

    // Create and schedule GooseUsers. This requires that the host that will be load tested
    // has been configured.
    fn prepare_load_test(&mut self) -> Result<(), GooseError> {
        // Be sure a valid host has been defined before building configuration.
        self.validate_host()?;

        // A Gaggle worker may hold no users, when there are more workers than
        // users.
        #[cfg(feature = "gaggle")]
        let share_of_none = self.gaggle_worker();
        #[cfg(not(feature = "gaggle"))]
        let share_of_none = false;

        // A load test with no users never starts: weighting zero users never ends.
        if self.test_plan.total_users() == 0 && !share_of_none {
            return Err(GooseError::InvalidOption {
                option: NO_USERS_OPTION.to_string(),
                value: "0".to_string(),
                detail: "There are no users to start: configure users or a test plan.".to_string(),
            });
        }

        // Apply weights to transactions in each scenario.
        for scenario in &mut self.scenarios {
            let (
                weighted_on_start_transactions,
                weighted_transactions,
                weighted_on_stop_transactions,
            ) = allocate_transactions(scenario, &self.scheduler);
            scenario.weighted_on_start_transactions = weighted_on_start_transactions;
            scenario.weighted_transactions = weighted_transactions;
            scenario.weighted_on_stop_transactions = weighted_on_stop_transactions;
            debug!(
                "weighted {} on_start: {:?} transactions: {:?} on_stop: {:?}",
                scenario.name,
                scenario.weighted_on_start_transactions,
                scenario.weighted_transactions,
                scenario.weighted_on_stop_transactions
            );
        }

        // Stand-alone processes can display metrics.
        if !self.configuration.no_metrics && !self.configuration.no_print_metrics {
            self.metrics.display_metrics = true;
        }

        // A Gaggle manager runs no users of its own.
        #[cfg(feature = "gaggle")]
        if self.gaggle_manager() {
            return Ok(());
        }

        // Allocate a state for each of the users we are about to start.
        self.weighted_users = self.weight_scenario_users(self.test_plan.total_users())?;

        Ok(())
    }

    /// Helper to wrap configured host in `Option<>` if set.
    fn get_configuration_host(&self) -> Option<String> {
        self.configuration.host().map(str::to_string)
    }

    // Helper to spawn a throttle thread if configured. The throttle thread opens
    // a bounded channel to control how quickly [`GooseUser`](./goose/struct.GooseUser.html)
    // threads can make requests.
    async fn setup_throttle(
        &self,
    ) -> (
        // A channel used by [`GooseUser`](./goose/struct.GooseUser.html)s to throttle requests.
        Option<flume::Sender<bool>>,
        // A channel used by parent to tell throttle the load test is complete.
        Option<flume::Sender<bool>>,
    ) {
        // If the throttle isn't enabled, return immediately.
        let throttle_requests = self.configuration.throttle_requests();
        if throttle_requests == 0 {
            return (None, None);
        }

        // Create a bounded channel allowing single-sender multi-receiver to throttle
        // [`GooseUser`](./goose/struct.GooseUser.html) threads.
        let (all_threads_throttle, throttle_receiver): (
            flume::Sender<bool>,
            flume::Receiver<bool>,
        ) = flume::bounded(throttle_requests);

        // Create a channel allowing the parent to inform the throttle thread when the
        // load test is finished. Even though we only send one message, we can't use a
        // oneshot channel as we don't want to block waiting for a message.
        let (parent_to_throttle_tx, throttle_rx) = flume::bounded(1);

        // Launch a new thread for throttling, no need to rejoin it.
        let _ = Some(tokio::spawn(throttle::throttle_main(
            throttle_requests,
            throttle_receiver,
            throttle_rx,
        )));

        let sender = all_threads_throttle.clone();
        // We start from 1 instead of 0 to intentionally fill all but one slot in the
        // channel to avoid a burst of traffic during startup. The channel then provides
        // an implementation of the leaky bucket algorithm as a queue. Requests have to
        // add a token to the bucket before making a request, and are blocked until this
        // throttle thread "leaks out" a token thereby creating space. More information
        // can be found at: https://en.wikipedia.org/wiki/Leaky_bucket
        for _ in 1..throttle_requests {
            let _ = sender.send_async(true).await;
        }

        (Some(all_threads_throttle), Some(parent_to_throttle_tx))
    }

    // Helper to optionally spawn a telnet and/or WebSocket Controller thread. The Controller
    // threads send commands to the parent process on `control_tx`, each with a one-shot
    // channel for the reply.
    async fn setup_controllers(
        &self,
        control_tx: &flume::Sender<ControlRequest>,
    ) -> controller::ListeningControllers {
        let mut listening = controller::ListeningControllers::default();
        // Configured telnet Controller if not disabled.
        if !self.configuration.no_telnet {
            if let Some(listener) =
                controller::bind_controller(&self.configuration, ControllerProtocol::Telnet).await
            {
                // Spawn the initial controller thread to allow real-time control of the load test.
                // There is no need to rejoin this thread when the load test ends.
                tokio::spawn(controller::controller_main(
                    listener,
                    control_tx.clone(),
                    ControllerProtocol::Telnet,
                ));
                listening.telnet = true;
            }
        }

        // Configured WebSocket Controller if enabled with --websocket.
        if self.configuration.websocket {
            if let Some(listener) =
                controller::bind_controller(&self.configuration, ControllerProtocol::WebSocket)
                    .await
            {
                // Spawn the initial controller thread to allow real-time control of the load test.
                // There is no need to rejoin this thread when the load test ends.
                tokio::spawn(controller::controller_main(
                    listener,
                    control_tx.clone(),
                    ControllerProtocol::WebSocket,
                ));
                listening.websocket = true;
            }
        }
        listening
    }

    /// After an autostarted run, whether to stay up for the dashboard: Save
    /// report wrote the run's report, the run was not stopped or canceled on
    /// purpose, and any client is connected to the event stream.
    #[cfg(feature = "dashboard")]
    fn should_wait_for_dashboard(&self, goose_attack_run_state: &GooseAttackRunState) -> bool {
        let saved = self.output.last_report.is_some();
        let ended_on_its_own = matches!(
            self.ended_by,
            None | Some((runs::EndedBy::Completed, _)) | Some((runs::EndedBy::UsersExited, _))
        );
        let tab_open = goose_attack_run_state
            .dashboard_active_sse
            .as_ref()
            .is_some_and(|active| active.load(std::sync::atomic::Ordering::Relaxed) > 0);
        saved && ended_on_its_own && tab_open
    }

    /// Go Idle after an autostarted run instead of shutting down, as a
    /// dashboard Stop does, so Idle does not start the test again.
    #[cfg(feature = "dashboard")]
    fn start_dashboard_wait(&mut self, goose_attack_run_state: &mut GooseAttackRunState) {
        self.configuration.no_autostart = true;
        goose_attack_run_state.shutdown_after_stop = false;
        let address = goose_attack_run_state
            .dashboard_url
            .as_deref()
            .unwrap_or_default();
        if self.configuration.dashboard_control {
            println!(
                "Run finished. Download it from the dashboard at {address}. Press Quit in the dashboard or Ctrl-C to exit."
            );
        } else {
            println!(
                "Run finished. Download it from the dashboard at {address}. Press Ctrl-C to exit."
            );
        }
    }

    /// Shut down from Idle as a Controller `shutdown` does: no run to stop,
    /// so no metrics to print and no report to write.
    fn shutdown_from_idle(&mut self, goose_attack_run_state: &mut GooseAttackRunState) {
        self.metrics.display_metrics = false;
        goose_attack_run_state.shutdown_after_stop = true;
        self.stop_without_users(goose_attack_run_state);
    }

    /// Stop with no user running: refuse Users until the run stops, and leave
    /// a plan with nothing left to run, so the Decrease phase finds the load
    /// test fully stopped.
    fn stop_without_users(&mut self, goose_attack_run_state: &mut GooseAttackRunState) {
        goose_attack_run_state.stopping = true;
        self.run_shape = None;
        self.test_plan.steps = vec![(0, 0)];
        self.test_plan.current = 0;
        self.set_attack_phase(goose_attack_run_state, AttackPhase::Decrease);
    }

    /// Handle dashboard snapshot and series requests from the HTTP server task.
    ///
    /// Drains up to a small batch per main-loop iteration; at most one
    /// snapshot is forwarded per tick (latest wins). Dashboard control goes
    /// through [`Self::handle_control_requests`], called before this, so a
    /// snapshot forwarded here sees the phase a command in the same tick set.
    ///
    /// Snapshots are **non-blocking** for the attack loop: the parent forwards
    /// [`MetricsCommand::GetDashboardSnapshot`] with the hub's oneshot and does
    /// not await the metrics processor.
    ///
    /// When the metrics processor is unavailable (typical after control Stop
    /// merges final metrics and returns to Idle until the next Start), a
    /// snapshot is built from the main-loop [`GooseMetrics`] / [`GraphData`] so
    /// poll/SSE clients still observe `phase: "idle"` and can re-enable Start.
    #[cfg(feature = "dashboard")]
    async fn handle_dashboard_requests(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        let mut series_requests = Vec::new();
        let mut pending_snapshot: Option<(
            tokio::sync::oneshot::Sender<metrics::DashboardSnapshot>,
            Option<metrics::dashboard_snapshot::HeatmapCursor>,
        )> = None;
        let mut disconnected = false;

        {
            let Some(rx) = goose_attack_run_state.dashboard_channel_rx.as_ref() else {
                return Ok(());
            };

            for _ in 0..controller::MAX_DRAIN {
                match rx.try_recv() {
                    Ok(dashboard::DashboardRequest::GetSnapshot {
                        respond,
                        heatmap_since,
                    }) => {
                        // Coalesce: only the latest snapshot request is kept.
                        pending_snapshot = Some((respond, heatmap_since));
                    }
                    Ok(dashboard::DashboardRequest::GetRequestSeries {
                        method,
                        name,
                        since,
                        respond,
                    }) => {
                        series_requests.push((method, name, since, respond));
                    }
                    Err(flume::TryRecvError::Empty) => break,
                    Err(flume::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }

        if disconnected {
            goose_attack_run_state.dashboard_channel_rx = None;
        }

        // Forward at most one snapshot request without parking the attack loop.
        if let Some((respond, heatmap_since)) = pending_snapshot {
            if respond.is_closed() {
                debug!("[dashboard]: snapshot skipped; hub waiter disconnected");
            } else {
                self.update_duration();
                let phase = attack_phase_str(self.attack_phase).to_string();
                let active_users = goose_attack_run_state.active_users;
                let target_users = self.current_target_users();
                match goose_attack_run_state.metrics_cmd_tx.send(
                    MetricsCommand::GetDashboardSnapshot {
                        duration: self.metrics.duration,
                        total_users: self.metrics.total_users,
                        maximum_users: self.metrics.maximum_users,
                        active_users,
                        target_users,
                        series_elapsed_secs: self.series_elapsed_secs(),
                        phase: phase.clone(),
                        stopping: goose_attack_run_state.stopping,
                        series_window_secs: metrics::dashboard_snapshot::SERIES_WINDOW_SECS,
                        report: self.report_snapshot(),
                        settings: Box::new(self.snapshot_settings(goose_attack_run_state)),
                        heatmap_since,
                        respond,
                    },
                ) {
                    Ok(()) => {}
                    // Processor gone (e.g. Idle after control Stop): serve from
                    // main-loop state so the SPA can observe idle and re-Start.
                    Err(flume::SendError(MetricsCommand::GetDashboardSnapshot {
                        respond,
                        phase,
                        stopping,
                        active_users,
                        settings,
                        heatmap_since,
                        ..
                    })) => {
                        debug!(
                            "[dashboard]: metrics processor unavailable; serving local snapshot (phase={phase})"
                        );
                        let snapshot = self.build_local_dashboard_snapshot(
                            phase,
                            stopping,
                            active_users,
                            *settings,
                            heatmap_since,
                        );
                        let _ = respond.send(snapshot);
                    }
                    Err(_) => {
                        // Other command variants cannot appear here.
                        debug!("[dashboard]: metrics processor unavailable");
                    }
                }
            }
        }

        // Then each series request, also without waiting for the processor.
        for (method, name, since, respond) in series_requests {
            if respond.is_closed() {
                continue;
            }
            match goose_attack_run_state
                .metrics_cmd_tx
                .send(MetricsCommand::GetRequestSeries {
                    method,
                    name,
                    series_elapsed_secs: self.series_elapsed_secs(),
                    since,
                    respond,
                }) {
                Ok(()) => {}
                // Processor gone (e.g. Idle after control Stop): answer from
                // main-loop state, as for snapshots.
                Err(flume::SendError(MetricsCommand::GetRequestSeries {
                    method,
                    name,
                    series_elapsed_secs,
                    since,
                    respond,
                })) => {
                    let series = self.graph_data.export_request_series(
                        method,
                        name,
                        metrics::dashboard_snapshot::SERIES_WINDOW_SECS,
                        Some(series_elapsed_secs),
                        self.metrics
                            .coordinated_omission_metrics
                            .as_ref()
                            .is_some_and(|m| m.has_events()),
                        since,
                    );
                    let _ = respond.send(series);
                }
                Err(_) => {
                    // Other command variants cannot appear here.
                    debug!("[dashboard]: metrics processor unavailable");
                }
            }
        }

        Ok(())
    }

    /// Seconds on the continuous series clock (0 if the run has not started).
    fn series_elapsed_secs(&self) -> usize {
        self.series_started
            .map(|started| started.elapsed().as_secs() as usize)
            .unwrap_or(0)
    }

    /// Users the attack is currently increasing/decreasing toward.
    ///
    /// Prefers the active test-plan step target; falls back to the last plan
    /// step when `current` has advanced past the end (common after the final
    /// step / cancel), then to configured `--users` when the plan is empty.
    fn current_target_users(&self) -> usize {
        self.test_plan
            .steps
            .get(self.test_plan.current)
            .or_else(|| self.test_plan.steps.last())
            .map(|&(users, _)| users)
            .or(self.configuration.users)
            .unwrap_or_default()
    }

    /// Build a dashboard snapshot from main-loop metrics/graph state.
    ///
    /// Used when the dedicated metrics processor is not running (Idle after
    /// Stop) so clients still receive a coherent phase/users view.
    #[cfg(feature = "dashboard")]
    fn build_local_dashboard_snapshot(
        &mut self,
        phase: String,
        stopping: bool,
        active_users: usize,
        settings: metrics::dashboard_snapshot::SnapshotSettings,
        heatmap_since: Option<metrics::dashboard_snapshot::HeatmapCursor>,
    ) -> metrics::DashboardSnapshot {
        // Main-loop graph_data is usually empty until Shutdown merge. Export with
        // series-clock "now" so any retained users level holds to the right edge.
        let series = self.graph_data.export_series_window_with_heatmap(
            metrics::dashboard_snapshot::SERIES_WINDOW_SECS,
            Some(self.series_elapsed_secs()),
            self.metrics
                .coordinated_omission_metrics
                .as_ref()
                .is_some_and(|m| m.has_events()),
            heatmap_since,
        );
        metrics::dashboard_snapshot::build_dashboard_snapshot(
            metrics::dashboard_snapshot::DashboardSnapshotInput {
                metrics: &self.metrics,
                series,
                active_users,
                maximum_users: self.metrics.maximum_users,
                target_users: self.current_target_users(),
                total_users: self.metrics.total_users,
                phase,
                stopping,
                series_window_secs: metrics::dashboard_snapshot::SERIES_WINDOW_SECS,
                no_status_codes: self.configuration.no_status_codes,
                metrics_disabled: self.configuration.no_metrics,
                no_transaction_metrics: self.configuration.no_transaction_metrics,
                no_scenario_metrics: self.configuration.no_scenario_metrics,
                report: self.report_snapshot(),
                settings,
                percentiles_over_time: self.configuration.records_latency_series(),
                series_keys: self.graph_data.latency_series_keys(),
            },
        )
    }

    /// Where the run is: Idle, stopping (a cancel, or Shutdown), a running
    /// test plan, or a users ramp's stage from its shape.
    #[cfg(any(test, feature = "dashboard"))]
    fn stage(&self, goose_attack_run_state: &GooseAttackRunState) -> run_shape::Stage {
        match self.attack_phase {
            AttackPhase::Idle => run_shape::Stage::Idle,
            AttackPhase::Shutdown => run_shape::Stage::Stopping,
            _ if goose_attack_run_state.stopping => run_shape::Stage::Stopping,
            _ => self
                .run_shape
                .as_ref()
                .map_or(run_shape::Stage::TestPlan, run_shape::RunShape::stage),
        }
    }

    /// The snapshot's run settings: the configured settings, built once
    /// after each control command, plus the stage and the hold's countdown,
    /// computed for each snapshot.
    #[cfg(feature = "dashboard")]
    fn snapshot_settings(
        &mut self,
        goose_attack_run_state: &GooseAttackRunState,
    ) -> metrics::dashboard_snapshot::SnapshotSettings {
        let stage = self.stage(goose_attack_run_state);
        let hold_ends_in_secs = match stage {
            run_shape::Stage::Hold => self
                .run_shape
                .as_ref()
                .and_then(|shape| shape.hold_ends_in_secs(time::Instant::now())),
            _ => None,
        };
        let mut settings = match &self.settings {
            Some(settings) => settings.clone(),
            None => {
                let settings = self.configured_settings();
                self.settings = Some(settings.clone());
                settings
            }
        };
        settings.stage = stage.as_str().to_string();
        settings.hold_ends_in_secs = hold_ends_in_secs;
        settings
    }

    /// The configured run settings, without the stage and countdown.
    #[cfg(feature = "dashboard")]
    fn configured_settings(&self) -> metrics::dashboard_snapshot::SnapshotSettings {
        let configuration = &self.configuration;
        // Resolved per scenario as a Start resolves them, without
        // `get_base_url`: a scenario with no host from any source adds none.
        let hosts: std::collections::BTreeSet<String> = self
            .scenarios
            .iter()
            .filter_map(|scenario| {
                configuration
                    .host()
                    .or(scenario.host.as_deref())
                    .or(self.defaults.host.as_deref())
            })
            .filter_map(|host| url::Url::parse(host).ok())
            .map(|url| url.to_string())
            .collect();
        let rate = |rate: &Option<String>| rate.as_deref().and_then(|r| r.parse::<f64>().ok());
        let secs = |secs: usize| (secs > 0).then_some(secs as u64);
        metrics::dashboard_snapshot::SnapshotSettings {
            host: configuration.host().map(str::to_string),
            hosts: hosts.into_iter().collect(),
            users: configuration.users.map(|users| users as u64),
            increase_rate: rate(&configuration.increase_rate),
            increase_time_secs: secs(configuration.increase_time_secs()),
            decrease_rate: rate(&configuration.decrease_rate),
            decrease_time_secs: secs(configuration.decrease_time_secs()),
            run_time_secs: secs(configuration.run_time_secs()),
            test_plan: configuration.test_plan.as_ref().map(TestPlan::to_string),
            hold_ends_in_secs: None,
            stage: String::new(),
        }
    }

    // Invoke `test_start` transactions if existing.
    async fn run_test_start(&self) -> Result<(), GooseError> {
        // First run global test_start_transaction, if defined.
        if let Some(t) = &self.test_start_transaction {
            info!("running test_start_transaction");
            // Create a one-time-use User to run the test_start_transaction.
            let base_url = goose::get_base_url(
                self.get_configuration_host(),
                None,
                self.defaults.host.clone(),
            )?;
            let mut user = GooseUser::single_with_client(
                base_url,
                &self.configuration,
                self.client_strategy
                    .as_ref()
                    .map(client::ClientStrategy::user_client)
                    .transpose()?,
            )?;
            let function = &t.function;
            let _ = function(&mut user).await;
        }
        Ok(())
    }

    // Invoke `test_stop` transactions if existing.
    async fn run_test_stop(&self) -> Result<(), GooseError> {
        // First run global test_stop_transaction, if defined.
        if let Some(t) = &self.test_stop_transaction {
            info!("running test_stop_transaction");
            // Create a one-time-use User to run the test_stop_transaction.
            let base_url = goose::get_base_url(
                self.get_configuration_host(),
                None,
                self.defaults.host.clone(),
            )?;
            let mut user = GooseUser::single_with_client(
                base_url,
                &self.configuration,
                self.client_strategy
                    .as_ref()
                    .map(client::ClientStrategy::user_client)
                    .transpose()?,
            )?;
            let function = &t.function;
            let _ = function(&mut user).await;
        }
        Ok(())
    }

    // Create a GooseAttackRunState object and do all initialization required
    // to start a [`GooseAttack`](./struct.GooseAttack.html).
    async fn initialize_attack(&mut self) -> Result<GooseAttackRunState, GooseError> {
        trace!("initialize_attack");

        // Create a single channel used to send metrics from GooseUser threads
        // to the dedicated metrics processor task.
        let (all_threads_metrics_tx, metrics_rx): (
            flume::Sender<GooseMetric>,
            flume::Receiver<GooseMetric>,
        ) = flume::unbounded();

        // Create a channel for sending commands from the main loop to the
        // metrics processor task.
        let (metrics_cmd_tx, metrics_cmd_rx): (
            flume::Sender<MetricsCommand>,
            flume::Receiver<MetricsCommand>,
        ) = flume::unbounded();

        // Create a single channel to allow GooseUser threads to notify the
        // parent thread when they exit.
        let (all_threads_shutdown_tx, shutdown_rx): (flume::Sender<usize>, flume::Receiver<usize>) =
            flume::unbounded();

        // Telnet and WebSocket send commands to the parent on one channel, and
        // dashboard control on another, which the parent answers first.
        let (control_tx, controller_channel_rx) = (!self.configuration.no_telnet
            || self.configuration.websocket)
            .then(flume::unbounded)
            .unzip();
        #[cfg(feature = "dashboard")]
        let (dashboard_control_tx, dashboard_control_rx) = self
            .configuration
            .dashboard_control
            .then(flume::unbounded)
            .unzip();
        #[cfg(not(feature = "dashboard"))]
        let dashboard_control_rx = None;

        // Optionally spawn a telnet and/or Websocket Controller thread.
        let listening_controllers = match &control_tx {
            Some(control_tx) => self.setup_controllers(control_tx).await,
            None => controller::ListeningControllers::default(),
        };

        // Optionally spawn the read-only live dashboard HTTP server.
        // Bind failure aborts attack init when --dashboard is set.
        #[cfg(feature = "dashboard")]
        let (
            dashboard_channel_rx,
            dashboard_close_tx,
            dashboard_active_sse,
            dashboard_running_folder,
            dashboard_log_folder,
            dashboard_url,
            dashboard_server,
        ) = match dashboard::setup_dashboard(
            &self.configuration,
            dashboard_control_tx,
            listening_controllers,
        )
        .await?
        {
            Some(setup) => (
                Some(setup.request_rx),
                Some(setup.close_tx),
                Some(setup.active_sse),
                Some(setup.running_folder),
                Some(setup.log_folder),
                Some(setup.url),
                Some(setup.server),
            ),
            None => (None, None, None, None, None, None, None),
        };

        // Grab now() once from the standard library, used by multiple timers in
        // the run state.
        let std_now = std::time::Instant::now();

        // Create the shared atomic request counter registry.
        let request_counter_registry: crate::metrics::GooseRequestCounterRegistry =
            Arc::new(Mutex::new(HashMap::new()));

        // Create the shared batch epoch counter (starts at 0).
        let metrics_epoch: crate::metrics::MetricsEpoch =
            Arc::new(std::sync::atomic::AtomicU64::new(0));

        // Spawn the dedicated metrics processor task. It receives raw metrics
        // from user threads and commands from the main loop.
        let processor = MetricsProcessor::new(
            metrics_rx,
            metrics_cmd_rx,
            self.metrics.clone(),
            GraphData::for_metrics_processor(
                self.configuration.records_latency_series(),
                self.configuration.records_synthetic_latency(),
                self.records_latency_columns(),
            ),
            self.configuration.clone(),
            self.scenarios.clone(),
            self.defaults.clone(),
            request_counter_registry.clone(),
            // Logger not yet initialized; this processor is a short-lived
            // placeholder needed to service controller commands during the Idle
            // phase. It will be replaced by reset_run_state() (which initializes
            // the logger) before any user threads are spawned.
            None,
            metrics_epoch.clone(),
            self.keeps_full_graph(),
        );
        let metrics_processor_handle = Some(tokio::spawn(processor.run()));

        let goose_attack_run_state = GooseAttackRunState {
            adjust_user_timer: std_now,
            adjust_user_in_ms: 0,
            active_users: 0,
            completed_users: 0,
            drift_timer: tokio::time::Instant::now(),
            all_threads_metrics_tx,
            all_threads_shutdown_tx,
            metrics_cmd_tx,
            metrics_processor_handle,
            shutdown_rx,
            logger_handle: None,
            all_threads_logger_tx: None,
            throttle_threads_tx: None,
            parent_to_throttle_tx: None,
            controller_channel_rx,
            dashboard_control_rx,
            #[cfg(feature = "dashboard")]
            dashboard_channel_rx,
            #[cfg(feature = "dashboard")]
            dashboard_close_tx,
            #[cfg(feature = "gaggle")]
            gaggle_shadow_second: None,
            #[cfg(feature = "dashboard")]
            dashboard_active_sse,
            #[cfg(feature = "dashboard")]
            dashboard_running_folder,
            #[cfg(feature = "dashboard")]
            dashboard_log_folder,
            #[cfg(feature = "dashboard")]
            dashboard_url,
            #[cfg(feature = "dashboard")]
            dashboard_server,
            metrics_header_displayed: false,
            idle_status_displayed: false,
            users: Vec::new(),
            user_channels: Vec::new(),
            running_users: Vec::new(),
            running_metrics_timer: std_now,
            threshold_check_at: None,
            users_shutdown: HashSet::new(),
            listening_controllers,
            all_users_spawned: false,
            shutdown_after_stop: self.shuts_down_after_stop(),
            canceling: false,
            stopping: false,
            request_counter_registry,
            metrics_epoch,
        };

        // Catch ctrl-c to allow clean shutdown to display metrics.
        util::setup_ctrlc_handler();

        Ok(goose_attack_run_state)
    }

    // Determine how long has elapsed since this step started.
    fn step_elapsed(&mut self) -> u128 {
        if let Some(step_started) = self.step_started {
            step_started.elapsed().as_millis()
        } else if let Some(started) = self.started {
            started.elapsed().as_millis()
        } else {
            // An idle load test.
            0
        }
    }

    // Add delay before starting next step if there's time remaining.
    async fn end_of_step_delay(&mut self) {
        // Determine if there's remaining time in this step.
        let elapsed = self.step_elapsed() as u64;
        if elapsed < self.test_plan.steps[self.test_plan.current].1 as u64 {
            let remainder = self.test_plan.steps[self.test_plan.current].1 as u64 - elapsed;
            // Sleep 500ms, or all remaining time if less -- this will continue looping until all time remaining
            // on the current step runs out, waking up regularly to handle events like the load test being
            // canceled or a controller command.
            let maximum_sleep = 500;
            let sleep_duration = if remainder > maximum_sleep {
                Duration::from_millis(maximum_sleep)
            } else {
                Duration::from_millis(remainder)
            };
            tokio::time::sleep(sleep_duration).await
        }
    }

    // Increase the number of active [`GooseUser`](./goose/struct.GooseUser.html) threads in the
    // active [`GooseAttack`](./struct.GooseAttack.html).
    async fn increase_attack(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        // Determine if enough users have been launched.
        let all_users_launched =
            goose_attack_run_state.active_users >= self.test_plan.steps[self.test_plan.current].0;

        if all_users_launched {
            // All users were increased, delay until test_plan step time has elapsed.
            self.end_of_step_delay().await;

            if self.step_elapsed() as usize >= self.test_plan.steps[self.test_plan.current].1 {
                // Automatically reset metrics if appropriate.
                self.reset_metrics(goose_attack_run_state).await?;

                // Moving to the next phase, reset adjust_user_in_ms.
                goose_attack_run_state.adjust_user_in_ms = 0;

                // Advance to the next TestPlan step.
                self.advance_test_plan(goose_attack_run_state);
            }
        } else {
            // If this is the first load plan step, then there were no previously started users.
            let previous_users = if self.test_plan.current == 0 {
                0
            // Otherwise retreive the number of users configured in the previous step.
            } else {
                self.test_plan.steps[self.test_plan.current - 1].0
            };

            // Sanity check: increase_attack can only be called if the number of users is increasing
            // in the current step.
            assert!(self.test_plan.steps[self.test_plan.current].0 > previous_users);

            // Divide the number of new users to launch by the time configured to launch them.
            let increase_rate: f32 = (self.test_plan.steps[self.test_plan.current].0 - previous_users)
                as f32
                / self.test_plan.steps[self.test_plan.current].1 as f32
                // Convert from milliseconds to seconds.
                * 1_000.0;

            // A Gaggle manager's users run on its workers: it counts them as
            // its plan starts them, to keep the run's clock.
            #[cfg(feature = "gaggle")]
            if self.gaggle_manager() {
                if goose_attack_run_state.adjust_due() {
                    goose_attack_run_state.schedule_next_adjust(Duration::from_secs_f64(
                        1.0 / f64::from(increase_rate),
                    ));
                    goose_attack_run_state.active_users += 1;
                    self.metrics.total_users += 1;
                    if goose_attack_run_state.active_users > self.metrics.maximum_users {
                        self.metrics.maximum_users = goose_attack_run_state.active_users;
                    }
                } else {
                    goose_attack_run_state.sleep_until_adjust().await;
                }
                return Ok(());
            }

            // Determine if it's time to spawn a GooseUser.
            if goose_attack_run_state.adjust_due() {
                let mut thread_user = self
                    .weighted_users
                    .pop()
                    .expect("insufficent weighted_users");
                // The next GooseUser is due one interval, 1 second divided by the
                // increase_rate, after this one was.
                goose_attack_run_state
                    .schedule_next_adjust(Duration::from_secs_f64(1.0 / f64::from(increase_rate)));

                // Remember which task group this user is using.
                #[cfg(feature = "gaggle")]
                let first_user = self.user_offset;
                #[cfg(not(feature = "gaggle"))]
                let first_user = 0;
                thread_user.weighted_users_index = first_user + self.metrics.total_users;

                // Create a per-thread channel allowing parent thread to control child threads.
                let (parent_sender, thread_receiver): (
                    flume::Sender<GooseUserCommand>,
                    flume::Receiver<GooseUserCommand>,
                ) = flume::unbounded();
                goose_attack_run_state.user_channels.push(parent_sender);
                goose_attack_run_state.running_users.push((
                    thread_user.scenarios_index,
                    thread_user.weighted_users_index,
                ));
                // The user runs its scenario from now, iteration or not.
                let _ = goose_attack_run_state
                    .metrics_cmd_tx
                    .send(MetricsCommand::UserStarted {
                        scenario_index: thread_user.scenarios_index,
                        user: thread_user.weighted_users_index,
                    });

                // Clone the logger_tx if enabled, otherwise is None.
                thread_user
                    .logger
                    .clone_from(&goose_attack_run_state.all_threads_logger_tx);

                // Copy the GooseUser-throttle receiver channel, used by all threads.
                thread_user.throttle = if self.configuration.throttle_requests() > 0 {
                    Some(goose_attack_run_state.throttle_threads_tx.clone().unwrap())
                } else {
                    None
                };

                // The worker data hook's bytes, shared by every user.
                thread_user.worker_data.clone_from(&self.worker_data);

                // Copy the GooseUser-metrics sender channel, used by all threads.
                thread_user.metrics_channel =
                    Some(goose_attack_run_state.all_threads_metrics_tx.clone());

                // Initialize the per-user metrics batch accumulator.
                let epoch = goose_attack_run_state
                    .metrics_epoch
                    .load(std::sync::atomic::Ordering::Relaxed);
                thread_user.metrics_batch = Some(crate::metrics::GooseMetricBatch::new(epoch));
                thread_user.metrics_epoch = Some(goose_attack_run_state.metrics_epoch.clone());

                // Share the atomic request counter registry with this user thread.
                thread_user.request_counters =
                    Some(goose_attack_run_state.request_counter_registry.clone());

                // Copy the GooseUser-shutdown sender channel, used by all threads.
                thread_user.shutdown_channel =
                    Some(goose_attack_run_state.all_threads_shutdown_tx.clone());

                // Copy the appropriate task_set into the thread.
                let thread_scenario = self.scenarios[thread_user.scenarios_index].clone();

                // Start at 1 as this is human visible.
                let thread_number = self.metrics.total_users + 1;

                // Align request `elapsed` (graph second) with the continuous series
                // clock so RPS and active-users charts share one timeline. Weighted
                // users are often created before the run starts; mid-test increases
                // create users later — both must report seconds since series start.
                if let Some(series_started) = self.series_started {
                    thread_user.started = series_started;
                }

                // Launch a new user.
                let user = tokio::spawn(user::user_main(
                    thread_number,
                    thread_scenario,
                    thread_user,
                    thread_receiver,
                ));

                // A Gaggle worker's dead man's switch stops it at once.
                #[cfg(feature = "gaggle")]
                if let Some(link) = self.gaggle_link() {
                    link.watch(user.abort_handle());
                }

                goose_attack_run_state.users.push(user);
                goose_attack_run_state.active_users += 1;
                self.metrics.total_users += 1;
                if goose_attack_run_state.active_users > self.metrics.maximum_users {
                    self.metrics.maximum_users = goose_attack_run_state.active_users;
                }

                if let Some(running_metrics) = self.configuration.running_metrics {
                    if util::ms_timer_expired(
                        goose_attack_run_state.running_metrics_timer,
                        running_metrics,
                    ) {
                        goose_attack_run_state.running_metrics_timer = time::Instant::now();
                        self.update_duration();
                        // Send display command to the metrics processor task.
                        let _ = goose_attack_run_state.metrics_cmd_tx.send(
                            MetricsCommand::DisplayRunning {
                                duration: self.metrics.duration,
                                total_users: self.metrics.total_users,
                                maximum_users: self.metrics.maximum_users,
                            },
                        );
                    }
                }
            } else {
                goose_attack_run_state.sleep_until_adjust().await;
            }
        }

        Ok(())
    }

    // Maintain the number of active [`GooseUser`](./goose/struct.GooseUser.html) threads in the
    // active [`GooseAttack`](./struct.GooseAttack.html).
    async fn maintain_attack(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        // Determine if it's time to move to the next test plan step.
        if self.test_plan.current < self.test_plan.steps.len()
            && util::ms_timer_expired(
                self.step_started.unwrap(),
                self.test_plan.steps[self.test_plan.current].1,
            )
        {
            self.advance_test_plan(goose_attack_run_state);
        } else {
            // Subtract the time spent doing other things, running the main parent loop twice
            // per second.
            goose_attack_run_state.drift_timer = util::sleep_minus_drift(
                time::Duration::from_millis(500),
                goose_attack_run_state.drift_timer,
            )
            .await;
        }

        Ok(())
    }

    // Decrease the number of active [`GooseUser`](./goose/struct.GooseUser.html) threads in the
    // active [`GooseAttack`](./struct.GooseAttack.html).
    async fn decrease_attack(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        // Sanity check: if there's more than one step the first step can't decrease the attack. If there's
        // only one step then an idle load test may be being shut down through the controller.
        if self.test_plan.steps.len() > 1 {
            assert!(self.test_plan.current > 0);
        }

        // If this is the last step of the load test and there are 0 users, shut down.
        if goose_attack_run_state.active_users == 0
            // Subtract 1 from len() as it starts at 1 while current starts at 0.
            && self.test_plan.current >= self.test_plan.steps.len() - 1
        {
            // If throttle is enabled, tell throttle thread the load test is over.
            if let Some(throttle_tx) = goose_attack_run_state.parent_to_throttle_tx.clone() {
                let _ = throttle_tx.send(false);
            }

            // Take the users vector out of the GooseAttackRunState object so it can be
            // consumed by futures::future::join_all().
            let users = std::mem::take(&mut goose_attack_run_state.users);
            futures::future::join_all(users).await;
            debug!("all users exited");

            // If the logger thread is enabled, tell it to flush and exit.
            if goose_attack_run_state.logger_handle.is_some() {
                if let Err(e) = goose_attack_run_state
                    .all_threads_logger_tx
                    .clone()
                    .unwrap()
                    .send(None)
                {
                    warn!("unexpected error telling logger thread to exit: {e}");
                };
                // Take logger out of the GooseAttackRunState object so it can be
                // consumed by tokio::join!().
                let logger = std::mem::take(&mut goose_attack_run_state.logger_handle);
                let _ = tokio::join!(logger.unwrap());
            }

            // A Gaggle manager stops its workers and waits for their final
            // metrics; a worker sends its own.
            #[cfg(feature = "gaggle")]
            if self.run_in_progress {
                self.gaggle_end_of_run(goose_attack_run_state).await;
            }

            // Run test_stop and mark the metrics final, only when a run
            // happened since the last time: leaving Idle has nothing to stop,
            // and test_stop already ran when the run itself ended.
            if self.run_in_progress {
                self.stop_attack().await?;
            }

            // Record final users for the users-per-second graph before shutting
            // down the metrics processor. Use the continuous series clock so
            // this sample stays aligned with ramp history after metrics reset.
            if self.series_started.is_some() {
                let _ = goose_attack_run_state
                    .metrics_cmd_tx
                    .send(MetricsCommand::RecordUsers {
                        active_users: goose_attack_run_state.active_users,
                        elapsed_secs: self.series_elapsed_secs(),
                    });
            }

            // Tell the metrics processor to flush all remaining metrics and
            // return the final state for reporting. The processor may have
            // already been shut down (e.g., controller Stop followed by
            // Shutdown), so handle the send failure gracefully.
            // Note: do NOT call update_duration() here — the duration was already
            // set by the main loop iteration, and the shutdown work above (waiting
            // for users, flushing logger, stop_attack) adds extra time that would
            // inflate the duration past the expected value.
            let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();
            if goose_attack_run_state
                .metrics_cmd_tx
                .send(MetricsCommand::Shutdown {
                    duration: self.metrics.duration,
                    total_users: self.metrics.total_users,
                    maximum_users: self.metrics.maximum_users,
                    respond: respond_tx,
                })
                .is_ok()
            {
                if let Ok((mut final_metrics, final_graph_data)) = respond_rx.await {
                    // Await the processor task to ensure clean shutdown.
                    if let Some(handle) = goose_attack_run_state.metrics_processor_handle.take() {
                        let _ = handle.await;
                    }

                    // Merge main-loop-only fields into the processor's metrics.
                    final_metrics.history = std::mem::take(&mut self.metrics.history);
                    final_metrics.hosts.clone_from(&self.metrics.hosts);
                    final_metrics.hash = self.metrics.hash;
                    final_metrics.display_metrics = self.metrics.display_metrics;
                    final_metrics.display_status_codes = self.metrics.display_status_codes;
                    final_metrics.final_metrics = self.metrics.final_metrics;
                    self.metrics = final_metrics;
                    self.graph_data = final_graph_data;
                }
            }
            // Judge every threshold on the final metrics, before any report is
            // written, so every report shows the results.
            if self.run_in_progress && !self.configuration.thresholds.is_empty() {
                self.metrics.thresholds = threshold::evaluate(
                    &self.configuration.thresholds,
                    &self.metrics,
                    threshold::Check::Final,
                    self.metrics
                        .threshold_abort
                        .as_ref()
                        .map(|(index, observed)| (*index, observed.as_str())),
                    self.elapsed,
                );
                for warning in
                    threshold::unknown_requests(&self.configuration.thresholds, &self.metrics)
                {
                    warn!("{warning}");
                }
            }
            // The load test is fully stopped at this point.
            self.metrics
                .history
                .push(TestPlanHistory::step(TestPlanStepAction::Finished, 0));
            // Write the --report-file reports and Save report's, only when a
            // run happened since the last time (not on a shutdown from Idle).
            let ran = self.run_in_progress;
            // A Gaggle manager collects its workers' logs first, so the
            // reports can name any left on a worker.
            #[cfg(feature = "gaggle")]
            if ran {
                self.gaggle_collect_logs().await;
            }
            let reports = if ran {
                self.write_end_of_run(!goose_attack_run_state.shutdown_after_stop)
                    .await
            } else {
                Ok(())
            };
            // Then its workers shut down.
            #[cfg(feature = "gaggle")]
            if ran {
                self.gaggle_shut_down().await;
            }
            self.run_in_progress = false;
            // A bare --goose-log stays open in the run's folder until the next
            // run starts, and the folder stays reserved while the log is open.
            if !self.goose_log_in_folder() {
                self.output.release_folder();
            }
            self.output.end_save_report_hold();
            // End the Goose log's gzip member, so the finished run's log is
            // whole while Idle; lines until the next run are whole members.
            logger::finish_goose_log();
            #[cfg(feature = "dashboard")]
            if let Some(running) = &goose_attack_run_state.dashboard_running_folder {
                running.set(None);
            }
            if let Err(e) = reports {
                // Print the final metrics before failing on a report.
                if self.metrics.display_metrics {
                    info!(
                        "printing final metrics after {} seconds...",
                        self.metrics.duration
                    );
                    print!("{}", self.metrics);
                }
                return Err(e);
            }
            // After an autostarted run, wait for a dashboard tab that is
            // open so the run's report can still be downloaded from it.
            #[cfg(feature = "dashboard")]
            let wait_for_dashboard = ran
                && goose_attack_run_state.shutdown_after_stop
                && self.should_wait_for_dashboard(goose_attack_run_state);
            #[cfg(not(feature = "dashboard"))]
            let wait_for_dashboard = false;
            // Shutdown Goose or go into an idle waiting state.
            if goose_attack_run_state.shutdown_after_stop && !wait_for_dashboard {
                self.set_attack_phase(goose_attack_run_state, AttackPhase::Shutdown);
            } else {
                // Print metrics, if enabled.
                if self.metrics.display_metrics {
                    println!("{}", self.metrics);
                }
                #[cfg(feature = "dashboard")]
                if wait_for_dashboard {
                    self.start_dashboard_wait(goose_attack_run_state);
                }
                // Return to an Idle state.
                self.set_attack_phase(goose_attack_run_state, AttackPhase::Idle);
            }
        // If this is not the last step of the load test and sufficient users decreased, move to next step.
        } else if goose_attack_run_state.active_users
            <= self.test_plan.steps[self.test_plan.current].0
        {
            // Be sure step takes as long as it was configured to.
            self.end_of_step_delay().await;

            // Then advance to next step.
            if self.step_elapsed() as usize >= self.test_plan.steps[self.test_plan.current].1 {
                // Moving to the next phase, reset adjust_user_in_ms.
                goose_attack_run_state.adjust_user_in_ms = 0;

                // Advance to the next TestPlan step.
                self.advance_test_plan(goose_attack_run_state);
            }
        // Otherwise, decrease a user when ready.
        } else {
            // Retreive the number of users configured in the previous step.
            let previous_users = self.test_plan.steps[self.test_plan.current - 1].0;

            // Divide the number of users to decrease by the time configured to decrease them.
            let decrease_rate: f32 = (previous_users - self.test_plan.steps[self.test_plan.current].0)
                as f32
                / self.test_plan.steps[self.test_plan.current].1 as f32
                // Convert from milliseconds to seconds.
                * 1_000.0;

            // Determine if it's time to decrease a GooseUser.
            if goose_attack_run_state.adjust_due() {
                // The next GooseUser is due to stop one interval, 1 second divided
                // by the decrease_rate, after this one was.
                goose_attack_run_state
                    .schedule_next_adjust(Duration::from_secs_f64(1.0 / f64::from(decrease_rate)));

                // A Gaggle manager's users run on its workers.
                #[cfg(feature = "gaggle")]
                if self.gaggle_manager() {
                    goose_attack_run_state.completed_users += 1;
                    goose_attack_run_state.active_users -= 1;
                    return Ok(());
                }

                goose_attack_run_state.running_users.pop();
                if let Some(send_to_user) = goose_attack_run_state.user_channels.pop() {
                    match send_to_user.send(GooseUserCommand::Exit) {
                        Ok(_) => {
                            debug!(
                                "telling user {} to exit",
                                goose_attack_run_state.completed_users
                            );
                        }
                        Err(e) => {
                            // Error is expected if this user already shut down.
                            if !goose_attack_run_state
                                .users_shutdown
                                .contains(&goose_attack_run_state.completed_users)
                            {
                                info!(
                                    "failed to tell user {} to exit: {}",
                                    goose_attack_run_state.completed_users, e
                                );
                            }
                        }
                    }
                    goose_attack_run_state.completed_users += 1;
                    goose_attack_run_state.active_users -= 1;
                }
            } else {
                goose_attack_run_state.sleep_until_adjust().await;
            }
        }

        Ok(())
    }

    // ========================================================================
    // Staged control helpers (shared by Controllers and dashboard control)
    //
    // Soft vs hard errors:
    // - Soft: expected rejections → ControlOutcome { ok: false, error: Some(...) }
    // - Hard: Result::Err(GooseError) (weight_scenario_users on a users or test
    //   plan increase; reset_run_state in start finish; cancel_attack).
    // `GooseAttack::control` replies with the outcome, or `internal` on a hard
    // error before returning it, so every caller still waiting gets an answer.
    // ========================================================================

    /// Build a [`ControlOutcome`] snapshot from the current attack/run state.
    fn control_outcome(
        &self,
        goose_attack_run_state: &GooseAttackRunState,
        ok: bool,
        error: Option<&'static str>,
        message: impl Into<String>,
        target_users: Option<usize>,
    ) -> ControlOutcome {
        ControlOutcome {
            ok,
            error,
            message: message.into(),
            phase: attack_phase_str(self.attack_phase).to_string(),
            active_users: goose_attack_run_state.active_users,
            target_users,
            field: None,
            #[cfg(feature = "dashboard")]
            settings: None,
        }
    }

    /// Pre-reply Start work: validate Idle, build plan, prepare, enter Increase.
    ///
    /// Soft rejections: `invalid_phase`, `prepare_failed`.
    /// On [`ControlStartBegin::Accepted`], the caller must reply success immediately,
    /// then invoke [`Self::control_start_finish`].
    pub(crate) fn control_start_begin(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<ControlStartBegin, GooseError> {
        // Can only start an idle load test.
        if self.attack_phase != AttackPhase::Idle {
            return Ok(ControlStartBegin::Rejected(self.control_outcome(
                goose_attack_run_state,
                false,
                Some("invalid_phase"),
                "unable to start load test, it is not idle",
                None,
            )));
        }

        let prepared = TestPlan::build(&self.configuration).and_then(|test_plan| {
            self.test_plan = test_plan;
            self.prepare_load_test()
        });
        // The shape is the run's: a refused Start leaves none.
        self.run_shape = prepared
            .is_ok()
            .then(|| run_shape::RunShape::for_configuration(&self.configuration))
            .flatten();
        if let Err(e) = prepared {
            // Soft: do not move to Increase if unable to prepare load test. Name
            // the cause: prepare_load_test refuses no host and no users itself,
            // with the options these constants name.
            let (reason, field) = match e {
                GooseError::InvalidOption { option, detail, .. } if option == NO_HOST_OPTION => {
                    let reason = match HOST_TRANSACTIONS
                        .iter()
                        .find(|transaction| detail == no_transaction_host_detail(transaction))
                    {
                        Some(transaction) => {
                            format!("no host is configured for the {transaction} transaction")
                        }
                        None => "no host is configured".to_string(),
                    };
                    (reason, Some("host"))
                }
                GooseError::InvalidOption { option, .. } if option == NO_USERS_OPTION => {
                    ("there are no users to start".to_string(), None)
                }
                GooseError::InvalidOption { detail, .. } => (detail, None),
                e => (e.to_string(), None),
            };
            let outcome = self.control_outcome(
                goose_attack_run_state,
                false,
                Some("prepare_failed"),
                format!("unable to start load test, {reason}"),
                None,
            );
            return Ok(ControlStartBegin::Rejected(match field {
                Some(field) => outcome.with_field(field),
                None => outcome,
            }));
        }

        self.set_attack_phase(goose_attack_run_state, AttackPhase::Increase);

        // Prefer configured users; fall back to first non-zero plan step.
        let target_users = self.configuration.users.or_else(|| {
            self.test_plan
                .steps
                .iter()
                .map(|(users, _)| *users)
                .find(|&u| u > 0)
        });

        Ok(ControlStartBegin::Accepted(self.control_outcome(
            goose_attack_run_state,
            true,
            None,
            "load test started",
            target_users,
        )))
    }

    /// Post-reply Start work: reset run state and push Increasing history.
    ///
    /// Hard errors only (`reset_run_state` / metrics init). Caller has already
    /// replied success to the client.
    pub(crate) async fn control_start_finish(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        self.reset_run_state(goose_attack_run_state).await?;
        self.metrics
            .history
            .push(TestPlanHistory::step(TestPlanStepAction::Increasing, 0));
        Ok(())
    }

    /// Begin canceling a running load test (Increase/Maintain → Decrease/Canceling).
    ///
    /// Soft failure: `invalid_phase` when not Increase|Maintain (Decrease is not
    /// stoppable). Hard errors: `cancel_attack` if it returns Err.
    ///
    /// Success leaves phase as **decrease** (not idle); Idle is eventual after
    /// the cancel ramp completes with `shutdown_after_stop = false`.
    pub(crate) async fn control_stop(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<ControlOutcome, GooseError> {
        if ![AttackPhase::Increase, AttackPhase::Maintain].contains(&self.attack_phase) {
            return Ok(self.control_outcome(
                goose_attack_run_state,
                false,
                Some("invalid_phase"),
                "load test not running, failed to stop",
                None,
            ));
        }

        // Don't shutdown when load test is stopped by controller; remain idle instead.
        goose_attack_run_state.shutdown_after_stop = false;
        // Don't automatically restart the load test.
        self.configuration.no_autostart = true;
        self.set_ended_by(runs::EndedBy::Stopped, None);
        self.cancel_attack(goose_attack_run_state).await?;

        Ok(self.control_outcome(
            goose_attack_run_state,
            true,
            None,
            "load test stopped",
            Some(0),
        ))
    }

    /// Shut Goose down from Idle (dashboard Quit), as Ctrl-C does there.
    ///
    /// Soft failure: `invalid_phase` in any phase but Idle, so a running load
    /// test must be stopped first.
    pub(crate) fn control_quit(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> ControlOutcome {
        if self.attack_phase != AttackPhase::Idle {
            return self.control_outcome(
                goose_attack_run_state,
                false,
                Some("invalid_phase"),
                "unable to quit, be sure the load test is idle",
                None,
            );
        }
        info!("[control]: quit, shutting down");
        self.shutdown_from_idle(goose_attack_run_state);
        self.control_outcome(
            goose_attack_run_state,
            true,
            None,
            "goose is shutting down",
            None,
        )
    }

    /// Refuse to reconfigure `what` (users or the test plan) while a cancel
    /// (Stop, `shutdown`, Ctrl-C) is in progress: rebuilding the plan then
    /// would replace its ramp to zero and leave the load test running. Also
    /// refused in Shutdown, where there is no plan left to change.
    fn refuse_reconfigure(
        &self,
        goose_attack_run_state: &GooseAttackRunState,
        what: &str,
    ) -> Option<ControlOutcome> {
        let message = if goose_attack_run_state.stopping {
            format!("load test is stopping, failed to reconfigure {what}")
        } else if self.attack_phase == AttackPhase::Shutdown {
            format!("load test not idle, failed to reconfigure {what}")
        } else {
            return None;
        };
        Some(self.control_outcome(
            goose_attack_run_state,
            false,
            Some("invalid_phase"),
            message,
            None,
        ))
    }

    /// Set absolute target user count (Controller `users` command), already
    /// bounded by [`ControllerCommand::users`](controller::ControllerCommand).
    ///
    /// Soft failure: `invalid_phase` as [`Self::refuse_reconfigure`] says, and
    /// while a test plan runs, as the plan sets users. In Idle, refused as
    /// `conflicting_settings` below 2 users while an increase or decrease
    /// time is set, as on the command line. A users ramp's own ramp down is
    /// not a cancel: a new count abandons it and holds until stopped.
    /// Hard error: `weight_scenario_users` when increasing while running.
    pub(crate) fn control_set_users(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        new_users: usize,
    ) -> Result<ControlOutcome, GooseError> {
        if let Some(refused) = self.refuse_reconfigure(goose_attack_run_state, "users") {
            return Ok(refused.with_field("users"));
        }
        if self.attack_phase == AttackPhase::Idle {
            if new_users < 2 {
                let time = if self.configuration.increase_time_secs() > 0 {
                    Some("increase_time")
                } else if self.configuration.decrease_time_secs() > 0 {
                    Some("decrease_time")
                } else {
                    None
                };
                if let Some(time) = time {
                    return Ok(self
                        .control_outcome(
                            goose_attack_run_state,
                            false,
                            Some("conflicting_settings"),
                            format!("failed to configure users, {time} needs at least 2 users"),
                            None,
                        )
                        .with_field("users"));
                }
            }
            // If the load test is idle, simply update the configuration.
            // Safe when `current` has advanced past the last plan step.
            let current_users = self.current_target_users();
            info!("changing users from {current_users:?} to {new_users}");
        } else {
            let Some(mut shape) = self.run_shape.take() else {
                // A test plan sets users itself.
                return Ok(self
                    .control_outcome(
                        goose_attack_run_state,
                        false,
                        Some("invalid_phase"),
                        "failed to reconfigure users, the test plan sets users: edit the plan, or stop and switch mode",
                        None,
                    )
                    .with_field("users"));
            };
            info!(
                "changing users from {} to {new_users}",
                goose_attack_run_state.active_users
            );
            // Determine how long has elapsed since this step started.
            let elapsed = self.step_elapsed() as usize;

            // Determine how many users to increase or decrease by.
            let user_difference = goose_attack_run_state.active_users.abs_diff(new_users);

            // Determine how quickly to adjust user count, using
            // increase_rate when adding users and decrease_rate when
            // removing users.
            let rate = if new_users > goose_attack_run_state.active_users {
                if let Some(increase_rate) = self.configuration.increase_rate.as_ref() {
                    util::parse_rate(Some(increase_rate.to_string()))
                } else {
                    util::parse_rate(None)
                }
            } else if let Some(decrease_rate) = self.configuration.decrease_rate.as_ref() {
                util::parse_rate(Some(decrease_rate.to_string()))
            } else if let Some(increase_rate) = self.configuration.increase_rate.as_ref() {
                util::parse_rate(Some(increase_rate.to_string()))
            } else {
                util::parse_rate(None)
            };
            let total_time = run_shape::ramp_ms(user_difference, rate);

            // Allocate more users if increasing users (hard Err on failure),
            // before anything changes.
            if new_users > goose_attack_run_state.active_users {
                let weighted = self.weight_scenario_users(user_difference);
                match weighted {
                    Ok(weighted) => self.weighted_users = weighted,
                    Err(e) => {
                        self.run_shape = Some(shape);
                        return Err(e);
                    }
                }
            }

            // The rest of the run, generated again from its shape: a ramp to
            // the new count, then the hold and ramp down still ahead.
            self.test_plan.steps = shape.retarget(
                goose_attack_run_state.active_users,
                elapsed,
                new_users,
                total_time,
            );
            self.run_shape = Some(shape);

            // Reset the current step to what was happening when reconfiguration happened.
            self.test_plan.current = 0;

            // Finally, advance to the next step to adjust user count.
            self.advance_test_plan(goose_attack_run_state);
        }
        // Clear configured test plan only when the command is accepted:
        // refusals must not destroy an existing plan. Also update the running
        // configuration (this impacts if the test is stopped and then
        // restarted through the controller).
        self.configuration.test_plan = None;
        self.configuration.users = Some(new_users);
        Ok(self.control_outcome(
            goose_attack_run_state,
            true,
            None,
            "users configured",
            Some(new_users),
        ))
    }

    /// Switch a running load test to `test_plan`, from its first step and
    /// the current user count, replacing a users ramp and its shape.
    ///
    /// Hard error: `weight_scenario_users`.
    pub(crate) fn run_test_plan(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        test_plan: TestPlan,
    ) -> Result<(), GooseError> {
        self.run_shape = None;
        self.test_plan = test_plan;
        // Reallocate users.
        self.weighted_users = self.weight_scenario_users(self.test_plan.total_users())?;
        // Insert the current state of the test plan before the new test plan.
        let elapsed = self.step_elapsed() as usize;
        self.test_plan
            .steps
            .insert(0, (goose_attack_run_state.active_users, elapsed));
        // Finally, advance to the next step to adjust user count.
        self.advance_test_plan(goose_attack_run_state);
        Ok(())
    }

    /// What would have no host with `configuration`: a `test_start` or
    /// `test_stop` transaction, or a scenario, as a Start checks them but
    /// without resolving a url.
    pub(crate) fn without_host(&self, configuration: &GooseConfiguration) -> Option<String> {
        if configuration.host().is_some() || self.defaults.host.is_some() {
            return None;
        }
        if self.test_start_transaction.is_some() {
            return Some(format!("the {} transaction", HOST_TRANSACTIONS[0]));
        }
        if self.test_stop_transaction.is_some() {
            return Some(format!("the {} transaction", HOST_TRANSACTIONS[1]));
        }
        self.scenarios
            .iter()
            .find(|scenario| scenario.host.is_none())
            .map(|scenario| format!("the {} scenario", scenario.name))
    }

    // Quickly abort and shut down an active [`GooseAttack`](./struct.GooseAttack.html).
    async fn cancel_attack(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        if goose_attack_run_state.active_users == 0 {
            // A stop or Ctrl-C in the tick of its start, or a second shutdown from Idle:
            // a plan maintaining 0 users would never end.
            goose_attack_run_state.adjust_user_in_ms = 0;
            self.stop_without_users(goose_attack_run_state);
            self.metrics
                .history
                .push(TestPlanHistory::step(TestPlanStepAction::Canceling, 0));
            return Ok(());
        }

        // Determine how long has elapsed since this step started.
        let elapsed = self.step_elapsed() as usize;

        // Reset the test_plan to stop all users quickly.
        self.test_plan.steps = vec![
            // Record how many active users there are currently.
            (goose_attack_run_state.active_users, elapsed),
            // Record how long the attack ran in this step.
            (0, 0),
        ];
        // Reset the current step to what was happening when canceled.
        self.test_plan.current = 0;

        // Moving to the last phase, reset adjust_user_in_ms.
        goose_attack_run_state.adjust_user_in_ms = 0;

        // Refuse user count changes until the run reaches Idle; a cancel
        // ends the run's shape.
        goose_attack_run_state.stopping = true;
        self.run_shape = None;

        // Advance to the final decrease phase.
        self.advance_test_plan(goose_attack_run_state);

        // Load test isn't just decreasing, it's canceling.
        self.metrics
            .history
            .last_mut()
            .expect("tried to cancel load test with no history")
            .action = TestPlanStepAction::Canceling;

        Ok(())
    }

    // Cleanly shut down the [`GooseAttack`](./struct.GooseAttack.html).
    async fn stop_attack(&mut self) -> Result<(), GooseError> {
        // Run any configured test_stop() functions: a Gaggle worker never
        // does, its manager runs them once.
        #[cfg(feature = "gaggle")]
        let worker = self.gaggle_worker();
        #[cfg(not(feature = "gaggle"))]
        let worker = false;
        if !worker {
            self.run_test_stop().await?;
        }

        // Percentile and errors are only displayed when the load test is finished.
        self.metrics.final_metrics = true;

        Ok(())
    }

    /// Whether Goose shuts down once a run stops: after an autostarted run,
    /// but never a Gaggle worker, which waits for its manager to say so.
    fn shuts_down_after_stop(&self) -> bool {
        #[cfg(feature = "gaggle")]
        if self.gaggle_worker() {
            return false;
        }
        !self.configuration.no_autostart
    }

    /// Whether Idle waits for something other than a Controller: a Gaggle
    /// worker waits for its manager.
    fn gaggle_waits_idle(&self) -> bool {
        #[cfg(feature = "gaggle")]
        {
            self.gaggle_worker()
        }
        #[cfg(not(feature = "gaggle"))]
        false
    }

    // Reset the GooseAttackRunState before starting a load test. This is to allow a Controller
    // to stop and start the load test multiple times, for example from a UI.
    async fn reset_run_state(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        // A new run starts without the previous run's cancel.
        reset_killswitch();

        // Run any configured test_start() functions: a Gaggle manager ran
        // them before its workers started, and a worker never does.
        #[cfg(feature = "gaggle")]
        let gaggle = self.gaggle_manager() || self.gaggle_worker();
        #[cfg(not(feature = "gaggle"))]
        let gaggle = false;
        if !gaggle {
            self.run_test_start().await?;
            // A run of one process is worker 0 of 1 to the worker data hook.
            let share = WorkerShare {
                index: 0,
                workers: 1,
                users: self.test_plan.total_users(),
                user_offset: 0,
            };
            if let Some(data) = self.call_worker_data_hook(&share)? {
                self.worker_data = Some(Arc::from(data));
            }
        }

        // Prepare to collect metrics, if enabled.
        self.metrics = GooseMetrics::default();
        if !self.configuration.no_metrics {
            self.metrics.initialize_transaction_metrics(
                &self.scenarios,
                &self.configuration,
                &self.defaults,
            )?;
            self.metrics
                .initialize_scenario_metrics(&self.scenarios, &self.configuration);
            if !self.configuration.no_print_metrics {
                self.metrics.display_metrics = true;
            }
            // Only display status codes if not disaled.
            self.metrics.display_status_codes = !self.configuration.no_status_codes;

            // Initialize CO tracker if mitigation is enabled
            if let Some(co_mitigation) = &self.configuration.co_mitigation {
                if co_mitigation != &metrics::GooseCoordinatedOmissionMitigation::Disabled {
                    self.metrics.coordinated_omission_metrics = Some(
                        metrics::CoordinatedOmissionMetrics::new(co_mitigation.clone()),
                    );
                }
            }
        }

        // Reset the run state.
        let std_now = std::time::Instant::now();
        goose_attack_run_state.adjust_user_timer = std_now;
        goose_attack_run_state.adjust_user_in_ms = 0;
        goose_attack_run_state.active_users = 0;
        goose_attack_run_state.drift_timer = tokio::time::Instant::now();
        goose_attack_run_state.metrics_header_displayed = false;
        goose_attack_run_state.idle_status_displayed = false;
        goose_attack_run_state.users = Vec::new();
        goose_attack_run_state.user_channels = Vec::new();
        goose_attack_run_state.running_users = Vec::new();
        goose_attack_run_state.running_metrics_timer = std_now;
        // The thresholds are checked from when the run first enters Maintain.
        goose_attack_run_state.threshold_check_at = None;
        goose_attack_run_state.shutdown_after_stop = self.shuts_down_after_stop();
        goose_attack_run_state.all_users_spawned = false;

        // Choose this run's folder, when it may need one, before any log opens
        // in it, and start the Goose log's gzip member for the run.
        self.start_run_folder()?;
        logger::start_goose_log();
        #[cfg(feature = "dashboard")]
        if let Some(running) = &goose_attack_run_state.dashboard_running_folder {
            running.set(
                self.output
                    .folder
                    .as_ref()
                    .map(|folder| folder.path.as_path()),
            );
        }
        // A bare --goose-log moved into this run's folder, and stays open there
        // after the run ends.
        #[cfg(feature = "dashboard")]
        if self.goose_log_in_folder() {
            if let (Some(log_folder), Some(folder)) = (
                &goose_attack_run_state.dashboard_log_folder,
                &self.output.folder,
            ) {
                log_folder.set(Some(&folder.path));
            }
        }

        // If enabled, spawn a logger thread. A Gaggle manager writes no logs
        // of its own: its workers write them, and send them at the end.
        #[cfg(feature = "gaggle")]
        let (logger_handle, all_threads_logger_tx) = if self.gaggle_manager() {
            (None, None)
        } else {
            self.configuration
                .setup_loggers(&self.defaults, self.output.folder.as_ref())
                .await?
        };
        #[cfg(not(feature = "gaggle"))]
        let (logger_handle, all_threads_logger_tx) = self
            .configuration
            .setup_loggers(&self.defaults, self.output.folder.as_ref())
            .await?;
        goose_attack_run_state.logger_handle = logger_handle;
        goose_attack_run_state.all_threads_logger_tx = all_threads_logger_tx;

        // If enabled, spawn a throttle thread.
        let (throttle_threads_tx, parent_to_throttle_tx) = self.setup_throttle().await;
        goose_attack_run_state.throttle_threads_tx = throttle_threads_tx;
        goose_attack_run_state.parent_to_throttle_tx = parent_to_throttle_tx;

        // A new run counts its requests from zero: the atomic counters
        // outlive the processor, and batches from the last run's users are
        // dropped.
        {
            let registry = goose_attack_run_state
                .request_counter_registry
                .lock()
                .expect("request_counter_registry mutex poisoned");
            for counters in registry.values() {
                counters.reset();
            }
        }
        goose_attack_run_state
            .metrics_epoch
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Re-create the metrics channel and spawn a fresh processor. It reads
        // Save report as start_run_folder left it, so start_run_folder runs
        // first.
        // The old processor exits when its command channel closes.
        let (all_threads_metrics_tx, metrics_rx): (
            flume::Sender<GooseMetric>,
            flume::Receiver<GooseMetric>,
        ) = flume::unbounded();
        goose_attack_run_state.all_threads_metrics_tx = all_threads_metrics_tx;
        let (metrics_cmd_tx, metrics_cmd_rx) = flume::unbounded();
        #[cfg_attr(not(feature = "gaggle"), allow(unused_mut))]
        let mut processor = MetricsProcessor::new(
            metrics_rx,
            metrics_cmd_rx,
            self.metrics.clone(),
            GraphData::for_metrics_processor(
                self.configuration.records_latency_series(),
                self.configuration.records_synthetic_latency(),
                self.records_latency_columns(),
            ),
            self.configuration.clone(),
            self.scenarios.clone(),
            self.defaults.clone(),
            goose_attack_run_state.request_counter_registry.clone(),
            goose_attack_run_state.all_threads_logger_tx.clone(),
            goose_attack_run_state.metrics_epoch.clone(),
            self.keeps_full_graph(),
        );
        #[cfg(feature = "gaggle")]
        if self.configuration.gaggle_shadow_merge {
            let outcome = Arc::new(Mutex::new(None));
            processor.gaggle_enable_shadow(outcome.clone());
            self.gaggle_shadow = Some(outcome);
            goose_attack_run_state.gaggle_shadow_second = None;
        } else if self.gaggle_manager() {
            processor.gaggle_enable_manager();
        } else if let Some(link) = self.gaggle_link() {
            processor.gaggle_enable(link.id);
        }
        goose_attack_run_state.metrics_cmd_tx = metrics_cmd_tx;
        if let Some(handle) = goose_attack_run_state.metrics_processor_handle.take() {
            let _ = handle.await;
        }
        goose_attack_run_state.metrics_processor_handle = Some(tokio::spawn(processor.run()));

        // Try to create the requested report files, to confirm access.
        self.create_reports().await?;

        self.run_in_progress = true;
        self.ended_by = None;

        // Record when the GooseAttack officially started. series_started is the
        // continuous graph clock and is not restarted on metrics reset.
        let now = time::Instant::now();
        self.started = Some(now);
        self.series_started = Some(now);
        // The same start, as the reports' record gives it.
        self.output.started = Some(chrono::Utc::now());
        // Align pre-created weighted users so request.elapsed matches series time.
        for user in &mut self.weighted_users {
            user.started = now;
        }

        Ok(())
    }

    // Called internally in local-mode and gaggle-mode.
    async fn start_attack(mut self) -> Result<GooseAttack, GooseError> {
        // The GooseAttackRunState is used while spawning and running the
        // GooseUser threads that generate the load test.
        // Propagate initialization errors (e.g. dashboard bind failure) as
        // `GooseError` rather than panicking — library callers of `execute()`
        // should get a structured Result.
        let mut goose_attack_run_state = self.initialize_attack().await?;

        // Without autostart, something has to be able to start the load test.
        if self.configuration.no_autostart
            && !goose_attack_run_state.listening_controllers.telnet
            && !goose_attack_run_state.listening_controllers.websocket
            && !self.configuration.dashboard_control
        {
            return Err(GooseError::InvalidOption {
                option: "`configuration.no_autostart`".to_string(),
                value: true.to_string(),
                detail: "`configuration.no_autostart` requires a Controller or dashboard control, and no Controller could listen on its port."
                    .to_string(),
            });
        }

        let result = self.run_attack(&mut goose_attack_run_state).await;

        // A run that ended early, on an error, leaves its logger running: end it,
        // so every line is written.
        if let Some(logger) = goose_attack_run_state.logger_handle.take() {
            if let Some(logger_tx) = goose_attack_run_state.all_threads_logger_tx.as_ref() {
                let _ = logger_tx.send(None);
            }
            let _ = logger.await;
        }

        // Stop the dashboard server on every return path, including errors
        // from the phase loop, and wait for it so the port is free when
        // `execute()` returns.
        #[cfg(feature = "dashboard")]
        if let Some(server) = goose_attack_run_state.dashboard_server.take() {
            server.shutdown().await;
        }

        result.map(|()| self)
    }

    /// The parent process GooseAttack loop: runs until Goose shuts down. Goose
    /// enters the loop in AttackPhase::Idle, and exits in AttackPhase::Shutdown.
    async fn run_attack(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        loop {
            match self.attack_phase {
                // In the Idle phase the Goose configuration can be changed by a Controller,
                // and otherwise nothing happens but sleeping an checking for messages.
                AttackPhase::Idle if self.gaggle_waits_idle() => {
                    #[cfg(feature = "gaggle")]
                    self.gaggle_worker_idle(goose_attack_run_state).await?;
                }
                AttackPhase::Idle => {
                    if self.configuration.no_autostart {
                        // Sleep then check for further instructions.
                        if goose_attack_run_state.idle_status_displayed {
                            let sleep_duration = Duration::from_millis(250);
                            debug!("sleeping {sleep_duration:?}...");
                            goose_attack_run_state.drift_timer = util::sleep_minus_drift(
                                sleep_duration,
                                goose_attack_run_state.drift_timer,
                            )
                            .await;
                        // Only display informational message about being idle one time.
                        } else {
                            info!("Goose is currently idle.");
                            goose_attack_run_state.idle_status_displayed = true;
                        }
                    } else {
                        // Prepare to start the load test, resetting timers and counters.
                        self.reset_run_state(goose_attack_run_state).await?;
                        self.metrics
                            .history
                            .push(TestPlanHistory::step(TestPlanStepAction::Increasing, 0));
                        //self.graph_data.set_starting(Utc::now());
                        self.set_attack_phase(goose_attack_run_state, AttackPhase::Increase);
                    }
                }
                // In the Increase phase, Goose launches GooseUser threads.
                AttackPhase::Increase => {
                    self.update_duration();
                    self.increase_attack(goose_attack_run_state).await?;
                }
                // In the Maintain phase, Goose continues runnning all launched GooseUser threads.
                AttackPhase::Maintain => {
                    self.update_duration();
                    self.maintain_attack(goose_attack_run_state).await?;
                }
                // In the Decrease phase, Goose stops GooseUser threads.
                AttackPhase::Decrease => {
                    // If displaying metrics, update internal state reflecting how long load test
                    // has been running.
                    self.update_duration();
                    // Reduce the number of GooseUsers running.
                    self.decrease_attack(goose_attack_run_state).await?;
                }
                // By reaching the Shutdown phase, break out of the GooseAttack loop.
                AttackPhase::Shutdown => {
                    // A Gaggle worker's last messages leave first.
                    #[cfg(feature = "gaggle")]
                    self.gaggle_worker_exit().await;
                    // Proactively close SSE clients so browsers see `event: closed`
                    // without waiting for the next 1 Hz hub tick / flume disconnect.
                    #[cfg(feature = "dashboard")]
                    if let Some(close_tx) = goose_attack_run_state.dashboard_close_tx.take() {
                        let _ = close_tx.send(true);
                    }
                    break;
                }
            }

            // Record current users for the active-users graph (report + dashboard).
            // Use the continuous series clock — not `started`, which restarts on
            // metrics reset and would wipe/offset ramp history on the chart.
            if self.series_started.is_some() {
                let _ = goose_attack_run_state
                    .metrics_cmd_tx
                    .send(MetricsCommand::RecordUsers {
                        active_users: goose_attack_run_state.active_users,
                        elapsed_secs: self.series_elapsed_secs(),
                    });
            };

            // `--gaggle-shadow-merge`: one delta each second.
            #[cfg(feature = "gaggle")]
            if self.gaggle_shadow.is_some() && self.series_started.is_some() {
                let now = self.series_elapsed_secs();
                if goose_attack_run_state.gaggle_shadow_second != Some(now) {
                    goose_attack_run_state.gaggle_shadow_second = Some(now);
                    let _ = goose_attack_run_state
                        .metrics_cmd_tx
                        .send(MetricsCommand::GaggleShadowTick { now });
                }
            }

            // Check if it's time to display running metrics, sending a command
            // to the dedicated metrics processor task if so.
            self.sync_metrics(goose_attack_run_state);

            // Check if it's time to check the thresholds, sending a command to the
            // dedicated metrics processor task if so.
            self.check_thresholds(goose_attack_run_state);

            // Check if the dashboard or a Controller has sent a command.
            self.handle_control_requests(goose_attack_run_state).await?;

            // A Gaggle manager's workers, or a worker's manager.
            #[cfg(feature = "gaggle")]
            self.gaggle_tick(goose_attack_run_state).await?;

            // Check if the dashboard has requested a snapshot.
            #[cfg(feature = "dashboard")]
            self.handle_dashboard_requests(goose_attack_run_state)
                .await?;

            let mut message = goose_attack_run_state.shutdown_rx.try_recv();
            while message.is_ok() {
                goose_attack_run_state
                    .users_shutdown
                    .insert(message.expect("failed to wrap OK message"));

                // In Stand-alone mode, all users are started.
                if goose_attack_run_state.users_shutdown.len() == self.test_plan.total_users() {
                    self.set_ended_by(runs::EndedBy::UsersExited, None);
                    self.cancel_attack(goose_attack_run_state).await?;
                }

                message = goose_attack_run_state.shutdown_rx.try_recv();
            }

            // Gracefully exit loop if ctrl-c is caught.
            if self.attack_phase != AttackPhase::Shutdown
                && !goose_attack_run_state.canceling
                && is_killswitch_triggered()
            {
                // Shutdown after stopping as the load test was canceled.
                goose_attack_run_state.shutdown_after_stop = true;

                if self.attack_phase == AttackPhase::Idle {
                    // Nothing to cancel: canceling from Idle would leave a
                    // plan maintaining 0 users with no end.
                    self.shutdown_from_idle(goose_attack_run_state);
                } else {
                    self.set_ended_by(runs::EndedBy::Canceled, killswitch_reason());
                    // Cleanly stop the load test.
                    self.cancel_attack(goose_attack_run_state).await?;
                }

                // Load test is actively canceling.
                goose_attack_run_state.canceling = true;
            }
        }

        Ok(())
    }
}

/// Use the configured GooseScheduler to allocate all [`Transaction`](./goose/struct.Transaction.html)s
/// within the [`Scenario`](./goose/struct.Scenario.html) in the appropriate order. Returns
/// three set of ordered transactions: `on_start_transactions`, `transactions`, and `on_stop_transactions`.
/// The `on_start_transactions` are only run once when the [`GooseAttack`](./struct.GooseAttack.html) first
/// starts. Normal `transactions` are then run for the duration of the
/// [`GooseAttack`](./struct.GooseAttack.html). The `on_stop_transactions` finally are only run once when
/// the [`GooseAttack`](./struct.GooseAttack.html) stops.
fn allocate_transactions(
    scenario: &Scenario,
    scheduler: &GooseScheduler,
) -> (
    WeightedTransactions,
    WeightedTransactions,
    WeightedTransactions,
) {
    debug!("allocating Transactions on GooseUsers with {scheduler:?} scheduler");

    // A BTreeMap of Vectors allows us to group and sort transactions per sequence value.
    let mut sequenced_transactions: SequencedTransactions = BTreeMap::new();
    let mut sequenced_on_start_transactions: SequencedTransactions = BTreeMap::new();
    let mut sequenced_on_stop_transactions: SequencedTransactions = BTreeMap::new();
    let mut unsequenced_transactions: UnsequencedTransactions = Vec::new();
    let mut unsequenced_on_start_transactions: UnsequencedTransactions = Vec::new();
    let mut unsequenced_on_stop_transactions: UnsequencedTransactions = Vec::new();
    let mut u: usize = 0;
    let mut v: usize;

    // Find the greatest common divisor of all transactions in the scenario.
    for transaction in &scenario.transactions {
        if transaction.sequence > 0 {
            if transaction.on_start {
                if let Some(sequence) =
                    sequenced_on_start_transactions.get_mut(&transaction.sequence)
                {
                    // This is another transaction with this order value.
                    sequence.push(transaction.clone());
                } else {
                    // This is the first transaction with this order value.
                    sequenced_on_start_transactions
                        .insert(transaction.sequence, vec![transaction.clone()]);
                }
            }
            // Allow a transaction to be both on_start and on_stop.
            if transaction.on_stop {
                if let Some(sequence) =
                    sequenced_on_stop_transactions.get_mut(&transaction.sequence)
                {
                    // This is another transaction with this order value.
                    sequence.push(transaction.clone());
                } else {
                    // This is the first transaction with this order value.
                    sequenced_on_stop_transactions
                        .insert(transaction.sequence, vec![transaction.clone()]);
                }
            }
            if !transaction.on_start && !transaction.on_stop {
                if let Some(sequence) = sequenced_transactions.get_mut(&transaction.sequence) {
                    // This is another transaction with this order value.
                    sequence.push(transaction.clone());
                } else {
                    // This is the first transaction with this order value.
                    sequenced_transactions.insert(transaction.sequence, vec![transaction.clone()]);
                }
            }
        } else {
            if transaction.on_start {
                unsequenced_on_start_transactions.push(transaction.clone());
            }
            if transaction.on_stop {
                unsequenced_on_stop_transactions.push(transaction.clone());
            }
            if !transaction.on_start && !transaction.on_stop {
                unsequenced_transactions.push(transaction.clone());
            }
        }
        // Look for lowest common divisor amongst all transactions of any weight.
        if u == 0 {
            u = transaction.weight;
        } else {
            v = transaction.weight;
            trace!("calculating greatest common denominator of {u} and {v}");
            u = util::gcd(u, v);
            trace!("inner gcd: {u}");
        }
    }
    // 'u' will always be the greatest common divisor
    debug!("gcd: {u}");

    // Apply weights to sequenced transactions.
    let weighted_sequenced_on_start_transactions =
        weight_sequenced_transactions(&sequenced_on_start_transactions, u);
    let weighted_sequenced_transactions = weight_sequenced_transactions(&sequenced_transactions, u);
    let weighted_sequenced_on_stop_transactions =
        weight_sequenced_transactions(&sequenced_on_stop_transactions, u);

    // Apply weights to unsequenced transactions.
    let (weighted_unsequenced_on_start_transactions, total_unsequenced_on_start_transactions) =
        weight_unsequenced_transactions(&unsequenced_on_start_transactions, u);
    let (weighted_unsequenced_transactions, total_unsequenced_transactions) =
        weight_unsequenced_transactions(&unsequenced_transactions, u);
    let (weighted_unsequenced_on_stop_transactions, total_unsequenced_on_stop_transactions) =
        weight_unsequenced_transactions(&unsequenced_on_stop_transactions, u);

    // The Random scheduler shuffles only the transactions a user runs over and
    // over; its on_start and on_stop transactions run in Serial order.
    let start_stop_scheduler = match scheduler {
        GooseScheduler::Random => &GooseScheduler::Serial,
        _ => scheduler,
    };

    // Schedule sequenced transactions.
    let scheduled_sequenced_on_start_transactions = schedule_sequenced_transactions(
        &weighted_sequenced_on_start_transactions,
        start_stop_scheduler,
    );
    let scheduled_sequenced_transactions =
        schedule_sequenced_transactions(&weighted_sequenced_transactions, scheduler);
    let scheduled_sequenced_on_stop_transactions = schedule_sequenced_transactions(
        &weighted_sequenced_on_stop_transactions,
        start_stop_scheduler,
    );

    // Schedule unsequenced transactions.
    let scheduled_unsequenced_on_start_transactions = schedule_unsequenced_transactions(
        &weighted_unsequenced_on_start_transactions,
        total_unsequenced_on_start_transactions,
        start_stop_scheduler,
    );
    let scheduled_unsequenced_transactions = schedule_unsequenced_transactions(
        &weighted_unsequenced_transactions,
        total_unsequenced_transactions,
        scheduler,
    );
    let scheduled_unsequenced_on_stop_transactions = schedule_unsequenced_transactions(
        &weighted_unsequenced_on_stop_transactions,
        total_unsequenced_on_stop_transactions,
        start_stop_scheduler,
    );

    // Finally build a Vector of tuples: (transaction id, transaction name)
    let mut on_start_transactions = Vec::new();
    let mut transactions = Vec::new();
    let mut on_stop_transactions = Vec::new();

    // Sequenced transactions come first.
    for transaction in scheduled_sequenced_on_start_transactions.iter() {
        on_start_transactions.extend(vec![(
            *transaction,
            scenario.transactions[*transaction].name.clone(),
        )])
    }
    for transaction in scheduled_sequenced_transactions.iter() {
        transactions.extend(vec![(
            *transaction,
            scenario.transactions[*transaction].name.clone(),
        )])
    }
    for transaction in scheduled_sequenced_on_stop_transactions.iter() {
        on_stop_transactions.extend(vec![(
            *transaction,
            scenario.transactions[*transaction].name.clone(),
        )])
    }

    // Unsequenced transactions come last.
    for transaction in scheduled_unsequenced_on_start_transactions.iter() {
        on_start_transactions.extend(vec![(
            *transaction,
            scenario.transactions[*transaction].name.clone(),
        )])
    }
    for transaction in scheduled_unsequenced_transactions.iter() {
        transactions.extend(vec![(
            *transaction,
            scenario.transactions[*transaction].name.clone(),
        )])
    }
    for transaction in scheduled_unsequenced_on_stop_transactions.iter() {
        on_stop_transactions.extend(vec![(
            *transaction,
            scenario.transactions[*transaction].name.clone(),
        )])
    }

    // Return sequenced buckets of weighted usize pointers to and names of Transactions.
    (on_start_transactions, transactions, on_stop_transactions)
}

/// Build a weighted vector of vectors of unsequenced Transactions.
fn weight_unsequenced_transactions(
    unsequenced_transactions: &[Transaction],
    u: usize,
) -> (Vec<Vec<usize>>, usize) {
    // Build a vector of vectors to be used to schedule users.
    let mut available_unsequenced_transactions = Vec::with_capacity(unsequenced_transactions.len());
    let mut total_transactions = 0;
    for transaction in unsequenced_transactions.iter() {
        // divide by greatest common divisor so vector is as short as possible
        let weight = transaction.weight / u;
        trace!(
            "{}: {} has weight of {} (reduced with gcd to {})",
            transaction.transactions_index,
            transaction.name.name_for_transaction(),
            transaction.weight,
            weight
        );
        let weighted_transactions = vec![transaction.transactions_index; weight];
        available_unsequenced_transactions.push(weighted_transactions);
        total_transactions += weight;
    }
    (available_unsequenced_transactions, total_transactions)
}

/// Build a weighted vector of vectors of sequenced Transactions.
fn weight_sequenced_transactions(
    sequenced_transactions: &SequencedTransactions,
    u: usize,
) -> BTreeMap<usize, Vec<Vec<usize>>> {
    // Build a sequenced BTreeMap containing weighted vectors of Transactions.
    let mut available_sequenced_transactions = BTreeMap::new();
    // Step through sequences, each containing a bucket of all Transactions with the same
    // sequence value, allowing actual weighting to be done by weight_unsequenced_transactions().
    for (sequence, unsequenced_transactions) in sequenced_transactions.iter() {
        let (weighted_transactions, _total_weighted_transactions) =
            weight_unsequenced_transactions(unsequenced_transactions, u);
        available_sequenced_transactions.insert(*sequence, weighted_transactions);
    }

    available_sequenced_transactions
}

fn schedule_sequenced_transactions(
    available_sequenced_transactions: &BTreeMap<usize, Vec<Vec<usize>>>,
    scheduler: &GooseScheduler,
) -> Vec<usize> {
    let mut weighted_transactions: Vec<usize> = Vec::new();

    for transactions in available_sequenced_transactions.values() {
        let scheduled_transactions =
            schedule_unsequenced_transactions(transactions, transactions[0].len(), scheduler);
        weighted_transactions.extend(scheduled_transactions);
    }

    weighted_transactions
}

// Return a list of transactions in the order to be run.
fn schedule_unsequenced_transactions(
    available_unsequenced_transactions: &[Vec<usize>],
    total_transactions: usize,
    scheduler: &GooseScheduler,
) -> Vec<usize> {
    // Now build the weighted list with the appropriate scheduler.
    let mut weighted_transactions = Vec::new();

    match scheduler {
        GooseScheduler::RoundRobin => {
            // Allocate round robin.
            let transactions_len = available_unsequenced_transactions.len();
            let mut available_transactions = available_unsequenced_transactions.to_owned();
            loop {
                // Transactions are contained in a vector of vectors. The outer vectors each
                // contain a different Transaction, and the inner vectors contain each
                // instance of that specific Transaction.
                for (transaction_index, transactions) in available_transactions
                    .iter_mut()
                    .enumerate()
                    .take(transactions_len)
                {
                    if let Some(transaction) = transactions.pop() {
                        debug!("allocating transaction from Transaction {transaction_index}");
                        weighted_transactions.push(transaction);
                    }
                }
                if weighted_transactions.len() >= total_transactions {
                    break;
                }
            }
        }
        GooseScheduler::Serial | GooseScheduler::Random => {
            // Allocate serially in the weighted order defined.
            for (transaction_index, transactions) in
                available_unsequenced_transactions.iter().enumerate()
            {
                debug!(
                    "allocating all {} transactions from Transaction {}",
                    transactions.len(),
                    transaction_index
                );
                weighted_transactions.extend_from_slice(transactions);
            }
            // The Random scheduler then shuffles the whole list once. Each
            // Transaction's own vector holds only copies of its index, so
            // shuffling those changes nothing.
            if scheduler == &GooseScheduler::Random {
                weighted_transactions.shuffle(&mut rand::rng());
            }
        }
    }

    weighted_transactions
}

/// The last `--gaggle-shadow-merge` outcome a run reported (unit tests).
#[cfg(all(test, feature = "gaggle"))]
static GAGGLE_SHADOW_LAST: Mutex<Option<gaggle::shadow::ShadowOutcome>> = Mutex::new(None);

/// The load test plan's hash, the same for the same plan on any build: a
/// 64 bit FNV-1a over each scenario's name, weight, `transaction_wait` and
/// host, and each of its transactions' name, weight, sequence and on start
/// and on stop flags, in declared order, then the scheduler. A Gaggle
/// manager checks every worker runs the same plan with it.
pub(crate) fn plan_hash(scenarios: &[Scenario], scheduler: &GooseScheduler) -> u64 {
    struct Fnv(u64);
    impl Fnv {
        fn bytes(&mut self, bytes: &[u8]) {
            for &byte in bytes {
                self.0 ^= u64::from(byte);
                self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        fn number(&mut self, value: u64) {
            self.bytes(&value.to_le_bytes());
        }
        fn text(&mut self, text: &str) {
            self.number(text.len() as u64);
            self.bytes(text.as_bytes());
        }
        fn option(&mut self, text: Option<&str>) {
            match text {
                Some(text) => {
                    self.number(1);
                    self.text(text);
                }
                None => self.number(0),
            }
        }
    }
    let mut hash = Fnv(0xcbf2_9ce4_8422_2325);
    hash.number(scenarios.len() as u64);
    for scenario in scenarios {
        hash.text(&scenario.name);
        hash.number(scenario.weight as u64);
        match scenario.transaction_wait {
            Some((min, max)) => {
                hash.number(1);
                hash.number(min.as_nanos() as u64);
                hash.number(max.as_nanos() as u64);
            }
            None => hash.number(0),
        }
        hash.option(scenario.host.as_deref());
        hash.number(scenario.transactions.len() as u64);
        for transaction in &scenario.transactions {
            match &transaction.name {
                TransactionName::InheritNameByRequests(name) => {
                    hash.number(0);
                    hash.text(name);
                }
                TransactionName::TransactionOnly(name) => {
                    hash.number(1);
                    hash.text(name);
                }
            }
            hash.number(transaction.weight as u64);
            hash.number(transaction.sequence as u64);
            hash.number(u64::from(transaction.on_start));
            hash.number(u64::from(transaction.on_stop));
        }
    }
    hash.number(match scheduler {
        GooseScheduler::RoundRobin => 0,
        GooseScheduler::Serial => 1,
        GooseScheduler::Random => 2,
    });
    hash.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The telnet Controller listens by default and the WebSocket Controller
    /// only with `--websocket`, and with no host given both listen on
    /// 127.0.0.1 only: a connection to this machine's own non-loopback
    /// address, where it has one, is refused.
    #[tokio::test]
    async fn websocket_controller_listens_only_when_asked_for() {
        /// A port nothing listens on, for now.
        fn free_port() -> String {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port().to_string()
        }
        for websocket in [false, true] {
            let (telnet_port, websocket_port) = (free_port(), free_port());
            let mut args = vec![
                "--telnet-port",
                &telnet_port,
                "--websocket-port",
                &websocket_port,
            ];
            if websocket {
                args.push("--websocket");
            }
            let configuration = GooseConfiguration::parse_args_default(&args).unwrap();
            let mut goose_attack = GooseAttack::initialize_with_config(configuration).unwrap();
            let defaults = goose_attack.defaults.clone();
            goose_attack.configuration.configure(&defaults).unwrap();
            goose_attack.configuration.validate().unwrap();
            let (control_tx, _control_rx) = flume::unbounded();
            let listening = goose_attack.setup_controllers(&control_tx).await;
            assert!(listening.telnet);
            assert_eq!(listening.websocket, websocket);
            let connects = |host: std::net::IpAddr, port: &str| {
                let address = std::net::SocketAddr::new(host, port.parse().unwrap());
                async move { tokio::net::TcpStream::connect(address).await.is_ok() }
            };
            let loopback = std::net::Ipv4Addr::LOCALHOST.into();
            assert!(connects(loopback, &telnet_port).await);
            if let Some(lan) = lan_address() {
                assert!(!connects(lan, &telnet_port).await, "telnet on {}", lan);
                assert!(
                    !connects(lan, &websocket_port).await,
                    "WebSocket on {}",
                    lan
                );
            }
            assert_eq!(
                connects(loopback, &websocket_port).await,
                websocket,
                "--websocket {}",
                websocket
            );
        }
    }

    /// This machine's address on the network a default route leads to, if it
    /// has one: a UDP connect picks the source address and sends nothing.
    fn lan_address() -> Option<std::net::IpAddr> {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
        socket.connect("192.0.2.1:9").ok()?;
        let ip = socket.local_addr().ok()?.ip();
        (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
    }

    // Serial with every test that calls `initialize_attack`: installing the
    // Ctrl-C handler a second time resets the killswitch this test reads.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_killswitch_functions() {
        // Reset the killswitch state for testing
        reset_killswitch();

        // Initially, killswitch should not be triggered
        assert!(
            !is_killswitch_triggered(),
            "Killswitch should initially be false"
        );

        // Trigger the killswitch with a reason
        trigger_killswitch("Test reason: threshold exceeded");

        // Verify killswitch is now triggered
        assert!(
            is_killswitch_triggered(),
            "Killswitch should be true after triggering"
        );

        // Trigger again with a different reason (should not log since already triggered)
        trigger_killswitch("Test reason: second trigger");

        // Verify it's still triggered
        assert!(is_killswitch_triggered(), "Killswitch should remain true");

        // Only the first reason is kept.
        assert_eq!(
            killswitch_reason().as_deref(),
            Some("Test reason: threshold exceeded")
        );

        // Reset for other tests
        reset_killswitch();

        assert!(
            !is_killswitch_triggered(),
            "Killswitch should be false after reset"
        );
    }

    /// Checking starts when the run first enters Maintain, through
    /// `set_attack_phase`. From there the main loop sends `CheckThresholds` at most
    /// once a second, in Maintain and in a later Increase but never in Decrease or
    /// Idle, once `--threshold-abort-delay` has passed since Maintain. Without a
    /// threshold that can stop the run it sends no check. With an `rps` rule, in
    /// every mode, each entry to Maintain and each exit from it is sent, for the
    /// holds `rps` rules count.
    #[tokio::test]
    #[serial_test::serial]
    async fn checks_start_when_the_run_first_enters_maintain() {
        /// An idle attack with `args`, whose metrics commands come out of the receiver.
        async fn attack(
            args: &[&str],
        ) -> (
            GooseAttack,
            GooseAttackRunState,
            flume::Receiver<MetricsCommand>,
        ) {
            let mut all = vec!["--no-telnet", "--no-websocket"];
            all.extend_from_slice(args);
            let configuration = GooseConfiguration::parse_args_default(&all).unwrap();
            let mut goose_attack = GooseAttack::initialize_with_config(configuration).unwrap();
            let defaults = goose_attack.defaults.clone();
            goose_attack.configuration.configure(&defaults).unwrap();
            let mut run_state = goose_attack.initialize_attack().await.unwrap();
            let (metrics_cmd_tx, metrics_cmd_rx) = flume::unbounded();
            run_state.metrics_cmd_tx = metrics_cmd_tx;
            assert_eq!(goose_attack.attack_phase, AttackPhase::Idle);
            (goose_attack, run_state, metrics_cmd_rx)
        }

        /// Enter `phase`, then take two loop turns in the same second: the checks,
        /// the entries to Maintain and the exits from it sent.
        fn enter(
            goose_attack: &mut GooseAttack,
            run_state: &mut GooseAttackRunState,
            commands: &flume::Receiver<MetricsCommand>,
            phase: AttackPhase,
        ) -> (usize, usize, usize) {
            goose_attack.set_attack_phase(run_state, phase);
            goose_attack.check_thresholds(run_state);
            goose_attack.check_thresholds(run_state);
            commands.try_iter().fold(
                (0, 0, 0),
                |(checks, enters, leaves), command| match command {
                    MetricsCommand::CheckThresholds { .. } => (checks + 1, enters, leaves),
                    MetricsCommand::EnterMaintain { .. } => (checks, enters + 1, leaves),
                    MetricsCommand::LeaveMaintain { .. } => (checks, enters, leaves + 1),
                    _ => (checks, enters, leaves),
                },
            )
        }

        let (mut goose_attack, mut run_state, commands) =
            attack(&["--threshold", "p95<500ms", "--threshold", "rps>=1"]).await;
        // Nothing while users start, before the run first holds them.
        assert_eq!(
            enter(
                &mut goose_attack,
                &mut run_state,
                &commands,
                AttackPhase::Increase
            ),
            (0, 0, 0)
        );
        // Checking starts in Maintain, once.
        assert_eq!(
            enter(
                &mut goose_attack,
                &mut run_state,
                &commands,
                AttackPhase::Maintain
            ),
            (1, 1, 0)
        );
        // A second later, a later Increase is checked, though it ends the hold,
        // and Decrease is not checked.
        run_state.threshold_check_at = Some(std::time::Instant::now());
        assert_eq!(
            enter(
                &mut goose_attack,
                &mut run_state,
                &commands,
                AttackPhase::Increase
            ),
            (1, 0, 1)
        );
        run_state.threshold_check_at = Some(std::time::Instant::now());
        assert_eq!(
            enter(
                &mut goose_attack,
                &mut run_state,
                &commands,
                AttackPhase::Decrease
            ),
            (0, 0, 0)
        );
        // Entering Maintain again starts a hold without starting checking again.
        let due = run_state.threshold_check_at;
        goose_attack.set_attack_phase(&mut run_state, AttackPhase::Maintain);
        assert_eq!(run_state.threshold_check_at, due);
        run_state.threshold_check_at = Some(std::time::Instant::now());
        // The hold was sent, and Idle ends it, sending no check.
        assert_eq!(
            enter(
                &mut goose_attack,
                &mut run_state,
                &commands,
                AttackPhase::Idle
            ),
            (0, 1, 1)
        );

        // The delay counts from Maintain. Without an `rps` rule no hold is sent.
        let (mut goose_attack, mut run_state, commands) =
            attack(&["--threshold", "p95<500ms", "--threshold-abort-delay", "3"]).await;
        assert_eq!(
            enter(
                &mut goose_attack,
                &mut run_state,
                &commands,
                AttackPhase::Increase
            ),
            (0, 0, 0)
        );
        let entered = std::time::Instant::now();
        assert_eq!(
            enter(
                &mut goose_attack,
                &mut run_state,
                &commands,
                AttackPhase::Maintain
            ),
            (0, 0, 0)
        );
        let due = run_state.threshold_check_at.expect("checking started");
        assert!(due >= entered + Duration::from_secs(3));
        assert!(due <= std::time::Instant::now() + Duration::from_secs(3));

        // No check without a threshold that can stop the run, but an `rps` rule
        // still counts its holds under `--no-threshold-abort`.
        for (args, holds) in [
            (&[][..], 0),
            (&["--threshold", "p95<500ms", "--no-threshold-abort"], 0),
            (&["--threshold", "rps>=1", "--no-threshold-abort"], 1),
        ] {
            let (mut goose_attack, mut run_state, commands) = attack(args).await;
            for (phase, expected) in [
                (AttackPhase::Increase, (0, 0, 0)),
                (AttackPhase::Maintain, (0, holds, 0)),
                (AttackPhase::Decrease, (0, 0, holds)),
            ] {
                assert_eq!(
                    enter(&mut goose_attack, &mut run_state, &commands, phase),
                    expected,
                    "{:?} {:?}",
                    args,
                    phase
                );
            }
            assert_eq!(run_state.threshold_check_at, None);
        }
    }

    /// A `main` that returns `ThresholdsFailed` prints the rules that failed, not
    /// the metrics of the whole run, which still read through the error.
    #[test]
    fn thresholds_failed_prints_only_the_rules() {
        let error = GooseError::ThresholdsFailed {
            failed: vec![threshold::ThresholdResult {
                rule: "p95<500ms".to_string(),
                observed: "812ms".to_string(),
                value: Some(812.0),
                passed: false,
                aborted: true,
            }],
            metrics: ThresholdMetrics(Box::new(GooseMetrics {
                duration: 7,
                ..Default::default()
            })),
        };
        assert_eq!(
            format!("{error:?}"),
            "ThresholdsFailed { failed: [ThresholdResult { rule: \"p95<500ms\", \
             observed: \"812ms\", value: Some(812.0), passed: false, aborted: true }], \
             metrics: ThresholdMetrics { .. } }"
        );
        assert_eq!(
            error.to_string(),
            "GooseError: one or more thresholds failed (p95<500ms, observed 812ms)"
        );
        let GooseError::ThresholdsFailed { metrics, .. } = error else {
            unreachable!()
        };
        assert_eq!(metrics.duration, 7);
        assert_eq!(metrics.0.duration, 7);
    }

    /// A refused Start names its one cause: not idle, no host, or no users.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_control_start_refusal_names_cause() {
        async fn noop(_user: &mut GooseUser) -> crate::goose::TransactionResult {
            Ok(())
        }
        let configuration =
            GooseConfiguration::parse_args_default(&["--no-telnet", "--no-websocket"]).unwrap();
        let mut goose_attack = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(scenario!("NoHost").register_transaction(transaction!(noop)));
        let mut run_state = goose_attack.initialize_attack().await.unwrap();
        let mut refusal = |goose_attack: &mut GooseAttack| match goose_attack
            .control_start_begin(&mut run_state)
            .unwrap()
        {
            ControlStartBegin::Rejected(outcome) => (outcome.error, outcome.message),
            ControlStartBegin::Accepted(outcome) => panic!("accepted: {:?}", outcome),
        };

        goose_attack.configuration.users = Some(1);
        assert_eq!(
            refusal(&mut goose_attack),
            (
                Some("prepare_failed"),
                "unable to start load test, no host is configured".to_string()
            )
        );

        goose_attack.configuration.host = Some("http://127.0.0.1".to_string());
        goose_attack.configuration.users = None;
        goose_attack.configuration.test_plan = None;
        // With no scenario, a Start that got past the users check would fail
        // the test instead of weighting zero users forever.
        goose_attack.scenarios.clear();
        assert_eq!(
            refusal(&mut goose_attack),
            (
                Some("prepare_failed"),
                "unable to start load test, there are no users to start".to_string()
            )
        );

        goose_attack.attack_phase = AttackPhase::Maintain;
        assert_eq!(
            refusal(&mut goose_attack),
            (
                Some("invalid_phase"),
                "unable to start load test, it is not idle".to_string()
            )
        );
    }

    /// A Start with scenario hosts only, no default host and a `test_start`
    /// or `test_stop` transaction is refused before it replies, naming the
    /// host and the transaction; it used to panic after replying ok. An autostarted run fails
    /// at launch instead.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_start_refused_without_host_for_test_start() {
        async fn noop(_user: &mut GooseUser) -> crate::goose::TransactionResult {
            Ok(())
        }
        for test_stop in [false, true] {
            let configuration = GooseConfiguration::parse_args_default(&[
                "--no-telnet",
                "--no-websocket",
                "--users",
                "1",
            ])
            .unwrap();
            let goose_attack = GooseAttack::initialize_with_config(configuration)
                .unwrap()
                .register_scenario(
                    scenario!("Hosted")
                        .set_host("http://127.0.0.1")
                        .register_transaction(transaction!(noop)),
                );
            let mut goose_attack = if test_stop {
                goose_attack.test_stop(transaction!(noop))
            } else {
                goose_attack.test_start(transaction!(noop))
            };
            let mut run_state = goose_attack.initialize_attack().await.unwrap();
            match goose_attack.control_start_begin(&mut run_state).unwrap() {
                ControlStartBegin::Rejected(outcome) => {
                    assert_eq!(outcome.error, Some("prepare_failed"));
                    assert_eq!(outcome.field, Some("host"));
                    let transaction = if test_stop { "test_stop" } else { "test_start" };
                    assert_eq!(
                        outcome.message,
                        format!(
                            "unable to start load test, no host is configured for the {transaction} transaction"
                        )
                    );
                }
                ControlStartBegin::Accepted(outcome) => panic!("accepted: {:?}", outcome),
            }
            assert_eq!(goose_attack.attack_phase, AttackPhase::Idle);

            // A default host is enough for the transaction.
            goose_attack.defaults.host = Some("http://127.0.0.1".to_string());
            assert!(matches!(
                goose_attack.control_start_begin(&mut run_state).unwrap(),
                ControlStartBegin::Accepted(_)
            ));
        }

        // Autostarted, the same configuration is a startup error.
        let configuration = GooseConfiguration::parse_args_default(&[
            "--no-telnet",
            "--no-websocket",
            "--users",
            "1",
            "--quiet",
        ])
        .unwrap();
        let error = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(
                scenario!("Hosted")
                    .set_host("http://127.0.0.1")
                    .register_transaction(transaction!(noop)),
            )
            .test_start(transaction!(noop))
            .execute()
            .await
            .unwrap_err();
        assert!(
            matches!(&error, GooseError::InvalidOption { option, .. } if option == NO_HOST_OPTION),
            "{}",
            error
        );
    }

    /// The snapshot's run settings: the configured values, hosts resolved
    /// per scenario without `get_base_url`, and the stage per snapshot.
    #[cfg(feature = "dashboard")]
    #[tokio::test]
    #[serial_test::serial]
    async fn test_snapshot_settings() {
        async fn noop(_user: &mut GooseUser) -> crate::goose::TransactionResult {
            Ok(())
        }
        let configuration = GooseConfiguration::parse_args_default(&[
            "--no-telnet",
            "--no-websocket",
            "--no-autostart",
            "--users",
            "100",
            "--increase-rate",
            "10",
            "--run-time",
            "10m",
            "--decrease-time",
            "30s",
        ])
        .unwrap();
        // With --no-autostart and no host, a scenario without one adds no
        // host, and nothing panics.
        let mut goose_attack = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(scenario!("NoHost").register_transaction(transaction!(noop)))
            .register_scenario(
                scenario!("Hosted")
                    .set_host("http://localhost")
                    .register_transaction(transaction!(noop)),
            )
            .register_scenario(
                scenario!("Again")
                    .set_host("http://localhost/")
                    .register_transaction(transaction!(noop)),
            );
        let mut run_state = goose_attack.initialize_attack().await.unwrap();
        let settings = goose_attack.snapshot_settings(&run_state);
        assert_eq!(
            settings,
            metrics::dashboard_snapshot::SnapshotSettings {
                host: None,
                hosts: vec!["http://localhost/".to_string()],
                users: Some(100),
                increase_rate: Some(10.0),
                increase_time_secs: None,
                decrease_rate: None,
                decrease_time_secs: Some(30),
                run_time_secs: Some(600),
                test_plan: None,
                hold_ends_in_secs: None,
                stage: "idle".to_string(),
            }
        );
        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(json["increase_rate"], serde_json::json!(10.0));

        // A default host fills in for the scenario with none, and --host
        // overrides them all; settings are rebuilt after a command.
        goose_attack.defaults.host = Some("https://example.com".to_string());
        goose_attack.settings = None;
        assert_eq!(
            goose_attack.snapshot_settings(&run_state).hosts,
            ["http://localhost/", "https://example.com/"]
        );
        assert!(
            controller::tests::control(
                &mut goose_attack,
                &mut run_state,
                "host https://override.test"
            )
            .await
        );
        let settings = goose_attack.snapshot_settings(&run_state);
        assert_eq!(settings.host.as_deref(), Some("https://override.test"));
        assert_eq!(settings.hosts, ["https://override.test/"]);

        // A test plan in canonical form, with no users.
        assert!(
            controller::tests::control(&mut goose_attack, &mut run_state, "testplan 10,1m30s;0,0")
                .await
        );
        let settings = goose_attack.snapshot_settings(&run_state);
        assert_eq!(settings.test_plan.as_deref(), Some("10,90;0,0"));
        assert_eq!(settings.users, None);
        assert_eq!(settings.run_time_secs, None);

        // Stages: a running test plan, then a cancel.
        goose_attack.set_attack_phase(&mut run_state, AttackPhase::Increase);
        assert_eq!(
            goose_attack.snapshot_settings(&run_state).stage,
            "test_plan"
        );
        run_state.stopping = true;
        assert_eq!(goose_attack.snapshot_settings(&run_state).stage, "stopping");
        goose_attack.set_attack_phase(&mut run_state, AttackPhase::Idle);
        assert_eq!(goose_attack.snapshot_settings(&run_state).stage, "idle");
    }

    /// Where the run is: Idle, a users ramp's stage, a running test plan,
    /// and stopping for a cancel or Shutdown.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_stage() {
        use run_shape::Stage;
        let configuration = GooseConfiguration::parse_args_default(&[
            "--no-telnet",
            "--no-websocket",
            "--users",
            "4",
        ])
        .unwrap();
        let mut goose_attack = GooseAttack::initialize_with_config(configuration).unwrap();
        let mut run_state = goose_attack.initialize_attack().await.unwrap();
        assert_eq!(goose_attack.stage(&run_state), Stage::Idle);
        controller::tests::run_users_ramp(&mut goose_attack, &mut run_state, 4);
        assert_eq!(goose_attack.stage(&run_state), Stage::RampUp);
        goose_attack.run_shape = None;
        assert_eq!(goose_attack.stage(&run_state), Stage::TestPlan);
        run_state.stopping = true;
        assert_eq!(goose_attack.stage(&run_state), Stage::Stopping);
        run_state.stopping = false;
        goose_attack.set_attack_phase(&mut run_state, AttackPhase::Shutdown);
        assert_eq!(goose_attack.stage(&run_state), Stage::Stopping);
    }

    /// A new step schedules its users from its own start: a ramp a Controller
    /// left mid way leaves no due time behind, which the next ramp would catch
    /// up on in a burst.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_new_step_forgets_the_last_due_time() {
        let configuration = GooseConfiguration::parse_args_default(&[
            "--no-telnet",
            "--no-websocket",
            "--users",
            "4",
        ])
        .unwrap();
        let mut goose_attack = GooseAttack::initialize_with_config(configuration).unwrap();
        let mut run_state = goose_attack.initialize_attack().await.unwrap();
        controller::tests::run_users_ramp(&mut goose_attack, &mut run_state, 4);
        // Mid ramp, with the next user due a minute ago.
        run_state.adjust_user_in_ms = 667;
        run_state.adjust_user_timer = std::time::Instant::now() - Duration::from_secs(60);
        goose_attack.advance_test_plan(&mut run_state);
        assert_eq!(run_state.adjust_user_in_ms, 0);
        assert!(run_state.adjust_due());
    }

    /// A refused Start leaves the run settings and target users as they were.
    #[cfg(feature = "dashboard")]
    #[tokio::test]
    #[serial_test::serial]
    async fn test_refused_start_leaves_settings() {
        async fn noop(_user: &mut GooseUser) -> crate::goose::TransactionResult {
            Ok(())
        }
        let configuration = GooseConfiguration::parse_args_default(&[
            "--no-telnet",
            "--no-websocket",
            "--users",
            "5",
        ])
        .unwrap();
        let mut goose_attack = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(scenario!("NoHost").register_transaction(transaction!(noop)));
        let mut run_state = goose_attack.initialize_attack().await.unwrap();
        let settings = goose_attack.snapshot_settings(&run_state);
        let target = goose_attack.current_target_users();
        assert!(!controller::tests::control(&mut goose_attack, &mut run_state, "start").await);
        assert_eq!(goose_attack.snapshot_settings(&run_state), settings);
        assert_eq!(goose_attack.current_target_users(), target);
        assert!(goose_attack.run_shape.is_none());
    }

    /// A Start with settings whose prepare fails puts every setting back:
    /// the run settings, the target users and the configuration are as they
    /// were, and no run shape is left.
    #[cfg(feature = "dashboard")]
    #[tokio::test]
    #[serial_test::serial]
    async fn test_refused_start_with_settings_leaves_settings() {
        async fn noop(_user: &mut GooseUser) -> crate::goose::TransactionResult {
            Ok(())
        }
        let configuration = GooseConfiguration::parse_args_default(&[
            "--no-telnet",
            "--no-websocket",
            "--users",
            "5",
            "--increase-rate",
            "2",
        ])
        .unwrap();
        let mut goose_attack = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(scenario!("NoHost").register_transaction(transaction!(noop)));
        let mut run_state = goose_attack.initialize_attack().await.unwrap();
        let settings = goose_attack.snapshot_settings(&run_state);
        let target = goose_attack.current_target_users();
        let configuration = format!("{:?}", goose_attack.configuration);
        let body = r#"{"users":8,"increase_time_secs":4,"run_time_secs":30,"decrease_rate":3}"#;
        let refused =
            controller::tests::settings(&mut goose_attack, &mut run_state, body, true).await;
        assert!(!refused.ok);
        assert_eq!(refused.error, Some("prepare_failed"));
        assert_eq!(refused.field, Some("host"));
        assert_eq!(refused.phase, "idle");
        assert_eq!(goose_attack.snapshot_settings(&run_state), settings);
        assert_eq!(refused.settings.as_deref(), Some(&settings));
        assert_eq!(goose_attack.current_target_users(), target);
        assert_eq!(format!("{:?}", goose_attack.configuration), configuration);
        assert!(goose_attack.run_shape.is_none());
    }

    /// Resolving a base url with no host from any source is an error, not a
    /// panic.
    #[test]
    fn get_base_url_without_host_is_an_error() {
        assert!(goose::get_base_url(None, None, None).is_err());
    }

    /// Users is refused while a cancel ramps the run down, accepted during a
    /// users ramp's own ramp down, and accepted again once the run is Idle.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_set_users_refused_while_stopping() {
        let configuration =
            GooseConfiguration::parse_args_default(&["--no-telnet", "--no-websocket"]).unwrap();
        let mut goose_attack = GooseAttack::initialize_with_config(configuration).unwrap();
        let mut run_state = goose_attack.initialize_attack().await.unwrap();

        // A users ramp's own ramp down from 10 users to 0.
        goose_attack.configuration.users = Some(10);
        goose_attack.configuration.decrease_rate = Some("1".to_string());
        controller::tests::run_users_ramp(&mut goose_attack, &mut run_state, 10);
        goose_attack.advance_test_plan(&mut run_state);
        assert_eq!(goose_attack.attack_phase, AttackPhase::Decrease);
        assert!(!run_state.stopping);
        let outcome = goose_attack.control_set_users(&mut run_state, 5).unwrap();
        assert!(outcome.ok, "ramp down must accept users: {:?}", outcome);
        assert_eq!(outcome.target_users, Some(5));

        // A cancel, as Stop, `shutdown` and Ctrl-C all start one.
        goose_attack.cancel_attack(&mut run_state).await.unwrap();
        assert!(run_state.stopping);
        assert_eq!(goose_attack.attack_phase, AttackPhase::Decrease);
        let outcome = goose_attack.control_set_users(&mut run_state, 5).unwrap();
        assert!(!outcome.ok, "cancel must refuse users: {:?}", outcome);
        assert_eq!(outcome.error, Some("invalid_phase"));
        assert_eq!(outcome.phase, "decrease");
        assert_eq!(outcome.target_users, None);
        // The cancel's ramp to zero is left in place.
        assert_eq!(goose_attack.test_plan.steps.last(), Some(&(0, 0)));
        assert_eq!(goose_attack.attack_phase, AttackPhase::Decrease);

        // Reaching Idle ends the cancel, so a later run can change users.
        goose_attack.set_attack_phase(&mut run_state, AttackPhase::Idle);
        assert!(!run_state.stopping);
        let outcome = goose_attack.control_set_users(&mut run_state, 5).unwrap();
        assert!(outcome.ok, "idle must accept users: {:?}", outcome);
    }

    /// A Stop moves a running load test to Decrease, and a second Stop while
    /// it ramps down is refused as `invalid_phase` without touching the cancel.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_second_stop_refused_while_decreasing() {
        let configuration =
            GooseConfiguration::parse_args_default(&["--no-telnet", "--no-websocket"]).unwrap();
        let mut goose_attack = GooseAttack::initialize_with_config(configuration).unwrap();
        let mut run_state = goose_attack.initialize_attack().await.unwrap();

        // A test plan maintaining 10 users.
        goose_attack.test_plan = TestPlan {
            steps: vec![(10, 0), (10, 10_000)],
            current: 0,
        };
        run_state.active_users = 10;
        goose_attack.advance_test_plan(&mut run_state);
        assert_eq!(goose_attack.attack_phase, AttackPhase::Maintain);

        let first = goose_attack.control_stop(&mut run_state).await.unwrap();
        assert!(first.ok, "maintain must accept stop: {:?}", first);
        assert_eq!(first.phase, "decrease");
        assert_eq!(goose_attack.attack_phase, AttackPhase::Decrease);
        assert!(run_state.stopping);
        let steps = goose_attack.test_plan.steps.clone();
        let current = goose_attack.test_plan.current;
        let history_len = goose_attack.metrics.history.len();

        let second = goose_attack.control_stop(&mut run_state).await.unwrap();
        assert!(!second.ok, "decrease must refuse stop: {:?}", second);
        assert_eq!(second.error, Some("invalid_phase"));
        assert_eq!(second.phase, "decrease");
        // The first Stop's ramp to zero is left in place.
        assert_eq!(goose_attack.test_plan.steps, steps);
        assert_eq!(goose_attack.test_plan.current, current);
        assert_eq!(goose_attack.metrics.history.len(), history_len);
        assert_eq!(goose_attack.attack_phase, AttackPhase::Decrease);
        assert!(run_state.stopping);
        assert!(goose_attack.configuration.no_autostart);
        assert!(!run_state.shutdown_after_stop);
    }

    /// Every snapshot the main loop serves carries the stopping flag: the one
    /// forwarded to the metrics processor, the local fallback when the
    /// processor is gone, and the one served in the tick of a command, which
    /// the stopping flag then refuses.
    #[cfg(feature = "dashboard")]
    #[tokio::test]
    #[serial_test::serial]
    async fn test_dashboard_snapshot_carries_stopping() {
        let configuration =
            GooseConfiguration::parse_args_default(&["--no-telnet", "--no-websocket"]).unwrap();
        let mut goose_attack = GooseAttack::initialize_with_config(configuration).unwrap();
        let mut run_state = goose_attack.initialize_attack().await.unwrap();
        let (dashboard_tx, dashboard_rx) = flume::unbounded();
        run_state.dashboard_channel_rx = Some(dashboard_rx);
        let (metrics_cmd_tx, metrics_cmd_rx) = flume::unbounded();
        run_state.metrics_cmd_tx = metrics_cmd_tx;
        run_state.stopping = true;

        // Forwarded to the metrics processor, with the hub's heatmap cursor.
        let cursor = metrics::dashboard_snapshot::HeatmapCursor {
            generation: 7,
            rev: 41,
        };
        let (respond, _snapshot_rx) = tokio::sync::oneshot::channel();
        dashboard_tx
            .send(dashboard::DashboardRequest::GetSnapshot {
                respond,
                heatmap_since: Some(cursor),
            })
            .unwrap();
        goose_attack
            .handle_dashboard_requests(&mut run_state)
            .await
            .unwrap();
        let Ok(MetricsCommand::GetDashboardSnapshot {
            stopping,
            heatmap_since,
            ..
        }) = metrics_cmd_rx.try_recv()
        else {
            panic!("expected a forwarded snapshot request");
        };
        assert!(stopping);
        assert_eq!(heatmap_since, Some(cursor));

        // Processor gone: served from main-loop state.
        drop(metrics_cmd_rx);
        let (respond, snapshot_rx) = tokio::sync::oneshot::channel();
        dashboard_tx
            .send(dashboard::DashboardRequest::GetSnapshot {
                respond,
                heatmap_since: None,
            })
            .unwrap();
        goose_attack
            .handle_dashboard_requests(&mut run_state)
            .await
            .unwrap();
        assert!(snapshot_rx.await.unwrap().stopping);

        // Served in the tick of a command, which then refuses Users.
        let (respond, snapshot_rx) = tokio::sync::oneshot::channel();
        dashboard_tx
            .send(dashboard::DashboardRequest::GetSnapshot {
                respond,
                heatmap_since: None,
            })
            .unwrap();
        goose_attack
            .handle_dashboard_requests(&mut run_state)
            .await
            .unwrap();
        assert!(snapshot_rx.await.unwrap().stopping);
        assert!(!controller::tests::control(&mut goose_attack, &mut run_state, "users 5").await);
    }

    /// A `--scenarios` pattern that matches no machine name is reported, as
    /// `load_test` is for the scenario `loadtest`.
    #[test]
    fn unmatched_scenario_patterns_are_named() {
        let configuration =
            GooseConfiguration::parse_args_default(&["--scenarios", "load_test,load*,nope*"])
                .unwrap();
        let goose_attack = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(Scenario::new("LoadTest"));
        assert_eq!(
            goose_attack.unmatched_scenario_patterns(),
            ["load_test", "nope*"]
        );
    }

    /// A run started after another, as a Controller start after stop does,
    /// counts its requests from zero.
    #[tokio::test]
    async fn a_new_run_counts_requests_from_zero() {
        let configuration = GooseConfiguration::parse_args_default(&[
            "--no-telnet",
            "--no-websocket",
            "--host",
            "http://localhost",
        ])
        .unwrap();
        let mut goose_attack = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(
                Scenario::new("LoadTest").register_transaction(Transaction::new(Arc::new(
                    |user: &mut GooseUser| Box::pin(async move { user.get("/").await.map(|_| ()) }),
                ))),
            );
        let defaults = goose_attack.defaults.clone();
        goose_attack.configuration.configure(&defaults).unwrap();
        let mut run_state = goose_attack.initialize_attack().await.unwrap();
        let counters = Arc::new(metrics::GooseRequestCounters::new());
        counters
            .success
            .fetch_add(5, std::sync::atomic::Ordering::Relaxed);
        counters
            .failure
            .fetch_add(2, std::sync::atomic::Ordering::Relaxed);
        run_state
            .request_counter_registry
            .lock()
            .unwrap()
            .insert("GET /".to_string(), counters.clone());
        let epoch = run_state
            .metrics_epoch
            .load(std::sync::atomic::Ordering::Relaxed);

        goose_attack.reset_run_state(&mut run_state).await.unwrap();

        assert_eq!(
            counters.success.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(
            counters.failure.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert!(
            run_state
                .metrics_epoch
                .load(std::sync::atomic::Ordering::Relaxed)
                > epoch
        );
    }

    /// A duplicate scenario name, renamed `name_1`, selects only that
    /// scenario, and `--scenarios-list` names both.
    #[test]
    fn duplicate_scenario_name_can_be_selected() {
        let configuration =
            GooseConfiguration::parse_args_default(&["--scenarios", "loadtest_1"]).unwrap();
        let goose_attack = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(Scenario::new("LoadTest"))
            .register_scenario(Scenario::new("LoadTest"));
        let names: Vec<&str> = goose_attack
            .scenarios
            .iter()
            .map(|scenario| &*scenario.machine_name)
            .collect();
        assert_eq!(names, ["loadtest", "loadtest_1"]);
        assert!(!goose_attack.scenario_is_active(&goose_attack.scenarios[0]));
        assert!(goose_attack.scenario_is_active(&goose_attack.scenarios[1]));
        assert!(goose_attack.unmatched_scenario_patterns().is_empty());
        assert_eq!(
            goose_attack.scenarios_list(),
            "Scenarios:\n - loadtest: (\"LoadTest\")\n - loadtest_1: (\"LoadTest\")\n"
        );
    }

    #[test]
    fn test_wildcard_match_exact() {
        assert!(wildcard_match("scenarioa1", "scenarioa1"));
        assert!(!wildcard_match("scenarioa1", "scenarioa1extended"));
        assert!(!wildcard_match("scenarioa1", "prescenarioa1"));
        assert!(!wildcard_match("scenarioa1", "scenarioa2"));
    }

    #[test]
    fn test_wildcard_match_star() {
        // Prefix match.
        assert!(wildcard_match("scenario*", "scenarioa1"));
        assert!(wildcard_match("scenario*", "scenariob2"));
        assert!(!wildcard_match("scenario*", "other"));

        // Suffix match.
        assert!(wildcard_match("*a1", "scenarioa1"));
        assert!(!wildcard_match("*a1", "scenarioa2"));

        // Contains match.
        assert!(wildcard_match("*nario*", "scenarioa1"));
        assert!(!wildcard_match("*nario*", "other"));

        // Match all.
        assert!(wildcard_match("*", "anything"));
        assert!(wildcard_match("*", ""));

        // Multiple wildcards.
        assert!(wildcard_match("*a*b*", "xaxbx"));
        assert!(!wildcard_match("*a*b*", "xbxa"));
    }

    #[test]
    fn test_wildcard_match_issue_612() {
        // The exact bug: "put1024bytes" should NOT match "getput1024bytes".
        assert!(wildcard_match("put1024bytes", "put1024bytes"));
        assert!(!wildcard_match("put1024bytes", "getput1024bytes"));
        assert!(!wildcard_match("put1024bytes", "put1024bytesextra"));
    }

    /// Before the first Start no metrics processor has run, and the snapshot
    /// built from the main loop still says percentiles over time are on.
    #[cfg(feature = "dashboard")]
    #[test]
    fn local_dashboard_snapshot_reports_percentiles_over_time() {
        for (args, expected) in [
            (vec!["--dashboard"], true),
            (vec!["--dashboard", "--no-percentiles-over-time"], false),
        ] {
            let configuration = GooseConfiguration::parse_args_default(&args).unwrap();
            let mut goose_attack = GooseAttack::initialize_with_config(configuration).unwrap();
            let snapshot = goose_attack.build_local_dashboard_snapshot(
                "idle".to_string(),
                false,
                0,
                metrics::dashboard_snapshot::SnapshotSettings::idle(),
                None,
            );
            assert_eq!(snapshot.flags.percentiles_over_time, expected, "{args:?}");
            // No histograms are kept before the first test starts.
            assert_eq!(snapshot.series.heatmap, None);
        }
    }

    /// A real run's final metrics are marked final, so the summary printed
    /// when it ends carries the percentile table.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_final_metrics_after_execute() {
        async fn get_index(user: &mut GooseUser) -> crate::goose::TransactionResult {
            let _goose = user.get("/").await?;
            Ok(())
        }
        let server = httpmock::MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/");
            then.status(200);
        });
        let configuration = GooseConfiguration::parse_args_default(&[
            "--no-telnet",
            "--no-websocket",
            "--host",
            &server.base_url(),
            "--users",
            "1",
            "--run-time",
            "2",
        ])
        .unwrap();
        let goose_metrics = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(scenario!("Index").register_transaction(transaction!(get_index)))
            .execute()
            .await
            .unwrap();
        assert!(goose_metrics.final_metrics);
        let printed = goose_metrics.to_string();
        assert!(
            printed.contains("Slowest page load within specified percentile of requests (in ms):"),
            "{}",
            printed
        );
    }

    /// The transaction indexes `allocate_transactions` schedules for
    /// `scenario`, ignoring on_start and on_stop.
    fn scheduled(scenario: &Scenario, scheduler: &GooseScheduler) -> Vec<usize> {
        let (_, transactions, _) = allocate_transactions(scenario, scheduler);
        transactions.into_iter().map(|(index, _)| index).collect()
    }

    async fn noop_transaction(_user: &mut GooseUser) -> crate::goose::TransactionResult {
        Ok(())
    }

    /// The Random scheduler runs the Serial list in a random order: the same
    /// transactions, each as often, not in Serial order.
    #[test]
    fn test_random_scheduler_shuffles_transactions() {
        // Weights of 10 each reduce by their greatest common divisor to one
        // each, 3 orders of 6 being Serial in each run; weights of 10, 11 and
        // 12 do not reduce, and one in more than 10^14 orders is Serial.
        for weights in [[10, 10, 10], [10, 11, 12]] {
            let scenario = weights
                .iter()
                .fold(scenario!("Random"), |scenario, weight| {
                    scenario.register_transaction(
                        transaction!(noop_transaction).set_weight(*weight).unwrap(),
                    )
                });
            let serial = scheduled(&scenario, &GooseScheduler::Serial);
            let mut differs = false;
            for _ in 0..20 {
                let random = scheduled(&scenario, &GooseScheduler::Random);
                let mut sorted = random.clone();
                sorted.sort_unstable();
                assert_eq!(sorted, serial);
                differs |= random != serial;
            }
            assert!(
                differs,
                "Random scheduled transactions weighted {:?} in Serial order: {:?}",
                weights, serial
            );
        }
    }

    /// The Random scheduler shuffles within each sequence, and sequences keep
    /// their order.
    #[test]
    fn test_random_scheduler_keeps_sequence_order() {
        let scenario = scenario!("Sequenced")
            .register_transaction(
                transaction!(noop_transaction)
                    .set_sequence(2)
                    .set_weight(5)
                    .unwrap(),
            )
            .register_transaction(
                transaction!(noop_transaction)
                    .set_sequence(1)
                    .set_weight(5)
                    .unwrap(),
            )
            .register_transaction(
                transaction!(noop_transaction)
                    .set_sequence(1)
                    .set_weight(6)
                    .unwrap(),
            )
            .register_transaction(
                transaction!(noop_transaction)
                    .set_sequence(2)
                    .set_weight(6)
                    .unwrap(),
            );
        let serial = scheduled(&scenario, &GooseScheduler::Serial);
        assert_eq!(
            serial,
            [vec![1; 5], vec![2; 6], vec![0; 5], vec![3; 6]].concat()
        );
        let mut differs = false;
        for _ in 0..10 {
            let random = scheduled(&scenario, &GooseScheduler::Random);
            // Sequence 1, transactions 1 and 2, comes first, then sequence 2.
            let (first, second) = random.split_at(11);
            let mut first_sorted = first.to_vec();
            first_sorted.sort_unstable();
            assert_eq!(first_sorted, [vec![1; 5], vec![2; 6]].concat());
            let mut second_sorted = second.to_vec();
            second_sorted.sort_unstable();
            assert_eq!(second_sorted, [vec![0; 5], vec![3; 6]].concat());
            differs |= random != serial;
        }
        assert!(differs, "Random scheduled each sequence in Serial order");
    }

    /// The Random scheduler leaves on_start and on_stop transactions in Serial
    /// order.
    #[test]
    fn test_random_scheduler_keeps_on_start_and_on_stop_serial() {
        let scenario = (0..3).fold(scenario!("StartStop"), |scenario, _| {
            scenario.register_transaction(
                transaction!(noop_transaction)
                    .set_on_start()
                    .set_on_stop()
                    .set_weight(10)
                    .unwrap(),
            )
        });
        let scenario =
            scenario.register_transaction(transaction!(noop_transaction).set_weight(11).unwrap());
        let (serial_start, _, serial_stop) =
            allocate_transactions(&scenario, &GooseScheduler::Serial);
        let indexes = |list: WeightedTransactions| -> Vec<usize> {
            list.into_iter().map(|(index, _)| index).collect()
        };
        let (serial_start, serial_stop) = (indexes(serial_start), indexes(serial_stop));
        assert_eq!(serial_start.len(), 30);
        for _ in 0..10 {
            let (start, _, stop) = allocate_transactions(&scenario, &GooseScheduler::Random);
            assert_eq!(indexes(start), serial_start);
            assert_eq!(indexes(stop), serial_stop);
        }
    }

    /// Users weighted after the shared client is built, as a Controller adds them
    /// to a running load test, reuse that client: it is built once.
    #[test]
    fn test_shared_client_is_shared_by_users_added_later() {
        let configuration =
            GooseConfiguration::parse_args_default(&["--host", "http://127.0.0.1", "--quiet"])
                .unwrap();
        let mut goose_attack = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(
                scenario!("Shared").register_transaction(transaction!(noop_transaction)),
            )
            .set_client_builder_without_cookies(client::GooseClientBuilder::new().without_cookies())
            .unwrap();
        goose_attack
            .configuration
            .configure(&goose_attack.defaults)
            .unwrap();

        let builds = || client::SHARED_CLIENT_BUILDS.with(|builds| builds.get());
        let before = builds();
        goose_attack
            .build_client_strategy(client::CommandLineClientOptions::default())
            .unwrap();
        let mut users = goose_attack.weight_scenario_users(2).unwrap();
        users.extend(goose_attack.weight_scenario_users(2).unwrap());
        assert_eq!(users.len(), 4);
        assert_eq!(
            builds() - before,
            1,
            "users weighted later were given a newly built shared client"
        );
    }

    /// A transaction that does nothing, for plans in tests.
    fn gaggle_noop() -> Transaction {
        Transaction::new(Arc::new(|_user| Box::pin(async { Ok(()) })))
    }

    /// A load test with three weighted scenarios and `scheduler`.
    fn gaggle_attack(scheduler: GooseScheduler) -> GooseAttack {
        let configuration =
            GooseConfiguration::parse_args_default(&["--host", "http://127.0.0.1"]).unwrap();
        let mut attack = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .set_scheduler(scheduler);
        for (name, weight) in [("One", 3), ("Two", 2), ("Three", 1)] {
            attack = attack.register_scenario(
                Scenario::new(name)
                    .set_weight(weight)
                    .unwrap()
                    .register_transaction(gaggle_noop()),
            );
        }
        attack
    }

    #[cfg(feature = "gaggle")]
    /// Splits of `total` users over `workers` workers: one each way the
    /// generator enumerates, uneven, some empty when there are more workers
    /// than users.
    fn gaggle_splits(total: usize, workers: usize, seed: u64) -> Vec<Vec<usize>> {
        let mut state = seed;
        let mut next = |modulus: usize| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as usize % modulus.max(1)
        };
        let mut splits = Vec::new();
        // Even, everything on the first worker, everything on the last.
        splits.push(
            (0..workers)
                .map(|w| (w + 1) * total / workers - w * total / workers)
                .collect(),
        );
        let mut first = vec![0; workers];
        first[0] = total;
        splits.push(first);
        let mut last = vec![0; workers];
        last[workers - 1] = total;
        splits.push(last);
        for _ in 0..12 {
            let mut cuts: Vec<usize> = (0..workers - 1).map(|_| next(total + 1)).collect();
            cuts.sort_unstable();
            let mut split = Vec::with_capacity(workers);
            let mut from = 0;
            for cut in cuts.into_iter().chain([total]) {
                split.push(cut - from);
                from = cut;
            }
            splits.push(split);
        }
        splits
    }

    /// The scenario of each user a single process starts, in the order it
    /// starts them, by the allocation before Gaggle: the weighted list
    /// repeated, built in order and popped from the end.
    fn gaggle_single_order(attack: &mut GooseAttack, total: usize) -> Vec<usize> {
        let weighted = attack.allocate_scenarios();
        let mut built: Vec<usize> = Vec::new();
        while built.len() < total {
            for &scenario in &weighted {
                built.push(scenario);
                if built.len() == total {
                    break;
                }
            }
        }
        let mut started = Vec::new();
        while let Some(scenario) = built.pop() {
            started.push(scenario);
        }
        started
    }

    /// Users are built in the order the allocation before Gaggle built
    /// them, the weighted scenarios repeated, for the schedulers that
    /// allocate the same way every time: what `user_scenario_order` gives
    /// when the process runs every user, as it always does without the
    /// `gaggle` feature.
    #[test]
    fn user_scenario_order_is_the_weighted_list_repeated() {
        for scheduler in [GooseScheduler::RoundRobin, GooseScheduler::Serial] {
            for total in [0, 1, 4, 10, 1_000] {
                let mut attack = gaggle_attack(scheduler.clone());
                let mut built = gaggle_single_order(&mut attack, total);
                built.reverse();
                assert_eq!(
                    attack.user_scenario_order(total).unwrap(),
                    built,
                    "{:?} {}",
                    scheduler,
                    total
                );
            }
        }
    }

    /// B5: workers running shares of the users from their global ordinals
    /// start, in turn, exactly the scenarios one process running them all
    /// would, for every split tried, with the deterministic schedulers
    /// working out their own share and the random one handed its slice.
    #[cfg(feature = "gaggle")]
    #[test]
    fn gaggle_ordinals_match_one_process() {
        for scheduler in [
            GooseScheduler::RoundRobin,
            GooseScheduler::Serial,
            GooseScheduler::Random,
        ] {
            for total in [1, 4, 10, 1_000] {
                let mut single = gaggle_attack(scheduler.clone());
                let expected = match scheduler {
                    GooseScheduler::Random => single.user_scenarios(total).unwrap(),
                    _ => gaggle_single_order(&mut single, total),
                };
                assert_eq!(expected.len(), total);
                if !matches!(scheduler, GooseScheduler::Random) {
                    assert_eq!(single.user_scenarios(total).unwrap(), expected);
                }
                for workers in 1..=7 {
                    for split in gaggle_splits(total, workers, (total * 31 + workers) as u64) {
                        let mut started = Vec::new();
                        let mut offset = 0;
                        for users in split {
                            let mut worker = gaggle_attack(scheduler.clone());
                            match scheduler {
                                GooseScheduler::Random => worker.set_user_slice(
                                    offset,
                                    expected[offset..offset + users].to_vec(),
                                ),
                                _ => worker.set_user_ordinals(offset, users, total).unwrap(),
                            }
                            assert_eq!(worker.user_offset, offset);
                            // The worker builds its users in this order and
                            // pops them, last first.
                            let mut built = worker.user_scenario_order(users).unwrap();
                            while let Some(scenario) = built.pop() {
                                started.push(scenario);
                            }
                            offset += users;
                        }
                        assert_eq!(
                            started, expected,
                            "{scheduler:?}, {total} users on {workers} workers"
                        );
                    }
                }
            }
        }
    }

    /// A worker whose share is not the number of users it is told to run
    /// fails to launch, rather than running some other allocation.
    #[cfg(feature = "gaggle")]
    #[tokio::test]
    async fn gaggle_worker_share_of_another_size_fails_to_launch() {
        let mut attack = gaggle_attack(GooseScheduler::RoundRobin);
        attack.set_user_slice(5, vec![0, 1]);
        assert!(matches!(
            attack.user_scenario_order(3),
            Err(GooseError::InvalidOption { .. })
        ));
        let configuration = GooseConfiguration::parse_args_default(&[
            "--host",
            "http://127.0.0.1:9",
            "--users",
            "3",
            "--run-time",
            "1",
            "--no-telnet",
            "--quiet",
        ])
        .unwrap();
        let mut attack = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(Scenario::new("One").register_transaction(gaggle_noop()));
        attack.set_user_slice(5, vec![0, 0]);
        let ended = tokio::time::timeout(Duration::from_secs(20), attack.execute())
            .await
            .expect("the run ends");
        assert!(
            matches!(ended, Err(GooseError::InvalidOption { ref option, .. }) if option == "--users"),
            "{:?}",
            ended.err()
        );
    }

    /// A worker's share that runs past the last user, or past the last
    /// ordinal there is, is an error, not a shorter share.
    #[cfg(feature = "gaggle")]
    #[test]
    fn gaggle_worker_share_past_the_end_is_an_error() {
        let mut attack = gaggle_attack(GooseScheduler::RoundRobin);
        for (offset, users) in [(8, 3), (usize::MAX, 2), (1, usize::MAX)] {
            assert!(
                matches!(
                    attack.set_user_ordinals(offset, users, 10),
                    Err(GooseError::InvalidOption { .. })
                ),
                "{} {}",
                offset,
                users
            );
        }
        attack.set_user_ordinals(7, 3, 10).unwrap();
        assert_eq!(attack.user_scenario_order(3).unwrap().len(), 3);
    }

    /// A worker's users start with global ordinals from its offset on: each
    /// scenario counts the ordinals of the users that ran it.
    #[cfg(feature = "gaggle")]
    #[tokio::test]
    async fn gaggle_worker_users_start_at_offset() {
        let configuration = GooseConfiguration::parse_args_default(&[
            "--host",
            "http://127.0.0.1:9",
            "--users",
            "3",
            "--increase-rate",
            "3",
            "--run-time",
            "1",
            "--no-telnet",
            "--quiet",
        ])
        .unwrap();
        let mut attack = GooseAttack::initialize_with_config(configuration).unwrap();
        for name in ["One", "Two", "Three"] {
            attack = attack.register_scenario(
                Scenario::new(name)
                    .set_wait_time(Duration::from_secs(1), Duration::from_secs(1))
                    .unwrap()
                    .register_transaction(gaggle_noop()),
            );
        }
        attack.set_user_slice(5, vec![0, 1, 2]);
        let metrics = tokio::time::timeout(Duration::from_secs(20), attack.execute())
            .await
            .expect("the run ends")
            .unwrap();
        for (scenario, ordinal) in [(0, 5), (1, 6), (2, 7)] {
            assert_eq!(
                metrics.scenarios[scenario].users,
                std::iter::once(ordinal).collect(),
                "scenario {scenario}"
            );
        }
    }

    /// `--gaggle-shadow-merge` runs inside `execute()`: the shadow merge is
    /// enabled, takes a delta each second and matches.
    #[cfg(feature = "gaggle")]
    #[tokio::test]
    async fn gaggle_shadow_merge_runs_in_execute() {
        let configuration = GooseConfiguration::parse_args_default(&[
            "--host",
            "http://127.0.0.1:9",
            "--users",
            "2",
            "--increase-rate",
            "2",
            "--run-time",
            "3",
            "--no-telnet",
            "--quiet",
            "--gaggle-shadow-merge",
        ])
        .unwrap();
        let get = Transaction::new(Arc::new(|user| {
            Box::pin(async move {
                let _ = user.get("/").await;
                Ok(())
            })
        }));
        let attack = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(Scenario::new("One").register_transaction(get));
        tokio::time::timeout(Duration::from_secs(20), attack.execute())
            .await
            .expect("the run ends")
            .expect("the shadow merge matches");
        let outcome = GAGGLE_SHADOW_LAST
            .lock()
            .unwrap()
            .take()
            .expect("execute ran the shadow merge");
        assert!(outcome.differences.is_empty(), "{:?}", outcome.differences);
        assert!(
            outcome.deltas >= 4,
            "a delta each second of a 3 s run, then the last and final: {}",
            outcome.deltas
        );
    }

    /// A fixed plan for the hash tests, each part of it set.
    fn gaggle_plan() -> (Vec<Scenario>, GooseScheduler) {
        let one = Scenario::new("One")
            .set_weight(3)
            .unwrap()
            .set_wait_time(Duration::from_millis(100), Duration::from_millis(250))
            .unwrap()
            .set_host("http://one.example")
            .register_transaction(gaggle_noop().set_name("start").set_on_start())
            .register_transaction(gaggle_noop().set_name("index").set_weight(4).unwrap())
            .register_transaction(gaggle_noop().set_name("about").set_sequence(2))
            .register_transaction(gaggle_noop().set_name("stop").set_on_stop());
        let two = Scenario::new("Two")
            .register_transaction(gaggle_noop().set_name_for_transaction_and_requests("all"));
        (vec![one, two], GooseScheduler::RoundRobin)
    }

    /// B6: the plan hash of a fixed plan never drifts, equal plans built
    /// twice hash equal, and changing any one part changes it.
    #[test]
    fn gaggle_plan_hash_is_stable() {
        let (scenarios, scheduler) = gaggle_plan();
        let hash = plan_hash(&scenarios, &scheduler);
        assert_eq!(hash, GAGGLE_PLAN_HASH, "the golden plan hash: {hash:#x}");
        let (again, scheduler_again) = gaggle_plan();
        assert_eq!(plan_hash(&again, &scheduler_again), hash);

        type Change = fn(&mut Vec<Scenario>, &mut GooseScheduler);
        let changes: [(&str, Change); 14] = [
            ("scenario name", |s, _| s[1].name = Arc::from("Twos")),
            ("scenario weight", |s, _| s[0].weight = 2),
            ("wait time", |s, _| {
                s[0].transaction_wait =
                    Some((Duration::from_millis(100), Duration::from_millis(251)))
            }),
            ("no wait time", |s, _| s[0].transaction_wait = None),
            ("host", |s, _| {
                s[0].host = Some("http://two.example".to_string())
            }),
            ("no host", |s, _| s[0].host = None),
            ("transaction name", |s, _| {
                s[0].transactions[1].name = TransactionName::TransactionOnly(Arc::from("home"))
            }),
            ("transaction name kind", |s, _| {
                s[1].transactions[0].name = TransactionName::TransactionOnly(Arc::from("all"))
            }),
            ("transaction weight", |s, _| s[0].transactions[1].weight = 5),
            ("sequence", |s, _| s[0].transactions[2].sequence = 3),
            ("on start", |s, _| s[0].transactions[0].on_start = false),
            ("on stop", |s, _| s[0].transactions[3].on_stop = false),
            ("order", |s, _| s.swap(0, 1)),
            ("scheduler", |_, scheduler| {
                *scheduler = GooseScheduler::Serial
            }),
        ];
        for (what, change) in changes {
            let (mut scenarios, mut scheduler) = gaggle_plan();
            change(&mut scenarios, &mut scheduler);
            assert_ne!(plan_hash(&scenarios, &scheduler), hash, "{what}");
        }
    }

    /// The golden value of [`gaggle_plan`]'s hash: changing it changes every
    /// plan's hash, which the CHANGELOG must say.
    const GAGGLE_PLAN_HASH: u64 = 0xda7d_f883_b1f9_971b;
}
