/// @TODO:
///  - compile the goosefile as a dynamic binary included at run-time
///  - provide tools for goose to compile goosefiles
///  - ultimately load-tests are shipped with two compiled binaries:
///      o the main goose binary (pre-compiled)
///      o the goosefile dynamic binary (compiled with a goose helper)

// @TODO: this needs to be entirely provided by goose or goose_codegen

/// A global list of all Goose task sets
#[derive(Debug)]
pub struct GooseTaskSets {
    pub task_sets: Vec<GooseTaskSet>,
}
impl GooseTaskSets {
    pub fn new() -> Self {
        let goose_tasksets = GooseTaskSets { 
            task_sets: Vec::new(),
        };
        goose_tasksets
    }

    pub fn initialize_goosefile(&mut self) {
        // @TODO: metaprogramming to automate initialization

        // Register a website task set and contained tasks
        let mut website_tasks = GooseTaskSet::new("WebsiteTasks");
        website_tasks.register_task(GooseTask::new("on_start"));
        website_tasks.register_task(GooseTask::new("index"));
        website_tasks.register_task(GooseTask::new("about"));
        self.register_taskset(website_tasks);

        // Register an API task set and contained tasks
        let mut api_tasks = GooseTaskSet::new("APITasks");
        api_tasks.register_task(GooseTask::new("on_start"));
        api_tasks.register_task(GooseTask::new("listing"));
        self.register_taskset(api_tasks);
    }


    pub fn register_taskset(&mut self, taskset: GooseTaskSet) {
        self.task_sets.push(taskset);
    }
}

/// An individual task set
#[derive(Debug)]
pub struct GooseTaskSet {
    pub name: String,
    weight: u16,
    pub tasks: Vec<GooseTask>,
    //pub wait_time: (u16, 16),
    //host: String,
}
impl GooseTaskSet {
    pub fn new(name: &str) -> Self {
        let task_set = GooseTaskSet { 
            name: name.to_string(),
            weight: 0,
            tasks: Vec::new(),
        };
        task_set
    }

    pub fn register_task(&mut self, task: GooseTask) {
        self.tasks.push(task);
    }
}

/// An individual task within a task set
#[derive(Debug)]
pub struct GooseTask {
    pub name: String,
    pub weight: u16,
    //pub code: @TODO, closure?,
}
impl GooseTask {
    pub fn new(name: &str) -> Self {
        let task = GooseTask {
            name: name.to_string(),
            weight: 0,
        };
        task
    }

    //pub fn set_weight(&mut self, weight: u16) -> Self {
    //    self.weight = weight;
    //    self
    //}
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