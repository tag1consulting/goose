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
//! ## Documentation
//!
//! - [README](https://github.com/tag1consulting/goose/blob/main/README.md)
//! - [Developer documentation](https://docs.rs/goose/)
//! - [Blogs and more](https://tag1.com/goose/)
//!   - [Goose vs Locust and jMeter](https://www.tag1consulting.com/blog/jmeter-vs-locust-vs-goose)
//!   - [Real-life load testing with Goose](https://www.tag1consulting.com/blog/real-life-goose-load-testing)
//!   - [Gaggle: a distributed load test](https://www.tag1consulting.com/blog/show-me-how-flock-flies-working-gaggle-goose)
//!   - [Optimizing Goose performance](https://www.tag1consulting.com/blog/golden-goose-egg-compile-time-adventure)
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
mod report;
mod throttle;
mod user;
mod util;
#[cfg(feature = "gaggle")]
mod worker;

use chrono::prelude::*;
use chrono::Duration;
use gumdrop::Options;
use itertools::Itertools;
use lazy_static::lazy_static;
#[cfg(feature = "gaggle")]
use nng::Socket;
use rand::seq::SliceRandom;
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
use std::{fmt, io, time};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::runtime::Runtime;
use url::Url;

use crate::goose::{
    GaggleUser, GooseDebug, GooseRawRequest, GooseRequest, GooseTask, GooseTaskSet, GooseUser,
    GooseUserCommand,
};
use crate::metrics::{GooseErrorMetric, GooseMetric, GooseMetrics};
#[cfg(feature = "gaggle")]
use crate::worker::{register_shutdown_pipe_handler, GaggleMetrics};

/// Constant defining Goose's default port when running a Gaggle.
const DEFAULT_PORT: &str = "5115";

// WORKER_ID is only used when running a gaggle (a distributed load test).
lazy_static! {
    static ref WORKER_ID: AtomicUsize = AtomicUsize::new(0);
}

/// Internal representation of a weighted task list.
type WeightedGooseTasks = Vec<Vec<(usize, String)>>;

type DebugLoggerHandle = Option<tokio::task::JoinHandle<()>>;
type DebugLoggerChannel = Option<flume::Sender<Option<GooseDebug>>>;

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
pub enum AttackMode {
    /// A mode has not yet been assigned.
    Undefined,
    /// A single standalone process performing a load test.
    StandAlone,
    /// The controlling process in a Gaggle distributed load test.
    Manager,
    /// One of one or more working processes in a Gaggle distributed load test.
    Worker,
}

#[derive(Clone, Debug, PartialEq)]
/// A GooseAttack load test can operate in only one mode.
pub enum AttackPhase {
    /// Memory is being allocated for the GooseAttack.
    Initializing,
    /// GooseUsers are starting and beginning to generate load.
    Starting,
    /// All GooseUsers are started and generating load.
    Running,
    /// GooseUsers are stopping.
    Stopping,
}

#[derive(Clone, Debug, PartialEq)]
/// Defines the order GooseTaskSets are allocated to GooseUsers at startup time.
pub enum GooseTaskSetScheduler {
    /// Allocate one of each available type of GooseTaskSet at a time (default).
    RoundRobin,
    /// Allocate GooseTaskSets in the order and weighting they are defined.
    Serial,
    /// Allocate GooseTaskSets in a random order.
    Random,
}

/// Optional default values for Goose run-time options.
#[derive(Clone, Debug, Default)]
pub struct GooseDefaults {
    /// An optional default host to run this load test against.
    host: Option<String>,
    /// An optional default number of users to simulate.
    users: Option<usize>,
    /// An optional default number of clients to start per second.
    hatch_rate: Option<String>,
    /// An optional default number of seconds for the test to run.
    run_time: Option<usize>,
    /// An optional default log level.
    log_level: Option<u8>,
    /// An optional default for the log file name.
    log_file: Option<String>,
    /// An optional default value for verbosity level.
    verbose: Option<u8>,
    /// An optional default for printing running metrics.
    running_metrics: Option<usize>,
    /// An optional default for not resetting metrics after all users started.
    no_reset_metrics: Option<bool>,
    /// An optional default for not tracking metrics.
    no_metrics: Option<bool>,
    /// An optional default for not tracking task metrics.
    no_task_metrics: Option<bool>,
    /// An optional default for not displaying an error summary.
    no_error_summary: Option<bool>,
    /// An optional default for the html-formatted report file name.
    report_file: Option<String>,
    /// An optional default for the requests log file name.
    requests_file: Option<String>,
    /// An optional default for the requests log file format.
    metrics_format: Option<String>,
    /// An optional default for the debug log file name.
    debug_file: Option<String>,
    /// An optional default for the debug log format.
    debug_format: Option<String>,
    /// An optional default for not logging response body in debug log.
    no_debug_body: Option<bool>,
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

/// Allows the optional configuration of Goose's defaults.
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
    /// An optional default for printing running metrics.
    RunningMetrics,
    /// An optional default for not resetting metrics after all users started.
    NoResetMetrics,
    /// An optional default for not tracking metrics.
    NoMetrics,
    /// An optional default for not tracking task metrics.
    NoTaskMetrics,
    /// An optional default for not displaying an error summary.
    NoErrorSummary,
    /// An optional default for the report file name.
    ReportFile,
    /// An optional default for the requests log file name.
    RequestsFile,
    /// An optional default for the requests log file format.
    RequestsFormat,
    /// An optional default for the debug log file name.
    DebugFile,
    /// An optional default for the debug log format.
    DebugFormat,
    /// An optional default for not logging the response body in the debug log.
    NoDebugBody,
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

/// Internal global run state for load test.
pub struct GooseAttackRunState {
    /// A timestamp tracking when the previous GooseUser was launched.
    spawn_user_timer: std::time::Instant,
    /// How many milliseconds until the next user should be spawned.
    spawn_user_in_ms: usize,
    /// A counter tracking which GooseUser is being spawned.
    spawn_user_counter: usize,
    /// This variable accounts for time spent doing things which is then subtracted from
    /// the time sleeping to avoid an unintentional drift in events that are supposed to
    /// happen regularly.
    drift_timer: tokio::time::Instant,
    /// Unbounded sender used by all GooseUser threads to send metrics to parent.
    all_threads_metrics_tx: flume::Sender<GooseMetric>,
    /// Unbounded receiver used by Goose parent to receive metrics from GooseUsers.
    metrics_rx: flume::Receiver<GooseMetric>,
    /// Optional unbounded receiver for logger thread, if enabled.
    debug_logger: DebugLoggerHandle,
    /// Optional unbounded sender from all GooseUsers to logger thread, if enabled.
    all_threads_debug_logger_tx: DebugLoggerChannel,
    /// Optional receiver for all GooseUsers from throttle thread, if enabled.
    throttle_threads_tx: Option<flume::Sender<bool>>,
    /// Optional sender for throttle thread, if enabled.
    parent_to_throttle_tx: Option<flume::Sender<bool>>,
    /// Optional buffered writer for requests log file, if enabled.
    requests_file: Option<BufWriter<File>>,
    /// Optional unbuffered writer for html-formatted report file, if enabled.
    report_file: Option<File>,
    /// A flag tracking whether or not the header has been written when the metrics
    /// log is enabled.
    metrics_header_displayed: bool,
    /// Collection of all GooseUser threads so they can be stopped later.
    users: Vec<tokio::task::JoinHandle<()>>,
    /// All unbounded senders to allow communication with GooseUser threads.
    user_channels: Vec<flume::Sender<GooseUserCommand>>,
    /// Timer tracking when to display running metrics, if enabled.
    running_metrics_timer: std::time::Instant,
    /// Boolean flag indicating if running metrics should be displayed.
    display_running_metrics: bool,
    /// Boolean flag indicating if all GooseUsers have been spawned.
    all_users_spawned: bool,
    /// Thread-safe boolean flag indicating if the GooseAttack has been canceled.
    canceled: Arc<AtomicBool>,
    /// Optional socket used to coordinate a distributed Gaggle.
    socket: Option<Socket>,
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
    /// A weighted vector containing a lightweight GaggleUser object that will get sent to Workers.
    weighted_gaggle_users: Vec<GaggleUser>,
    /// An optional default host to run this load test against.
    defaults: GooseDefaults,
    /// Configuration object managed by StructOpt.
    configuration: GooseConfiguration,
    /// Track how long the load test should run.
    run_time: usize,
    /// Which mode this GooseAttack is operating in.
    attack_mode: AttackMode,
    /// Which mode this GooseAttack is operating in.
    attack_phase: AttackPhase,
    /// Defines the order GooseTaskSets are allocated to GooseUsers at startup time.
    scheduler: GooseTaskSetScheduler,
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
            weighted_gaggle_users: Vec::new(),
            defaults: GooseDefaults::default(),
            configuration: GooseConfiguration::parse_args_default_or_exit(),
            run_time: 0,
            attack_mode: AttackMode::Undefined,
            attack_phase: AttackPhase::Initializing,
            scheduler: GooseTaskSetScheduler::RoundRobin,
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
            weighted_gaggle_users: Vec::new(),
            defaults: GooseDefaults::default(),
            configuration,
            run_time: 0,
            attack_mode: AttackMode::Undefined,
            attack_phase: AttackPhase::Initializing,
            scheduler: GooseTaskSetScheduler::RoundRobin,
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
                SimpleLogger::new(debug_level, Config::default()),
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
            match CombinedLogger::init(vec![SimpleLogger::new(debug_level, Config::default())]) {
                Ok(_) => (),
                Err(e) => {
                    info!("failed to initialize CombinedLogger: {}", e);
                }
            }
        }

