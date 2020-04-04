use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Instant;

use reqwest::blocking::{Client, Response};
use reqwest::Error;

/// A global list of all Goose task sets
#[derive(Debug)]
pub struct GooseTaskSets {
    pub task_sets: Vec<GooseTaskSet>,
    pub weighted_task_sets: Vec<usize>,
}
impl GooseTaskSets {
    pub fn new() -> Self {
        let goose_tasksets = GooseTaskSets { 
            task_sets: Vec::new(),
            weighted_task_sets: Vec::new(),
        };
        goose_tasksets
    }

    pub fn register_taskset(&mut self, taskset: GooseTaskSet) {
        self.task_sets.push(taskset);
    }
}

/// An individual task set
#[derive(Debug, Clone)]
pub struct GooseTaskSet {
    pub name: String,
    pub weight: usize,
    pub tasks: Vec<GooseTask>,
    pub weighted_tasks: Vec<usize>,
    pub weighted_position: usize,
    pub counter: usize,
    pub state: GooseTaskSetState,
}
impl GooseTaskSet {
    pub fn new(name: &str) -> Self {
        trace!("new taskset: name: {}", &name);
        let task_set = GooseTaskSet { 
            name: name.to_string(),
            weight: 1,
            tasks: Vec::new(),
            weighted_tasks: Vec::new(),
            weighted_position: 0,
            counter: 0,
            state: GooseTaskSetState::new(),
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
pub struct GooseTaskSetState {
    pub client: Client,
    pub success_count: usize,
    pub fail_count: usize,
    pub response_times: Vec<f32>,
}
impl GooseTaskSetState {
    pub fn new() -> Self {
        trace!("new task state");
        let state = GooseTaskSetState {
            client: Client::new(),
            success_count: 0,
            fail_count: 0,
            response_times: Vec::new(),
        };
        state
    }
}

/// An individual task within a task set
#[derive(Debug, Clone)]
pub struct GooseTask {
    pub name: String,
    pub weight: usize,
    pub counter: Arc<AtomicUsize>,
    pub function: Option<fn(GooseTaskSetState) -> GooseTaskSetState>,
}
impl GooseTask {
    pub fn new(name: &str) -> Self {
        trace!("new task: name: {}", &name);
        let task = GooseTask {
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

    pub fn set_function(mut self, function: fn(GooseTaskSetState) -> GooseTaskSetState) -> Self {
        trace!("{} set_function: {:?}", self.name, function);
        self.function = Some(function);
        self
    }
}

pub fn url_get(task_state: &mut GooseTaskSetState, url: &str) -> Result<Response, Error> {
    let started = Instant::now();
    let response = task_state.client.get(url).send();
    let elapsed = started.elapsed() * 100;
    trace!("GET {} elapsed: {:?}", url, elapsed);
    task_state.response_times.push(elapsed.as_secs_f32());
    match &response {
        Ok(r) => {
            let status_code = r.status();
            debug!("{}: status_code {}", url, status_code);
            if status_code.is_success() {
                task_state.success_count += 1;
            }
            // @TODO: properly track redirects and other code ranges
            else {
                // @TODO: handle this correctly
                eprintln!("{}: non-success status_code: {:?}", url, status_code);
                task_state.fail_count += 1;
            }
        }
        Err(e) => {
            // @TODO: what can we learn from a reqwest error?
            debug!("{}: error: {}", url, e);
            task_state.fail_count += 1;
        }
    };
    response
}