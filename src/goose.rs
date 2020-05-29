//! Helpers and objects for building Goose load tests.
//!
//! Goose manages load tests with a series of objects:
//!
//! - [`GooseTaskSet`](./struct.GooseTaskSet.html) each client is assigned a task set, which is a collection of tasks.
//! - [`GooseTask`](./struct.GooseTask.html) tasks define one or more web requests and are assigned to task sets.
//! - [`GooseClient`](./struct.GooseClient.html) a client state responsible for repeatedly running all tasks in the assigned task set.
//! - [`GooseRequest`](./struct.GooseRequest.html) optional statistics collected for each URL/method pair.
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
//! A weight can be assigned to a task set, controlling how often it is assigned to client
//! threads. The larger the value of weight, the more it will be assigned to clients. In the
//! following example, `FooTasks` will be assigned to clients twice as often as `Bar` tasks.
//! We could have just added a weight of `2` to `FooTasks` and left the default weight of `1`
//! assigned to `BarTasks` for the same weighting:
//!
//! ```rust
//!     use goose::prelude::*;
//!
//!     let mut foo_tasks = taskset!("FooTasks").set_weight(10);
//!     let mut bar_tasks = taskset!("BarTasks").set_weight(5);
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
//! the task set, the client will pause for a random number of seconds inclusively between
//! the low and high wait times. In the following example, Clients loading `foo` tasks will
//! sleep 0 to 3 seconds after each task completes, and Clients loading `bar` tasks will
//! sleep 5 to 10 seconds after each task completes.
//!
//! ```rust
//!     use goose::prelude::*;
//!
//!     let mut foo_tasks = taskset!("FooTasks").set_wait_time(0, 3);
//!     let mut bar_tasks = taskset!("BarTasks").set_wait_time(5, 10);
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
//!     /// A very simple task that simply loads the front page.
//!     async fn task_function(client: &mut GooseClient) {
//!       let _response = client.get("/");
//!     }
//! ```
//!
//! ### Task Name
//!
//! A name can be assigned to a task, and will be displayed in statistics about all requests
//! made by the task.
//!
//! ```rust
//!     use goose::prelude::*;
//!
//!     let mut a_task = task!(task_function).set_name("a");
//!
//!     /// A very simple task that simply loads the front page.
//!     async fn task_function(client: &mut GooseClient) {
//!       let _response = client.get("/");
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
//!     use goose::prelude::*;
//!
//!     let mut a_task = task!(a_task_function).set_weight(9);
//!     let mut b_task = task!(b_task_function).set_weight(3);
//!
//!     /// A very simple task that simply loads the "a" page.
//!     async fn a_task_function(client: &mut GooseClient) {
//!       let _response = client.get("/a/");
//!     }
//!
//!     /// Another very simple task that simply loads the "b" page.
//!     async fn b_task_function(client: &mut GooseClient) {
//!       let _response = client.get("/b/");
//!     }
//! ```
//!
//! ### Task Sequence
//!
//! Tasks can also be configured to run in a sequence. For example, a task with a sequence
//! value of `1` will always run before a task with a sequence value of `2`. Weight can
//! be applied to sequenced tasks, so for example a task with a weight of `2` and a sequence
//! of `1` will run two times before a task with a sequence of `2`. Task sets can contain
//! tasks with sequence values and without sequence values, and in this case all tasks with
//! a sequence value will run before tasks without a sequence value. In the folllowing example,
//! `a_task` runs before `b_task`, which runs before `c_task`:
//!
//! ```rust
//!     use goose::prelude::*;
//!
//!     let mut a_task = task!(a_task_function).set_sequence(1);
//!     let mut b_task = task!(b_task_function).set_sequence(2);
//!     let mut c_task = task!(c_task_function);
//!
//!     /// A very simple task that simply loads the "a" page.
//!     async fn a_task_function(client: &mut GooseClient) {
//!       let _response = client.get("/a/");
//!     }
//!
//!     /// Another very simple task that simply loads the "b" page.
//!     async fn b_task_function(client: &mut GooseClient) {
//!       let _response = client.get("/b/");
//!     }
//!
//!     /// Another very simple task that simply loads the "c" page.
//!     async fn c_task_function(client: &mut GooseClient) {
//!       let _response = client.get("/c/");
//!     }
//! ```
//!
//! ### Task On Start
//!
//! Tasks can be flagged to only run when a client first starts. This can be useful if you'd
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
//!     /// A very simple task that simply loads the "a" page.
//!     async fn a_task_function(client: &mut GooseClient) {
//!       let _response = client.get("/a/");
//!     }
//! ```
//!
//! ### Task On Stop
//!
//! Tasks can be flagged to only run when a client stops. This can be useful if you'd like your
//! load test to simluate a user logging out when it finishes. It is possible to assign sequences
//! and weights to `on_stop` functions if you want to have multiple tasks run in a specific order
//! at stop time, and/or the tasks to run multiple times. A task can be flagged to run both on
//! start and on stop.
//!
//! ```rust
//!     use goose::prelude::*;
//!
//!     let mut b_task = task!(b_task_function).set_sequence(2).set_on_stop();
//!
//!     /// Another very simple task that simply loads the "b" page.
//!     async fn b_task_function(client: &mut GooseClient) {
//!       let _response = client.get("/b/");
//!     }
//! ```
//!
//! ## Controlling Clients
//!
//! When Goose starts, it creates one or more [`GooseClient`](./struct.GooseClient.html)s,
//! assigning a single [`GooseTaskSet`](./struct.GooseTaskSet.html) to each. This client is
//! then used to generate load. Behind the scenes, Goose is leveraging the
//! [`reqwest::blocking::client`](https://docs.rs/reqwest/*/reqwest/blocking/struct.Client.html)
//! to load web pages, and Goose can therefor do anything Reqwest can do.
//!
//! The most common request types are [`GET`](./struct.GooseClient.html#method.get) and
//! [`POST`](./struct.GooseClient.html#method.post), but [`HEAD`](./struct.GooseClient.html#method.head),
//! PUT, PATCH and [`DELETE`](./struct.GooseClient.html#method.delete) are also supported.
//!
//! ### GET
//!
//! A helper to make a `GET` request of a path and collect relevant statistics.
//! Automatically prepends the correct host.
//!
//! ```rust
//!     use goose::prelude::*;
//!
//!     let mut task = task!(get_function);
//!
//!     /// A very simple task that makes a GET request.
//!     async  fn get_function(client: &mut GooseClient) {
//!       let _response = client.get("/path/to/foo/");
//!     }
//! ```
//!
//! The returned response is a [`reqwest::blocking::Response`](https://docs.rs/reqwest/*/reqwest/blocking/struct.Response.html)
//! struct. You can use it as you would any Reqwest Response.
//!
//!
//! ### POST
//!
//! A helper to make a `POST` request of a string value to the path and collect relevant
//! statistics. Automatically prepends the correct host. The returned response is a
//! [`reqwest::blocking::Response`](https://docs.rs/reqwest/*/reqwest/blocking/struct.Response.html)
//!
//! ```rust
//!     use goose::prelude::*;
//!
//!     let mut task = task!(post_function);
//!
//!     /// A very simple task that makes a POST request.
//!     async fn post_function(client: &mut GooseClient) {
//!       let _response = client.post("/path/to/foo/", "string value to post".to_string());
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
use reqwest::Error;
use reqwest::{Client, RequestBuilder, Response};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::{future::Future, pin::Pin, time::Instant};
use tokio::sync::mpsc;
use url::Url;

