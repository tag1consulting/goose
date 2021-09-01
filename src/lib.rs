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
//! goose = "0.13"
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
//! pub use goose::config::{GooseDefault, GooseDefaultType};
//! pub use goose::goose::{
//!     GooseTask, GooseTaskError, GooseTaskFunction, GooseTaskResult, GooseTaskSet, GooseUser,
//! };
//! pub use goose::metrics::{GooseCoordinatedOmissionMitigation, GooseMetrics};
//! pub use goose::{task, taskset, GooseAttack, GooseError, GooseScheduler};
//! ```
//!
//! Below your `main` function (which currently is the default `Hello, world!`), add
//! one or more load test functions. The names of these functions are arbitrary, but it is
//! recommended you use self-documenting names. Load test functions must be async. Each load
//! test function must accept a reference to a [`GooseUser`](./goose/struct.GooseUser.html) object
//! and return a [`GooseTaskResult`](./goose/type.GooseTaskResult.html). For example:
//!
//! ```rust
//! use goose::prelude::*;
//!
//! async fn loadtest_foo(user: &mut GooseUser) -> GooseTaskResult {
//!   let _goose = user.get("/path/to/foo").await?;
//!
//!   Ok(())
//! }
//! ```
//!
//! In the above example, we're using the [`GooseUser`](./goose/struct.GooseUser.html) helper
//! [`get`](./goose/struct.GooseUser.html#method.get) to load a path on the website we are load
//! testing. This helper creates a
//! [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
//! object and uses it to build and execute a request for the above path. If you want access
//! to the [`RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
//! object, you can instead use the [`goose_get`](./goose/struct.GooseUser.html#method.goose_get)
//! helper, for example to set a timeout on this specific request:
//!
//! ```rust
//! use std::time;
//!
//! use goose::prelude::*;
//!
//! async fn loadtest_bar(user: &mut GooseUser) -> GooseTaskResult {
//!     let request_builder = user.goose_get("/path/to/bar")?;
//!     let _goose = user.goose_send(request_builder.timeout(time::Duration::from_secs(3)), None).await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! We pass the [`RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
//! object to [`goose_send`](./goose/struct.GooseUser.html#method.goose_send) which builds and
//! executes it, also collecting useful metrics. The
//! [`.await`](https://doc.rust-lang.org/std/keyword.await.html) at the end is necessary as
//! [`goose_send`](./goose/struct.GooseUser.html#method.goose_send) is an async function.
//!
//! Once all our tasks are created, we edit the main function to initialize goose and register
//! the tasks. In this very simple example we only have two tasks to register, while in a real
//! load test you can have any number of task sets with any number of individual tasks.
//!
//! ```rust
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
//!         .set_default(GooseDefault::Host, "http://dev.local/")?
//!         // We set a default run time so this test runs to completion.
//!         .set_default(GooseDefault::RunTime, 1)?
//!         .execute()?;
//!
//!     Ok(())
//! }
//!
//! // A task function that loads `/path/to/foo`.
//! async fn loadtest_foo(user: &mut GooseUser) -> GooseTaskResult {
//!     let _goose = user.get("/path/to/foo").await?;
//!
//!     Ok(())
//! }
//!
//! // A task function that loads `/path/to/bar`.
//! async fn loadtest_bar(user: &mut GooseUser) -> GooseTaskResult {
//!     let _goose = user.get("/path/to/bar").await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! Goose now spins up a configurable number of users, each simulating a user on your
//! website. Thanks to [`reqwest`](https://docs.rs/reqwest/), each user maintains its own
//! web client state, handling cookies and more so your "users" can log in, fill out forms,
//! and more, as real users on your sites would do.
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
//! Goose can optionally display running metrics if started with `--running-metrics INT`
//! where INT is an integer value in seconds. For example, if Goose is started with
//! `--running-metrics 15` it will display running values approximately every 15 seconds.
//! Running metrics are broken into several tables. First are the per-task metrics which
//! are further split into two sections. The first section shows how many requests have
//! been made, how many of them failed (non-2xx response), and the corresponding per-second
//! rates.
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
//! The second table breaks down the same metrics by request instead of by Task. For
//! our simple load test, each Task only makes a single request, so the metrics are
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
//! Note that Goose respected the per-task weights we set, and `foo` (with a weight of 10)
//! is being loaded five times as often as `bar` (with a weight of 2). On average
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
//! ```bash
//!  ------------------------------------------------------------------------------
//!  Users: 2
//!  Target host: http://dev.local/
//!  Starting: 2021-08-12 15:42:23 - 2021-08-12 15:42:31 (duration: 00:00:08)
//!  During:  2021-08-12 15:42:31 - 2021-08-12 15:43:02 (duration: 00:00:30)
//!  Stopping: 2021-08-12 15:43:02 - 2021-08-12 15:43:02 (duration: 00:00:00)
//!
//!  goose v0.13.3
//!  ------------------------------------------------------------------------------
//! ```
//!
//! And the final table shows an overview of the load test configuration and
//! duration.
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
pub mod logger;
#[cfg(feature = "gaggle")]
mod manager;
pub mod metrics;
pub mod prelude;
mod report;
mod throttle;
mod user;
pub mod util;
#[cfg(feature = "gaggle")]
mod worker;

