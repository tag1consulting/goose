//! Goose load test example, leveraging the per-GooseUser `GooseUserData` field
// to store a per-user session JWT authentication token.
//!
//! ## License
//!
//! Copyright 2020-2022 Jeremy Andrews
//!
//! Licensed under the Apache License, Version 2.0 (the "License");
//! you may not use this file except in compliance with the License.
//! You may obtain a copy of the License at
//!
//! <http://www.apache.org/licenses/LICENSE-2.0>
//!
//! Unless required by applicable law or agreed to in writing, software
//! distributed under the License is distributed on an "AS IS" BASIS,
//! WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//! See the License for the specific language governing permissions and
//! limitations under the License.

use goose::prelude::*;
use serde::Deserialize;
use std::time::Duration;

struct Session {
    jwt_token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthenticationResponse {
    jwt_token: String,
}

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        // In this example, we only create a single scenario, named "WebsiteUser".
        .register_scenario(
            scenario!("WebsiteUser")
                // After each transaction runs, sleep randomly from 5 to 15 seconds.
                .set_wait_time(Duration::from_secs(5), Duration::from_secs(15))?
                // This transaction only runs one time when the user first starts.
                .register_transaction(transaction!(website_signup).set_on_start())
                // These next two transactions run repeatedly as long as the load test is running.
                .register_transaction(transaction!(authenticated_index)),
        )
        .execute()
        .await?;

    Ok(())
}

/// Demonstrates how to log in and set a session when a user starts. We flag this transaction as an
/// on_start transaction when registering it above. This means it only runs one time
/// per user, when the user thread first starts.
async fn website_signup(user: &mut GooseUser) -> TransactionResult {
    let params = [("username", "test_user"), ("password", "")];
    let response = match user.post_form("/signup", &params).await?.response {
        Ok(r) => match r.json::<AuthenticationResponse>().await {
            Ok(j) => j,
            Err(e) => return Err(Box::new(e.into())),
        },
        Err(e) => return Err(Box::new(e.into())),
    };

    user.set_session_data(Session {
        jwt_token: response.jwt_token,
    });

    Ok(())
}

/// A very simple transaction that simply loads the front page.
async fn authenticated_index(user: &mut GooseUser) -> TransactionResult {
    // This will panic if the session is missing or if the session is not of the right type.
    // Use `get_session_data` to handle a missing session.
    let session = user.get_session_data_unchecked::<Session>();

    // Create a Reqwest RequestBuilder object and configure bearer authentication when making
    // a GET request for the index.
    let reqwest_request_builder = user
        .get_request_builder(&GooseMethod::Get, "/")?
        .bearer_auth(&session.jwt_token);

    // Add the manually created RequestBuilder and build a GooseRequest object.
    let goose_request = GooseRequest::builder()
        .set_request_builder(reqwest_request_builder)
        .build();

    // Make the actual request.
    user.request(goose_request).await?;

    Ok(())
}
