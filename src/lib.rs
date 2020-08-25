//! # Goose
//!
//! Have you ever been attacked by a goose?
//!
//! Goose is a load testing tool inspired by [Locust](https://locust.io/).
//! User behavior is defined with standard Rust code.
//!
//! Goose load tests, called Goose Attacks, are built by creating an application
//! with Cargo, and declaring a dependency on the Goose library.
//!
//! Goose uses [`reqwest`](https://docs.rs/reqwest/) to provide a convenient HTTP
//! client.
//!
//! ## Creating and running a Goose load test
//!
//! ### Creating a simple Goose load test
//!
//! First create a new empty cargo application, for example:
//!
//! ```bash
//! $ cargo new loadtest
//!      Created binary (application) `loadtest` package
//! $ cd loadtest/
//! ```
//!
//! Add Goose as a dependency in `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! goose = "0.10"
//! ```
//!
//! Add the following boilerplate `use` declaration at the top of your `src/main.rs`:
//!
//! ```rust
//! use goose::prelude::*;
//! ```
//!
//! Using the above prelude will automatically add the following `use` statements
//! necessary for your load test, so you don't need to manually add them:
//!
//! ```rust
//! use goose::goose::{
//!     GooseTask, GooseTaskError, GooseTaskFunction, GooseTaskResult, GooseTaskSet, GooseUser,
//! };
//! use goose::metrics::GooseMetrics;
//! use goose::{task, taskset, GooseAttack, GooseError};
//! ```
//!
//! Below your `main` function (which currently is the default `Hello, world!`), add
//! one or more load test functions. The names of these functions are arbitrary, but it is
//! recommended you use self-documenting names. Load test functions must be async. Each load
//! test function must accept a GooseUser pointer. For example:
//!
//! ```rust
//! use goose::prelude::*;
//!
//! async fn loadtest_foo(user: &GooseUser) -> GooseTaskResult {
//!   let _goose = user.get("/path/to/foo").await?;
//!
//!   Ok(())
//! }   
//! ```
//!
//! In the above example, we're using the GooseUser helper method `get` to load a path
//! on the website we are load testing. This helper creates a Reqwest request builder, and
//! uses it to build and execute a request for the above path. If you want access to the
//! request builder object, you can instead use the `goose_get` helper, for example to
//! set a timout on this specific request:
//!
//! ```rust
//! use std::time;
//!
//! use goose::prelude::*;
//!
//! async fn loadtest_bar(user: &GooseUser) -> GooseTaskResult {
//!     let request_builder = user.goose_get("/path/to/bar").await?;
//!     let _goose = user.goose_send(request_builder.timeout(time::Duration::from_secs(3)), None).await?;
//!
//!     Ok(())
//! }   
//! ```
//!
//! We pass the `request_builder` object to `goose_send` which builds and executes it, also
//! collecting useful metrics. The `.await` at the end is necessary as `goose_send` is an
//! async function.
//!
//! Once all our tasks are created, we edit the main function to initialize goose and register
//! the tasks. In this very simple example we only have two tasks to register, while in a real
//! load test you can have any number of task sets with any number of individual tasks.
//!
//! ```rust,no_run
//! use goose::prelude::*;
//!
//! fn main() -> Result<(), GooseError> {
//!     let _goose_metrics = GooseAttack::initialize()?
//!         .register_taskset(taskset!("LoadtestTasks")
//!             .set_wait_time(0, 3)?
//!             // Register the foo task, assigning it a weight of 10.
//!             .register_task(task!(loadtest_foo).set_weight(10)?)
//!             // Register the bar task, assigning it a weight of 2 (so it
//!             // runs 1/5 as often as bar). Apply a task name which shows up
//!             // in metrics.
//!             .register_task(task!(loadtest_bar).set_name("bar").set_weight(2)?)
//!         )
//!         // You could also set a default host here, for example:
//!         //.set_host("http://dev.local/")
//!         .execute()?;
//!
//!     Ok(())
//! }
//!
//! async fn loadtest_foo(user: &GooseUser) -> GooseTaskResult {
//!     let _goose = user.get("/path/to/foo").await?;
//!
//!     Ok(())
//! }   
//!
//! async fn loadtest_bar(user: &GooseUser) -> GooseTaskResult {
//!     let _goose = user.get("/path/to/bar").await?;
//!
//!     Ok(())
//! }   
//! ```
//!
//! Goose now spins up a configurable number of users, each simulating a user on your
//! website. Thanks to Reqwest, each user maintains its own web client state, handling
//! cookies and more so your "users" can log in, fill out forms, and more, as real users
//! on your sites would do.
//!
//! ### Running the Goose load test
//!
//! Attempts to run our example will result in an error, as we have not yet defined the
//! host against which this load test should be run. We intentionally do not hard code the
//! host in the individual tasks, as this allows us to run the test against different
//! environments, such as local and staging.
//!
//! ```bash
//! $ cargo run --release
//!    Compiling loadtest v0.1.0 (~/loadtest)
//!     Finished release [optimized] target(s) in 1.52s
//!      Running `target/release/loadtest`
//! 05:33:06 [ERROR] Host must be defined globally or per-TaskSet. No host defined for LoadtestTasks.
//! ```
//! Pass in the `-h` flag to see all available run-time options. For now, we'll use a few
//! options to customize our load test.
//!
//! ```bash
//! $ cargo run --release -- --host http://dev.local -t 30s -v
//! ```
//!
//! The first option we specified is `--host`, and in this case tells Goose to run the load test
//! against an 8-core VM on my local network. The `-t 30s` option tells Goose to end the load test
//! after 30 seconds (for real load tests you'll certainly want to run it longer, you can use `m` to
//! specify minutes and `h` to specify hours. For example, `-t 1h30m` would run the load test for 1
//! hour 30 minutes). Finally, the `-v` flag tells goose to display INFO and higher level logs to
//! stdout, giving more insight into what is happening. (Additional `-v` flags will result in
//! considerably more debug output, and are not recommended for running actual load tests; they're
//! only useful if you're trying to debug Goose itself.)
//!
//! Running the test results in the following output (broken up to explain it as it goes):
//!
//! ```bash
//!    Finished release [optimized] target(s) in 0.05s
//!     Running `target/release/loadtest --host 'http://dev.local' -t 30s -v`
//! 05:56:30 [ INFO] Output verbosity level: INFO
//! 05:56:30 [ INFO] Logfile verbosity level: INFO
//! 05:56:30 [ INFO] Writing to log file: goose.log

