/// @TODO:
///  - compile the goosefile as a dynamic binary included at run-time
///  - provide tools for goose to compile goosefiles
///  - ultimately load-tests are shipped with two compiled binaries:
///      o the main goose binary (pre-compiled)
///      o the goosefile dynamic binary (compiled with a goose helper)

use crate::goose::{GooseTaskSets, GooseTaskSet, GooseTask};

// @TODO: this needs to be entirely provided by goose or goose_codegen

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

fn website_task_index(client: reqwest::blocking::Client) -> reqwest::blocking::Client {
    match client.get("http://localhost/").send() {
        Ok(r) => {
            let content_length = match r.content_length() {
                Some(l) => l,
                None => 0,
            };
            debug!("index: content_length: {}", content_length);
        }
        Err(e) => {
            debug!("index: error: {}", e);
        }
    };
    client
}

fn website_task_story(client: reqwest::blocking::Client) -> reqwest::blocking::Client {
    match client.get("http://localhost/story").send() {
        Ok(r) => {
            let content_length = match r.content_length() {
                Some(l) => l,
                None => 0,
            };
            debug!("story: content_length: {}", content_length);
        }
        Err(e) => {
            debug!("story: error: {}", e);

        }
    };
    client
}

fn website_task_about(client: reqwest::blocking::Client) -> reqwest::blocking::Client {
    match client.get("http://localhost/about").send() {
        Ok(r) => {
            let content_length = match r.content_length() {
                Some(l) => l,
                None => 0,
            };
            debug!("about: content_length: {}", content_length);
        }
        Err(e) => {
            debug!("about: error: {}", e);
        }
    };
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