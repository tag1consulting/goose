//! Helpers and objects for building Goose load tests.
//!
//! Goose manages load tests with a series of objects:
//!
//! - [`GooseTaskSet`](./struct.GooseTaskSet.html) each user is assigned a task set, which is a collection of tasks.
//! - [`GooseTask`](./struct.GooseTask.html) tasks define one or more web requests and are assigned to task sets.
//! - [`GooseUser`](./struct.GooseUser.html) a user state responsible for repeatedly running all tasks in the assigned task set.
//! - [`GooseRequest`](./struct.GooseRequest.html) optional metrics collected for each URL/method pair.
//!
//! ## Creating Task Sets
//!
//! A [`GooseTaskSet`](./struct.GooseTaskSet.html) is created by passing in a `&str` name to the `new` function, for example:
//!
//! ```rust
//!     use goose::prelude::*;
//!
//!     let mut loadtest_tasks = taskset!("LoadtestTasks");
//! ```
//!
//! ### Task Set Weight
//!
//! A weight can be assigned to a task set, controlling how often it is assigned to user
//! threads. The larger the value of weight, the more it will be assigned to users. In the
//! following example, `FooTasks` will be assigned to users twice as often as `Bar` tasks.
//! We could have just added a weight of `2` to `FooTasks` and left the default weight of `1`
//! assigned to `BarTasks` for the same weighting:
//!
//! ```rust
//! use goose::prelude::*;
//!
//! fn main() -> Result<(), GooseError> {
//!     let mut foo_tasks = taskset!("FooTasks").set_weight(10)?;
//!     let mut bar_tasks = taskset!("BarTasks").set_weight(5)?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Task Set Host
//!
//! A default host can be assigned to a task set, which will be used only if the `--host`
//! CLI option is not set at run-time. For example, this can configure your load test to
//! run against your local development environment by default, allowing the `--host` option
//! to override host when you want to load test production. You can also assign different
//! hosts to different task sets if this is desirable:
//!
//! ```rust
//!     use goose::prelude::*;
//!
//!     let mut foo_tasks = taskset!("FooTasks").set_host("http://www.local");
//!     let mut bar_tasks = taskset!("BarTasks").set_host("http://www2.local");
//! ```
//!
//! ### Task Set Wait Time
//!
//! Wait time is specified as a low-high integer range. Each time a task completes in
//! the task set, the user will pause for a random number of seconds inclusively between
//! the low and high wait times. In the following example, users loading `foo` tasks will
//! sleep 0 to 3 seconds after each task completes, and users loading `bar` tasks will
//! sleep 5 to 10 seconds after each task completes.
//!
//! ```rust
//!     use goose::prelude::*;
//!
//!     let mut foo_tasks = taskset!("FooTasks").set_wait_time(0, 3).unwrap();
//!     let mut bar_tasks = taskset!("BarTasks").set_wait_time(5, 10).unwrap();
//! ```
//! ## Creating Tasks
//!
//! A [`GooseTask`](./struct.GooseTask.html) must include a pointer to a function which
//! will be executed each time the task is run.
//!
//! ```rust
//!     use goose::prelude::*;
//!
//!     let mut a_task = task!(task_function);
//!
//!     /// A very simple task that loads the front page.
//!     async fn task_function(user: &GooseUser) -> GooseTaskResult {
//!       let _goose = user.get("/").await?;
//!
//!       Ok(())
//!     }
//! ```
//!
//! ### Task Name
//!
//! A name can be assigned to a task, and will be displayed in metrics about all requests
//! made by the task.
//!
//! ```rust
//!     use goose::prelude::*;
//!
//!     let mut a_task = task!(task_function).set_name("a");
//!
//!     /// A very simple task that loads the front page.
//!     async fn task_function(user: &GooseUser) -> GooseTaskResult {
//!       let _goose = user.get("/").await?;
//!
//!       Ok(())
//!     }
//! ```
//!
//! ### Task Weight
//!
//! Individual tasks can be assigned a weight, controlling how often the task runs. The
//! larger the value of weight, the more it will run. In the following example, `a_task`
//! runs 3 times as often as `b_task`:
//!
//! ```rust
//! use goose::prelude::*;
//!
//! fn main() -> Result<(), GooseError> {
//!     let mut a_task = task!(a_task_function).set_weight(9)?;
//!     let mut b_task = task!(b_task_function).set_weight(3)?;
//!
//!     Ok(())
//! }
//!
//! /// A very simple task that loads the "a" page.
//! async fn a_task_function(user: &GooseUser) -> GooseTaskResult {
//!     let _goose = user.get("/a/").await?;
//!
//!     Ok(())
//! }
//!
//! /// Another very simple task that loads the "b" page.
//! async fn b_task_function(user: &GooseUser) -> GooseTaskResult {
//!     let _goose = user.get("/b/").await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Task Sequence
//!
//! Tasks can also be configured to run in a sequence. For example, a task with a sequence
//! value of `1` will always run before a task with a sequence value of `2`. Weight can
//! be applied to sequenced tasks, so for example a task with a weight of `2` and a sequence
//! of `1` will run two times before a task with a sequence of `2`. Task sets can contain
//! tasks with sequence values and without sequence values, and in this case all tasks with
//! a sequence value will run before tasks without a sequence value. In the following example,
//! `a_task` runs before `b_task`, which runs before `c_task`:
//!
//! ```rust
//!     use goose::prelude::*;
//!
//!     let mut a_task = task!(a_task_function).set_sequence(1);
//!     let mut b_task = task!(b_task_function).set_sequence(2);
//!     let mut c_task = task!(c_task_function);
//!
//!     /// A very simple task that loads the "a" page.
//!     async fn a_task_function(user: &GooseUser) -> GooseTaskResult {
//!       let _goose = user.get("/a/").await?;
//!
//!       Ok(())
//!     }
//!
//!     /// Another very simple task that loads the "b" page.
//!     async fn b_task_function(user: &GooseUser) -> GooseTaskResult {
//!       let _goose = user.get("/b/").await?;
//!
//!       Ok(())
//!     }
//!
//!     /// Another very simple task that loads the "c" page.
//!     async fn c_task_function(user: &GooseUser) -> GooseTaskResult {
//!       let _goose = user.get("/c/").await?;
//!
//!       Ok(())
//!     }
//! ```
//!
//! ### Task On Start
//!
//! Tasks can be flagged to only run when a user first starts. This can be useful if you'd
//! like your load test to use a logged-in user. It is possible to assign sequences and weights
//! to `on_start` functions if you want to have multiple tasks run in a specific order at start
//! time, and/or the tasks to run multiple times. A task can be flagged to run both on start
//! and on stop.
//!
//! ```rust
//!     use goose::prelude::*;
//!
//!     let mut a_task = task!(a_task_function).set_sequence(1).set_on_start();
//!
//!     /// A very simple task that loads the "a" page.
//!     async fn a_task_function(user: &GooseUser) -> GooseTaskResult {
//!       let _goose = user.get("/a/").await?;
//!
//!       Ok(())
//!     }
//! ```
//!
//! ### Task On Stop
//!
//! Tasks can be flagged to only run when a user stops. This can be useful if you'd like your
//! load test to simulate a user logging out when it finishes. It is possible to assign sequences
//! and weights to `on_stop` functions if you want to have multiple tasks run in a specific order
//! at stop time, and/or the tasks to run multiple times. A task can be flagged to run both on
//! start and on stop.
//!
//! ```rust
//!     use goose::prelude::*;
//!
//!     let mut b_task = task!(b_task_function).set_sequence(2).set_on_stop();
//!
//!     /// Another very simple task that loads the "b" page.
//!     async fn b_task_function(user: &GooseUser) -> GooseTaskResult {
//!       let _goose = user.get("/b/").await?;
//!
//!       Ok(())
//!     }
//! ```
//!
//! ## Controlling User
//!
//! When Goose starts, it creates one or more [`GooseUser`](./struct.GooseUser.html)s,
//! assigning a single [`GooseTaskSet`](./struct.GooseTaskSet.html) to each. This user is
//! then used to generate load. Behind the scenes, Goose is leveraging the
//! [`reqwest::client`](https://docs.rs/reqwest/*/reqwest/struct.Client.html)
//! to load web pages, and Goose can therefor do anything Reqwest can do.
//!
//! The most common request types are [`GET`](./struct.GooseUser.html#method.get) and
//! [`POST`](./struct.GooseUser.html#method.post), but [`HEAD`](./struct.GooseUser.html#method.head),
//! PUT, PATCH and [`DELETE`](./struct.GooseUser.html#method.delete) are also supported.
//!
//! ### GET
//!
//! A helper to make a `GET` request of a path and collect relevant metrics.
//! Automatically prepends the correct host.
//!
//! ```rust
//!     use goose::prelude::*;
//!
//!     let mut task = task!(get_function);
//!
//!     /// A very simple task that makes a GET request.
//!     async  fn get_function(user: &GooseUser) -> GooseTaskResult {
//!       let _goose = user.get("/path/to/foo/").await?;
//!
//!       Ok(())
//!     }
//! ```
//!
//! The returned response is a [`reqwest::Response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)
//! struct. You can use it as you would any Reqwest Response.
//!
//!
//! ### POST
//!
//! A helper to make a `POST` request of a string value to the path and collect relevant
//! metrics. Automatically prepends the correct host. The returned response is a
//! [`reqwest::Response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)
//!
//! ```rust
//!     use goose::prelude::*;
//!
//!     let mut task = task!(post_function);
//!
//!     /// A very simple task that makes a POST request.
//!     async fn post_function(user: &GooseUser) -> GooseTaskResult {
//!       let _goose = user.post("/path/to/foo/", "string value to post").await?;
//!
//!       Ok(())
//!     }
//! ```
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

use http::method::Method;
use http::StatusCode;
use reqwest::{header, Client, ClientBuilder, RequestBuilder, Response};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{self, AtomicUsize};
use std::sync::Arc;
use std::{future::Future, pin::Pin, time::Instant};
use tokio::sync::{Mutex, RwLock};
use url::Url;

use crate::metrics::GooseMetric;
use crate::{GooseConfiguration, GooseError, WeightedGooseTasks};

static APP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

/// task!(foo) expands to GooseTask::new(foo), but also does some boxing to work around a limitation in the compiler.
#[macro_export]
macro_rules! task {
    ($task_func:ident) => {
        GooseTask::new(std::sync::Arc::new(move |s| {
            std::boxed::Box::pin($task_func(s))
        }))
    };
}

/// taskset!("foo") expands to GooseTaskSet::new("foo").
#[macro_export]
macro_rules! taskset {
    ($name:tt) => {
        GooseTaskSet::new($name)
    };
}

/// Goose tasks return a result, which is empty on success, or contains a GooseTaskError
/// on error.
pub type GooseTaskResult = Result<(), GooseTaskError>;

/// Definition of all errors Goose Tasks can return.
#[derive(Debug)]
pub enum GooseTaskError {
    /// Contains a reqwest::Error.
    Reqwest(reqwest::Error),
    /// Contains a url::ParseError.
    Url(url::ParseError),
    /// The request failed. The `GooseRawRequest` that failed can be found in
    /// `.raw_request`.
    RequestFailed { raw_request: GooseRawRequest },
    /// The request was canceled (this happens when the throttle is enabled and
    /// the load test finished). A `GooseRawRequest` has not yet been constructed,
    // so is not available in this error.
    RequestCanceled { source: flume::SendError<bool> },
    /// There was an error sending the metrics for a request to the parent thread.
    /// The `GooseRawRequest` that was not recorded can be extracted from the error
    /// chain, available inside `.source`.
    MetricsFailed {
        source: flume::SendError<GooseMetric>,
    },
    /// Attempt to send debug detail to logger failed.
    /// There was an error sending debug information to the logger thread. The
    /// `GooseDebug` that was not logged can be extracted from the error chain,
    /// available inside `.source`.
    LoggerFailed {
        source: flume::SendError<Option<GooseDebug>>,
    },
    /// Attempted an unrecognized HTTP request method. The unrecognized method
    /// is available in `.method`.
    InvalidMethod { method: Method },
}
impl GooseTaskError {
    fn describe(&self) -> &str {
        match *self {
            GooseTaskError::Reqwest(_) => "reqwest::Error",
            GooseTaskError::Url(_) => "url::ParseError",
            GooseTaskError::RequestFailed { .. } => "request failed",
            GooseTaskError::RequestCanceled { .. } => {
                "request canceled because throttled load test ended"
            }
            GooseTaskError::MetricsFailed { .. } => "failed to send metrics to parent thread",
            GooseTaskError::LoggerFailed { .. } => "failed to send log message to logger thread",
            GooseTaskError::InvalidMethod { .. } => "unrecognized HTTP request method",
        }
    }
}