//! ```
//!
//! By default Goose will write a log file with INFO and higher level logs into the same directory
//! as you run the test from.
//!
//! ```bash
//! 05:56:30 [ INFO] run_time = 30
//! 05:56:30 [ INFO] concurrent users defaulted to 8 (number of CPUs)
//! ```
//!
//! Goose will default to launching 1 user per available CPU core, and will launch them all in
//! one second. You can change how many users are launched with the `-u` option, and you can
//! change how many users are launched per second with the `-r` option. For example, `-u 30 -r 2`
//! would launch 30 users over 15 seconds, or two users per second.
//!
//! ```bash
//! 05:56:30 [ INFO] global host configured: http://dev.local
//! 05:56:30 [ INFO] launching user 1 from LoadtestTasks...
//! 05:56:30 [ INFO] launching user 2 from LoadtestTasks...
//! 05:56:30 [ INFO] launching user 3 from LoadtestTasks...
//! 05:56:30 [ INFO] launching user 4 from LoadtestTasks...
//! 05:56:30 [ INFO] launching user 5 from LoadtestTasks...
//! 05:56:30 [ INFO] launching user 6 from LoadtestTasks...
//! 05:56:30 [ INFO] launching user 7 from LoadtestTasks...
//! 05:56:31 [ INFO] launching user 8 from LoadtestTasks...
//! 05:56:31 [ INFO] launched 8 users...
//! ```
//!
//! Each user is launched in its own thread with its own user state. Goose is able to make
//! very efficient use of server resources.
//!
//! ```bash
//! 05:56:46 [ INFO] printing running metrics after 15 seconds...
//! ------------------------------------------------------------------------------
//!  Name                    | # reqs         | # fails        | req/s  | fail/s
//!  -----------------------------------------------------------------------------
//!  GET /path/to/foo        | 15,795         | 0 (0%)         | 1,053  | 0    
//!  GET bar                 | 3,161          | 0 (0%)         | 210    | 0    
//!  ------------------------+----------------+----------------+--------+---------
//!  Aggregated              | 18,956         | 0 (0%)         | 1,263  | 0    
//! ------------------------------------------------------------------------------
//! ```
//!
//! When printing metrics, by default Goose will display running values approximately
//! every 15 seconds. Running metrics are broken into two tables. The first, above,
//! shows how many requests have been made, how many of them failed (non-2xx response),
//! and the corresponding per-second rates.
//!
//! Note that Goose respected the per-task weights we set, and `foo` (with a weight of
//! 10) is being loaded five times as often as `bar` (with a weight of 2). Also notice
//! that because we didn't name the `foo` task by default we see the URL loaded in the
//! metrics, whereas we did name the `bar` task so we see the name in the metrics.
//!
//! ```bash
//!  Name                    | Avg (ms)   | Min        | Max        | Mean      
//!  -----------------------------------------------------------------------------
//!  GET /path/to/foo        | 67         | 31         | 1351       | 53      
//!  GET bar                 | 60         | 33         | 1342       | 53      
//!  ------------------------+------------+------------+------------+-------------
//!  Aggregated              | 66         | 31         | 1351       | 56      
//! ```
//!
//! The second table in running metrics provides details on response times. In our
//! example (which is running over wifi from my development laptop), on average each
//! page is returning within `66` milliseconds. The quickest page response was for
//! `foo` in `31` milliseconds. The slowest page response was also for `foo` in `1351`
//! milliseconds.
//!
//!
//! ```bash
//! 05:37:10 [ INFO] stopping after 30 seconds...
//! 05:37:10 [ INFO] waiting for users to exit
//! ```
//!
//! Our example only runs for 30 seconds, so we only see running metrics once. When
//! the test completes, we get more detail in the final summary. The first two tables
//! are the same as what we saw earlier, however now they include all metrics for the
//! entire load test:
//!
//! ```bash
//! ------------------------------------------------------------------------------
//!  Name                    | # reqs         | # fails        | req/s  | fail/s
//!  -----------------------------------------------------------------------------
//!  GET bar                 | 6,050          | 0 (0%)         | 201    | 0    
//!  GET /path/to/foo        | 30,257         | 0 (0%)         | 1,008  | 0    
//!  ------------------------+----------------+----------------+--------+----------
//!  Aggregated              | 36,307         | 0 (0%)         | 1,210  | 0    
//! -------------------------------------------------------------------------------
//!  Name                    | Avg (ms)   | Min        | Max        | Mean      
//!  -----------------------------------------------------------------------------
//!  GET bar                 | 66         | 32         | 1388       | 53      
//!  GET /path/to/foo        | 68         | 31         | 1395       | 53      
//!  ------------------------+------------+------------+------------+-------------
//!  Aggregated              | 67         | 31         | 1395       | 50      
//! -------------------------------------------------------------------------------
//! ```
//!
//! The ratio between `foo` and `bar` remained 5:2 as expected. As the test ran,
//! however, we saw some slower page loads, with the slowest again `foo` this time
//! at 1395 milliseconds.
//!
//! ```bash
//! Slowest page load within specified percentile of requests (in ms):
//! ------------------------------------------------------------------------------
//! Name                    | 50%    | 75%    | 98%    | 99%    | 99.9%  | 99.99%
//! -----------------------------------------------------------------------------
//! GET bar                 | 53     | 66     | 217   | 537     | 1872   | 12316
//! GET /path/to/foo        | 53     | 66     | 265   | 1060    | 1800   | 10732
//! ------------------------+--------+--------+-------+---------+--------+-------
//! Aggregated              | 53     | 66     | 237   | 645     | 1832   | 10818
//! ```
//!
//! A new table shows additional information, breaking down response-time by
//! percentile. This shows that the slowest page loads only happened in the
//! slowest .001% of page loads, so were very much an edge case. 99.9% of the time
//! page loads happened in less than 2 seconds.
//!
//! ## License
//!
//! Copyright 2020 Jeremy Andrews
//!
//! Licensed under the Apache License, Version 2.0 (the "License");
//! you may not use this file except in compliance with the License.
//! You may obtain a copy of the License at
//!
//! http://www.apache.org/licenses/LICENSE-2.0
//!
//! Unless required by applicable law or agreed to in writing, software
//! distributed under the License is distributed on an "AS IS" BASIS,
//! WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//! See the License for the specific language governing permissions and
//! limitations under the License.

#[macro_use]
extern crate log;

pub mod goose;
pub mod logger;
#[cfg(feature = "gaggle")]
mod manager;
pub mod metrics;
pub mod prelude;
mod throttle;
mod user;
mod util;
#[cfg(feature = "gaggle")]
mod worker;

use gumdrop::Options;
use lazy_static::lazy_static;
#[cfg(feature = "gaggle")]
use nng::Socket;
use serde::{Deserialize, Serialize};
use serde_json::json;
use simplelog::*;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::{f32, fmt, io, time};
use tokio::fs::File;
use tokio::io::BufWriter;
use tokio::prelude::*;
use tokio::sync::mpsc;
use url::Url;

use crate::goose::{
    GooseDebug, GooseRawRequest, GooseRequest, GooseTask, GooseTaskSet, GooseUser, GooseUserCommand,
};
use crate::metrics::{GooseMetric, GooseMetrics};
#[cfg(feature = "gaggle")]
use crate::worker::GaggleMetrics;

/// Constant defining how often metrics should be displayed while load test is running.
const RUNNING_METRICS_EVERY: usize = 15;

/// Constant defining Goose's default port when running a Gaggle.
const DEFAULT_PORT: &str = "5115";

// WORKER_ID is only used when running a gaggle (a distributed load test).
lazy_static! {
    static ref WORKER_ID: AtomicUsize = AtomicUsize::new(0);
}

/// Internal representation of a weighted task list.
type WeightedGooseTasks = Vec<Vec<usize>>;

/// Worker ID to aid in tracing logs when running a Gaggle.
pub fn get_worker_id() -> usize {
    WORKER_ID.load(Ordering::Relaxed)
}

#[cfg(not(feature = "gaggle"))]
#[derive(Debug)]
/// Socket used for coordinating a Gaggle, a distributed load test.
pub struct Socket {}

/// Definition of all errors a GooseAttack can return.
#[derive(Debug)]
pub enum GooseError {
    /// Contains an io::Error.
    Io(io::Error),
    /// Contains a reqwest::Error.
    Reqwest(reqwest::Error),
    /// Failed attempt to use code that requires a compile-time feature be enabled. The missing
    /// feature is named in `.feature`. An optional explanation may be found in `.detail`.
    FeatureNotEnabled { feature: String, detail: String },
    /// Failed to parse hostname. The invalid hostname that caused this error is found in
    /// `.host`. An optional explanation may be found in `.detail`. The lower level
    /// `url::ParseError` is contained in `.parse_error`.
    InvalidHost {
        host: String,
        detail: String,
        parse_error: url::ParseError,
    },
    /// Invalid option or value specified, may only be invalid in context. The invalid option
    /// is found in `.option`, while the invalid value is found in `.value`. An optional
    /// explanation providing context may be found in `.detail`.
    InvalidOption {
        option: String,
        value: String,
        detail: String,
    },
    /// Invalid wait time specified. The minimum wait time and maximum wait time are found in
    /// `.min_wait` and `.max_wait` respectively. An optional explanation providing context may
    /// be found in `.detail`.
    InvalidWaitTime {
        min_wait: usize,
        max_wait: usize,
        detail: String,
    },
    /// Invalid weight specified. The invalid weight value is found in `.weight`. An optional
    // explanation providing context may be found in `.detail`.
    InvalidWeight { weight: usize, detail: String },
    /// `GooseAttack` has no `GooseTaskSet` defined. An optional explanation may be found in
    /// `.detail`.
    NoTaskSets { detail: String },
}
impl GooseError {
    fn describe(&self) -> &str {
        match *self {
            GooseError::Io(_) => "io::Error",
            GooseError::Reqwest(_) => "reqwest::Error",
            GooseError::FeatureNotEnabled { .. } => "required compile-time feature not enabled",
            GooseError::InvalidHost { .. } => "failed to parse hostname",
            GooseError::InvalidOption { .. } => "invalid option or value specified",
            GooseError::InvalidWaitTime { .. } => "invalid wait_time specified",
            GooseError::InvalidWeight { .. } => "invalid weight specified",
            GooseError::NoTaskSets { .. } => "no task sets defined",
        }
    }
}

