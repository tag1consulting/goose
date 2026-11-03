use httpmock::{
    Method::{GET, POST},
    Mock, MockServer,
};

mod common;

use goose::config::GooseConfiguration;
use goose::prelude::*;

// Paths used in load tests performed during these tests.
const LOGIN_PATH: &str = "/login";
const LOGOUT_PATH: &str = "/logout";

// Indexes to the above paths.
const LOGIN_KEY: usize = 0;
const LOGOUT_KEY: usize = 1;

// Load test configuration.
const USERS: usize = 5;
const RUN_TIME: usize = 2;

// Test transaction.
pub async fn login(user: &mut GooseUser) -> TransactionResult {
    let params = [("username", "me"), ("password", "s3crET!")];
    let _goose = user.post_form(LOGIN_PATH, &params).await?;
    Ok(())
}

// Test transaction.
pub async fn logout(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(LOGOUT_PATH).await?;
    Ok(())
}

// All tests in this file run against common endpoints.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<Mock<'_>> {
    vec![
        // First set up LOGIN_PATH, store in vector at LOGIN_KEY.
        server.mock(|when, then| {
            when.method(POST).path(LOGIN_PATH);
            then.status(200);
        }),
        // Next set up LOGOUT_PATH, store in vector at LOGOUT_KEY.
        server.mock(|when, then| {
            when.method(GET).path(LOGOUT_PATH);
            then.status(200);
        }),
    ]
}

// Build appropriate configuration for these tests.
fn common_build_configuration(server: &MockServer) -> GooseConfiguration {
    common::build_configuration(
        server,
        vec![
            "--users",
            &USERS.to_string(),
            "--increase-rate",
            &USERS.to_string(),
            "--run-time",
            &RUN_TIME.to_string(),
            "--no-reset-metrics",
        ],
    )
}

// Helper to confirm all variations generate appropriate results.
fn validate_test(mock_endpoints: &[Mock]) {
    // Confirm that the on_start and on_exit transactions actually ran once per GooseUser.
    mock_endpoints[LOGIN_KEY].assert_calls(USERS);
    mock_endpoints[LOGOUT_KEY].assert_calls(USERS);
}

// Returns the appropriate scenario needed to build these tests.
fn get_transactions() -> Scenario {
    scenario!("LoadTest")
        .register_transaction(transaction!(login).set_on_start())
        .register_transaction(transaction!(logout).set_on_stop())
}

// Helper to run the test.
async fn run_load_test() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let configuration = common_build_configuration(&server);

    // Run the Goose Attack.
    common::run_load_test(common::build_load_test(
        configuration,
        vec![get_transactions()],
        None,
        None,
    ))
    .await;

    // Confirm the load test ran correctly.
    validate_test(&mock_endpoints);
}

#[tokio::test]
// Test scenario with only on_start() and on_stop() transactions.
async fn test_no_normal_transactions() {
    run_load_test().await;
}
