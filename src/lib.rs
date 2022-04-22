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
//!   - [Gaggle: a distributed load test](https://www.tag1consulting.com/blog/show-me-how-flock-flies-working-gaggle-goose)
//!   - [Optimizing Goose performance](https://www.tag1consulting.com/blog/golden-goose-egg-compile-time-adventure)
//!
//! ## License
//!
//! Copyright 2020-21 Jeremy Andrews
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

pub mod config;
pub mod controller;
pub mod goose;
mod graph;
pub mod logger;
#[cfg(feature = "gaggle")]
mod manager;
pub mod metrics;
pub mod prelude;
mod report;
mod test_plan;
mod throttle;
mod user;
pub mod util;
#[cfg(feature = "gaggle")]
mod worker;

use gumdrop::Options;
use lazy_static::lazy_static;
#[cfg(feature = "gaggle")]
use nng::Socket;
use rand::seq::SliceRandom;
use rand::thread_rng;
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::time::{self, Duration};
use std::{fmt, io};
use tokio::fs::File;

use crate::config::{GooseConfiguration, GooseDefaults};
use crate::controller::{GooseControllerProtocol, GooseControllerRequest};
use crate::goose::{GaggleUser, GooseUser, GooseUserCommand, Scenario, Transaction};
use crate::graph::GraphData;
use crate::logger::{GooseLoggerJoinHandle, GooseLoggerTx};
use crate::metrics::{GooseMetric, GooseMetrics};
use crate::test_plan::{TestPlan, TestPlanHistory, TestPlanStepAction};
#[cfg(feature = "gaggle")]
use crate::worker::{register_shutdown_pipe_handler, GaggleMetrics};

/// Constant defining Goose's default telnet Controller port.
const DEFAULT_TELNET_PORT: &str = "5116";

/// Constant defining Goose's default WebSocket Controller port.
const DEFAULT_WEBSOCKET_PORT: &str = "5117";

// WORKER_ID is only used when running a gaggle (a distributed load test).
lazy_static! {
    static ref WORKER_ID: AtomicUsize = AtomicUsize::new(0);
}

/// Internal representation of a weighted transaction list.
type WeightedTransactions = Vec<(usize, String)>;

/// Internal representation of unsequenced transactions.
type UnsequencedTransactions = Vec<Transaction>;
/// Internal representation of sequenced transactions.
type SequencedTransactions = BTreeMap<usize, Vec<Transaction>>;

/// Returns the unique identifier of the running Worker when running in Gaggle mode.
///
/// The first Worker to connect to the Manager is assigned an ID of 1. For each
/// subsequent Worker to connect to the Manager the ID is incremented by 1. This
/// identifier is primarily an aid in tracing logs.
pub fn get_worker_id() -> usize {
    WORKER_ID.load(Ordering::Relaxed)
}

#[cfg(not(feature = "gaggle"))]
#[derive(Debug, Clone)]
/// Socket used for coordinating a Gaggle distributed load test.
pub(crate) struct Socket {}