// Define how to display errors.
impl fmt::Display for GooseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            GooseError::Io(ref source) => write!(f, "GooseError: {} ({})", self.describe(), source),
            GooseError::Reqwest(ref source) => {
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

/// Internal global state for load test.
#[derive(Clone)]
pub struct GooseAttack {
    /// An optional task to run one time before starting users and running task sets.
    test_start_task: Option<GooseTask>,
    /// An optional task to run one time after users have finished running task sets.
    test_stop_task: Option<GooseTask>,
    /// A vector containing one copy of each GooseTaskSet that will run during this load test.
    task_sets: Vec<GooseTaskSet>,
    /// A weighted vector containing a GooseUser object for each user that will run during this load test.
    weighted_users: Vec<GooseUser>,
    /// An optional default host to run this load test against.
    host: Option<String>,
    /// Configuration object managed by StructOpt.
    configuration: GooseConfiguration,
    /// By default launch 1 user per number of CPUs.
    number_of_cpus: usize,
    /// Track how long the load test should run.
    run_time: usize,
    /// Track total number of users to run for this load test.
    users: usize,
    /// When the load test started.
    started: Option<time::Instant>,
    /// All metrics merged together.
    metrics: GooseMetrics,
}
/// Goose's internal global state.
impl GooseAttack {
    /// Load configuration from command line and initialize a GooseAttack.
    ///
    /// # Example
    /// ```rust,no_run
    ///     use goose::prelude::*;
    ///
    ///     let mut goose_attack = GooseAttack::initialize();
    /// ```
    pub fn initialize() -> Result<GooseAttack, GooseError> {
        let config = GooseConfiguration::parse_args_default_or_exit();
        let users = GooseAttack::set_users(&config)?;
        let goose_attack = GooseAttack {
            test_start_task: None,
            test_stop_task: None,
            task_sets: Vec::new(),
            weighted_users: Vec::new(),
            host: None,
            configuration: config,
            number_of_cpus: num_cpus::get(),
            run_time: 0,
            users: users,
            started: None,
            metrics: GooseMetrics::default(),
        };
        Ok(goose_attack.setup()?)
    }

    /// Initialize a GooseAttack with an already loaded configuration.
    /// This should only be called by worker instances.
    ///
    /// # Example
    /// ```rust,no_run
    ///     use goose::{GooseAttack, GooseConfiguration};
    ///     use gumdrop::Options;
    ///
    ///     let configuration = GooseConfiguration::parse_args_default_or_exit();
    ///     let mut goose_attack = GooseAttack::initialize_with_config(configuration);
    /// ```
    pub fn initialize_with_config(config: GooseConfiguration) -> Result<GooseAttack, GooseError> {
        let users = GooseAttack::set_users(&config)?;
        Ok(GooseAttack {
            test_start_task: None,
            test_stop_task: None,
            task_sets: Vec::new(),
            weighted_users: Vec::new(),
            host: None,
            configuration: config,
            number_of_cpus: num_cpus::get(),
            run_time: 0,
            users: users,
            started: None,
            metrics: GooseMetrics::default(),
        })
    }

    pub fn initialize_logger(&self) {
        // Allow optionally controlling debug output level
        let debug_level;
        match self.configuration.verbose {
            0 => debug_level = LevelFilter::Warn,
            1 => debug_level = LevelFilter::Info,
            2 => debug_level = LevelFilter::Debug,
            _ => debug_level = LevelFilter::Trace,
        }

        // Allow optionally controlling log level
        let log_level;
        match self.configuration.log_level {
            0 => log_level = LevelFilter::Info,
            1 => log_level = LevelFilter::Debug,
            _ => log_level = LevelFilter::Trace,
        }

        let log_file = PathBuf::from(&self.configuration.log_file);

        match CombinedLogger::init(vec![
            match TermLogger::new(debug_level, Config::default(), TerminalMode::Mixed) {
                Some(t) => t,
                None => {
                    eprintln!("failed to initialize TermLogger");
                    return;
                }
            },
            WriteLogger::new(
                log_level,
                Config::default(),
                std::fs::File::create(&log_file).unwrap(),
            ),
        ]) {
            Ok(_) => (),
            Err(e) => {
                info!("failed to initialize CombinedLogger: {}", e);
            }
        }
        info!("Output verbosity level: {}", debug_level);
        info!("Logfile verbosity level: {}", log_level);
        info!("Writing to log file: {}", log_file.display());
    }

    // Helper to determine the number of user threads to launch, defaulting to the
    // number of CPU cores available.
    fn set_users(config: &GooseConfiguration) -> Result<usize, GooseError> {
        match config.users {
            Some(u) => {
                if u == 0 {
                    if config.worker {
                        return Err(GooseError::InvalidOption {
                            option: "--users".to_string(),
                            value: u.to_string(),
                            detail: "The --users option must be set to at least 1.".to_string(),
                        });
                    }
                    Ok(0)
                } else {
                    if config.worker {
                        return Err(GooseError::InvalidOption {
                            option: "--users".to_string(),
                            value: u.to_string(),
                            detail:
                                "The --users option can not be set together with the --worker flag."
                                    .to_string(),
                        });
                    }
                    Ok(u)
                }
            }
            None => {
                let u = num_cpus::get();
                if !config.manager && !config.worker {
                    info!("concurrent users defaulted to {} (number of CPUs)", u);
                }
                Ok(u)
            }
        }
    }

    pub fn setup(mut self) -> Result<Self, GooseError> {
        // If version flag is set, display package name and version and exit.
        if self.configuration.version {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }

        self.initialize_logger();

        // Collecting metrics is required for the following options.
        if self.configuration.no_metrics {
            // Don't allow overhead of collecting metrics unless we're printing them.
            if self.configuration.status_codes {
                return Err(GooseError::InvalidOption {
                    option: "--no-metrics".to_string(),
                    value: "true".to_string(),
                    detail: "The --no-metrics flag can not be set together with the --status-codes flag.".to_string(),
                });
            }

            // Don't allow overhead of collecting metrics unless we're printing them.
            if self.configuration.only_summary {
                return Err(GooseError::InvalidOption {
                    option: "--no-metrics".to_string(),
                    value: "true".to_string(),
                    detail: "The --no-metrics flag can not be set together with the --only-summary flag.".to_string(),
                });
            }

            // There is nothing to log if metrics are disabled.
            if !self.configuration.metrics_file.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "--no-metrics".to_string(),
                    value: "true".to_string(),
                    detail: "The --no-metrics flag can not be set together with the --metrics-file option.".to_string(),
                });
            }

            // There is nothing to log if metrics are disabled.
            if !self.configuration.metrics_file.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "--no-metrics".to_string(),
                    value: "true".to_string(),
                    detail: "The --no-metrics flag can not be set together with the --metrics-format option.".to_string(),
                });
            }
        }

        if self.configuration.metrics_format != "json" {
            // Log format isn't relevant if log not enabled.
            if self.configuration.metrics_file.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "--metrics-format".to_string(),
                    value: self.configuration.metrics_format,
                    detail: "The --metrics-file option must be set together with the --metrics-format option.".to_string(),
                });
            }

            // All of these options must be defined below, search for formatted_log.
            let options = vec!["json", "csv", "raw"];
            if !options.contains(&self.configuration.metrics_format.as_str()) {
                return Err(GooseError::InvalidOption {
                    option: "--metrics-format".to_string(),
                    value: self.configuration.metrics_format,
                    detail: format!(
                        "The --metrics-format option must be set to one of: {}.",
                        options.join(", ")
                    ),
                });
            }
        }

        if self.configuration.debug_format != "json" {
            // Log format isn't relevant if log not enabled.
            if self.configuration.debug_file.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "--debug-format".to_string(),
                    value: self.configuration.debug_format,
                    detail: "The --debug-file option must be set together with the --debug-format option.".to_string(),
                });
            }

            // All of these options must be defined below, search for formatted_log.
            let options = vec!["json", "raw"];
            if !options.contains(&self.configuration.debug_format.as_str()) {
                return Err(GooseError::InvalidOption {
                    option: "--debug-format".to_string(),
                    value: self.configuration.debug_format,
                    detail: format!(
                        "The --debug-format option must be set to one of: {}.",
                        options.join(", ")
                    ),
                });
            }
        }

        // Configure maximum run time if specified, otherwise run until canceled.
        if self.configuration.worker {
            if self.configuration.run_time != "" {
                return Err(GooseError::InvalidOption {
                    option: "--run-time".to_string(),
                    value: "true".to_string(),
                    detail: "The --run-time option can not be set together with the --worker flag."
                        .to_string(),
                });
            }
            self.run_time = 0;
        } else if self.configuration.run_time != "" {
            self.run_time = util::parse_timespan(&self.configuration.run_time);
            info!("run_time = {}", self.run_time);
        } else {
            self.run_time = 0;
        }

        if !self.configuration.manager && !self.configuration.worker {
            debug!("users = {}", self.users);
        }

        Ok(self)
    }

    /// A load test must contain one or more `GooseTaskSet`s. Each task set must
    /// be registered into Goose's global state with this method for it to run.
    ///
    /// # Example
    /// ```rust,no_run
    ///     use goose::prelude::*;
    ///
    /// fn main() -> Result<(), GooseError> {
    ///     GooseAttack::initialize()?
    ///         .register_taskset(taskset!("ExampleTasks")
    ///             .register_task(task!(example_task))
    ///         )
    ///         .register_taskset(taskset!("OtherTasks")
    ///             .register_task(task!(other_task))
    ///         );
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn example_task(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.get("/foo").await?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn other_task(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.get("/bar").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn register_taskset(mut self, mut taskset: GooseTaskSet) -> Self {
        taskset.task_sets_index = self.task_sets.len();
        self.task_sets.push(taskset);
        self
    }

    /// Optionally define a task to run before users are started and all task sets
    /// start running. This is would generally be used to set up anything required
    /// for the load test.
    ///
    /// When running in a distributed Gaggle, this task is only run one time by the
    /// Manager.
    ///
    /// # Example
    /// ```rust,no_run
    ///     use goose::prelude::*;
    ///
    /// fn main() -> Result<(), GooseError> {
    ///     GooseAttack::initialize()?
    ///         .test_start(task!(setup));
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn setup(user: &GooseUser) -> GooseTaskResult {
    ///     // do stuff to set up load test ...
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn test_start(mut self, task: GooseTask) -> Self {
        self.test_start_task = Some(task);
        self
    }

    /// Optionally define a task to run after all users have finished running
    /// all defined task sets. This would generally be used to clean up anything
    /// that was specifically set up for the load test.
    ///
    /// When running in a distributed Gaggle, this task is only run one time by the
    /// Manager.
    ///
    /// # Example
    /// ```rust,no_run
    ///     use goose::prelude::*;
    ///
    /// fn main() -> Result<(), GooseError> {
    ///     GooseAttack::initialize()?
    ///         .test_stop(task!(teardown));
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn teardown(user: &GooseUser) -> GooseTaskResult {
    ///     // do stuff to tear down the load test ...
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn test_stop(mut self, task: GooseTask) -> Self {
        self.test_stop_task = Some(task);
        self
    }

    /// Optionally configure a default host for the load test. This is used if
    /// no per-GooseTaskSet host is defined, no `--host` CLI option is configurared,
    /// and if the GooseTask itself doesn't hard-code the host in its request. The
    /// host is prepended on all requests.
    ///
    /// For example, your load test may default to running against your local development
    /// container, and the `--host` option could be used to override host to run the load
    /// test against production.
    ///
    /// # Example
    /// ```rust,no_run
    ///     use goose::prelude::*;
    ///
    /// fn main() -> Result<(), GooseError> {
    ///     GooseAttack::initialize()?
    ///         .set_host("local.dev");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_host(mut self, host: &str) -> Self {
        trace!("set_host: {}", host);
        // Host validation happens in main() at startup.
        self.host = Some(host.to_string());
        self
    }

    /// Allocate a vector of weighted GooseUser.
    fn weight_task_set_users(&mut self) -> Result<Vec<GooseUser>, GooseError> {
        trace!("weight_task_set_users");

        let mut u: usize = 0;
        let mut v: usize;
        for task_set in &self.task_sets {
            if u == 0 {
                u = task_set.weight;
            } else {
                v = task_set.weight;
                trace!("calculating greatest common denominator of {} and {}", u, v);
                u = util::gcd(u, v);
                trace!("inner gcd: {}", u);
            }
        }
        // 'u' will always be the greatest common divisor
        debug!("gcd: {}", u);

        // Build a weighted lists of task sets (identified by index)
        let mut weighted_task_sets = Vec::new();
        for (index, task_set) in self.task_sets.iter().enumerate() {
            // divide by greatest common divisor so vector is as short as possible
            let weight = task_set.weight / u;
            trace!(
                "{}: {} has weight of {} (reduced with gcd to {})",
                index,
                task_set.name,
                task_set.weight,
                weight
            );
            let mut weighted_sets = vec![index; weight];
            weighted_task_sets.append(&mut weighted_sets);
        }

        // Allocate a state for each user that will be hatched.
        info!("initializing user states...");
        let mut weighted_users = Vec::new();
        let mut user_count = 0;
        loop {
            for task_sets_index in &weighted_task_sets {
                let base_url = goose::get_base_url(
                    self.get_configuration_host(),
                    self.task_sets[*task_sets_index].host.clone(),
                    self.host.clone(),
                )?;
                weighted_users.push(GooseUser::new(
                    self.task_sets[*task_sets_index].task_sets_index,
                    base_url,
                    self.task_sets[*task_sets_index].min_wait,
                    self.task_sets[*task_sets_index].max_wait,
                    &self.configuration,
                    self.metrics.hash,
                )?);
                user_count += 1;
                if user_count >= self.users {
                    trace!("created {} weighted_users", user_count);
                    return Ok(weighted_users);
                }
            }
        }
    }

    /// Execute the load test.
    ///
    /// # Example
    /// ```rust,no_run
    /// use goose::prelude::*;
    ///
    /// fn main() -> Result<(), GooseError> {
    ///     let _goose_metrics = GooseAttack::initialize()?
    ///         .register_taskset(taskset!("ExampleTasks")
    ///             .register_task(task!(example_task).set_weight(2)?)
    ///             .register_task(task!(another_example_task).set_weight(3)?)
    ///         )
    ///         .execute()?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn example_task(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.get("/foo").await?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn another_example_task(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.get("/bar").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn execute(mut self) -> Result<GooseMetrics, GooseError> {
        // At least one task set is required.
        if self.task_sets.is_empty() {
            return Err(GooseError::NoTaskSets {
                detail: "No task sets are defined.".to_string(),
            });
        }

        if self.configuration.list {
            // Display task sets and tasks, then exit.
            println!("Available tasks:");
            for task_set in self.task_sets {
                println!(" - {} (weight: {})", task_set.name, task_set.weight);
                for task in task_set.tasks {
                    println!("    o {} (weight: {})", task.name, task.weight);
                }
            }
            std::process::exit(0);
        }

        // Manager mode.
        if self.configuration.manager {
            // @TODO: support running in both manager and worker mode.
            if self.configuration.worker {
                return Err(GooseError::InvalidOption {
                    option: "--worker".to_string(),
                    value: "true".to_string(),
                    detail: "The --worker flag can not be set together with the --manager flag"
                        .to_string(),
                });
            }

            if self.configuration.expect_workers < 1 {
                return Err(GooseError::InvalidOption {
                    option: "--expect-workers".to_string(),
                    value: self.configuration.expect_workers.to_string(),
                    detail: "The --expect-workers option must be set to at least 1.".to_string(),
                });
            }
            if self.configuration.expect_workers as usize > self.users {
                return Err(GooseError::InvalidOption {
                    option: "--expect-workers".to_string(),
                    value: self.configuration.expect_workers.to_string(),
                    detail: "The --expect-workers option can not be set to a value larger than --users option.".to_string(),
                });
            }

            if !self.configuration.debug_file.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "--debug-file".to_string(),
                    value: self.configuration.debug_file,
                    detail:
                        "The --debug-file option can not be set together with the --manager flag."
                            .to_string(),
                });
            }

            if self.configuration.throttle_requests.is_some() {
                return Err(GooseError::InvalidOption {
                    option: "--throttle-requests".to_string(),
                    value: self.configuration.throttle_requests.unwrap().to_string(),
                    detail: "The --throttle-requests option can not be set together with the --manager flag.".to_string(),
                });
            }
        }

        // Validate throttle_requests, which must be a value from 1 to 1,000,000.
        match self.configuration.throttle_requests {
            Some(throttle) if throttle == 0 => {
                return Err(GooseError::InvalidOption {
                    option: "--throttle-requests".to_string(),
                    value: throttle.to_string(),
                    detail: "The --throttle-requests option must be set to at least 1 request per second.".to_string(),
                });
            }
            Some(throttle) if throttle > 1_000_000 => {
                return Err(GooseError::InvalidOption {
                    option: "--throttle-requests".to_string(),
                    value: throttle.to_string(),
                    detail: "The --throttle-requests option can not be set to more than 1,000,000 requests per second.".to_string(),
                });
            }
            // Everything else is valid.
            _ => (),
        }

        // Worker mode.
        if self.configuration.worker {
            // @TODO: support running in both manager and worker mode.
            if self.configuration.manager {
                return Err(GooseError::InvalidOption {
                    option: "--manager".to_string(),
                    value: "true".to_string(),
                    detail: "The --manager flag can not be set together with the --worker flag."
                        .to_string(),
                });
            }

            if self.configuration.expect_workers > 0 {
                return Err(GooseError::InvalidOption {
                    option: "--expect-workers".to_string(),
                    value: self.configuration.expect_workers.to_string(),
                    detail: "The --expect-workers option can not be set together with the --worker flag.".to_string(),
                });
            }

            if !self.configuration.host.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "--host".to_string(),
                    value: self.configuration.host,
                    detail: "The --host option can not be set together with the --worker flag."
                        .to_string(),
                });
            }

            if self.configuration.manager_bind_host != "0.0.0.0" {
                return Err(GooseError::InvalidOption {
                    option: "--manager-bind-host".to_string(),
                    value: self.configuration.manager_bind_host,
                    detail: "The --manager-bind-host option can not be set together with the --worker flag.".to_string(),
                });
            }

            let default_port: u16 = DEFAULT_PORT.to_string().parse().unwrap();
            if self.configuration.manager_bind_port != default_port {
                return Err(GooseError::InvalidOption {
                    option: "--manager-bind-port".to_string(),
                    value: self.configuration.manager_bind_port.to_string(),
                    detail: "The --manager-bind-port option can not be set together with the --worker flag.".to_string(),
                });
            }

            if self.configuration.no_metrics {
                return Err(GooseError::InvalidOption {
                    option: "--no-metrics".to_string(),
                    value: self.configuration.no_metrics.to_string(),
                    detail: "The --no-metrics flag can not be set together with the --worker flag."
                        .to_string(),
                });
            }

            if self.configuration.no_task_metrics {
                return Err(GooseError::InvalidOption {
                    option: "--no-task-metrics".to_string(),
                    value: self.configuration.no_task_metrics.to_string(),
                    detail:
                        "The --no-task-metrics flag can not be set together with the --worker flag."
                            .to_string(),
                });
            }

            if self.configuration.only_summary {
                return Err(GooseError::InvalidOption {
                    option: "--only-summary".to_string(),
                    value: self.configuration.only_summary.to_string(),
                    detail:
                        "The --only-summary flag can not be set together with the --worker flag."
                            .to_string(),
                });
            }

            if self.configuration.status_codes {
                return Err(GooseError::InvalidOption {
                    option: "--status-codes".to_string(),
                    value: self.configuration.status_codes.to_string(),
                    detail:
                        "The --status-codes flag can not be set together with the --worker flag."
                            .to_string(),
                });
            }

            if self.configuration.no_reset_metrics {
                return Err(GooseError::InvalidOption {
                    option: "--no-reset-metrics".to_string(),
                    value: self.configuration.no_reset_metrics.to_string(),
                    detail: "The --no-reset-metrics flag can not be set together with the --worker flag.".to_string(),
                });
            }

            if self.configuration.no_hash_check {
                return Err(GooseError::InvalidOption {
                    option: "--no-hash-check".to_string(),
                    value: self.configuration.no_hash_check.to_string(),
                    detail:
                        "The --no-hash-check flag can not be set together with the --worker flag."
                            .to_string(),
                });
            }
        }

        if !self.configuration.manager && !self.configuration.worker {
            if self.configuration.no_hash_check {
                return Err(GooseError::InvalidOption {
                    option: "--no-hash-check".to_string(),
                    value: self.configuration.no_hash_check.to_string(),
                    detail: "The --no-hash-check flag can not be set without also setting the --manager flag.".to_string(),
                });
            }

            if self.configuration.expect_workers > 0 {
                return Err(GooseError::InvalidOption {
                    option: "--expect-workers".to_string(),
                    value: self.configuration.expect_workers.to_string(),
                    detail: "The --expect-workers flag can not be set without also setting the --manager flag.".to_string(),
                });
            }
        }

        // Configure number of user threads to launch per second, defaults to 1.
        if self.configuration.hatch_rate == 0 {
            return Err(GooseError::InvalidOption {
                option: "--hatch-rate".to_string(),
                value: self.configuration.hatch_rate.to_string(),
                detail: "The --hatch-rate option must be set to at least 1.".to_string(),
            });
        }
        if self.configuration.hatch_rate > 1 && self.configuration.worker {
            return Err(GooseError::InvalidOption {
                option: "--hatch-rate".to_string(),
                value: self.configuration.hatch_rate.to_string(),
                detail: "The --hatch-rate option can not be set together with the --worker flag."
                    .to_string(),
            });
        }
        debug!("hatch_rate = {}", self.configuration.hatch_rate);

        // Confirm there's either a global host, or each task set has a host defined.
        if self.configuration.host.is_empty() {
            for task_set in &self.task_sets {
                match &task_set.host {
                    Some(h) => {
                        if is_valid_host(h).is_ok() {
                            info!("host for {} configured: {}", task_set.name, h);
                        }
                    }
                    None => match &self.host {
                        Some(h) => {
                            if is_valid_host(h).is_ok() {
                                info!("host for {} configured: {}", task_set.name, h);
                            }
                        }
                        None => {
                            if !self.configuration.worker {
                                return Err(GooseError::InvalidOption {
                                    option: "--host".to_string(),
                                    value: "".to_string(),
                                    detail: format!("A host must be defined via the --host option, the GooseAttack.set_host() function, or the GooseTaskSet.set_host() function (no host defined for {}).", task_set.name)
                                });
                            }
                        }
                    },
                }
            }
        } else if is_valid_host(&self.configuration.host).is_ok() {
            info!("global host configured: {}", self.configuration.host);
        }

        // Apply weights to tasks in each task set.
        for task_set in &mut self.task_sets {
            let (weighted_on_start_tasks, weighted_tasks, weighted_on_stop_tasks) =
                weight_tasks(&task_set);
            task_set.weighted_on_start_tasks = weighted_on_start_tasks;
            task_set.weighted_tasks = weighted_tasks;
            task_set.weighted_on_stop_tasks = weighted_on_stop_tasks;
            debug!(
                "weighted {} on_start: {:?} tasks: {:?} on_stop: {:?}",
                task_set.name,
                task_set.weighted_on_start_tasks,
                task_set.weighted_tasks,
                task_set.weighted_on_stop_tasks
            );
        }

        if !self.configuration.worker {
            // Allocate a state for each of the users we are about to start.
            self.weighted_users = self.weight_task_set_users()?;

            // Stand-alone and Manager processes can display metrics.
            if !self.configuration.no_metrics {
                self.metrics.display_metrics = true;
            }
        }

        // Calculate a unique hash for the current load test.
        let mut s = DefaultHasher::new();
        self.task_sets.hash(&mut s);
        self.metrics.hash = s.finish();
        debug!("hash: {}", self.metrics.hash);

        // Our load test is officially starting.
        self.started = Some(time::Instant::now());
        // Hatch users at hatch_rate per second, or one every 1 / hatch_rate fraction of a second.
        let sleep_float = 1.0 / self.configuration.hatch_rate as f32;
        let sleep_duration = time::Duration::from_secs_f32(sleep_float);

        // Start goose in manager mode.
        if self.configuration.manager {
            #[cfg(feature = "gaggle")]
            {
                let mut rt = tokio::runtime::Runtime::new().unwrap();
                self = rt.block_on(manager::manager_main(self));
            }

            #[cfg(not(feature = "gaggle"))]
            {
                return Err(GooseError::FeatureNotEnabled {
                    feature: "gaggle".to_string(), detail: "Load test must be recompiled with `--features gaggle` to start in manager mode.".to_string()
                });
            }
        }
        // Start goose in worker mode.
        else if self.configuration.worker {
            #[cfg(feature = "gaggle")]
            {
                let mut rt = tokio::runtime::Runtime::new().unwrap();
                self = rt.block_on(worker::worker_main(&self));
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
            let mut rt = tokio::runtime::Runtime::new().unwrap();
            self = rt.block_on(self.launch_users(sleep_duration, None))?;
        }

        Ok(self.metrics)
    }

    /// Helper to wrap configured host in Option<> if set.
    fn get_configuration_host(&self) -> Option<String> {
        if self.configuration.host.is_empty() {
            None
        } else {
            Some(self.configuration.host.to_string())
        }
    }

    /// Helper to create CSV-formatted logs.
    fn prepare_csv(raw_request: &GooseRawRequest, header: &mut bool) -> String {
        let body = format!(
            // Put quotes around name, url and final_url as they are strings.
            "{},{:?},\"{}\",\"{}\",\"{}\",{},{},{},{},{},{}",
            raw_request.elapsed,
            raw_request.method,
            raw_request.name,
            raw_request.url,
            raw_request.final_url,
            raw_request.redirected,
            raw_request.response_time,
            raw_request.status_code,
            raw_request.success,
            raw_request.update,
            raw_request.user
        );
        // Concatenate the header before the body one time.
        if *header {
            *header = false;
            format!(
                // No quotes needed in header.
                "{},{},{},{},{},{},{},{},{},{},{}\n",
                "elapsed",
                "method",
                "name",
                "url",
                "final_url",
                "redirected",
                "response_time",
                "status_code",
                "success",
                "update",
                "user"
            ) + &body
        } else {
            body
        }
    }

    // Helper to spawn a logger thread if configured.
    fn setup_logger(
        &self,
    ) -> (
        // A handle to later rejoin the logger thread.
        Option<tokio::task::JoinHandle<()>>,
        // A channel used by GooseClients to send logs.
        Option<mpsc::UnboundedSender<Option<GooseDebug>>>,
    ) {
        // If the logger isn't configured, return immediately.
        if self.configuration.debug_file.is_empty() {
            return (None, None);
        }

        // Create an unbounded channel allowing GooseUser threads to log errors.
        let (all_threads_logger, logger_receiver): (
            mpsc::UnboundedSender<Option<GooseDebug>>,
            mpsc::UnboundedReceiver<Option<GooseDebug>>,
        ) = mpsc::unbounded_channel();
        // Launch a new thread for logging.
        let logger_thread = tokio::spawn(logger::logger_main(
            self.configuration.clone(),
            logger_receiver,
        ));
        (Some(logger_thread), Some(all_threads_logger))
    }

    // Helper to spawn a throttle thread if configured.
    async fn setup_throttle(
        &self,
    ) -> (
        // A channel used by GooseClients to throttle requests.
        Option<mpsc::Sender<bool>>,
        // A channel used by parent to tell throttle the load test is complete.
        Option<mpsc::Sender<bool>>,
    ) {
        // If the throttle isn't configured, return immediately.
        if self.configuration.throttle_requests.is_none() {
            return (None, None);
        }

        // Unwrap is safe here as we exit early if the throttle isn't configured.
        let throttle_requests = self.configuration.throttle_requests.unwrap();

        // Create a bounded channel allowing single-sender multi-receiver to throttle
        // GooseUser threads.
        let (all_threads_throttle, throttle_receiver): (mpsc::Sender<bool>, mpsc::Receiver<bool>) =
            mpsc::channel(throttle_requests);

        // Create a channel allowing the parent to inform the throttle thread when the
        // load test is finished. Even though we only send one message, we can't use a
        // oneshot channel as we don't want to block waiting for a message.
        let (parent_to_throttle_tx, throttle_rx) = mpsc::channel(1);

        // Launch a new thread for throttling, no need to rejoin it.
        let _ = Some(tokio::spawn(throttle::throttle_main(
            throttle_requests,
            throttle_receiver,
            throttle_rx,
        )));

        let mut sender = all_threads_throttle.clone();
        // We start from 1 instead of 0 to intentionally fill all but one slot in the
        // channel to avoid a burst of traffic during startup. The channel then provides
        // an implementation of the leaky bucket algorithm as a queue. Requests have to
        // add a token to the bucket before making a request, and are blocked until this
        // throttle thread "leaks out" a token thereby creating space. More information
        // can be found at: https://en.wikipedia.org/wiki/Leaky_bucket
        for _ in 1..throttle_requests {
            let _ = sender.send(true).await;
        }

        (Some(all_threads_throttle), Some(parent_to_throttle_tx))
    }

    /// Called internally in local-mode and gaggle-mode.
    async fn launch_users(
        mut self,
        sleep_duration: time::Duration,
        socket: Option<Socket>,
    ) -> Result<GooseAttack, GooseError> {
        trace!(
            "launch users: sleep_duration({:?}) socket({:?})",
            sleep_duration,
            socket
        );

        // Initilize per-user states.
        if !self.configuration.worker {
            // First run global test_start_task, if defined.
            match &self.test_start_task {
                Some(t) => {
                    info!("running test_start_task");
                    // Create a one-time-use User to run the test_start_task.
                    let base_url = goose::get_base_url(
                        self.get_configuration_host(),
                        None,
                        self.host.clone(),
                    )?;
                    let user = GooseUser::single(base_url, &self.configuration)?;
                    let function = &t.function;
                    let _ = function(&user).await;
                }
                // No test_start_task defined, nothing to do.
                None => (),
            }
        }

        // If enabled, spawn a logger thread.
        let (logger_thread, all_threads_logger) = self.setup_logger();

        // If enabled, spawn a throttle thread.
        let (all_threads_throttle, parent_to_throttle_tx) = self.setup_throttle().await;

        // Collect user threads in a vector for when we want to stop them later.
        let mut users = vec![];
        // Collect user thread channels in a vector so we can talk to the user threads.
        let mut user_channels = vec![];
        // Create a single channel allowing all Goose child threads to sync metrics back
        // to the parent process.
        let (all_threads_sender, mut metric_receiver): (
            mpsc::UnboundedSender<GooseMetric>,
            mpsc::UnboundedReceiver<GooseMetric>,
        ) = mpsc::unbounded_channel();

        // A new user thread will be spawned at regular intervals. The spawning_user_drift
        // variable tracks how much time is spent on everything else, and is subtracted from
        // the time spent sleeping.
        let mut spawning_user_drift = tokio::time::Instant::now();

        // Spawn users, each with their own weighted task_set.
        for mut thread_user in self.weighted_users.clone() {
            // Stop launching threads if the run_timer has expired, unwrap is safe as we only get here if we started.
            if util::timer_expired(self.started.unwrap(), self.run_time) {
                break;
            }

            // Copy weighted tasks and weighted on start tasks into the user thread.
            thread_user.weighted_tasks = self.task_sets[thread_user.task_sets_index]
                .weighted_tasks
                .clone();
            thread_user.weighted_on_start_tasks = self.task_sets[thread_user.task_sets_index]
                .weighted_on_start_tasks
                .clone();
            thread_user.weighted_on_stop_tasks = self.task_sets[thread_user.task_sets_index]
                .weighted_on_stop_tasks
                .clone();
            // Remember which task group this user is using.
            thread_user.weighted_users_index = self.metrics.users;

            // Create a per-thread channel allowing parent thread to control child threads.
            let (parent_sender, thread_receiver): (
                mpsc::UnboundedSender<GooseUserCommand>,
                mpsc::UnboundedReceiver<GooseUserCommand>,
            ) = mpsc::unbounded_channel();
            user_channels.push(parent_sender);

            if !self.configuration.debug_file.is_empty() {
                // Copy the GooseUser-to-logger sender channel, used by all threads.
                thread_user.logger = Some(all_threads_logger.clone().unwrap());
            } else {
                thread_user.logger = None;
            }

            // Copy the GooseUser-throttle receiver channel, used by all threads.
            match self.configuration.throttle_requests {
                Some(_) => thread_user.throttle = Some(all_threads_throttle.clone().unwrap()),
                None => thread_user.throttle = None,
            }

            // Copy the GooseUser-to-parent sender channel, used by all threads.
            thread_user.channel_to_parent = Some(all_threads_sender.clone());

            // Copy the appropriate task_set into the thread.
            let thread_task_set = self.task_sets[thread_user.task_sets_index].clone();

            // We number threads from 1 as they're human-visible (in the logs), whereas
            // metrics.users starts at 0.
            let thread_number = self.metrics.users + 1;

            let is_worker = self.configuration.worker;

            // Launch a new user.
            let user = tokio::spawn(user::user_main(
                thread_number,
                thread_task_set,
                thread_user,
                thread_receiver,
                is_worker,
            ));

            users.push(user);
            self.metrics.users += 1;
            debug!("sleeping {:?} milliseconds...", sleep_duration);

            spawning_user_drift =
                util::sleep_minus_drift(sleep_duration, spawning_user_drift).await;
        }
        if self.configuration.worker {
            info!(
                "[{}] launched {} users...",
                get_worker_id(),
                self.metrics.users
            );
        } else {
            info!("launched {} users...", self.metrics.users);
        }

        // Only display status codes if enabled.
        self.metrics.display_status_codes = self.configuration.status_codes;

        // Track whether or not we've finished launching users.
        let mut users_launched: bool = false;

        // Catch ctrl-c to allow clean shutdown to display metrics.
        let canceled = Arc::new(AtomicBool::new(false));
        util::setup_ctrlc_handler(&canceled);

        // Determine when to display running metrics (if enabled).
        let mut metrics_timer = time::Instant::now();
        let mut display_running_metrics = false;

        // Prepare an asynchronous buffered file writer for metrics_file (if enabled).
        let mut metrics_file = None;
        if !self.configuration.no_metrics && !self.configuration.metrics_file.is_empty() {
            info!(
                "opening file to log metrics: {}",
                self.configuration.metrics_file
            );
            let file = File::create(&self.configuration.metrics_file).await?;
            metrics_file = Some(BufWriter::new(file));
        }

        // Initialize the optional task metrics.
        self.metrics
            .initialize_task_metrics(&self.task_sets, &self.configuration);

        // If logging metrics to CSV, use this flag to write header; otherwise it's ignored.
        let mut header = true;
        loop {
            // Regularly sync data from user threads first.
            if !self.configuration.no_metrics {
                // Check if we're displaying running metrics.
                if !self.configuration.only_summary
                    && !self.configuration.worker
                    && util::timer_expired(metrics_timer, RUNNING_METRICS_EVERY)
                {
                    metrics_timer = time::Instant::now();
                    display_running_metrics = true;
                }

                // Load messages from user threads until the receiver queue is empty.
                let received_message = self
                    .receive_metrics(&mut metric_receiver, &mut header, &mut metrics_file)
                    .await;

                // As worker, push metrics up to manager.
                if self.configuration.worker && received_message {
                    #[cfg(feature = "gaggle")]
                    {
                        // Push metrics to manager process.
                        if !worker::push_metrics_to_manager(
                            &socket.clone().unwrap(),
                            vec![
                                GaggleMetrics::Requests(self.metrics.requests.clone()),
                                GaggleMetrics::Tasks(self.metrics.tasks.clone()),
                            ],
                            true,
                        ) {
                            // EXIT received, cancel.
                            canceled.store(true, Ordering::SeqCst);
                        }
                        // The manager has all our metrics, reset locally.
                        self.metrics.requests = HashMap::new();
                        self.metrics
                            .initialize_task_metrics(&self.task_sets, &self.configuration);
                    }
                }

                // Flush metrics collected prior to all user threads running
                if !users_launched {
                    users_launched = true;
                    if !self.configuration.no_reset_metrics {
                        self.metrics.duration = self.started.unwrap().elapsed().as_secs() as usize;
                        self.metrics.print_running();

                        if self.metrics.display_metrics {
                            if self.metrics.users < self.users {
                                println!(
                                    "{} of {} users hatched, timer expired, resetting metrics (disable with --no-reset-metrics).\n", self.metrics.users, self.users
                                );
                            } else {
                                println!(
                                    "All {} users hatched, resetting metrics (disable with --no-reset-metrics).\n", self.metrics.users
                                );
                            }
                        }

                        self.metrics.requests = HashMap::new();
                        self.metrics
                            .initialize_task_metrics(&self.task_sets, &self.configuration);
                        // Restart the timer now that all threads are launched.
                        self.started = Some(time::Instant::now());
                    } else if self.metrics.users < self.users {
                        println!(
                            "{} of {} users hatched, timer expired.\n",
                            self.metrics.users, self.users
                        );
                    } else {
                        println!("All {} users hatched.\n", self.metrics.users);
                    }
                }
            }

            if util::timer_expired(self.started.unwrap(), self.run_time)
                || canceled.load(Ordering::SeqCst)
            {
                if self.configuration.worker {
                    info!(
                        "[{}] stopping after {} seconds...",
                        get_worker_id(),
                        self.started.unwrap().elapsed().as_secs()
                    );
                } else {
                    info!(
                        "stopping after {} seconds...",
                        self.started.unwrap().elapsed().as_secs()
                    );
                }
                for (index, send_to_user) in user_channels.iter().enumerate() {
                    match send_to_user.send(GooseUserCommand::EXIT) {
                        Ok(_) => {
                            debug!("telling user {} to exit", index);
                        }
                        Err(e) => {
                            info!("failed to tell user {} to exit: {}", index, e);
                        }
                    }
                }
                if self.configuration.worker {
                    info!("[{}] waiting for users to exit", get_worker_id());
                } else {
                    info!("waiting for users to exit");
                }

                // If throttle is enabled, tell throttle thread the load test is over.
                if let Some(mut tx) = parent_to_throttle_tx {
                    let _ = tx.send(false).await;
                }

                futures::future::join_all(users).await;
                debug!("all users exited");

                if !self.configuration.debug_file.is_empty() {
                    // Tell logger thread to flush and exit.
                    if let Err(e) = all_threads_logger.unwrap().send(None) {
                        warn!("unexpected error telling logger thread to exit: {}", e);
                    };
                    // Wait for logger thread to flush and exit.
                    let _ = tokio::join!(logger_thread.unwrap());
                }

                // If we're printing metrics, collect the final metrics received from users.
                if !self.configuration.no_metrics {
                    let _received_message = self
                        .receive_metrics(&mut metric_receiver, &mut header, &mut metrics_file)
                        .await;
                }

                #[cfg(feature = "gaggle")]
                {
                    // As worker, push metrics up to manager.
                    if self.configuration.worker {
                        worker::push_metrics_to_manager(
                            &socket.clone().unwrap(),
                            vec![
                                GaggleMetrics::Requests(self.metrics.requests.clone()),
                                GaggleMetrics::Tasks(self.metrics.tasks.clone()),
                            ],
                            true,
                        );
                        // No need to reset local metrics, the worker is exiting.
                    }
                }

                // All users are done, exit out of loop for final cleanup.
                break;
            }

            // If enabled, display running metrics after sync
            if display_running_metrics {
                display_running_metrics = false;
                self.metrics.duration = self.started.unwrap().elapsed().as_secs() as usize;
                self.metrics.print_running();
            }

            let one_second = time::Duration::from_secs(1);
            tokio::time::delay_for(one_second).await;
        }
        self.metrics.duration = self.started.unwrap().elapsed().as_secs() as usize;

        if !self.configuration.worker {
            // Run global test_stop_task, if defined.
            match &self.test_stop_task {
                Some(t) => {
                    info!("running test_stop_task");
                    let base_url = goose::get_base_url(
                        self.get_configuration_host(),
                        None,
                        self.host.clone(),
                    )?;
                    // Create a one-time-use user to run the test_stop_task.
                    let user = GooseUser::single(base_url, &self.configuration)?;
                    let function = &t.function;
                    let _ = function(&user).await;
                }
                // No test_stop_task defined, nothing to do.
                None => (),
            }
        }

        // If metrics logging is enabled, flush all metrics before we exit.
        if let Some(file) = metrics_file.as_mut() {
            info!(
                "flushing metrics_file: {}",
                &self.configuration.metrics_file
            );
            let _ = file.flush().await;
        };
        // Only display percentile once the load test is finished.
        self.metrics.display_percentile = true;

        Ok(self)
    }

    async fn receive_metrics(
        &mut self,
        metric_receiver: &mut mpsc::UnboundedReceiver<GooseMetric>,
        header: &mut bool,
        metrics_file: &mut Option<BufWriter<File>>,
    ) -> bool {
        let mut received_message = false;
        let mut message = metric_receiver.try_recv();
        while message.is_ok() {
            received_message = true;
            match message.unwrap() {
                GooseMetric::Request(raw_request) => {
                    // Options should appear above, search for formatted_log.
                    let formatted_log = match self.configuration.metrics_format.as_str() {
                        // Use serde_json to create JSON.
                        "json" => json!(raw_request).to_string(),
                        // Manually create CSV, library doesn't support single-row string conversion.
                        "csv" => GooseAttack::prepare_csv(&raw_request, header),
                        // Raw format is Debug output for GooseRawRequest structure.
                        "raw" => format!("{:?}", raw_request).to_string(),
                        _ => unreachable!(),
                    };
                    if let Some(file) = metrics_file.as_mut() {
                        match file.write(format!("{}\n", formatted_log).as_ref()).await {
                            Ok(_) => (),
                            Err(e) => {
                                warn!(
                                    "failed to write metrics to {}: {}",
                                    &self.configuration.metrics_file, e
                                );
                            }
                        }
                    }
                    let key = format!("{:?} {}", raw_request.method, raw_request.name);
                    let mut merge_request = match self.metrics.requests.get(&key) {
                        Some(m) => m.clone(),
                        None => GooseRequest::new(&raw_request.name, raw_request.method, 0),
                    };
                    // Handle a metrics update.
                    if raw_request.update {
                        if raw_request.success {
                            merge_request.success_count += 1;
                            merge_request.fail_count -= 1;
                        } else {
                            merge_request.success_count -= 1;
                            merge_request.fail_count += 1;
                        }
                    }
                    // Store a new metric.
                    else {
                        merge_request.set_response_time(raw_request.response_time);
                        if self.configuration.status_codes {
                            merge_request.set_status_code(raw_request.status_code);
                        }
                        if raw_request.success {
                            merge_request.success_count += 1;
                        } else {
                            merge_request.fail_count += 1;
                        }
                    }

                    self.metrics.requests.insert(key.to_string(), merge_request);
                }
                GooseMetric::Task(raw_task) => {
                    // Store a new metric.
                    self.metrics.tasks[raw_task.taskset_index][raw_task.task_index]
                        .set_time(raw_task.run_time, raw_task.success);
                }
            }
            message = metric_receiver.try_recv();
        }
        received_message
    }
}

