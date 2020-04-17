//! Simple Goose load test example. Duplicates the simple example on Locust project page.

use goose::{goose_init, goose_launch};
use goose::goose::{GooseTaskSets, GooseTaskSet, GooseClient, GooseTask};

fn main() {
    let goose_state = goose_init();
    let mut goose_task_sets = GooseTaskSets::new();

    // Create and configure a task set.
    let mut websiteuser_tasks = GooseTaskSet::new("WebsiteUser")
        // Optional, a random sleep value selected from low to high, randomly invoked after each task is run
        .set_wait_time(5, 15);
    
    websiteuser_tasks.register_task(GooseTask::new().set_function(website_task_login));
    websiteuser_tasks.register_task(GooseTask::new().set_function(website_task_index));
    websiteuser_tasks.register_task(GooseTask::new().set_function(website_task_about));
    goose_task_sets.register_taskset(websiteuser_tasks);

    goose_launch(goose_state, goose_task_sets);
}

fn website_task_login(client: &mut GooseClient) {
    let request = client.goose_post("/login");
    let params = [("username", "test_user"), ("password", "")];
    let _response = client.goose_send(request.form(&params));
}

fn website_task_index(client: &mut GooseClient) {
    let _response = client.get("/");
}

fn website_task_about(client: &mut GooseClient) {
    let _response = client.get("/about/");
}
