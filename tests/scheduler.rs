use httpmock::Method::GET;
use httpmock::{Mock, MockRef, MockServer};
use serial_test::serial;
use tokio::time::{delay_for, Duration};

mod common;

use goose::prelude::*;
use goose::GooseConfiguration;

// Paths used in load tests performed during these tests.
const ONE_PATH: &str = "/one";
const TWO_PATH: &str = "/two";
const START_ONE_PATH: &str = "/start/one";
const STOP_ONE_PATH: &str = "/stop/one";

// Indexes to the above paths.
const ONE_KEY: usize = 0;
const TWO_KEY: usize = 1;
const START_ONE_KEY: usize = 2;
const STOP_ONE_KEY: usize = 3;

// Load test configuration.
const EXPECT_WORKERS: usize = 3;
// Users needs to be an even number.
const USERS: usize = 10;
const RUN_TIME: usize = 2;

// There are multiple test variations in this file.
#[derive(Clone)]
enum TestType {
    // Launch task sets serially as defined.
    Serial,
    // Launch task sets in a round robin fashion.
    RoundRobin,
}

// Test task.
pub async fn one_with_delay(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(ONE_PATH).await?;

    // "Run out the clock" on the load test when this function runs. Sleep for
    // the total duration the test is to run plus 1 second to be sure no
    // additional tasks will run after this one.
    delay_for(Duration::from_secs(RUN_TIME as u64 + 1)).await;

    Ok(())
}

// Test task.
pub async fn two_with_delay(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(TWO_PATH).await?;

    // "Run out the clock" on the load test when this function runs. Sleep for
    // the total duration the test is to run plus 1 second to be sure no
    // additional tasks will run after this one.
    delay_for(Duration::from_secs(RUN_TIME as u64 + 1)).await;

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
    endpoints.push(
        Mock::new()
            .expect_method(GET)
            .expect_path(ONE_PATH)
            .return_status(200)
            .create_on(&server),
    );
    // Next set up TWO_PATH, store in vector at TWO_KEY.
    endpoints.push(
        Mock::new()
            .expect_method(GET)
            .expect_path(TWO_PATH)
            .return_status(200)
            .create_on(&server),
    );
    // Next set up START_ONE_PATH, store in vector at START_ONE_KEY.
    endpoints.push(
        Mock::new()
            .expect_method(GET)
            .expect_path(START_ONE_PATH)
            .return_status(200)
            .create_on(&server),
    );
    // Next set up STOP_ONE_PATH, store in vector at STOP_ONE_KEY.
    endpoints.push(
        Mock::new()
            .expect_method(GET)
            .expect_path(STOP_ONE_PATH)
            .return_status(200)
            .create_on(&server),
    );

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
    assert!(mock_endpoints[START_ONE_KEY].times_called() == 1);

    // Now confirm TestType-specific counters.
    match test_type {
        TestType::RoundRobin => {
            // We launch an equal number of each task set, so we call both endpoints
            // an equal number of times.
            assert!(
                mock_endpoints[TWO_KEY].times_called() == mock_endpoints[ONE_KEY].times_called()
            );
            assert!(mock_endpoints[ONE_KEY].times_called() == USERS / 2);
        }
        TestType::Serial => {
            // As we only launch as many users as the weight of the first task set, we only
            // call the first endpoint, never the second endpoint.
            assert!(mock_endpoints[ONE_KEY].times_called() == USERS);
            assert!(mock_endpoints[TWO_KEY].times_called() == 0);
        }
    }

    // STOP_ONE_PATH is loaded one and only one time on all variations.
    assert!(mock_endpoints[STOP_ONE_KEY].times_called() == 1);
}

// Returns the appropriate taskset, start_task and stop_task needed to build these tests.
fn get_tasks() -> (GooseTaskSet, GooseTaskSet, GooseTask, GooseTask) {
    (
        taskset!("TaskSetOne")
            .register_task(task!(one_with_delay))
            .set_weight(USERS)
            .unwrap(),
        taskset!("TaskSetTwo")
            .register_task(task!(two_with_delay))
            .set_weight(USERS + 1)
            .unwrap(),
        // Start runs before all other tasks, regardless of where defined.
        task!(start_one),
        // Stop runs after all other tasks, regardless of where defined.
        task!(stop_one),
    )
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
    let (taskset1, taskset2, start_task, stop_task) = get_tasks();

    // First set up the common base configuration.
    let mut goose_attack = crate::GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_taskset(taskset1)
        .register_taskset(taskset2)
        .test_start(start_task)
        .test_stop(stop_task);

    // Then configure which scheduler the GooseAttack should launch users with.
    goose_attack = match test_type {
        TestType::RoundRobin => goose_attack.set_scheduler(GooseScheduler::RoundRobin),
        TestType::Serial => goose_attack.set_scheduler(GooseScheduler::Serial),
    };

    // Run the Goose Attack.
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

    // Get the taskset, start and stop tasks to build a load test.
    let (taskset1, taskset2, start_task, stop_task) = get_tasks();

    // First set up the common base configuration.
    let mut goose_attack = crate::GooseAttack::initialize_with_config(worker_configuration)
        .unwrap()
        .register_taskset(taskset1.clone())
        .register_taskset(taskset2.clone())
        .test_start(start_task.clone())
        .test_stop(stop_task.clone());

    // Then configure which scheduler the GooseAttack should launch users with.
    goose_attack = match test_type {
        TestType::RoundRobin => goose_attack.set_scheduler(GooseScheduler::RoundRobin),
        TestType::Serial => goose_attack.set_scheduler(GooseScheduler::Serial),
    };

    // Workers launched in own threads, store thread handles.
    let worker_handles = common::launch_gaggle_workers(goose_attack, EXPECT_WORKERS);

    // Build Manager configuration.
    let manager_configuration = common_build_configuration(&server, None, Some(EXPECT_WORKERS));

    // Build the load test for the Manager.
    let mut manager_goose_attack =
        crate::GooseAttack::initialize_with_config(manager_configuration)
            .unwrap()
            .register_taskset(taskset1)
            .register_taskset(taskset2)
            .test_start(start_task)
            .test_stop(stop_task);

    // Then configure which scheduler the GooseAttack should launch users with.
    manager_goose_attack = match test_type {
        TestType::RoundRobin => manager_goose_attack.set_scheduler(GooseScheduler::RoundRobin),
        TestType::Serial => manager_goose_attack.set_scheduler(GooseScheduler::Serial),
    };

    // Run the Goose Attack.
    common::run_load_test(manager_goose_attack, Some(worker_handles));

    // Confirm the load test ran correctly.
    validate_test(&test_type, &mock_endpoints);
}

#[test]
// Load test with multiple tasks allocating GooseTaskSets in round robin order.
fn test_round_robin() {
    run_standalone_test(TestType::RoundRobin);
}

#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Load test with multiple tasks allocating GooseTaskSets in round robin order, in
// Gaggle mode.
fn test_round_robin_gaggle() {
    run_gaggle_test(TestType::RoundRobin);
}

#[test]
// Load test with multiple tasks allocating GooseTaskSets in serial order.
fn test_serial() {
    run_standalone_test(TestType::Serial);
}

#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Load test with multiple tasks allocating GooseTaskSets in serial order, in
// Gaggle mode.
fn test_serial_gaggle() {
    run_gaggle_test(TestType::Serial);
}
