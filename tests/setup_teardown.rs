use httpmock::Method::{GET, POST};
use httpmock::{Mock, MockRef, MockServer};
use serial_test::serial;

mod common;

use goose::prelude::*;
use goose::GooseConfiguration;

const INDEX_PATH: &str = "/";
const SETUP_PATH: &str = "/setup";
const TEARDOWN_PATH: &str = "/teardown";

const INDEX_KEY: usize = 0;
const SETUP_KEY: usize = 1;
const TEARDOWN_KEY: usize = 2;

const EXPECT_WORKERS: usize = 2;
const USERS: &str = "4";

// Defines the different types of tests.
#[derive(Clone)]
enum TestType {
    // Testing on_start alone.
    Start,
    // Testing on_stop alone.
    Stop,
    // Testing on_start and on_stop together.
    StartAndStop,
}

pub async fn setup(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.post(SETUP_PATH, "setting up load test").await?;
    Ok(())
}

pub async fn teardown(user: &GooseUser) -> GooseTaskResult {
    let _goose = user
        .post(TEARDOWN_PATH, "cleaning up after load test")
        .await?;
    Ok(())
}

pub async fn get_index(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// All tests in this file run against common endpoints.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<MockRef> {
    let mut endpoints: Vec<MockRef> = Vec::new();

    // First set up INDEX_PATH, store in vector at INDEX_KEY.
    endpoints.push(
        Mock::new()
            .expect_method(GET)
            .expect_path(INDEX_PATH)
            .return_status(201)
            .create_on(&server),
    );
    // Next set up SETUP_PATH, store in vector at SETUP_KEY.
    endpoints.push(
        Mock::new()
            .expect_method(POST)
            .expect_path(SETUP_PATH)
            .return_status(205)
            .create_on(&server),
    );
    // Next set up TEARDOWN_PATH, store in vector at TEARDOWN_KEY.
    endpoints.push(
        Mock::new()
            .expect_method(POST)
            .expect_path(TEARDOWN_PATH)
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

// Common validation for the load tests in this file.
// @FIXME: remove the `is_gaggle` flag once issue #182 lands.
fn validate_test(test_type: &TestType, mock_endpoints: &[MockRef], is_gaggle: bool) {
    // Confirm the load test ran.
    assert!(mock_endpoints[INDEX_KEY].times_called() > 0);

    // Now confirm TestType-specific counters.
    if is_gaggle {
        // @FIXME: when https://github.com/tag1consulting/goose/issues/182 lands
        // remove the `is_gaggle` flag and everything in it: the gaggle and standalone
        // tests should have the same validation.
        match test_type {
            TestType::Start => {
                // @FIXME: setup should have run.
                assert!(mock_endpoints[SETUP_KEY].times_called() == 0);
                assert!(mock_endpoints[TEARDOWN_KEY].times_called() == 0);
            }
            TestType::Stop => {
                // @FIXME: teardown should have run.
                assert!(mock_endpoints[SETUP_KEY].times_called() == 0);
                assert!(mock_endpoints[TEARDOWN_KEY].times_called() == 0);
            }
            TestType::StartAndStop => {
                // @FIXME: setup and teardown should have run.
                assert!(mock_endpoints[SETUP_KEY].times_called() == 0);
                assert!(mock_endpoints[TEARDOWN_KEY].times_called() == 0);
            }
        }
    } else {
        match test_type {
            TestType::Start => {
                // Confirm setup ran one time.
                assert!(mock_endpoints[SETUP_KEY].times_called() == 1);
                // Confirm teardown did not run.
                assert!(mock_endpoints[TEARDOWN_KEY].times_called() == 0);
            }
            TestType::Stop => {
                // Confirm setup did not run.
                assert!(mock_endpoints[SETUP_KEY].times_called() == 0);
                // Confirm teardown ran one time.
                assert!(mock_endpoints[TEARDOWN_KEY].times_called() == 1);
            }
            TestType::StartAndStop => {
                // Confirm setup ran one time.
                assert!(mock_endpoints[SETUP_KEY].times_called() == 1);
                // Confirm teardown ran one time.
                assert!(mock_endpoints[TEARDOWN_KEY].times_called() == 1);
            }
        }
    }
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
        TestType::Start => goose.test_start(task!(setup)).register_taskset(
            taskset!("LoadTest").register_task(task!(get_index).set_weight(9).unwrap()),
        ),
        TestType::Stop => goose.test_stop(task!(teardown)).register_taskset(
            taskset!("LoadTest").register_task(task!(get_index).set_weight(9).unwrap()),
        ),
        TestType::StartAndStop => goose
            .test_start(task!(setup))
            .test_stop(task!(teardown))
            .register_taskset(
                taskset!("LoadTest").register_task(task!(get_index).set_weight(9).unwrap()),
            ),
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

/// Test test_start alone.
#[test]
fn test_setup() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let configuration = common_build_configuration(&server, None, None);

    // Define the type of test.
    let test_type = TestType::Start;

    // Run the load test as configured.
    run_load_test(&test_type, &configuration, None);

    // Confirm the load test ran correctly.
    validate_test(&TestType::Start, &mock_endpoints, false);
}

/// Test test_start alone.
#[test]
// Only run gaggle tests if the feature is compiled into the codebase.
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Gaggle tests have to be run serially instead of in parallel.
#[serial]
fn test_setup_gaggle() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let configuration = common_build_configuration(&server, Some(true), None);

    // Define the type of test.
    let test_type = TestType::Start;

    // Workers launched in own threads, store thread handles.
    let worker_handles = launch_workers(EXPECT_WORKERS, &test_type, &configuration);

    // Build Manager configuration.
    let manager_configuration = common_build_configuration(&server, None, Some(EXPECT_WORKERS));

    // Run the load test as configured.
    run_load_test(&test_type, &manager_configuration, Some(worker_handles));

    // Confirm the load test ran correctly.
    validate_test(&test_type, &mock_endpoints, true);
}

/// Test test_stop alone.
#[test]
fn test_teardown() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let configuration = common_build_configuration(&server, None, None);

    // Define the type of test.
    let test_type = TestType::Stop;

    // Run the load test as configured.
    run_load_test(&test_type, &configuration, None);

    // Confirm the load test ran correctly.
    validate_test(&test_type, &mock_endpoints, false);
}

