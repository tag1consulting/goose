use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Instant;

use reqwest::blocking::{Client, Response};
use reqwest::Error;

/// A global list of all Goose task sets
#[derive(Clone)]
pub struct GooseTaskSets {
    pub task_sets: Vec<GooseTaskSet>,
    pub weighted_states: Vec<GooseClient>,
    pub weighted_states_order: Vec<usize>,
}
impl GooseTaskSets {
    pub fn new() -> Self {
        let goose_tasksets = GooseTaskSets { 
            task_sets: Vec::new(),
            weighted_states: Vec::new(),
            weighted_states_order: Vec::new(),
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
pub enum GooseClientCommand {
    //START,
    //STOP,
    // Tell client thread to push statistics to parent
    SYNC,
    // Tell client thread to exit
    EXIT,
}

#[derive(Debug, Clone)]
pub struct GooseClient {
    // This is the GooseTaskSets.task_sets index
    pub task_sets_index: usize,
    // This is the reqwest.blocking.client
    pub client: Client,
    pub weighted_states_index: usize,
    pub mode: GooseClientMode,
    // Per-task statistics, using task index
    pub response_times: Vec<Vec<f32>>,
    pub success_count: Vec<usize>,
    pub fail_count: Vec<usize>,
    pub weighted_tasks: Vec<usize>,
    pub weighted_position: usize,
}
impl GooseClient {
    pub fn new(task_count: usize, index: usize, ) -> Self {
        trace!("new task state");
        let state = GooseClient {
            task_sets_index: index,
            client: Client::new(),
            weighted_states_index: usize::max_value(),
            mode: GooseClientMode::INIT,
            response_times: vec![vec![]; task_count],
            success_count: vec![0; task_count],
            fail_count: vec![0; task_count],
            weighted_tasks: Vec::new(),
            weighted_position: 0,
        };
        state
    }

    pub fn set_mode(&mut self, mode: GooseClientMode) {
        self.mode = mode;
    }

    pub fn get(&mut self, url: &str) -> Result<Response, Error> {
        let started = Instant::now();
        let response = self.client.get(url).send();
        let elapsed = started.elapsed() * 100;
        trace!("GET {} elapsed: {:?}", url, elapsed);

        // data is collected per-task, vectors are indexed by the task_id
        let task_id = self.weighted_tasks[self.weighted_position];

        self.response_times[task_id].push(elapsed.as_secs_f32());
        match &response {
            Ok(r) => {
                let status_code = r.status();
                debug!("{}: status_code {}", url, status_code);
                if status_code.is_success() {
                    self.success_count[task_id] += 1;
                }
                // @TODO: properly track redirects and other code ranges
                else {
                    // @TODO: handle this correctly
                    eprintln!("{}: non-success status_code: {:?}", url, status_code);
                    self.fail_count[task_id] += 1;
                }
            }
            Err(e) => {
                // @TODO: what can we learn from a reqwest error?
                debug!("{}: error: {}", url, e);
                self.fail_count[task_id] += 1;
            }
        };
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