// Define how to display errors.
impl fmt::Display for GooseTaskError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            GooseTaskError::Reqwest(ref source) => {
                write!(f, "GooseTaskError: {} ({})", self.describe(), source)
            }
            GooseTaskError::Url(ref source) => {
                write!(f, "GooseTaskError: {} ({})", self.describe(), source)
            }
            GooseTaskError::RequestCanceled { ref source } => {
                write!(f, "GooseTaskError: {} ({})", self.describe(), source)
            }
            GooseTaskError::MetricsFailed { ref source } => {
                write!(f, "GooseTaskError: {} ({})", self.describe(), source)
            }
            GooseTaskError::LoggerFailed { ref source } => {
                write!(f, "GooseTaskError: {} ({})", self.describe(), source)
            }
            _ => write!(f, "GooseTaskError: {}", self.describe()),
        }
    }
}

// Define the lower level source of this error, if any.
impl std::error::Error for GooseTaskError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            GooseTaskError::Reqwest(ref source) => Some(source),
            GooseTaskError::Url(ref source) => Some(source),
            GooseTaskError::RequestCanceled { ref source } => Some(source),
            GooseTaskError::MetricsFailed { ref source } => Some(source),
            GooseTaskError::LoggerFailed { ref source } => Some(source),
            _ => None,
        }
    }
}

/// Auto-convert Reqwest errors.
impl From<reqwest::Error> for GooseTaskError {
    fn from(err: reqwest::Error) -> GooseTaskError {
        GooseTaskError::Reqwest(err)
    }
}

/// Auto-convert Url errors.
impl From<url::ParseError> for GooseTaskError {
    fn from(err: url::ParseError) -> GooseTaskError {
        GooseTaskError::Url(err)
    }
}

/// When the throttle is enabled and the load test ends, the throttle channel is
/// shut down. This causes a SendError, which gets automatically converted to
// `RequestCanceled`.
impl From<flume::SendError<bool>> for GooseTaskError {
    fn from(source: flume::SendError<bool>) -> GooseTaskError {
        GooseTaskError::RequestCanceled { source }
    }
}

/// Attempt to send metrics to the parent thread failed.
impl From<flume::SendError<GooseMetric>> for GooseTaskError {
    fn from(source: flume::SendError<GooseMetric>) -> GooseTaskError {
        GooseTaskError::MetricsFailed { source }
    }
}

/// Attempt to send logs to the logger thread failed.
impl From<flume::SendError<Option<GooseDebug>>> for GooseTaskError {
    fn from(source: flume::SendError<Option<GooseDebug>>) -> GooseTaskError {
        GooseTaskError::LoggerFailed { source }
    }
}

/// An individual task set.
#[derive(Clone, Hash)]
pub struct GooseTaskSet {
    /// The name of the task set.
    pub name: String,
    /// An integer reflecting where this task set lives in the internal `GooseTest.task_sets` vector.
    pub task_sets_index: usize,
    /// An integer value that controls the frequency that this task set will be assigned to a user.
    pub weight: usize,
    /// An integer value indicating the minimum number of seconds a user will sleep after running a task.
    pub min_wait: usize,
    /// An integer value indicating the maximum number of seconds a user will sleep after running a task.
    pub max_wait: usize,
    /// A vector containing one copy of each GooseTask that will run by users running this task set.
    pub tasks: Vec<GooseTask>,
    /// A vector of vectors of integers, controlling the sequence and order GooseTasks are run.
    pub weighted_tasks: WeightedGooseTasks,
    /// A vector of vectors of integers, controlling the sequence and order on_start GooseTasks are run when the user first starts.
    pub weighted_on_start_tasks: WeightedGooseTasks,
    /// A vector of vectors of integers, controlling the sequence and order on_stop GooseTasks are run when the user stops.
    pub weighted_on_stop_tasks: WeightedGooseTasks,
    /// An optional default host to run this TaskSet against.
    pub host: Option<String>,
}
impl GooseTaskSet {
    /// Creates a new GooseTaskSet. Once created, GooseTasks must be assigned to it, and finally it must be
    /// registered with the GooseAttack object. The returned object must be stored in a mutable value.
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut example_tasks = taskset!("ExampleTasks");
    /// ```
    pub fn new(name: &str) -> Self {
        trace!("new taskset: name: {}", &name);
        GooseTaskSet {
            name: name.to_string(),
            task_sets_index: usize::max_value(),
            weight: 1,
            min_wait: 0,
            max_wait: 0,
            tasks: Vec::new(),
            weighted_tasks: Vec::new(),
            weighted_on_start_tasks: Vec::new(),
            weighted_on_stop_tasks: Vec::new(),
            host: None,
        }
    }

    /// Registers a GooseTask with a GooseTaskSet, where it is stored in the GooseTaskSet.tasks vector. The
    /// function associated with the task will be run during the load test.
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut example_tasks = taskset!("ExampleTasks");
    ///     example_tasks.register_task(task!(a_task_function));
    ///
    ///     /// A very simple task that loads the "a" page.
    ///     async fn a_task_function(user: &GooseUser) -> GooseTaskResult {
    ///       let _goose = user.get("/a/").await?;
    ///
    ///       Ok(())
    ///     }
    /// ```
    pub fn register_task(mut self, mut task: GooseTask) -> Self {
        trace!("{} register_task: {}", self.name, task.name);
        task.tasks_index = self.tasks.len();
        self.tasks.push(task);
        self
    }

    /// Sets a weight on a task set. The larger the value of weight, the more often the task set will
    /// be assigned to users. For example, if you have task set foo with a weight of 3, and task set
    /// bar with a weight of 1, and you spin up a load test with 8 users, 6 of them will be running
    /// the foo task set, and 2 will be running the bar task set.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// fn main() -> Result<(), GooseError> {
    ///     let mut example_tasks = taskset!("ExampleTasks").set_weight(3)?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_weight(mut self, weight: usize) -> Result<Self, GooseError> {
        trace!("{} set_weight: {}", self.name, weight);
        if weight == 0 {
            return Err(GooseError::InvalidWeight {
                weight,
                detail: ("Weight must be set to at least 1.".to_string()),
            });
        }
        self.weight = weight;

        Ok(self)
    }

    /// Set a default host for the task set. If no `--host` flag is set when running the load test, this
    /// host will be pre-pended on all requests. For example, this can configure your load test to run
    /// against your local development environment by default, and the `--host` option could be used to
    /// override host when running the load test against production.
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut example_tasks = taskset!("ExampleTasks").set_host("http://10.1.1.42");
    /// ```
    pub fn set_host(mut self, host: &str) -> Self {
        trace!("{} set_host: {}", self.name, host);
        // Host validation happens in main() at startup.
        self.host = Some(host.to_string());
        self
    }

    /// Configure a task_set to to pause after running each task. The length of the pause will be randomly
    /// selected from `min_weight` to `max_wait` inclusively.  For example, if `min_wait` is `0` and
    /// `max_weight` is `2`, the user will randomly sleep for 0, 1 or 2 seconds after each task completes.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// fn main() -> Result<(), GooseError> {
    ///     taskset!("ExampleTasks").set_wait_time(0, 1)?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_wait_time(mut self, min_wait: usize, max_wait: usize) -> Result<Self, GooseError> {
        trace!(
            "{} set_wait time: min: {} max: {}",
            self.name,
            min_wait,
            max_wait
        );
        if min_wait > max_wait {
            return Err(GooseError::InvalidWaitTime {
                min_wait,
                max_wait,
                detail:
                    "The min_wait option can not be set to a larger value than the max_wait option."
                        .to_string(),
            });
        }
        self.min_wait = min_wait;
        self.max_wait = max_wait;

        Ok(self)
    }
}

/// Commands sent between the parent and user threads, and between manager and
/// worker processes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GooseUserCommand {
    /// Tell worker process to pause load test.
    WAIT,
    /// Tell worker process to start load test.
    RUN,
    /// Tell user thread to exit.
    EXIT,
}

/// Supported HTTP methods.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize, Ord, PartialOrd)]
pub enum GooseMethod {
    DELETE,
    GET,
    HEAD,
    PATCH,
    POST,
    PUT,
}

fn goose_method_from_method(method: Method) -> Result<GooseMethod, GooseTaskError> {
    Ok(match method {
        Method::DELETE => GooseMethod::DELETE,
        Method::GET => GooseMethod::GET,
        Method::HEAD => GooseMethod::HEAD,
        Method::PATCH => GooseMethod::PATCH,
        Method::POST => GooseMethod::POST,
        Method::PUT => GooseMethod::PUT,
        _ => {
            return Err(GooseTaskError::InvalidMethod { method });
        }
    })
}

/// The request that Goose is making. User threads send this data to the parent thread
/// when metrics are enabled. This request object must be provided to calls to
/// [`set_success`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_success)
/// or
/// [`set_failure`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_failure)
/// so Goose knows which request is being updated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GooseRawRequest {
    /// How many milliseconds the load test has been running.
    pub elapsed: u64,
    /// The method being used (ie, GET, POST, etc).
    pub method: GooseMethod,
    /// The optional name of the request.
    pub name: String,
    /// The full URL that was requested.
    pub url: String,
    /// The final full URL that was requested, after redirects.
    pub final_url: String,
    /// How many milliseconds the request took.
    pub redirected: bool,
    /// How many milliseconds the request took.
    pub response_time: u64,
    /// The HTTP response code (optional).
    pub status_code: u16,
    /// Whether or not the request was successful.
    pub success: bool,
    /// Whether or not we're updating a previous request, modifies how the parent thread records it.
    pub update: bool,
    /// Which GooseUser thread processed the request.
    pub user: usize,
    /// The optional error caused by this request.
    pub error: String,
}
impl GooseRawRequest {
    pub fn new(method: GooseMethod, name: &str, url: &str, elapsed: u128, user: usize) -> Self {
        GooseRawRequest {
            elapsed: elapsed as u64,
            method,
            name: name.to_string(),
            url: url.to_string(),
            final_url: "".to_string(),
            redirected: false,
            response_time: 0,
            status_code: 0,
            success: true,
            update: false,
            user,
            error: "".to_string(),
        }
    }

    // Record the final URL returned.
    fn set_final_url(&mut self, final_url: &str) {
        self.final_url = final_url.to_string();
        if self.final_url != self.url {
            self.redirected = true;
        }
    }

    fn set_response_time(&mut self, response_time: u128) {
        self.response_time = response_time as u64;
    }

    fn set_status_code(&mut self, status_code: Option<StatusCode>) {
        self.status_code = match status_code {
            Some(status_code) => status_code.as_u16(),
            None => 0,
        };
    }
}

