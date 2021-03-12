use httpmock::{Method::GET, MockRef, MockServer};
use serial_test::serial;
use tokio::time::{sleep, Duration};

mod common;

use goose::prelude::*;
use goose::GooseConfiguration;

// Paths used in load tests performed during these tests.
const ONE_PATH: &str = "/one";
const TWO_PATH: &str = "/two";
const THREE_PATH: &str = "/three";
const START_ONE_PATH: &str = "/start/one";
const STOP_ONE_PATH: &str = "/stop/one";

// Indexes to the above paths.
const ONE_KEY: usize = 0;
const TWO_KEY: usize = 1;
const THREE_KEY: usize = 2;
const START_ONE_KEY: usize = 3;
const STOP_ONE_KEY: usize = 4;

// Load test configuration.
const EXPECT_WORKERS: usize = 2;
const USERS: usize = 4;
const RUN_TIME: usize = 2;

// There are multiple test variations in this file.
#[derive(Clone)]
enum TestType {
    // No sequences defined in load test.
    NotSequenced,
    // Sequences defined in load test.
    Sequenced,
}

// Test task.
pub async fn one(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(ONE_PATH).await?;

    Ok(())
}

// Test task.
pub async fn two_with_delay(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(TWO_PATH).await?;

    // "Run out the clock" on the load test when this function runs. Sleep for
    // the total duration the test is to run plus 1 second to be sure no
    // additional tasks will run after this one.
    sleep(Duration::from_secs(RUN_TIME as u64 + 1)).await;

    Ok(())
}

// Test task.
pub async fn three(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(THREE_PATH).await?;

    Ok(())
}

// Used as a test_start() function, which always runs one time.
pub async fn start_one(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(START_ONE_PATH).await?;

    Ok(())
}

// Used as a test_stop() function, which always runs one time.
pub async fn stop_one(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(STOP_ONE_PATH).await?;

    Ok(())
}

// All tests in this file run against common endpoints.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<MockRef> {
    let mut endpoints: Vec<MockRef> = Vec::new();

    // First set up ONE_PATH, store in vector at ONE_KEY.
    endpoints.push(server.mock(|when, then| {
        when.method(GET).path(ONE_PATH);
        then.status(200);
    }));
    // Next set up TWO_PATH, store in vector at TWO_KEY.
    endpoints.push(server.mock(|when, then| {
        when.method(GET).path(TWO_PATH);
        then.status(200);
    }));
    // Next set up THREE_PATH, store in vector at THREE_KEY.
    endpoints.push(server.mock(|when, then| {
        when.method(GET).path(THREE_PATH);
        then.status(200);
    }));
    // Next set up START_ONE_PATH, store in vector at START_ONE_KEY.
    endpoints.push(server.mock(|when, then| {
        when.method(GET).path(START_ONE_PATH);
        then.status(200);
    }));
    // Next set up STOP_ONE_PATH, store in vector at STOP_ONE_KEY.
    endpoints.push(server.mock(|when, then| {
        when.method(GET).path(STOP_ONE_PATH);
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
                &USERS.to_string(),
                "--hatch-rate",
                &USERS.to_string(),
                "--run-time",
                &RUN_TIME.to_string(),
                "--no-reset-metrics",
            ],
        )
    } else if worker.is_some() {
        common::build_configuration(&server, vec!["--worker"])
    } else {
        common::build_configuration(
            &server,
            vec![
                "--users",
                &USERS.to_string(),
                "--hatch-rate",
                &USERS.to_string(),
                "--run-time",
                &RUN_TIME.to_string(),
                "--no-reset-metrics",
            ],
        )
    }
}

