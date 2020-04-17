use std::collections::HashMap;
use std::time::Instant;

use http::StatusCode;
use http::method::Method;
use reqwest::blocking::{Client, Response, RequestBuilder};
use reqwest::Error;
use url::Url;

use crate::Configuration;

static APP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

/// A global list of all Goose task sets
#[derive(Clone)]
pub struct GooseTaskSets {
    pub task_sets: Vec<GooseTaskSet>,
    pub weighted_clients: Vec<GooseClient>,
    pub weighted_clients_order: Vec<usize>,
}
impl GooseTaskSets {
    pub fn new() -> Self {
        let goose_tasksets = GooseTaskSets { 
            task_sets: Vec::new(),
            weighted_clients: Vec::new(),
            weighted_clients_order: Vec::new(),
        };
        goose_tasksets
    }

    pub fn register_taskset(&mut self, mut taskset: GooseTaskSet) {
        taskset.task_sets_index = self.task_sets.len();
        self.task_sets.push(taskset);
    }
}

/// An individual task set
#[derive(Clone)]
pub struct GooseTaskSet {
    pub name: String,
    // This is the GooseTaskSets.task_sets index
    pub task_sets_index: usize,
    pub weight: usize,
    pub min_wait: usize,
    pub max_wait: usize,
    pub tasks: Vec<GooseTask>,
    pub weighted_tasks: Vec<Vec<usize>>,
    pub weighted_on_start_tasks: Vec<Vec<usize>>,
    pub weighted_on_stop_tasks: Vec<Vec<usize>>,
    pub host: Option<String>,
}
impl GooseTaskSet {
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

    pub fn register_task(&mut self, mut task: GooseTask) {
        trace!("{} register_task: {}", self.name, task.name);
        task.tasks_index = self.tasks.len();
        self.tasks.push(task);
    }

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

    pub fn set_host(mut self, host: &str) -> Self {
        trace!("{} set_host: {}", self.name, host);
        // Host validation happens in main() at startup.
        self.host = Some(host.to_string());
        self
    }

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

#[derive(Debug, Clone)]
pub enum GooseClientMode {
    INIT,
    HATCHING,
    RUNNING,
    EXITING,
}

#[derive(Debug, Clone)]
pub enum GooseClientCommand {
    // Tell client thread to push statistics to parent
    SYNC,
    // Tell client thread to exit
    EXIT,
}

#[derive(Debug, Clone)]
pub struct GooseRequest {
    pub url: String,
    pub method: Method,
    pub response_times: Vec<f32>,
    pub status_code_counts: HashMap<u16, usize>,
    pub success_count: usize,
    pub fail_count: usize,
}
impl GooseRequest {
    pub fn new(url: &str, method: Method) -> Self {
        trace!("new request");
        GooseRequest {
            url: url.to_string(),
            method: method,
            response_times: Vec::new(),
            status_code_counts: HashMap::new(),
            success_count: 0,
            fail_count: 0,
        }
    }

    pub fn set_response_time(&mut self, response_time: f32) {
        self.response_times.push(response_time);
    }