use crate::GooseConfiguration;

static APP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

/// task!(foo) expands to GooseTask::new(foo), but also does some boxing to work around a limitation in the compiler.
#[macro_export]
macro_rules! task {
    ($task_func:ident) => {
        GooseTask::new(move |s| std::boxed::Box::pin($task_func(s)))
    };
}

/// taskset!("foo") expands to GooseTaskSet::new("foo").
#[macro_export]
macro_rules! taskset {
    ($name:tt) => {
        GooseTaskSet::new($name)
    };
}

/// An individual task set.
#[derive(Clone, Hash)]
pub struct GooseTaskSet {
    /// The name of the task set.
    pub name: String,
    /// An integer reflecting where this task set lives in the internal `GooseTest.task_sets` vector.
    pub task_sets_index: usize,
    /// An integer value that controls the frequency that this task set will be assigned to a client.
    pub weight: usize,
    /// An integer value indicating the minimum number of seconds a client will sleep after running a task.
    pub min_wait: usize,
    /// An integer value indicating the maximum number of seconds a client will sleep after running a task.
    pub max_wait: usize,
    /// A vector containing one copy of each GooseTask that will run by clients running this task set.
    pub tasks: Vec<GooseTask>,
    /// A vector of vectors of integers, controlling the sequence and order GooseTasks are run.
    pub weighted_tasks: Vec<Vec<usize>>,
    /// A vector of vectors of integers, controlling the sequence and order on_start GooseTasks are run when the client first starts.
    pub weighted_on_start_tasks: Vec<Vec<usize>>,
    /// A vector of vectors of integers, controlling the sequence and order on_stop GooseTasks are run when a client stops.
    pub weighted_on_stop_tasks: Vec<Vec<usize>>,
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
    ///     /// A very simple task that simply loads the "a" page.
    ///     async fn a_task_function(client: &mut GooseClient) {
    ///       let _response = client.get("/a/");
    ///     }
    /// ```
    pub fn register_task(mut self, mut task: GooseTask) -> Self {
        trace!("{} register_task: {}", self.name, task.name);
        task.tasks_index = self.tasks.len();
        self.tasks.push(task);
        self
    }

    /// Sets a weight on a task set. The larger the value of weight, the more often the task set will
    /// be assigned to clients. For example, if you have task set foo with a weight of 3, and task set
    /// bar with a weight of 1, and you spin up a load test with 8 clients, 6 of them will be running
    /// the foo task set, and 2 will be running the bar task set.
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut example_tasks = taskset!("ExampleTasks").set_weight(3);
    /// ```
    pub fn set_weight(mut self, weight: usize) -> Self {
        trace!("{} set_weight: {}", self.name, weight);
        if weight < 1 {
            error!("{} weight of {} not allowed", self.name, weight);
            std::process::exit(1);
        } else {
            self.weight = weight;
        }
        self
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
    /// `max_weight` is `2`, the client will randomly sleep for 0, 1 or 2 seconds after each task completes.
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut example_tasks = taskset!("ExampleTasks").set_wait_time(0, 1);
    /// ```
    pub fn set_wait_time(mut self, min_wait: usize, max_wait: usize) -> Self {
        trace!(
            "{} set_wait time: min: {} max: {}",
            self.name,
            min_wait,
            max_wait
        );
        if min_wait > max_wait {
            error!(
                "min_wait({}) can't be larger than max_weight({})",
                min_wait, max_wait
            );
            std::process::exit(1);
        }
        self.min_wait = min_wait;
        self.max_wait = max_wait;
        self
    }
}

/// Commands sent between the parent and client threads, and between manager and
/// worker processes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GooseClientCommand {
    /// Tell worker process to pause load test.
    WAIT,
    /// Tell worker process to start load test.
    RUN,
    /// Tell client thread to exit.
    EXIT,
}

/// Supported HTTP methods.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GooseMethod {
    DELETE,
    GET,
    HEAD,
    PATCH,
    POST,
    PUT,
}

fn goose_method_from_method(method: Method) -> GooseMethod {
    match method {
        Method::DELETE => GooseMethod::DELETE,
        Method::GET => GooseMethod::GET,
        Method::HEAD => GooseMethod::HEAD,
        Method::PATCH => GooseMethod::PATCH,
        Method::POST => GooseMethod::POST,
        Method::PUT => GooseMethod::PUT,
        _ => {
            error!("unsupported method: {}", method);
            std::process::exit(1);
        }
    }
}