/// Options available when launching a Goose load test.
#[derive(Options, Debug, Clone, Serialize, Deserialize)]
pub struct GooseConfiguration {
    /// Displays this help
    #[options(short = "h")]
    pub help: bool,
    /// Prints version information
    #[options(short = "V")]
    pub version: bool,
    // Add a blank line after this option
    #[options(short = "l", help = "Lists all tasks and exits\n")]
    pub list: bool,

    /// Defines host to load test (ie http://10.21.32.33)
    #[options(short = "H")]
    pub host: String,
    /// Sets concurrent users (default: number of CPUs)
    #[options(short = "u")]
    pub users: Option<usize>,
    /// Sets per-second user hatch rate
    #[options(short = "r", default = "1", meta = "RATE")]
    pub hatch_rate: usize,
    /// Stops after (30s, 20m, 3h, 1h30m, etc)
    #[options(short = "t", meta = "TIME")]
    pub run_time: String,
    /// Sets log level (-g, -gg, etc)
    #[options(short = "g", count)]
    pub log_level: u8,
    /// Sets log file name
    #[options(default = "goose.log", help = "Sets log file name", meta = "NAME")]
    pub log_file: String,
    #[options(
        count,
        short = "v",
        // Add a blank line and then a 'Metrics:' header after this option
        help = "Sets debug level (-v, -vv, etc)\n\nMetrics:"
    )]
    pub verbose: u8,

    /// Only prints final summary metrics
    #[options(no_short)]
    pub only_summary: bool,
    /// Doesn't reset metrics after all users have started
    #[options(no_short)]
    pub no_reset_metrics: bool,
    /// Doesn't track metrics
    #[options(no_short)]
    pub no_metrics: bool,
    /// Doesn't track task metrics
    #[options(no_short)]
    pub no_task_metrics: bool,
    /// Sets metrics log file name
    #[options(short = "m", meta = "NAME")]
    pub metrics_file: String,
    /// Sets metrics log format (csv, json, raw)
    #[options(no_short, default = "json", meta = "FORMAT")]
    pub metrics_format: String,
    /// Sets debug log file name
    #[options(short = "d", meta = "NAME")]
    pub debug_file: String,
    /// Sets debug log format (json, raw)
    #[options(no_short, default = "json", meta = "FORMAT")]
    pub debug_format: String,
    // Add a blank line and then an Advanced: header after this option
    #[options(no_short, help = "Tracks additional status code metrics\n\nAdvanced:")]
    pub status_codes: bool,

    /// Sets maximum requests per second
    #[options(no_short, meta = "VALUE")]
    pub throttle_requests: Option<usize>,
    #[options(
        no_short,
        help = "Follows base_url redirect with subsequent requests\n\nGaggle:"
    )]
    pub sticky_follow: bool,

    /// Enables distributed load test Manager mode
    #[options(no_short)]
    pub manager: bool,
    /// Sets number of Workers to expect
    #[options(no_short, meta = "VALUE")]
    pub expect_workers: u16,
    /// Tells Manager to ignore load test checksum
    #[options(no_short)]
    pub no_hash_check: bool,
    /// Sets host Manager listens on
    #[options(no_short, default = "0.0.0.0", meta = "HOST")]
    pub manager_bind_host: String,
    /// Sets port Manager listens on
    // @TODO: use const for default
    #[options(no_short, default = "5115", meta = "PORT")]
    pub manager_bind_port: u16,
    /// Enables distributed load test Worker mode
    #[options(no_short)]
    pub worker: bool,
    /// Sets host Worker connects to
    #[options(no_short, default = "127.0.0.1", meta = "HOST")]
    pub manager_host: String,
    /// Sets port Worker connects to
    // @TODO: use const for default
    #[options(no_short, default = "5115", meta = "PORT")]
    pub manager_port: u16,
}

