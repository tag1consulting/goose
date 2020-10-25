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
//! use goose::{task, taskset, GooseAttack, GooseDefault, GooseDefaultType, GooseError};
//! ```
//!
//! Below your `main` function (which currently is the default `Hello, world!`), add
//! one or more load test functions. The names of these functions are arbitrary, but it is
//! recommended you use self-documenting names. Load test functions must be async. Each load
//! test function must accept a reference to a `GooseUser` object and return a
//! `GooseTaskResult`. For example:
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
//! set a timeout on this specific request:
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
//!             // Register the foo task, assigning it a weight of 10.
//!             .register_task(task!(loadtest_foo).set_weight(10)?)
//!             // Register the bar task, assigning it a weight of 2 (so it
//!             // runs 1/5 as often as bar). Apply a task name which shows up
//!             // in metrics.
//!             .register_task(task!(loadtest_bar).set_name("bar").set_weight(2)?)
//!         )
//!         // You could also set a default host here, for example:
//!         //.set_default(GooseDefault::Host, "http://dev.local/")?
//!         .execute()?;
//!
//!     Ok(())
//! }
//!
//! // A task function that loads `/path/to/foo`.
//! async fn loadtest_foo(user: &GooseUser) -> GooseTaskResult {
//!     let _goose = user.get("/path/to/foo").await?;
//!
//!     Ok(())
//! }   
//!
//! // A task function that loads `/path/to/bar`.
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
//! environments, such as local development, staging, and production.
//!
//! ```bash
//! $ cargo run --release
//!    Compiling loadtest v0.1.0 (~/loadtest)
//!     Finished release [optimized] target(s) in 1.52s
//!      Running `target/release/loadtest`
//! Error: InvalidOption { option: "--host", value: "", detail: "A host must be defined via the --host option, the GooseAttack.set_default() function, or the GooseTaskSet.set_host() function (no host defined for WebsiteUser)." }
//! ```
//! Pass in the `-h` flag to see all available run-time options. For now, we'll use a few
//! options to customize our load test.
//!
//! ```bash
//! $ cargo run --release -- --host http://dev.local -t 30s -v
//! ```
//!
//! The first option we specified is `--host`, and in this case tells Goose to run the load test
//! against a VM on my local network. The `-t 30s` option tells Goose to end the load test after 30
//! seconds (for real load tests you'll certainly want to run it longer, you can use `h`, `m`, and
//! `s` to specify hours, minutes and seconds respectively. For example, `-t1h30m` would run the
//! load test for 1 hour 30 minutes). Finally, the `-v` flag tells goose to display INFO and higher
//! level logs to stdout, giving more insight into what is happening. (Additional `-v` flags will
//! result in considerably more debug output, and are not recommended for running actual load tests;
//! they're only useful if you're trying to debug Goose itself.)
//!
//! Running the test results in the following output (broken up to explain it as it goes):
//!
//! ```bash
//!    Finished release [optimized] target(s) in 0.05s
//!     Running `target/release/loadtest --host 'http://dev.local' -t 30s -v`
//! 15:42:23 [ INFO] Output verbosity level: INFO
//! 15:42:23 [ INFO] Logfile verbosity level: WARN
//! ```
//!
//! If we set the `--log-file` flag, Goose will write a log file with WARN and higher level logs
//! as you run the test from (add a `-g` flag to log all INFO and higher level logs).
//!
//! ```bash
//! 15:42:23 [ INFO] concurrent users defaulted to 8 (number of CPUs)
//! 15:42:23 [ INFO] run_time = 30
//! 15:42:23 [ INFO] hatch_rate = 1
//! ```
//!
//! Goose will default to launching 1 user per available CPU core, and will launch them all in
//! one second. You can change how many users are launched with the `-u` option, and you can
//! change how many users are launched per second with the `-r` option. For example, `-u30 -r2`
//! would launch 30 users over 15 seconds (two users per second).
//!
//! ```bash
//! 15:42:23 [ INFO] global host configured: http://dev.local/
//! 15:42:23 [ INFO] initializing user states...
//! 15:42:23 [ INFO] launching user 1 from LoadtestTasks...
//! 15:42:24 [ INFO] launching user 2 from LoadtestTasks...
//! 15:42:25 [ INFO] launching user 3 from LoadtestTasks...
//! 15:42:26 [ INFO] launching user 4 from LoadtestTasks...
//! 15:42:27 [ INFO] launching user 5 from LoadtestTasks...
//! 15:42:28 [ INFO] launching user 6 from LoadtestTasks...
//! 15:42:29 [ INFO] launching user 7 from LoadtestTasks...
//! 15:42:30 [ INFO] launching user 8 from LoadtestTasks...
//! 15:42:31 [ INFO] launched 8 users...
//! 15:42:31 [ INFO] printing running metrics after 8 seconds...
//! ```
//!
//! Each user is launched in its own thread with its own user state. Goose is able to make
//! very efficient use of server resources. By default Goose resets the metrics after all
//! users are launched, but first it outputs the metrics collected while ramping up:
//!
//! ```bash
//! 15:42:31 [ INFO] printing running metrics after 8 seconds...
//!
//!  === PER TASK METRICS ===
//!  ------------------------------------------------------------------------------
//!  Name                     |   # times run |        # fails |   task/s |  fail/s
//!  ------------------------------------------------------------------------------
//!  1: LoadtestTasks         |
//!    1:                     |         2,033 |         0 (0%) |   254.12 |    0.00
//!    2: bar                 |           407 |         0 (0%) |    50.88 |    0.00
//!  -------------------------+---------------+----------------+----------+--------
//!  Aggregated               |         2,440 |         0 (0%) |   305.00 |    0.00
//!  ------------------------------------------------------------------------------
//!  Name                     |    Avg (ms) |        Min |         Max |     Median
//!  ------------------------------------------------------------------------------
//!  1: LoadtestTasks         |
//!    1:                     |       14.23 |          6 |          32 |         14
//!    2: bar                 |       14.13 |          6 |          30 |         14
//!  -------------------------+-------------+------------+-------------+-----------
//!  Aggregated               |       14.21 |          6 |          32 |         14
//!
//!  === PER REQUEST METRICS ===
//!  ------------------------------------------------------------------------------
//!  Name                     |        # reqs |        # fails |    req/s |  fail/s
//!  ------------------------------------------------------------------------------
//!  GET /                    |         2,033 |         0 (0%) |   254.12 |    0.00
//!  GET bar                  |           407 |         0 (0%) |    50.88 |    0.00
//!  -------------------------+---------------+----------------+----------+--------
//!  Aggregated               |         2,440 |         0 (0%) |   305.00 |    0.00
//!  ------------------------------------------------------------------------------
//!  Name                     |    Avg (ms) |        Min |        Max |      Median
//!  ------------------------------------------------------------------------------
//!  GET /                    |       14.18 |          6 |          32 |         14
//!  GET bar                  |       14.08 |          6 |          30 |         14
//!  -------------------------+-------------+------------+-------------+-----------
//!  Aggregated               |       14.16 |          6 |          32 |         14
//!
//! All 8 users hatched, resetting metrics (disable with --no-reset-metrics).
//! ```
//!
//! When printing metrics, by default Goose will display running values approximately
//! every 15 seconds. Running metrics are broken into several tables. First are the
//! per-task metrics which are further split into two sections. The first section shows
//! how many requests have been made, how many of them failed (non-2xx response), and
//! the corresponding per-second rates.
//!
//! This table shows details for all Task Sets and all Tasks defined by your load test,
//! regardless of if they actually run. This can be useful to ensure that you have set
//! up weighting as intended, and that you are simulating enough users. As our first
//! task wasn't named, it just showed up as "1:". Our second task was named, so it shows
//! up as the name we gave it, "bar".
//!
//! ```bash
//! 15:42:46 [ INFO] printing running metrics after 15 seconds...
//!
//!  === PER TASK METRICS ===
//!  ------------------------------------------------------------------------------
//!  Name                     |   # times run |        # fails |   task/s |  fail/s
//!  ------------------------------------------------------------------------------
//!  1: LoadtestTasks         |
//!    1:                     |         4,618 |         0 (0%) |   307.87 |    0.00
//!    2: bar                 |           924 |         0 (0%) |    61.60 |    0.00
//!  -------------------------+---------------+----------------+----------+--------
//!  Aggregated               |         5,542 |         0 (0%) |   369.47 |    0.00
//!  ------------------------------------------------------------------------------
//!  Name                     |    Avg (ms) |        Min |         Max |     Median
//!  ------------------------------------------------------------------------------
//!  1: LoadtestTasks         |
//!    1:                     |       21.17 |          8 |         151 |         19
//!    2: bar                 |       21.62 |          9 |         156 |         19
//!  -------------------------+-------------+------------+-------------+-----------
//!  Aggregated               |       21.24 |          8 |         156 |         19
//! ```
//!
//! The second table breaks down the same metrics by Request instead of by Task. For
//! our simple load test, each Task only makes a single Request, so the metrics are
//! the same. There are two main differences. First, metrics are listed by request
//! type and path or name. The first request shows up as `GET /path/to/foo` as the
//! request was not named. The second request shows up as `GET bar` as the request
//! was named. The times to complete each are slightly smaller as this is only the
//! time to make the request, not the time for Goose to execute the entire task.
//!
//! ```bash
//!  === PER REQUEST METRICS ===
//!  ------------------------------------------------------------------------------
//!  Name                     |        # reqs |        # fails |    req/s |  fail/s
//!  ------------------------------------------------------------------------------
//!  GET /path/to/foo         |         4,618 |         0 (0%) |   307.87 |    0.00
//!  GET bar                  |           924 |         0 (0%) |    61.60 |    0.00
//!  -------------------------+---------------+----------------+----------+--------
//!  Aggregated               |         5,542 |         0 (0%) |   369.47 |    0.00
//!  ------------------------------------------------------------------------------
//!  Name                     |    Avg (ms) |        Min |        Max |      Median
//!  ------------------------------------------------------------------------------
//!  GET /path/to/foo         |       21.13 |          8 |         151 |         19
//!  GET bar                  |       21.58 |          9 |         156 |         19
//!  -------------------------+-------------+------------+-------------+-----------
//!  Aggregated               |       21.20 |          8 |         156 |         19
//! ```
//!
//! Note that Goose respected the per-task weights we set, and `foo` (with a weight of
//! 10) is being loaded five times as often as `bar` (with a weight of 2). On average
//! each page is returning within `21.2` milliseconds. The quickest page response was
//! for `foo` in `8` milliseconds. The slowest page response was for `bar` in `156`
//! milliseconds.
//!
//! ```bash
//! 15:43:02 [ INFO] stopping after 30 seconds...
//! 15:43:02 [ INFO] waiting for users to exit
//! 15:43:02 [ INFO] exiting user 3 from LoadtestTasks...
//! 15:43:02 [ INFO] exiting user 4 from LoadtestTasks...
//! 15:43:02 [ INFO] exiting user 5 from LoadtestTasks...
//! 15:43:02 [ INFO] exiting user 8 from LoadtestTasks...
//! 15:43:02 [ INFO] exiting user 2 from LoadtestTasks...
//! 15:43:02 [ INFO] exiting user 7 from LoadtestTasks...
//! 15:43:02 [ INFO] exiting user 6 from LoadtestTasks...
//! 15:43:02 [ INFO] exiting user 1 from LoadtestTasks...
//! 15:43:02 [ INFO] printing metrics after 30 seconds...
//! ```
//!
//! Our example only runs for 30 seconds, so we only see running metrics once. When
//! the test completes, we get more detail in the final summary. The first two tables
//! are the same as what we saw earlier, however now they include all metrics for the
//! entire length of the load test:
//!
//! ```bash
//!  === PER TASK METRICS ===
//!  ------------------------------------------------------------------------------
//!  Name                     |   # times run |        # fails |   task/s |  fail/s
//!  ------------------------------------------------------------------------------
//!  1: LoadtestTasks         |
//!    1:                     |         9,974 |         0 (0%) |   332.47 |    0.00
//!    2: bar                 |         1,995 |         0 (0%) |    66.50 |    0.00
//!  -------------------------+---------------+----------------+----------+--------
//!  Aggregated               |        11,969 |         0 (0%) |   398.97 |    0.00
//!  ------------------------------------------------------------------------------
//!  Name                     |    Avg (ms) |        Min |         Max |     Median
//!  ------------------------------------------------------------------------------
//!  1: LoadtestTasks         |
//!    1:                     |       19.65 |          8 |         151 |         18
//!    2: bar                 |       19.92 |          9 |         156 |         18
//!  -------------------------+-------------+------------+-------------+-----------
//!  Aggregated               |       19.69 |          8 |         156 |         18
//!
//!  === PER REQUEST METRICS ===
//!  ------------------------------------------------------------------------------
//!  Name                     |        # reqs |        # fails |    req/s |  fail/s
//!  ------------------------------------------------------------------------------
//!  GET /                    |         9,974 |         0 (0%) |   332.47 |    0.00
//!  GET bar                  |         1,995 |         0 (0%) |    66.50 |    0.00
//!  -------------------------+---------------+----------------+----------+--------
//!  Aggregated               |        11,969 |         0 (0%) |   398.97 |    0.00
//!  ------------------------------------------------------------------------------
//!  Name                     |    Avg (ms) |        Min |        Max |      Median
//!  ------------------------------------------------------------------------------
//!  GET /                    |       19.61 |          8 |         151 |         18
//!  GET bar                  |       19.88 |          9 |         156 |         18
//!  -------------------------+-------------+------------+-------------+-----------
//!  Aggregated               |       19.66 |          8 |         156 |         18
//!  ------------------------------------------------------------------------------
//! ```
//!
//! The ratio between `foo` and `bar` remained 5:2 as expected.
//!
//! ```bash
//!  ------------------------------------------------------------------------------
//!  Slowest page load within specified percentile of requests (in ms):
//!  ------------------------------------------------------------------------------
//!  Name                     |    50% |    75% |    98% |    99% |  99.9% | 99.99%
//!  ------------------------------------------------------------------------------
//!  GET /                    |     18 |     21 |     29 |     79 |    140 |    140
//!  GET bar                  |     18 |     21 |     29 |    120 |    150 |    150
//!  -------------------------+--------+--------+--------+--------+--------+-------
//!  Aggregated               |     18 |     21 |     29 |     84 |    140 |    156
//! ```
//!
//! A new table shows additional information, breaking down response-time by
//! percentile. This shows that the slowest page loads only happened in the
//! slowest 1% of page loads, so were an edge case. 98% of the time page loads
//! happened in 29 milliseconds or less.
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
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use url::Url;