// Helper to confirm all variations generate appropriate results.
fn validate_test(test_type: &TestType, mock_endpoints: &[MockRef]) {
    // START_ONE_PATH is loaded one and only one time on all variations.
    mock_endpoints[START_ONE_KEY].assert_hits(1);

    // ONE_PATH is loaded twice per user (due to weight) on all variations.
    mock_endpoints[ONE_KEY].assert_hits(USERS * 2);

    // Now confirm TestType-specific counters.
    match test_type {
        TestType::NotSequenced => {
            // All tasks run one time.
            mock_endpoints[TWO_KEY].assert_hits(USERS);
            mock_endpoints[THREE_KEY].assert_hits(USERS)
        }
        TestType::Sequenced => {
            // Two runs out the clock, so three never runs.
            mock_endpoints[TWO_KEY].assert_hits(USERS);
            mock_endpoints[THREE_KEY].assert_hits(0);
        }
    }

    // STOP_ONE_PATH is loaded one and only one time on all variations.
    mock_endpoints[STOP_ONE_KEY].assert_hits(1);
}

// Returns the appropriate taskset, start_task and stop_task needed to build these tests.
fn get_tasks(test_type: &TestType) -> (GooseTaskSet, GooseTask, GooseTask) {
    match test_type {
        // No sequence declared, so tasks run in the order they are defined: 1, 1, 3, 2...
        TestType::NotSequenced => (
            taskset!("LoadTest")
                .register_task(task!(one).set_weight(2).unwrap())
                .register_task(task!(three))
                .register_task(task!(two_with_delay)),
            // Start runs before all other tasks, regardless of where defined.
            task!(start_one),
            // Stop runs after all other tasks, regardless of where defined.
            task!(stop_one),
        ),
        // Sequence added, so tasks run in the declared sequence order: 1, 1, 2, 3...
        TestType::Sequenced => (
            taskset!("LoadTest")
                .register_task(task!(one).set_sequence(1).set_weight(2).unwrap())
                .register_task(task!(three).set_sequence(3))
                .register_task(task!(two_with_delay).set_sequence(2)),
            // Start runs before all other tasks, regardless of where defined.
            task!(start_one),
            // Stop runs after all other tasks, regardless of where defined.
            task!(stop_one),
        ),
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

    // Get the taskset, start and stop tasks to build a load test.
    let (taskset, start_task, stop_task) = get_tasks(&test_type);

    // Run the Goose Attack.
    common::run_load_test(
        common::build_load_test(configuration, &taskset, Some(&start_task), Some(&stop_task)),
        None,
    );

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

    // Get the taskset, start and stop tasks to build a load test.
    let (taskset, start_task, stop_task) = get_tasks(&test_type);

    // Build the load test for the Workers.
    let goose_attack = common::build_load_test(
        worker_configuration,
        &taskset,
        Some(&start_task),
        Some(&stop_task),
    );

    // Workers launched in own threads, store thread handles.
    let worker_handles = common::launch_gaggle_workers(goose_attack, EXPECT_WORKERS);

    // Build Manager configuration.
    let manager_configuration = common_build_configuration(&server, None, Some(EXPECT_WORKERS));

    // Build the load test for the Manager.
    let manager_goose_attack = common::build_load_test(
        manager_configuration,
        &taskset,
        Some(&start_task),
        Some(&stop_task),
    );

    // Run the Goose Attack.
    common::run_load_test(manager_goose_attack, Some(worker_handles));

    // Confirm the load test ran correctly.
    validate_test(&test_type, &mock_endpoints);
}

#[test]
// Load test with multiple tasks and no sequences defined.
fn test_not_sequenced() {
    run_standalone_test(TestType::NotSequenced);
}

#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Load test with multiple tasks and no sequences defined, in Gaggle mode.
fn test_not_sequenced_gaggle() {
    run_gaggle_test(TestType::NotSequenced);
}

#[test]
// Load test with multiple tasks and sequences defined.
fn test_sequenced() {
    run_standalone_test(TestType::Sequenced);
}

#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Load test with multiple tasks and sequences defined, in Gaggle mode.
fn test_sequenced_gaggle() {
    run_gaggle_test(TestType::Sequenced);
}