pub struct GooseRawRequest {
    /// The method being used (ie, GET, POST, etc).
    pub method: GooseMethod,
    /// The optional name of the request.
    pub name: String,
    /// How many milliseconds the request took.
    pub response_time: u128,
    /// The HTTP response code.
    pub status_code: Option<StatusCode>,
    /// Whether or not request was successful.
    pub success: bool,
    /// Whether or not we're updating a previous request.
    pub update: bool,
}
impl GooseRawRequest {
    pub fn new(method: GooseMethod, name: &str) -> Self {
        let name_string = name.to_string();
        GooseRawRequest {
            method,
            name: name_string,
            response_time: 0,
            status_code: None,
            success: true,
            update: false,
        }
    }

    fn set_response_time(&mut self, response_time: u128) {
        self.response_time = response_time;
    }

    fn set_status_code(&mut self, status_code: Option<StatusCode>) {
        self.status_code = status_code;
    }
}

/// Statistics collected about a path-method pair, (for example `/index`-`GET`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GooseRequest {
    /// The path for which statistics are being collected.
    pub path: String,
    /// The method for which statistics are being collected.
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
    pub fn set_response_time(&mut self, response_time: u128) {
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
    pub fn set_status_code(&mut self, status_code: Option<StatusCode>) {
        let status_code_u16 = match status_code {
            Some(s) => s.as_u16(),
            _ => 0,
        };
        let counter = match self.status_code_counts.get(&status_code_u16) {
            // We've seen this status code before, increment counter.
            Some(c) => {
                debug!("got {:?} counter: {}", status_code, c);
                *c + 1
            }
            // First time we've seen this status code, initialize counter.
            None => {
                debug!("no match for counter: {}", status_code_u16);
                1
            }
        };
        self.status_code_counts.insert(status_code_u16, counter);
        debug!("incremented {} counter: {}", status_code_u16, counter);
    }
}

/// An individual client state, repeatedly running all GooseTasks in a specific GooseTaskSet.
#[derive(Debug, Clone)]
pub struct GooseClient {
    /// An index into the internal `GooseTest.task_sets` vector, indicating which GooseTaskSet is running.
    pub task_sets_index: usize,
    /// A [`reqwest.client`](https://docs.rs/reqwest/*/reqwest/struct.Client.html) instance
    pub client: Client,
    /// Channel
    pub parent: Option<mpsc::UnboundedSender<GooseRawRequest>>,
    /// Optional global host, can be overridden per-task-set or via the cli.
    pub default_host: Option<String>,
    /// Optional per-task-set .host.
    pub task_set_host: Option<String>,
    /// Minimum amount of time to sleep after running a task.
    pub min_wait: usize,
    /// Maximum amount of time to sleep after running a task.
    pub max_wait: usize,
    /// A local copy of the global GooseConfiguration.
    pub config: GooseConfiguration,
    /// An index into the internal `GooseTest.weighted_clients, indicating which weighted GooseTaskSet is running.
    pub weighted_clients_index: usize,
    /// A weighted list of all tasks that run when the client first starts.
    pub weighted_on_start_tasks: Vec<Vec<usize>>,
    /// A weighted list of all tasks that this client runs once started.
    pub weighted_tasks: Vec<Vec<usize>>,
    /// A pointer into which sequenced bucket the client is currently running tasks from.
    pub weighted_bucket: usize,
    /// A pointer of which task within the current sequenced bucket is currently running.
    pub weighted_bucket_position: usize,
    /// A weighted list of all tasks that run when the client stops.
    pub weighted_on_stop_tasks: Vec<Vec<usize>>,
    /// Optional name of all requests made within the current task.
    pub task_request_name: Option<String>,
    /// Optional name of all requests made within the current task.
    pub request_name: Option<String>,
    /// Load test hash.
    pub load_test_hash: u64,
}
impl GooseClient {
    /// Create a new client state.
    pub fn new(
        counter: usize,
        task_sets_index: usize,
        default_host: Option<String>,
        task_set_host: Option<String>,
        min_wait: usize,
        max_wait: usize,
        configuration: &GooseConfiguration,
        load_test_hash: u64,
    ) -> Self {
        trace!("new client");
        let builder = Client::builder()
            .user_agent(APP_USER_AGENT)
            .cookie_store(true);
        let client = match builder.build() {
            Ok(c) => c,
            Err(e) => {
                error!(
                    "failed to build client {} for task {}: {}",
                    counter, task_sets_index, e
                );
                std::process::exit(1);
            }
        };
        GooseClient {
            task_sets_index,
            default_host,
            task_set_host,
            client,
            parent: None,
            config: configuration.clone(),
            min_wait,
            max_wait,
            // A value of max_value() indicates this client isn't fully initialized yet.
            weighted_clients_index: usize::max_value(),
            weighted_on_start_tasks: Vec::new(),
            weighted_tasks: Vec::new(),
            weighted_bucket: 0,
            weighted_bucket_position: 0,
            weighted_on_stop_tasks: Vec::new(),
            task_request_name: None,
            request_name: None,
            load_test_hash,
        }
    }

    /// Sets a name for the next request made.
    ///
    /// One example use case of this is to group together requests to different URLs in the
    /// statistics that don't need to be split out, perhaps because they're all the same type
    /// of page.
    ///
    /// # Examples
    ///
    /// In this example, the request will show up as "GET foo":
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut task = task!(get_function);
    ///
    ///     /// A very simple task that makes a GET request.
    ///     async fn get_function(client: &mut GooseClient) {
    ///       let _response = client.set_request_name("foo").get("/path/to/foo");
    ///     }
    /// ```
    ///
    /// In this example, the first request will show up in the statistics as "GET foo", and the
    /// second request will show up as "GET /path/to/foo".
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut task = task!(get_function);
    ///
    ///     /// A very simple task that makes a GET request.
    ///     async fn get_function(client: &mut GooseClient) {
    ///       let _response = client.set_request_name("foo").get("/path/to/foo").await;
    ///       let _response = client.get("/path/to/foo").await;
    ///     }
    /// ```
    pub fn set_request_name(&mut self, name: &str) -> &mut Self {
        if name != "" {
            self.request_name = Some(name.to_string());
        } else {
            self.request_name = None;
        }
        self
    }