use crate::goose::{
    GooseDebug, GooseRawRequest, GooseRequest, GooseTask, GooseTaskSet, GooseUser, GooseUserCommand,
};
use crate::metrics::{GooseMetric, GooseMetrics};
#[cfg(feature = "gaggle")]
use crate::worker::{register_shutdown_pipe_handler, GaggleMetrics};

/// Constant defining how often metrics should be displayed while load test is running.
const RUNNING_METRICS_EVERY: usize = 15;

/// Constant defining Goose's default port when running a Gaggle.
const DEFAULT_PORT: &str = "5115";

// WORKER_ID is only used when running a gaggle (a distributed load test).
lazy_static! {
    static ref WORKER_ID: AtomicUsize = AtomicUsize::new(0);
}

/// Internal representation of a weighted task list.
type WeightedGooseTasks = Vec<Vec<(usize, String)>>;

type DebugLoggerHandle = Option<tokio::task::JoinHandle<()>>;
type DebugLoggerChannel = Option<mpsc::UnboundedSender<Option<GooseDebug>>>;

/// Worker ID to aid in tracing logs when running a Gaggle.
pub fn get_worker_id() -> usize {
    WORKER_ID.load(Ordering::Relaxed)
}

#[cfg(not(feature = "gaggle"))]
#[derive(Debug, Clone)]
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

#[derive(Clone, Debug, PartialEq)]
/// A GooseAttack load test can operate in only one mode.
pub enum GooseMode {
    /// A mode has not yet been assigned.
    Undefined,
    /// A single standalone process performing a load test.
    StandAlone,
    /// The controlling process in a Gaggle distributed load test.
    Manager,
    /// One of one or more working processes in a Gaggle distributed load test.
    Worker,
}

/// Optional default values for Goose run-time options.
#[derive(Clone, Debug, Default)]
pub struct GooseDefaults {
    /// An optional default host to run this load test against.
    host: Option<String>,
    /// An optional default number of users to simulate.
    users: Option<usize>,
    /// An optional default number of clients to start per second.
    hatch_rate: Option<usize>,
    /// An optional default number of seconds for the test to run.
    run_time: Option<usize>,
    /// An optional default log level.
    log_level: Option<u8>,
    /// An optional default for the log file name.
    log_file: Option<String>,
    /// An optional default value for verbosity level.
    verbose: Option<u8>,
    /// An optional default for only printing final summary metrics.
    only_summary: Option<bool>,
    /// An optional default for not resetting metrics after all users started.
    no_reset_metrics: Option<bool>,
    /// An optional default for not tracking metrics.
    no_metrics: Option<bool>,
    /// An optional default for not tracking task metrics.
    no_task_metrics: Option<bool>,
    /// An optional default for the metrics log file name.
    metrics_file: Option<String>,
    /// An optional default for the metrics log file format.
    metrics_format: Option<String>,
    /// An optional default for the debug log file name.
    debug_file: Option<String>,
    /// An optional default for the debug log format.
    debug_format: Option<String>,
    /// An optional default to track additional status code metrics.
    status_codes: Option<bool>,
    /// An optional default maximum requests per second.
    throttle_requests: Option<usize>,
    /// An optional default to follows base_url redirect with subsequent request.
    sticky_follow: Option<bool>,
    /// An optional default to enable Manager mode.
    manager: Option<bool>,
    /// An optional default for number of Workers to expect.
    expect_workers: Option<u16>,
    /// An optional default for Manager to ignore load test checksum.
    no_hash_check: Option<bool>,
    /// An optional default for host Manager listens on.
    manager_bind_host: Option<String>,
    /// An optional default for port Manager listens on.
    manager_bind_port: Option<u16>,
    /// An optional default to enable Worker mode.
    worker: Option<bool>,
    /// An optional default for host Worker connects to.
    manager_host: Option<String>,
    /// An optional default for port Worker connects to.
    manager_port: Option<u16>,
}

