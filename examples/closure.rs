//! Simple Goose load test example using closures.
//!
//! ## License
//!
//! Copyright 2020 Fabian Franz
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
use std::boxed::Box;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    let mut scenario = scenario!("WebsiteUser")
        // After each transaction runs, sleep randomly from 5 to 15 seconds.
        .set_wait_time(Duration::from_secs(5), Duration::from_secs(15))?;

    let paths = vec!["/", "/about", "/our-team"];
    for request_path in paths {
        let path = request_path;

        let closure: TransactionFunction = Arc::new(move |user| {
            Box::pin(async move {
                let _goose = user.get(path).await?;

                Ok(())
            })
        });

        let transaction = Transaction::new(closure);
        // We need to do the variable dance as scenario.register_transaction returns self and hence moves
        // self out of `scenario`. By storing it in a new local variable and then moving it over
        // we can avoid that error.
        let new_scenario = scenario.register_transaction(transaction);
        scenario = new_scenario;
    }

    GooseAttack::initialize()?
        // In this example, we only create a single scenario, named "WebsiteUser".
        .register_scenario(scenario)
        .execute()
        .await?;

    Ok(())
}