/// Metrics collected about a path-method pair, (for example `/index`-`GET`).
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct GooseRequest {
    /// The path for which metrics are being collected.
    pub path: String,
    /// The method for which metrics are being collected.
    pub method: GooseMethod,
    /// Per-response-time counters, tracking how often pages are returned with this response time.
    pub response_times: BTreeMap<usize, usize>,
    /// The shortest response time seen so far.
    pub min_response_time: usize,
    /// The longest response time seen so far.
    pub max_response_time: usize,
    /// Total combined response times seen so far.
    pub total_response_time: usize,
    /// Total number of response times seen so far.
    pub response_time_counter: usize,
    /// Per-status-code counters, tracking how often each response code was returned for this request.
    pub status_code_counts: HashMap<u16, usize>,
    /// Total number of times this path-method request resulted in a successful (2xx) status code.
    pub success_count: usize,
    /// Total number of times this path-method request resulted in a non-successful (non-2xx) status code.
    pub fail_count: usize,
    /// Load test hash.
    pub load_test_hash: u64,
}
impl GooseRequest {
    /// Create a new GooseRequest object.
    pub fn new(path: &str, method: GooseMethod, load_test_hash: u64) -> Self {
        trace!("new request");
        GooseRequest {
            path: path.to_string(),
            method,
            response_times: BTreeMap::new(),
            min_response_time: 0,
            max_response_time: 0,
            total_response_time: 0,
            response_time_counter: 0,
            status_code_counts: HashMap::new(),
            success_count: 0,
            fail_count: 0,
            load_test_hash,
        }
    }

    /// Track response time.
    pub fn set_response_time(&mut self, response_time: u64) {
        // Perform this conversin only once, then re-use throughout this funciton.
        let response_time_usize = response_time as usize;

        // Update minimum if this one is fastest yet.
        if self.min_response_time == 0 || response_time_usize < self.min_response_time {
            self.min_response_time = response_time_usize;
        }

        // Update maximum if this one is slowest yet.
        if response_time_usize > self.max_response_time {
            self.max_response_time = response_time_usize;
        }

        // Update total_response time, adding in this one.
        self.total_response_time += response_time_usize;

        // Each time we store a new response time, increment counter by one.
        self.response_time_counter += 1;

        // Round the response time so we can combine similar times together and
        // minimize required memory to store and push upstream to the parent.
        let rounded_response_time: usize;

        // No rounding for 1-100ms response times.
        if response_time < 100 {
            rounded_response_time = response_time_usize;
        }
        // Round to nearest 10 for 100-500ms response times.
        else if response_time < 500 {
            rounded_response_time = ((response_time as f64 / 10.0).round() * 10.0) as usize;
        }
        // Round to nearest 100 for 500-1000ms response times.
        else if response_time < 1000 {
            rounded_response_time = ((response_time as f64 / 100.0).round() * 100.0) as usize;
        }
        // Round to nearest 1000 for all larger response times.
        else {
            rounded_response_time = ((response_time as f64 / 1000.0).round() * 1000.0) as usize;
        }

        let counter = match self.response_times.get(&rounded_response_time) {
            // We've seen this response_time before, increment counter.
            Some(c) => {
                debug!("got {:?} counter: {}", rounded_response_time, c);
                *c + 1
            }
            // First time we've seen this response time, initialize counter.
            None => {
                debug!("no match for counter: {}", rounded_response_time);
                1
            }
        };
        self.response_times.insert(rounded_response_time, counter);
        debug!("incremented {} counter: {}", rounded_response_time, counter);
    }

    /// Increment counter for status code, creating new counter if first time seeing status code.
    pub fn set_status_code(&mut self, status_code: u16) {
        let counter = match self.status_code_counts.get(&status_code) {
            // We've seen this status code before, increment counter.
            Some(c) => {
                debug!("got {:?} counter: {}", status_code, c);
                *c + 1
            }
            // First time we've seen this status code, initialize counter.
            None => {
                debug!("no match for counter: {}", status_code);
                1
            }
        };
        self.status_code_counts.insert(status_code, counter);
        debug!("incremented {} counter: {}", status_code, counter);
    }
}
impl Ord for GooseRequest {
    fn cmp(&self, other: &Self) -> Ordering {
        (&self.method, &self.path).cmp(&(&other.method, &other.path))
    }
}
impl PartialOrd for GooseRequest {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The response to a GooseRequest
#[derive(Debug)]
pub struct GooseResponse {
    pub request: GooseRawRequest,
    pub response: Result<Response, reqwest::Error>,
}
impl GooseResponse {
    pub fn new(request: GooseRawRequest, response: Result<Response, reqwest::Error>) -> Self {
        GooseResponse { request, response }
    }
}

/// Object created by log_debug() and written to log to assist in debugging.
#[derive(Debug, Serialize)]
pub struct GooseDebug {
    /// String to identify the source of the log message.
    pub tag: String,
    /// Optional request made.
    pub request: Option<GooseRawRequest>,
    /// Optional headers returned by server.
    pub header: Option<String>,
    /// Optional body text returned by server.
    pub body: Option<String>,
}
impl GooseDebug {
    fn new(
        tag: &str,
        request: Option<&GooseRawRequest>,
        header: Option<&header::HeaderMap>,
        body: Option<&str>,
    ) -> Self {
        GooseDebug {
            // Convert tag from &str to string.
            tag: tag.to_string(),
            // If request is defined, clone it.
            request: request.cloned(),
            // If header is defined, convert it to a string.
            header: header.map(|h| format!("{:?}", h)),
            // If header is defined, convert from &str to string.
            body: body.map(|b| b.to_string()),
        }
    }
}

/// The elements needed to build an individual user state on a Gaggle Worker.
#[derive(Debug, Clone)]
pub struct GaggleUser {
    /// An index into the internal `GooseTest.task_sets` vector, indicating which GooseTaskSet is running.
    pub task_sets_index: usize,
    /// The base URL to prepend to all relative paths.
    pub base_url: Arc<RwLock<Url>>,
    /// Minimum amount of time to sleep after running a task.
    pub min_wait: usize,
    /// Maximum amount of time to sleep after running a task.
    pub max_wait: usize,
    /// A local copy of the global GooseConfiguration.
    pub config: GooseConfiguration,
    /// Load test hash.
    pub load_test_hash: u64,
}
impl GaggleUser {
    /// Create a new user state.
    pub fn new(
        task_sets_index: usize,
        base_url: Url,
        min_wait: usize,
        max_wait: usize,
        configuration: &GooseConfiguration,
        load_test_hash: u64,
    ) -> Self {
        trace!("new gaggle user");
        GaggleUser {
            task_sets_index,
            base_url: Arc::new(RwLock::new(base_url)),
            min_wait,
            max_wait,
            config: configuration.clone(),
            load_test_hash,
        }
    }
}

/// An individual user state, repeatedly running all GooseTasks in a specific GooseTaskSet.
#[derive(Debug, Clone)]
pub struct GooseUser {
    /// The Instant when this GooseUser client started.
    pub started: Instant,
    /// An index into the internal `GooseTest.task_sets` vector, indicating which GooseTaskSet is running.
    pub task_sets_index: usize,
    /// Client used to make requests, managing sessions and cookies.
    pub client: Arc<Mutex<Client>>,
    /// Integer value tracking the sequenced bucket user is running tasks from.
    pub weighted_bucket: Arc<AtomicUsize>,
    /// Integer value tracking the current task user is running.
    pub weighted_bucket_position: Arc<AtomicUsize>,
    /// The base URL to prepend to all relative paths.
    pub base_url: Arc<RwLock<Url>>,
    /// Minimum amount of time to sleep after running a task.
    pub min_wait: usize,
    /// Maximum amount of time to sleep after running a task.
    pub max_wait: usize,
    /// A local copy of the global GooseConfiguration.
    pub config: GooseConfiguration,
    /// Channel to logger.
    pub debug_logger: Option<flume::Sender<Option<GooseDebug>>>,
    /// Channel to throttle.
    pub throttle: Option<flume::Sender<bool>>,
    /// Normal tasks are optionally throttled, test_start and test_stop tasks are not.
    pub is_throttled: bool,
    /// Channel to parent.
    pub channel_to_parent: Option<flume::Sender<GooseMetric>>,
    /// An index into the internal `GooseTest.weighted_users, indicating which weighted GooseTaskSet is running.
    pub weighted_users_index: usize,
    /// A weighted list of all tasks that run when the user first starts.
    pub weighted_on_start_tasks: WeightedGooseTasks,
    /// A weighted list of all tasks that this user runs once started.
    pub weighted_tasks: WeightedGooseTasks,
    /// A weighted list of all tasks that run when the user stops.
    pub weighted_on_stop_tasks: WeightedGooseTasks,
    /// Load test hash.
    pub load_test_hash: u64,
}
impl GooseUser {
    /// Create a new user state.
    pub fn new(
        task_sets_index: usize,
        base_url: Url,
        min_wait: usize,
        max_wait: usize,
        configuration: &GooseConfiguration,
        load_test_hash: u64,
    ) -> Result<Self, GooseError> {
        trace!("new user");
        let client = Client::builder()
            .user_agent(APP_USER_AGENT)
            .cookie_store(true)
            .build()?;

        Ok(GooseUser {
            started: Instant::now(),
            task_sets_index,
            client: Arc::new(Mutex::new(client)),
            weighted_bucket: Arc::new(AtomicUsize::new(0)),
            weighted_bucket_position: Arc::new(AtomicUsize::new(0)),
            base_url: Arc::new(RwLock::new(base_url)),
            min_wait,
            max_wait,
            config: configuration.clone(),
            debug_logger: None,
            throttle: None,
            is_throttled: true,
            channel_to_parent: None,
            // A value of max_value() indicates this user isn't fully initialized yet.
            weighted_users_index: usize::max_value(),
            weighted_on_start_tasks: Vec::new(),
            weighted_tasks: Vec::new(),
            weighted_on_stop_tasks: Vec::new(),
            load_test_hash,
        })
    }

    /// Create a new single-use user.
    pub fn single(base_url: Url, configuration: &GooseConfiguration) -> Result<Self, GooseError> {
        let mut single_user = GooseUser::new(0, base_url, 0, 0, configuration, 0)?;
        // Only one user, so index is 0.
        single_user.weighted_users_index = 0;
        // Do not throttle test_start (setup) and test_stop (teardown) tasks.
        single_user.is_throttled = false;

        Ok(single_user)
    }

    /// A helper that prepends a base_url to all relative paths.
    ///
    /// A base_url is determined per user thread, using the following order
    /// of precedence:
    ///  1. `--host` (host specified on the command line when running load test)
    ///  2. `GooseTaskSet.host` (default host defined for the current task set)
    ///  3. `GooseAttack.host` (default host defined for the current load test)
    pub async fn build_url(&self, path: &str) -> Result<String, GooseTaskError> {
        // If URL includes a host, simply use it.
        if let Ok(parsed_path) = Url::parse(path) {
            if let Some(_host) = parsed_path.host() {
                return Ok(path.to_string());
            }
        }

        // Otherwise use the base_url.
        Ok(self.base_url.read().await.join(path)?.to_string())
    }

    /// A helper to make a `GET` request of a path and collect relevant metrics.
    /// Automatically prepends the correct host.
    ///
    /// (If you need to set headers, change timeouts, or otherwise make use of the
    /// [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object, you can instead call `goose_get` which returns a RequestBuilder, then
    /// call `goose_send` to invoke the request.)
    ///
    /// Calls to `user.get` return a `GooseResponse` object which contains a copy of
    /// the request you made
    /// ([`goose.request`](https://docs.rs/goose/*/goose/goose/struct.GooseRawRequest)), and the response
    /// ([`goose.response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut task = task!(get_function);
    ///
    /// /// A very simple task that makes a GET request.
    /// async fn get_function(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.get("/path/to/foo/").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn get(&self, path: &str) -> Result<GooseResponse, GooseTaskError> {
        let request_builder = self.goose_get(path).await?;

        Ok(self.goose_send(request_builder, None).await?)
    }