#[derive(Debug)]
pub enum GooseDefault {
    /// An optional default host to run this load test against.
    Host,
    /// An optional default number of users to simulate.
    Users,
    /// An optional default number of clients to start per second.
    HatchRate,
    /// An optional default number of seconds for the test to run.
    RunTime,
    /// An optional default log level.
    LogLevel,
    /// An optional default for the log file name.
    LogFile,
    /// An optional default value for verbosity level.
    Verbose,
    /// An optional default for only printing final summary metrics.
    OnlySummary,
    /// An optional default for not resetting metrics after all users started.
    NoResetMetrics,
    /// An optional default for not tracking metrics.
    NoMetrics,
    /// An optional default for not tracking task metrics.
    NoTaskMetrics,
    /// An optional default for the metrics log file name.
    MetricsFile,
    /// An optional default for the metrics log file format.
    MetricsFormat,
    /// An optional default for the debug log file name.
    DebugFile,
    /// An optional default for the debug log format.
    DebugFormat,
    /// An optional default to track additional status code metrics.
    StatusCodes,
    /// An optional default maximum requests per second.
    ThrottleRequests,
    /// An optional default to follows base_url redirect with subsequent request.
    StickyFollow,
    /// An optional default to enable Manager mode.
    Manager,
    /// An optional default for number of Workers to expect.
    ExpectWorkers,
    /// An optional default for Manager to ignore load test checksum.
    NoHashCheck,
    /// An optional default for host Manager listens on.
    ManagerBindHost,
    /// An optional default for port Manager listens on.
    ManagerBindPort,
    /// An optional default to enable Worker mode.
    Worker,
    /// An optional default for host Worker connects to.
    ManagerHost,
    /// An optional default for port Worker connects to.
    ManagerPort,
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
    defaults: GooseDefaults,
    /// Configuration object managed by StructOpt.
    configuration: GooseConfiguration,
    /// Track how long the load test should run.
    run_time: usize,
    /// Which mode this GooseAttack is operating in.
    attack_mode: GooseMode,
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
        Ok(GooseAttack {
            test_start_task: None,
            test_stop_task: None,
            task_sets: Vec::new(),
            weighted_users: Vec::new(),
            defaults: GooseDefaults::default(),
            configuration: GooseConfiguration::parse_args_default_or_exit(),
            run_time: 0,
            attack_mode: GooseMode::Undefined,
            started: None,
            metrics: GooseMetrics::default(),
        })
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
    pub fn initialize_with_config(
        configuration: GooseConfiguration,
    ) -> Result<GooseAttack, GooseError> {
        Ok(GooseAttack {
            test_start_task: None,
            test_stop_task: None,
            task_sets: Vec::new(),
            weighted_users: Vec::new(),
            defaults: GooseDefaults::default(),
            configuration,
            run_time: 0,
            attack_mode: GooseMode::Undefined,
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

        // Set log level based on run-time option or default if set.
        let log_level_value = if self.configuration.log_level > 0 {
            self.configuration.log_level
        } else if let Some(default_log_level) = self.defaults.log_level {
            default_log_level
        } else {
            0
        };
        let log_level = match log_level_value {
            0 => LevelFilter::Warn,
            1 => LevelFilter::Info,
            2 => LevelFilter::Debug,
            _ => LevelFilter::Trace,
        };

        let log_file: Option<PathBuf>;
        // Use --log-file if set.
        if !self.configuration.log_file.is_empty() {
            log_file = Some(PathBuf::from(&self.configuration.log_file));
        }
        // Otherwise use goose_attack.defaults.log_file if set.
        else if let Some(default_log_file) = &self.defaults.log_file {
            log_file = Some(PathBuf::from(default_log_file));
        }
        // Otherwise disable the log.
        else {
            log_file = None;
        }

        if let Some(log_to_file) = log_file {
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
                    std::fs::File::create(&log_to_file).unwrap(),
                ),
            ]) {
                Ok(_) => (),
                Err(e) => {
                    info!("failed to initialize CombinedLogger: {}", e);
                }
            }
            info!("Writing to log file: {}", log_to_file.display());
        } else {
            match CombinedLogger::init(vec![match TermLogger::new(
                debug_level,
                Config::default(),
                TerminalMode::Mixed,
            ) {
                Some(t) => t,
                None => {
                    eprintln!("failed to initialize TermLogger");
                    return;
                }
            }]) {
                Ok(_) => (),
                Err(e) => {
                    info!("failed to initialize CombinedLogger: {}", e);
                }
            }
        }