/// Returns a sequenced bucket of weighted usize pointers to Goose Tasks
fn weight_tasks(
    task_set: &GooseTaskSet,
) -> (WeightedGooseTasks, WeightedGooseTasks, WeightedGooseTasks) {
    trace!("weight_tasks for {}", task_set.name);

    // A BTreeMap of Vectors allows us to group and sort tasks per sequence value.
    let mut sequenced_tasks: BTreeMap<usize, Vec<GooseTask>> = BTreeMap::new();
    let mut sequenced_on_start_tasks: BTreeMap<usize, Vec<GooseTask>> = BTreeMap::new();
    let mut sequenced_on_stop_tasks: BTreeMap<usize, Vec<GooseTask>> = BTreeMap::new();
    let mut unsequenced_tasks: Vec<GooseTask> = Vec::new();
    let mut unsequenced_on_start_tasks: Vec<GooseTask> = Vec::new();
    let mut unsequenced_on_stop_tasks: Vec<GooseTask> = Vec::new();
    let mut u: usize = 0;
    let mut v: usize;
    // Handle ordering of tasks.
    for task in &task_set.tasks {
        if task.sequence > 0 {
            if task.on_start {
                if let Some(sequence) = sequenced_on_start_tasks.get_mut(&task.sequence) {
                    // This is another task with this order value.
                    sequence.push(task.clone());
                } else {
                    // This is the first task with this order value.
                    sequenced_on_start_tasks.insert(task.sequence, vec![task.clone()]);
                }
            }
            // Allow a task to be both on_start and on_stop.
            if task.on_stop {
                if let Some(sequence) = sequenced_on_stop_tasks.get_mut(&task.sequence) {
                    // This is another task with this order value.
                    sequence.push(task.clone());
                } else {
                    // This is the first task with this order value.
                    sequenced_on_stop_tasks.insert(task.sequence, vec![task.clone()]);
                }
            }
            if !task.on_start && !task.on_stop {
                if let Some(sequence) = sequenced_tasks.get_mut(&task.sequence) {
                    // This is another task with this order value.
                    sequence.push(task.clone());
                } else {
                    // This is the first task with this order value.
                    sequenced_tasks.insert(task.sequence, vec![task.clone()]);
                }
            }
        } else {
            if task.on_start {
                unsequenced_on_start_tasks.push(task.clone());
            }
            if task.on_stop {
                unsequenced_on_stop_tasks.push(task.clone());
            }
            if !task.on_start && !task.on_stop {
                unsequenced_tasks.push(task.clone());
            }
        }
        // Look for lowest common divisor amongst all tasks of any weight.
        if u == 0 {
            u = task.weight;
        } else {
            v = task.weight;
            trace!("calculating greatest common denominator of {} and {}", u, v);
            u = util::gcd(u, v);
            trace!("inner gcd: {}", u);
        }
    }
    // 'u' will always be the greatest common divisor
    debug!("gcd: {}", u);

    // Apply weight to sequenced tasks.
    let mut weighted_tasks: WeightedGooseTasks = Vec::new();
    for (_sequence, tasks) in sequenced_tasks.iter() {
        let mut sequence_weighted_tasks = Vec::new();
        for task in tasks {
            // divide by greatest common divisor so bucket is as small as possible
            let weight = task.weight / u;
            trace!(
                "{}: {} has weight of {} (reduced with gcd to {})",
                task.tasks_index,
                task.name,
                task.weight,
                weight
            );
            let mut tasks = vec![task.tasks_index; weight];
            sequence_weighted_tasks.append(&mut tasks);
        }
        weighted_tasks.push(sequence_weighted_tasks);
    }
    // Apply weight to unsequenced tasks.
    trace!("created weighted_tasks: {:?}", weighted_tasks);
    let mut weighted_unsequenced_tasks = Vec::new();
    for task in unsequenced_tasks {
        // divide by greatest common divisor so bucket is as small as possible
        let weight = task.weight / u;
        trace!(
            "{}: {} has weight of {} (reduced with gcd to {})",
            task.tasks_index,
            task.name,
            task.weight,
            weight
        );
        let mut tasks = vec![task.tasks_index; weight];
        weighted_unsequenced_tasks.append(&mut tasks);
    }
    // Unsequenced tasks come last.
    if !weighted_unsequenced_tasks.is_empty() {
        weighted_tasks.push(weighted_unsequenced_tasks);
    }

    // Apply weight to on_start sequenced tasks.
    let mut weighted_on_start_tasks: WeightedGooseTasks = Vec::new();
    for (_sequence, tasks) in sequenced_on_start_tasks.iter() {
        let mut sequence_on_start_weighted_tasks = Vec::new();
        for task in tasks {
            // divide by greatest common divisor so bucket is as small as possible
            let weight = task.weight / u;
            trace!(
                "{}: {} has weight of {} (reduced with gcd to {})",
                task.tasks_index,
                task.name,
                task.weight,
                weight
            );
            let mut tasks = vec![task.tasks_index; weight];
            sequence_on_start_weighted_tasks.append(&mut tasks);
        }
        weighted_on_start_tasks.push(sequence_on_start_weighted_tasks);
    }
    // Apply weight to unsequenced on_start tasks.
    trace!("created weighted_on_start_tasks: {:?}", weighted_tasks);
    let mut weighted_on_start_unsequenced_tasks = Vec::new();
    for task in unsequenced_on_start_tasks {
        // divide by greatest common divisor so bucket is as small as possible
        let weight = task.weight / u;
        trace!(
            "{}: {} has weight of {} (reduced with gcd to {})",
            task.tasks_index,
            task.name,
            task.weight,
            weight
        );
        let mut tasks = vec![task.tasks_index; weight];
        weighted_on_start_unsequenced_tasks.append(&mut tasks);
    }
    // Unsequenced tasks come lost.
    weighted_on_start_tasks.push(weighted_on_start_unsequenced_tasks);

    // Apply weight to on_stop sequenced tasks.
    let mut weighted_on_stop_tasks: WeightedGooseTasks = Vec::new();
    for (_sequence, tasks) in sequenced_on_stop_tasks.iter() {
        let mut sequence_on_stop_weighted_tasks = Vec::new();
        for task in tasks {
            // divide by greatest common divisor so bucket is as small as possible
            let weight = task.weight / u;
            trace!(
                "{}: {} has weight of {} (reduced with gcd to {})",
                task.tasks_index,
                task.name,
                task.weight,
                weight
            );
            let mut tasks = vec![task.tasks_index; weight];
            sequence_on_stop_weighted_tasks.append(&mut tasks);
        }
        weighted_on_stop_tasks.push(sequence_on_stop_weighted_tasks);
    }
    // Apply weight to unsequenced on_stop tasks.
    trace!("created weighted_on_stop_tasks: {:?}", weighted_tasks);
    let mut weighted_on_stop_unsequenced_tasks = Vec::new();
    for task in unsequenced_on_stop_tasks {
        // divide by greatest common divisor so bucket is as small as possible
        let weight = task.weight / u;
        trace!(
            "{}: {} has weight of {} (reduced with gcd to {})",
            task.tasks_index,
            task.name,
            task.weight,
            weight
        );
        let mut tasks = vec![task.tasks_index; weight];
        weighted_on_stop_unsequenced_tasks.append(&mut tasks);
    }
    // Unsequenced tasks come last.
    weighted_on_stop_tasks.push(weighted_on_stop_unsequenced_tasks);

    (
        weighted_on_start_tasks,
        weighted_tasks,
        weighted_on_stop_tasks,
    )
}

