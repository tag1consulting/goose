use httpmock::{
    Method::{GET, POST},
    Mock, MockServer,
};

mod common;

use goose::config::GooseConfiguration;
use goose::prelude::*;

// Paths used in load tests performed during these tests.
const INDEX_PATH: &str = "/";
const SETUP_PATH: &str = "/setup";
const TEARDOWN_PATH: &str = "/teardown";

// Indexes to the above paths.
const INDEX_KEY: usize = 0;
const SETUP_KEY: usize = 1;
const TEARDOWN_KEY: usize = 2;

// Load test configuration.
const USERS: &str = "4";

// There are multiple test variations in this file.
#[derive(Clone)]
enum TestType {
    // Test on_start alone.
    Start,
    // Test on_stop alone.
    Stop,
    // Test on_start and on_stop both together.
    StartAndStop,
}

// Test transaction.
pub async fn setup(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.post(SETUP_PATH, "setting up load test").await?;
    Ok(())
}

// Test transaction.
pub async fn teardown(user: &mut GooseUser) -> TransactionResult {
    let _goose = user
        .post(TEARDOWN_PATH, "cleaning up after load test")
        .await?;
    Ok(())
}

// Test transaction.
pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// All tests in this file run against common endpoints.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<Mock<'_>> {
    vec![
        // First set up INDEX_PATH, store in vector at INDEX_KEY.
        server.mock(|when, then| {
            when.method(GET).path(INDEX_PATH);
            then.status(201);
        }),
        // Next set up SETUP_PATH, store in vector at SETUP_KEY.
        server.mock(|when, then| {
            when.method(POST).path(SETUP_PATH);
            then.status(205);
        }),
        // Next set up TEARDOWN_PATH, store in vector at TEARDOWN_KEY.
        server.mock(|when, then| {
            when.method(POST).path(TEARDOWN_PATH);
            then.status(200);
        }),
    ]
}

// Build appropriate configuration for these tests.
fn common_build_configuration(server: &MockServer) -> GooseConfiguration {
    common::build_configuration(server, vec!["--users", USERS, "--increase-rate", USERS])
}

// Helper to confirm all variations generate appropriate results.
fn validate_test(test_type: &TestType, mock_endpoints: &[Mock]) {
    // Confirm the load test ran.
    assert!(mock_endpoints[INDEX_KEY].calls() > 0);

    // Now confirm TestType-specific counters.
    match test_type {
        TestType::Start => {
            // Confirm setup ran one time.
            mock_endpoints[SETUP_KEY].assert_calls(1);
            // Confirm teardown did not run.
            mock_endpoints[TEARDOWN_KEY].assert_calls(0);
        }
        TestType::Stop => {
            // Confirm setup did not run.
            mock_endpoints[SETUP_KEY].assert_calls(0);
            // Confirm teardown ran one time.
            mock_endpoints[TEARDOWN_KEY].assert_calls(1);
        }
        TestType::StartAndStop => {
            // Confirm setup ran one time.
            mock_endpoints[SETUP_KEY].assert_calls(1);
            // Confirm teardown ran one time.
            mock_endpoints[TEARDOWN_KEY].assert_calls(1);
        }
    }
}

// Build an appropriate GooseAttack object for test type, using supplied configuration.
fn build_goose_attack(test_type: &TestType, configuration: GooseConfiguration) -> GooseAttack {
    let scenario =
        scenario!("LoadTest").register_transaction(transaction!(get_index).set_weight(9).unwrap());
    let start_transaction = transaction!(setup);
    let stop_transaction = transaction!(teardown);
    match test_type {
        TestType::Start => common::build_load_test(
            configuration,
            vec![scenario],
            Some(&start_transaction),
            None,
        ),
        TestType::Stop => {
            common::build_load_test(configuration, vec![scenario], None, Some(&stop_transaction))
        }
        TestType::StartAndStop => common::build_load_test(
            configuration,
            vec![scenario],
            Some(&start_transaction),
            Some(&stop_transaction),
        ),
    }
}

// Helper to run all standalone tests.
async fn run_standalone_test(test_type: TestType) {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let configuration = common_build_configuration(&server);

    // Use configuration to generate the load test.
    let goose_attack = build_goose_attack(&test_type, configuration);

    // Run the load test.
    common::run_load_test(goose_attack).await;

    // Confirm the load test ran correctly.
    validate_test(&test_type, &mock_endpoints);
}

#[tokio::test]
// Test test_start().
async fn test_setup() {
    run_standalone_test(TestType::Start).await;
}

#[tokio::test]
// Test test_stop().
async fn test_teardown() {
    run_standalone_test(TestType::Stop).await;
}

#[tokio::test]
/// Test test_start and test_stop together.
async fn test_setup_teardown() {
    run_standalone_test(TestType::StartAndStop).await;
}
