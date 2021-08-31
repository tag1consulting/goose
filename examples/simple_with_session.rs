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
use serde::Deserialize;

struct Session {
    jwt_token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthenticationResponse {
    jwt_token: String,
}

fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        // In this example, we only create a single taskset, named "WebsiteUser".
        .register_taskset(
            taskset!("WebsiteUser")
                // After each task runs, sleep randomly from 5 to 15 seconds.
                .set_wait_time(5, 15)?
                // This task only runs one time when the user first starts.
                .register_task(task!(website_signup).set_on_start())
                // These next two tasks run repeatedly as long as the load test is running.
                .register_task(task!(authenticated_index)),
        )
        .execute()?
        .print();

    Ok(())
}

/// Demonstrates how to log in and set a session when a user starts. We flag this task as an
/// on_start task when registering it above. This means it only runs one time
/// per user, when the user thread first starts.
async fn website_signup(user: &mut GooseUser) -> GooseTaskResult {
    let request_builder = user.goose_post("/signup")?;
    // https://docs.rs/reqwest/*/reqwest/blocking/struct.RequestBuilder.html#method.form
    let params = [("username", "test_user"), ("password", "")];
    let response = user
        .goose_send(request_builder.form(&params), None)
        .await?
        .response?
        .json::<AuthenticationResponse>()
        .await?;

    user.set_session_data(Session {
        jwt_token: response.jwt_token,
    });

    Ok(())
}

/// A very simple task that simply loads the front page.
async fn authenticated_index(user: &mut GooseUser) -> GooseTaskResult {
    // This will panic if the session is missing or if the session is not of the right type
    // use `get_session_data` to handle missing session 
    let session = user.get_session_data_uncheck::<Session>();
    let request = user.goose_get("/")?.bearer_auth(&session.jwt_token);
    let _goose = user.goose_send(request, None).await?;

    Ok(())
}
