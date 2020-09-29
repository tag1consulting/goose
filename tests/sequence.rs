use httpmock::Method::GET;
use httpmock::{Mock, MockRef, MockServer};
use serial_test::serial;
use tokio::time::{delay_for, Duration};

mod common;

use goose::prelude::*;
use goose::GooseConfiguration;

const ONE_PATH: &str = "/one";
const TWO_PATH: &str = "/two";
const THREE_PATH: &str = "/three";
const START_ONE_PATH: &str = "/start/one";
const STOP_ONE_PATH: &str = "/stop/one";

const ONE_KEY: usize = 0;
const TWO_KEY: usize = 1;
const THREE_KEY: usize = 2;
const START_ONE_KEY: usize = 3;
const STOP_ONE_KEY: usize = 4;

const EXPECT_WORKERS: usize = 2;
const USERS: usize = 4;
const RUN_TIME: usize = 2;

// Define the different types of tests run in this file.
#[derive(Clone)]
enum TestType {
    NotSequenced,
    Sequenced,
}

pub async fn one(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(ONE_PATH).await?;

    Ok(())
}

pub async fn two_with_delay(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(TWO_PATH).await?;

    // "Run out the clock" on the load test when this function runs. Sleep for
    // the total duration the test is to run plus 50 milliseconds to be sure
    // no additional tasks will run after this one.
    delay_for(Duration::from_millis(RUN_TIME as u64 * 1000 + 50)).await;

    Ok(())
}

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
    // Next set up THREE_PATH, store in vector at THREE_KEY.
    endpoints.push(
        Mock::new()
            .expect_method(GET)
            .expect_path(THREE_PATH)
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

// Create a custom configuration for this test.
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

// Common validation for the load tests in this file.
fn validate_test(test_type: &TestType, mock_endpoints: &[MockRef]) {
    // START_ONE_PATH is loaded one and only one time on all variations.
    assert!(mock_endpoints[START_ONE_KEY].times_called() == 1);

    // ONE_PATH is loaded twice per user (due to weight) on all variations.
    assert!(mock_endpoints[ONE_KEY].times_called() == USERS * 2);

    // Now confirm TestType-specific counters.
    match test_type {
        TestType::NotSequenced => {
            // All tasks run one time.
            assert!(mock_endpoints[TWO_KEY].times_called() == USERS);
            assert!(mock_endpoints[THREE_KEY].times_called() == USERS);
        }
        TestType::Sequenced => {
            // Two runs out the clock, so three never runs.
            assert!(mock_endpoints[TWO_KEY].times_called() == USERS);
            assert!(mock_endpoints[THREE_KEY].times_called() == 0);
        }
    }

    // STOP_ONE_PATH is loaded one and only one time on all variations.
    assert!(mock_endpoints[STOP_ONE_KEY].times_called() == 1);
}

// Execute each Worker in its own thread, returning a vector of handles.
fn launch_workers(
    expect_workers: usize,
    test_type: &TestType,
    configuration: &GooseConfiguration,
) -> Vec<std::thread::JoinHandle<()>> {
    // Launch each worker in its own thread, storing the join handles.
    let mut worker_handles = Vec::new();
    for _ in 0..expect_workers {
        let worker_test_type = test_type.clone();
        let worker_configuration = configuration.clone();
        // Start worker instance of the load test.
        worker_handles.push(std::thread::spawn(move || {
            // Run the load test as configured.
            run_load_test(&worker_test_type, &worker_configuration, None);
        }));
    }

    worker_handles
}

// Run the actual load test. Rely on the mock server to confirm it ran correctly, so
// do not return metrics.
fn run_load_test(
    test_type: &TestType,
    configuration: &GooseConfiguration,
    worker_handles: Option<Vec<std::thread::JoinHandle<()>>>,
) {
    // First set up the common base configuration.
    let goose = crate::GooseAttack::initialize_with_config(configuration.clone()).unwrap();

    // Next add the appropriate TaskSet.
    let goose = match test_type {
        // No sequence declared, so tasks run in the order they are defined: 1, 1, 3, 2...
        TestType::NotSequenced => goose
            .register_taskset(
                taskset!("LoadTest")
                    .register_task(task!(one).set_weight(2).unwrap())
                    .register_task(task!(three))
                    .register_task(task!(two_with_delay)),
            )
            // Stop runs after all other tasks, regardless of where defined.
            .test_stop(task!(stop_one))
            // Start runs before all other tasks, regardless of where defined.
            .test_start(task!(start_one)),
        // Sequence added, so tasks run in the declared sequence order: 1, 1, 2, 3...
        TestType::Sequenced => goose
            .register_taskset(
                taskset!("LoadTest")
                    .register_task(task!(one).set_sequence(1).set_weight(2).unwrap())
                    .register_task(task!(three).set_sequence(3))
                    .register_task(task!(two_with_delay).set_sequence(2)),
            )
            // Stop runs after all other tasks, regardless of where defined.
            .test_stop(task!(stop_one))
            // Start runs before all other tasks, regardless of where defined.
            .test_start(task!(start_one)),
    };

    // Execute the load test. Validation is done by the mock server, so do not
    // return the goose_statistics.
    let _goose_statistics = goose.execute().unwrap();

    // If this is a Manager test, first wait for the Workers to exit to return.
    if let Some(handles) = worker_handles {
        // Wait for both worker threads to finish and exit.
        for handle in handles {
            let _ = handle.join();
        }
    }
}

// Baseline, run a test with no sequences defined.
#[test]
fn test_not_sequenced() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let configuration = common_build_configuration(&server, None, None);

    // Define the type of test.
    let test_type = TestType::NotSequenced;

    // Run the load test as configured.
    run_load_test(&test_type, &configuration, None);

    // Confirm the load test ran correctly.
    validate_test(&test_type, &mock_endpoints);
}

