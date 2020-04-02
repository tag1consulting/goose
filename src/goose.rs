use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use reqwest::blocking::Client;

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

/// An individual task within a task set
#[derive(Debug, Clone)]
pub struct GooseTask {
    pub name: String,
    pub weight: usize,
    pub counter: Arc<AtomicUsize>,
    pub function: Option<fn(GooseTaskState) -> GooseTaskState>,
    pub state: GooseTaskState,
}
impl GooseTask {
    pub fn new(name: &str) -> Self {
        trace!("new task: name: {}", &name);
        let task = GooseTask {
            name: name.to_string(),
            weight: 1,
            counter: Arc::new(AtomicUsize::new(0)),
            function: None,
            state: GooseTaskState::new(),
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

    pub fn set_function(mut self, function: fn(GooseTaskState) -> GooseTaskState) -> Self {
        trace!("{} set_function: {:?}", self.name, function);
        self.function = Some(function);
        self
    }
}

#[derive(Debug, Clone)]
pub struct GooseTaskState {
    pub client: Client,
}
impl GooseTaskState {
    pub fn new() -> Self {
        trace!("new task state");
        let state = GooseTaskState {
            client: Client::new(),
        };
        state
    }
}