    /// A helper that pre-pends a hostname to the path. For example, if you pass in `/foo`
    /// and `--host` is set to `http://127.0.0.1` it will return `http://127.0.0.1/foo`.
    /// Respects per-`GooseTaskSet` `host` configuration, global `GooseAttack` `host`
    /// configuration, and `--host` CLI configuration option.
    ///
    /// If `path` is passed in with a hard-coded host, this will be used instead.
    ///
    /// Host is defined in the following order:
    ///  - If `path` includes the host, use this
    ///  - Otherwise, if `--host` is defined, use this
    ///  - Otherwise, if `GooseTaskSet.host` is defined, use this
    ///  - Otherwise, use global `GooseAttack.host`.
    pub fn build_url(&mut self, path: &str) -> String {
        // If URL includes a host, use it.
        if let Ok(parsed_path) = Url::parse(path) {
            if let Some(_uri) = parsed_path.host() {
                return path.to_string();
            }
        }

        let base_url = if !self.config.host.is_empty() {
            // If the `--host` CLI option is set, use it to build the URL
            Url::parse(&self.config.host).unwrap()
        } else {
            match &self.task_set_host {
                // Otherwise, if `GooseTaskSet.host` is defined, usee this
                Some(host) => Url::parse(host).unwrap(),
                // Otherwise, use global `GooseAttack.host`. `unwrap` okay as host validation was done at startup.
                None => Url::parse(&self.default_host.clone().unwrap()).unwrap(),
            }
        };
        match base_url.join(path) {
            Ok(url) => url.to_string(),
            Err(e) => {
                error!(
                    "failed to build url from base {} and path {} for task {}: {}",
                    &base_url, &path, self.task_sets_index, e
                );
                std::process::exit(1);
            }
        }
    }

    /// A helper to make a `GET` request of a path and collect relevant statistics.
    /// Automatically prepends the correct host.
    ///
    /// (If you need to set headers, change timeouts, or otherwise make use of the
    /// [`reqwest::RequestBuilder`](reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object, you can instead call `goose_get` which returns a RequestBuilder, then
    /// call `goose_send` to invoke the request.)
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut task = task!(get_function);
    ///
    ///     /// A very simple task that makes a GET request.
    ///     async fn get_function(client: &mut GooseClient) {
    ///       let _response = client.get("/path/to/foo/");
    ///     }
    /// ```
    pub async fn get(&mut self, path: &str) -> Result<Response, Error> {
        let request_builder = self.goose_get(path);
        self.goose_send(request_builder).await
    }

    /// A helper to make a `POST` request of a path and collect relevant statistics.
    /// Automatically prepends the correct host.
    ///
    /// (If you need to set headers, change timeouts, or otherwise make use of the
    /// [`reqwest::RequestBuilder`](reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object, you can instead call `goose_post` which returns a RequestBuilder, then
    /// call `goose_send` to invoke the request.)
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut task = task!(post_function);
    ///
    ///     /// A very simple task that makes a POST request.
    ///     async fn post_function(client: &mut GooseClient) {
    ///       let _response = client.post("/path/to/foo/", "BODY BEING POSTED".to_string());
    ///     }
    /// ```
    pub async fn post(&mut self, path: &str, body: String) -> Result<Response, Error> {
        let request_builder = self.goose_post(path).body(body);
        self.goose_send(request_builder).await
    }

    /// A helper to make a `HEAD` request of a path and collect relevant statistics.
    /// Automatically prepends the correct host.
    ///
    /// (If you need to set headers, change timeouts, or otherwise make use of the
    /// [`reqwest::RequestBuilder`](reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object, you can instead call `goose_head` which returns a RequestBuilder, then
    /// call `goose_send` to invoke the request.)
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut task = task!(head_function);
    ///
    ///     /// A very simple task that makes a HEAD request.
    ///     async fn head_function(client: &mut GooseClient) {
    ///       let _response = client.head("/path/to/foo/");
    ///     }
    /// ```
    pub async fn head(&mut self, path: &str) -> Result<Response, Error> {
        let request_builder = self.goose_head(path);
        self.goose_send(request_builder).await
    }

    /// A helper to make a `DELETE` request of a path and collect relevant statistics.
    /// Automatically prepends the correct host.
    ///
    /// (If you need to set headers, change timeouts, or otherwise make use of the
    /// [`reqwest::RequestBuilder`](reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object, you can instead call `goose_delete` which returns a RequestBuilder,
    /// then call `goose_send` to invoke the request.)
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut task = task!(delete_function);
    ///
    ///     /// A very simple task that makes a DELETE request.
    ///     async fn delete_function(client: &mut GooseClient) {
    ///       let _response = client.delete("/path/to/foo/");
    ///     }
    /// ```
    pub async fn delete(&mut self, path: &str) -> Result<Response, Error> {
        let request_builder = self.goose_delete(path);
        self.goose_send(request_builder).await
    }

    /// Prepends the correct host on the path, then prepares a
    /// [`reqwest::RequestBuilder`](reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object for making a `GET` request.
    ///
    /// (You must then call `goose_send` on this object to actually execute the request.)
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut task = task!(get_function);
    ///
    ///     /// A simple task that makes a GET request, exposing the Reqwest
    ///     /// request builder.
    ///     async fn get_function(client: &mut GooseClient) {
    ///       let request_builder = client.goose_get("/path/to/foo");
    ///       let response = client.goose_send(request_builder).await;
    ///     }
    /// ```
    pub fn goose_get(&mut self, path: &str) -> RequestBuilder {
        let url = self.build_url(path);
        self.client.get(&url)
    }

    /// Prepends the correct host on the path, then prepares a
    /// [`reqwest::RequestBuilder`](reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object for making a `POST` request.
    ///
    /// (You must then call `goose_send` on this object to actually execute the request.)
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut task = task!(post_function);
    ///
    ///     /// A simple task that makes a POST request, exposing the Reqwest
    ///     /// request builder.
    ///     async fn post_function(client: &mut GooseClient) {
    ///       let request_builder = client.goose_post("/path/to/foo");
    ///       let response = client.goose_send(request_builder).await;
    ///     }
    /// ```
    pub fn goose_post(&mut self, path: &str) -> RequestBuilder {
        let url = self.build_url(path);
        self.client.post(&url)
    }