use chrono::prelude::*;
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
use std::{fmt, io, time};
use tokio::fs::File;
use tokio::runtime::Runtime;

use crate::config::{GooseConfiguration, GooseDefaults};
use crate::controller::{GooseControllerProtocol, GooseControllerRequest};
use crate::goose::{GaggleUser, GooseTask, GooseTaskSet, GooseUser, GooseUserCommand};
use crate::logger::{GooseLoggerJoinHandle, GooseLoggerTx};
use crate::metrics::{GooseMetric, GooseMetrics};
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

/// Internal representation of a weighted task list.
type WeightedGooseTasks = Vec<(usize, String)>;

/// Internal representation of unsequenced tasks.
type UnsequencedGooseTasks = Vec<GooseTask>;
/// Internal representation of sequenced tasks.
type SequencedGooseTasks = BTreeMap<usize, Vec<GooseTask>>;

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
    //std::convert::From<tokio::task::JoinError>
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
        min_wait: usize,
        // The specified maximum wait time.
        max_wait: usize,
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
    /// [`GooseAttack`](./struct.GooseAttack.html) has no [`GooseTaskSet`](./goose/struct.GooseTaskSet.html) defined.
    NoTaskSets {
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
            GooseError::NoTaskSets { .. } => "no task sets defined",
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
    /// [`GooseUser`](./goose/struct.GooseUser.html)s are launching and beginning to generate
    /// load.
    Starting,
    /// All [`GooseUser`](./goose/struct.GooseUser.html)s have launched and are generating load.
    Running,
    /// [`GooseUser`](./goose/struct.GooseUser.html)s are stopping.
    Stopping,
    /// Exiting the load test.
    Shutdown,
}

