use httpmock::Method::{GET, POST};
use httpmock::{Mock, MockRef, MockServer};

mod common;

use goose::prelude::*;
use goose::GooseConfiguration;

const INDEX_PATH: &str = "/";
const SETUP_PATH: &str = "/setup";
const TEARDOWN_PATH: &str = "/teardown";

const INDEX_KEY: usize = 0;
const SETUP_KEY: usize = 1;
const TEARDOWN_KEY: usize = 2;

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
    custom: Option<&mut Vec<&str>>,
) -> GooseConfiguration {
    // The base configuration is just the defaults.
    let mut configuration = vec![];

    // Add custom elements if defined.
    if let Some(custom_configuration) = custom {
        configuration.append(custom_configuration);
    }

    // Return the resulting configuration.
    common::build_configuration(&server, configuration)
}

fn validate_test(test_type: &TestType, mock_endpoints: &Vec<MockRef>) {
    // Confirm the load test ran.
    assert!(mock_endpoints[INDEX_KEY].times_called() > 0);

    // Now confirm TestType-specific counters.
    match test_type {
        TestType::Start => {
            // Confirm we ran setup one time.
            assert!(mock_endpoints[SETUP_KEY].times_called() == 1);
            // Confirm we did not run the teardown.
            assert!(mock_endpoints[TEARDOWN_KEY].times_called() == 0);
        }
        TestType::Stop => {
            // Confirm we did not run setup.
            assert!(mock_endpoints[SETUP_KEY].times_called() == 0);
            // Confirm we ran the teardown 1 time.
            assert!(mock_endpoints[TEARDOWN_KEY].times_called() == 1);
        }
        TestType::StartAndStop => {
            // Confirm we ran setup one time.
            assert!(mock_endpoints[SETUP_KEY].times_called() == 1);
            // Confirm we ran teardown one time.
            assert!(mock_endpoints[TEARDOWN_KEY].times_called() == 1);
        }
    }
}

// Run the actual load test. Rely on the mock server to confirm it ran correctly, so
// do not return metrics.
fn run_load_test(test_type: &TestType, configuration: &GooseConfiguration) {
    let goose = crate::GooseAttack::initialize_with_config(configuration.clone()).unwrap();

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

    // Finally, execute the load test.
    let _ = goose.execute().unwrap();
}

/// Test test_start alone.
#[test]
fn test_setup() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let configuration = common_build_configuration(&server, None);

    // Define the type of test.
    let test_type = TestType::Start;

    // Run the load test as configured.
    run_load_test(&test_type, &configuration);

    // Confirm the load test ran correctly.
    validate_test(&TestType::Start, &mock_endpoints);
}

/// Test test_stop alone.
#[test]
fn test_teardown() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let configuration = common_build_configuration(&server, None);

    // Define the type of test.
    let test_type = TestType::Stop;

    // Run the load test as configured.
    run_load_test(&test_type, &configuration);

    // Confirm the load test ran correctly.
    validate_test(&test_type, &mock_endpoints);
}

/// Test test_start and test_stop together.
#[test]
fn test_setup_teardown() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration, add additional users to be sure setup and teardown only run
    // one time.
    let mut custom_configuration = vec!["--users", "5", "--hatch-rate", "5"];
    let configuration = common_build_configuration(&server, Some(&mut custom_configuration));

    // Define the type of test.
    let test_type = TestType::StartAndStop;

    // Run the load test as configured.
    run_load_test(&test_type, &configuration);

    // Confirm the load test ran correctly.
    validate_test(&test_type, &mock_endpoints);
}
