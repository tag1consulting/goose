//! Simple Goose load test example. Duplicates the simple example on Locust project page.

use goose::GooseState;
use goose::goose::{GooseTaskSet, GooseClient, GooseTask};

fn main() {
    let mut goose_state = GooseState::initialize();

    // Create and configure a task set.
    let mut websiteuser_tasks = GooseTaskSet::new("WebsiteUser")
        // Optional, a random sleep value selected from low to high, randomly invoked after each task is run
        .set_wait_time(5, 15);
    
    websiteuser_tasks.register_task(GooseTask::new(website_task_login).set_on_start());
    websiteuser_tasks.register_task(GooseTask::new(website_task_index));
    websiteuser_tasks.register_task(GooseTask::new(website_task_about));
    goose_state.register_taskset(websiteuser_tasks);

    goose_state.execute();
}

fn website_task_login(client: &mut GooseClient) {
    let params = [("username", "test_user"), ("password", "")];
    let request_builder = client.goose_post("/login");
    let _response = client.goose_send(request_builder.form(&params));
}

fn website_task_index(client: &mut GooseClient) {
    let _response = client.get("/");
}

fn website_task_about(client: &mut GooseClient) {
    let _response = client.get("/about/");
}