    /// A helper to make a named `GET` request of a path and collect relevant metrics.
    /// Automatically prepends the correct host. Naming a request only affects collected
    /// metrics.
    ///
    /// Calls to `user.get_named` return a `GooseResponse` object which contains a copy of
    /// the request you made
    /// ([`goose.request`](https://docs.rs/goose/*/goose/goose/struct.GooseRawRequest)), and the response
    /// ([`goose.response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut task = task!(get_function);
    ///
    /// /// A very simple task that makes a GET request.
    /// async fn get_function(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.get_named("/path/to/foo/", "foo").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_named(
        &self,
        path: &str,
        request_name: &str,
    ) -> Result<GooseResponse, GooseTaskError> {
        let request_builder = self.goose_get(path).await?;

        Ok(self.goose_send(request_builder, Some(request_name)).await?)
    }

    /// A helper to make a `POST` request of a path and collect relevant metrics.
    /// Automatically prepends the correct host.
    ///
    /// (If you need to set headers, change timeouts, or otherwise make use of the
    /// [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object, you can instead call `goose_post` which returns a RequestBuilder, then
    /// call `goose_send` to invoke the request.)
    ///
    /// Calls to `user.post` return a `GooseResponse` object which contains a copy of
    /// the request you made
    /// ([`goose.request`](https://docs.rs/goose/*/goose/goose/struct.GooseRawRequest)), and the response
    /// ([`goose.response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut task = task!(post_function);
    ///
    /// /// A very simple task that makes a POST request.
    /// async fn post_function(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.post("/path/to/foo/", "BODY BEING POSTED").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn post(&self, path: &str, body: &str) -> Result<GooseResponse, GooseTaskError> {
        let request_builder = self.goose_post(path).await?.body(body.to_string());

        Ok(self.goose_send(request_builder, None).await?)
    }

    /// A helper to make a named `POST` request of a path and collect relevant metrics.
    /// Automatically prepends the correct host. Naming a request only affects collected
    /// metrics.
    ///
    /// Calls to `user.post` return a `GooseResponse` object which contains a copy of
    /// the request you made
    /// ([`goose.request`](https://docs.rs/goose/*/goose/goose/struct.GooseRawRequest)), and the response
    /// ([`goose.response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut task = task!(post_function);
    ///
    /// /// A very simple task that makes a POST request.
    /// async fn post_function(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.post_named("/path/to/foo/", "foo", "BODY BEING POSTED").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn post_named(
        &self,
        path: &str,
        request_name: &str,
        body: &str,
    ) -> Result<GooseResponse, GooseTaskError> {
        let request_builder = self.goose_post(path).await?.body(body.to_string());

        Ok(self.goose_send(request_builder, Some(request_name)).await?)
    }

    /// A helper to make a `HEAD` request of a path and collect relevant metrics.
    /// Automatically prepends the correct host.
    ///
    /// (If you need to set headers, change timeouts, or otherwise make use of the
    /// [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object, you can instead call `goose_head` which returns a RequestBuilder, then
    /// call `goose_send` to invoke the request.)
    ///
    /// Calls to `user.head` return a `GooseResponse` object which contains a copy of
    /// the request you made
    /// ([`goose.request`](https://docs.rs/goose/*/goose/goose/struct.GooseRawRequest)), and the response
    /// ([`goose.response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut task = task!(head_function);
    ///
    /// /// A very simple task that makes a HEAD request.
    /// async fn head_function(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.head("/path/to/foo/").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn head(&self, path: &str) -> Result<GooseResponse, GooseTaskError> {
        let request_builder = self.goose_head(path).await?;

        Ok(self.goose_send(request_builder, None).await?)
    }

    /// A helper to make a named `HEAD` request of a path and collect relevant metrics.
    /// Automatically prepends the correct host. Naming a request only affects collected
    /// metrics.
    ///
    /// Calls to `user.head` return a `GooseResponse` object which contains a copy of
    /// the request you made
    /// ([`goose.request`](https://docs.rs/goose/*/goose/goose/struct.GooseRawRequest)), and the response
    /// ([`goose.response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut task = task!(head_function);
    ///
    /// /// A very simple task that makes a HEAD request.
    /// async fn head_function(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.head_named("/path/to/foo/", "foo").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn head_named(
        &self,
        path: &str,
        request_name: &str,
    ) -> Result<GooseResponse, GooseTaskError> {
        let request_builder = self.goose_head(path).await?;

        Ok(self.goose_send(request_builder, Some(request_name)).await?)
    }

    /// A helper to make a `DELETE` request of a path and collect relevant metrics.
    /// Automatically prepends the correct host.
    ///
    /// (If you need to set headers, change timeouts, or otherwise make use of the
    /// [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object, you can instead call `goose_delete` which returns a RequestBuilder,
    /// then call `goose_send` to invoke the request.)
    ///
    /// Calls to `user.delete` return a `GooseResponse` object which contains a copy of
    /// the request you made
    /// ([`goose.request`](https://docs.rs/goose/*/goose/goose/struct.GooseRawRequest)), and the response
    /// ([`goose.response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut task = task!(delete_function);
    ///
    /// /// A very simple task that makes a DELETE request.
    /// async fn delete_function(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.delete("/path/to/foo/").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn delete(&self, path: &str) -> Result<GooseResponse, GooseTaskError> {
        let request_builder = self.goose_delete(path).await?;

        Ok(self.goose_send(request_builder, None).await?)
    }

    /// A helper to make a named `DELETE` request of a path and collect relevant metrics.
    /// Automatically prepends the correct host. Naming a request only affects collected
    /// metrics.
    ///
    /// Calls to `user.delete` return a `GooseResponse` object which contains a copy of
    /// the request you made
    /// ([`goose.request`](https://docs.rs/goose/*/goose/goose/struct.GooseRawRequest)), and the response
    /// ([`goose.response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut task = task!(delete_function);
    ///
    /// /// A very simple task that makes a DELETE request.
    /// async fn delete_function(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.delete_named("/path/to/foo/", "foo").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn delete_named(
        &self,
        path: &str,
        request_name: &str,
    ) -> Result<GooseResponse, GooseTaskError> {
        let request_builder = self.goose_delete(path).await?;

        Ok(self.goose_send(request_builder, Some(request_name)).await?)
    }

    /// Prepends the correct host on the path, then prepares a
    /// [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object for making a `GET` request.
    ///
    /// (You must then call `goose_send` on this object to actually execute the request.)
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut task = task!(get_function);
    ///
    /// /// A simple task that makes a GET request, exposing the Reqwest
    /// /// request builder.
    /// async fn get_function(user: &GooseUser) -> GooseTaskResult {
    ///     let request_builder = user.goose_get("/path/to/foo").await?;
    ///     let _goose = user.goose_send(request_builder, None).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn goose_get(&self, path: &str) -> Result<RequestBuilder, GooseTaskError> {
        let url = self.build_url(path).await?;

        Ok(self.client.lock().await.get(&url))
    }

    /// Prepends the correct host on the path, then prepares a
    /// [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object for making a `POST` request.
    ///
    /// (You must then call `goose_send` on this object to actually execute the request.)
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut task = task!(post_function);
    ///
    /// /// A simple task that makes a POST request, exposing the Reqwest
    /// /// request builder.
    /// async fn post_function(user: &GooseUser) -> GooseTaskResult {
    ///     let request_builder = user.goose_post("/path/to/foo").await?;
    ///     let _goose = user.goose_send(request_builder, None).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn goose_post(&self, path: &str) -> Result<RequestBuilder, GooseTaskError> {
        let url = self.build_url(path).await?;

        Ok(self.client.lock().await.post(&url))
    }

    /// Prepends the correct host on the path, then prepares a
    /// [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object for making a `HEAD` request.
    ///
    /// (You must then call `goose_send` on this object to actually execute the request.)
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut task = task!(head_function);
    ///
    /// /// A simple task that makes a HEAD request, exposing the Reqwest
    /// /// request builder.
    /// async fn head_function(user: &GooseUser) -> GooseTaskResult {
    ///     let request_builder = user.goose_head("/path/to/foo").await?;
    ///     let _goose = user.goose_send(request_builder, None).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn goose_head(&self, path: &str) -> Result<RequestBuilder, GooseTaskError> {
        let url = self.build_url(path).await?;

        Ok(self.client.lock().await.head(&url))
    }

    /// Prepends the correct host on the path, then prepares a
    /// [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object for making a `PUT` request.
    ///
    /// (You must then call `goose_send` on this object to actually execute the request.)
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut task = task!(put_function);
    ///
    /// /// A simple task that makes a PUT request, exposing the Reqwest
    /// /// request builder.
    /// async fn put_function(user: &GooseUser) -> GooseTaskResult {
    ///     let request_builder = user.goose_put("/path/to/foo").await?;
    ///     let _goose = user.goose_send(request_builder, None).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn goose_put(&self, path: &str) -> Result<RequestBuilder, GooseTaskError> {
        let url = self.build_url(path).await?;

        Ok(self.client.lock().await.put(&url))
    }

    /// Prepends the correct host on the path, then prepares a
    /// [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object for making a `PATCH` request.
    ///
    /// (You must then call `goose_send` on this object to actually execute the request.)
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut task = task!(patch_function);
    ///
    /// /// A simple task that makes a PUT request, exposing the Reqwest
    /// /// request builder.
    /// async fn patch_function(user: &GooseUser) -> GooseTaskResult {
    ///     let request_builder = user.goose_patch("/path/to/foo").await?;
    ///     let _goose = user.goose_send(request_builder, None).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn goose_patch(&self, path: &str) -> Result<RequestBuilder, GooseTaskError> {
        let url = self.build_url(path).await?;

        Ok(self.client.lock().await.patch(&url))
    }

    /// Prepends the correct host on the path, then prepares a
    /// [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object for making a `DELETE` request.
    ///
    /// (You must then call `goose_send` on this object to actually execute the request.)
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut task = task!(delete_function);
    ///
    /// /// A simple task that makes a DELETE request, exposing the Reqwest
    /// /// request builder.
    /// async fn delete_function(user: &GooseUser) -> GooseTaskResult {
    ///     let request_builder = user.goose_delete("/path/to/foo").await?;
    ///     let _goose = user.goose_send(request_builder, None).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn goose_delete(&self, path: &str) -> Result<RequestBuilder, GooseTaskError> {
        let url = self.build_url(path).await?;

        Ok(self.client.lock().await.delete(&url))
    }

    /// Builds the provided
    /// [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object and then executes the response. If metrics are being displayed, it
    /// also captures request metrics.
    ///
    /// It is possible to build and execute a `RequestBuilder` object directly with
    /// Reqwest without using this helper function, but then Goose is unable to capture
    /// metrics.
    ///
    /// Calls to `user.goose_send()` returns a `Result` containing a `GooseResponse` on success,
    /// and a `flume::SendError<bool>` on failure. Failure only happens when `--throttle-requests`
    /// is enabled and the load test completes. The `GooseResponse` object contains a copy of the
    /// request made
    /// ([`goose.request`](https://docs.rs/goose/*/goose/goose/struct.GooseRawRequest)), and the
    /// Reqwest response ([`goose.response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut task = task!(get_function);
    ///
    ///     /// A simple task that makes a GET request, exposing the Reqwest
    ///     /// request builder.
    ///     async fn get_function(user: &GooseUser) -> GooseTaskResult {
    ///         let request_builder = user.goose_get("/path/to/foo").await?;
    ///         let goose = user.goose_send(request_builder, None).await?;
    ///
    ///         // Do stuff with goose.request and/or goose.response here.
    ///
    ///         Ok(())
    ///     }
    /// ```
    pub async fn goose_send(
        &self,
        request_builder: RequestBuilder,
        request_name: Option<&str>,
    ) -> Result<GooseResponse, GooseTaskError> {
        // If throttle-requests is enabled...
        if self.is_throttled && self.throttle.is_some() {
            // ...wait until there's room to add a token to the throttle channel before proceeding.
            debug!("GooseUser: waiting on throttle");
            // Will result in GooseTaskError::RequestCanceled if this fails.
            self.throttle.clone().unwrap().send_async(true).await?;
        };

        let started = Instant::now();
        let request = request_builder.build()?;

        // String version of request path.
        let path = match Url::parse(&request.url().to_string()) {
            Ok(u) => u.path().to_string(),
            Err(e) => {
                error!("failed to parse url: {}", e);
                "".to_string()
            }
        };
        let method = goose_method_from_method(request.method().clone())?;
        let request_name = self.get_request_name(&path, request_name);

        // Record information about the request.
        let mut raw_request = GooseRawRequest::new(
            method,
            &request_name,
            &request.url().to_string(),
            self.started.elapsed().as_millis(),
            self.weighted_users_index,
        );

        // Make the actual request.
        let response = self.client.lock().await.execute(request).await;
        raw_request.set_response_time(started.elapsed().as_millis());

        match &response {
            Ok(r) => {
                let status_code = r.status();
                debug!("{:?}: status_code {}", &path, status_code);
                // @TODO: match/handle all is_foo() https://docs.rs/http/0.2.1/http/status/struct.StatusCode.html
                if !status_code.is_success() {
                    raw_request.success = false;
                    raw_request.error = format!("{}: {}", status_code, &path);
                }
                raw_request.set_status_code(Some(status_code));
                raw_request.set_final_url(r.url().as_str());

                // Load test user was redirected.
                if self.config.sticky_follow && raw_request.url != raw_request.final_url {
                    let base_url = self.base_url.read().await.to_string();
                    // Check if the URL redirected started with the load test base_url.
                    if !raw_request.final_url.starts_with(&base_url) {
                        let redirected_url = Url::parse(&raw_request.final_url)?;
                        let redirected_base_url =
                            redirected_url[..url::Position::BeforePath].to_string();
                        info!(
                            "base_url for user {} redirected from {} to {}",
                            self.weighted_users_index + 1,
                            &base_url,
                            &redirected_base_url
                        );
                        self.set_base_url(&redirected_base_url).await?;
                    }
                }
            }
            Err(e) => {
                // @TODO: what can we learn from a reqwest error?
                warn!("{:?}: {}", &path, e);
                raw_request.success = false;
                raw_request.set_status_code(None);
                raw_request.error = e.to_string();
            }
        };

        // Send a copy of the raw request object to the parent process if
        // we're tracking metrics.
        if !self.config.no_metrics {
            self.send_to_parent(GooseMetric::Request(raw_request.clone()))?;
        }

        Ok(GooseResponse::new(raw_request, response))
    }

    fn send_to_parent(&self, metric: GooseMetric) -> GooseTaskResult {
        // Parent is not defined when running test_start_task, test_stop_task,
        // and during testing.
        if let Some(parent) = self.channel_to_parent.clone() {
            parent.send(metric)?;
        }

        Ok(())
    }

    /// If `request_name` is set, unwrap and use this. Otherwise, if the GooseTask has a name
    /// set use it. Otherwise use the path.
    fn get_request_name(&self, path: &str, request_name: Option<&str>) -> String {
        match request_name {
            // If a request_name was passed in, unwrap and return a copy of it.
            Some(rn) => rn.to_string(),
            None => {
                // Otherwise determine if the current GooseTask is named, and if so return
                // a copy of it.
                let weighted_bucket = self.weighted_bucket.load(atomic::Ordering::SeqCst);
                let weighted_bucket_position =
                    self.weighted_bucket_position.load(atomic::Ordering::SeqCst);
                if !self.weighted_tasks.is_empty()
                    && !self.weighted_tasks[weighted_bucket][weighted_bucket_position]
                        .1
                        .is_empty()
                {
                    self.weighted_tasks[weighted_bucket][weighted_bucket_position]
                        .1
                        .clone()
                } else {
                    // Otherwise return a copy of the the path.
                    path.to_string()
                }
            }
        }
    }

    /// Manually mark a request as a success.
    ///
    /// By default, Goose will consider any response with a 2xx status code as a success.
    /// It may be valid in your test for a non-2xx HTTP status code to be returned. A copy
    /// of your original request is returned with the response, and a mutable copy must be
    /// included when setting a request as a success.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut task = task!(get_function);
    ///
    /// /// A simple task that makes a GET request.
    /// async fn get_function(user: &GooseUser) -> GooseTaskResult {
    ///     let mut goose = user.get("/404").await?;
    ///
    ///     if let Ok(response) = &goose.response {
    ///         // We expect a 404 here.
    ///         if response.status() == 404 {
    ///             return user.set_success(&mut goose.request);
    ///         }
    ///     }
    ///
    ///     Err(GooseTaskError::RequestFailed {
    ///         raw_request: goose.request.clone(),
    ///     })
    /// }
    /// ````
    pub fn set_success(&self, request: &mut GooseRawRequest) -> GooseTaskResult {
        // Only send update if this was previously not a success.
        if !request.success {
            request.success = true;
            request.update = true;
            self.send_to_parent(GooseMetric::Request(request.clone()))?;
        }

        Ok(())
    }

    /// Manually mark a request as a failure.
    ///
    /// By default, Goose will consider any response with a 2xx status code as a success.
    /// You may require more advanced logic, in which a 2xx status code is actually a
    /// failure. A copy of your original request is returned with the response, and a
    /// mutable copy must be included when setting a request as a failure.
    ///
    /// Calls to `set_failure` must include four parameters. The first, `tag`, is an
    /// arbitrary string identifying the reason for the failure, used when logging. The
    /// second, `request`, is a mutable reference to the `GooseRawRequest` object of the
    /// request being identified as a failure (the contained `success` field will be set
    /// to `false`, and the `update` field will be set to `true`). The last two
    /// parameters, `header` and `body`, are optional and used to provide more detail in
    /// logs.
    ///
    /// The value of `tag` will normally be collected into the errors summary table if
    /// metrics are being displayed. However, if `set_failure` is called multiple times,
    /// or is called on a request that was already an error, only the first error will
    /// be collected.
    ///
    /// This also calls
    /// [`log_debug`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.log_debug).
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut task = task!(loadtest_index_page);
    ///
    ///     async fn loadtest_index_page(user: &GooseUser) -> GooseTaskResult {
    ///         let mut goose = user.get_named("/", "index").await?;
    ///
    ///         if let Ok(response) = goose.response {
    ///             // We only need to check pages that returned a success status code.
    ///             if response.status().is_success() {
    ///                 match response.text().await {
    ///                     Ok(text) => {
    ///                         // If the expected string doesn't exist, this page load
    ///                         // was a failure.
    ///                         if !text.contains("this string must exist") {
    ///                             // As this is a named request, pass in the name not the URL
    ///                             return user.set_failure("string missing", &mut goose.request, None, None);
    ///                         }
    ///                     }
    ///                     // Empty page, this is a failure.
    ///                     Err(_) => {
    ///                         return user.set_failure("empty page", &mut goose.request, None, None);
    ///                     }
    ///                 }
    ///             }
    ///         };
    ///
    ///         Ok(())
    ///     }
    /// ````
    pub fn set_failure(
        &self,
        tag: &str,
        request: &mut GooseRawRequest,
        headers: Option<&header::HeaderMap>,
        body: Option<&str>,
    ) -> GooseTaskResult {
        // Only send update if this was previously a success.
        if request.success {
            request.success = false;
            request.update = true;
            request.error = tag.to_string();
            self.send_to_parent(GooseMetric::Request(request.clone()))?;
        }
        // Write failure to log, converting `&mut request` to `&request` as needed by `log_debug()`.
        self.log_debug(tag, Some(&*request), headers, body)?;

        // Print log to stdout if `-v` is enabled.
        info!("set_failure: {}", tag);

        Err(GooseTaskError::RequestFailed {
            raw_request: request.clone(),
        })
    }

    /// Write to debug_file if enabled.
    ///
    /// This function provides a mechanism for optional debug logging when a load test
    /// is running. This can be especially helpful when writing a load test. Each entry
    /// must include a tag, which is an arbitrary string identifying the debug message.
    /// It may also optionally include references to the GooseRawRequest made, the headers
    /// returned by the server, and the response body returned by the server,
    ///
    /// As the response body can be large, the `--no-debug-body` option (or
    /// `GooseDefault::NoDebugBody` default) can be set to prevent the debug log from
    /// including the response body. When this option is enabled, the body will always
    /// show up as `null` in the debug log.
    ///
    /// Calls to
    /// [`set_failure`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_failure)
    // automatically invoke `log_debug`.
    ///
    /// To enable the debug log, a load test must be run with the `--debug-log-file=foo`
    /// option set, where `foo` is either a relative or an absolute path of the log file
    /// to create. Any existing file will be overwritten.
    ///
    /// In the following example, we are logging debug messages whenever there are errors.
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut task = task!(loadtest_index_page);
    ///
    ///     async fn loadtest_index_page(user: &GooseUser) -> GooseTaskResult {
    ///         let mut goose = user.get("/").await?;
    ///
    ///         match goose.response {
    ///             Ok(response) => {
    ///                 // Grab a copy of the headers so we can include them when logging errors.
    ///                 let headers = &response.headers().clone();
    ///                 // We only need to check pages that returned a success status code.
    ///                 if !response.status().is_success() {
    ///                     match response.text().await {
    ///                         Ok(html) => {
    ///                             // Server returned an error code, log everything.
    ///                             user.log_debug(
    ///                                 "error loading /",
    ///                                 Some(&goose.request),
    ///                                 Some(headers),
    ///                                 Some(&html),
    ///                             );
    ///                         },
    ///                         Err(e) => {
    ///                             // No body was returned, log everything else.
    ///                             user.log_debug(
    ///                                 &format!("error loading /: {}", e),
    ///                                 Some(&goose.request),
    ///                                 Some(headers),
    ///                                 None,
    ///                             );
    ///                         }
    ///                     }
    ///                 }
    ///             },
    ///             // No response from server.
    ///             Err(e) => {
    ///                 user.log_debug(
    ///                     "no response from server when loading /",
    ///                     Some(&goose.request),
    ///                     None,
    ///                     None,
    ///                 );
    ///             }
    ///         }
    ///
    ///         Ok(())
    ///     }
    /// ````
    pub fn log_debug(
        &self,
        tag: &str,
        request: Option<&GooseRawRequest>,
        headers: Option<&header::HeaderMap>,
        body: Option<&str>,
    ) -> GooseTaskResult {
        if !self.config.debug_file.is_empty() {
            // Logger is not defined when running test_start_task, test_stop_task,
            // and during testing.
            if let Some(debug_logger) = self.debug_logger.clone() {
                if self.config.no_debug_body {
                    debug_logger.send(Some(GooseDebug::new(tag, request, headers, None)))?;
                } else {
                    debug_logger.send(Some(GooseDebug::new(tag, request, headers, body)))?;
                }
            }
        }

        Ok(())
    }

    /// Manually build a Reqwest client.
    ///
    /// By default, Goose configures two options when building a Reqwest client. The first
    /// configures Goose to report itself as the user agent requesting web pages (ie
    /// `goose/0.10.9`). The second option configures Reqwest to store cookies, which is
    /// generally necessary if you aim to simulate logged in users.
    ///
    /// # Default configuration:
    ///
    /// ```rust
    /// use reqwest::Client;
    ///
    /// static APP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
    ///
    /// let builder = Client::builder()
    ///   .user_agent(APP_USER_AGENT)
    ///   .cookie_store(true);
    /// ```
    ///
    /// Alternatively, you can use this function to manually build a Reqwest client with custom
    /// configuration. Available options are found in the
    /// [Reqwest `ClientBuilder`](https://docs.rs/reqwest/*/reqwest/struct.ClientBuilder.html)
    /// documentation.
    ///
    /// When manually building a Reqwest client, there are a few things to be aware of:
    ///  - Manually building a client in `test_start` will only affect requests made during
    ///    test setup;
    ///  - Manually building a client in `test_stop` will only affect requests made during
    ///    test teardown;
    ///  - A manually built client is specific to a single Goose thread -- if you are
    ///    generating a large load test with many users, each will need to manually build their
    ///    own client (typically you'd do this in a Task that is registered with `set_on_start()`
    ///    in each Task Set requiring a custom client;
    ///  - Manually building a client will completely replace the automatically built client
    ///    with a brand new one, so any configuration, cookies or headers set in the previously
    ///    built client will be gone;
    ///  - You must include all desired configuration, as you are completely replacing Goose
    ///    defaults. For example, if you want Goose clients to store cookies, you will have to
    ///    include `.cookie_store(true)`.
    ///
    /// In the following example, the Goose client is configured with a different user agent,
    /// sets a default header on every request, and stores cookies.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// task!(setup_custom_client).set_on_start();
    ///
    /// async fn setup_custom_client(user: &GooseUser) -> GooseTaskResult {
    ///     use reqwest::{Client, header};
    ///
    ///     // Build a custom HeaderMap to include with all requests made by this client.
    ///     let mut headers = header::HeaderMap::new();
    ///     headers.insert("X-Custom-Header", header::HeaderValue::from_str("custom value").unwrap());
    ///
    ///     let builder = Client::builder()
    ///         .default_headers(headers)
    ///         .user_agent("custom user agent")
    ///         .cookie_store(true);
    ///
    ///     user.set_client_builder(builder).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn set_client_builder(&self, builder: ClientBuilder) -> Result<(), GooseTaskError> {
        *self.client.lock().await = builder.build()?;

        Ok(())
    }

    /// Some websites use multiple domains to serve traffic, redirecting depending on
    /// the user's roll. For this reason, Goose needs to respect a redirect of the
    /// base_url and subsequent paths should be built from the redirect domain.
    ///
    /// For example, if the base_url (ie --host) is set to foo.example.com and the
    /// load test requests /login, thereby loading http://foo.example.com/login and
    /// this request gets redirected by the server to http://foo-secure.example.com/,
    /// subsequent requests made by this user need to be against the new
    /// foo-secure.example.com domain. (Further, if the base_url is again redirected,
    /// such as when loading http://foo-secure.example.com/logout, the user should
    /// again follow for subsequent requests, perhaps in this case back to
    /// foo.example.com.)
    ///
    /// Load tests can also request absolute URLs, and if these URLs are redirected
    /// it does not affect the base_url of the load test. For example, if
    /// foo.example.com is the base url, and the load test requests
    /// http://bar.example.com (a different domain) and this request gets redirected
    /// to http://other.example.com, subsequent relative requests would still be made
    /// against foo.example.com.
    ///
    /// This functionality is used internally by Goose to follow redirects of the
    /// base_url when `--sticky-follow` is specified at run time, or
    /// `set_default(GooseDefault::StickyFollow, true)` is enabled. It is also
    /// available to be manually invoked from a load test such as in the following
    /// example.
    ///
    /// # Example
    /// ```rust,no_run
    /// use goose::prelude::*;
    ///
    /// fn main() -> Result<(), GooseError> {
    ///     let _goose_metrics = GooseAttack::initialize()?
    ///         .register_taskset(taskset!("LoadtestTasks").set_host("http//foo.example.com/")
    ///             .set_wait_time(0, 3)?
    ///             .register_task(task!(task_foo).set_weight(10)?)
    ///             .register_task(task!(task_bar))
    ///         )
    ///         .execute()?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn task_foo(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.get("/").await?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn task_bar(user: &GooseUser) -> GooseTaskResult {
    ///     // Before this task runs, all requests are being made against
    ///     // http://foo.example.com, after this task runs all subsequent
    ///     // requests are made against http://bar.example.com/.
    ///     user.set_base_url("http://bar.example.com/");
    ///     let _goose = user.get("/").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn set_base_url(&self, host: &str) -> Result<(), GooseTaskError> {
        let url = Url::parse(host)?;
        *self.base_url.write().await = url;

        Ok(())
    }
}

