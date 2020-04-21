//! Simple Goose load test example. Duplicates the simple example on Locust project page.
//! 
//! ## License
//! 
//! Copyright 2020 Jeremy Andrews
//! 
//! Licensed under the Apache License, Version 2.0 (the "License");
//! you may not use this file except in compliance with the License.
//! You may obtain a copy of the License at
//! 
//! http://www.apache.org/licenses/LICENSE-2.0
//! 
//! Unless required by applicable law or agreed to in writing, software
//! distributed under the License is distributed on an "AS IS" BASIS,
//! WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//! See the License for the specific language governing permissions and
//! limitations under the License.

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
    //client.set_response_err("error text here");
    //client.set_response_ok();
}

fn website_task_about(client: &mut GooseClient) {
    let _response = client.get("/about/");
}
