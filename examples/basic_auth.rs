//! Goose load test example demonstrating different approaches to HTTP Basic Authentication.
//!
//! This example shows multiple ways to handle Basic Auth in Goose load tests:
//! 1. Per-request basic auth using request builders
//! 2. Using a helper function for consistent auth across requests
//! 3. Setting up a custom client with default Authorization headers (recommended)
//!
//! ## Usage
//!
//! Set environment variables for authentication:
//! ```bash
//! export BASIC_AUTH_USERNAME="your_username"
//! export BASIC_AUTH_PASSWORD="your_password"
//! ```
//!
//! Or use the combined format:
//! ```bash
//! export BASIC_AUTH="username:password"
//! ```
//!
//! Run the example:
//! ```bash
//! cargo run --example basic_auth -- --host http://httpbin.org
//! ```
//!
//! ## License
//!
//! Copyright 2020-2025 Jeremy Andrews
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

#[cfg(not(feature = "cookies"))]
fn main() {
    eprintln!("ERROR: The basic_auth example requires the 'cookies' feature to be enabled.");
    eprintln!("This example demonstrates session management with Basic Authentication,");
    eprintln!("which requires cookie support to maintain authentication state across requests.");
    eprintln!();
    eprintln!("To run this example, use:");
    eprintln!("  cargo run --example basic_auth --features cookies");
    eprintln!("or:");
    eprintln!("  cargo run --example basic_auth  # (cookies enabled by default)");
    std::process::exit(1);
}

#[cfg(feature = "cookies")]
mod basic_auth_example {
    use goose::goose::GooseResponse;
    use goose::prelude::*;
    use reqwest::{header, Client};
    use std::{env, time::Duration};

    /// Simple base64 encoding function to avoid external dependencies
    ///
    /// Note: In production code, you could use the `base64` crate for this functionality:
    /// ```toml
    /// [dependencies]
    /// base64 = "0.21"
    /// ```
    /// We implement it manually here to keep the example self-contained without
    /// adding extra dependencies just for demonstration purposes.
    fn base64_encode(input: &[u8]) -> String {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut result = String::new();

        for chunk in input.chunks(3) {
            let mut buf = [0u8; 3];
            for (i, &byte) in chunk.iter().enumerate() {
                buf[i] = byte;
            }

            let b = ((buf[0] as u32) << 16) | ((buf[1] as u32) << 8) | (buf[2] as u32);

            result.push(CHARS[((b >> 18) & 63) as usize] as char);
            result.push(CHARS[((b >> 12) & 63) as usize] as char);
            result.push(if chunk.len() > 1 {
                CHARS[((b >> 6) & 63) as usize] as char
            } else {
                '='
            });
            result.push(if chunk.len() > 2 {
                CHARS[(b & 63) as usize] as char
            } else {
                '='
            });
        }

        result
    }

    #[tokio::main]
    pub async fn main() -> Result<(), GooseError> {
        GooseAttack::initialize()?
            // Scenario 1: Using custom client with default headers (recommended approach)
            .register_scenario(
                scenario!("CustomClientAuth")
                    .set_weight(3)?
                    .set_wait_time(Duration::from_secs(1), Duration::from_secs(3))?
                    // Set up custom client with Basic Auth headers on user start
                    .register_transaction(transaction!(setup_basic_auth_client).set_on_start())
                    // All subsequent requests will automatically include Basic Auth
                    .register_transaction(transaction!(test_basic_auth_endpoint))
                    .register_transaction(transaction!(test_user_agent_endpoint))
                    .register_transaction(transaction!(test_json_endpoint)),
            )
            // Scenario 2: Using helper function for per-request auth
            .register_scenario(
                scenario!("HelperFunctionAuth")
                    .set_weight(2)?
                    .set_wait_time(Duration::from_secs(1), Duration::from_secs(3))?
                    .register_transaction(transaction!(test_with_helper_function))
                    .register_transaction(transaction!(test_post_with_helper)),
            )
            // Scenario 3: Manual per-request auth (demonstrates the original problem)
            .register_scenario(
                scenario!("ManualPerRequestAuth")
                    .set_weight(1)?
                    .set_wait_time(Duration::from_secs(1), Duration::from_secs(3))?
                    .register_transaction(transaction!(test_manual_basic_auth))
                    .register_transaction(transaction!(test_manual_with_validation)),
            )
            .execute()
            .await?;

        Ok(())
    }

