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
//!     let mut loadtest_tasks = GooseTaskSet::new("LoadtestTasks");
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
//!     let mut foo_tasks = GooseTaskSet::new("FooTasks").set_weight(10);
//!     let mut bar_tasks = GooseTaskSet::new("BarTasks").set_weight(5);
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
//!     foo_tasks.set_host("http://www.local");
//!     bar_tasks.set_host("http://www2.local");
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
//!     foo_tasks.set_wait_time(0, 3);
//!     bar_tasks.set_host(5, 10);
//! ```
//! ## Creating Tasks
//! 
//! A [`GooseTask`](./struct.GooseTask.html) must include a pointer to a function which
//! will be executed each time the task is run.
//! 
//! ```rust
//!     let mut a_task = GooseTask::new(task_function);
//! ```
//! 
//! ### Task Name
//! 
//! A name can be assigned to a task, and will be displayed in statistics about all requests
//! made by the task.
//! 
//! ```rust
//!     let mut a_task = GooseTask::new(task_function).set_name("a");
//! ```
//! 
//! ### Task Weight
//! 
//! Individual tasks can be assigned a weight, controlling how often the task runs. The
//! larger the value of weight, the more it will run. In the following example, `a_task`
//! runs 3 times as often as `b_task`:
//! 
//! ```rust
//!     let mut a_task = GooseTask::new(a_task_function).set_weight(9);
//!     let mut b_task = GooseTask::new(b_task_function).set_weight(3);
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
//!     let mut a_task = GooseTask::new(a_task_function).set_sequence(1);
//!     let mut b_task = GooseTask::new(b_task_function).set_sequence(2);
//!     let mut c_task = GooseTask::new(c_task_function);
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
//!     a_task.set_on_start();
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
//!     b_task.set_on_stop();
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
//!     let _response = client.get("/path/to/foo");
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
//!     let _response = client.post("/path/to/foo", "string value to post");
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

use std::collections::HashMap;
use std::time::Instant;

use http::StatusCode;
use http::method::Method;
use reqwest::blocking::{Client, Response, RequestBuilder};
use reqwest::Error;
use url::Url;

use crate::GooseConfiguration;

static APP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