        info!("Output verbosity level: {}", debug_level);
        info!("Logfile verbosity level: {}", log_level);
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
    /// The GooseUser used to run the `test_start` tasks is not preserved and does not
    /// otherwise affect the subsequent GooseUsers that run the rest of the load test.
    /// For example, if the GooseUser logs in during `test_start`, subsequent GooseUsers
    /// do not retain this session and are therefor not already logged in.
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
                    self.defaults.host.clone(),
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
                // Users are required here so unwrap() is safe.
                if user_count >= self.configuration.users.unwrap() {
                    trace!("created {} weighted_users", user_count);
                    return Ok(weighted_users);
                }
            }
        }
    }

    // Configure which mode this GooseAttack will run in.
    fn set_attack_mode(&mut self) -> Result<(), GooseError> {
        // Determine if Manager is enabled by default.
        let manager_is_default = if let Some(value) = self.defaults.manager {
            value
        } else {
            false
        };

        // Determine if Worker is enabled by default.
        let worker_is_default = if let Some(value) = self.defaults.worker {
            value
        } else {
            false
        };

        // Don't allow Manager and Worker to both be the default.
        if manager_is_default && worker_is_default {
            return Err(GooseError::InvalidOption {
                option: "GooseDefault::Worker".to_string(),
                value: "true".to_string(),
                detail: "The GooseDefault::Worker default can not be set together with the GooseDefault::Manager default"
                    .to_string(),
            });
        }

        // Manager mode if --manager is set, or --worker is not set and Manager is default.
        if self.configuration.manager || (!self.configuration.worker && manager_is_default) {
            self.attack_mode = GooseMode::Manager;
            if self.configuration.worker {
                return Err(GooseError::InvalidOption {
                    option: "--worker".to_string(),
                    value: "true".to_string(),
                    detail: "The --worker flag can not be set together with the --manager flag"
                        .to_string(),
                });
            }

            if self.get_debug_file_path()?.is_some() {
                return Err(GooseError::InvalidOption {
                    option: "--debug-file".to_string(),
                    value: self.configuration.debug_file.clone(),
                    detail:
                        "The --debug-file option can not be set together with the --manager flag."
                            .to_string(),
                });
            }
        }

        // Worker mode if --worker is set, or --manager is not set and Worker is default.
        if self.configuration.worker || (!self.configuration.manager && worker_is_default) {
            self.attack_mode = GooseMode::Worker;
            if self.configuration.manager {
                return Err(GooseError::InvalidOption {
                    option: "--manager".to_string(),
                    value: "true".to_string(),
                    detail: "The --manager flag can not be set together with the --worker flag."
                        .to_string(),
                });
            }

            if !self.configuration.host.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "--host".to_string(),
                    value: self.configuration.host.clone(),
                    detail: "The --host option can not be set together with the --worker flag."
                        .to_string(),
                });
            }
        }

        // Otherwise run in standalone attack mode.
        if self.attack_mode == GooseMode::Undefined {
            self.attack_mode = GooseMode::StandAlone;

            if self.configuration.no_hash_check {
                return Err(GooseError::InvalidOption {
                    option: "--no-hash-check".to_string(),
                    value: self.configuration.no_hash_check.to_string(),
                    detail: "The --no-hash-check flag can not be set without also setting the --manager flag.".to_string(),
                });
            }
        }

        Ok(())
    }

    // Determine how many workers to expect.
    fn set_expect_workers(&mut self) -> Result<(), GooseError> {
        // Track how value gets set so we can return a meaningful error if necessary.
        let mut key = "configuration.expect_workers";

        // Check if --expect-workers was set.
        if self.configuration.expect_workers.is_some() {
            key = "--expect-workers";
        // Otherwise check if a custom default is set.
        } else if let Some(default_expect_workers) = self.defaults.expect_workers {
            if self.attack_mode == GooseMode::Manager {
                key = "set_default(GooseDefault::ExpectWorkers)";

                self.configuration.expect_workers = Some(default_expect_workers);
            }
        }

        if let Some(expect_workers) = self.configuration.expect_workers {
            // Disallow --expect-workers without --manager.
            if self.attack_mode != GooseMode::Manager {
                return Err(GooseError::InvalidOption {
                    option: key.to_string(),
                    value: expect_workers.to_string(),
                    detail: format!(
                        "{} can not be set without also setting the --manager flag.",
                        key
                    ),
                });
            } else {
                // Must expect at least 1 Worker when running as Manager.
                if expect_workers < 1 {
                    return Err(GooseError::InvalidOption {
                        option: key.to_string(),
                        value: expect_workers.to_string(),
                        detail: format!("{} must be set to at least 1.", key),
                    });
                }

                // Must not expect more Workers than Users. Users are required at this point so
                // using unwrap() is safe.
                if expect_workers as usize > self.configuration.users.unwrap() {
                    return Err(GooseError::InvalidOption {
                        option: key.to_string(),
                        value: expect_workers.to_string(),
                        detail: format!(
                            "{} can not be set to a value larger than --users option.",
                            key
                        ),
                    });
                }
            }
        }

        Ok(())
    }

    fn set_gaggle_host_and_port(&mut self) -> Result<(), GooseError> {
        // Configure manager_bind_host and manager_bind_port.
        if self.attack_mode == GooseMode::Manager {
            // Use default if run-time option not set.
            if self.configuration.manager_bind_host.is_empty() {
                self.configuration.manager_bind_host =
                    if let Some(host) = self.defaults.manager_bind_host.clone() {
                        host
                    } else {
                        "0.0.0.0".to_string()
                    }
            }

            // Use default if run-time option not set.
            if self.configuration.manager_bind_port == 0 {
                self.configuration.manager_bind_port =
                    if let Some(port) = self.defaults.manager_bind_port {
                        port
                    } else {
                        DEFAULT_PORT.to_string().parse().unwrap()
                    };
            }
        } else {
            if !self.configuration.manager_bind_host.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "--manager-bind-host".to_string(),
                    value: self.configuration.manager_bind_host.clone(),
                    detail: "The --manager-bind-host option can not be set together with the --worker flag.".to_string(),
                });
            }

            if self.configuration.manager_bind_port != 0 {
                return Err(GooseError::InvalidOption {
                    option: "--manager-bind-port".to_string(),
                    value: self.configuration.manager_bind_port.to_string(),
                    detail: "The --manager-bind-port option can not be set together with the --worker flag.".to_string(),
                });
            }
        }

        // Configure manager_host and manager_port.
        if self.attack_mode == GooseMode::Worker {
            // Use default if run-time option not set.
            if self.configuration.manager_host.is_empty() {
                self.configuration.manager_host =
                    if let Some(host) = self.defaults.manager_host.clone() {
                        host
                    } else {
                        "127.0.0.1".to_string()
                    }
            }

            // Use default if run-time option not set.
            if self.configuration.manager_port == 0 {
                self.configuration.manager_port = if let Some(port) = self.defaults.manager_port {
                    port
                } else {
                    DEFAULT_PORT.to_string().parse().unwrap()
                };
            }
        } else {
            if !self.configuration.manager_host.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "--manager-host".to_string(),
                    value: self.configuration.manager_host.clone(),
                    detail:
                        "The --manager-host option must be set together with the --worker flag."
                            .to_string(),
                });
            }

            if self.configuration.manager_port != 0 {
                return Err(GooseError::InvalidOption {
                    option: "--manager-port".to_string(),
                    value: self.configuration.manager_port.to_string(),
                    detail:
                        "The --manager-port option must be set together with the --worker flag."
                            .to_string(),
                });
            }
        }

        Ok(())
    }

    // Configure how many Goose Users to hatch.
    fn set_users(&mut self) -> Result<(), GooseError> {
        // Track how value gets set so we can return a meaningful error if necessary.
        let mut key = "configuration.users";
        let mut value = 0;

        // Check if --users is set.
        if let Some(users) = self.configuration.users {
            key = "--users";
            value = users;
        // If not, check if a default for users is set.
        } else if let Some(default_users) = self.defaults.users {
            // On Worker hatch_rate comes from the Manager.
            if self.attack_mode == GooseMode::Worker {
                self.configuration.users = None;
            // Otherwise use default.
            } else {
                key = "set_default(GooseDefault::Users)";
                value = default_users;

                self.configuration.users = Some(default_users);
            }
        // If not and if not running on Worker, default to 1.
        } else if self.attack_mode != GooseMode::Worker {
            // This should not be able to fail, but setting up debug in case the number
            // of cpus library returns an invalid number.
            key = "num_cpus::get()";
            value = num_cpus::get();

            info!("concurrent users defaulted to {} (number of CPUs)", value);

            self.configuration.users = Some(value);
        }

        // Perform bounds checking.
        if let Some(users) = self.configuration.users {
            // Setting --users with --worker is not allowed.
            if self.attack_mode == GooseMode::Worker {
                return Err(GooseError::InvalidOption {
                    option: key.to_string(),
                    value: value.to_string(),
                    detail: format!("{} can not be set together with the --worker flag.", key),
                });
            }

            // Setting users to 0 is not allowed.
            if users == 0 {
                return Err(GooseError::InvalidOption {
                    option: key.to_string(),
                    value: "0".to_string(),
                    detail: "The --users option must be set to at least 1.".to_string(),
                });
            }

            // Debug output.
            info!("users = {}", users);
        }

        Ok(())
    }

    // Configure maximum run time if specified, otherwise run until canceled.
    fn set_run_time(&mut self) -> Result<(), GooseError> {
        // Track how value gets set so we can return a meaningful error if necessary.
        let mut key = "configuration.run_time";
        let mut value = 0;

        // Use --run-time if set, don't allow on Worker.
        self.run_time = if !self.configuration.run_time.is_empty() {
            key = "--run-time";
            value = util::parse_timespan(&self.configuration.run_time);
            value
        // Otherwise, use default if set, but not on Worker.
        } else if let Some(default_run_time) = self.defaults.run_time {
            if self.attack_mode == GooseMode::Worker {
                0
            } else {
                key = "set_default(GooseDefault::RunTime)";
                value = default_run_time;
                default_run_time
            }
        }
        // Otherwise the test runs until canceled.
        else {
            0
        };

        if self.run_time > 0 {
            if self.attack_mode == GooseMode::Worker {
                return Err(GooseError::InvalidOption {
                    option: key.to_string(),
                    value: value.to_string(),
                    detail: format!("{} can not be set together with the --worker flag.", key),
                });
            }

            // Debug output.
            info!("run_time = {}", self.run_time);
        }

        Ok(())
    }

    // Configure how quickly to hatch Goose Users.
    fn set_hatch_rate(&mut self) -> Result<(), GooseError> {
        // Track how value gets set so we can return a meaningful error if necessary.
        let mut key = "configuration.hatch_rate";
        let mut value = 0;

        // Check if --hash-rate is set.
        if let Some(hatch_rate) = self.configuration.hatch_rate {
            key = "--hatch_rate";
            value = hatch_rate;
        // If not, check if a default hatch_rate is set.
        } else if let Some(default_hatch_rate) = self.defaults.hatch_rate {
            // On Worker hatch_rate comes from the Manager.
            if self.attack_mode == GooseMode::Worker {
                self.configuration.hatch_rate = None;
            // Otherwise use default.
            } else {
                key = "set_default(GooseDefault::HatchRate)";
                value = default_hatch_rate;
                self.configuration.hatch_rate = Some(default_hatch_rate);
            }
        // If not and if not running on Worker, default to 1.
        } else if self.attack_mode != GooseMode::Worker {
            // This should not be able to fail, but setting up debug in case a later
            // change introduces the potential for failure.
            key = "Goose default";
            value = 1;
            self.configuration.hatch_rate = Some(value);
        }

        // Verbose output.
        if let Some(hatch_rate) = self.configuration.hatch_rate {
            // Setting --hatch-rate with --worker is not allowed.
            if self.attack_mode == GooseMode::Worker {
                return Err(GooseError::InvalidOption {
                    option: key.to_string(),
                    value: value.to_string(),
                    detail: format!("{} can not be set together with the --worker flag.", key),
                });
            }

            // Setting --hatch-rate of 0 is not allowed.
            if hatch_rate == 0 {
                return Err(GooseError::InvalidOption {
                    option: key.to_string(),
                    value: value.to_string(),
                    detail: format!("{} must be set to at least 1.", key),
                });
            }

            // Debug output.
            info!("hatch_rate = {}", hatch_rate);
        }

        Ok(())
    }

    // Configure maximum requests per second if throttle enabled.
    fn set_throttle_requests(&mut self) -> Result<(), GooseError> {
        // Track how value gets set so we can return a meaningful error if necessary.
        let mut key = "configuration.hatch_rate";
        let mut value = 0;

        if self.configuration.throttle_requests > 0 {
            key = "--throttle-requests";
            value = self.configuration.throttle_requests;
        }

        // Use default for throttle_requests if set and not on Worker.
        if self.configuration.throttle_requests == 0 {
            if let Some(default_throttle_requests) = self.defaults.throttle_requests {
                // In Gaggles, throttle_requests is only set on Worker.
                if self.attack_mode != GooseMode::Manager {
                    key = "set_default(GooseDefault::ThrottleRequests)";
                    value = default_throttle_requests;

                    self.configuration.throttle_requests = default_throttle_requests;
                }
            }
        }

        if self.configuration.throttle_requests > 0 {
            // Setting --throttle-requests with --worker is not allowed.
            if self.attack_mode == GooseMode::Manager {
                return Err(GooseError::InvalidOption {
                    option: key.to_string(),
                    value: value.to_string(),
                    detail: format!("{} can not be set together with the --manager flag.", key),
                });
            }

            // Be sure throttle_requests is in allowed range.
            if self.configuration.throttle_requests == 0 {
                return Err(GooseError::InvalidOption {
                    option: key.to_string(),
                    value: value.to_string(),
                    detail: format!("{} must be set to at least 1 request per second.", key),
                });
            } else if self.configuration.throttle_requests > 1_000_000 {
                return Err(GooseError::InvalidOption {
                    option: key.to_string(),
                    value: value.to_string(),
                    detail: format!(
                        "{} can not be set to more than 1,000,000 requests per second.",
                        key
                    ),
                });
            }

            info!(
                "throttle_requests = {}",
                self.configuration.throttle_requests
            );
        }

        Ok(())
    }

    // Determine if no_reset_statics is enabled.
    fn set_no_reset_metrics(&mut self) -> Result<(), GooseError> {
        // Track how value gets set so we can return a meaningful error if necessary.
        let mut key = "configuration.no_reset_metrics";
        let mut value = false;

        if self.configuration.no_reset_metrics {
            key = "--no-reset-metrics";
            value = true;
        // If not otherwise set and not Worker, check if there's a default.
        } else if self.attack_mode != GooseMode::Worker {
            if let Some(default_no_reset_metrics) = self.defaults.no_reset_metrics {
                key = "set_default(GooseDefault::NoResetMetrics)";
                value = default_no_reset_metrics;

                // Optionally set default.
                self.configuration.no_reset_metrics = default_no_reset_metrics;
            }
        }

        // Setting --no-reset-metrics with --worker is not allowed.
        if self.configuration.no_reset_metrics && self.attack_mode == GooseMode::Worker {
            return Err(GooseError::InvalidOption {
                option: key.to_string(),
                value: value.to_string(),
                detail: format!("{} can not be set together with the --worker flag.", key),
            });
        }

        Ok(())
    }

    // Determine if the status_codes flag is enabled.
    fn set_status_codes(&mut self) -> Result<(), GooseError> {
        // Track how value gets set so we can return a meaningful error if necessary.
        let mut key = "configuration.status_codes";
        let mut value = false;

        if self.configuration.status_codes {
            key = "--status-codes";
            value = true;
        // If not otherwise set and not Worker, check if there's a default.
        } else if self.attack_mode != GooseMode::Worker {
            if let Some(default_status_codes) = self.defaults.status_codes {
                key = "set_default(GooseDefault::StatusCodes)";
                value = default_status_codes;

                // Optionally set default.
                self.configuration.status_codes = default_status_codes;
            }
        }

        // Setting --status-codes with --worker is not allowed.
        if self.configuration.status_codes && self.attack_mode == GooseMode::Worker {
            return Err(GooseError::InvalidOption {
                option: key.to_string(),
                value: value.to_string(),
                detail: format!("{} can not be set together with the --worker flag.", key),
            });
        }

        Ok(())
    }

    // Determine if the only_summary flag is enabled.
    fn set_only_summary(&mut self) -> Result<(), GooseError> {
        // Track how value gets set so we can return a meaningful error if necessary.
        let mut key = "configuration.only_summary";
        let mut value = false;

        if self.configuration.only_summary {
            key = "--only-summary";
            value = true;
        // If not otherwise set and not Worker, check if there's a default.
        } else if self.attack_mode != GooseMode::Worker {
            // Optionally set default.
            if let Some(default_only_summary) = self.defaults.only_summary {
                key = "set_default(GooseDefault::OnlySummary)";
                value = default_only_summary;

                self.configuration.only_summary = default_only_summary;
            }
        }

        // Setting --only-summary with --worker is not allowed.
        if self.configuration.only_summary && self.attack_mode == GooseMode::Worker {
            return Err(GooseError::InvalidOption {
                option: key.to_string(),
                value: value.to_string(),
                detail: format!("{} can not be set together with the --worker flag.", key),
            });
        }

        Ok(())
    }

    // Determine if the no_task_metrics flag is enabled.
    fn set_no_task_metrics(&mut self) -> Result<(), GooseError> {
        // Track how value gets set so we can return a meaningful error if necessary.
        let mut key = "configuration.no_task_metrics";
        let mut value = false;

        if self.configuration.no_task_metrics {
            key = "--no-task-metrics";
            value = true;
        // If not otherwise set and not Worker, check if there's a default.
        } else if self.attack_mode != GooseMode::Worker {
            // Optionally set default.
            if let Some(default_no_task_metrics) = self.defaults.no_task_metrics {
                key = "set_default(GooseDefault::NoTaskMetrics)";
                value = default_no_task_metrics;

                self.configuration.no_task_metrics = default_no_task_metrics;
            }
        }

        // Setting --no-task-metrics with --worker is not allowed.
        if self.configuration.no_task_metrics && self.attack_mode == GooseMode::Worker {
            return Err(GooseError::InvalidOption {
                option: key.to_string(),
                value: value.to_string(),
                detail: format!("{} can not be set together with the --worker flag.", key),
            });
        }

        Ok(())
    }

    // Determine if the no_metrics flag is enabled.
    fn set_no_metrics(&mut self) -> Result<(), GooseError> {
        // Track how value gets set so we can return a meaningful error if necessary.
        let mut key = "configuration.no_metrics";
        let mut value = false;

        if self.configuration.no_metrics {
            key = "--no-metrics";
            value = true;
        // If not otherwise set and not Worker, check if there's a default.
        } else if self.attack_mode != GooseMode::Worker {
            // Optionally set default.
            if let Some(default_no_metrics) = self.defaults.no_metrics {
                key = "set_default(GooseDefault::NoMetrics)";
                value = default_no_metrics;

                self.configuration.no_metrics = default_no_metrics;
            }
        }

        // Setting --no-metrics with --worker is not allowed.
        if self.configuration.no_metrics && self.attack_mode == GooseMode::Worker {
            return Err(GooseError::InvalidOption {
                option: key.to_string(),
                value: value.to_string(),
                detail: format!("{} can not be set together with the --worker flag.", key),
            });
        }

        // Don't allow overhead of collecting metrics unless we're printing them.
        if self.configuration.no_metrics {
            if self.configuration.status_codes {
                return Err(GooseError::InvalidOption {
                    option: key.to_string(),
                    value: value.to_string(),
                    detail: format!(
                        "{} can not be set together with the --status-codes flag.",
                        key
                    ),
                });
            }

            // Don't allow overhead of collecting metrics unless we're printing them.
            if self.configuration.only_summary {
                return Err(GooseError::InvalidOption {
                    option: key.to_string(),
                    value: value.to_string(),
                    detail: format!(
                        "{} can not be set together with the --only-summary flag.",
                        key
                    ),
                });
            }

            // There is nothing to log if metrics are disabled.
            if !self.configuration.metrics_file.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: key.to_string(),
                    value: value.to_string(),
                    detail: format!(
                        "{} can not be set together with the --metrics-file option.",
                        key
                    ),
                });
            }
        }

        Ok(())
    }

    // Determine if the sticky_follow flag is enabled.
    fn set_sticky_follow(&mut self) -> Result<(), GooseError> {
        // Track how value gets set so we can return a meaningful error if necessary.
        let mut key = "configuration.sticky_follow";
        let mut value = false;

        if self.configuration.sticky_follow {
            key = "--sticky-follow";
            value = true;
        // If not otherwise set and not Worker, check if there's a default.
        } else if self.attack_mode != GooseMode::Worker {
            // Optionally set default.
            if let Some(default_sticky_follow) = self.defaults.sticky_follow {
                key = "set_default(GooseDefault::StickyFollow)";
                value = default_sticky_follow;

                self.configuration.sticky_follow = default_sticky_follow;
            }
        }

        if self.configuration.sticky_follow && self.attack_mode == GooseMode::Worker {
            return Err(GooseError::InvalidOption {
                option: key.to_string(),
                value: value.to_string(),
                detail: format!("{} can not be set together with the --worker flag.", key),
            });
        }

        Ok(())
    }

    #[cfg(feature = "gaggle")]
    // Determine if no_hash_check flag is enabled.
    fn set_no_hash_check(&mut self) -> Result<(), GooseError> {
        // Track how value gets set so we can return a meaningful error if necessary.
        let mut key = "configuration.no_hash_check";
        let mut value = false;

        if self.configuration.no_hash_check {
            key = "--no-hash-check";
            value = true;
        // If not otherwise set and not Worker, check if there's a default.
        } else if self.attack_mode != GooseMode::Worker {
            // Optionally set default.
            if let Some(default_no_hash_check) = self.defaults.no_hash_check {
                key = "set_default(GooseDefault::NoHashCheck)";
                value = default_no_hash_check;

                self.configuration.no_hash_check = default_no_hash_check;
            }
        }

        if self.configuration.no_hash_check && self.attack_mode == GooseMode::Worker {
            return Err(GooseError::InvalidOption {
                option: key.to_string(),
                value: value.to_string(),
                detail: format!("{} can not be set together with the --worker flag.", key),
            });
        }

        Ok(())
    }

    // If enabled, returns the path of the metrics_file, otherwise returns None.
    fn get_metrics_file_path(&mut self) -> Result<Option<&str>, GooseError> {
        // If metrics are disabled, or running in Manager mode, there is no
        // metrics file, exit immediately.
        if self.configuration.no_metrics || self.attack_mode == GooseMode::Manager {
            return Ok(None);
        }

        // If --metrics-file is set, return it.
        if !self.configuration.metrics_file.is_empty() {
            return Ok(Some(&self.configuration.metrics_file));
        }

        // If GooseDefault::MetricFile is set, return it.
        if let Some(default_metrics_file) = &self.defaults.metrics_file {
            return Ok(Some(default_metrics_file));
        }

        // Otherwise there is no metrics file.
        Ok(None)
    }

    // Configure metrics log format.
    fn set_metrics_format(&mut self) -> Result<(), GooseError> {
        if self.configuration.metrics_format.is_empty() {
            if let Some(default_metrics_format) = &self.defaults.metrics_format {
                self.configuration.metrics_format = default_metrics_format.to_string();
            } else {
                self.configuration.metrics_format = "json".to_string();
            }
        } else {
            // Log format isn't relevant if metrics aren't enabled.
            if self.configuration.no_metrics {
                return Err(GooseError::InvalidOption {
                    option: "--no-metrics".to_string(),
                    value: "true".to_string(),
                    detail: "The --no-metrics flag can not be set together with the --metrics-format option.".to_string(),
                });
            }
            // Log format isn't relevant if log not enabled.
            else if self.get_metrics_file_path()?.is_none() {
                return Err(GooseError::InvalidOption {
                    option: "--metrics-format".to_string(),
                    value: self.configuration.metrics_format.clone(),
                    detail: "The --metrics-file option must be set together with the --metrics-format option.".to_string(),
                });
            }
        }

        let options = vec!["json", "csv", "raw"];
        if !options.contains(&self.configuration.metrics_format.as_str()) {
            return Err(GooseError::InvalidOption {
                option: "--metrics-format".to_string(),
                value: self.configuration.metrics_format.clone(),
                detail: format!(
                    "The --metrics-format option must be set to one of: {}.",
                    options.join(", ")
                ),
            });
        }

        Ok(())
    }

    // If enabled, returns the path of the metrics_file, otherwise returns None.
    fn get_debug_file_path(&self) -> Result<Option<&str>, GooseError> {
        // If running in Manager mode there is no debug file, exit immediately.
        if self.attack_mode == GooseMode::Manager {
            return Ok(None);
        }

        // If --debug-file is set, return it.
        if !self.configuration.debug_file.is_empty() {
            return Ok(Some(&self.configuration.debug_file));
        }

        // If GooseDefault::DebugFile is set, return it.
        if let Some(default_debug_file) = &self.defaults.debug_file {
            return Ok(Some(default_debug_file));
        }

        // Otherwise there is no debug file.
        Ok(None)
    }

    // Configure debug log format.
    fn set_debug_format(&mut self) -> Result<(), GooseError> {
        if self.configuration.debug_format.is_empty() {
            if let Some(default_debug_format) = &self.defaults.debug_format {
                self.configuration.debug_format = default_debug_format.to_string();
            } else {
                self.configuration.debug_format = "json".to_string();
            }
        } else {
            // Log format isn't relevant if log not enabled.
            if self.configuration.debug_file.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "--debug-format".to_string(),
                    value: self.configuration.metrics_format.clone(),
                    detail: "The --debug-file option must be set together with the --debug-format option.".to_string(),
                });
            }
        }

        let options = vec!["json", "raw"];
        if !options.contains(&self.configuration.debug_format.as_str()) {
            return Err(GooseError::InvalidOption {
                option: "--debug-format".to_string(),
                value: self.configuration.debug_format.clone(),
                detail: format!(
                    "The --debug-format option must be set to one of: {}.",
                    options.join(", ")
                ),
            });
        }

        Ok(())
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
        // If version flag is set, display package name and version and exit.
        if self.configuration.version {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }

        // At least one task set is required.
        if self.task_sets.is_empty() {
            return Err(GooseError::NoTaskSets {
                detail: "No task sets are defined.".to_string(),
            });
        }

        // Display task sets and tasks, then exit.
        if self.configuration.list {
            println!("Available tasks:");
            for task_set in self.task_sets {
                println!(" - {} (weight: {})", task_set.name, task_set.weight);
                for task in task_set.tasks {
                    println!("    o {} (weight: {})", task.name, task.weight);
                }
            }
            std::process::exit(0);
        }

        // Initialize logger.
        self.initialize_logger();

        // Configure run mode (StandAlone, Worker, Manager).
        self.set_attack_mode()?;

        // Configure number of users to simulate.
        self.set_users()?;

        // Configure expect_workers if running in Manager attack mode.
        self.set_expect_workers()?;

        // Configure host and ports if running in a Gaggle distributed load test.
        self.set_gaggle_host_and_port()?;

        // Configure how long to run.
        self.set_run_time()?;

        // Configure how many users to hatch per second.
        self.set_hatch_rate()?;

        // Configure the metrics log format.
        self.set_metrics_format()?;

        // Configure the debug log format.
        self.set_debug_format()?;

        // Configure throttle if enabled.
        self.set_throttle_requests()?;

        // Configure status_codes flag.
        self.set_status_codes()?;

        // Configure only_summary flag.
        self.set_only_summary()?;

        // Configure no_reset_metrics flag.
        self.set_no_reset_metrics()?;

        // Configure no_task_metrics flag.
        self.set_no_task_metrics()?;

        // Configure no_metrics flag.
        self.set_no_metrics()?;

        // Configure sticky_follow flag.
        self.set_sticky_follow()?;

        // Configure no_hash_check flag.
        #[cfg(feature = "gaggle")]
        self.set_no_hash_check()?;

        // Confirm there's either a global host, or each task set has a host defined.
        if self.configuration.host.is_empty() {
            for task_set in &self.task_sets {
                match &task_set.host {
                    Some(h) => {
                        if is_valid_host(h).is_ok() {
                            info!("host for {} configured: {}", task_set.name, h);
                        }
                    }
                    None => match &self.defaults.host {
                        Some(h) => {
                            if is_valid_host(h).is_ok() {
                                info!("host for {} configured: {}", task_set.name, h);
                            }
                        }
                        None => {
                            if self.attack_mode != GooseMode::Worker {
                                return Err(GooseError::InvalidOption {
                                    option: "--host".to_string(),
                                    value: "".to_string(),
                                    detail: format!("A host must be defined via the --host option, the GooseAttack.set_default() function, or the GooseTaskSet.set_host() function (no host defined for {}).", task_set.name)
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

        if self.attack_mode != GooseMode::Worker {
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
        let sleep_duration;
        if self.attack_mode != GooseMode::Worker {
            // Hatch rate required to get here, so unwrap() is safe.
            let sleep_float = 1.0 / self.configuration.hatch_rate.unwrap() as f32;
            sleep_duration = time::Duration::from_secs_f32(sleep_float);
        } else {
            sleep_duration = time::Duration::from_secs_f32(0.0);
        }

        // Start goose in manager mode.
        if self.attack_mode == GooseMode::Manager {
            #[cfg(feature = "gaggle")]
            {
                let rt = Runtime::new().unwrap();
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
        else if self.attack_mode == GooseMode::Worker {
            #[cfg(feature = "gaggle")]
            {
                let rt = Runtime::new().unwrap();
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
            let rt = Runtime::new().unwrap();
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
    fn setup_debug_logger(
        &mut self,
    ) -> Result<(DebugLoggerHandle, DebugLoggerChannel), GooseError> {
        // Set configuration from default if available, making it available to
        // GooseUser threads.
        self.configuration.debug_file = if let Some(debug_file) = self.get_debug_file_path()? {
            debug_file.to_string()
        } else {
            "".to_string()
        };
        // If the logger isn't configured, return immediately.
        if self.configuration.debug_file == "" {
            return Ok((None, None));
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
        Ok((Some(logger_thread), Some(all_threads_logger)))
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
        // If the throttle isn't enabled, return immediately.
        if self.configuration.throttle_requests == 0 {
            return (None, None);
        }

        // Create a bounded channel allowing single-sender multi-receiver to throttle
        // GooseUser threads.
        let (all_threads_throttle, throttle_receiver): (mpsc::Sender<bool>, mpsc::Receiver<bool>) =
            mpsc::channel(self.configuration.throttle_requests);

        // Create a channel allowing the parent to inform the throttle thread when the
        // load test is finished. Even though we only send one message, we can't use a
        // oneshot channel as we don't want to block waiting for a message.
        let (parent_to_throttle_tx, throttle_rx) = mpsc::channel(1);

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
            let _ = sender.send(true).await;
        }

        (Some(all_threads_throttle), Some(parent_to_throttle_tx))
    }

    // Prepare an asynchronous buffered file writer for metrics_file (if enabled).
    async fn prepare_metrics_file(&mut self) -> Result<Option<BufWriter<File>>, GooseError> {
        if let Some(metrics_file_path) = self.get_metrics_file_path()? {
            Ok(Some(BufWriter::new(
                File::create(&metrics_file_path).await?,
            )))
        } else {
            Ok(None)
        }
    }

    // Invoke test_start tasks if existing.
    async fn run_test_start(&self) -> Result<(), GooseError> {
        // Initialize per-user states.
        if self.attack_mode != GooseMode::Worker {
            // First run global test_start_task, if defined.
            match &self.test_start_task {
                Some(t) => {
                    info!("running test_start_task");
                    // Create a one-time-use User to run the test_start_task.
                    let base_url = goose::get_base_url(
                        self.get_configuration_host(),
                        None,
                        self.defaults.host.clone(),
                    )?;
                    let user = GooseUser::single(base_url, &self.configuration)?;
                    let function = &t.function;
                    let _ = function(&user).await;
                }
                // No test_start_task defined, nothing to do.
                None => (),
            }
        }

        Ok(())
    }

    // Invoke test_stop tasks if existing.
    async fn run_test_stop(&self) -> Result<(), GooseError> {
        // Initialize per-user states.
        if self.attack_mode != GooseMode::Worker {
            // First run global test_stop_task, if defined.
            match &self.test_stop_task {
                Some(t) => {
                    info!("running test_stop_task");
                    // Create a one-time-use User to run the test_stop_task.
                    let base_url = goose::get_base_url(
                        self.get_configuration_host(),
                        None,
                        self.defaults.host.clone(),
                    )?;
                    let user = GooseUser::single(base_url, &self.configuration)?;
                    let function = &t.function;
                    let _ = function(&user).await;
                }
                // No test_stop_task defined, nothing to do.
                None => (),
            }
        }

        Ok(())
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

        // Run any configured test_start() functions.
        self.run_test_start().await?;

        // If enabled, spawn a logger thread.
        let (logger_thread, all_threads_logger) = self.setup_debug_logger()?;

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

            if self.get_debug_file_path()?.is_some() {
                // Copy the GooseUser-to-logger sender channel, used by all threads.
                thread_user.logger = Some(all_threads_logger.clone().unwrap());
            } else {
                thread_user.logger = None;
            }

            // Copy the GooseUser-throttle receiver channel, used by all threads.
            thread_user.throttle = if self.configuration.throttle_requests > 0 {
                Some(all_threads_throttle.clone().unwrap())
            } else {
                None
            };

            // Copy the GooseUser-to-parent sender channel, used by all threads.
            thread_user.channel_to_parent = Some(all_threads_sender.clone());

            // Copy the appropriate task_set into the thread.
            let thread_task_set = self.task_sets[thread_user.task_sets_index].clone();

            // We number threads from 1 as they're human-visible (in the logs), whereas
            // metrics.users starts at 0.
            let thread_number = self.metrics.users + 1;

            let is_worker = self.attack_mode == GooseMode::Worker;

            // If running on Worker, use Worker configuration in GooseUser.
            if is_worker {
                thread_user.config = self.configuration.clone();
            }

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
        if self.attack_mode == GooseMode::Worker {
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

        let mut metrics_file = self.prepare_metrics_file().await?;

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
                    && self.attack_mode != GooseMode::Worker
                    && util::timer_expired(metrics_timer, RUNNING_METRICS_EVERY)
                {
                    metrics_timer = time::Instant::now();
                    display_running_metrics = true;
                }

                // Load messages from user threads until the receiver queue is empty.
                let received_message = self
                    .receive_metrics(&mut metric_receiver, &mut header, &mut metrics_file)
                    .await?;

                // As worker, push metrics up to manager.
                if self.attack_mode == GooseMode::Worker && received_message {
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
                    let users = self.configuration.users.clone().unwrap();
                    if !self.configuration.no_reset_metrics {
                        self.metrics.duration = self.started.unwrap().elapsed().as_secs() as usize;
                        self.metrics.print_running();

                        if self.metrics.display_metrics {
                            // Users is required here so unwrap() is safe.
                            if self.metrics.users < users {
                                println!(
                                    "{} of {} users hatched, timer expired, resetting metrics (disable with --no-reset-metrics).\n", self.metrics.users, users
                                );
                            } else {
                                println!(
                                    "All {} users hatched, resetting metrics (disable with --no-reset-metrics).\n", users
                                );
                            }
                        }

                        self.metrics.requests = HashMap::new();
                        self.metrics
                            .initialize_task_metrics(&self.task_sets, &self.configuration);
                        // Restart the timer now that all threads are launched.
                        self.started = Some(time::Instant::now());
                    } else if self.metrics.users < users {
                        println!(
                            "{} of {} users hatched, timer expired.\n",
                            self.metrics.users, users
                        );
                    } else {
                        println!("All {} users hatched.\n", self.metrics.users);
                    }
                }
            }

            if util::timer_expired(self.started.unwrap(), self.run_time)
                || canceled.load(Ordering::SeqCst)
            {
                if self.attack_mode == GooseMode::Worker {
                    info!(
                        "[{}] stopping after {} seconds...",
                        get_worker_id(),
                        self.started.unwrap().elapsed().as_secs()
                    );

                    // Load test is shutting down, update pipe handler so there is no panic
                    // when the Manager goes away.
                    #[cfg(feature = "gaggle")]
                    {
                        let manager = socket.clone().unwrap();
                        register_shutdown_pipe_handler(&manager);
                    }
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
                if self.attack_mode == GooseMode::Worker {
                    info!("[{}] waiting for users to exit", get_worker_id());
                } else {
                    info!("waiting for users to exit");
                }

                // If throttle is enabled, tell throttle thread the load test is over.
                if let Some(tx) = parent_to_throttle_tx {
                    let _ = tx.send(false).await;
                }

                futures::future::join_all(users).await;
                debug!("all users exited");

                if self.get_debug_file_path()?.is_some() {
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
                        .await?;
                }

                #[cfg(feature = "gaggle")]
                {
                    // As worker, push metrics up to manager.
                    if self.attack_mode == GooseMode::Worker {
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
            tokio::time::sleep(one_second).await;
        }
        self.metrics.duration = self.started.unwrap().elapsed().as_secs() as usize;

        // Run any configured test_stop() functions.
        self.run_test_stop().await?;

        // If metrics logging is enabled, flush all metrics before we exit.
        if let Some(file) = metrics_file.as_mut() {
            info!(
                "flushing metrics_file: {}",
                // Unwrap is safe as we can't get here unless a metrics file path
                // is defined.
                self.get_metrics_file_path()?.unwrap()
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
    ) -> Result<bool, GooseError> {
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
                        "raw" => format!("{:?}", raw_request),
                        _ => unreachable!(),
                    };
                    if let Some(file) = metrics_file.as_mut() {
                        match file.write(format!("{}\n", formatted_log).as_ref()).await {
                            Ok(_) => (),
                            Err(e) => {
                                warn!(
                                    "failed to write metrics to {}: {}",
                                    // Unwrap is safe as we can't get here unless a metrics file path
                                    // is defined.
                                    self.get_metrics_file_path()?.unwrap(),
                                    e
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
        Ok(received_message)
    }
}

/// All run-time options can optionally be configured with custom defaults. For
/// example, you can optionally configure a default host for the load test. This is
/// used if no per-GooseTaskSet host is defined, no `--host` CLI option is
/// configured, and if the GooseTask itself doesn't hard-code the host in the base
/// url of its request. In that case, this host is added to all requests.
///
/// For example, a load test could be configured to default to running against a local
/// development container, and the `--host` option could be used to override the host
/// value to run the load test against the production environment.
///
/// # Example
/// ```rust,no_run
///     use goose::prelude::*;
///
/// fn main() -> Result<(), GooseError> {
///     GooseAttack::initialize()?
///         .set_default(GooseDefault::Host, "local.dev")?;
///
///     Ok(())
/// }
/// ```
///
/// The following run-time options can be configured with a custom default using a
/// borrowed string slice (`&str`):
///  - GooseDefault::Host
///  - GooseDefault::LogFile
///  - GooseDefault::MetricsFile
///  - GooseDefault::MetricsFormat
///  - GooseDefault::DebugFile
///  - GooseDefault::DebugFormat
///  - GooseDefault::ManagerBindHost
///  - GooseDefault::ManagerHost
///
/// The following run-time options can be configured with a custom default using a
/// `usize` integer:
///  - GooseDefault::Users
///  - GooseDefault::HatchRate
///  - GooseDefault::RunTime
///  - GooseDefault::LogLevel
///  - GooseDefault::Verbose
///  - GooseDefault::ThrottleRequests
///  - GooseDefault::ExpectWorkers
///  - GooseDefault::ManagerBindPort
///  - GooseDefault::ManagerPort
///
/// The following run-time flags can be configured with a custom default using a
/// `bool` (and otherwise default to `false`).
///  - GooseDefault::OnlySummary
///  - GooseDefault::NoResetMetrics
///  - GooseDefault::NoMetrics
///  - GooseDefault::NoTaskMetrics
///  - GooseDefault::StatusCodes
///  - GooseDefault::StickyFollow
///  - GooseDefault::Manager
///  - GooseDefault::NoHashCheck
///  - GooseDefault::Worker
///
/// # Another Example
/// ```rust,no_run
///     use goose::prelude::*;
///
/// fn main() -> Result<(), GooseError> {
///     GooseAttack::initialize()?
///         .set_default(GooseDefault::OnlySummary, true)?
///         .set_default(GooseDefault::Verbose, 1)?
///         .set_default(GooseDefault::MetricsFile, "goose-metrics.log")?;
///
///     Ok(())
/// }
/// ```
pub trait GooseDefaultType<T> {
    fn set_default(self, key: GooseDefault, value: T) -> Result<Box<Self>, GooseError>;
}
impl GooseDefaultType<&str> for GooseAttack {
    fn set_default(mut self, key: GooseDefault, value: &str) -> Result<Box<Self>, GooseError> {
        match key {
            // Set valid defaults.
            GooseDefault::Host => self.defaults.host = Some(value.to_string()),
            GooseDefault::LogFile => self.defaults.log_file = Some(value.to_string()),
            GooseDefault::MetricsFile => self.defaults.metrics_file = Some(value.to_string()),
            GooseDefault::MetricsFormat => self.defaults.metrics_format = Some(value.to_string()),
            GooseDefault::DebugFile => self.defaults.debug_file = Some(value.to_string()),
            GooseDefault::DebugFormat => self.defaults.debug_format = Some(value.to_string()),
            GooseDefault::ManagerBindHost => {
                self.defaults.manager_bind_host = Some(value.to_string())
            }
            GooseDefault::ManagerHost => self.defaults.manager_host = Some(value.to_string()),
            // Otherwise display a helpful and explicit error.
            GooseDefault::Users
            | GooseDefault::HatchRate
            | GooseDefault::RunTime
            | GooseDefault::LogLevel
            | GooseDefault::Verbose
            | GooseDefault::ThrottleRequests
            | GooseDefault::ExpectWorkers
            | GooseDefault::ManagerBindPort
            | GooseDefault::ManagerPort => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {}) expected usize value, received &str",
                        key, value
                    ),
                });
            }
            GooseDefault::OnlySummary
            | GooseDefault::NoResetMetrics
            | GooseDefault::NoMetrics
            | GooseDefault::NoTaskMetrics
            | GooseDefault::StatusCodes
            | GooseDefault::StickyFollow
            | GooseDefault::Manager
            | GooseDefault::NoHashCheck
            | GooseDefault::Worker => panic!(format!(
                "set_default(GooseDefault::{:?}, {}) expected bool value, received &str",
                key, value
            )),
        }
        Ok(Box::new(self))
    }
}
impl GooseDefaultType<usize> for GooseAttack {
    fn set_default(mut self, key: GooseDefault, value: usize) -> Result<Box<Self>, GooseError> {
        match key {
            GooseDefault::Users => self.defaults.users = Some(value),
            GooseDefault::HatchRate => self.defaults.hatch_rate = Some(value),
            GooseDefault::RunTime => self.defaults.run_time = Some(value),
            GooseDefault::LogLevel => self.defaults.log_level = Some(value as u8),
            GooseDefault::Verbose => self.defaults.verbose = Some(value as u8),
            GooseDefault::ThrottleRequests => self.defaults.throttle_requests = Some(value),
            GooseDefault::ExpectWorkers => self.defaults.expect_workers = Some(value as u16),
            GooseDefault::ManagerBindPort => self.defaults.manager_bind_port = Some(value as u16),
            GooseDefault::ManagerPort => self.defaults.manager_port = Some(value as u16),
            // Otherwise display a helpful and explicit error.
            GooseDefault::Host
            | GooseDefault::LogFile
            | GooseDefault::MetricsFile
            | GooseDefault::MetricsFormat
            | GooseDefault::DebugFile
            | GooseDefault::DebugFormat
            | GooseDefault::ManagerBindHost
            | GooseDefault::ManagerHost => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: format!("{}", value),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {}) expected &str value, received usize",
                        key, value
                    ),
                })
            }
            GooseDefault::OnlySummary
            | GooseDefault::NoResetMetrics
            | GooseDefault::NoMetrics
            | GooseDefault::NoTaskMetrics
            | GooseDefault::StatusCodes
            | GooseDefault::StickyFollow
            | GooseDefault::Manager
            | GooseDefault::NoHashCheck
            | GooseDefault::Worker => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: format!("{}", value),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {}) expected bool value, received usize",
                        key, value
                    ),
                })
            }
        }
        Ok(Box::new(self))
    }
}
impl GooseDefaultType<bool> for GooseAttack {
    fn set_default(mut self, key: GooseDefault, value: bool) -> Result<Box<Self>, GooseError> {
        match key {
            GooseDefault::OnlySummary => self.defaults.only_summary = Some(value),
            GooseDefault::NoResetMetrics => self.defaults.no_reset_metrics = Some(value),
            GooseDefault::NoMetrics => self.defaults.no_metrics = Some(value),
            GooseDefault::NoTaskMetrics => self.defaults.no_task_metrics = Some(value),
            GooseDefault::StatusCodes => self.defaults.status_codes = Some(value),
            GooseDefault::StickyFollow => self.defaults.sticky_follow = Some(value),
            GooseDefault::Manager => self.defaults.manager = Some(value),
            GooseDefault::NoHashCheck => self.defaults.no_hash_check = Some(value),
            GooseDefault::Worker => self.defaults.worker = Some(value),
            // Otherwise display a helpful and explicit error.
            GooseDefault::Host
            | GooseDefault::LogFile
            | GooseDefault::MetricsFile
            | GooseDefault::MetricsFormat
            | GooseDefault::DebugFile
            | GooseDefault::DebugFormat
            | GooseDefault::ManagerBindHost
            | GooseDefault::ManagerHost => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: format!("{}", value),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {}) expected &str value, received bool",
                        key, value
                    ),
                })
            }
            GooseDefault::Users
            | GooseDefault::HatchRate
            | GooseDefault::RunTime
            | GooseDefault::LogLevel
            | GooseDefault::Verbose
            | GooseDefault::ThrottleRequests
            | GooseDefault::ExpectWorkers
            | GooseDefault::ManagerBindPort
            | GooseDefault::ManagerPort => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: format!("{}", value),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {}) expected usize value, received bool",
                        key, value
                    ),
                })
            }
        }
        Ok(Box::new(self))
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
    /// Sets per-second user hatch rate (default: 1)
    #[options(short = "r", meta = "RATE")]
    pub hatch_rate: Option<usize>,
    /// Stops after (30s, 20m, 3h, 1h30m, etc)
    #[options(short = "t", meta = "TIME")]
    pub run_time: String,
    /// Sets log level (-g, -gg, etc)
    #[options(short = "g", count)]
    pub log_level: u8,
    /// Enables log file and sets name
    #[options(meta = "NAME")]
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
    #[options(no_short, meta = "FORMAT")]
    pub metrics_format: String,
    /// Sets debug log file name
    #[options(short = "d", meta = "NAME")]
    pub debug_file: String,
    /// Sets debug log format (json, raw)
    #[options(no_short, meta = "FORMAT")]
    pub debug_format: String,
    // Add a blank line and then an Advanced: header after this option
    #[options(no_short, help = "Tracks additional status code metrics\n\nAdvanced:")]
    pub status_codes: bool,

    /// Sets maximum requests per second
    #[options(no_short, meta = "VALUE")]
    pub throttle_requests: usize,
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
    pub expect_workers: Option<u16>,
    /// Tells Manager to ignore load test checksum
    #[options(no_short)]
    pub no_hash_check: bool,
    /// Sets host Manager listens on (default: 0.0.0.0)
    #[options(no_short, meta = "HOST")]
    pub manager_bind_host: String,
    /// Sets port Manager listens on (default: 5115)
    #[options(no_short, meta = "PORT")]
    pub manager_bind_port: u16,
    /// Enables distributed load test Worker mode
    #[options(no_short)]
    pub worker: bool,
    /// Sets host Worker connects to (default: 127.0.0.1)
    #[options(no_short, meta = "HOST")]
    pub manager_host: String,
    /// Sets port Worker connects to (default: 5115)
    #[options(no_short, meta = "PORT")]
    pub manager_port: u16,
}