/// A helper to determine which host should be prepended to relative load test
/// paths in this TaskSet.
///
/// The first of these defined will be returned as the prepended host:
///  1. `--host` (host specified on the command line when running load test)
///  2. `GooseTaskSet.host` (default host defined for the current task set)
///  3. `GooseAttack.host` (default host defined for the current load test)
pub fn get_base_url(
    config_host: Option<String>,
    task_set_host: Option<String>,
    default_host: Option<String>,
) -> Result<Url, GooseError> {
    // If the `--host` CLI option is set, build the URL with it.
    match config_host {
        Some(host) => Ok(
            Url::parse(&host).map_err(|parse_error| GooseError::InvalidHost {
                host,
                detail: "There was a failure parsing the host specified with --host.".to_string(),
                parse_error,
            })?,
        ),
        None => {
            match task_set_host {
                // Otherwise, if `GooseTaskSet.host` is defined, usee this
                Some(host) => {
                    Ok(
                        Url::parse(&host).map_err(|parse_error| GooseError::InvalidHost {
                            host,
                            detail: "There was a failure parsing the host specified with the GooseTaskSet.set_host() function.".to_string(),
                            parse_error,
                        })?,
                    )
                }
                // Otherwise, use global `GooseAttack.host`. `unwrap` okay as host validation was done at startup.
                None => {
                    // Host is required, if we get here it's safe to unwrap this variable.
                    let default_host = default_host.unwrap();
                    Ok(
                        Url::parse(&default_host).map_err(|parse_error| GooseError::InvalidHost {
                            host: default_host.to_string(),
                            detail: "There was a failure parsing the host specified globally with the GooseAttack.set_default() function.".to_string(),
                            parse_error,
                        })?,
                    )
                }
            }
        }
    }
}

