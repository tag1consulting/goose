/// @TODO:
///  - compile the goosefile as a dynamic binary included at run-time
///  - provide tools for goose to compile goosefiles
///  - ultimately load-tests are shipped with two compiled binaries:
///      o the main goose binary (pre-compiled)
///      o the goosefile dynamic binary (compiled with a goose helper)

use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

// @TODO: this needs to be entirely provided by goose or goose_codegen

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

    pub fn initialize_goosefile(&mut self) {
        trace!("initialize_goosefile");
        // @TODO: metaprogramming to automate initialization

        // Register a website task set and contained tasks
        let mut website_tasks = GooseTaskSet::new("WebsiteTasks").set_weight(10);
        //website_tasks.register_task(GooseTask::new("on_start"));
        website_tasks.register_task(GooseTask::new("index").set_weight(6).set_function(website_task_index));
        website_tasks.register_task(GooseTask::new("story").set_weight(9).set_function(website_task_story));
        website_tasks.register_task(GooseTask::new("about").set_weight(3).set_function(website_task_about));
        self.register_taskset(website_tasks);

        // Register an API task set and contained tasks
        let mut api_tasks = GooseTaskSet::new("APITasks").set_weight(3);
        //api_tasks.register_task(GooseTask::new("on_start"));
        api_tasks.register_task(GooseTask::new("listing1").set_weight(3));
        api_tasks.register_task(GooseTask::new("listing2").set_weight(3));
        api_tasks.register_task(GooseTask::new("listing3").set_weight(0));
        self.register_taskset(api_tasks);

        let empty_tasks = GooseTaskSet::new("EmptyTasks").set_weight(1);
        self.register_taskset(empty_tasks);
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
    //pub wait_time: (u16, 16),
    //host: String,
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
    // @TODO: pass in a struct containing Client and other state
    pub function: Option<fn(reqwest::Client) -> reqwest::Client>,
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

    pub fn set_function(mut self, function: fn(reqwest::Client) -> reqwest::Client) -> Self {
        trace!("{} set_function: {:?}", self.name, function);
        self.function = Some(function);
        self
    }
}

fn website_task_index(client: reqwest::Client) -> reqwest::Client {
    let resp = client.get("http://localhost/");
    debug!("index: {:#?}", resp);
    client
}

fn website_task_story(client: reqwest::Client) -> reqwest::Client {
    let resp = client.get("http://localhost/story");
    debug!("story: {:#?}", resp);
    client
}

fn website_task_about(client: reqwest::Client) -> reqwest::Client {
    let resp = client.get("http://localhost/about");
    debug!("about: {:#?}", resp);
    client
}

/*
impl WebsiteTasks {
    #[task]
    fn on_start(&self) -> Result<(), Box<dyn std::error::Error>> {
        let params = [("username", "test_user"), ("password", "secure_example")];
        let client = reqwest::Client::new();
        let res = client.post("/login")
            .form(&params)
            .send()?;
        Ok(())
    }

    #[task]
    fn index(&self) -> Result<(), Box<dyn std::error::Error>> {
        let resp = reqwest::blocking::get("/");
        println!("{:#?}", resp);
        Ok(())
    }

    #[task]
    fn about(&self) {
        let resp = reqwest::blocking::get("/about/");
        println!("{:#?}", resp);
        Ok(())
    }
}
*/

/*
class WebsiteUser(HttpLocust):
    task_set = WebsiteTasks
    wait_time = between(5, 15)
*/