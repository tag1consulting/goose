use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Instant;

use http::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::Error;

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

    pub fn register_taskset(&mut self, taskset: GooseTaskSet) {
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
    pub tasks: Vec<GooseTask>,
    pub weighted_tasks: Vec<usize>,
}
impl GooseTaskSet {
    pub fn new(name: &str) -> Self {
        trace!("new taskset: name: {}", &name);
        let task_set = GooseTaskSet { 
            name: name.to_string(),
            task_sets_index: usize::max_value(),
            weight: 1,
            tasks: Vec::new(),
            weighted_tasks: Vec::new(),
        };
        task_set
    }

    pub fn register_task(&mut self, task: GooseTask) {
        trace!("{} register_task: {}", self.name, task.name);
        self.tasks.push(task);
    }

    pub fn set_weight(mut self, weight: usize) -> Self {
        trace!("{} set_weight: {}", self.name, weight);
        if weight < 1 {
            info!("{} weight of {} not allowed, set to 1", self.name, weight);
            self.weight = 1;
        }
        else {
            self.weight = weight;
        }
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
pub enum GooseRequestMethod {
    GET,
    //POST,
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
    pub method: GooseRequestMethod,
    pub response_times: Vec<f32>,
    pub status_code_counts: HashMap<u16, usize>,
    pub success_count: usize,
    pub fail_count: usize,
}
impl GooseRequest {
    pub fn new(url: &str, method: GooseRequestMethod, ) -> Self {
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
    pub weighted_clients_index: usize,
    pub mode: GooseClientMode,
    pub weighted_tasks: Vec<usize>,
    pub weighted_position: usize,
    pub requests: HashMap<String, GooseRequest>,
    // Per-task statistics, using task index (@TODO: remove, the)
    pub response_times: Vec<Vec<f32>>,
    pub success_count: Vec<usize>,
    pub fail_count: Vec<usize>,
}
impl GooseClient {
    /// Create a new client state.
    pub fn new(task_count: usize, index: usize, ) -> Self {
        trace!("new client");
        GooseClient {
            task_sets_index: index,
            client: Client::new(),
            weighted_clients_index: usize::max_value(),
            mode: GooseClientMode::INIT,
            weighted_tasks: Vec::new(),
            weighted_position: 0,
            requests: HashMap::new(),
            response_times: vec![vec![]; task_count],
            success_count: vec![0; task_count],
            fail_count: vec![0; task_count],
        }
    }

    pub fn set_mode(&mut self, mode: GooseClientMode) {
        self.mode = mode;
    }

    pub fn get_request(&mut self, url: &str, method: GooseRequestMethod) -> GooseRequest {
        let key = format!("{:?} {}", method, url);
        trace!("get key: {}", &key);
        match self.requests.get(&key) {
            // @TODO: is there a way to do this without clone()?
            Some(r) => r.clone(),
            None => GooseRequest::new(url, method),
        }
    }

    pub fn set_request(&mut self, url: &str, method: GooseRequestMethod, request: GooseRequest) {
        let key = format!("{:?} {}", method, url);
        trace!("set key: {}", &key);
        self.requests.insert(key, request);
    }

    pub fn get(&mut self, url: &str) -> Result<Response, Error> {
        let started = Instant::now();
        let response = self.client.get(url).send();
        let elapsed = started.elapsed() * 100;
        trace!("GET {} elapsed: {:?}", url, elapsed);

        let mut goose_request = self.get_request(url, GooseRequestMethod::GET);
        goose_request.set_response_time(elapsed.as_secs_f32());

        // data is collected per-task, vectors are indexed by the task_id
        let task_id = self.weighted_tasks[self.weighted_position];
        self.response_times[task_id].push(elapsed.as_secs_f32());
        match &response {
            Ok(r) => {
                let status_code = r.status();
                goose_request.set_status_code(status_code);

                debug!("{}: status_code {}", url, status_code);
                // @TODO: match/handle all is_foo() https://docs.rs/http/0.2.1/http/status/struct.StatusCode.html
                if status_code.is_success() {
                    goose_request.success_count += 1;
                    self.success_count[task_id] += 1;
                }
                // @TODO: properly track redirects and other code ranges
                else {
                    // @TODO: handle this correctly
                    debug!("{}: non-success status_code: {:?}", url, status_code);
                    goose_request.fail_count += 1;
                    self.fail_count[task_id] += 1;
                }
            }
            Err(e) => {
                // @TODO: what can we learn from a reqwest error?
                debug!("{}: error: {}", url, e);
                goose_request.fail_count += 1;
                self.fail_count[task_id] += 1;
            }
        };
        self.set_request(url, GooseRequestMethod::GET, goose_request);
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
    pub counter: Arc<AtomicUsize>,
    pub function: Option<fn(&mut GooseClient)>,
}
impl GooseTask {
    pub fn new(name: &str) -> Self {
        trace!("new task: name: {}", &name);
        let task = GooseTask {
            tasks_index: usize::max_value(),
            name: name.to_string(),
            weight: 1,
            counter: Arc::new(AtomicUsize::new(0)),
            function: None,
        };
        task
    }

    pub fn set_weight(mut self, weight: usize) -> Self {
        trace!("{} set_weight: {}", self.name, weight);
        if weight < 1 {
            info!("{} weight of {} not allowed, set to 1", self.name, weight);
            self.weight = 1;
        }
        else {
            self.weight = weight;
        }
        self
    }

    pub fn set_function(mut self, function: fn(&mut GooseClient)) -> Self {
        self.function = Some(function);
        self
    }
}