/// The function type of a goose task function.
pub type GooseTaskFunction = Arc<
    dyn for<'r> Fn(&'r GooseUser) -> Pin<Box<dyn Future<Output = GooseTaskResult> + Send + 'r>>
        + Send
        + Sync,
>;

/// An individual task within a `GooseTaskSet`.
#[derive(Clone)]
pub struct GooseTask {
    /// An index into GooseTaskSet.task, indicating which task this is.
    pub tasks_index: usize,
    /// An optional name for the task, used when displaying metrics about requests made.
    pub name: String,
    /// An integer value that controls the frequency that this task will be run.
    pub weight: usize,
    /// An integer value that controls when this task runs compared to other tasks in the same GooseTaskSet.
    pub sequence: usize,
    /// A flag indicating that this task runs when the user starts.
    pub on_start: bool,
    /// A flag indicating that this task runs when the user stops.
    pub on_stop: bool,
    /// A required function that is executed each time this task runs.
    pub function: GooseTaskFunction,
}
impl GooseTask {
    pub fn new(function: GooseTaskFunction) -> Self {
        trace!("new task");
        GooseTask {
            tasks_index: usize::max_value(),
            name: "".to_string(),
            weight: 1,
            sequence: 0,
            on_start: false,
            on_stop: false,
            function,
        }
    }

    /// Set an optional name for the task, used when displaying metrics about
    /// requests made by the task.
    ///
    /// Individual requests can also be named withing your load test. See the
    /// documentation for `GooseUser`.
    /// [`set_request_name()`](./struct.GooseUser.html#method.set_request_name)
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     task!(my_task_function).set_name("foo");
    ///
    ///     async fn my_task_function(user: &GooseUser) -> GooseTaskResult {
    ///       let _goose = user.get("/").await?;
    ///
    ///       Ok(())
    ///     }
    /// ```
    pub fn set_name(mut self, name: &str) -> Self {
        trace!("[{}] set_name: {}", self.tasks_index, self.name);
        self.name = name.to_string();
        self
    }

    /// Set an optional flag indicating that this task should be run when
    /// a user first starts. This could be used to log the user in, and
    /// so all subsequent tasks are done as a logged in user. A task with
    /// this flag set will only run at start time (and optionally at stop
    /// time as well, if that flag is also set).
    ///
    /// On-start tasks can be sequenced and weighted. Sequences allow
    /// multiple on-start tasks to run in a controlled order. Weights allow
    /// on-start tasks to run multiple times when a user starts.
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     task!(my_on_start_function).set_on_start();
    ///
    ///     async fn my_on_start_function(user: &GooseUser) -> GooseTaskResult {
    ///       let _goose = user.get("/").await?;
    ///
    ///       Ok(())
    ///     }
    /// ```
    pub fn set_on_start(mut self) -> Self {
        trace!("{} [{}] set_on_start task", self.name, self.tasks_index);
        self.on_start = true;
        self
    }

    /// Set an optional flag indicating that this task should be run when
    /// a user stops. This could be used to log a user out when the user
    /// finishes its load test. A task with this flag set will only run at
    /// stop time (and optionally at start time as well, if that flag is
    /// also set).
    ///
    /// On-stop tasks can be sequenced and weighted. Sequences allow
    /// multiple on-stop tasks to run in a controlled order. Weights allow
    /// on-stop tasks to run multiple times when a user stops.
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     task!(my_on_stop_function).set_on_stop();
    ///
    ///     async fn my_on_stop_function(user: &GooseUser) -> GooseTaskResult {
    ///       let _goose = user.get("/").await?;
    ///
    ///       Ok(())
    ///     }
    /// ```
    pub fn set_on_stop(mut self) -> Self {
        trace!("{} [{}] set_on_stop task", self.name, self.tasks_index);
        self.on_stop = true;
        self
    }

    /// Sets a weight on an individual task. The larger the value of weight, the more often it will be run
    /// in the TaskSet. For example, if one task has a weight of 3 and another task has a weight of 1, the
    /// first task will run 3 times as often.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// fn main() -> Result<(), GooseError> {
    ///     task!(task_function).set_weight(3)?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn task_function(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.get("/").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_weight(mut self, weight: usize) -> Result<Self, GooseError> {
        trace!(
            "{} [{}] set_weight: {}",
            self.name,
            self.tasks_index,
            weight
        );
        if weight == 0 {
            return Err(GooseError::InvalidWeight {
                weight,
                detail: "Weight must be set to at least 1.".to_string(),
            });
        }
        self.weight = weight;

        Ok(self)
    }

    /// Defines the sequence value of an individual tasks. Tasks are run in order of their sequence value,
    /// so a task with a sequence value of 1 will run before a task with a sequence value of 2. Tasks with
    /// no sequence value (or a sequence value of 0) will run last, after all tasks with positive sequence
    /// values.
    ///
    /// All tasks with the same sequence value will run in a random order. Tasks can be assigned both
    /// squence values and weights.
    ///
    /// # Examples
    /// In this first example, the variable names indicate the order the tasks will be run in:
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let runs_first = task!(first_task_function).set_sequence(3);
    ///     let runs_second = task!(second_task_function).set_sequence(5835);
    ///     let runs_last = task!(third_task_function);
    ///
    ///     async fn first_task_function(user: &GooseUser) -> GooseTaskResult {
    ///       let _goose = user.get("/1").await?;
    ///
    ///       Ok(())
    ///     }
    ///
    ///     async fn second_task_function(user: &GooseUser) -> GooseTaskResult {
    ///       let _goose = user.get("/2").await?;
    ///
    ///       Ok(())
    ///     }
    ///
    ///     async fn third_task_function(user: &GooseUser) -> GooseTaskResult {
    ///       let _goose = user.get("/3").await?;
    ///
    ///       Ok(())
    ///     }
    /// ```
    ///
    /// In the following example, the `runs_first` task runs two times, then one instance of `runs_second`
    /// and two instances of `also_runs_second` are all three run. The user will do this over and over
    /// the entire time it runs, with `runs_first` always running first, then the other tasks being
    /// run in a random and weighted order:
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// fn main() -> Result<(), GooseError> {
    ///     let runs_first = task!(first_task_function).set_sequence(1).set_weight(2)?;
    ///     let runs_second = task!(second_task_function_a).set_sequence(2);
    ///     let also_runs_second = task!(second_task_function_b).set_sequence(2).set_weight(2)?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn first_task_function(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.get("/1").await?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn second_task_function_a(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.get("/2a").await?;
    ///
    ///     Ok(())
    /// }
    ///
    ///     async fn second_task_function_b(user: &GooseUser) -> GooseTaskResult {
    ///       let _goose = user.get("/2b").await?;
    ///
    ///       Ok(())
    ///     }
    /// ```
    pub fn set_sequence(mut self, sequence: usize) -> Self {
        trace!(
            "{} [{}] set_sequence: {}",
            self.name,
            self.tasks_index,
            sequence
        );
        if sequence < 1 {
            info!(
                "setting sequence to 0 for task {} is unnecessary, sequence disabled",
                self.name
            );
        }
        self.sequence = sequence;
        self
    }
}
impl Hash for GooseTask {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.tasks_index.hash(state);
        self.name.hash(state);
        self.weight.hash(state);
        self.sequence.hash(state);
        self.on_start.hash(state);
        self.on_stop.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use gumdrop::Options;
    use httpmock::{
        Method::{GET, POST},
        MockServer,
    };