    /// Prepends the correct host on the path, then prepares a
    /// [`reqwest::RequestBuilder`](reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object for making a `HEAD` request.
    ///
    /// (You must then call `goose_send` on this object to actually execute the request.)
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut task = task!(head_function);
    ///
    ///     /// A simple task that makes a HEAD request, exposing the Reqwest
    ///     /// request builder.
    ///     async fn head_function(client: &mut GooseClient) {
    ///       let request_builder = client.goose_head("/path/to/foo");
    ///       let response = client.goose_send(request_builder).await;
    ///     }
    /// ```
    pub fn goose_head(&mut self, path: &str) -> RequestBuilder {
        let url = self.build_url(path);
        self.client.head(&url)
    }

    /// Prepends the correct host on the path, then prepares a
    /// [`reqwest::RequestBuilder`](reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object for making a `PUT` request.
    ///
    /// (You must then call `goose_send` on this object to actually execute the request.)
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut task = task!(put_function);
    ///
    ///     /// A simple task that makes a PUT request, exposing the Reqwest
    ///     /// request builder.
    ///     async fn put_function(client: &mut GooseClient) {
    ///       let request_builder = client.goose_put("/path/to/foo");
    ///       let response = client.goose_send(request_builder).await;
    ///     }
    /// ```
    pub fn goose_put(&mut self, path: &str) -> RequestBuilder {
        let url = self.build_url(path);
        self.client.put(&url)
    }

    /// Prepends the correct host on the path, then prepares a
    /// [`reqwest::RequestBuilder`](reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object for making a `PATCH` request.
    ///
    /// (You must then call `goose_send` on this object to actually execute the request.)
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut task = task!(patch_function);
    ///
    ///     /// A simple task that makes a PUT request, exposing the Reqwest
    ///     /// request builder.
    ///     async fn patch_function(client: &mut GooseClient) {
    ///       let request_builder = client.goose_patch("/path/to/foo");
    ///       let response = client.goose_send(request_builder).await;
    ///     }
    /// ```
    pub fn goose_patch(&mut self, path: &str) -> RequestBuilder {
        let url = self.build_url(path);
        self.client.patch(&url)
    }

    /// Prepends the correct host on the path, then prepares a
    /// [`reqwest::RequestBuilder`](reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object for making a `DELETE` request.
    ///
    /// (You must then call `goose_send` on this object to actually execute the request.)
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut task = task!(delete_function);
    ///
    ///     /// A simple task that makes a DELETE request, exposing the Reqwest
    ///     /// request builder.
    ///     async fn delete_function(client: &mut GooseClient) {
    ///       let request_builder = client.goose_delete("/path/to/foo");
    ///       let response = client.goose_send(request_builder).await;
    ///     }
    /// ```
    pub fn goose_delete(&mut self, path: &str) -> RequestBuilder {
        let url = self.build_url(path);
        self.client.delete(&url)
    }

    /// Builds the provided
    /// [`reqwest::RequestBuilder`](reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object and then executes the response. If statistics are being displayed, it
    /// also captures request statistics.
    ///
    /// It is possible to build and execute a `RequestBuilder` object directly with
    /// Reqwest without using this helper function, but then Goose is unable to capture
    /// statistics.
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut task = task!(get_function);
    ///
    ///     /// A simple task that makes a GET request, exposing the Reqwest
    ///     /// request builder.
    ///     async fn get_function(client: &mut GooseClient) {
    ///       let request_builder = client.goose_get("/path/to/foo");
    ///       let response = client.goose_send(request_builder).await;
    ///     }
    /// ```
    pub async fn goose_send(&mut self, request_builder: RequestBuilder) -> Result<Response, Error> {
        let started = Instant::now();
        let request = match request_builder.build() {
            Ok(r) => r,
            Err(e) => {
                error!("goose_send failed to build request: {}", e);
                std::process::exit(1);
            }
        };

        // String version of request path.
        let path = match Url::parse(&request.url().to_string()) {
            Ok(u) => u.path().to_string(),
            Err(e) => {
                error!("failed to parse url: {}", e);
                "".to_string()
            }
        };
        let method = goose_method_from_method(request.method().clone());

        // Make the actual request.
        let response = self.client.execute(request).await;
        let elapsed = started.elapsed();

        // Create a raw request object if we're tracking statistics.
        if !self.config.no_stats {
            // Determine what to name current request.
            let request_name = self.get_request_name(&path);
            let mut raw_request = GooseRawRequest::new(method, &request_name);
            raw_request.set_response_time(elapsed.as_millis());
            match &response {
                Ok(r) => {
                    let status_code = r.status();
                    // Only increment status_code_counts if we're displaying the results
                    if self.config.status_codes {
                        raw_request.set_status_code(Some(status_code));
                    }
                    debug!("{:?}: status_code {}", &path, status_code);
                    // @TODO: match/handle all is_foo() https://docs.rs/http/0.2.1/http/status/struct.StatusCode.html
                    if !status_code.is_success() {
                        raw_request.success = false;
                    }
                }
                Err(e) => {
                    // @TODO: what can we learn from a reqwest error?
                    warn!("{:?}: {}", &path, e);
                    raw_request.success = false;
                    raw_request.set_status_code(None);
                }
            };

            self.send_to_parent(raw_request);
        }

        // @TODO: (improve comment) Consume request_name, if set.
        if self.request_name != None {
            self.request_name = None;
        }

        response
    }

    fn send_to_parent(&mut self, raw_request: GooseRawRequest) {
        let parent = match self.parent.clone() {
            Some(p) => p,
            None => {
                error!("unable to communicate with parent thread, exiting");
                std::process::exit(1);
            }
        };
        match parent.send(raw_request) {
            Ok(_) => (),
            Err(e) => {
                error!("unable to communicate with parent thread, exiting: {}", e);
                std::process::exit(1);
            }
        }
    }

