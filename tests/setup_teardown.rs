use httpmock::{
    Method::{GET, POST},
    MockRef, MockServer,
};
use serial_test::serial;

mod common;

use goose::prelude::*;
use goose::GooseConfiguration;

// Paths used in load tests performed during these tests.
const INDEX_PATH: &str = "/";
const SETUP_PATH: &str = "/setup";
const TEARDOWN_PATH: &str = "/teardown";

// Indexes to the above paths.
const INDEX_KEY: usize = 0;
const SETUP_KEY: usize = 1;
const TEARDOWN_KEY: usize = 2;

// Load test configuration.
const EXPECT_WORKERS: usize = 2;
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

// Test task.
pub async fn setup(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.post(SETUP_PATH, "setting up load test").await?;
    Ok(())
}

// Test task.
pub async fn teardown(user: &GooseUser) -> GooseTaskResult {
    let _goose = user
        .post(TEARDOWN_PATH, "cleaning up after load test")
        .await?;
    Ok(())
}

// Test task.
pub async fn get_index(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// All tests in this file run against common endpoints.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<MockRef> {
    let mut endpoints: Vec<MockRef> = Vec::new();

    // First set up INDEX_PATH, store in vector at INDEX_KEY.
    endpoints.push(server.mock(|when, then| {
        when.method(GET).path(INDEX_PATH);
        then.status(201);
    }));
    // Next set up SETUP_PATH, store in vector at SETUP_KEY.
    endpoints.push(server.mock(|when, then| {
        when.method(POST).path(SETUP_PATH);
        then.status(205);
    }));
    // Next set up TEARDOWN_PATH, store in vector at TEARDOWN_KEY.
    endpoints.push(server.mock(|when, then| {
        when.method(POST).path(TEARDOWN_PATH);
        then.status(200);
    }));

    endpoints
}

// Build appropriate configuration for these tests.
fn common_build_configuration(
    server: &MockServer,
    worker: Option<bool>,
    manager: Option<usize>,
) -> GooseConfiguration {
    if let Some(expect_workers) = manager {
        common::build_configuration(
            &server,
            vec![
                "--manager",
                "--expect-workers",
                &expect_workers.to_string(),
                "--users",
                USERS,
                "--hatch-rate",
                USERS,
            ],
        )
    } else if worker.is_some() {
        common::build_configuration(&server, vec!["--worker"])
    } else {
        common::build_configuration(&server, vec!["--users", USERS, "--hatch-rate", USERS])
    }
}

// Helper to confirm all variations generate appropriate results.
fn validate_test(test_type: &TestType, mock_endpoints: &[MockRef]) {
    // Confirm the load test ran.
    assert!(mock_endpoints[INDEX_KEY].hits() > 0);

    // Now confirm TestType-specific counters.
    match test_type {
        TestType::Start => {
            // Confirm setup ran one time.
            mock_endpoints[SETUP_KEY].assert_hits(1);
            // Confirm teardown did not run.
            mock_endpoints[TEARDOWN_KEY].assert_hits(0);
        }
        TestType::Stop => {
            // Confirm setup did not run.
            mock_endpoints[SETUP_KEY].assert_hits(0);
            // Confirm teardown ran one time.
            mock_endpoints[TEARDOWN_KEY].assert_hits(1);
        }
        TestType::StartAndStop => {
            // Confirm setup ran one time.
            mock_endpoints[SETUP_KEY].assert_hits(1);
            // Confirm teardown ran one time.
            mock_endpoints[TEARDOWN_KEY].assert_hits(1);
        }
    }
}

// Build an appropriate GooseAttack object for test type, using supplied configuration.
fn build_goose_attack(test_type: &TestType, configuration: GooseConfiguration) -> GooseAttack {
    let taskset = taskset!("LoadTest").register_task(task!(get_index).set_weight(9).unwrap());
    let start_task = task!(setup);
    let stop_task = task!(teardown);
    match test_type {
        TestType::Start => {
            common::build_load_test(configuration, &taskset, Some(&start_task), None)
        }
        TestType::Stop => common::build_load_test(configuration, &taskset, None, Some(&stop_task)),
        TestType::StartAndStop => {
            common::build_load_test(configuration, &taskset, Some(&start_task), Some(&stop_task))
        }
    }
}

// Helper to run all standalone tests.
fn run_standalone_test(test_type: TestType) {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let configuration = common_build_configuration(&server, None, None);

    // Use configuration to generate the load test.
    let goose_attack = build_goose_attack(&test_type, configuration);

    // Run the load test.
    common::run_load_test(goose_attack, None);

    // Confirm the load test ran correctly.
    validate_test(&test_type, &mock_endpoints);
}

// Helper to run all gaggle tests.
fn run_gaggle_test(test_type: TestType) {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let worker_configuration = common_build_configuration(&server, Some(true), None);

    // Use Worker configuration to generate the load test.
    let goose_attack = build_goose_attack(&test_type, worker_configuration);

    // Workers launched in own threads, store thread handles.
    let worker_handles = common::launch_gaggle_workers(goose_attack, EXPECT_WORKERS);

    // Build Manager configuration.
    let manager_configuration = common_build_configuration(&server, None, Some(EXPECT_WORKERS));

    // Use Manager configuration to generate the load test.
    let goose_attack = build_goose_attack(&test_type, manager_configuration);

    // Run the load test.
    common::run_load_test(goose_attack, Some(worker_handles));

    // Confirm the load test ran correctly.
    validate_test(&test_type, &mock_endpoints);
}

#[test]
// Test test_start().
fn test_setup() {
    run_standalone_test(TestType::Start);
}

#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Test test_start(), in Gaggle mode.
fn test_setup_gaggle() {
    run_gaggle_test(TestType::Start);
}

#[test]
// Test test_stop().
fn test_teardown() {
    run_standalone_test(TestType::Stop);
}

#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Test test_stop(), in Gaggle mode.
fn test_teardown_gaggle() {
    run_gaggle_test(TestType::Stop);
}

#[test]
/// Test test_start and test_stop together.
fn test_setup_teardown() {
    run_standalone_test(TestType::StartAndStop);
}

#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
/// Test test_start and test_stop together, in Gaggle mode.
fn test_setup_teardown_gaggle() {
    run_gaggle_test(TestType::StartAndStop);
}