    const EMPTY_ARGS: Vec<&str> = vec![];

    async fn setup_user(server: &MockServer) -> Result<GooseUser, GooseError> {
        let configuration = GooseConfiguration::parse_args_default(&EMPTY_ARGS).unwrap();
        let base_url = get_base_url(Some(server.url("/")), None, None).unwrap();
        GooseUser::single(base_url, &configuration)
    }

    #[test]
    fn goose_task_set() {
        // Simplistic test task functions.
        async fn test_function_a(user: &GooseUser) -> GooseTaskResult {
            let _goose = user.get("/a/").await?;

            Ok(())
        }

        async fn test_function_b(user: &GooseUser) -> GooseTaskResult {
            let _goose = user.get("/b/").await?;

            Ok(())
        }

        let mut task_set = taskset!("foo");
        assert_eq!(task_set.name, "foo");
        assert_eq!(task_set.task_sets_index, usize::max_value());
        assert_eq!(task_set.weight, 1);
        assert_eq!(task_set.min_wait, 0);
        assert_eq!(task_set.max_wait, 0);
        assert_eq!(task_set.host, None);
        assert_eq!(task_set.tasks.len(), 0);
        assert_eq!(task_set.weighted_tasks.len(), 0);
        assert_eq!(task_set.weighted_on_start_tasks.len(), 0);
        assert_eq!(task_set.weighted_on_stop_tasks.len(), 0);

        // Registering a task adds it to tasks, but doesn't update weighted_tasks.
        task_set = task_set.register_task(task!(test_function_a));
        assert_eq!(task_set.tasks.len(), 1);
        assert_eq!(task_set.weighted_tasks.len(), 0);
        assert_eq!(task_set.task_sets_index, usize::max_value());
        assert_eq!(task_set.weight, 1);
        assert_eq!(task_set.min_wait, 0);
        assert_eq!(task_set.max_wait, 0);
        assert_eq!(task_set.host, None);

        // Different task can be registered.
        task_set = task_set.register_task(task!(test_function_b));
        assert_eq!(task_set.tasks.len(), 2);
        assert_eq!(task_set.weighted_tasks.len(), 0);
        assert_eq!(task_set.task_sets_index, usize::max_value());
        assert_eq!(task_set.weight, 1);
        assert_eq!(task_set.min_wait, 0);
        assert_eq!(task_set.max_wait, 0);
        assert_eq!(task_set.host, None);

        // Same task can be registered again.
        task_set = task_set.register_task(task!(test_function_a));
        assert_eq!(task_set.tasks.len(), 3);
        assert_eq!(task_set.weighted_tasks.len(), 0);
        assert_eq!(task_set.task_sets_index, usize::max_value());
        assert_eq!(task_set.weight, 1);
        assert_eq!(task_set.min_wait, 0);
        assert_eq!(task_set.max_wait, 0);
        assert_eq!(task_set.host, None);

        // Setting weight only affects weight field.
        task_set = task_set.set_weight(50).unwrap();
        assert_eq!(task_set.weight, 50);
        assert_eq!(task_set.tasks.len(), 3);
        assert_eq!(task_set.weighted_tasks.len(), 0);
        assert_eq!(task_set.task_sets_index, usize::max_value());
        assert_eq!(task_set.min_wait, 0);
        assert_eq!(task_set.max_wait, 0);
        assert_eq!(task_set.host, None);

        // Weight can be changed.
        task_set = task_set.set_weight(5).unwrap();
        assert_eq!(task_set.weight, 5);

        // Setting host only affects host field.
        task_set = task_set.set_host("http://foo.example.com/");
        assert_eq!(task_set.host, Some("http://foo.example.com/".to_string()));
        assert_eq!(task_set.weight, 5);
        assert_eq!(task_set.tasks.len(), 3);
        assert_eq!(task_set.weighted_tasks.len(), 0);
        assert_eq!(task_set.task_sets_index, usize::max_value());
        assert_eq!(task_set.min_wait, 0);
        assert_eq!(task_set.max_wait, 0);

        // Host field can be changed.
        task_set = task_set.set_host("https://bar.example.com/");
        assert_eq!(task_set.host, Some("https://bar.example.com/".to_string()));

        // Wait time only affects wait time fields.
        task_set = task_set.set_wait_time(1, 10).unwrap();
        assert_eq!(task_set.min_wait, 1);
        assert_eq!(task_set.max_wait, 10);
        assert_eq!(task_set.host, Some("https://bar.example.com/".to_string()));
        assert_eq!(task_set.weight, 5);
        assert_eq!(task_set.tasks.len(), 3);
        assert_eq!(task_set.weighted_tasks.len(), 0);
        assert_eq!(task_set.task_sets_index, usize::max_value());

        // Wait time can be changed.
        task_set = task_set.set_wait_time(3, 9).unwrap();
        assert_eq!(task_set.min_wait, 3);
        assert_eq!(task_set.max_wait, 9);
    }

    #[test]
    fn goose_task() {
        // Simplistic test task functions.
        async fn test_function_a(user: &GooseUser) -> GooseTaskResult {
            let _goose = user.get("/a/").await?;

            Ok(())
        }

        // Initialize task set.
        let mut task = task!(test_function_a);
        assert_eq!(task.tasks_index, usize::max_value());
        assert_eq!(task.name, "".to_string());
        assert_eq!(task.weight, 1);
        assert_eq!(task.sequence, 0);
        assert_eq!(task.on_start, false);
        assert_eq!(task.on_stop, false);

        // Name can be set, without affecting other fields.
        task = task.set_name("foo");
        assert_eq!(task.name, "foo".to_string());
        assert_eq!(task.weight, 1);
        assert_eq!(task.sequence, 0);
        assert_eq!(task.on_start, false);
        assert_eq!(task.on_stop, false);

        // Name can be set multiple times.
        task = task.set_name("bar");
        assert_eq!(task.name, "bar".to_string());

        // On start flag can be set, without affecting other fields.
        task = task.set_on_start();
        assert_eq!(task.on_start, true);
        assert_eq!(task.name, "bar".to_string());
        assert_eq!(task.weight, 1);
        assert_eq!(task.sequence, 0);
        assert_eq!(task.on_stop, false);

        // Setting on start flag twice doesn't change anything.
        task = task.set_on_start();
        assert_eq!(task.on_start, true);

        // On stop flag can be set, without affecting other fields.
        // It's possible to set both on_start and on_stop for same task.
        task = task.set_on_stop();
        assert_eq!(task.on_stop, true);
        assert_eq!(task.on_start, true);
        assert_eq!(task.name, "bar".to_string());
        assert_eq!(task.weight, 1);
        assert_eq!(task.sequence, 0);

        // Setting on stop flag twice doesn't change anything.
        task = task.set_on_stop();
        assert_eq!(task.on_stop, true);

        // Setting weight doesn't change anything else.
        task = task.set_weight(2).unwrap();
        assert_eq!(task.weight, 2);
        assert_eq!(task.on_stop, true);
        assert_eq!(task.on_start, true);
        assert_eq!(task.name, "bar".to_string());
        assert_eq!(task.sequence, 0);

        // Weight field can be changed multiple times.
        task = task.set_weight(3).unwrap();
        assert_eq!(task.weight, 3);

        // Setting sequence doesn't change anything else.
        task = task.set_sequence(4);
        assert_eq!(task.sequence, 4);
        assert_eq!(task.weight, 3);
        assert_eq!(task.on_stop, true);
        assert_eq!(task.on_start, true);
        assert_eq!(task.name, "bar".to_string());

        // Sequence field can be changed multiple times.
        task = task.set_sequence(8);
        assert_eq!(task.sequence, 8);
    }

    #[test]
    fn goose_raw_request() {
        const PATH: &str = "http://127.0.0.1/";
        let mut raw_request = GooseRawRequest::new(GooseMethod::GET, "/", PATH, 0, 0);
        assert_eq!(raw_request.method, GooseMethod::GET);
        assert_eq!(raw_request.name, "/".to_string());
        assert_eq!(raw_request.url, PATH.to_string());
        assert_eq!(raw_request.response_time, 0);
        assert_eq!(raw_request.status_code, 0);
        assert_eq!(raw_request.success, true);
        assert_eq!(raw_request.update, false);

        let response_time = 123;
        raw_request.set_response_time(response_time);
        assert_eq!(raw_request.method, GooseMethod::GET);
        assert_eq!(raw_request.name, "/".to_string());
        assert_eq!(raw_request.url, PATH.to_string());
        assert_eq!(raw_request.response_time, response_time as u64);
        assert_eq!(raw_request.status_code, 0);
        assert_eq!(raw_request.success, true);
        assert_eq!(raw_request.update, false);

        let status_code = http::StatusCode::OK;
        raw_request.set_status_code(Some(status_code));
        assert_eq!(raw_request.method, GooseMethod::GET);
        assert_eq!(raw_request.name, "/".to_string());
        assert_eq!(raw_request.url, PATH.to_string());
        assert_eq!(raw_request.response_time, response_time as u64);
        assert_eq!(raw_request.status_code, 200);
        assert_eq!(raw_request.success, true);
        assert_eq!(raw_request.update, false);
    }