        info!("Output verbosity level: {}", debug_level);
        info!("Logfile verbosity level: {}", log_level);
    }

    /// Define the order `GooseTaskSet`s are allocated to new `GooseUser`s as they
    /// are launched.
    ///
    /// By default, GooseTaskSets are allocated to new GooseUser's in a round robin
    /// style. For example, if TaskSet A has a weight of 5, Task Set B has a weight
    ///  of 3, and you launch 20 users, they will be launched in the following order:
    ///  A, B, A, B, A, B, A, A, A, B, A, B, A, B, A, A, A, B, A, B
    ///
    /// Note that the following pattern is repeated:
    ///  A, B, A, B, A, B, A, A
    ///
    /// If reconfigured to schedule serially, then they will instead be allocated in
    /// the following order:
    /// A, A, A, A, A, B, B, B, A, A, A, A, A, B, B, B, A, A, A, A
    ///
    /// In the serial case, the following pattern is repeated:
    /// A, A, A, A, A, B, B, B
    ///
    /// In the following example, GooseTaskSets are allocated to launching GooseUsers
    /// in a random order. This means running the test multiple times can generate
    /// different amounts of load, as depending on your weighting rules you may
    /// have a different number of GooseUsers running each GooseTaskSet each time.
    ///
    /// # Example
    /// ```rust,no_run
    ///     use goose::prelude::*;
    ///
    /// fn main() -> Result<(), GooseError> {
    ///     GooseAttack::initialize()?
    ///         .set_scheduler(GooseTaskSetScheduler::Random)
    ///         .register_taskset(taskset!("A Tasks")
    ///             .set_weight(5)?
    ///             .register_task(task!(a_task_1))
    ///         )
    ///         .register_taskset(taskset!("B Tasks")
    ///             .set_weight(3)?
    ///             .register_task(task!(b_task_1))
    ///         );
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn a_task_1(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.get("/foo").await?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn b_task_1(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.get("/bar").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_scheduler(mut self, scheduler: GooseTaskSetScheduler) -> Self {
        self.scheduler = scheduler;
        self
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

    /// Use configured GooseTaskSetScheduler to build out a properly
    /// weighted list of TaskSets to be assigned to GooseUsers.
    fn allocate_tasks(&mut self) -> Vec<usize> {
        trace!("allocate_tasks");

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

        // Build a vector of vectors to be used to schedule users.
        let mut available_task_sets = Vec::with_capacity(self.task_sets.len());
        let mut total_task_sets = 0;
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
            let weighted_sets = vec![index; weight];
            total_task_sets += weight;
            available_task_sets.push(weighted_sets);
        }

        info!(
            "allocating GooseTasks to GooseUsers with {:?} scheduler",
            self.scheduler
        );

        // Now build the weighted list with the appropriate scheduler.
        let mut weighted_task_sets = Vec::new();
        match self.scheduler {
            GooseTaskSetScheduler::RoundRobin => {
                // Allocate task sets round robin.
                let task_sets_len = available_task_sets.len();
                loop {
                    for (task_set_index, task_sets) in available_task_sets
                        .iter_mut()
                        .enumerate()
                        .take(task_sets_len)
                    {
                        if let Some(task_set) = task_sets.pop() {
                            debug!("allocating 1 user from TaskSet {}", task_set_index);
                            weighted_task_sets.push(task_set);
                        }
                    }
                    if weighted_task_sets.len() >= total_task_sets {
                        break;
                    }
                }
            }
            GooseTaskSetScheduler::Serial => {
                // Allocate task sets serially in the weighted order defined.
                for (task_set_index, task_sets) in available_task_sets.iter().enumerate() {
                    debug!(
                        "allocating all {} users from TaskSet {}",
                        task_sets.len(),
                        task_set_index
                    );
                    weighted_task_sets.append(&mut task_sets.clone());
                }
            }
            GooseTaskSetScheduler::Random => {
                // Allocate task sets randomly.
                loop {
                    let task_set = available_task_sets.choose_mut(&mut rand::thread_rng());
                    match task_set {
                        Some(set) => {
                            if let Some(s) = set.pop() {
                                weighted_task_sets.push(s);
                            }
                        }
                        None => warn!("randomly allocating a GooseTaskSet failed, trying again"),
                    }
                    if weighted_task_sets.len() >= total_task_sets {
                        break;
                    }
                }
            }
        }
        weighted_task_sets
    }

    /// Allocate a vector of weighted GooseUser.
    fn weight_task_set_users(&mut self) -> Result<Vec<GooseUser>, GooseError> {
        trace!("weight_task_set_users");

        let weighted_task_sets = self.allocate_tasks();

        // Allocate a state for each user that will be hatched.
        info!("initializing user states...");
        let mut weighted_users = Vec::new();
        let mut user_count = 0;
        loop {
            for task_sets_index in &weighted_task_sets {
                debug!(
                    "creating user state: {} ({})",
                    weighted_users.len(),
                    task_sets_index
                );
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
                    debug!("created {} weighted_users", user_count);
                    return Ok(weighted_users);
                }
            }
        }
    }

    /// Allocate a vector of weighted GaggleUser.
    fn prepare_worker_task_set_users(&mut self) -> Result<Vec<GaggleUser>, GooseError> {
        trace!("prepare_worker_task_set_users");

        let weighted_task_sets = self.allocate_tasks();

        // Determine the users sent to each Worker.
        info!("preparing users for Workers...");
        let mut weighted_users = Vec::new();
        let mut user_count = 0;
        loop {
            for task_sets_index in &weighted_task_sets {
                let base_url = goose::get_base_url(
                    self.get_configuration_host(),
                    self.task_sets[*task_sets_index].host.clone(),
                    self.defaults.host.clone(),
                )?;
                weighted_users.push(GaggleUser::new(
                    self.task_sets[*task_sets_index].task_sets_index,
                    base_url,
                    self.task_sets[*task_sets_index].min_wait,
                    self.task_sets[*task_sets_index].max_wait,
                    &self.configuration,
                    self.metrics.hash,
                ));
                user_count += 1;
                // Users are required here so unwrap() is safe.
                if user_count >= self.configuration.users.unwrap() {
                    debug!("prepared {} weighted_gaggle_users", user_count);
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
            self.attack_mode = AttackMode::Manager;
            if self.configuration.worker {
                return Err(GooseError::InvalidOption {
                    option: "--worker".to_string(),
                    value: "true".to_string(),
                    detail: "The --worker flag can not be set together with the --manager flag"
                        .to_string(),
                });
            }

            if self.get_debug_file_path().is_some() {
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
            self.attack_mode = AttackMode::Worker;
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
        if self.attack_mode == AttackMode::Undefined {
            self.attack_mode = AttackMode::StandAlone;

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

    // Determine how many workers to expect.
    fn set_expect_workers(&mut self) -> Result<(), GooseError> {
        // Track how value gets set so we can return a meaningful error if necessary.
        let mut key = "configuration.expect_workers";

        // Check if --expect-workers was set.
        if self.configuration.expect_workers.is_some() {
            key = "--expect-workers";
        // Otherwise check if a custom default is set.
        } else if let Some(default_expect_workers) = self.defaults.expect_workers {
            if self.attack_mode == AttackMode::Manager {
                key = "set_default(GooseDefault::ExpectWorkers)";

                self.configuration.expect_workers = Some(default_expect_workers);
            }
        }

        if let Some(expect_workers) = self.configuration.expect_workers {
            // Disallow --expect-workers without --manager.
            if self.attack_mode != AttackMode::Manager {
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
        if self.attack_mode == AttackMode::Manager {
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
        if self.attack_mode == AttackMode::Worker {
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
            // On Worker users comes from the Manager.
            if self.attack_mode == AttackMode::Worker {
                self.configuration.users = None;
            // Otherwise use default.
            } else {
                key = "set_default(GooseDefault::Users)";
                value = default_users;

                self.configuration.users = Some(default_users);
            }
        // If not and if not running on Worker, default to 1.
        } else if self.attack_mode != AttackMode::Worker {
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
            if self.attack_mode == AttackMode::Worker {
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
            if self.attack_mode == AttackMode::Worker {
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
            if self.attack_mode == AttackMode::Worker {
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
        let mut value = "".to_string();

        // Check if --hash-rate is set.
        if let Some(hatch_rate) = &self.configuration.hatch_rate {
            key = "--hatch_rate";
            value = hatch_rate.to_string();
        // If not, check if a default hatch_rate is set.
        } else if let Some(default_hatch_rate) = &self.defaults.hatch_rate {
            // On Worker hatch_rate comes from the Manager.
            if self.attack_mode == AttackMode::Worker {
                self.configuration.hatch_rate = None;
            // Otherwise use default.
            } else {
                key = "set_default(GooseDefault::HatchRate)";
                value = default_hatch_rate.to_string();
                self.configuration.hatch_rate = Some(default_hatch_rate.to_string());
            }
        // If not and if not running on Worker, default to 1.
        } else if self.attack_mode != AttackMode::Worker {
            // This should not be able to fail, but setting up debug in case a later
            // change introduces the potential for failure.
            key = "Goose default";
            value = "1".to_string();
            self.configuration.hatch_rate = Some(value.to_string());
        }

        // Verbose output.
        if let Some(hatch_rate) = &self.configuration.hatch_rate {
            // Setting --hatch-rate with --worker is not allowed.
            if self.attack_mode == AttackMode::Worker {
                return Err(GooseError::InvalidOption {
                    option: key.to_string(),
                    value,
                    detail: format!("{} can not be set together with the --worker flag.", key),
                });
            }

            // Setting --hatch-rate of 0 is not allowed.
            if hatch_rate.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: key.to_string(),
                    value,
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
        let mut key = "configuration.throttle_requests";
        let mut value = 0;

        if self.configuration.throttle_requests > 0 {
            key = "--throttle-requests";
            value = self.configuration.throttle_requests;
        }

        // Use default for throttle_requests if set and not on Worker.
        if self.configuration.throttle_requests == 0 {
            if let Some(default_throttle_requests) = self.defaults.throttle_requests {
                // In Gaggles, throttle_requests is only set on Worker.
                if self.attack_mode != AttackMode::Manager {
                    key = "set_default(GooseDefault::ThrottleRequests)";
                    value = default_throttle_requests;

                    self.configuration.throttle_requests = default_throttle_requests;
                }
            }
        }

        if self.configuration.throttle_requests > 0 {
            // Setting --throttle-requests with --worker is not allowed.
            if self.attack_mode == AttackMode::Manager {
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
        } else if self.attack_mode != AttackMode::Worker {
            if let Some(default_no_reset_metrics) = self.defaults.no_reset_metrics {
                key = "set_default(GooseDefault::NoResetMetrics)";
                value = default_no_reset_metrics;

                // Optionally set default.
                self.configuration.no_reset_metrics = default_no_reset_metrics;
            }
        }

        // Setting --no-reset-metrics with --worker is not allowed.
        if self.configuration.no_reset_metrics && self.attack_mode == AttackMode::Worker {
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
        } else if self.attack_mode != AttackMode::Worker {
            if let Some(default_status_codes) = self.defaults.status_codes {
                key = "set_default(GooseDefault::StatusCodes)";
                value = default_status_codes;

                // Optionally set default.
                self.configuration.status_codes = default_status_codes;
            }
        }

        // Setting --status-codes with --worker is not allowed.
        if self.configuration.status_codes && self.attack_mode == AttackMode::Worker {
            return Err(GooseError::InvalidOption {
                option: key.to_string(),
                value: value.to_string(),
                detail: format!("{} can not be set together with the --worker flag.", key),
            });
        }

        Ok(())
    }

    // Determine if the running_metrics flag is enabled.
    fn set_running_metrics(&mut self) -> Result<(), GooseError> {
        // Track how value gets set so we can return a meaningful error if necessary.
        let mut key = "configuration.running_metrics";
        let mut value = 0;

        if let Some(running_metrics) = self.configuration.running_metrics {
            key = "--running-metrics";
            value = running_metrics;
        // If not otherwise set and not Worker, check if there's a default.
        } else if self.attack_mode != AttackMode::Worker {
            // Optionally set default.
            if let Some(default_running_metrics) = self.defaults.running_metrics {
                key = "set_default(GooseDefault::RunningMetrics)";
                value = default_running_metrics;

                self.configuration.running_metrics = Some(default_running_metrics);
            }
        }

        // Setting --running-metrics with --worker is not allowed.
        if let Some(running_metrics) = self.configuration.running_metrics {
            if self.attack_mode == AttackMode::Worker {
                return Err(GooseError::InvalidOption {
                    option: key.to_string(),
                    value: value.to_string(),
                    detail: format!("{} can not be set together with the --worker flag.", key),
                });
            }

            if running_metrics > 0 {
                info!("running_metrics = {}", running_metrics);
            }
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
        } else if self.attack_mode != AttackMode::Worker {
            // Optionally set default.
            if let Some(default_no_task_metrics) = self.defaults.no_task_metrics {
                key = "set_default(GooseDefault::NoTaskMetrics)";
                value = default_no_task_metrics;

                self.configuration.no_task_metrics = default_no_task_metrics;
            }
        }

        // Setting --no-task-metrics with --worker is not allowed.
        if self.configuration.no_task_metrics && self.attack_mode == AttackMode::Worker {
            return Err(GooseError::InvalidOption {
                option: key.to_string(),
                value: value.to_string(),
                detail: format!("{} can not be set together with the --worker flag.", key),
            });
        }

        Ok(())
    }

    // Determine if the no_error_summary flag is enabled.
    fn set_no_error_summary(&mut self) -> Result<(), GooseError> {
        // Track how value gets set so we can return a meaningful error if necessary.
        let mut key = "configuration.no_error_summary";
        let mut value = false;

        if self.configuration.no_error_summary {
            key = "--no-error-summary";
            value = true;
        // If not otherwise set and not Worker, check if there's a default.
        } else if self.attack_mode != AttackMode::Worker {
            // Optionally set default.
            if let Some(default_no_error_summary) = self.defaults.no_error_summary {
                key = "set_default(GooseDefault::NoErrorSummary)";
                value = default_no_error_summary;

                self.configuration.no_error_summary = default_no_error_summary;
            }
        }

        // Setting --no-error-summary with --worker is not allowed.
        if self.configuration.no_error_summary && self.attack_mode == AttackMode::Worker {
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
        } else if self.attack_mode != AttackMode::Worker {
            // Optionally set default.
            if let Some(default_no_metrics) = self.defaults.no_metrics {
                key = "set_default(GooseDefault::NoMetrics)";
                value = default_no_metrics;

                self.configuration.no_metrics = default_no_metrics;
            }
        }

        // Setting --no-metrics with --worker is not allowed.
        if self.configuration.no_metrics && self.attack_mode == AttackMode::Worker {
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
            if self.configuration.running_metrics.is_some() {
                return Err(GooseError::InvalidOption {
                    option: key.to_string(),
                    value: value.to_string(),
                    detail: format!(
                        "{} can not be set together with the --running_metrics option.",
                        key
                    ),
                });
            }

            // There is nothing to log if metrics are disabled.
            if !self.configuration.requests_file.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: key.to_string(),
                    value: value.to_string(),
                    detail: format!(
                        "{} can not be set together with the --requests-file option.",
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
        } else if self.attack_mode != AttackMode::Worker {
            // Optionally set default.
            if let Some(default_sticky_follow) = self.defaults.sticky_follow {
                key = "set_default(GooseDefault::StickyFollow)";
                value = default_sticky_follow;

                self.configuration.sticky_follow = default_sticky_follow;
            }
        }

        if self.configuration.sticky_follow && self.attack_mode == AttackMode::Worker {
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
        } else if self.attack_mode != AttackMode::Worker {
            // Optionally set default.
            if let Some(default_no_hash_check) = self.defaults.no_hash_check {
                key = "set_default(GooseDefault::NoHashCheck)";
                value = default_no_hash_check;

                self.configuration.no_hash_check = default_no_hash_check;
            }
        }

        if self.configuration.no_hash_check && self.attack_mode == AttackMode::Worker {
            return Err(GooseError::InvalidOption {
                option: key.to_string(),
                value: value.to_string(),
                detail: format!("{} can not be set together with the --worker flag.", key),
            });
        }

        Ok(())
    }

    // If enabled, returns the path of the report_file, otherwise returns None.
    fn get_report_file_path(&mut self) -> Option<String> {
        // If metrics are disabled, or running in Manager mode, there is no
        // report file, exit immediately.
        if self.configuration.no_metrics || self.attack_mode == AttackMode::Manager {
            return None;
        }

        // If --report-file is set, return it.
        if !self.configuration.report_file.is_empty() {
            return Some(self.configuration.report_file.to_string());
        }

        // If GooseDefault::ReportFile is set, return it.
        if let Some(default_report_file) = &self.defaults.report_file {
            return Some(default_report_file.to_string());
        }

        // Otherwise there is no report file.
        None
    }

    // If enabled, returns the path of the requests_file, otherwise returns None.
    fn get_requests_file_path(&mut self) -> Option<&str> {
        // If metrics are disabled, or running in Manager mode, there is no
        // requests file, exit immediately.
        if self.configuration.no_metrics || self.attack_mode == AttackMode::Manager {
            return None;
        }

        // If --requests-file is set, return it.
        if !self.configuration.requests_file.is_empty() {
            return Some(&self.configuration.requests_file);
        }

        // If GooseDefault::MetricFile is set, return it.
        if let Some(default_requests_file) = &self.defaults.requests_file {
            return Some(default_requests_file);
        }

        // Otherwise there is no requests file.
        None
    }

    // Configure requests log format.
    fn set_requests_format(&mut self) -> Result<(), GooseError> {
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
                    detail: "The --no-metrics flag can not be set together with the --requests-format option.".to_string(),
                });
            }
            // Log format isn't relevant if log not enabled.
            else if self.get_requests_file_path().is_none() {
                return Err(GooseError::InvalidOption {
                    option: "--requests-format".to_string(),
                    value: self.configuration.metrics_format.clone(),
                    detail: "The --requests-file option must be set together with the --requests-format option.".to_string(),
                });
            }
        }

        let options = vec!["json", "csv", "raw"];
        if !options.contains(&self.configuration.metrics_format.as_str()) {
            return Err(GooseError::InvalidOption {
                option: "--requests-format".to_string(),
                value: self.configuration.metrics_format.clone(),
                detail: format!(
                    "The --requests-format option must be set to one of: {}.",
                    options.join(", ")
                ),
            });
        }

        Ok(())
    }

    // If enabled, returns the path of the debug_file, otherwise returns None.
    fn get_debug_file_path(&self) -> Option<&str> {
        // If running in Manager mode there is no debug file, exit immediately.
        if self.attack_mode == AttackMode::Manager {
            return None;
        }

        // If --debug-file is set, return it.
        if !self.configuration.debug_file.is_empty() {
            return Some(&self.configuration.debug_file);
        }

        // If GooseDefault::DebugFile is set, return it.
        if let Some(default_debug_file) = &self.defaults.debug_file {
            return Some(default_debug_file);
        }

        // Otherwise there is no debug file.
        None
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

    // Configure whether to log response body.
    fn set_no_debug_body(&mut self) -> Result<(), GooseError> {
        // Track how value gets set so we can return a meaningful error if necessary.
        let mut key = "configuration.no_debug_body";
        let mut value = false;

        if self.configuration.no_debug_body {
            key = "--no-debug-body";
            value = true;
        // If not otherwise set and not Manager, check if there's a default.
        } else if self.attack_mode != AttackMode::Manager {
            // Optionally set default.
            if let Some(default_no_debug_body) = self.defaults.no_debug_body {
                key = "set_default(GooseDefault::NoDebugBody)";
                value = default_no_debug_body;

                self.configuration.no_debug_body = default_no_debug_body;
            }
        }

        if self.configuration.no_debug_body && self.attack_mode == AttackMode::Manager {
            return Err(GooseError::InvalidOption {
                option: key.to_string(),
                value: value.to_string(),
                detail: format!("{} can not be set together with the --manager flag.", key),
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

        // Configure the requests log format.
        self.set_requests_format()?;

        // Configure the debug log format.
        self.set_debug_format()?;

        // Determine whether or not to log response body.
        self.set_no_debug_body()?;

        // Configure throttle if enabled.
        self.set_throttle_requests()?;

        // Configure status_codes flag.
        self.set_status_codes()?;

        // Configure running_metrics flag.
        self.set_running_metrics()?;

        // Configure no_reset_metrics flag.
        self.set_no_reset_metrics()?;

        // Configure no_task_metrics flag.
        self.set_no_task_metrics()?;

        // Configure no_error_summary flag.
        self.set_no_error_summary()?;

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
                            if self.attack_mode != AttackMode::Worker {
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

        if self.attack_mode != AttackMode::Worker {
            // Stand-alone and Manager processes can display metrics.
            if !self.configuration.no_metrics {
                self.metrics.display_metrics = true;
            }

            if self.attack_mode == AttackMode::StandAlone {
                // Allocate a state for each of the users we are about to start.
                self.weighted_users = self.weight_task_set_users()?;
            } else if self.attack_mode == AttackMode::Manager {
                // Build a list of users to be allocated on Workers.
                self.weighted_gaggle_users = self.prepare_worker_task_set_users()?;
            }
        }

        // Calculate a unique hash for the current load test.
        let mut s = DefaultHasher::new();
        self.task_sets.hash(&mut s);
        self.metrics.hash = s.finish();
        debug!("hash: {}", self.metrics.hash);

        // Start goose in manager mode.
        if self.attack_mode == AttackMode::Manager {
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
        else if self.attack_mode == AttackMode::Worker {
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
            self = rt.block_on(self.start_attack(None))?;
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
    fn prepare_csv(
        raw_request: &GooseRawRequest,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> String {
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
        if !goose_attack_run_state.metrics_header_displayed {
            goose_attack_run_state.metrics_header_displayed = true;
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
    fn setup_debug_logger(&mut self) -> (DebugLoggerHandle, DebugLoggerChannel) {
        // Set configuration from default if available, making it available to
        // GooseUser threads.
        self.configuration.debug_file = if let Some(debug_file) = self.get_debug_file_path() {
            debug_file.to_string()
        } else {
            "".to_string()
        };
        // If the logger isn't configured, return immediately.
        if self.configuration.debug_file.is_empty() {
            return (None, None);
        }

        // Create an unbounded channel allowing GooseUser threads to log errors.
        let (all_threads_debug_logger, logger_receiver): (
            flume::Sender<Option<GooseDebug>>,
            flume::Receiver<Option<GooseDebug>>,
        ) = flume::unbounded();
        // Launch a new thread for logging.
        let logger_thread = tokio::spawn(logger::logger_main(
            self.configuration.clone(),
            logger_receiver,
        ));
        (Some(logger_thread), Some(all_threads_debug_logger))
    }

    // Helper to spawn a throttle thread if configured.
    async fn setup_throttle(
        &self,
    ) -> (
        // A channel used by GooseClients to throttle requests.
        Option<flume::Sender<bool>>,
        // A channel used by parent to tell throttle the load test is complete.
        Option<flume::Sender<bool>>,
    ) {
        // If the throttle isn't enabled, return immediately.
        if self.configuration.throttle_requests == 0 {
            return (None, None);
        }

        // Create a bounded channel allowing single-sender multi-receiver to throttle
        // GooseUser threads.
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

    // Prepare an asynchronous file writer for report_file (if enabled).
    async fn prepare_report_file(&mut self) -> Result<Option<File>, GooseError> {
        if let Some(report_file_path) = self.get_report_file_path() {
            Ok(Some(File::create(&report_file_path).await?))
        } else {
            Ok(None)
        }
    }

    // Prepare an asynchronous buffered file writer for requests_file (if enabled).
    async fn prepare_requests_file(&mut self) -> Result<Option<BufWriter<File>>, GooseError> {
        if let Some(requests_file_path) = self.get_requests_file_path() {
            Ok(Some(BufWriter::new(
                File::create(&requests_file_path).await?,
            )))
        } else {
            Ok(None)
        }
    }

    // Invoke test_start tasks if existing.
    async fn run_test_start(&self) -> Result<(), GooseError> {
        // Initialize per-user states.
        if self.attack_mode != AttackMode::Worker {
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
        if self.attack_mode != AttackMode::Worker {
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

    // Create a GooseAttackRunState object and do all initialization required
    // to start a GooseAttack.
    async fn initialize_attack(
        &mut self,
        socket: Option<Socket>,
    ) -> Result<GooseAttackRunState, GooseError> {
        trace!("initialize_attack");

        // Run any configured test_start() functions.
        self.run_test_start().await?;

        // Only display status codes if enabled.
        self.metrics.display_status_codes = self.configuration.status_codes;

        // Create a single channel used to send metrics from GooseUser threads
        // to parent thread.
        let (all_threads_metrics_tx, metrics_rx): (
            flume::Sender<GooseMetric>,
            flume::Receiver<GooseMetric>,
        ) = flume::unbounded();

        // If enabled, spawn a logger thread.
        let (debug_logger, all_threads_debug_logger_tx) = self.setup_debug_logger();

        // If enabled, spawn a throttle thread.
        let (throttle_threads_tx, parent_to_throttle_tx) = self.setup_throttle().await;

        // Grab now() once from the standard library, used by multiple timers in
        // the run state.
        let std_now = std::time::Instant::now();

        // If the report file is enabled, open it now to confirm we have access
        let report_file = match self.prepare_report_file().await {
            Ok(f) => f,
            Err(e) => {
                return Err(GooseError::InvalidOption {
                    option: "--report-file".to_string(),
                    value: self.get_report_file_path().unwrap(),
                    detail: format!("Failed to create report file: {}", e),
                })
            }
        };

        // If the requests file is enabled, open it now to confirm we have access
        let requests_file = match self.prepare_requests_file().await {
            Ok(f) => f,
            Err(e) => {
                return Err(GooseError::InvalidOption {
                    option: "--requests-file".to_string(),
                    value: self.get_requests_file_path().unwrap().to_string(),
                    detail: format!("Failed to create request file: {}", e),
                })
            }
        };

        let goose_attack_run_state = GooseAttackRunState {
            spawn_user_timer: std_now,
            spawn_user_in_ms: 0,
            spawn_user_counter: 0,
            drift_timer: tokio::time::Instant::now(),
            all_threads_metrics_tx,
            metrics_rx,
            debug_logger,
            all_threads_debug_logger_tx,
            throttle_threads_tx,
            parent_to_throttle_tx,
            report_file,
            requests_file,
            metrics_header_displayed: false,
            users: Vec::new(),
            user_channels: Vec::new(),
            running_metrics_timer: std_now,
            display_running_metrics: false,
            all_users_spawned: false,
            canceled: Arc::new(AtomicBool::new(false)),
            socket,
        };

        // Access socket to avoid errors.
        trace!("socket: {:?}", &goose_attack_run_state.socket);

        // Catch ctrl-c to allow clean shutdown to display metrics.
        util::setup_ctrlc_handler(&goose_attack_run_state.canceled);

        // Initialize the optional task metrics.
        self.metrics
            .initialize_task_metrics(&self.task_sets, &self.configuration);

        // Our load test is officially starting. Store an initial measurement
        // of a monotonically nondecreasing clock so we can see how long has
        // elapsed without worrying about the clock going backward.
        self.started = Some(time::Instant::now());

        // Also store a formattable timestamp, for human readable reports.
        self.metrics.started = Some(Local::now());

        Ok(goose_attack_run_state)
    }

    // Spawn GooseUsers to generate a GooseAttack.
    async fn spawn_attack(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        // If the run_timer has expired or ctrl-c was caught, stop spawning
        // threads and start stopping threads. Unwrap is safe here because
        // we had to have started to get here.
        if util::timer_expired(self.started.unwrap(), self.run_time)
            || goose_attack_run_state.canceled.load(Ordering::SeqCst)
        {
            self.set_attack_phase(goose_attack_run_state, AttackPhase::Stopping);
            return Ok(());
        }

        // Hatch rate is used to schedule the next user, and to ensure we don't
        // sleep too long.
        let hatch_rate = util::get_hatch_rate(self.configuration.hatch_rate.clone());

        // Determine if it's time to spawn a GooseUser.
        if goose_attack_run_state.spawn_user_in_ms == 0
            || util::ms_timer_expired(
                goose_attack_run_state.spawn_user_timer,
                goose_attack_run_state.spawn_user_in_ms,
            )
        {
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
            }

            // Spawn next scheduled GooseUser.
            let mut thread_user =
                self.weighted_users[goose_attack_run_state.spawn_user_counter].clone();
            goose_attack_run_state.spawn_user_counter += 1;

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
                flume::Sender<GooseUserCommand>,
                flume::Receiver<GooseUserCommand>,
            ) = flume::unbounded();
            goose_attack_run_state.user_channels.push(parent_sender);

            if self.get_debug_file_path().is_some() {
                // Copy the GooseUser-to-logger sender channel, used by all threads.
                thread_user.debug_logger = Some(
                    goose_attack_run_state
                        .all_threads_debug_logger_tx
                        .clone()
                        .unwrap(),
                );
            } else {
                thread_user.debug_logger = None;
            }

            // Copy the GooseUser-throttle receiver channel, used by all threads.
            thread_user.throttle = if self.configuration.throttle_requests > 0 {
                Some(goose_attack_run_state.throttle_threads_tx.clone().unwrap())
            } else {
                None
            };

            // Copy the GooseUser-to-parent sender channel, used by all threads.
            thread_user.channel_to_parent =
                Some(goose_attack_run_state.all_threads_metrics_tx.clone());

            // Copy the appropriate task_set into the thread.
            let thread_task_set = self.task_sets[thread_user.task_sets_index].clone();

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
                thread_task_set,
                thread_user,
                thread_receiver,
                is_worker,
            ));

            goose_attack_run_state.users.push(user);
            self.metrics.users += 1;

            if let Some(running_metrics) = self.configuration.running_metrics {
                if self.attack_mode != AttackMode::Worker
                    && util::timer_expired(
                        goose_attack_run_state.running_metrics_timer,
                        running_metrics,
                    )
                {
                    goose_attack_run_state.running_metrics_timer = time::Instant::now();
                    self.metrics.print_running();
                }
            }
        } else {
            // If displaying running metrics, be sure we wake up often enough to
            // display them at the configured rate.
            let running_metrics = self.configuration.running_metrics.unwrap_or(0);

            // Otherwise, sleep until the next time something needs to happen.
            let sleep_duration = if running_metrics > 0
                && running_metrics * 1_000 < goose_attack_run_state.spawn_user_in_ms
            {
                let sleep_delay = self.configuration.running_metrics.unwrap() * 1_000;
                goose_attack_run_state.spawn_user_in_ms -= sleep_delay;
                tokio::time::Duration::from_millis(sleep_delay as u64)
            } else {
                tokio::time::Duration::from_millis(goose_attack_run_state.spawn_user_in_ms as u64)
            };
            debug!("sleeping {:?}...", sleep_duration);
            goose_attack_run_state.drift_timer =
                util::sleep_minus_drift(sleep_duration, goose_attack_run_state.drift_timer).await;
        }

        // If enough users have been spawned, move onto the next attack phase.
        if self.metrics.users >= self.weighted_users.len() {
            // Pause a tenth of a second waiting for the final user to fully start up.
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            if self.attack_mode == AttackMode::Worker {
                info!(
                    "[{}] launched {} users...",
                    get_worker_id(),
                    self.metrics.users
                );
            } else {
                info!("launched {} users...", self.metrics.users);
            }

            self.reset_metrics(goose_attack_run_state).await?;
            self.set_attack_phase(goose_attack_run_state, AttackPhase::Running);
        }

        Ok(())
    }

    // Let the GooseAttack run until the timer expires (or the test is canceled), and then
    // trigger a shut down.
    async fn monitor_attack(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        if util::timer_expired(self.started.unwrap(), self.run_time)
            || goose_attack_run_state.canceled.load(Ordering::SeqCst)
        {
            if self.attack_mode == AttackMode::Worker {
                info!(
                    "[{}] stopping after {} seconds...",
                    get_worker_id(),
                    self.started.unwrap().elapsed().as_secs()
                );

                // Load test is shutting down, update pipe handler so there is no panic
                // when the Manager goes away.
                #[cfg(feature = "gaggle")]
                {
                    let manager = goose_attack_run_state.socket.clone().unwrap();
                    register_shutdown_pipe_handler(&manager);
                }
            } else {
                info!(
                    "stopping after {} seconds...",
                    self.started.unwrap().elapsed().as_secs()
                );
            }
            for (index, send_to_user) in goose_attack_run_state.user_channels.iter().enumerate() {
                match send_to_user.send(GooseUserCommand::EXIT) {
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
            if let Some(tx) = goose_attack_run_state.parent_to_throttle_tx.clone() {
                let _ = tx.send(false);
            }

            // Take the users vector out of the GooseAttackRunState object so it can be
            // consumed by futures::future::join_all().
            let users = std::mem::take(&mut goose_attack_run_state.users);
            futures::future::join_all(users).await;
            debug!("all users exited");

            if self.get_debug_file_path().is_some() {
                // Tell logger thread to flush and exit.
                if let Err(e) = goose_attack_run_state
                    .all_threads_debug_logger_tx
                    .clone()
                    .unwrap()
                    .send(None)
                {
                    warn!("unexpected error telling logger thread to exit: {}", e);
                };
                // If the debug logger is enabled, wait for thread to flush and exit.
                if goose_attack_run_state.debug_logger.is_some() {
                    // Take debug_logger out of the GooseAttackRunState object so it can be
                    // consumed by tokio::join!().
                    let debug_logger = std::mem::take(&mut goose_attack_run_state.debug_logger);
                    let _ = tokio::join!(debug_logger.unwrap());
                }
            }

            // If we're printing metrics, collect the final metrics received from users.
            if !self.configuration.no_metrics {
                let _received_message = self.receive_metrics(goose_attack_run_state).await?;
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
                            GaggleMetrics::Tasks(self.metrics.tasks.clone()),
                        ],
                        true,
                    );
                    // No need to reset local metrics, the worker is exiting.
                }
            }

            // All users are done, exit out of loop for final cleanup.
            self.set_attack_phase(goose_attack_run_state, AttackPhase::Stopping);
        } else {
            // Subtract the time spent doing other things, with the goal of running the
            // main parent loop once every second.
            goose_attack_run_state.drift_timer = util::sleep_minus_drift(
                time::Duration::from_secs(1),
                goose_attack_run_state.drift_timer,
            )
            .await;
        }

        Ok(())
    }

    // If metrics are enabled, synchronize metrics from child threads to the parent.
    async fn sync_metrics(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        if !self.configuration.no_metrics {
            // Check if we're displaying running metrics.
            if let Some(running_metrics) = self.configuration.running_metrics {
                if self.attack_mode != AttackMode::Worker
                    && util::timer_expired(
                        goose_attack_run_state.running_metrics_timer,
                        running_metrics,
                    )
                {
                    goose_attack_run_state.running_metrics_timer = time::Instant::now();
                    goose_attack_run_state.display_running_metrics = true;
                }
            }

            // Load messages from user threads until the receiver queue is empty.
            let received_message = self.receive_metrics(goose_attack_run_state).await?;

            // As worker, push metrics up to manager.
            if self.attack_mode == AttackMode::Worker && received_message {
                #[cfg(feature = "gaggle")]
                {
                    // Push metrics to manager process.
                    if !worker::push_metrics_to_manager(
                        &goose_attack_run_state.socket.clone().unwrap(),
                        vec![
                            GaggleMetrics::Requests(self.metrics.requests.clone()),
                            GaggleMetrics::Tasks(self.metrics.tasks.clone()),
                        ],
                        true,
                    ) {
                        // EXIT received, cancel.
                        goose_attack_run_state
                            .canceled
                            .store(true, Ordering::SeqCst);
                    }
                    // The manager has all our metrics, reset locally.
                    self.metrics.requests = HashMap::new();
                    self.metrics
                        .initialize_task_metrics(&self.task_sets, &self.configuration);
                }
            }
        }

        // If enabled, display running metrics after sync
        if goose_attack_run_state.display_running_metrics {
            goose_attack_run_state.display_running_metrics = false;
            self.metrics.duration = self.started.unwrap().elapsed().as_secs() as usize;
            self.metrics.print_running();
        }

        Ok(())
    }

    // When the Goose Attack starts, optionally flush metrics.
    async fn reset_metrics(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        // Flush metrics collected prior to all user threads running
        if !goose_attack_run_state.all_users_spawned {
            // Receive metrics before resetting them.
            self.sync_metrics(goose_attack_run_state).await?;

            goose_attack_run_state.all_users_spawned = true;
            let users = self.configuration.users.clone().unwrap();
            if !self.configuration.no_reset_metrics {
                // Display the running metrics collected so far, before resetting them.
                self.metrics.duration = self.started.unwrap().elapsed().as_secs() as usize;
                self.metrics.print_running();
                // Reset running_metrics_timer.
                goose_attack_run_state.running_metrics_timer = time::Instant::now();

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

        Ok(())
    }

    // Cleanly shut down the Goose Attack.
    async fn stop_attack(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        self.metrics.duration = self.started.unwrap().elapsed().as_secs() as usize;

        // Run any configured test_stop() functions.
        self.run_test_stop().await?;

        // If requests logging is enabled, flush all metrics before we exit.
        if let Some(file) = goose_attack_run_state.requests_file.as_mut() {
            info!(
                "flushing requests_file: {}",
                // Unwrap is safe as we can't get here unless a requests file path
                // is defined.
                self.get_requests_file_path().unwrap()
            );
            let _ = file.flush().await;
        };
        // Percentile and errors are only displayed when the load test is finished.
        self.metrics.final_metrics = true;

        Ok(())
    }

    async fn write_html_report(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        // Only write the report if enabled.
        if let Some(report_file) = goose_attack_run_state.report_file.as_mut() {
            // Prepare report summary variables.
            let started = self.metrics.started.clone().unwrap();
            let start_time = started.format("%Y-%m-%d %H:%M:%S").to_string();
            let end_time = (started + Duration::seconds(self.metrics.duration as i64))
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            let host = match self.get_configuration_host() {
                Some(h) => h.to_string(),
                None => "".to_string(),
            };

            // Prepare requests and responses variables.
            let mut request_metrics = Vec::new();
            let mut response_metrics = Vec::new();
            let mut aggregate_total_count = 0;
            let mut aggregate_fail_count = 0;
            let mut aggregate_response_time_counter: usize = 0;
            let mut aggregate_response_time_minimum: usize = 0;
            let mut aggregate_response_time_maximum: usize = 0;
            let mut aggregate_response_times: BTreeMap<usize, usize> = BTreeMap::new();
            for (request_key, request) in self.metrics.requests.iter().sorted() {
                let method = format!("{:?}", request.method);
                // The request_key is "{method} {name}", so by stripping the "{method} "
                // prefix we get the name.
                // @TODO: consider storing the name as a field in GooseRequest.
                let name = request_key
                    .strip_prefix(&format!("{:?} ", request.method))
                    .unwrap()
                    .to_string();
                let total_request_count = request.success_count + request.fail_count;
                let (requests_per_second, failures_per_second) = metrics::per_second_calculations(
                    self.metrics.duration,
                    total_request_count,
                    request.fail_count,
                );
                // Prepare per-request metrics.
                request_metrics.push(report::RequestMetric {
                    method: method.to_string(),
                    name: name.to_string(),
                    number_of_requests: total_request_count,
                    number_of_failures: request.fail_count,
                    response_time_average: format!(
                        "{:.2}",
                        request.total_response_time as f32 / request.response_time_counter as f32
                    ),
                    response_time_minimum: request.min_response_time,
                    response_time_maximum: request.max_response_time,
                    requests_per_second: format!("{:.2}", requests_per_second),
                    failures_per_second: format!("{:.2}", failures_per_second),
                });

                // Prepare per-response metrics.
                response_metrics.push(report::get_response_metric(
                    &method,
                    &name,
                    &request.response_times,
                    request.response_time_counter,
                    request.min_response_time,
                    request.max_response_time,
                ));

                // Collect aggregated request and response metrics.
                aggregate_total_count += total_request_count;
                aggregate_fail_count += request.fail_count;
                aggregate_response_time_counter += request.total_response_time;
                aggregate_response_time_minimum = metrics::update_min_time(
                    aggregate_response_time_minimum,
                    request.min_response_time,
                );
                aggregate_response_time_maximum = metrics::update_max_time(
                    aggregate_response_time_maximum,
                    request.max_response_time,
                );
                aggregate_response_times =
                    metrics::merge_times(aggregate_response_times, request.response_times.clone());
            }

            // Prepare aggregate per-request metrics.
            let (aggregate_requests_per_second, aggregate_failures_per_second) =
                metrics::per_second_calculations(
                    self.metrics.duration,
                    aggregate_total_count,
                    aggregate_fail_count,
                );
            request_metrics.push(report::RequestMetric {
                method: "".to_string(),
                name: "Aggregated".to_string(),
                number_of_requests: aggregate_total_count,
                number_of_failures: aggregate_fail_count,
                response_time_average: format!(
                    "{:.2}",
                    aggregate_response_time_counter as f32 / aggregate_total_count as f32
                ),
                response_time_minimum: aggregate_response_time_minimum,
                response_time_maximum: aggregate_response_time_maximum,
                requests_per_second: format!("{:.2}", aggregate_requests_per_second),
                failures_per_second: format!("{:.2}", aggregate_failures_per_second),
            });

            // Prepare aggregate per-response metrics.
            response_metrics.push(report::get_response_metric(
                "",
                "Aggregated",
                &aggregate_response_times,
                aggregate_total_count,
                aggregate_response_time_minimum,
                aggregate_response_time_maximum,
            ));

            // Compile the request metrics template.
            let mut requests_rows = Vec::new();
            for metric in request_metrics {
                requests_rows.push(report::request_metrics_row(metric));
            }

            // Compile the response metrics template.
            let mut responses_rows = Vec::new();
            for metric in response_metrics {
                responses_rows.push(report::response_metrics_row(metric));
            }

            // Only build the tasks template if --no-task-metrics isn't enabled.
            let tasks_template: String;
            if !self.configuration.no_task_metrics {
                let mut task_metrics = Vec::new();
                let mut aggregate_total_count = 0;
                let mut aggregate_fail_count = 0;
                let mut aggregate_task_time_counter: usize = 0;
                let mut aggregate_task_time_minimum: usize = 0;
                let mut aggregate_task_time_maximum: usize = 0;
                let mut aggregate_task_times: BTreeMap<usize, usize> = BTreeMap::new();
                for (task_set_counter, task_set) in self.metrics.tasks.iter().enumerate() {
                    for (task_counter, task) in task_set.iter().enumerate() {
                        if task_counter == 0 {
                            // Only the taskset_name is used for task sets.
                            task_metrics.push(report::TaskMetric {
                                is_task_set: true,
                                task: "".to_string(),
                                name: task.taskset_name.to_string(),
                                number_of_requests: 0,
                                number_of_failures: 0,
                                response_time_average: "".to_string(),
                                response_time_minimum: 0,
                                response_time_maximum: 0,
                                requests_per_second: "".to_string(),
                                failures_per_second: "".to_string(),
                            });
                        }
                        let total_run_count = task.success_count + task.fail_count;
                        let (requests_per_second, failures_per_second) =
                            metrics::per_second_calculations(
                                self.metrics.duration,
                                total_run_count,
                                task.fail_count,
                            );
                        let average = match task.counter {
                            0 => 0.00,
                            _ => task.total_time as f32 / task.counter as f32,
                        };
                        task_metrics.push(report::TaskMetric {
                            is_task_set: false,
                            task: format!("{}.{}", task_set_counter, task_counter),
                            name: task.task_name.to_string(),
                            number_of_requests: total_run_count,
                            number_of_failures: task.fail_count,
                            response_time_average: format!("{:.2}", average),
                            response_time_minimum: task.min_time,
                            response_time_maximum: task.max_time,
                            requests_per_second: format!("{:.2}", requests_per_second),
                            failures_per_second: format!("{:.2}", failures_per_second),
                        });

                        aggregate_total_count += total_run_count;
                        aggregate_fail_count += task.fail_count;
                        aggregate_task_times =
                            metrics::merge_times(aggregate_task_times, task.times.clone());
                        aggregate_task_time_counter += &task.counter;
                        aggregate_task_time_minimum =
                            metrics::update_min_time(aggregate_task_time_minimum, task.min_time);
                        aggregate_task_time_maximum =
                            metrics::update_max_time(aggregate_task_time_maximum, task.max_time);
                    }
                }

                let (aggregate_requests_per_second, aggregate_failures_per_second) =
                    metrics::per_second_calculations(
                        self.metrics.duration,
                        aggregate_total_count,
                        aggregate_fail_count,
                    );
                task_metrics.push(report::TaskMetric {
                    is_task_set: false,
                    task: "".to_string(),
                    name: "Aggregated".to_string(),
                    number_of_requests: aggregate_total_count,
                    number_of_failures: aggregate_fail_count,
                    response_time_average: format!(
                        "{:.2}",
                        aggregate_response_time_counter as f32 / aggregate_total_count as f32
                    ),
                    response_time_minimum: aggregate_task_time_minimum,
                    response_time_maximum: aggregate_task_time_maximum,
                    requests_per_second: format!("{:.2}", aggregate_requests_per_second),
                    failures_per_second: format!("{:.2}", aggregate_failures_per_second),
                });
                let mut tasks_rows = Vec::new();
                // Compile the task metrics template.
                for metric in task_metrics {
                    tasks_rows.push(report::task_metrics_row(metric));
                }

                tasks_template = report::task_metrics_template(&tasks_rows.join("\n"));
            } else {
                tasks_template = "".to_string();
            }

            // Only build the tasks template if --no-task-metrics isn't enabled.
            let errors_template: String;
            if !self.metrics.errors.is_empty() {
                let mut error_rows = Vec::new();
                for error in self.metrics.errors.values() {
                    error_rows.push(report::error_row(error));
                }
                errors_template = report::errors_template(&error_rows.join("\n"));
            } else {
                errors_template = "".to_string();
            }

            // Only build the status_code template if --status-codes is enabled.
            let status_code_template: String;
            if self.configuration.status_codes {
                let mut status_code_metrics = Vec::new();
                let mut aggregated_status_code_counts: HashMap<u16, usize> = HashMap::new();
                for (request_key, request) in self.metrics.requests.iter().sorted() {
                    let method = format!("{:?}", request.method);
                    // The request_key is "{method} {name}", so by stripping the "{method} "
                    // prefix we get the name.
                    // @TODO: consider storing the name as a field in GooseRequest.
                    let name = request_key
                        .strip_prefix(&format!("{:?} ", request.method))
                        .unwrap()
                        .to_string();

                    // Build a list of status codes, and update the aggregate record.
                    let codes = metrics::prepare_status_codes(
                        &request.status_code_counts,
                        &mut Some(&mut aggregated_status_code_counts),
                    );

                    // Add a row of data for the status code table.
                    status_code_metrics.push(report::StatusCodeMetric {
                        method,
                        name,
                        status_codes: codes,
                    });
                }

                // Build a list of aggregate status codes.
                let aggregated_codes =
                    metrics::prepare_status_codes(&aggregated_status_code_counts, &mut None);

                // Add a final row of aggregate data for the status code table.
                status_code_metrics.push(report::StatusCodeMetric {
                    method: "".to_string(),
                    name: "Aggregated".to_string(),
                    status_codes: aggregated_codes,
                });

                // Compile the status_code metrics rows.
                let mut status_code_rows = Vec::new();
                for metric in status_code_metrics {
                    status_code_rows.push(report::status_code_metrics_row(metric));
                }

                // Compile the status_code metrics template.
                status_code_template =
                    report::status_code_metrics_template(&status_code_rows.join("\n"));
            } else {
                // If --status-codes is not enabled, return an empty template.
                status_code_template = "".to_string();
            }

            // Compile the report template.
            let report = report::build_report(
                &start_time,
                &end_time,
                &host,
                report::GooseReportTemplates {
                    requests_template: &requests_rows.join("\n"),
                    responses_template: &responses_rows.join("\n"),
                    tasks_template: &tasks_template,
                    status_codes_template: &status_code_template,
                    errors_template: &errors_template,
                },
            );

            // Write the report to file.
            if let Err(e) = report_file.write(report.as_ref()).await {
                return Err(GooseError::InvalidOption {
                    option: "--report-file".to_string(),
                    value: self.get_report_file_path().unwrap(),
                    detail: format!("Failed to create report file: {}", e),
                });
            };
            // Be sure the file flushes to disk.
            report_file.flush().await?;

            info!(
                "wrote html report file to: {}",
                self.get_report_file_path().unwrap()
            );
        }

        Ok(())
    }

    /// Called internally in local-mode and gaggle-mode.
    async fn start_attack(mut self, socket: Option<Socket>) -> Result<GooseAttack, GooseError> {
        trace!("start_attack: socket({:?})", socket);

        // The GooseAttackRunState is used while spawning and running the
        // GooseUser threads that generate the load test.
        let mut goose_attack_run_state = self
            .initialize_attack(socket)
            .await
            .expect("failed to initialize GooseAttackRunState");

        // Only initialize once, then change to the next attack phase.
        self.set_attack_phase(&mut goose_attack_run_state, AttackPhase::Starting);

        // Start a timer to track when to next synchronize the metrics.
        let mut sync_metrics_timer = std::time::Instant::now();
        // Sync at least as often as we display metrics, or every ten seconds.
        let mut sync_every = self.configuration.running_metrics.unwrap_or(10);
        if sync_every > 10 {
            sync_every = 10;
        }

        loop {
            match self.attack_phase {
                // Start spawning GooseUser threads.
                AttackPhase::Starting => self
                    .spawn_attack(&mut goose_attack_run_state)
                    .await
                    .expect("failed to start GooseAttack"),
                // Now that all GooseUser threads started, run the load test.
                AttackPhase::Running => self.monitor_attack(&mut goose_attack_run_state).await?,
                // Stop all GooseUser threads and clean up.
                AttackPhase::Stopping => {
                    self.stop_attack(&mut goose_attack_run_state).await?;
                    self.sync_metrics(&mut goose_attack_run_state).await?;
                    self.write_html_report(&mut goose_attack_run_state).await?;
                    break;
                }
                _ => panic!("GooseAttack entered an impossible phase"),
            }
            // Synchronize metrics if enough time has elapsed.
            if util::timer_expired(sync_metrics_timer, sync_every) {
                self.sync_metrics(&mut goose_attack_run_state).await?;
                sync_metrics_timer = std::time::Instant::now();
            }
        }

        Ok(self)
    }

    async fn receive_metrics(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<bool, GooseError> {
        let mut received_message = false;
        let mut message = goose_attack_run_state.metrics_rx.try_recv();

        while message.is_ok() {
            received_message = true;
            match message.unwrap() {
                GooseMetric::Request(raw_request) => {
                    // Options should appear above, search for formatted_log.
                    let formatted_log = match self.configuration.metrics_format.as_str() {
                        // Use serde_json to create JSON.
                        "json" => json!(raw_request).to_string(),
                        // Manually create CSV, library doesn't support single-row string conversion.
                        "csv" => GooseAttack::prepare_csv(&raw_request, goose_attack_run_state),
                        // Raw format is Debug output for GooseRawRequest structure.
                        "raw" => format!("{:?}", raw_request),
                        _ => unreachable!(),
                    };
                    if let Some(file) = goose_attack_run_state.requests_file.as_mut() {
                        match file.write(format!("{}\n", formatted_log).as_ref()).await {
                            Ok(_) => (),
                            Err(e) => {
                                warn!(
                                    "failed to write metrics to {}: {}",
                                    // Unwrap is safe as we can't get here unless a requests file path
                                    // is defined.
                                    self.get_requests_file_path().unwrap(),
                                    e
                                );
                            }
                        }
                    }

                    // If there was an error, store it.
                    if !raw_request.error.is_empty() {
                        self.record_error(&raw_request);
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
                GooseMetric::Error(raw_error) => {
                    // Recreate the string used to uniquely identify errors.
                    let error_key = format!(
                        "{}.{:?}.{}",
                        raw_error.error, raw_error.method, raw_error.name
                    );
                    let mut merge_error = match self.metrics.errors.get(&error_key) {
                        Some(error) => error.clone(),
                        None => GooseErrorMetric::new(
                            raw_error.method.clone(),
                            raw_error.name.to_string(),
                            raw_error.error.to_string(),
                        ),
                    };
                    merge_error.occurrences += raw_error.occurrences;
                    self.metrics
                        .errors
                        .insert(error_key.to_string(), merge_error);
                }
                GooseMetric::Task(raw_task) => {
                    // Store a new metric.
                    self.metrics.tasks[raw_task.taskset_index][raw_task.task_index]
                        .set_time(raw_task.run_time, raw_task.success);
                }
            }
            message = goose_attack_run_state.metrics_rx.try_recv();
        }

        Ok(received_message)
    }

    /// Update error metrics.
    pub fn record_error(&mut self, raw_request: &GooseRawRequest) {
        // If the error summary is disabled, return immediately without collecting errors.
        if self.configuration.no_error_summary {
            return;
        }

        // Create a string to uniquely identify errors for tracking metrics.
        let error_string = format!(
            "{}.{:?}.{}",
            raw_request.error, raw_request.method, raw_request.name
        );

        let mut error_metrics = match self.metrics.errors.get(&error_string) {
            // We've seen this error before.
            Some(m) => m.clone(),
            // First time we've seen this error.
            None => GooseErrorMetric::new(
                raw_request.method.clone(),
                raw_request.name.to_string(),
                raw_request.error.to_string(),
            ),
        };
        error_metrics.occurrences += 1;
        self.metrics.errors.insert(error_string, error_metrics);
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
///  - GooseDefault::RequestsFile
///  - GooseDefault::RequestsFormat
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
///  - GooseDefault::RunningMetrics
///  - GooseDefault::LogLevel
///  - GooseDefault::Verbose
///  - GooseDefault::ThrottleRequests
///  - GooseDefault::ExpectWorkers
///  - GooseDefault::ManagerBindPort
///  - GooseDefault::ManagerPort
///
/// The following run-time flags can be configured with a custom default using a
/// `bool` (and otherwise default to `false`).
///  - GooseDefault::NoResetMetrics
///  - GooseDefault::NoMetrics
///  - GooseDefault::NoTaskMetrics
///  - GooseDefault::NoErrorSummary
///  - GooseDefault::NoDebugBody
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
///         .set_default(GooseDefault::NoResetMetrics, true)?
///         .set_default(GooseDefault::Verbose, 1)?
///         .set_default(GooseDefault::RequestsFile, "goose-metrics.log")?;
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
            GooseDefault::HatchRate => self.defaults.hatch_rate = Some(value.to_string()),
            GooseDefault::Host => self.defaults.host = Some(value.to_string()),
            GooseDefault::LogFile => self.defaults.log_file = Some(value.to_string()),
            GooseDefault::ReportFile => self.defaults.report_file = Some(value.to_string()),
            GooseDefault::RequestsFile => self.defaults.requests_file = Some(value.to_string()),
            GooseDefault::RequestsFormat => self.defaults.metrics_format = Some(value.to_string()),
            GooseDefault::DebugFile => self.defaults.debug_file = Some(value.to_string()),
            GooseDefault::DebugFormat => self.defaults.debug_format = Some(value.to_string()),
            GooseDefault::ManagerBindHost => {
                self.defaults.manager_bind_host = Some(value.to_string())
            }
            GooseDefault::ManagerHost => self.defaults.manager_host = Some(value.to_string()),
            // Otherwise display a helpful and explicit error.
            GooseDefault::Users
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
            GooseDefault::RunningMetrics
            | GooseDefault::NoResetMetrics
            | GooseDefault::NoMetrics
            | GooseDefault::NoTaskMetrics
            | GooseDefault::NoErrorSummary
            | GooseDefault::NoDebugBody
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
            GooseDefault::RunTime => self.defaults.run_time = Some(value),
            GooseDefault::RunningMetrics => self.defaults.running_metrics = Some(value),
            GooseDefault::LogLevel => self.defaults.log_level = Some(value as u8),
            GooseDefault::Verbose => self.defaults.verbose = Some(value as u8),
            GooseDefault::ThrottleRequests => self.defaults.throttle_requests = Some(value),
            GooseDefault::ExpectWorkers => self.defaults.expect_workers = Some(value as u16),
            GooseDefault::ManagerBindPort => self.defaults.manager_bind_port = Some(value as u16),
            GooseDefault::ManagerPort => self.defaults.manager_port = Some(value as u16),
            // Otherwise display a helpful and explicit error.
            GooseDefault::Host
            | GooseDefault::HatchRate
            | GooseDefault::LogFile
            | GooseDefault::ReportFile
            | GooseDefault::RequestsFile
            | GooseDefault::RequestsFormat
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
            GooseDefault::NoResetMetrics
            | GooseDefault::NoMetrics
            | GooseDefault::NoTaskMetrics
            | GooseDefault::NoErrorSummary
            | GooseDefault::NoDebugBody
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
            GooseDefault::NoResetMetrics => self.defaults.no_reset_metrics = Some(value),
            GooseDefault::NoMetrics => self.defaults.no_metrics = Some(value),
            GooseDefault::NoTaskMetrics => self.defaults.no_task_metrics = Some(value),
            GooseDefault::NoErrorSummary => self.defaults.no_error_summary = Some(value),
            GooseDefault::NoDebugBody => self.defaults.no_debug_body = Some(value),
            GooseDefault::StatusCodes => self.defaults.status_codes = Some(value),
            GooseDefault::StickyFollow => self.defaults.sticky_follow = Some(value),
            GooseDefault::Manager => self.defaults.manager = Some(value),
            GooseDefault::NoHashCheck => self.defaults.no_hash_check = Some(value),
            GooseDefault::Worker => self.defaults.worker = Some(value),
            // Otherwise display a helpful and explicit error.
            GooseDefault::Host
            | GooseDefault::LogFile
            | GooseDefault::ReportFile
            | GooseDefault::RequestsFile
            | GooseDefault::RequestsFormat
            | GooseDefault::RunningMetrics
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
    pub hatch_rate: Option<String>,
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

    /// How often to optionally print running metrics
    #[options(no_short, meta = "TIME")]
    pub running_metrics: Option<usize>,
    /// Doesn't reset metrics after all users have started
    #[options(no_short)]
    pub no_reset_metrics: bool,
    /// Doesn't track metrics
    #[options(no_short)]
    pub no_metrics: bool,
    /// Doesn't track task metrics
    #[options(no_short)]
    pub no_task_metrics: bool,
    /// Doesn't display an error summary
    #[options(no_short)]
    pub no_error_summary: bool,
    /// Create an html-formatted report
    #[options(meta = "NAME")]
    pub report_file: String,
    /// Sets requests log file name
    #[options(short = "m", meta = "NAME")]
    pub requests_file: String,
    /// Sets requests log format (csv, json, raw)
    #[options(no_short, meta = "FORMAT")]
    pub metrics_format: String,
    /// Sets debug log file name
    #[options(short = "d", meta = "NAME")]
    pub debug_file: String,
    /// Sets debug log format (json, raw)
    #[options(no_short, meta = "FORMAT")]
    pub debug_format: String,
    /// Do not include the response body in the debug log
    #[options(no_short)]
    pub no_debug_body: bool,
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
        let hatch_rate = "2".to_string();
        let log_level: usize = 1;
        let log_file = "custom-goose.log".to_string();
        let verbose: usize = 0;
        let report_file = "custom-goose-report.html".to_string();
        let requests_file = "custom-goose-metrics.log".to_string();
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
            .set_default(GooseDefault::HatchRate, hatch_rate.as_str())
            .unwrap()
            .set_default(GooseDefault::LogLevel, log_level)
            .unwrap()
            .set_default(GooseDefault::LogFile, log_file.as_str())
            .unwrap()
            .set_default(GooseDefault::Verbose, verbose)
            .unwrap()
            .set_default(GooseDefault::RunningMetrics, 15)
            .unwrap()
            .set_default(GooseDefault::NoResetMetrics, true)
            .unwrap()
            .set_default(GooseDefault::NoMetrics, true)
            .unwrap()
            .set_default(GooseDefault::NoTaskMetrics, true)
            .unwrap()
            .set_default(GooseDefault::NoErrorSummary, true)
            .unwrap()
            .set_default(GooseDefault::ReportFile, report_file.as_str())
            .unwrap()
            .set_default(GooseDefault::RequestsFile, requests_file.as_str())
            .unwrap()
            .set_default(GooseDefault::RequestsFormat, metrics_format.as_str())
            .unwrap()
            .set_default(GooseDefault::DebugFile, debug_file.as_str())
            .unwrap()
            .set_default(GooseDefault::DebugFormat, debug_format.as_str())
            .unwrap()
            .set_default(GooseDefault::NoDebugBody, true)
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
        assert!(goose_attack.defaults.no_debug_body == Some(true));
        assert!(goose_attack.defaults.verbose == Some(verbose as u8));
        assert!(goose_attack.defaults.running_metrics == Some(15));
        assert!(goose_attack.defaults.no_reset_metrics == Some(true));
        assert!(goose_attack.defaults.no_metrics == Some(true));
        assert!(goose_attack.defaults.no_task_metrics == Some(true));
        assert!(goose_attack.defaults.no_error_summary == Some(true));
        assert!(goose_attack.defaults.report_file == Some(report_file));
        assert!(goose_attack.defaults.requests_file == Some(requests_file));
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