/// An individual task set.
#[derive(Clone)]
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
    /// registered with the GooseState object. The returned object must be stored in a mutable value.
    /// 
    /// # Example
    /// ```rust
    ///     let mut example_tasks = GooseTaskSet::new("ExampleTasks");
    /// ```
    pub fn new(name: &str) -> Self {
        trace!("new taskset: name: {}", &name);
        let task_set = GooseTaskSet { 
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
        };
        task_set
    }

    /// Registers a GooseTask with a GooseTaskSet, where it is stored in the GooseTaskSet.tasks vector. The
    /// function associated with the task will be run during the load test.
    /// 
    /// # Example
    /// ```rust
    ///     let mut example_tasks = GooseTaskSet::new("ExampleTasks");
    ///     example_tasks.register_task(GooseTask::new(a_task_function));
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
    ///     let mut example_tasks = GooseTaskSet::new("ExampleTasks").set_weight(3);
    /// ```
    pub fn set_weight(mut self, weight: usize) -> Self {
        trace!("{} set_weight: {}", self.name, weight);
        if weight < 1 {
            error!("{} weight of {} not allowed", self.name, weight);
            std::process::exit(1);
        }
        else {
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
    ///     let mut example_tasks = GooseTaskSet::new("ExampleTasks").set_host("http://10.1.1.42");
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
    ///     let mut example_tasks = GooseTaskSet::new("ExampleTasks").set_wait_time(0, 1);
    /// ```
    pub fn set_wait_time(mut self, min_wait: usize, max_wait: usize) -> Self {
        trace!("{} set_wait time: min: {} max: {}", self.name, min_wait, max_wait);
        if min_wait > max_wait {
            error!("min_wait({}) can't be larger than max_weight({})", min_wait, max_wait);
            std::process::exit(1);
        }
        self.min_wait = min_wait;
        self.max_wait = max_wait;
        self
    }
}

/// Tracks the current run-mode of a client.
#[derive(Debug, Clone)]
pub enum GooseClientMode {
    /// Clients are briefly in the INIT mode when first allocated.
    INIT,
    /// Clients are briefly in the HATCHING mode when setting things up.
    HATCHING,
    /// Clients spend most of their time in the RUNNING mode, executing tasks.
    RUNNING,
    /// Clients are briefly in the EXITING mode when stopping.
    EXITING,
}

/// Commands sent between the parent and client threads.
#[derive(Debug, Clone)]
pub enum GooseClientCommand {
    /// Tell client thread to push statistics to parent
    SYNC,
    /// Tell client thread to exit
    EXIT,
}

/// Statistics collected about a path-method pair, (for example `/index`-`GET`).
#[derive(Debug, Clone)]
pub struct GooseRequest {
    /// The path for which statistics are being collected.
    pub path: String,
    /// The method for which statistics are being collected.
    pub method: Method,
    /// Per-response-time counters, tracking how often pages are returned with this response time.
    pub response_times: HashMap<usize, usize>,
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
}
impl GooseRequest {
    /// Create a new GooseRequest object.
    fn new(path: &str, method: Method) -> Self {
        trace!("new request");
        GooseRequest {
            path: path.to_string(),
            method: method,
            response_times: HashMap::new(),
            min_response_time: 0,
            max_response_time: 0,
            total_response_time: 0,
            response_time_counter: 0,
            status_code_counts: HashMap::new(),
            success_count: 0,
            fail_count: 0,
        }
    }

    /// Append response time to `response_times` vector.
    fn set_response_time(&mut self, response_time: u128) {
        // Round the response time so we can combine similar times together and
        // minimize required memory to store and push upstream to the parent.
        let rounded_response_time: usize;
        // No rounding for 1-10ms response times.
        if response_time < 10 {
            rounded_response_time = response_time as usize;
        }
        // Round to nearest 10 for 10-100ms response times.
        else if response_time < 100 {
            rounded_response_time = ((response_time as f64 / 10.0).round() * 10.0) as usize;
        }
        // Round to nearest 100 for 100-1000ms response times.
        else if response_time < 1000 {
            rounded_response_time = ((response_time as f64 / 100.0).round() * 100.0) as usize;
        }
        // Round to nearest 1000 for all larger response times.
        else {
            rounded_response_time = ((response_time as f64 / 10000.0).round() * 10000.0) as usize;
        }

        // Update min_response_time if this one is fastest yet.
        if self.min_response_time == 0 || rounded_response_time < self.min_response_time {
            self.min_response_time = rounded_response_time;
        }

        // Update max_response_time if this one is slowest yet.
        if rounded_response_time > self.max_response_time {
            self.max_response_time = rounded_response_time;
        }

        // Update total_respone time, adding in this one.
        self.total_response_time += rounded_response_time;

        // Increment counter tracking total number of response times seen.
        self.response_time_counter += 1;

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
    fn set_status_code(&mut self, status_code: Option<StatusCode>) {
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
    /// A [`reqwest.blocking.client`](https://docs.rs/reqwest/*/reqwest/blocking/struct.Client.html) instance (@TODO: async).
    pub client: Client,
    /// The global GooseState host.
    pub default_host: Option<String>,
    /// The GooseTaskSet.host.
    pub task_set_host: Option<String>,
    /// Minimum amount of time to sleep after running a task.
    pub min_wait: usize,
    /// Maximum amount of time to sleep after running a task.
    pub max_wait: usize,
    /// A local copy of the global GooseConfiguration.
    pub config: GooseConfiguration,
    /// An index into the internal `GooseTest.weighted_clients, indicating which weighted GooseTaskSet is running.
    pub weighted_clients_index: usize,
    /// The current run mode of this client, see `enum GooseClientMode`.
    pub mode: GooseClientMode,
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
    /// Store the previous url.
    pub previous_path: Option<String>,
    /// Store the previous url.
    pub previous_method: Option<Method>,
    /// Store the optional request_name allowing tasks to toggle success/failure.
    pub previous_request_name: Option<String>,
    /// Store if the previous request was a success (false for failure).
    pub was_success: bool,
    /// Optional statistics collected about all requests made by this client.
    pub requests: HashMap<String, GooseRequest>,
}
impl GooseClient {
    /// Create a new client state.
    pub fn new(counter: usize, task_sets_index: usize, default_host: Option<String>, task_set_host: Option<String>, min_wait: usize, max_wait: usize, configuration: &GooseConfiguration) -> Self {
        trace!("new client");
        let builder = Client::builder()
            .user_agent(APP_USER_AGENT)
            .cookie_store(true);
        let client = match builder.build() {
            Ok(c) => c,
            Err(e) => {
                error!("failed to build client {} for task {}: {}", counter, task_sets_index, e);
                std::process::exit(1);
            }
        };
        GooseClient {
            task_sets_index: task_sets_index,
            default_host: default_host,
            task_set_host: task_set_host,
            client: client,
            config: configuration.clone(),
            min_wait: min_wait,
            max_wait: max_wait,
            // A value of max_value() indicates this client isn't fully initialized yet.
            weighted_clients_index: usize::max_value(),
            mode: GooseClientMode::INIT,
            weighted_on_start_tasks: Vec::new(),
            weighted_tasks: Vec::new(),
            weighted_bucket: 0,
            weighted_bucket_position: 0,
            weighted_on_stop_tasks: Vec::new(),
            task_request_name: None,
            request_name: None,
            previous_path: None,
            previous_method: None,
            previous_request_name: None,
            was_success: false,
            requests: HashMap::new(),
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
    ///     let _response = client.set_request_name("foo").get("/path/to/foo");
    /// ```
    /// 
    /// In this example, the first request will show up in the statistics as "GET foo", and the
    /// second request will show up as "GET /path/to/foo".
    /// ```rust
    ///     let _response = client.set_request_name("foo").get("/path/to/foo");
    ///     let _response = client.get("/path/to/foo");
    /// ```
    pub fn set_request_name(&mut self, name: &str) -> &mut Self {
        if name != "" {
            self.request_name = Some(name.to_string());
        }
        else {
            self.request_name = None;
        }
        self
    }

    /// Sets the current run mode of this client.
    pub fn set_mode(&mut self, mode: GooseClientMode) {
        self.mode = mode;
    }

    /// Checks if the current path-method pair has been requested before.
    fn get_request(&mut self, path: &str, method: &Method) -> GooseRequest {
        let key = format!("{:?} {}", method, path);
        trace!("get key: {}", &key);
        match self.requests.get(&key) {
            Some(r) => r.clone(),
            None => GooseRequest::new(path, method.clone()),
        }
    }

    /// Stores request statistics about the current path-method pair.
    fn set_request(&mut self, path: &str, method: &Method, request: GooseRequest) {
        let key = format!("{:?} {}", method, path);
        trace!("set key: {}", &key);
        self.requests.insert(key, request.clone());
    }

    /// A helper that pre-pends a hostname to the path. For example, if you pass in `/foo`
    /// and `--host` is set to `http://127.0.0.1` it will return `http://127.0.0.1/foo`.
    /// Respects per-`GooseTaskSet` `host` configuration, global `GooseState` `host`
    /// configuration, and `--host` CLI configuration option.
    /// 
    /// If `path` is passed in with a hard-coded host, this will be used instead.
    /// 
    /// Host is defined in the following order:
    ///  - If `path` includes the host, use this
    ///  - Otherwise, if `--host` is defined, use this
    ///  - Otherwise, if `GooseTaskSet.host` is defined, use this
    ///  - Otherwise, use global `GooseState.host`.
    pub fn build_url(&mut self, path: &str) -> String {
        // If URL includes a host, use it.
        if let Ok(parsed_path) = Url::parse(path) {
            if let Some(_uri) = parsed_path.host() {
                return path.to_string()
            }
        }

        let base_url;
        // If the `--host` CLI option is set, use it to build the URL
        if self.config.host.len() > 0 {
            base_url = Url::parse(&self.config.host).unwrap();
        }
        else {
            base_url = match &self.task_set_host {
                // Otherwise, if `GooseTaskSet.host` is defined, usee this
                Some(host) => Url::parse(host).unwrap(),
                // Otherwise, use global `GooseState.host`. `unwrap` okay as host validation was done at startup.
                None => Url::parse(&self.default_host.clone().unwrap()).unwrap(),
            };
        }
        match base_url.join(path) {
            Ok(url) => url.to_string(),
            Err(e) => {
                error!("failed to build url from base {} and path {} for task {}: {}", &base_url, &path, self.task_sets_index, e);
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
    ///     let _response = client.get("/path/to/foo");
    /// ```
    pub fn get(&mut self, path: &str) -> Result<Response, Error> {
        let request_builder = self.goose_get(path);
        let response = self.goose_send(request_builder);
        response
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
    ///     let _response = client.post("/path/to/foo", "BODY BEING POSTED");
    /// ```
    pub fn post(&mut self, path: &str, body: String) -> Result<Response, Error> {
        let request_builder = self.goose_post(path).body(body);
        let response = self.goose_send(request_builder);
        response
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
    ///     let _response = client.head("/path/to/foo");
    /// ```
    pub fn head(&mut self, path: &str) -> Result<Response, Error> {
        let request_builder = self.goose_head(path);
        let response = self.goose_send(request_builder);
        response
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
    ///     let _response = client.delete("/path/to/foo");
    /// ```
    pub fn delete(&mut self, path: &str) -> Result<Response, Error> {
        let request_builder = self.goose_delete(path);
        let response = self.goose_send(request_builder);
        response
    }

    /// Prepends the correct host on the path, then prepares a
    /// [`reqwest::RequestBuilder`](reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object for making a `GET` request.
    /// 
    /// (You must then call `goose_send` on this object to actually execute the request.)
    /// 
    /// # Example
    /// ```rust
    ///     let request_builder = client.goose_get("/path/to/foo");
    ///     let response = self.goose_send(request_builder);
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
    ///     let request_builder = client.goose_post("/path/to/foo");
    ///     let response = self.goose_send(request_builder);
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
    ///     let request_builder = client.goose_head("/path/to/foo");
    ///     let response = self.goose_send(request_builder);
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
    ///     let request_builder = client.goose_put("/login");
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
    ///     let request_builder = client.goose_patch("/path/to/foo");
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
    ///     let request_builder = client.goose_delete("/path/to/foo");
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
    ///     let request_builder = client.goose_get("/path/to/foo");
    ///     let response = self.goose_send(request_builder);
    /// ```
    pub fn goose_send(&mut self, request_builder: RequestBuilder) -> Result<Response, Error> {
        let started = Instant::now();
        let request = match request_builder.build() {
            Ok(r) => r,
            Err(e) => {
                error!("goose_send failed to build request: {}", e);
                std::process::exit(1);
            }
        };

        // Allows introspection, and toggling success/failure.
        self.previous_method = Some(request.method().clone());
        self.previous_path = match Url::parse(&request.url().to_string()) {
            Ok(u) => Some(u.path().to_string()),
            Err(e) => {
                error!("failed to parse url: {}", e);
                None
            }
        };
        self.previous_request_name = self.request_name.clone();

        // Make the actual request.
        let response = self.client.execute(request);
        let elapsed = started.elapsed() * 100;

        if self.config.print_stats {
            let path = self.previous_path.clone().expect("failed to unwrap previous_path").to_string();
            let method = self.previous_method.clone().expect("failed to unwrap previous_method");
            // Requests are named per-request, per-task, or based on the path loaded.
            let request_name = match &self.request_name {
                Some(rn) => rn.to_string(),
                None => match &self.task_request_name {
                    Some(trn) => trn.to_string(),
                    None => path.to_string(),
                }
            };

            let mut goose_request = self.get_request(&request_name, &method);
            goose_request.set_response_time(elapsed.as_millis());
            match &response {
                Ok(r) => {
                    let status_code = r.status();
                    // Only increment status_code_counts if we're displaying the results
                    if self.config.status_codes {
                        goose_request.set_status_code(Some(status_code));
                    }

                    debug!("{:?}: status_code {}", &path, status_code);
                    // @TODO: match/handle all is_foo() https://docs.rs/http/0.2.1/http/status/struct.StatusCode.html
                    if status_code.is_success() {
                        goose_request.success_count += 1;
                        self.was_success = true;
                    }
                    // @TODO: properly track redirects and other code ranges
                    else {
                        // @TODO: handle this correctly
                        debug!("{:?}: non-success status_code: {:?}", &path, status_code);
                        goose_request.fail_count += 1;
                        self.was_success = false;
                    }
                }
                Err(e) => {
                    // @TODO: what can we learn from a reqwest error?
                    warn!("{:?}: {}", &path, e);
                    goose_request.fail_count += 1;
                    self.was_success = false;
                    if self.config.status_codes {
                        goose_request.set_status_code(None);
                    }
                }
            };
            self.set_request(&request_name, &method, goose_request);
        }

        // Consume self.request_name if it was set.
        match self.request_name {
            Some(_) => self.request_name = None,
            None => (),
        };

        response
    }

    /// Helper to determine which request_name was used in the previous request.
    fn get_previous_request_name(&mut self) -> String {
        match &self.previous_request_name {
            Some(prn) => prn.to_string(),
            None => match &self.task_request_name {
                Some(trn) => trn.to_string(),
                None => self.previous_path.clone().expect("failed to unwrap previous_path").to_string(),
            }
        }
    }

    /// Manually mark a request as a success.
    /// 
    /// By default, Goose will consider any response with a 2xx status code as a success. It may be
    /// valid in your test for a non-2xx HTTP status code to be returned.
    /// 
    /// # Example
    /// ```rust
    ///     let response = client.get("/404");
    ///     match &response {
    ///         Ok(r) => {
    ///             // We expect a 404 here.
    ///             if r.status() == 404 {
    ///                 client.set_success();
    ///             }
    ///         },
    ///         Err(_) => (),
    ///         }
    ///     }
    /// ````
    pub fn set_success(&mut self) {
        // If the last request was a success, we don't need to change anything.
        if !self.was_success {
            let request_name = self.get_previous_request_name();
            let previous_method = self.previous_method.clone().expect("failed to unwrap previous_method");
            let mut goose_request = self.get_request(&request_name.to_string(), &previous_method.clone());
            goose_request.success_count += 1;
            goose_request.fail_count -= 1;
            self.set_request(&request_name, &previous_method, goose_request);
        }
    }

    /// Manually mark a request as a failure.
    /// 
    /// By default, Goose will consider any response with a 2xx status code as a success. You may require
    /// more advanced logic, in which a 2xx status code is actually a failure.
    /// 
    /// # Example
    /// ```rust
    /// fn loadtest_index(client: &mut GooseClient) {
    ///     let response = client.set_request_name("index").get("/");
    ///     // Extract the response Result.
    ///     match response {
    ///         Ok(r) => {
    ///             // We only need to check pages that returned a success status code.
    ///             if r.status().is_success() {
    ///                 match r.text() {
    ///                     Ok(text) => {
    ///                         // If the expected string doesn't exist, this page load
    ///                         // was a failure.
    ///                         if !text.contains("this string must exist") {
    ///                             client.set_failure();
    ///                         }
    ///                     }
    ///                     // Empty page, this is a failure.
    ///                     Err(_) => client.set_failure(),
    ///                 }
    ///             }
    ///         },
    ///         // Invalid response, this is already a failure.
    ///         Err(_) => (),
    ///     }
    /// }
    /// ````
    pub fn set_failure(&mut self) {
        // If the last request was a failure, we don't need to change anything.
        if self.was_success {
            let request_name = self.get_previous_request_name();
            let previous_method = self.previous_method.clone().expect("failed to unwrap previous_method");
            let mut goose_request = self.get_request(&request_name.to_string(), &previous_method.clone());
            goose_request.success_count -= 1;
            goose_request.fail_count += 1;
            self.set_request(&request_name, &previous_method, goose_request);
        }
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
    pub function: fn(&mut GooseClient),
}
impl GooseTask {
    pub fn new(function: fn(&mut GooseClient)) -> Self {
        trace!("new task");
        let task = GooseTask {
            tasks_index: usize::max_value(),
            name: "".to_string(),
            weight: 1,
            sequence: 0,
            on_start: false,
            on_stop: false,
            function: function,
        };
        task
    }

    /// Set an optional name for the task, used when displaying statistics about
    /// requests made by the task. 
    /// 
    /// Individual requests can also be named withing your load test. See the
    /// documentation for `GooseClient`.[`set_request_name()`](./struct.GooseClient.html#method.set_request_name)
    /// 
    /// # Example
    /// ```rust
    ///     GooseTask::new(my_task_function).set_name("foo");
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
    ///     GooseTask::new(my_on_start_function).set_on_start();
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
    ///     GooseTask::new(my_on_stop_function).set_on_stop();
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
    ///     GooseTask::new(task_function).set_weight(3);
    /// ```
    pub fn set_weight(mut self, weight: usize) -> Self {
        trace!("{} [{}] set_weight: {}", self.name, self.tasks_index, weight);
        if weight < 1 {
            error!("{} weight of {} not allowed", self.name, weight);
            std::process::exit(1);
        }
        else {
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
    ///     let runs_first = GooseTask::new(first_task_function).set_sequence(3);
    ///     let runs_second = GooseTask::new(second_task_function).set_sequence(5835);
    ///     let runs_last = GooseTask::new(third_task_function);
    /// ```
    /// 
    /// In the following example, the `runs_first` task runs two times, then one instance of `runs_second`
    /// and two instances of `also_runs_second` are all three run. The client will do this over and over
    /// the entire time it runs, with `runs_first` always running first, then the other tasks being
    /// run in a random and weighted order:
    /// ```rust
    ///     let runs_first = GooseTask::new(first_task_function).set_sequence(1).set_weight(2);
    ///     let runs_second = GooseTask::new(second_task_function_a).set_sequence(2);
    ///     let also_runs_second = GooseTask::new(second_task_function_b).set_sequence(2).set_weight(2);
    /// ```
    pub fn set_sequence(mut self, sequence: usize) -> Self {
        trace!("{} [{}] set_sequence: {}", self.name, self.tasks_index, sequence);
        if sequence < 1 {
            info!("setting sequence to 0 for task {} is unnecessary, sequence disabled", self.name);
        }
        self.sequence = sequence;
        self
    }
}