/// Test test_start alone in Gaggle mode.
#[test]
// Only run Gaggle tests if the feature is compiled into the codebase.
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Gaggle tests have to be run serially instead of in parallel.
#[serial]
fn test_teardown_gaggle() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let configuration = common_build_configuration(&server, Some(true), None);

    // Define the type of test.
    let test_type = TestType::Stop;

    // Workers launched in own threads, store thread handles.
    let worker_handles = launch_workers(EXPECT_WORKERS, &test_type, &configuration);

    // Build Manager configuration.
    let manager_configuration = common_build_configuration(&server, None, Some(EXPECT_WORKERS));

    // Run the load test as configured.
    run_load_test(&test_type, &manager_configuration, Some(worker_handles));

    // Confirm the load test ran correctly.
    validate_test(&test_type, &mock_endpoints, true);
}

/// Test test_start and test_stop together.
#[test]
fn test_setup_teardown() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let configuration = common_build_configuration(&server, None, None);

    // Define the type of test.
    let test_type = TestType::StartAndStop;

    // Run the load test as configured.
    run_load_test(&test_type, &configuration, None);

    // Confirm the load test ran correctly.
    validate_test(&test_type, &mock_endpoints, false);
}

/// Test test_start and test_Stop together in Gaggle mode.
#[test]
// Only run Gaggle tests if the feature is compiled into the codebase.
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Gaggle tests have to be run serially instead of in parallel.
#[serial]
fn test_setup_teardown_gaggle() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let configuration = common_build_configuration(&server, Some(true), None);

    // Define the type of test.
    let test_type = TestType::StartAndStop;

    // Workers launched in own threads, store thread handles.
    let worker_handles = launch_workers(EXPECT_WORKERS, &test_type, &configuration);

    // Build Manager configuration.
    let manager_configuration = common_build_configuration(&server, None, Some(EXPECT_WORKERS));

    // Run the load test as configured.
    run_load_test(&test_type, &manager_configuration, Some(worker_handles));

    // Confirm the load test ran correctly.
    validate_test(&test_type, &mock_endpoints, true);
}