/// An enumeration of all errors a [`GooseAttack`](./struct.GooseAttack.html) can return.
#[derive(Debug)]
pub enum GooseError {
    /// Wraps a [`std::io::Error`](https://doc.rust-lang.org/std/io/struct.Error.html).
    Io(io::Error),
    /// Wraps a [`reqwest::Error`](https://docs.rs/reqwest/*/reqwest/struct.Error.html).
    Reqwest(reqwest::Error),
    /// Wraps a ['tokio::task::JoinError'](https://tokio-rs.github.io/tokio/doc/tokio/task/struct.JoinError.html).
    TokioJoin(tokio::task::JoinError),
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
    /// [`GooseAttack`](./struct.GooseAttack.html) has no [`Scenario`](./goose/struct.Scenario.html) defined.
    NoScenarios {
        /// An optional explanation of the error.
        detail: String,
    },
}
/// Implement a helper to provide a text description of all possible types of errors.
impl GooseError {
    fn describe(&self) -> &str {
        match *self {
            GooseError::Io(_) => "io::Error",
            GooseError::Reqwest(_) => "reqwest::Error",
            GooseError::TokioJoin(_) => "tokio::task::JoinError",
            GooseError::FeatureNotEnabled { .. } => "required compile-time feature not enabled",
            GooseError::InvalidHost { .. } => "failed to parse hostname",
            GooseError::InvalidOption { .. } => "invalid option or value specified",
            GooseError::InvalidWaitTime { .. } => "invalid wait_time specified",
            GooseError::InvalidWeight { .. } => "invalid weight specified",
            GooseError::NoScenarios { .. } => "no scenarios defined",
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

#[derive(Clone, Debug, PartialEq)]
/// A [`GooseAttack`](./struct.GooseAttack.html) load test operates in one (and only one)
/// of the following modes.
pub enum AttackMode {
    /// During early startup before one of the following modes gets assigned.
    Undefined,
    /// A single standalone process performing a load test.
    StandAlone,
    /// The controlling process in a Gaggle distributed load test.
    Manager,
    /// One of one or more working processes in a Gaggle distributed load test.
    Worker,
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
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
    /// A timestamp tracking when the previous [`GooseUser`](./goose/struct.GooseUser.html)
    /// was launched.
    spawn_user_timer: std::time::Instant,
    /// How many milliseconds until the next [`GooseUser`](./goose/struct.GooseUser.html)
    /// should be spawned.
    spawn_user_in_ms: usize,
    /// A counter tracking which [`GooseUser`](./goose/struct.GooseUser.html) is being
    /// spawned.
    spawn_user_counter: usize,
    /// This variable accounts for time spent doing things which is then subtracted from
    /// the time sleeping to avoid an unintentional drift in events that are supposed to
    /// happen regularly.
    drift_timer: tokio::time::Instant,
    /// Unbounded sender used by all [`GooseUser`](./goose/struct.GooseUser.html)
    /// threads to send metrics to parent.
    all_threads_metrics_tx: flume::Sender<GooseMetric>,
    /// Unbounded receiver used by Goose parent to receive metrics from
    /// [`GooseUser`](./goose/struct.GooseUser.html)s.
    metrics_rx: flume::Receiver<GooseMetric>,
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
    /// Optional channel allowing controller thread to make requests, if not disabled.
    controller_channel_rx: Option<flume::Receiver<GooseControllerRequest>>,
    /// Optional unbuffered writer for html-formatted report file, if enabled.
    report_file: Option<File>,
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
    /// Timer tracking when to display running metrics, if enabled.
    running_metrics_timer: std::time::Instant,
    /// Boolean flag indicating if running metrics should be displayed.
    display_running_metrics: bool,
    /// Boolean flag indicating if all [`GooseUser`](./goose/struct.GooseUser.html)s
    /// have been spawned.
    all_users_spawned: bool,
    /// Boolean flag indicating of Goose should shutdown after stopping a running load test.
    shutdown_after_stop: bool,
    /// Thread-safe boolean flag indicating if the [`GooseAttack`](./struct.GooseAttack.html)
    /// has been canceled.
    canceled: Arc<AtomicBool>,
    /// Optional socket used to coordinate a distributed Gaggle.
    socket: Option<Socket>,
}

/// Global internal state for the load test.
pub struct GooseAttack {
    /// An optional transaction that is run one time before starting GooseUsers and running Scenarios.
    test_start_transaction: Option<Transaction>,
    /// An optional transaction that is run one time after all GooseUsers have finished.
    test_stop_transaction: Option<Transaction>,
    /// A vector containing one copy of each Scenario defined by this load test.
    scenarios: Vec<Scenario>,
    /// A weighted vector containing a GooseUser object for each GooseUser that will run during this load test.
    weighted_users: Vec<GooseUser>,
    /// A weighted vector containing a lightweight GaggleUser object that is sent to all Workers if running in Gaggle mode.
    weighted_gaggle_users: Vec<GaggleUser>,
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
    started: Option<time::Instant>,
    /// Internal Goose test plan representation.
    test_plan: TestPlan,
    /// When the current test plan step started.
    step_started: Option<time::Instant>,
    /// All metrics merged together.
    metrics: GooseMetrics,
    /// All data for report graphs.
    graph_data: GraphData,
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
            weighted_users: Vec::new(),
            weighted_gaggle_users: Vec::new(),
            defaults: GooseDefaults::default(),
            configuration,
            attack_mode: AttackMode::Undefined,
            attack_phase: AttackPhase::Idle,
            scheduler: GooseScheduler::RoundRobin,
            started: None,
            test_plan: TestPlan::new(),
            step_started: None,
            metrics: GooseMetrics::default(),
            graph_data: GraphData::new(),
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
    /// use gumdrop::Options;
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
            weighted_users: Vec::new(),
            weighted_gaggle_users: Vec::new(),
            defaults: GooseDefaults::default(),
            configuration,
            attack_mode: AttackMode::Undefined,
            attack_phase: AttackPhase::Idle,
            scheduler: GooseScheduler::RoundRobin,
            started: None,
            test_plan: TestPlan::new(),
            step_started: None,
            metrics: GooseMetrics::default(),
            graph_data: GraphData::new(),
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
    /// When running in a distributed Gaggle, this transaction is only run one time by the
    /// Manager.
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
    /// When running in a distributed Gaggle, this transaction is only run one time by the
    /// Manager.
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

    /// Use configured GooseScheduler to build out a properly weighted list of
    /// [`Scenario`](./goose/struct.Scenario.html)s to be assigned to
    /// [`GooseUser`](./goose/struct.GooseUser.html)s
    fn allocate_scenarios(&mut self) -> Vec<usize> {
        trace!("allocate_scenarios");

        let mut u: usize = 0;
        let mut v: usize;
        for scenario in &self.scenarios {
            if u == 0 {
                u = scenario.weight;
            } else {
                v = scenario.weight;
                trace!("calculating greatest common denominator of {} and {}", u, v);
                u = util::gcd(u, v);
                trace!("inner gcd: {}", u);
            }
        }
        // 'u' will always be the greatest common divisor
        debug!("gcd: {}", u);

        // Build a vector of vectors to be used to schedule users.
        let mut available_scenarios = Vec::with_capacity(self.scenarios.len());
        let mut total_scenarios = 0;
        for (index, scenario) in self.scenarios.iter().enumerate() {
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

        info!(
            "allocating transactions and scenarios with {:?} scheduler",
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
                            debug!("allocating 1 user from Scenario {}", scenario_index);
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
                    let scenario = available_scenarios.choose_mut(&mut rand::thread_rng());
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

    /// Pre-allocate a vector of weighted [`GooseUser`](./goose/struct.GooseUser.html)s.
    fn weight_scenario_users(&mut self) -> Result<Vec<GooseUser>, GooseError> {
        trace!("weight_scenario_users");

        let weighted_scenarios = self.allocate_scenarios();

        // Allocate a state for each user that will be hatched.
        info!("initializing {} user states...", self.test_plan.max_users());
        let mut weighted_users = Vec::new();
        let mut user_count = 0;
        loop {
            for scenarios_index in &weighted_scenarios {
                debug!(
                    "creating user state: {} ({})",
                    weighted_users.len(),
                    scenarios_index
                );
                let base_url = goose::get_base_url(
                    self.get_configuration_host(),
                    self.scenarios[*scenarios_index].host.clone(),
                    self.defaults.host.clone(),
                )?;
                weighted_users.push(GooseUser::new(
                    self.scenarios[*scenarios_index].scenarios_index,
                    base_url,
                    &self.configuration,
                    self.metrics.hash,
                )?);
                user_count += 1;
                if user_count == self.test_plan.max_users() {
                    debug!("created {} weighted_users", user_count);
                    return Ok(weighted_users);
                }
            }
        }
    }

    /// Allocate a vector of weighted [`GaggleUser`](./goose/struct.GaggleUser.html).
    fn prepare_worker_scenario_users(&mut self) -> Result<Vec<GaggleUser>, GooseError> {
        trace!("prepare_worker_scenario_users");

        let weighted_scenarios = self.allocate_scenarios();

        // Determine the users sent to each Worker.
        info!("preparing users for Workers...");
        let mut weighted_users = Vec::new();
        let mut user_count = 0;
        loop {
            for scenarios_index in &weighted_scenarios {
                let base_url = goose::get_base_url(
                    self.get_configuration_host(),
                    self.scenarios[*scenarios_index].host.clone(),
                    self.defaults.host.clone(),
                )?;
                weighted_users.push(GaggleUser::new(
                    self.scenarios[*scenarios_index].scenarios_index,
                    base_url,
                    &self.configuration,
                    self.metrics.hash,
                ));
                user_count += 1;
                if user_count == self.test_plan.max_users() {
                    debug!("prepared {} weighted_gaggle_users", user_count);
                    return Ok(weighted_users);
                }
            }
        }
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

        // Optional debug output.
        info!("entering GooseAttack phase: {:?}", &phase);

        // Update the current phase.
        self.attack_phase = phase;
    }

    // If enabled, returns the path of the report_file, otherwise returns None.
    fn get_report_file_path(&mut self) -> Option<String> {
        // Return if enabled.
        if !self.configuration.report_file.is_empty() {
            Some(self.configuration.report_file.to_string())
        // Otherwise there is no report file.
        } else {
            None
        }
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
    pub async fn execute(mut self) -> Result<GooseMetrics, GooseError> {
        // If version flag is set, display package name and version and exit.
        if self.configuration.version {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
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
                        transaction.name, transaction.weight
                    );
                }
            }
            std::process::exit(0);
        }

        // Configure GooseConfiguration.
        self.configuration.configure(&self.defaults);

        // Validate GooseConfiguration.
        self.configuration.validate()?;

        // Build TestPlan.
        self.test_plan = TestPlan::build(&self.configuration);

        // With a validated GooseConfiguration, enter a run mode.
        self.attack_mode = if self.configuration.manager {
            AttackMode::Manager
        } else if self.configuration.worker {
            AttackMode::Worker
        } else {
            AttackMode::StandAlone
        };

        // Confirm there's either a global host, or each scenario has a host defined.
        if let Err(e) = self.validate_host() {
            if self.configuration.no_autostart {
                info!("host must be configured via Controller before starting load test");
            } else {
                // If auto-starting, host must be valid.
                return Err(e);
            }
        } else {
            info!("global host configured: {}", self.configuration.host);
            self.prepare_load_test()?;
        }

        // Calculate a unique hash for the current load test.
        let mut s = DefaultHasher::new();
        self.scenarios.hash(&mut s);
        self.metrics.hash = s.finish();
        debug!("hash: {}", self.metrics.hash);

        // Start goose in manager mode.
        if self.attack_mode == AttackMode::Manager {
            #[cfg(feature = "gaggle")]
            {
                self = manager::manager_main(self).await;
            }

            #[cfg(not(feature = "gaggle"))]
            {
                return Err(GooseError::FeatureNotEnabled {
                    feature: "gaggle".to_string(), detail: "Load test must be recompiled with `--features gaggle` to start in manager mode.".to_string()
                });
            }
        }
        // Start goose in worker mode.
        else if self.attack_mode == AttackMode::Worker {
            #[cfg(feature = "gaggle")]
            {
                self = worker::worker_main(self).await;
            }

            #[cfg(not(feature = "gaggle"))]
            {
                return Err(GooseError::FeatureNotEnabled {
                    feature: "gaggle".to_string(),
                    detail: "Load test must be recompiled with `--features gaggle` to start in worker mode.".to_string(),
                });
            }
        }
        // Start goose in single-process mode.
        else {
            self = self.start_attack(None).await?;
        }

        if self.metrics.display_metrics {
            info!(
                "printing final metrics after {} seconds...",
                self.metrics.duration
            );
            print!("{}", self.metrics);
        }

        Ok(self.metrics)
    }

    // Returns OK(()) if there's a valid host, GooseError with details if not.
    fn validate_host(&mut self) -> Result<(), GooseError> {
        if self.configuration.host.is_empty() {
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
                            if self.attack_mode != AttackMode::Worker {
                                return Err(GooseError::InvalidOption {
                                    option: "--host".to_string(),
                                    value: "".to_string(),
                                    detail: format!("A host must be defined via the --host option, the GooseAttack.set_default() function, or the Scenario.set_host() function (no host defined for {}).", scenario.name)
                                });
                            }
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
        // If not on a Worker, be sure a valid host has been defined before building configuration.
        if self.attack_mode != AttackMode::Worker {
            self.validate_host()?;
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

        if self.attack_mode != AttackMode::Worker {
            // Stand-alone and Manager processes can display metrics.
            if !self.configuration.no_metrics && !self.configuration.no_print_metrics {
                self.metrics.display_metrics = true;
            }

            if self.attack_mode == AttackMode::StandAlone {
                // Allocate a state for each of the users we are about to start.
                self.weighted_users = self.weight_scenario_users()?;
            } else if self.attack_mode == AttackMode::Manager {
                // Build a list of users to be allocated on Workers.
                self.weighted_gaggle_users = self.prepare_worker_scenario_users()?;
            }
        }

        Ok(())
    }

    /// Helper to wrap configured host in `Option<>` if set.
    fn get_configuration_host(&self) -> Option<String> {
        if self.configuration.host.is_empty() {
            None
        } else {
            Some(self.configuration.host.to_string())
        }
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
        if self.configuration.throttle_requests == 0 {
            return (None, None);
        }

        // Create a bounded channel allowing single-sender multi-receiver to throttle
        // [`GooseUser`](./goose/struct.GooseUser.html) threads.
        let (all_threads_throttle, throttle_receiver): (
            flume::Sender<bool>,
            flume::Receiver<bool>,
        ) = flume::bounded(self.configuration.throttle_requests);

        // Create a channel allowing the parent to inform the throttle thread when the
        // load test is finished. Even though we only send one message, we can't use a
        // oneshot channel as we don't want to block waiting for a message.
        let (parent_to_throttle_tx, throttle_rx) = flume::bounded(1);

        // Launch a new thread for throttling, no need to rejoin it.
        let _ = Some(tokio::spawn(throttle::throttle_main(
            self.configuration.throttle_requests,
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
        for _ in 1..self.configuration.throttle_requests {
            let _ = sender.send_async(true).await;
        }

        (Some(all_threads_throttle), Some(parent_to_throttle_tx))
    }

    // Helper to optionally spawn a telnet and/or WebSocket Controller thread. The Controller
    // threads share a control channel, allowing it to send requests to the parent process. When
    // a response is required, the Controller will also send a one-shot channel allowing a direct
    // reply.
    async fn setup_controllers(&mut self) -> Option<flume::Receiver<GooseControllerRequest>> {
        // If the telnet controller is disabled, return immediately.
        if self.configuration.no_telnet && self.configuration.no_websocket {
            return None;
        }

        // Create an unbounded channel for controller threads to send requests to the parent
        // process.
        let (all_threads_controller_request_tx, controller_request_rx): (
            flume::Sender<GooseControllerRequest>,
            flume::Receiver<GooseControllerRequest>,
        ) = flume::unbounded();

        // Configured telnet Controller if not disabled.
        if !self.configuration.no_telnet {
            // Configure telnet_host, using default if run-time option is not set.
            if self.configuration.telnet_host.is_empty() {
                self.configuration.telnet_host =
                    if let Some(host) = self.defaults.telnet_host.clone() {
                        host
                    } else {
                        "0.0.0.0".to_string()
                    }
            }

            // Then configure telnet_port, using default if run-time option is not set.
            if self.configuration.telnet_port == 0 {
                self.configuration.telnet_port = if let Some(port) = self.defaults.telnet_port {
                    port
                } else {
                    DEFAULT_TELNET_PORT.to_string().parse().unwrap()
                };
            }

            // Spawn the initial controller thread to allow real-time control of the load test.
            // There is no need to rejoin this thread when the load test ends.
            let _ = Some(tokio::spawn(controller::controller_main(
                self.configuration.clone(),
                all_threads_controller_request_tx.clone(),
                GooseControllerProtocol::Telnet,
            )));
        }

        // Configured WebSocket Controller if not disabled.
        if !self.configuration.no_websocket {
            // Configure websocket_host, using default if run-time option is not set.
            if self.configuration.websocket_host.is_empty() {
                self.configuration.websocket_host =
                    if let Some(host) = self.defaults.websocket_host.clone() {
                        host
                    } else {
                        "0.0.0.0".to_string()
                    }
            }

            // Then configure websocket_port, using default if run-time option is not set.
            if self.configuration.websocket_port == 0 {
                self.configuration.websocket_port = if let Some(port) = self.defaults.websocket_port
                {
                    port
                } else {
                    DEFAULT_WEBSOCKET_PORT.to_string().parse().unwrap()
                };
            }

            // Spawn the initial controller thread to allow real-time control of the load test.
            // There is no need to rejoin this thread when the load test ends.
            let _ = Some(tokio::spawn(controller::controller_main(
                self.configuration.clone(),
                all_threads_controller_request_tx,
                GooseControllerProtocol::WebSocket,
            )));
        }

        // Return the parent end of the Controller channel.
        Some(controller_request_rx)
    }

    // Prepare an asynchronous file writer for `report_file` (if enabled).
    async fn prepare_report_file(&mut self) -> Result<Option<File>, GooseError> {
        if let Some(report_file_path) = self.get_report_file_path() {
            Ok(Some(File::create(&report_file_path).await?))
        } else {
            Ok(None)
        }
    }

    // Invoke `test_start` transactions if existing.
    async fn run_test_start(&self) -> Result<(), GooseError> {
        // Initialize per-user states.
        if self.attack_mode != AttackMode::Worker {
            // First run global test_start_transaction, if defined.
            match &self.test_start_transaction {
                Some(t) => {
                    info!("running test_start_transaction");
                    // Create a one-time-use User to run the test_start_transaction.
                    let base_url = goose::get_base_url(
                        self.get_configuration_host(),
                        None,
                        self.defaults.host.clone(),
                    )?;
                    let mut user = GooseUser::single(base_url, &self.configuration)?;
                    let function = &t.function;
                    let _ = function(&mut user).await;
                }
                // No test_start_transaction defined, nothing to do.
                None => (),
            }
        }

        Ok(())
    }

    // Invoke `test_stop` transactions if existing.
    async fn run_test_stop(&self) -> Result<(), GooseError> {
        // Initialize per-user states.
        if self.attack_mode != AttackMode::Worker {
            // First run global test_stop_transaction, if defined.
            match &self.test_stop_transaction {
                Some(t) => {
                    info!("running test_stop_transaction");
                    // Create a one-time-use User to run the test_stop_transaction.
                    let base_url = goose::get_base_url(
                        self.get_configuration_host(),
                        None,
                        self.defaults.host.clone(),
                    )?;
                    let mut user = GooseUser::single(base_url, &self.configuration)?;
                    let function = &t.function;
                    let _ = function(&mut user).await;
                }
                // No test_stop_transaction defined, nothing to do.
                None => (),
            }
        }

        Ok(())
    }

    // Create a GooseAttackRunState object and do all initialization required
    // to start a [`GooseAttack`](./struct.GooseAttack.html).
    async fn initialize_attack(
        &mut self,
        socket: Option<Socket>,
    ) -> Result<GooseAttackRunState, GooseError> {
        trace!("initialize_attack");

        // Create a single channel used to send metrics from GooseUser threads
        // to parent thread.
        let (all_threads_metrics_tx, metrics_rx): (
            flume::Sender<GooseMetric>,
            flume::Receiver<GooseMetric>,
        ) = flume::unbounded();

        // Optionally spawn a telnet and/or Websocket Controller thread.
        let controller_channel_rx = self.setup_controllers().await;

        // Grab now() once from the standard library, used by multiple timers in
        // the run state.
        let std_now = std::time::Instant::now();

        let goose_attack_run_state = GooseAttackRunState {
            spawn_user_timer: std_now,
            spawn_user_in_ms: 0,
            spawn_user_counter: 0,
            drift_timer: tokio::time::Instant::now(),
            all_threads_metrics_tx,
            metrics_rx,
            logger_handle: None,
            all_threads_logger_tx: None,
            throttle_threads_tx: None,
            parent_to_throttle_tx: None,
            controller_channel_rx,
            report_file: None,
            metrics_header_displayed: false,
            idle_status_displayed: false,
            users: Vec::new(),
            user_channels: Vec::new(),
            running_metrics_timer: std_now,
            display_running_metrics: false,
            all_users_spawned: false,
            shutdown_after_stop: !self.configuration.no_autostart,
            canceled: Arc::new(AtomicBool::new(false)),
            socket,
        };

        // Access socket to avoid errors.
        trace!("socket: {:?}", &goose_attack_run_state.socket);

        // Catch ctrl-c to allow clean shutdown to display metrics.
        util::setup_ctrlc_handler(&goose_attack_run_state.canceled);

        Ok(goose_attack_run_state)
    }

    // Increase the number of active [`GooseUser`](./goose/struct.GooseUser.html) threads in the
    // active [`GooseAttack`](./struct.GooseAttack.html).
    async fn increase_attack(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        // If this is the first load plan step, then there were no previously started users.
        let previous_users = if self.test_plan.current == 0 {
            0
        // Otherwise retreive the number of users configured in the previous step.
        } else {
            self.test_plan.steps[self.test_plan.current - 1].0
        };

        // Sanity check: increase_attack can only be called if the number of users is increasing
        // in the current step.
        debug_assert!(self.test_plan.steps[self.test_plan.current].0 > previous_users);

        // Divide the number of new users to launch (minus one, as the first launches immediately without any delay)
        // by the time configured to launch them.
        let hatch_rate: f32 = (self.test_plan.steps[self.test_plan.current].0 - previous_users - 1)
            as f32
            / self.test_plan.steps[self.test_plan.current].1 as f32
            // Convert from milliseconds to seconds.
            * 1_000.0;

        // Determine if it's time to spawn a GooseUser.
        if goose_attack_run_state.spawn_user_in_ms == 0
            || util::ms_timer_expired(
                goose_attack_run_state.spawn_user_timer,
                goose_attack_run_state.spawn_user_in_ms,
            )
        {
            if let Some(mut thread_user) = self.weighted_users.pop() {
                // Reset the spawn timer.
                goose_attack_run_state.spawn_user_timer = std::time::Instant::now();

                // To determine how long before we spawn the next GooseUser, start with 1,000.0
                // milliseconds and divide by the hatch_rate.
                goose_attack_run_state.spawn_user_in_ms = (1_000.0 / hatch_rate) as usize;

                // If running on a Worker, multiple by the number of workers as each is spawning
                // GooseUsers at this rate.
                if self.attack_mode == AttackMode::Worker {
                    goose_attack_run_state.spawn_user_in_ms *=
                        self.configuration.expect_workers.unwrap() as usize;
                };
                goose_attack_run_state.spawn_user_counter += 1;

                // Remember which scenario this user is using.
                thread_user.weighted_users_index = self.metrics.users;

                // Create a per-thread channel allowing parent thread to control child threads.
                let (parent_sender, thread_receiver): (
                    flume::Sender<GooseUserCommand>,
                    flume::Receiver<GooseUserCommand>,
                ) = flume::unbounded();
                goose_attack_run_state.user_channels.push(parent_sender);

                // Clone the logger_tx if enabled, otherwise is None.
                thread_user.logger = goose_attack_run_state.all_threads_logger_tx.clone();

                // Copy the GooseUser-throttle receiver channel, used by all threads.
                thread_user.throttle = if self.configuration.throttle_requests > 0 {
                    Some(goose_attack_run_state.throttle_threads_tx.clone().unwrap())
                } else {
                    None
                };

                // Copy the GooseUser-to-parent sender channel, used by all threads.
                thread_user.channel_to_parent =
                    Some(goose_attack_run_state.all_threads_metrics_tx.clone());

                // Copy the appropriate scenario into the thread.
                let thread_scenario = self.scenarios[thread_user.scenarios_index].clone();

                // We number threads from 1 as they're human-visible (in the logs),
                // whereas metrics.users starts at 0.
                let thread_number = self.metrics.users + 1;

                let is_worker = self.attack_mode == AttackMode::Worker;

                // If running on Worker, use Worker configuration in GooseUser.
                if is_worker {
                    thread_user.config = self.configuration.clone();
                }

                // Launch a new user.
                let user = tokio::spawn(user::user_main(
                    thread_number,
                    thread_scenario,
                    thread_user,
                    thread_receiver,
                    is_worker,
                ));

                goose_attack_run_state.users.push(user);
                self.metrics.users += 1;

                if let Some(running_metrics) = self.configuration.running_metrics {
                    if self.attack_mode != AttackMode::Worker
                        && util::ms_timer_expired(
                            goose_attack_run_state.running_metrics_timer,
                            running_metrics,
                        )
                    {
                        goose_attack_run_state.running_metrics_timer = time::Instant::now();
                        self.metrics.print_running();
                    }
                }
            }
        } else {
            // Wake up twice a second to handle messages and allow for a quick shutdown if the
            // load test is canceled during startup.
            let sleep_duration = if goose_attack_run_state.spawn_user_in_ms > 500 {
                Duration::from_millis(500)
            } else {
                Duration::from_millis(goose_attack_run_state.spawn_user_in_ms as u64)
            };
            debug!("sleeping {:?}...", sleep_duration);
            goose_attack_run_state.drift_timer =
                util::sleep_minus_drift(sleep_duration, goose_attack_run_state.drift_timer).await;
        }

        // Determine if enough users have been launched.
        let all_users_launched = match self.attack_mode {
            AttackMode::Worker | AttackMode::Manager => {
                // If running in Gaggle mode, all configured users must be launched.
                self.weighted_users.is_empty()
            }
            _ => {
                // If not running in Gaggle mode, all users for current step must be launched.
                self.metrics.users >= self.test_plan.steps[self.test_plan.current].0
            }
        };

        if all_users_launched {
            // Pause a tenth of a second waiting for the final user to fully start up.
            tokio::time::sleep(Duration::from_millis(100)).await;

            if self.attack_mode == AttackMode::Worker {
                info!(
                    "[{}] launched {} users...",
                    get_worker_id(),
                    self.test_plan.steps[self.test_plan.current].0
                );
            } else {
                info!(
                    "launched {} users...",
                    self.test_plan.steps[self.test_plan.current].0
                );
            }

            // Automatically reset metrics if appropriate.
            self.reset_metrics(goose_attack_run_state).await?;

            // Advance to the next TestPlan step.
            self.advance_test_plan(goose_attack_run_state);
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
        // @TODO: Reduce and recycle GooseUser threads: users ramp up and down multiple times.

        if self.attack_mode == AttackMode::Worker {
            info!(
                "[{}] stopping after {} seconds...",
                get_worker_id(),
                self.metrics.duration
            );

            // Load test is shutting down, update pipe handler so there is no panic
            // when the Manager goes away.
            #[cfg(feature = "gaggle")]
            {
                let manager = goose_attack_run_state.socket.clone().unwrap();
                register_shutdown_pipe_handler(&manager);
            }
        } else {
            info!("stopping after {} seconds...", self.metrics.duration);
        }
        for (index, send_to_user) in goose_attack_run_state.user_channels.iter().enumerate() {
            match send_to_user.send(GooseUserCommand::Exit) {
                Ok(_) => {
                    debug!("telling user {} to exit", index);
                }
                Err(e) => {
                    info!("failed to tell user {} to exit: {}", index, e);
                }
            }
        }
        if self.attack_mode == AttackMode::Worker {
            info!("[{}] waiting for users to exit", get_worker_id());
        } else {
            info!("waiting for users to exit");
        }

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
                warn!("unexpected error telling logger thread to exit: {}", e);
            };
            // Take logger out of the GooseAttackRunState object so it can be
            // consumed by tokio::join!().
            let logger = std::mem::take(&mut goose_attack_run_state.logger_handle);
            let _ = tokio::join!(logger.unwrap());
        }

        // If we're printing metrics, collect the final metrics received from users.
        if !self.configuration.no_metrics {
            // Set the second parameter to true, ensuring that Goose waits until all metrics
            // are received.
            let _received_message = self.receive_metrics(goose_attack_run_state, true).await?;
        }

        #[cfg(feature = "gaggle")]
        {
            // As worker, push metrics up to manager.
            if self.attack_mode == AttackMode::Worker {
                worker::push_metrics_to_manager(
                    &goose_attack_run_state.socket.clone().unwrap(),
                    vec![
                        GaggleMetrics::Requests(self.metrics.requests.clone()),
                        GaggleMetrics::Errors(self.metrics.errors.clone()),
                        GaggleMetrics::Transactions(self.metrics.transactions.clone()),
                    ],
                    true,
                );
                // No need to reset local metrics, the worker is exiting.
            }
        }

        Ok(())
    }

    // Cleanly shut down the [`GooseAttack`](./struct.GooseAttack.html).
    async fn stop_attack(&mut self) -> Result<(), GooseError> {
        // Run any configured test_stop() functions.
        self.run_test_stop().await?;

        // Percentile and errors are only displayed when the load test is finished.
        self.metrics.final_metrics = true;

        Ok(())
    }

    // Reset the GooseAttackRunState before starting a load test. This is to allow a Controller
    // to stop and start the load test multiple times, for example from a UI.
    async fn reset_run_state(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        // Run any configured test_start() functions.
        self.run_test_start().await.unwrap();

        // Prepare to collect metrics, if enabled.
        self.metrics = GooseMetrics::default();
        if !self.configuration.no_metrics {
            self.metrics.initialize_transaction_metrics(
                &self.scenarios,
                &self.configuration,
                &self.defaults,
            )?;
            if !self.configuration.no_print_metrics {
                self.metrics.display_metrics = true;
            }
            // Only display status codes if enabled.
            self.metrics.display_status_codes = self.configuration.status_codes;
        }

        // Reset the run state.
        let std_now = std::time::Instant::now();
        goose_attack_run_state.spawn_user_timer = std_now;
        goose_attack_run_state.spawn_user_in_ms = 0;
        goose_attack_run_state.spawn_user_counter = 0;
        goose_attack_run_state.drift_timer = tokio::time::Instant::now();
        goose_attack_run_state.metrics_header_displayed = false;
        goose_attack_run_state.idle_status_displayed = false;
        goose_attack_run_state.users = Vec::new();
        goose_attack_run_state.user_channels = Vec::new();
        goose_attack_run_state.running_metrics_timer = std_now;
        goose_attack_run_state.display_running_metrics = false;
        goose_attack_run_state.shutdown_after_stop = !self.configuration.no_autostart;
        goose_attack_run_state.all_users_spawned = false;

        // If enabled, spawn a logger thread.
        let (logger_handle, all_threads_logger_tx) =
            self.configuration.setup_loggers(&self.defaults).await?;
        goose_attack_run_state.logger_handle = logger_handle;
        goose_attack_run_state.all_threads_logger_tx = all_threads_logger_tx;

        // If enabled, spawn a throttle thread.
        let (throttle_threads_tx, parent_to_throttle_tx) = self.setup_throttle().await;
        goose_attack_run_state.throttle_threads_tx = throttle_threads_tx;
        goose_attack_run_state.parent_to_throttle_tx = parent_to_throttle_tx;

        // If enabled, create an report file and confirm access.
        goose_attack_run_state.report_file = match self.prepare_report_file().await {
            Ok(f) => f,
            Err(e) => {
                return Err(GooseError::InvalidOption {
                    option: "--report-file".to_string(),
                    value: self.get_report_file_path().unwrap(),
                    detail: format!("Failed to create report file: {}", e),
                })
            }
        };

        // Record when the GooseAttack officially started.
        self.started = Some(time::Instant::now());

        Ok(())
    }

    // Called internally in local-mode and gaggle-mode.
    async fn start_attack(mut self, socket: Option<Socket>) -> Result<GooseAttack, GooseError> {
        trace!("start_attack: socket({:?})", socket);

        // The GooseAttackRunState is used while spawning and running the
        // GooseUser threads that generate the load test.
        let mut goose_attack_run_state = self
            .initialize_attack(socket)
            .await
            .expect("failed to initialize GooseAttackRunState");

        // The Goose parent process GooseAttack loop runs until Goose shuts down. Goose enters
        // the loop in AttackPhase::Idle, and exits in AttackPhase::Shutdown.
        loop {
            match self.attack_phase {
                // In the Idle phase the Goose configuration can be changed by a Controller,
                // and otherwise nothing happens but sleeping an checking for messages.
                AttackPhase::Idle => {
                    if self.configuration.no_autostart {
                        // Sleep then check for further instructions.
                        if goose_attack_run_state.idle_status_displayed {
                            let sleep_duration = Duration::from_millis(250);
                            debug!("sleeping {:?}...", sleep_duration);
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
                        self.reset_run_state(&mut goose_attack_run_state).await?;
                        self.metrics
                            .history
                            .push(TestPlanHistory::step(TestPlanStepAction::Increasing, 0));
                        //self.graph_data.set_starting(Utc::now());
                        self.set_attack_phase(&mut goose_attack_run_state, AttackPhase::Increase);
                    }
                }
                // In the Increase phase, Goose launches GooseUser threads.
                AttackPhase::Increase => {
                    self.update_duration();
                    self.increase_attack(&mut goose_attack_run_state)
                        .await
                        .expect("failed to increase GooseAttack");
                }
                // In the Maintain phase, Goose continues runnning all launched GooseUser threads.
                AttackPhase::Maintain => {
                    self.update_duration();
                    self.maintain_attack(&mut goose_attack_run_state).await?;
                }
                // In the Decrease phase, Goose stops GooseUser threads.
                AttackPhase::Decrease => {
                    // If displaying metrics, update internal state reflecting how long load test
                    // has been running.
                    self.update_duration();
                    // Reduce the number of GooseUsers running.
                    self.decrease_attack(&mut goose_attack_run_state).await?;
                    // Stop any running GooseUser threads.
                    self.stop_attack().await?;
                    // Collect all metrics sent by GooseUser threads.
                    self.sync_metrics(&mut goose_attack_run_state, true).await?;
                    // Record last users for users per second graph in HTML report.
                    if let Some(started) = self.started {
                        self.graph_data.record_users_per_second(
                            self.metrics.users,
                            started.elapsed().as_secs() as usize,
                        );
                    };
                    // The load test is fully stopped at this point.
                    self.metrics
                        .history
                        .push(TestPlanHistory::step(TestPlanStepAction::Finished, 0));
                    // Write an html report, if enabled.
                    self.write_html_report(&mut goose_attack_run_state).await?;
                    // Shutdown Goose or go into an idle waiting state.
                    if goose_attack_run_state.shutdown_after_stop {
                        self.set_attack_phase(&mut goose_attack_run_state, AttackPhase::Shutdown);
                    } else {
                        // Print metrics, if enabled.
                        if !self.configuration.no_metrics {
                            println!("{}", self.metrics);
                        }
                        self.set_attack_phase(&mut goose_attack_run_state, AttackPhase::Idle);
                    }
                }
                // By reaching the Shutdown phase, break out of the GooseAttack loop.
                AttackPhase::Shutdown => {
                    self.update_duration();
                    break;
                }
            }

            // Record current users for users per second graph in HTML report.
            if let Some(started) = self.started {
                self.graph_data.record_users_per_second(
                    self.metrics.users,
                    started.elapsed().as_secs() as usize,
                );
            };

            // Regularly synchronize metrics.
            self.sync_metrics(&mut goose_attack_run_state, false)
                .await?;

            // Check if a Controller has made a request.
            self.handle_controller_requests(&mut goose_attack_run_state)
                .await?;

            // Gracefully exit loop if ctrl-c is caught.
            if self.attack_phase != AttackPhase::Shutdown
                && goose_attack_run_state.canceled.load(Ordering::SeqCst)
            {
                // Shutdown after stopping as the load test was canceled.
                goose_attack_run_state.shutdown_after_stop = true;

                // No metrics to display when sitting idle, so disable.
                if self.attack_phase == AttackPhase::Idle {
                    self.metrics.display_metrics = false;
                }

                // Cleanly stop the load test.
                self.set_attack_phase(&mut goose_attack_run_state, AttackPhase::Decrease);
                self.metrics.history.push(TestPlanHistory::step(
                    TestPlanStepAction::Decreasing,
                    self.metrics.users,
                ));
            }
        }

        Ok(self)
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
    debug!(
        "allocating Transactions on GooseUsers with {:?} scheduler",
        scheduler
    );

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
            trace!("calculating greatest common denominator of {} and {}", u, v);
            u = util::gcd(u, v);
            trace!("inner gcd: {}", u);
        }
    }
    // 'u' will always be the greatest common divisor
    debug!("gcd: {}", u);

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

    // Schedule sequenced transactions.
    let scheduled_sequenced_on_start_transactions =
        schedule_sequenced_transactions(&weighted_sequenced_on_start_transactions, scheduler);
    let scheduled_sequenced_transactions =
        schedule_sequenced_transactions(&weighted_sequenced_transactions, scheduler);
    let scheduled_sequenced_on_stop_transactions =
        schedule_sequenced_transactions(&weighted_sequenced_on_stop_transactions, scheduler);

    // Schedule unsequenced transactions.
    let scheduled_unsequenced_on_start_transactions = schedule_unsequenced_transactions(
        &weighted_unsequenced_on_start_transactions,
        total_unsequenced_on_start_transactions,
        scheduler,
    );
    let scheduled_unsequenced_transactions = schedule_unsequenced_transactions(
        &weighted_unsequenced_transactions,
        total_unsequenced_transactions,
        scheduler,
    );
    let scheduled_unsequenced_on_stop_transactions = schedule_unsequenced_transactions(
        &weighted_unsequenced_on_stop_transactions,
        total_unsequenced_on_stop_transactions,
        scheduler,
    );

    // Finally build a Vector of tuples: (transaction id, transaction name)
    let mut on_start_transactions = Vec::new();
    let mut transactions = Vec::new();
    let mut on_stop_transactions = Vec::new();

    // Sequenced transactions come first.
    for transaction in scheduled_sequenced_on_start_transactions.iter() {
        on_start_transactions.extend(vec![(
            *transaction,
            scenario.transactions[*transaction].name.to_string(),
        )])
    }
    for transaction in scheduled_sequenced_transactions.iter() {
        transactions.extend(vec![(
            *transaction,
            scenario.transactions[*transaction].name.to_string(),
        )])
    }
    for transaction in scheduled_sequenced_on_stop_transactions.iter() {
        on_stop_transactions.extend(vec![(
            *transaction,
            scenario.transactions[*transaction].name.to_string(),
        )])
    }

    // Unsequenced transactions come last.
    for transaction in scheduled_unsequenced_on_start_transactions.iter() {
        on_start_transactions.extend(vec![(
            *transaction,
            scenario.transactions[*transaction].name.to_string(),
        )])
    }
    for transaction in scheduled_unsequenced_transactions.iter() {
        transactions.extend(vec![(
            *transaction,
            scenario.transactions[*transaction].name.to_string(),
        )])
    }
    for transaction in scheduled_unsequenced_on_stop_transactions.iter() {
        on_stop_transactions.extend(vec![(
            *transaction,
            scenario.transactions[*transaction].name.to_string(),
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
            transaction.name,
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

    for (_sequence, transactions) in available_sequenced_transactions.iter() {
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
                        debug!(
                            "allocating transaction from Transaction {}",
                            transaction_index
                        );
                        weighted_transactions.push(transaction);
                    }
                }
                if weighted_transactions.len() >= total_transactions {
                    break;
                }
            }
        }
        GooseScheduler::Serial | GooseScheduler::Random => {
            // Allocate serially in the weighted order defined. If the Random scheduler is being used, they will get
            // shuffled later.
            for (transaction_index, transactions) in
                available_unsequenced_transactions.iter().enumerate()
            {
                debug!(
                    "allocating all {} transactions from Transaction {}",
                    transactions.len(),
                    transaction_index
                );

                let mut transactions_clone = transactions.clone();
                if scheduler == &GooseScheduler::Random {
                    transactions_clone.shuffle(&mut thread_rng());
                }
                weighted_transactions.append(&mut transactions_clone);
            }
        }
    }

    weighted_transactions
}
