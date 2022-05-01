//! Simple Goose load test example. Duplicates the simple example on the
//! Locust project page (<https://locust.io/>).
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
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        // In this example, we only create a single scenario, named "WebsiteUser".
        .register_scenario(
            scenario!("WebsiteUser")
                // After each transactions runs, sleep randomly from 5 to 15 seconds.
                .set_wait_time(Duration::from_secs(5), Duration::from_secs(15))?
                // This transaction only runs one time when the user first starts.
                .register_transaction(transaction!(website_login).set_on_start())
                // These next two transactions run repeatedly as long as the load test is running.
                .register_transaction(transaction!(website_index))
                .register_transaction(transaction!(website_about)),
        )
        .execute()
        .await?;

    Ok(())
}

/// Demonstrates how to log in when a user starts. We flag this transaction as an
/// on_start transaction when registering it above. This means it only runs one time
/// per user, when the user thread first starts.
async fn website_login(user: &mut GooseUser) -> TransactionResult {
    let params = [("username", "test_user"), ("password", "")];
    let _goose = user.post_form("/login", &params).await?;

    Ok(())
}

/// A very simple transaction that simply loads the front page.
async fn website_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;

    Ok(())
}

/// A very simple transaction that simply loads the about page.
async fn website_about(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/about/").await?;

    Ok(())
}