/// Returns sequenced buckets of weighted usize pointers to and names of Goose Tasks
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

    // First apply weights to sequenced tasks.
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
            // Weighted list of tuples holding the id and name of the task.
            let mut tasks = vec![(task.tasks_index, task.name.to_string()); weight];
            sequence_weighted_tasks.append(&mut tasks);
        }
        // Add in vectors grouped by sequence value, ordered lowest to highest value.
        weighted_tasks.push(sequence_weighted_tasks);
    }

    // Next apply weights to unsequenced tasks.
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
        // Weighted list of tuples holding the id and name of the task.
        let mut tasks = vec![(task.tasks_index, task.name.to_string()); weight];
        weighted_unsequenced_tasks.append(&mut tasks);
    }
    // Add final vector of unsequenced tasks last.
    if !weighted_unsequenced_tasks.is_empty() {
        weighted_tasks.push(weighted_unsequenced_tasks);
    }

    // Next apply weights to on_start sequenced tasks.
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
            // Weighted list of tuples holding the id and name of the task.
            let mut tasks = vec![(task.tasks_index, task.name.to_string()); weight];
            sequence_on_start_weighted_tasks.append(&mut tasks);
        }
        // Add in vectors grouped by sequence value, ordered lowest to highest value.
        weighted_on_start_tasks.push(sequence_on_start_weighted_tasks);
    }

    // Next apply weights to unsequenced on_start tasks.
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
        // Weighted list of tuples holding the id and name of the task.
        let mut tasks = vec![(task.tasks_index, task.name.to_string()); weight];
        weighted_on_start_unsequenced_tasks.append(&mut tasks);
    }
    // Add final vector of unsequenced on_start tasks last.
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
            // Weighted list of tuples holding the id and name of the task.
            let mut tasks = vec![(task.tasks_index, task.name.to_string()); weight];
            sequence_on_stop_weighted_tasks.append(&mut tasks);
        }
        // Add in vectors grouped by sequence value, ordered lowest to highest value.
        weighted_on_stop_tasks.push(sequence_on_stop_weighted_tasks);
    }

    // Finally apply weight to unsequenced on_stop tasks.
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
        // Weighted list of tuples holding the id and name of the task.
        let mut tasks = vec![(task.tasks_index, task.name.to_string()); weight];
        weighted_on_stop_unsequenced_tasks.append(&mut tasks);
    }
    // Add final vector of unsequenced on_stop tasks last.
    weighted_on_stop_tasks.push(weighted_on_stop_unsequenced_tasks);

    // Return sequenced buckets of weighted usize pointers to and names of Goose Tasks
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

    #[test]
    fn set_defaults() {
        let host = "http://example.com/".to_string();
        let users: usize = 10;
        let run_time: usize = 10;
        let hatch_rate: usize = 2;
        let log_level: usize = 1;
        let log_file = "custom-goose.log".to_string();
        let verbose: usize = 0;
        let metrics_file = "custom-goose-metrics.log".to_string();
        let metrics_format = "raw".to_string();
        let debug_file = "custom-goose-debug.log".to_string();
        let debug_format = "raw".to_string();
        let throttle_requests: usize = 25;
        let expect_workers: usize = 5;
        let manager_bind_host = "127.0.0.1".to_string();
        let manager_bind_port: usize = 1221;
        let manager_host = "127.0.0.1".to_string();
        let manager_port: usize = 1221;

        let goose_attack = GooseAttack::initialize()
            .unwrap()
            .set_default(GooseDefault::Host, host.as_str())
            .unwrap()
            .set_default(GooseDefault::Users, users)
            .unwrap()
            .set_default(GooseDefault::RunTime, run_time)
            .unwrap()
            .set_default(GooseDefault::HatchRate, hatch_rate)
            .unwrap()
            .set_default(GooseDefault::LogLevel, log_level)
            .unwrap()
            .set_default(GooseDefault::LogFile, log_file.as_str())
            .unwrap()
            .set_default(GooseDefault::Verbose, verbose)
            .unwrap()
            .set_default(GooseDefault::OnlySummary, true)
            .unwrap()
            .set_default(GooseDefault::NoResetMetrics, true)
            .unwrap()
            .set_default(GooseDefault::NoMetrics, true)
            .unwrap()
            .set_default(GooseDefault::NoTaskMetrics, true)
            .unwrap()
            .set_default(GooseDefault::MetricsFile, metrics_file.as_str())
            .unwrap()
            .set_default(GooseDefault::MetricsFormat, metrics_format.as_str())
            .unwrap()
            .set_default(GooseDefault::DebugFile, debug_file.as_str())
            .unwrap()
            .set_default(GooseDefault::DebugFormat, debug_format.as_str())
            .unwrap()
            .set_default(GooseDefault::StatusCodes, true)
            .unwrap()
            .set_default(GooseDefault::ThrottleRequests, throttle_requests)
            .unwrap()
            .set_default(GooseDefault::StickyFollow, true)
            .unwrap()
            .set_default(GooseDefault::Manager, true)
            .unwrap()
            .set_default(GooseDefault::ExpectWorkers, expect_workers)
            .unwrap()
            .set_default(GooseDefault::NoHashCheck, true)
            .unwrap()
            .set_default(GooseDefault::ManagerBindHost, manager_bind_host.as_str())
            .unwrap()
            .set_default(GooseDefault::ManagerBindPort, manager_bind_port)
            .unwrap()
            .set_default(GooseDefault::Worker, true)
            .unwrap()
            .set_default(GooseDefault::ManagerHost, manager_host.as_str())
            .unwrap()
            .set_default(GooseDefault::ManagerPort, manager_port)
            .unwrap();

        assert!(goose_attack.defaults.host == Some(host));
        assert!(goose_attack.defaults.users == Some(users));
        assert!(goose_attack.defaults.run_time == Some(run_time));
        assert!(goose_attack.defaults.hatch_rate == Some(hatch_rate));
        assert!(goose_attack.defaults.log_level == Some(log_level as u8));
        assert!(goose_attack.defaults.log_file == Some(log_file));
        assert!(goose_attack.defaults.verbose == Some(verbose as u8));
        assert!(goose_attack.defaults.only_summary == Some(true));
        assert!(goose_attack.defaults.no_reset_metrics == Some(true));
        assert!(goose_attack.defaults.no_metrics == Some(true));
        assert!(goose_attack.defaults.no_task_metrics == Some(true));
        assert!(goose_attack.defaults.metrics_file == Some(metrics_file));
        assert!(goose_attack.defaults.metrics_format == Some(metrics_format));
        assert!(goose_attack.defaults.debug_file == Some(debug_file));
        assert!(goose_attack.defaults.debug_format == Some(debug_format));
        assert!(goose_attack.defaults.status_codes == Some(true));
        assert!(goose_attack.defaults.throttle_requests == Some(throttle_requests));
        assert!(goose_attack.defaults.sticky_follow == Some(true));
        assert!(goose_attack.defaults.manager == Some(true));
        assert!(goose_attack.defaults.expect_workers == Some(expect_workers as u16));
        assert!(goose_attack.defaults.no_hash_check == Some(true));
        assert!(goose_attack.defaults.manager_bind_host == Some(manager_bind_host));
        assert!(goose_attack.defaults.manager_bind_port == Some(manager_bind_port as u16));
        assert!(goose_attack.defaults.worker == Some(true));
        assert!(goose_attack.defaults.manager_host == Some(manager_host));
        assert!(goose_attack.defaults.manager_port == Some(manager_port as u16));
    }
}
