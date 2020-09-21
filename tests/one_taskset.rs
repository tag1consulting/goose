use httpmock::Method::GET;
use httpmock::{Mock, MockRef, MockServer};

mod common;

use goose::goose::GooseMethod;
use goose::prelude::*;
use goose::GooseConfiguration;

const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about.html";
const INDEX_KEY: usize = 0;
const ABOUT_KEY: usize = 1;

pub async fn get_index(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

pub async fn get_about(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(ABOUT_PATH).await?;
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
            .return_status(200)
            .create_on(&server),
    );
    // Next set up ABOUT_PATH, store in vector at ABOUT_KEY.
    endpoints.push(
        Mock::new()
            .expect_method(GET)
            .expect_path(ABOUT_PATH)
            .return_status(200)
            .create_on(&server),
    );

    endpoints
}

// Create a custom configuration for this test.
fn common_build_configuration(server: &MockServer, custom: &mut Vec<&str>) -> GooseConfiguration {
    // Common elements in all our tests.
    let mut configuration = vec!["--users", "2", "--hatch-rate", "4", "--status-codes"];

    // Custom elements in some tests.
    configuration.append(custom);

    // Return the resulting configuration.
    common::build_configuration(&server, configuration)
}

// Common validation of load tests for the tests run in this file.
fn validate_one_taskset(
    goose_metrics: &GooseMetrics,
    mock_endpoints: &[MockRef],
    configuration: &GooseConfiguration,
    statistics_were_reset: bool,
) {
    // Confirm that we loaded the mock endpoints.
    assert!(mock_endpoints[INDEX_KEY].times_called() > 0);
    assert!(mock_endpoints[ABOUT_KEY].times_called() > 0);

    // Confirm that we loaded the index roughly three times as much as the about page.
    let one_third_index = mock_endpoints[INDEX_KEY].times_called() / 3;
    let difference = mock_endpoints[ABOUT_KEY].times_called() as i32 - one_third_index as i32;
    assert!(difference >= -2 && difference <= 2);

    // Get index and about out of goose metrics.
    let index_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", INDEX_PATH))
        .unwrap();
    let about_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", ABOUT_PATH))
        .unwrap();

    // Confirm that the path and method are correct in the statistics.
    assert!(index_metrics.path == INDEX_PATH);
    assert!(index_metrics.method == GooseMethod::GET);
    assert!(about_metrics.path == ABOUT_PATH);
    assert!(about_metrics.method == GooseMethod::GET);

    let status_code: u16 = 200;
    if statistics_were_reset {
        // Statistics were reset after all users were started, so Goose should report
        // fewer page loads than the server actually saw.
        assert!(index_metrics.response_time_counter < mock_endpoints[INDEX_KEY].times_called());
        assert!(
            index_metrics.status_code_counts[&status_code]
                < mock_endpoints[INDEX_KEY].times_called()
        );
        assert!(index_metrics.success_count < mock_endpoints[INDEX_KEY].times_called());
        assert!(about_metrics.response_time_counter < mock_endpoints[ABOUT_KEY].times_called());
        assert!(
            about_metrics.status_code_counts[&status_code]
                < mock_endpoints[ABOUT_KEY].times_called()
        );
        assert!(about_metrics.success_count < mock_endpoints[ABOUT_KEY].times_called());
    } else {
        // Statistics were not reset, so Goose should report the same number of page
        // loads as the server actually saw.
        assert!(index_metrics.response_time_counter == mock_endpoints[INDEX_KEY].times_called());
        assert!(
            index_metrics.status_code_counts[&status_code]
                == mock_endpoints[INDEX_KEY].times_called()
        );
        assert!(index_metrics.success_count == mock_endpoints[INDEX_KEY].times_called());
        assert!(about_metrics.response_time_counter == mock_endpoints[ABOUT_KEY].times_called());
        assert!(
            about_metrics.status_code_counts[&status_code]
                == mock_endpoints[ABOUT_KEY].times_called()
        );
        assert!(about_metrics.success_count == mock_endpoints[ABOUT_KEY].times_called());
    }

    // There should not have been any failures during this test.
    assert!(index_metrics.fail_count == 0);
    assert!(about_metrics.fail_count == 0);

    // Verify that Goose started the correct number of users.
    assert!(goose_metrics.users == configuration.users.unwrap());
}

// Run the actual load test, returning the resulting metrics.
fn run_load_test(configuration: &GooseConfiguration) -> GooseMetrics {
    crate::GooseAttack::initialize_with_config(configuration.clone())
        .unwrap()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_index).set_weight(9).unwrap())
                .register_task(task!(get_about).set_weight(3).unwrap()),
        )
        .execute()
        .unwrap()
}

#[test]
// Load test with a single task set containing two weighted tasks. Validate
// weighting and statistics.
fn test_single_taskset() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration elements, adding --no-reset-metrics.
    let configuration = common_build_configuration(&server, &mut vec!["--no-reset-metrics"]);

    // Run the load test.
    let goose_metrics = run_load_test(&configuration);

    // Confirm that the load test ran correctly.
    validate_one_taskset(&goose_metrics, &mock_endpoints, &configuration, false);
}

#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Spawn a gaggle of 1 manager and 2 workers each simulating one user. Run a load test,
// synchronize metrics from the workers to the manager, and validate that Goose tracked
// the same metrics as the mock server.
fn test_single_taskset_gaggle() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Launch workers in their own threads, storing the thread handle.
    let mut worker_handles = Vec::new();
    // Each worker has the same identical configuration.
    let worker_configuration = common::build_configuration(&server, vec!["--worker"]);

    for _ in 0..2 {
        let configuration = worker_configuration.clone();
        // Start worker instance of the load test.
        worker_handles.push(std::thread::spawn(move || {
            // Run the load test.
            let _goose_metrics = run_load_test(&configuration);
        }));
    }

    // Build common configuration elements, adding Manager Gaggle flags.
    let manager_configuration =
        common_build_configuration(&server, &mut vec!["--manager", "--expect-workers", "2"]);

    // Run the load test.
    let goose_metrics = run_load_test(&manager_configuration);

    // Confirm that the load test ran correctly.
    validate_one_taskset(
        &goose_metrics,
        &mock_endpoints,
        &manager_configuration,
        false,
    );
}

#[test]
// Load test with a single task set containing two weighted tasks. Validate
// weighting and statistics after resetting metrics.
fn test_single_taskset_reset_metrics() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration elements.
    let configuration = common_build_configuration(&server, &mut vec![]);

    // Run the load test.
    let goose_metrics = run_load_test(&configuration);

    // Confirm that the load test ran correctly.
    validate_one_taskset(&goose_metrics, &mock_endpoints, &configuration, true);
}
