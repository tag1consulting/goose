//! Goose load test example with a client of the load test's own. An on start
//! transaction builds a `reqwest::ClientBuilder` and passes it to
//! `GooseUser::set_client_builder`, and Goose builds that user's client from
//! it as it is: Goose's client defaults, its shared certificate verifier and
//! `--http1-only` do not apply to it.
//!
//! ## License
//!
//! Copyright 2026 Jeremy Andrews
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
        .register_scenario(
            scenario!("WebsiteUser")
                .register_transaction(transaction!(setup_client).set_on_start())
                .register_transaction(transaction!(load_homepage)),
        )
        .execute()
        .await?;

    Ok(())
}

async fn setup_client(user: &mut GooseUser) -> TransactionResult {
    // Goose's defaults don't apply to this client: set the ones you need.
    let builder = reqwest::Client::builder()
        .user_agent("my-loadtest/1.0")
        .timeout(Duration::from_secs(60))
        .gzip(true)
        .cookie_store(true);
    user.set_client_builder(builder).await?;
    Ok(())
}

async fn load_homepage(user: &mut GooseUser) -> TransactionResult {
    let _ = user.get("/").await?.discard_body().await;
    Ok(())
}