#[derive(Clone, Debug, PartialEq)]
/// Used to define the order [`GooseTaskSet`](./goose/struct.GooseTaskSet.html)s and
/// [`GooseTask`](./goose/struct.GooseTask.html)s are allocated.
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
    /// An optional task that is run one time before starting GooseUsers and running GooseTaskSets.
    test_start_task: Option<GooseTask>,
    /// An optional task that is run one time after all GooseUsers have finished.
    test_stop_task: Option<GooseTask>,
    /// A vector containing one copy of each GooseTaskSet defined by this load test.
    task_sets: Vec<GooseTaskSet>,
    /// A weighted vector containing a GooseUser object for each GooseUser that will run during this load test.
    weighted_users: Vec<GooseUser>,
    /// A weighted vector containing a lightweight GaggleUser object that is sent to all Workers if running in Gaggle mode.
    weighted_gaggle_users: Vec<GaggleUser>,
    /// Optional default values for Goose run-time options.
    defaults: GooseDefaults,
    /// Configuration object holding options set when launching the load test.
    configuration: GooseConfiguration,
    /// How long (in seconds) the load test should run.
    run_time: usize,
    /// The load test operates in only one of the following modes: StandAlone, Manager, or Worker.
    attack_mode: AttackMode,
    /// Which phase the load test is currently operating in.
    attack_phase: AttackPhase,
    /// Defines the order [`GooseTaskSet`](./goose/struct.GooseTaskSet.html)s and
    /// [`GooseTask`](./goose/struct.GooseTask.html)s are allocated.
    scheduler: GooseScheduler,
    /// When the load test started.
    started: Option<time::Instant>,
    /// All metrics merged together.
    metrics: GooseMetrics,
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
            attack_phase: AttackPhase::Idle,
            scheduler: GooseScheduler::RoundRobin,
            started: None,
            metrics: GooseMetrics::default(),
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
            test_start_task: None,
            test_stop_task: None,
            task_sets: Vec::new(),
            weighted_users: Vec::new(),
            weighted_gaggle_users: Vec::new(),
            defaults: GooseDefaults::default(),
            configuration,
            run_time: 0,
            attack_mode: AttackMode::Undefined,
            attack_phase: AttackPhase::Idle,
            scheduler: GooseScheduler::RoundRobin,
            started: None,
            metrics: GooseMetrics::default(),
        })
    }

    /// Define the order [`GooseTaskSet`](./goose/struct.GooseTaskSet.html)s are
    /// allocated to new [`GooseUser`](./goose/struct.GooseUser.html)s as they are
    /// launched.
    ///
    /// By default, [`GooseTaskSet`](./goose/struct.GooseTaskSet.html)s are allocated
    /// to new [`GooseUser`](./goose/struct.GooseUser.html)s in a round robin style.
    /// For example, if TaskSet A has a weight of 5, TaskSet B has a weight of 3, and
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
    /// In the following example, [`GooseTaskSet`](./goose/struct.GooseTaskSet.html)s
    /// are allocated to launching [`GooseUser`](./goose/struct.GooseUser.html)s in a
    /// random order. This means running the test multiple times can generate
    /// different amounts of load, as depending on your weighting rules you may
    /// have a different number of [`GooseUser`](./goose/struct.GooseUser.html)s
    /// running each [`GooseTaskSet`](./goose/struct.GooseTaskSet.html) each time.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// fn main() -> Result<(), GooseError> {
    ///     GooseAttack::initialize()?
    ///         .set_scheduler(GooseScheduler::Random)
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
    /// async fn a_task_1(user: &mut GooseUser) -> GooseTaskResult {
    ///     let _goose = user.get("/foo").await?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn b_task_1(user: &mut GooseUser) -> GooseTaskResult {
    ///     let _goose = user.get("/bar").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_scheduler(mut self, scheduler: GooseScheduler) -> Self {
        self.scheduler = scheduler;
        self
    }

    /// A load test must contain one or more [`GooseTaskSet`](./goose/struct.GooseTaskSet.html)s
    /// be registered into Goose's global state with this method for it to run.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
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
    /// async fn example_task(user: &mut GooseUser) -> GooseTaskResult {
    ///     let _goose = user.get("/foo").await?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn other_task(user: &mut GooseUser) -> GooseTaskResult {
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
    /// The [`GooseUser`](./goose/struct.GooseUser.html) used to run the `test_start`
    /// tasks is not preserved and does not otherwise affect the subsequent
    /// [`GooseUser`](./goose/struct.GooseUser.html)s that run the rest of the load
    /// test. For example, if the [`GooseUser`](./goose/struct.GooseUser.html)
    /// logs in during `test_start`, subsequent [`GooseUser`](./goose/struct.GooseUser.html)
    /// do not retain this session and are therefor not already logged in.
    ///
    /// When running in a distributed Gaggle, this task is only run one time by the
    /// Manager.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// fn main() -> Result<(), GooseError> {
    ///     GooseAttack::initialize()?
    ///         .test_start(task!(setup));
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn setup(user: &mut GooseUser) -> GooseTaskResult {
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
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// fn main() -> Result<(), GooseError> {
    ///     GooseAttack::initialize()?
    ///         .test_stop(task!(teardown));
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn teardown(user: &mut GooseUser) -> GooseTaskResult {
    ///     // do stuff to tear down the load test ...
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn test_stop(mut self, task: GooseTask) -> Self {
        self.test_stop_task = Some(task);
        self
    }

    /// Use configured GooseScheduler to build out a properly weighted list of
    /// [`GooseTaskSet`](./goose/struct.GooseTaskSet.html)s to be assigned to
    /// [`GooseUser`](./goose/struct.GooseUser.html)s
    fn allocate_task_sets(&mut self) -> Vec<usize> {
        trace!("allocate_task_sets");

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
            "allocating tasks and task sets with {:?} scheduler",
            self.scheduler
        );

        // Now build the weighted list with the appropriate scheduler.
        let mut weighted_task_sets = Vec::new();
        match self.scheduler {
            GooseScheduler::RoundRobin => {
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
            GooseScheduler::Serial => {
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
            GooseScheduler::Random => {
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

    /// Allocate a vector of weighted [`GooseUser`](./goose/struct.GooseUser.html)s.
    fn weight_task_set_users(&mut self) -> Result<Vec<GooseUser>, GooseError> {
        trace!("weight_task_set_users");

        let weighted_task_sets = self.allocate_task_sets();

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

    /// Allocate a vector of weighted [`GaggleUser`](./goose/struct.GaggleUser.html).
    fn prepare_worker_task_set_users(&mut self) -> Result<Vec<GaggleUser>, GooseError> {
        trace!("prepare_worker_task_set_users");

        let weighted_task_sets = self.allocate_task_sets();

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

    fn set_run_time(&mut self) -> Result<(), GooseError> {
        self.run_time = util::parse_timespan(&self.configuration.run_time);
        Ok(())
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
    /// fn main() -> Result<(), GooseError> {
    ///     let _goose_metrics = GooseAttack::initialize()?
    ///         .register_taskset(taskset!("ExampleTasks")
    ///             .register_task(task!(example_task).set_weight(2)?)
    ///             .register_task(task!(another_example_task).set_weight(3)?)
    ///             // Goose must run against a host, point to localhost so test starts.
    ///             .set_host("http://localhost")
    ///         )
    ///         // Exit after one second so test doesn't run forever.
    ///         .set_default(GooseDefault::RunTime, 1)?
    ///         .execute()?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn example_task(user: &mut GooseUser) -> GooseTaskResult {
    ///     let _goose = user.get("/foo").await?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn another_example_task(user: &mut GooseUser) -> GooseTaskResult {
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

        // Configure GooseConfiguration.
        self.configuration.configure(&self.defaults);

        // Validate GooseConfiguration.
        self.configuration.validate()?;

        // Configure the validated run time.
        self.set_run_time()?;

        // With a validated GooseConfiguration, enter a run mode.
        self.attack_mode = if self.configuration.manager {
            AttackMode::Manager
        } else if self.configuration.worker {
            AttackMode::Worker
        } else {
            AttackMode::StandAlone
        };

        // Confirm there's either a global host, or each task set has a host defined.
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

    // Returns OK(()) if there's a valid host, GooseError with details if not.
    fn validate_host(&mut self) -> Result<(), GooseError> {
        if self.configuration.host.is_empty() {
            for task_set in &self.task_sets {
                match &task_set.host {
                    Some(h) => {
                        if util::is_valid_host(h).is_ok() {
                            info!("host for {} configured: {}", task_set.name, h);
                        }
                    }
                    None => match &self.defaults.host {
                        Some(h) => {
                            if util::is_valid_host(h).is_ok() {
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

        // Apply weights to tasks in each task set.
        for task_set in &mut self.task_sets {
            let (weighted_on_start_tasks, weighted_tasks, weighted_on_stop_tasks) =
                allocate_tasks(task_set, &self.scheduler);
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

    // Invoke `test_start` tasks if existing.
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
                    let mut user = GooseUser::single(base_url, &self.configuration)?;
                    let function = &t.function;
                    let _ = function(&mut user).await;
                }
                // No test_start_task defined, nothing to do.
                None => (),
            }
        }

        Ok(())
    }

    // Invoke `test_stop` tasks if existing.
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
                    let mut user = GooseUser::single(base_url, &self.configuration)?;
                    let function = &t.function;
                    let _ = function(&mut user).await;
                }
                // No test_stop_task defined, nothing to do.
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

    // Spawn [`GooseUser`](./goose/struct.GooseUser.html) threads to generate a
    // [`GooseAttack`](./struct.GooseAttack.html).
    async fn spawn_attack(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        // If `startup_time` has been configured, calculate the hatch_rate.
        let hatch_rate = if self.configuration.startup_time != "0" {
            if let Some(users) = self.configuration.users {
                // Divide the number of users by the total time to start up to calculate the
                // hatch rate.
                users as f32 / util::parse_timespan(&self.configuration.startup_time) as f32
            } else {
                // Users have to be configured.
                unreachable!();
            }
        // Otherwise either `hatch_rate` was configured or Goose will default to launching
        // one GooseUser per second.
        } else {
            util::get_hatch_rate(self.configuration.hatch_rate.clone())
        };

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

                // Remember which task group this user is using.
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
        if self.weighted_users.is_empty() {
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
            // Also record a formattable timestamp, for human readable reports.
            self.metrics.started = Some(Local::now());
        }

        Ok(())
    }

    // Let the [`GooseAttack`](./struct.GooseAttack.html) run until the timer expires
    // (or the test is canceled), and then trigger a shut down.
    async fn monitor_attack(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        // Exit if run_time timer expires.
        if util::timer_expired(self.started.unwrap(), self.run_time) {
            self.set_attack_phase(goose_attack_run_state, AttackPhase::Stopping);
            self.metrics.stopping = Some(Local::now());
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

    async fn stop_running_users(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
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
                        GaggleMetrics::Tasks(self.metrics.tasks.clone()),
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
            self.metrics.initialize_task_metrics(
                &self.task_sets,
                &self.configuration,
                &self.defaults,
            )?;
            self.metrics.display_metrics = true;
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
                            let sleep_duration = tokio::time::Duration::from_millis(250);
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
                        self.set_attack_phase(&mut goose_attack_run_state, AttackPhase::Starting);
                        self.metrics.starting = Some(Local::now());
                    }
                }
                // In the Start phase, Goose launches GooseUser threads and starts a GooseAttack.
                AttackPhase::Starting => {
                    self.update_duration();
                    self.spawn_attack(&mut goose_attack_run_state)
                        .await
                        .expect("failed to start GooseAttack");
                }
                // In the Running phase, Goose maintains the configured GooseAttack.
                AttackPhase::Running => {
                    self.update_duration();
                    self.monitor_attack(&mut goose_attack_run_state).await?;
                }
                // In the Stopping phase, Goose stops all GooseUser threads and optionally reports
                // any collected metrics.
                AttackPhase::Stopping => {
                    // If displaying metrics, update internal state reflecting how long load test
                    // has been running.
                    self.update_duration();
                    // Tell all running GooseUsers to stop.
                    self.stop_running_users(&mut goose_attack_run_state).await?;
                    // Stop any running GooseUser threads.
                    self.stop_attack().await?;
                    // Collect all metrics sent by GooseUser threads.
                    self.sync_metrics(&mut goose_attack_run_state, true).await?;
                    // The load test is fully stopped at this point.
                    self.metrics.stopped = Some(Local::now());
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
                AttackPhase::Shutdown => break,
            }
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
                self.set_attack_phase(&mut goose_attack_run_state, AttackPhase::Stopping);
                self.metrics.stopping = Some(Local::now());
            }
        }

        Ok(self)
    }
}

/// Use the configured GooseScheduler to allocate all [`GooseTask`](./goose/struct.GooseTask.html)s
/// within the [`GooseTaskSet`](./goose/struct.GooseTaskSet.html) in the appropriate order. Returns
/// three set of ordered tasks: /// `on_start_tasks`, `tasks`, and `on_stop_tasks`. The
/// `on_start_tasks` are only run once when the [`GooseAttack`](./struct.GooseAttack.html) first
/// starts. Normal `tasks` are then run for the duration of the
/// [`GooseAttack`](./struct.GooseAttack.html). The `on_stop_tasks` finally are only run once when
/// the [`GooseAttack`](./struct.GooseAttack.html) stops.
fn allocate_tasks(
    task_set: &GooseTaskSet,
    scheduler: &GooseScheduler,
) -> (WeightedGooseTasks, WeightedGooseTasks, WeightedGooseTasks) {
    debug!(
        "allocating GooseTasks on GooseUsers with {:?} scheduler",
        scheduler
    );

    // A BTreeMap of Vectors allows us to group and sort tasks per sequence value.
    let mut sequenced_tasks: SequencedGooseTasks = BTreeMap::new();
    let mut sequenced_on_start_tasks: SequencedGooseTasks = BTreeMap::new();
    let mut sequenced_on_stop_tasks: SequencedGooseTasks = BTreeMap::new();
    let mut unsequenced_tasks: UnsequencedGooseTasks = Vec::new();
    let mut unsequenced_on_start_tasks: UnsequencedGooseTasks = Vec::new();
    let mut unsequenced_on_stop_tasks: UnsequencedGooseTasks = Vec::new();
    let mut u: usize = 0;
    let mut v: usize;

    // Find the greatest common divisor of all tasks in the task_set.
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

    // Apply weights to sequenced tasks.
    let weighted_sequenced_on_start_tasks = weight_sequenced_tasks(&sequenced_on_start_tasks, u);
    let weighted_sequenced_tasks = weight_sequenced_tasks(&sequenced_tasks, u);
    let weighted_sequenced_on_stop_tasks = weight_sequenced_tasks(&sequenced_on_stop_tasks, u);

    // Apply weights to unsequenced tasks.
    let (weighted_unsequenced_on_start_tasks, total_unsequenced_on_start_tasks) =
        weight_unsequenced_tasks(&unsequenced_on_start_tasks, u);
    let (weighted_unsequenced_tasks, total_unsequenced_tasks) =
        weight_unsequenced_tasks(&unsequenced_tasks, u);
    let (weighted_unsequenced_on_stop_tasks, total_unsequenced_on_stop_tasks) =
        weight_unsequenced_tasks(&unsequenced_on_stop_tasks, u);

    // Schedule sequenced tasks.
    let scheduled_sequenced_on_start_tasks =
        schedule_sequenced_tasks(&weighted_sequenced_on_start_tasks, scheduler);
    let scheduled_sequenced_tasks = schedule_sequenced_tasks(&weighted_sequenced_tasks, scheduler);
    let scheduled_sequenced_on_stop_tasks =
        schedule_sequenced_tasks(&weighted_sequenced_on_stop_tasks, scheduler);

    // Schedule unsequenced tasks.
    let scheduled_unsequenced_on_start_tasks = schedule_unsequenced_tasks(
        &weighted_unsequenced_on_start_tasks,
        total_unsequenced_on_start_tasks,
        scheduler,
    );
    let scheduled_unsequenced_tasks = schedule_unsequenced_tasks(
        &weighted_unsequenced_tasks,
        total_unsequenced_tasks,
        scheduler,
    );
    let scheduled_unsequenced_on_stop_tasks = schedule_unsequenced_tasks(
        &weighted_unsequenced_on_stop_tasks,
        total_unsequenced_on_stop_tasks,
        scheduler,
    );

    // Finally build a Vector of tuples: (task id, task name)
    let mut on_start_tasks = Vec::new();
    let mut tasks = Vec::new();
    let mut on_stop_tasks = Vec::new();

    // Sequenced tasks come first.
    for task in scheduled_sequenced_on_start_tasks.iter() {
        on_start_tasks.extend(vec![(*task, task_set.tasks[*task].name.to_string())])
    }
    for task in scheduled_sequenced_tasks.iter() {
        tasks.extend(vec![(*task, task_set.tasks[*task].name.to_string())])
    }
    for task in scheduled_sequenced_on_stop_tasks.iter() {
        on_stop_tasks.extend(vec![(*task, task_set.tasks[*task].name.to_string())])
    }

    // Unsequenced tasks come last.
    for task in scheduled_unsequenced_on_start_tasks.iter() {
        on_start_tasks.extend(vec![(*task, task_set.tasks[*task].name.to_string())])
    }
    for task in scheduled_unsequenced_tasks.iter() {
        tasks.extend(vec![(*task, task_set.tasks[*task].name.to_string())])
    }
    for task in scheduled_unsequenced_on_stop_tasks.iter() {
        on_stop_tasks.extend(vec![(*task, task_set.tasks[*task].name.to_string())])
    }

    // Return sequenced buckets of weighted usize pointers to and names of Goose Tasks
    (on_start_tasks, tasks, on_stop_tasks)
}

/// Build a weighted vector of vectors of unsequenced GooseTasks.
fn weight_unsequenced_tasks(unsequenced_tasks: &[GooseTask], u: usize) -> (Vec<Vec<usize>>, usize) {
    // Build a vector of vectors to be used to schedule users.
    let mut available_unsequenced_tasks = Vec::with_capacity(unsequenced_tasks.len());
    let mut total_tasks = 0;
    for task in unsequenced_tasks.iter() {
        // divide by greatest common divisor so vector is as short as possible
        let weight = task.weight / u;
        trace!(
            "{}: {} has weight of {} (reduced with gcd to {})",
            task.tasks_index,
            task.name,
            task.weight,
            weight
        );
        let weighted_tasks = vec![task.tasks_index; weight];
        available_unsequenced_tasks.push(weighted_tasks);
        total_tasks += weight;
    }
    (available_unsequenced_tasks, total_tasks)
}

/// Build a weighted vector of vectors of sequenced GooseTasks.
fn weight_sequenced_tasks(
    sequenced_tasks: &SequencedGooseTasks,
    u: usize,
) -> BTreeMap<usize, Vec<Vec<usize>>> {
    // Build a sequenced BTreeMap containing weighted vectors of GooseTasks.
    let mut available_sequenced_tasks = BTreeMap::new();
    // Step through sequences, each containing a bucket of all GooseTasks with the same
    // sequence value, allowing actual weighting to be done by weight_unsequenced_tasks().
    for (sequence, unsequenced_tasks) in sequenced_tasks.iter() {
        let (weighted_tasks, _total_weighted_tasks) =
            weight_unsequenced_tasks(unsequenced_tasks, u);
        available_sequenced_tasks.insert(*sequence, weighted_tasks);
    }

    available_sequenced_tasks
}

fn schedule_sequenced_tasks(
    available_sequenced_tasks: &BTreeMap<usize, Vec<Vec<usize>>>,
    scheduler: &GooseScheduler,
) -> Vec<usize> {
    let mut weighted_tasks: Vec<usize> = Vec::new();

    for (_sequence, tasks) in available_sequenced_tasks.iter() {
        let scheduled_tasks = schedule_unsequenced_tasks(tasks, tasks[0].len(), scheduler);
        weighted_tasks.extend(scheduled_tasks);
    }

    weighted_tasks
}

// Return a list of tasks in the order to be run.
fn schedule_unsequenced_tasks(
    available_unsequenced_tasks: &[Vec<usize>],
    total_tasks: usize,
    scheduler: &GooseScheduler,
) -> Vec<usize> {
    // Now build the weighted list with the appropriate scheduler.
    let mut weighted_tasks = Vec::new();

    match scheduler {
        GooseScheduler::RoundRobin => {
            // Allocate task sets round robin.
            let tasks_len = available_unsequenced_tasks.len();
            let mut available_tasks = available_unsequenced_tasks.to_owned();
            loop {
                // Tasks are contained in a vector of vectors. The outer vectors each
                // contain a different GooseTask, and the inner vectors contain each
                // instance of that specific GooseTask.
                for (task_index, tasks) in available_tasks.iter_mut().enumerate().take(tasks_len) {
                    if let Some(task) = tasks.pop() {
                        debug!("allocating task from Task {}", task_index);
                        weighted_tasks.push(task);
                    }
                }
                if weighted_tasks.len() >= total_tasks {
                    break;
                }
            }
        }
        GooseScheduler::Serial | GooseScheduler::Random => {
            // Allocate task sets serially in the weighted order defined. If the Random
            // scheduler is being used, tasks will get shuffled later.
            for (task_index, tasks) in available_unsequenced_tasks.iter().enumerate() {
                debug!(
                    "allocating all {} tasks from Task {}",
                    tasks.len(),
                    task_index
                );

                let mut tasks_clone = tasks.clone();
                if scheduler == &GooseScheduler::Random {
                    tasks_clone.shuffle(&mut thread_rng());
                }
                weighted_tasks.append(&mut tasks_clone);
            }
        }
    }

    weighted_tasks
}