    pub fn set_status_code(&mut self, status_code: StatusCode) {
        let status_code_u16 = status_code.as_u16();
        let counter = match self.status_code_counts.get(&status_code_u16) {
            // We've seen this status code before, increment counter.
            Some(c) => {
                debug!("got {} counter: {}", status_code, c);
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

#[derive(Debug, Clone)]
pub struct GooseClient {
    // This is the GooseTaskSets.task_sets index
    pub task_sets_index: usize,
    // This is the reqwest.blocking.client (@TODO: test with async)
    pub client: Client,
    pub task_set_host: Option<String>,
    pub min_wait: usize,
    pub max_wait: usize,
    pub config: Configuration,
    pub weighted_clients_index: usize,
    pub mode: GooseClientMode,
    pub weighted_on_start_tasks: Vec<Vec<usize>>,
    pub weighted_tasks: Vec<Vec<usize>>,
    pub weighted_bucket: usize,
    pub weighted_bucket_position: usize,
    pub weighted_on_stop_tasks: Vec<Vec<usize>>,
    pub request_name: String,
    pub requests: HashMap<String, GooseRequest>,
}
impl GooseClient {
    /// Create a new client state.
    pub fn new(index: usize, host: Option<String>, min_wait: usize, max_wait: usize, configuration: &Configuration) -> Self {
        trace!("new client");
        let builder = Client::builder()
            .user_agent(APP_USER_AGENT);
        let client = match builder.build() {
            Ok(c) => c,
            Err(e) => {
                error!("failed to build client {}: {}", index, e);
                std::process::exit(1);
            }
        };
        GooseClient {
            task_sets_index: index,
            task_set_host: host,
            client: client,
            config: configuration.clone(),
            min_wait: min_wait,
            max_wait: max_wait,
            weighted_clients_index: usize::max_value(),
            mode: GooseClientMode::INIT,
            weighted_on_start_tasks: Vec::new(),
            weighted_tasks: Vec::new(),
            weighted_bucket: 0,
            weighted_bucket_position: 0,
            weighted_on_stop_tasks: Vec::new(),
            request_name: "".to_string(),
            requests: HashMap::new(),
        }
    }

    pub fn set_mode(&mut self, mode: GooseClientMode) {
        self.mode = mode;
    }

    fn get_request(&mut self, url: &str, method: &Method) -> GooseRequest {
        let key = format!("{:?} {}", method, url);
        trace!("get key: {}", &key);
        match self.requests.get(&key) {
            Some(r) => r.clone(),
            None => GooseRequest::new(url, method.clone()),
        }
    }

    fn set_request(&mut self, url: &str, method: &Method, request: GooseRequest) {
        let key = format!("{:?} {}", method, url);
        trace!("set key: {}", &key);
        self.requests.insert(key, request.clone());
    }

    fn build_url(&mut self, path: &str) -> String {
        if self.config.host.len() > 0 {
            format!("{}{}", self.config.host, path)
        } else {
            // If no global URL is configured a task_set_host must be, so unwrap() is safe here.
            format!("{}{}", self.task_set_host.clone().unwrap(), path)
        }
    }

    // Simple get() wrapper that calls goose_get() followed by goose_send().
    pub fn get(&mut self, path: &str) -> Result<Response, Error> {
        let request_builder = self.goose_get(path);
        let response = self.goose_send(request_builder);
        response
    }

    // Simple post() wrapper that calls goose_post() followed by goose_send().
    // @TODO: helper should allow for a body
    // @TODO: remove this allow once we convert Goose to a library.
    #[allow(dead_code)]
    pub fn post(&mut self, path: &str) -> Result<Response, Error> {
        let request_builder = self.goose_post(path);
        let response = self.goose_send(request_builder);
        response
    }

    // Simple head() wrapper that calls goose_head() followed by goose_send().
    // @TODO: remove this allow once we convert Goose to a library.
    #[allow(dead_code)]
    pub fn head(&mut self, path: &str) -> Result<Response, Error> {
        let request_builder = self.goose_head(path);
        let response = self.goose_send(request_builder);
        response
    }

    // Simple delete() wrapper that calls goose_delete() followed by goose_send().
    // @TODO: remove this allow once we convert Goose to a library.
    #[allow(dead_code)]
    pub fn delete(&mut self, path: &str) -> Result<Response, Error> {
        let request_builder = self.goose_delete(path);
        let response = self.goose_send(request_builder);
        response
    }

    // Calls Reqwest get() and returns a Reqwest RequestBuilder.
    pub fn goose_get(&mut self, path: &str) -> RequestBuilder {
        let url = self.build_url(path);
        self.client.get(&url)
    }

    // Calls Reqwest post() and returns a Reqwest RequestBuilder.
    // @TODO: remove this allow once we convert Goose to a library.
    #[allow(dead_code)]
    pub fn goose_post(&mut self, path: &str) -> RequestBuilder {
        let url = self.build_url(path);
        self.client.post(&url)
    }

    // Calls Reqwest head() and returns a Reqwest RequestBuilder.
    // @TODO: remove this allow once we convert Goose to a library.
    #[allow(dead_code)]
    pub fn goose_head(&mut self, path: &str) -> RequestBuilder {
        let url = self.build_url(path);
        self.client.head(&url)
    }

    // Calls Reqwest put() and returns a Reqwest RequestBuilder.
    // @TODO: remove this allow once we convert Goose to a library.
    #[allow(dead_code)]
    pub fn goose_put(&mut self, path: &str) -> RequestBuilder {
        let url = self.build_url(path);
        self.client.put(&url)
    }

    // Calls Reqwest patch() and returns a Reqwest RequestBuilder.
    // @TODO: remove this allow once we convert Goose to a library.
    #[allow(dead_code)]
    pub fn goose_patch(&mut self, path: &str) -> RequestBuilder {
        let url = self.build_url(path);
        self.client.patch(&url)
    }

    // Calls Reqwest delete() and returns a Reqwest RequestBuilder.
    // @TODO: remove this allow once we convert Goose to a library.
    #[allow(dead_code)]
    pub fn goose_delete(&mut self, path: &str) -> RequestBuilder {
        let url = self.build_url(path);
        self.client.delete(&url)
    }

    // Executes a Reqwest RequestBuilder, optionally capturing statistics.
    pub fn goose_send(&mut self, request_builder: RequestBuilder) -> Result<Response, Error> {
        let started = Instant::now();
        let request = request_builder.build()?;

        // Allow introspection.
        let method = request.method().clone();
        let url = request.url().to_string();

        // Make the actual request.
        let response = self.client.execute(request);
        let elapsed = started.elapsed() * 100;

        if self.config.print_stats {
            // Introspect the request for logging and statistics
            let path = match Url::parse(&url) {
                Ok(u) => u.path().to_string(),
                Err(e) => {
                    warn!("failed to parse url: {}", e);
                    "parse error".to_string()
                }
            };
            // By default requests are recorded as "METHOD URL", allow override of "METHOD NAME"
            let request_name;
            if self.request_name != "" {
                request_name = self.request_name.to_string();
            }
            else {
                request_name = path.to_string();
            }
            let mut goose_request = self.get_request(&request_name, &method.clone());
            goose_request.set_response_time(elapsed.as_secs_f32());
            match &response {
                Ok(r) => {
                    let status_code = r.status();
                    // Only increment status_code_counts if we're displaying the results
                    if self.config.status_codes {
                        goose_request.set_status_code(status_code);
                    }

                    debug!("{:?}: status_code {}", &path, status_code);
                    // @TODO: match/handle all is_foo() https://docs.rs/http/0.2.1/http/status/struct.StatusCode.html
                    if status_code.is_success() {
                        goose_request.success_count += 1;
                    }
                    // @TODO: properly track redirects and other code ranges
                    else {
                        // @TODO: handle this correctly
                        debug!("{:?}: non-success status_code: {:?}", &path, status_code);
                        goose_request.fail_count += 1;
                    }
                }
                Err(e) => {
                    // @TODO: what can we learn from a reqwest error?
                    debug!("{:?}: error: {}", &path, e);
                    goose_request.fail_count += 1;
                }
            };
            self.set_request(&request_name, &method, goose_request);
        }
        response
    }
}

/// An individual task within a task set
#[derive(Clone)]
pub struct GooseTask {
    // This is the GooseTaskSet.tasks index
    pub tasks_index: usize,
    pub name: String,
    pub weight: usize,
    pub sequence: usize,
    pub on_start: bool,
    pub on_stop: bool,
    pub function: Option<fn(&mut GooseClient)>,
}
impl GooseTask {
    pub fn new() -> Self {
        trace!("new task");
        let task = GooseTask {
            tasks_index: usize::max_value(),
            name: "".to_string(),
            weight: 1,
            sequence: 0,
            on_start: false,
            on_stop: false,
            function: None,
        };
        task
    }

    pub fn named(name: &str) -> Self {
        trace!("new task: {}", name);
        let task = GooseTask {
            tasks_index: usize::max_value(),
            name: name.to_string(),
            weight: 1,
            sequence: 0,
            on_start: false,
            on_stop: false,
            function: None,
        };
        task
    }

    pub fn set_on_start(mut self) -> Self {
        trace!("{} [{}] set_on_start task", self.name, self.tasks_index);
        self.on_start = true;
        self
    }

    // @TODO: remove this allow once we convert Goose to a library.
    #[allow(dead_code)]
    pub fn set_on_stop(mut self) -> Self {
        trace!("{} [{}] set_on_stop task", self.name, self.tasks_index);
        self.on_stop = true;
        self
    }

    // @TODO: remove this allow once we convert Goose to a library.
    #[allow(dead_code)]
    pub fn set_name(mut self, name: &str) -> Self {
        trace!("[{}] set_name: {}", self.tasks_index, self.name);
        self.name = name.to_string();
        self
    }

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

    pub fn set_sequence(mut self, sequence: usize) -> Self {
        trace!("{} [{}] set_sequence: {}", self.name, self.tasks_index, sequence);
        if sequence < 1 {
            info!("setting sequence to 0 for task {} is unnecessary, sequence disabled", self.name);
        }
        self.sequence = sequence;
        self
    }

    pub fn set_function(mut self, function: fn(&mut GooseClient)) -> Self {
        self.function = Some(function);
        self
    }
}