    /// Approach 1: Set up a custom client with Basic Auth headers (RECOMMENDED)
    ///
    /// This approach sets up the HTTP client once per user with default Basic Auth headers.
    /// All subsequent requests will automatically include the authentication headers,
    /// including requests for static assets (CSS, JS, images) when using
    /// `validate_and_load_static_assets`.
    async fn setup_basic_auth_client(user: &mut GooseUser) -> TransactionResult {
        // Get credentials from environment variables
        let (username, password) = get_basic_auth_credentials()?;

        // Generate the Basic Auth header directly using base64 encoding
        let mut headers = header::HeaderMap::new();
        let credentials = format!("{}:{}", username, password);
        let encoded = base64_encode(credentials.as_bytes());
        let auth_header_value = format!("Basic {}", encoded);

        headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&auth_header_value)
                .map_err(|e| TransactionError::Custom(e.to_string()))?,
        );

        // Set up the client builder with default headers and other common settings
        // Enable cookie store for maintaining session state with Basic Auth
        let builder = Client::builder()
            .user_agent("Goose/1.0 BasicAuth Example")
            .default_headers(headers)
            .timeout(Duration::from_secs(30))
            .cookie_store(true);

        // Apply the custom client to this user
        user.set_client_builder(builder).await?;

        Ok(())
    }

    /// Test endpoint that requires Basic Auth - using custom client approach
    async fn test_basic_auth_endpoint(user: &mut GooseUser) -> TransactionResult {
        // Since we set up the client with default headers, this request will
        // automatically include the Basic Auth header
        let goose_metrics = user.get("/basic-auth/user/passwd").await?;

        // Since we're using httpbin.org for testing, we can validate the response
        if let Ok(response) = &goose_metrics.response {
            if response.status() == 200 {
                // Success! The Basic Auth worked
                println!("Basic Auth successful!");
            }
        }

        Ok(())
    }

    /// Test user-agent endpoint to show other headers work too
    async fn test_user_agent_endpoint(user: &mut GooseUser) -> TransactionResult {
        let _goose_metrics = user.get("/user-agent").await?;
        Ok(())
    }

    /// Test JSON endpoint with Basic Auth
    async fn test_json_endpoint(user: &mut GooseUser) -> TransactionResult {
        let _goose_metrics = user.get("/json").await?;
        Ok(())
    }

    /// Approach 2: Helper function for consistent Basic Auth across requests
    ///
    /// This approach uses a helper function that adds Basic Auth to each request.
    /// It's more flexible than the custom client approach but requires calling
    /// the helper for every request.
    async fn send_authenticated_request(
        user: &mut GooseUser,
        method: &GooseMethod,
        path: &str,
    ) -> Result<GooseResponse, Box<TransactionError>> {
        let mut reqwest_request_builder = user.get_request_builder(method, path)?;

        // Check for Basic Auth credentials in environment
        if let Ok((username, password)) = get_basic_auth_credentials() {
            reqwest_request_builder = reqwest_request_builder.basic_auth(username, Some(password));
        }

        // Build the GooseRequest and make the request
        let goose_request = GooseRequest::builder()
            .set_request_builder(reqwest_request_builder)
            .build();

        let goose_response = user.request(goose_request).await?;
        Ok(goose_response)
    }

    /// Test using the helper function approach
    async fn test_with_helper_function(user: &mut GooseUser) -> TransactionResult {
        let _goose_response =
            send_authenticated_request(user, &GooseMethod::Get, "/basic-auth/user/passwd").await?;
        Ok(())
    }

    /// Test POST request using helper function
    async fn test_post_with_helper(user: &mut GooseUser) -> TransactionResult {
        let _goose_response = send_authenticated_request(user, &GooseMethod::Post, "/post").await?;
        Ok(())
    }

    /// Approach 3: Manual per-request Basic Auth (demonstrates the original problem)
    ///
    /// This approach manually adds Basic Auth to each individual request.
    /// Note: This does NOT propagate to static asset requests, which is the
    /// original problem described in issue #608.
    async fn test_manual_basic_auth(user: &mut GooseUser) -> TransactionResult {
        let (username, password) = get_basic_auth_credentials()?;

        let reqwest_request_builder = user
            .get_request_builder(&GooseMethod::Get, "/basic-auth/user/passwd")?
            .basic_auth(username, Some(password));

        let goose_request = GooseRequest::builder()
            .set_request_builder(reqwest_request_builder)
            .build();

        let _goose_metrics = user.request(goose_request).await?;

        Ok(())
    }

    /// Manual approach with validation (shows the static asset problem)
    async fn test_manual_with_validation(user: &mut GooseUser) -> TransactionResult {
        let (username, password) = get_basic_auth_credentials()?;

        let reqwest_request_builder = user
            .get_request_builder(&GooseMethod::Get, "/basic-auth/user/passwd")?
            .basic_auth(username, Some(password));

        let goose_request = GooseRequest::builder()
            .set_request_builder(reqwest_request_builder)
            .build();

        let goose_metrics = user.request(goose_request).await?;

        // WARNING: If this page had static assets (CSS, JS, images), they would
        // fail to load because they won't have the Basic Auth header!
        // This is the problem that the custom client approach solves.
        if let Ok(response) = &goose_metrics.response {
            if response.status() == 200 {
                println!("Manual Basic Auth successful, but static assets would fail!");
            }
        }

        Ok(())
    }

    /// Helper function to get Basic Auth credentials from environment variables
    ///
    /// Supports two formats:
    /// 1. BASIC_AUTH_USERNAME and BASIC_AUTH_PASSWORD (separate variables)
    /// 2. BASIC_AUTH in "username:password" format (combined variable)
    fn get_basic_auth_credentials() -> Result<(String, String), Box<TransactionError>> {
        // Try the separate username/password format first
        if let (Ok(username), Ok(password)) = (
            env::var("BASIC_AUTH_USERNAME"),
            env::var("BASIC_AUTH_PASSWORD"),
        ) {
            return Ok((username, password));
        }

        // Try the combined format
        if let Ok(basic_auth) = env::var("BASIC_AUTH") {
            let parts: Vec<&str> = basic_auth.split(':').collect();
            if parts.len() == 2 {
                return Ok((parts[0].to_string(), parts[1].to_string()));
            } else {
                return Err(Box::new(TransactionError::Custom(format!(
                    "BASIC_AUTH must be in format 'username:password', got: {}",
                    basic_auth
                ))));
            }
        }

        // Default credentials for testing with httpbin.org
        eprintln!("Warning: No Basic Auth credentials found in environment variables.");
        eprintln!("Using default credentials 'user:passwd' for httpbin.org testing.");
        eprintln!("Set BASIC_AUTH_USERNAME/BASIC_AUTH_PASSWORD or BASIC_AUTH environment variables for real usage.");

        Ok(("user".to_string(), "passwd".to_string()))
    }
}

#[cfg(feature = "cookies")]
use basic_auth_example::main;