    /// If individual `request_name` is set, use this. Otherwise, if `task_request_name`
    /// is set, use this. Otherwise, use path.
    fn get_request_name(&mut self, path: &str) -> String {
        match &self.request_name {
            Some(rn) => rn.to_string(),
            None => match &self.task_request_name {
                Some(trn) => trn.to_string(),
                None => path.to_string(),
            },
        }
    }

    /// Manually mark a request as a success.
    ///
    /// By default, Goose will consider any response with a 2xx status code as a success. It may be
    /// valid in your test for a non-2xx HTTP status code to be returned.
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut task = task!(get_function);
    ///
    ///     /// A simple task that makes a GET request.
    ///     async fn get_function(client: &mut GooseClient) {
    ///         let response = client.get("/404").await;
    ///         match &response {
    ///             Ok(r) => {
    ///                 // We expect a 404 here.
    ///                 if r.status() == 404 {
    ///                     client.set_success(&GooseMethod::GET, "/404");
    ///                 }
    ///             },
    ///             Err(_) => (),
    ///         }
    ///     }
    /// ````
    pub fn set_success(&mut self, method: &GooseMethod, path: &str) {
        let request_name = self.get_request_name(path);
        let mut update_request = GooseRawRequest::new(method.clone(), &request_name);
        update_request.success = true;
        // This is an updaate to a previously recorded statistic.
        update_request.update = true;

        self.send_to_parent(update_request);
    }

    /// Manually mark a request as a failure.
    ///
    /// By default, Goose will consider any response with a 2xx status code as a success. You may require
    /// more advanced logic, in which a 2xx status code is actually a failure.
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let mut task = task!(loadtest_index_page);
    ///
    ///     async fn loadtest_index_page(client: &mut GooseClient) {
    ///         let response = client.set_request_name("index").get("/").await;
    ///         // Extract the response Result.
    ///         match response {
    ///             Ok(r) => {
    ///                 // We only need to check pages that returned a success status code.
    ///                 if r.status().is_success() {
    ///                     match r.text().await {
    ///                         Ok(text) => {
    ///                             // If the expected string doesn't exist, this page load
    ///                             // was a failure.
    ///                             if !text.contains("this string must exist") {
    ///                                 // As this is a named request, pass in the name not the URL
    ///                                 client.set_failure(&GooseMethod::GET, "index");
    ///                             }
    ///                         }
    ///                         // Empty page, this is a failure.
    ///                         Err(_) => client.set_failure(&GooseMethod::GET, "index"),
    ///                     }
    ///                 }
    ///             },
    ///             // Invalid response, this is already a failure.
    ///             Err(_) => (),
    ///         }
    ///     }
    /// ````
    pub fn set_failure(&mut self, method: &GooseMethod, path: &str) {
        let request_name = self.get_request_name(path);
        let mut update_request = GooseRawRequest::new(method.clone(), &request_name);
        update_request.success = false;
        // This is an updaate to a previously recorded statistic.
        update_request.update = true;

        self.send_to_parent(update_request);
    }
}