fn is_valid_host(host: &str) -> Result<bool, GooseError> {
    Url::parse(host).map_err(|parse_error| GooseError::InvalidHost {
        host: host.to_string(),
        detail: "Invalid host.".to_string(),
        parse_error,
    })?;
    Ok(true)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn valid_host() {
        assert_eq!(is_valid_host("http://example.com").is_ok(), true);
        assert_eq!(is_valid_host("example.com").is_ok(), false);
        assert_eq!(is_valid_host("http://example.com/").is_ok(), true);
        assert_eq!(is_valid_host("example.com/").is_ok(), false);
        assert_eq!(
            is_valid_host("https://www.example.com/and/with/path").is_ok(),
            true
        );
        assert_eq!(
            is_valid_host("www.example.com/and/with/path").is_ok(),
            false
        );
        assert_eq!(is_valid_host("foo://example.com").is_ok(), true);
        assert_eq!(is_valid_host("file:///path/to/file").is_ok(), true);
        assert_eq!(is_valid_host("/path/to/file").is_ok(), false);
        assert_eq!(is_valid_host("http://").is_ok(), false);
        assert_eq!(is_valid_host("http://foo").is_ok(), true);
        assert_eq!(is_valid_host("http:///example.com").is_ok(), true);
        assert_eq!(is_valid_host("http:// example.com").is_ok(), false);
    }
}