    #[test]
    fn goose_request() {
        let mut request = GooseRequest::new("/", GooseMethod::GET, 0);
        assert_eq!(request.path, "/".to_string());
        assert_eq!(request.method, GooseMethod::GET);
        assert_eq!(request.response_times.len(), 0);
        assert_eq!(request.min_response_time, 0);
        assert_eq!(request.max_response_time, 0);
        assert_eq!(request.total_response_time, 0);
        assert_eq!(request.response_time_counter, 0);
        assert_eq!(request.status_code_counts.len(), 0);
        assert_eq!(request.success_count, 0);
        assert_eq!(request.fail_count, 0);

        // Tracking a response time updates several fields.
        request.set_response_time(1);
        // We've seen only one response time so far.
        assert_eq!(request.response_times.len(), 1);
        // We've seen one response time of length 1.
        assert_eq!(request.response_times[&1], 1);
        // The minimum response time seen so far is 1.
        assert_eq!(request.min_response_time, 1);
        // The maximum response time seen so far is 1.
        assert_eq!(request.max_response_time, 1);
        // We've seen a total of 1 ms of response time so far.
        assert_eq!(request.total_response_time, 1);
        // We've seen a total of 2 response times so far.
        assert_eq!(request.response_time_counter, 1);
        // Nothing else changes.
        assert_eq!(request.path, "/".to_string());
        assert_eq!(request.method, GooseMethod::GET);
        assert_eq!(request.status_code_counts.len(), 0);
        assert_eq!(request.success_count, 0);
        assert_eq!(request.fail_count, 0);

        // Tracking another response time updates all related fields.
        request.set_response_time(10);
        // We've added a new unique response time.
        assert_eq!(request.response_times.len(), 2);
        // We've seen the 10 ms response time 1 time.
        assert_eq!(request.response_times[&10], 1);
        // Minimum doesn't change.
        assert_eq!(request.min_response_time, 1);
        // Maximum is new response time.
        assert_eq!(request.max_response_time, 10);
        // Total combined response times is now 11 ms.
        assert_eq!(request.total_response_time, 11);
        // We've seen two response times so far.
        assert_eq!(request.response_time_counter, 2);
        // Nothing else changes.
        assert_eq!(request.path, "/".to_string());
        assert_eq!(request.method, GooseMethod::GET);
        assert_eq!(request.status_code_counts.len(), 0);
        assert_eq!(request.success_count, 0);
        assert_eq!(request.fail_count, 0);

        // Tracking another response time updates all related fields.
        request.set_response_time(10);
        // We've incremented the counter of an existing response time.
        assert_eq!(request.response_times.len(), 2);
        // We've seen the 10 ms response time 2 times.
        assert_eq!(request.response_times[&10], 2);
        // Minimum doesn't change.
        assert_eq!(request.min_response_time, 1);
        // Maximum doesn't change.
        assert_eq!(request.max_response_time, 10);
        // Total combined response times is now 21 ms.
        assert_eq!(request.total_response_time, 21);
        // We've seen three response times so far.
        assert_eq!(request.response_time_counter, 3);

        // Tracking another response time updates all related fields.
        request.set_response_time(101);
        // We've added a new response time for the first time.
        assert_eq!(request.response_times.len(), 3);
        // The response time was internally rounded to 100, which we've seen once.
        assert_eq!(request.response_times[&100], 1);
        // Minimum doesn't change.
        assert_eq!(request.min_response_time, 1);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.max_response_time, 101);
        // Total combined response times is now 122 ms.
        assert_eq!(request.total_response_time, 122);
        // We've seen four response times so far.
        assert_eq!(request.response_time_counter, 4);

        // Tracking another response time updates all related fields.
        request.set_response_time(102);
        // Due to rounding, this increments the existing 100 ms response time.
        assert_eq!(request.response_times.len(), 3);
        // The response time was internally rounded to 100, which we've now seen twice.
        assert_eq!(request.response_times[&100], 2);
        // Minimum doesn't change.
        assert_eq!(request.min_response_time, 1);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.max_response_time, 102);
        // Add 102 to the total response time so far.
        assert_eq!(request.total_response_time, 224);
        // We've seen five response times so far.
        assert_eq!(request.response_time_counter, 5);

        // Tracking another response time updates all related fields.
        request.set_response_time(155);
        // Adds a new response time.
        assert_eq!(request.response_times.len(), 4);
        // The response time was internally rounded to 160, seen for the first time.
        assert_eq!(request.response_times[&160], 1);
        // Minimum doesn't change.
        assert_eq!(request.min_response_time, 1);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.max_response_time, 155);
        // Add 155 to the total response time so far.
        assert_eq!(request.total_response_time, 379);
        // We've seen six response times so far.
        assert_eq!(request.response_time_counter, 6);

        // Tracking another response time updates all related fields.
        request.set_response_time(2345);
        // Adds a new response time.
        assert_eq!(request.response_times.len(), 5);
        // The response time was internally rounded to 2000, seen for the first time.
        assert_eq!(request.response_times[&2000], 1);
        // Minimum doesn't change.
        assert_eq!(request.min_response_time, 1);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.max_response_time, 2345);
        // Add 2345 to the total response time so far.
        assert_eq!(request.total_response_time, 2724);
        // We've seen seven response times so far.
        assert_eq!(request.response_time_counter, 7);

        // Tracking another response time updates all related fields.
        request.set_response_time(987654321);
        // Adds a new response time.
        assert_eq!(request.response_times.len(), 6);
        // The response time was internally rounded to 987654000, seen for the first time.
        assert_eq!(request.response_times[&987654000], 1);
        // Minimum doesn't change.
        assert_eq!(request.min_response_time, 1);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.max_response_time, 987654321);
        // Add 987654321 to the total response time so far.
        assert_eq!(request.total_response_time, 987657045);
        // We've seen eight response times so far.
        assert_eq!(request.response_time_counter, 8);

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
        assert_eq!(request.response_times.len(), 6);
        assert_eq!(request.min_response_time, 1);
        assert_eq!(request.max_response_time, 987654321);
        assert_eq!(request.total_response_time, 987657045);
        assert_eq!(request.response_time_counter, 8);

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
        assert_eq!(request.response_times.len(), 6);
        assert_eq!(request.min_response_time, 1);
        assert_eq!(request.max_response_time, 987654321);
        assert_eq!(request.total_response_time, 987657045);
        assert_eq!(request.response_time_counter, 8);
    }

    #[tokio::test]
    async fn goose_user() {
        const HOST: &str = "http://example.com/";
        let configuration = GooseConfiguration::parse_args_default(&EMPTY_ARGS).unwrap();
        let base_url = get_base_url(Some(HOST.to_string()), None, None).unwrap();
        let user = GooseUser::new(0, base_url, 0, 0, &configuration, 0).unwrap();
        assert_eq!(user.task_sets_index, 0);
        assert_eq!(user.min_wait, 0);
        assert_eq!(user.max_wait, 0);
        assert_eq!(user.weighted_users_index, usize::max_value());
        assert_eq!(user.weighted_on_start_tasks.len(), 0);
        assert_eq!(user.weighted_tasks.len(), 0);
        assert_eq!(user.weighted_on_stop_tasks.len(), 0);

        // Confirm the URLs are correctly built using the default_host.
        let url = user.build_url("/foo").await.unwrap();
        assert_eq!(&url, &[HOST, "foo"].concat());
        let url = user.build_url("bar/").await.unwrap();
        assert_eq!(&url, &[HOST, "bar/"].concat());
        let url = user.build_url("/foo/bar").await.unwrap();
        assert_eq!(&url, &[HOST, "foo/bar"].concat());

        // Confirm the URLs are built with their own specified host.
        let url = user.build_url("https://example.com/foo").await.unwrap();
        assert_eq!(url, "https://example.com/foo");
        let url = user
            .build_url("https://www.example.com/path/to/resource")
            .await
            .unwrap();
        assert_eq!(url, "https://www.example.com/path/to/resource");

        // Create a second user, this time setting a task_set_host.
        let base_url = get_base_url(
            None,
            Some("http://www2.example.com/".to_string()),
            Some("http://www.example.com/".to_string()),
        )
        .unwrap();
        let user2 = GooseUser::new(0, base_url, 1, 3, &configuration, 0).unwrap();
        assert_eq!(user2.min_wait, 1);
        assert_eq!(user2.max_wait, 3);

        // Confirm the URLs are correctly built using the task_set_host.
        let url = user2.build_url("/foo").await.unwrap();
        assert_eq!(url, "http://www2.example.com/foo");

        // Confirm URLs are still built with their own specified host.
        let url = user2.build_url("https://example.com/foo").await.unwrap();
        assert_eq!(url, "https://example.com/foo");

        // Recreate user2.
        let server = MockServer::start();
        let user2 = setup_user(&server).await.unwrap();

        // Create a GET request.
        let mut goose_request = user2.goose_get("/foo").await.unwrap();
        let mut built_request = goose_request.build().unwrap();
        assert_eq!(built_request.method(), &Method::GET);
        assert_eq!(built_request.url().as_str(), server.url("/foo"));
        assert_eq!(built_request.timeout(), None);

        // Create a POST request.
        goose_request = user2.goose_post("/path/to/post").await.unwrap();
        built_request = goose_request.build().unwrap();
        assert_eq!(built_request.method(), &Method::POST);
        assert_eq!(built_request.url().as_str(), server.url("/path/to/post"));
        assert_eq!(built_request.timeout(), None);

        // Create a PUT request.
        goose_request = user2.goose_put("/path/to/put").await.unwrap();
        built_request = goose_request.build().unwrap();
        assert_eq!(built_request.method(), &Method::PUT);
        assert_eq!(built_request.url().as_str(), server.url("/path/to/put"));
        assert_eq!(built_request.timeout(), None);

        // Create a PATCH request.
        goose_request = user2.goose_patch("/path/to/patch").await.unwrap();
        built_request = goose_request.build().unwrap();
        assert_eq!(built_request.method(), &Method::PATCH);
        assert_eq!(built_request.url().as_str(), server.url("/path/to/patch"));
        assert_eq!(built_request.timeout(), None);

        // Create a DELETE request.
        goose_request = user2.goose_delete("/path/to/delete").await.unwrap();
        built_request = goose_request.build().unwrap();
        assert_eq!(built_request.method(), &Method::DELETE);
        assert_eq!(built_request.url().as_str(), server.url("/path/to/delete"));
        assert_eq!(built_request.timeout(), None);

        // Create a HEAD request.
        goose_request = user2.goose_head("/path/to/head").await.unwrap();
        built_request = goose_request.build().unwrap();
        assert_eq!(built_request.method(), &Method::HEAD);
        assert_eq!(built_request.url().as_str(), server.url("/path/to/head"));
        assert_eq!(built_request.timeout(), None);
    }

    #[tokio::test]
    async fn manual_requests() {
        let server = MockServer::start();

        let user = setup_user(&server).await.unwrap();

        // Set up a mock http server endpoint.
        const INDEX_PATH: &str = "/";
        let index = server.mock(|when, then| {
            when.method(GET).path(INDEX_PATH);
            then.status(200);
        });

        // Make a GET request to the mock http server and confirm we get a 200 response.
        assert_eq!(index.hits(), 0);
        let goose = user
            .get(INDEX_PATH)
            .await
            .expect("get returned unexpected error");
        let status = goose.response.unwrap().status();
        assert_eq!(status, 200);
        assert_eq!(goose.request.method, GooseMethod::GET);
        assert_eq!(goose.request.name, INDEX_PATH);
        assert_eq!(goose.request.success, true);
        assert_eq!(goose.request.update, false);
        assert_eq!(goose.request.status_code, 200);
        assert_eq!(index.hits(), 1);

        const NO_SUCH_PATH: &str = "/no/such/path";
        // Set up a mock http server endpoint.
        let not_found = server.mock(|when, then| {
            when.method(GET).path(NO_SUCH_PATH);
            then.status(404);
        });

        // Make an invalid GET request to the mock http server and confirm we get a 404 response.
        assert_eq!(not_found.hits(), 0);
        let goose = user
            .get(NO_SUCH_PATH)
            .await
            .expect("get returned unexpected error");
        let status = goose.response.unwrap().status();
        assert_eq!(status, 404);
        assert_eq!(goose.request.method, GooseMethod::GET);
        assert_eq!(goose.request.name, NO_SUCH_PATH);
        assert_eq!(goose.request.success, false);
        assert_eq!(goose.request.update, false);
        assert_eq!(goose.request.status_code, 404,);
        not_found.assert_hits(1);

        // Set up a mock http server endpoint.
        const COMMENT_PATH: &str = "/comment";
        let comment = server.mock(|when, then| {
            when.method(POST).path(COMMENT_PATH).body("foo");
            then.status(200).body("foo");
        });

        // Make a POST request to the mock http server and confirm we get a 200 OK response.
        assert_eq!(comment.hits(), 0);
        let goose = user
            .post(COMMENT_PATH, "foo")
            .await
            .expect("post returned unexpected error");
        let unwrapped_response = goose.response.unwrap();
        let status = unwrapped_response.status();
        assert_eq!(status, 200);
        let body = unwrapped_response.text().await.unwrap();
        assert_eq!(body, "foo");
        assert_eq!(goose.request.method, GooseMethod::POST);
        assert_eq!(goose.request.name, COMMENT_PATH);
        assert_eq!(goose.request.success, true);
        assert_eq!(goose.request.update, false);
        assert_eq!(goose.request.status_code, 200);
        comment.assert_hits(1);
    }
}
