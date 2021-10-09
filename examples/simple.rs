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

use goose::prelude::*;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        // In this example, we only create a single taskset, named "WebsiteUser".
        .register_taskset(
            taskset!("WebsiteUser")
                // After each task runs, sleep randomly from 5 to 15 seconds.
                .set_wait_time(5, 15)?
                // This task only runs one time when the user first starts.
                .register_task(task!(website_login).set_on_start())
                // These next two tasks run repeatedly as long as the load test is running.
                .register_task(task!(website_index))
                .register_task(task!(website_about)),
        )
        .execute()
        .await?
        .print();

    Ok(())
}

/// Demonstrates how to log in when a user starts. We flag this task as an
/// on_start task when registering it above. This means it only runs one time
/// per user, when the user thread first starts.
async fn website_login(user: &mut GooseUser) -> GooseTaskResult {
    let request_builder = user.goose_post("/login")?;
    // https://docs.rs/reqwest/*/reqwest/blocking/struct.RequestBuilder.html#method.form
    let params = [("username", "test_user"), ("password", "")];
    let _goose = user.goose_send(request_builder.form(&params), None).await?;

    Ok(())
}

/// A very simple task that simply loads the front page.
async fn website_index(user: &mut GooseUser) -> GooseTaskResult {
    let _goose = user.get("/").await?;

    Ok(())
}

/// A very simple task that simply loads the about page.
async fn website_about(user: &mut GooseUser) -> GooseTaskResult {
    let _goose = user.get("/about/").await?;

    Ok(())
}