/// Test test_start alone.
#[test]
// Only run gaggle tests if the feature is compiled into the codebase.
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Gaggle tests have to be run serially instead of in parallel.
#[serial]
fn test_not_sequenced_gaggle() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let configuration = common_build_configuration(&server, Some(true), None);

    // Define the type of test.
    let test_type = TestType::NotSequenced;

    // Workers launched in own threads, store thread handles.
    let worker_handles = launch_workers(EXPECT_WORKERS, &test_type, &configuration);

    // Build Manager configuration.
    let manager_configuration = common_build_configuration(&server, None, Some(EXPECT_WORKERS));

    // Run the load test as configured.
    run_load_test(&test_type, &manager_configuration, Some(worker_handles));

    // Confirm the load test ran correctly.
    validate_test(&test_type, &mock_endpoints);
}

// Now run a test with sequences defined.
#[test]
fn test_sequenced() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let configuration = common_build_configuration(&server, None, None);

    // Define the type of test.
    let test_type = TestType::Sequenced;

    // Run the load test as configured.
    run_load_test(&test_type, &configuration, None);

    // Confirm the load test ran correctly.
    validate_test(&test_type, &mock_endpoints);
}

/// Test test_start alone.
#[test]
// Only run gaggle tests if the feature is compiled into the codebase.
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Gaggle tests have to be run serially instead of in parallel.
#[serial]
fn test_sequenced_gaggle() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let configuration = common_build_configuration(&server, Some(true), None);

    // Define the type of test.
    let test_type = TestType::Sequenced;

    // Workers launched in own threads, store thread handles.
    let worker_handles = launch_workers(EXPECT_WORKERS, &test_type, &configuration);

    // Build Manager configuration.
    let manager_configuration = common_build_configuration(&server, None, Some(EXPECT_WORKERS));

    // Run the load test as configured.
    run_load_test(&test_type, &manager_configuration, Some(worker_handles));

    // Confirm the load test ran correctly.
    validate_test(&test_type, &mock_endpoints);
}
