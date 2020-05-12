//! Simple Goose load test example. Duplicates the simple example on the
//! Locust project page (https://locust.io/).
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


#[macro_use]
extern crate macro_rules_attribute;

use goose::dyn_async;
use goose::GooseState;
use goose::goose::{GooseTaskSet, GooseClient, GooseTask};

fn main() {
    GooseState::initialize()
        // In this example, we only create a single taskset, named "WebsiteUser".
        .register_taskset(GooseTaskSet::new("WebsiteUser")
            // After each task runs, sleep randomly from 5 to 15 seconds.
            .set_wait_time(5, 15)
            // This task only runs one time when the client first starts.
            .register_task(GooseTask::new(website_task_login).set_on_start())
            // These next two tasks run repeatedly as long as the load test is running.
            .register_task(GooseTask::new(website_task_index))
            .register_task(GooseTask::new(website_task_about))
        )
        .execute();
}

/// Demonstrates how to log in when a client starts. We flag this task as an
/// on_start task when registering it above. This means it only runs one time
/// per client, when the client thread first starts.
#[macro_rules_attribute(dyn_async!)]
async fn website_task_login<'fut>(client: &'fut mut GooseClient) -> () {
    let request_builder = client.goose_post("/login");
    // https://docs.rs/reqwest/*/reqwest/blocking/struct.RequestBuilder.html#method.form
    let params = [("username", "test_user"), ("password", "")];
    let _response = client.goose_send(request_builder.form(&params));
}

/// A very simple task that simply loads the front page.
#[macro_rules_attribute(dyn_async!)]
async fn website_task_index<'fut>(client: &'fut mut GooseClient) -> () {
    let _response = client.get("/");
}

/// A very simple task that simply loads the about page.
#[macro_rules_attribute(dyn_async!)]
async fn website_task_about<'fut>(client: &'fut mut GooseClient) -> () {
    let _response = client.get("/about/");
}