/// An individual task within a `GooseTaskSet`.
#[derive(Clone)]
pub struct GooseTask {
    /// An index into GooseTaskSet.task, indicating which task this is.
    pub tasks_index: usize,
    /// An optional name for the task, used when displaying statistics about requests made.
    pub name: String,
    /// An integer value that controls the frequency that this task will be run.
    pub weight: usize,
    /// An integer value that controls when this task runs compared to other tasks in the same GooseTaskSet.
    pub sequence: usize,
    /// A flag indicating that this task runs when the client starts.
    pub on_start: bool,
    /// A flag indicating that this task runs when the client stops.
    pub on_stop: bool,
    /// A required function that is executed each time this task runs.
    pub function: for<'r> fn(&'r mut GooseClient) -> Pin<Box<dyn Future<Output = ()> + Send + 'r>>,
}
impl GooseTask {
    pub fn new(
        function: for<'r> fn(&'r mut GooseClient) -> Pin<Box<dyn Future<Output = ()> + Send + 'r>>,
    ) -> Self {
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

    /// Set an optional name for the task, used when displaying statistics about
    /// requests made by the task.
    ///
    /// Individual requests can also be named withing your load test. See the
    /// documentation for `GooseClient`.[`set_request_name()`](./struct.GooseClient.html#method.set_request_name)
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     task!(my_task_function).set_name("foo");
    ///
    ///     async fn my_task_function(client: &mut GooseClient) {
    ///       let _response = client.get("/");
    ///     }
    /// ```
    pub fn set_name(mut self, name: &str) -> Self {
        trace!("[{}] set_name: {}", self.tasks_index, self.name);
        self.name = name.to_string();
        self
    }

    /// Set an optional flag indicating that this task should be run when
    /// a client first starts. This could be used to log the user in, and
    /// so all subsequent tasks are done as a logged in user. A task with
    /// this flag set will only run at start time (and optionally at stop
    /// time as well, if that flag is also set).
    ///
    /// On-start tasks can be sequenced and weighted. Sequences allow
    /// multiple on-start tasks to run in a controlled order. Weights allow
    /// on-start tasks to run multiple times when a client starts.
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     task!(my_on_start_function).set_on_start();
    ///
    ///     async fn my_on_start_function(client: &mut GooseClient) {
    ///       let _response = client.get("/");
    ///     }
    /// ```
    pub fn set_on_start(mut self) -> Self {
        trace!("{} [{}] set_on_start task", self.name, self.tasks_index);
        self.on_start = true;
        self
    }

    /// Set an optional flag indicating that this task should be run when
    /// a client stops. This could be used to log a user out when the client
    /// finishes its load test. A task with this flag set will only run at
    /// stop time (and optionally at start time as well, if that flag is
    /// also set).
    ///
    /// On-stop tasks can be sequenced and weighted. Sequences allow
    /// multiple on-stop tasks to run in a controlled order. Weights allow
    /// on-stop tasks to run multiple times when a client stops.
    ///
    /// # Example
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     task!(my_on_stop_function).set_on_stop();
    ///
    ///     async fn my_on_stop_function(client: &mut GooseClient) {
    ///       let _response = client.get("/");
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
    ///     use goose::prelude::*;
    ///
    ///     task!(task_function).set_weight(3);
    ///
    ///     async fn task_function(client: &mut GooseClient) {
    ///       let _response = client.get("/");
    ///     }
    /// ```
    pub fn set_weight(mut self, weight: usize) -> Self {
        trace!(
            "{} [{}] set_weight: {}",
            self.name,
            self.tasks_index,
            weight
        );
        if weight < 1 {
            error!("{} weight of {} not allowed", self.name, weight);
            std::process::exit(1);
        } else {
            self.weight = weight;
        }
        self
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
    ///     async fn first_task_function(client: &mut GooseClient) {
    ///       let _response = client.get("/1");
    ///     }
    ///
    ///     async fn second_task_function(client: &mut GooseClient) {
    ///       let _response = client.get("/2");
    ///     }
    ///
    ///     async fn third_task_function(client: &mut GooseClient) {
    ///       let _response = client.get("/3");
    ///     }
    /// ```
    ///
    /// In the following example, the `runs_first` task runs two times, then one instance of `runs_second`
    /// and two instances of `also_runs_second` are all three run. The client will do this over and over
    /// the entire time it runs, with `runs_first` always running first, then the other tasks being
    /// run in a random and weighted order:
    /// ```rust
    ///     use goose::prelude::*;
    ///
    ///     let runs_first = task!(first_task_function).set_sequence(1).set_weight(2);
    ///     let runs_second = task!(second_task_function_a).set_sequence(2);
    ///     let also_runs_second = task!(second_task_function_b).set_sequence(2).set_weight(2);
    ///
    ///     async fn first_task_function(client: &mut GooseClient) {
    ///       let _response = client.get("/1");
    ///     }
    ///
    ///     async fn second_task_function_a(client: &mut GooseClient) {
    ///       let _response = client.get("/2a");
    ///     }
    ///
    ///     async fn second_task_function_b(client: &mut GooseClient) {
    ///       let _response = client.get("/2b");
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

    #[test]
    fn goose_task_set() {
        // Simplistic test task functions.
        async fn test_function_a(client: &mut GooseClient) -> () {
            let _response = client.get("/a/").await;
        }

        async fn test_function_b(client: &mut GooseClient) -> () {
            let _response = client.get("/b/").await;
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
        task_set = task_set.set_weight(50);
        assert_eq!(task_set.weight, 50);
        assert_eq!(task_set.tasks.len(), 3);
        assert_eq!(task_set.weighted_tasks.len(), 0);
        assert_eq!(task_set.task_sets_index, usize::max_value());
        assert_eq!(task_set.min_wait, 0);
        assert_eq!(task_set.max_wait, 0);
        assert_eq!(task_set.host, None);

        // Weight can be changed.
        task_set = task_set.set_weight(5);
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
        task_set = task_set.set_wait_time(1, 10);
        assert_eq!(task_set.min_wait, 1);
        assert_eq!(task_set.max_wait, 10);
        assert_eq!(task_set.host, Some("https://bar.example.com/".to_string()));
        assert_eq!(task_set.weight, 5);
        assert_eq!(task_set.tasks.len(), 3);
        assert_eq!(task_set.weighted_tasks.len(), 0);
        assert_eq!(task_set.task_sets_index, usize::max_value());

        // Wait time can be changed.
        task_set = task_set.set_wait_time(3, 9);
        assert_eq!(task_set.min_wait, 3);
        assert_eq!(task_set.max_wait, 9);
    }

    #[test]
    fn goose_task() {
        // Simplistic test task functions.
        async fn test_function_a(client: &mut GooseClient) -> () {
            let _response = client.get("/a/");
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
        task = task.set_weight(2);
        assert_eq!(task.weight, 2);
        assert_eq!(task.on_stop, true);
        assert_eq!(task.on_start, true);
        assert_eq!(task.name, "bar".to_string());
        assert_eq!(task.sequence, 0);

        // Weight field can be changed multiple times.
        task = task.set_weight(3);
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
        let mut raw_request = GooseRawRequest::new(GooseMethod::GET, "/");
        assert_eq!(raw_request.name, "/".to_string());
        assert_eq!(raw_request.method, GooseMethod::GET);
        assert_eq!(raw_request.response_time, 0);
        assert_eq!(raw_request.status_code, None);
        assert_eq!(raw_request.success, true);
        assert_eq!(raw_request.update, false);

        let response_time = 123;
        raw_request.set_response_time(response_time);
        assert_eq!(raw_request.name, "/".to_string());
        assert_eq!(raw_request.method, GooseMethod::GET);
        assert_eq!(raw_request.response_time, response_time);
        assert_eq!(raw_request.status_code, None);
        assert_eq!(raw_request.success, true);
        assert_eq!(raw_request.update, false);

        let status_code = http::StatusCode::OK;
        raw_request.set_status_code(Some(status_code));
        assert_eq!(raw_request.name, "/".to_string());
        assert_eq!(raw_request.method, GooseMethod::GET);
        assert_eq!(raw_request.response_time, response_time);
        assert_eq!(raw_request.status_code, Some(status_code));
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
        request.set_status_code(Some(StatusCode::OK));
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
        request.set_status_code(Some(StatusCode::OK));
        // We've seen only one unique status code.
        assert_eq!(request.status_code_counts.len(), 1);
        // Second time seeing this status code.
        assert_eq!(request.status_code_counts[&200], 2);

        // Tracking status code updates all related fields.
        request.set_status_code(None);
        // We've seen two unique status codes.
        assert_eq!(request.status_code_counts.len(), 2);
        // First time seeing a client-side error.
        assert_eq!(request.status_code_counts[&0], 1);

        // Tracking status code updates all related fields.
        request.set_status_code(Some(StatusCode::INTERNAL_SERVER_ERROR));
        // We've seen three unique status codes.
        assert_eq!(request.status_code_counts.len(), 3);
        // First time seeing an internal server error.
        assert_eq!(request.status_code_counts[&500], 1);

        // Tracking status code updates all related fields.
        request.set_status_code(Some(StatusCode::PERMANENT_REDIRECT));
        // We've seen four unique status codes.
        assert_eq!(request.status_code_counts.len(), 4);
        // First time seeing an internal server error.
        assert_eq!(request.status_code_counts[&308], 1);

        // Tracking status code updates all related fields.
        request.set_status_code(Some(StatusCode::OK));
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

    #[test]
    fn goose_client() {
        let configuration = GooseConfiguration::default();
        let mut client = GooseClient::new(
            0,
            0,
            Some("http://example.com/".to_string()),
            None,
            0,
            0,
            &configuration,
            0,
        );
        assert_eq!(client.task_sets_index, 0);
        assert_eq!(client.default_host, Some("http://example.com/".to_string()));
        assert_eq!(client.task_set_host, None);
        assert_eq!(client.min_wait, 0);
        assert_eq!(client.max_wait, 0);
        assert_eq!(client.weighted_clients_index, usize::max_value());
        assert_eq!(client.weighted_on_start_tasks.len(), 0);
        assert_eq!(client.weighted_tasks.len(), 0);
        assert_eq!(client.weighted_bucket, 0);
        assert_eq!(client.weighted_bucket_position, 0);
        assert_eq!(client.weighted_on_stop_tasks.len(), 0);
        assert_eq!(client.task_request_name, None);
        assert_eq!(client.request_name, None);

        // Setting request name doesn't affect anything else.
        client.set_request_name("foo");
        assert_eq!(client.request_name, Some("foo".to_string()));
        assert_eq!(client.task_sets_index, 0);
        assert_eq!(client.default_host, Some("http://example.com/".to_string()));
        assert_eq!(client.task_set_host, None);
        assert_eq!(client.min_wait, 0);
        assert_eq!(client.max_wait, 0);
        assert_eq!(client.weighted_clients_index, usize::max_value());
        assert_eq!(client.weighted_on_start_tasks.len(), 0);
        assert_eq!(client.weighted_tasks.len(), 0);
        assert_eq!(client.weighted_bucket, 0);
        assert_eq!(client.weighted_bucket_position, 0);
        assert_eq!(client.weighted_on_stop_tasks.len(), 0);
        assert_eq!(client.task_request_name, None);

        // Can set request name multiple times.
        client.set_request_name("bar");
        assert_eq!(client.request_name, Some("bar".to_string()));

        // Confirm the URLs are correctly built using the default_host.
        let url = client.build_url("/foo");
        assert_eq!(url, "http://example.com/foo");
        let url = client.build_url("bar/");
        assert_eq!(url, "http://example.com/bar/");
        let url = client.build_url("/foo/bar");
        assert_eq!(url, "http://example.com/foo/bar");

        // Confirm the URLs are built with their own specified host.
        let url = client.build_url("https://example.com/foo");
        assert_eq!(url, "https://example.com/foo");
        let url = client.build_url("https://www.example.com/path/to/resource");
        assert_eq!(url, "https://www.example.com/path/to/resource");

        // Create a second client, this time setting a task_set_host.
        let mut client2 = GooseClient::new(
            0,
            0,
            Some("http://www.example.com/".to_string()),
            Some("http://www2.example.com/".to_string()),
            1,
            3,
            &configuration,
            0,
        );
        assert_eq!(
            client2.default_host,
            Some("http://www.example.com/".to_string())
        );
        assert_eq!(
            client2.task_set_host,
            Some("http://www2.example.com/".to_string())
        );
        assert_eq!(client2.min_wait, 1);
        assert_eq!(client2.max_wait, 3);

        // Confirm the URLs are correctly built using the task_set_host.
        let url = client2.build_url("/foo");
        assert_eq!(url, "http://www2.example.com/foo");

        // Confirm URLs are still built with their own specified host.
        let url = client.build_url("https://example.com/foo");
        assert_eq!(url, "https://example.com/foo");

        // Create a GET request.
        let mut goose_request = client.goose_get("/foo");
        let mut built_request = goose_request.build().unwrap();
        assert_eq!(built_request.method(), &Method::GET);
        assert_eq!(built_request.url().as_str(), "http://example.com/foo");
        assert_eq!(built_request.timeout(), None);

        // Create a POST request.
        goose_request = client.goose_post("/path/to/post");
        built_request = goose_request.build().unwrap();
        assert_eq!(built_request.method(), &Method::POST);
        assert_eq!(
            built_request.url().as_str(),
            "http://example.com/path/to/post"
        );
        assert_eq!(built_request.timeout(), None);

        // Create a PUT request.
        goose_request = client.goose_put("/path/to/put");
        built_request = goose_request.build().unwrap();
        assert_eq!(built_request.method(), &Method::PUT);
        assert_eq!(
            built_request.url().as_str(),
            "http://example.com/path/to/put"
        );
        assert_eq!(built_request.timeout(), None);

        // Create a PATCH request.
        goose_request = client.goose_patch("/path/to/patch");
        built_request = goose_request.build().unwrap();
        assert_eq!(built_request.method(), &Method::PATCH);
        assert_eq!(
            built_request.url().as_str(),
            "http://example.com/path/to/patch"
        );
        assert_eq!(built_request.timeout(), None);

        // Create a DELETE request.
        goose_request = client.goose_delete("/path/to/delete");
        built_request = goose_request.build().unwrap();
        assert_eq!(built_request.method(), &Method::DELETE);
        assert_eq!(
            built_request.url().as_str(),
            "http://example.com/path/to/delete"
        );
        assert_eq!(built_request.timeout(), None);

        // Create a HEAD request.
        goose_request = client.goose_head("/path/to/head");
        built_request = goose_request.build().unwrap();
        assert_eq!(built_request.method(), &Method::HEAD);
        assert_eq!(
            built_request.url().as_str(),
            "http://example.com/path/to/head"
        );
        assert_eq!(built_request.timeout(), None);
    }
}
