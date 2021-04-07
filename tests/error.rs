use httpmock::{Method::GET, MockRef, MockServer};
use serial_test::serial;

mod common;

use goose::goose::GooseMethod;
use goose::prelude::*;
use goose::GooseConfiguration;

// Paths used in load tests performed during these tests.
const INDEX_PATH: &str = "/";
const A_404_PATH: &str = "/404";

// Indexes to the above paths.
const INDEX_KEY: usize = 0;
const A_404_KEY: usize = 1;

// Load test configuration.
const EXPECT_WORKERS: usize = 2;

// There are multiple test variations in this file.
#[derive(Clone)]
enum TestType {
    // Enable --no-error-summary.
    NoErrorSummary,
    // Do not enable --no-error-summary.
    ErrorSummary,
}

// Test task.
pub async fn get_index(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// Test task.
pub async fn get_404_path(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(A_404_PATH).await?;
    Ok(())
}

// All tests in this file run against common endpoints.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<MockRef> {
    let mut endpoints: Vec<MockRef> = Vec::new();

    // First set up INDEX_PATH, store in vector at INDEX_KEY.
    endpoints.push(server.mock(|when, then| {
        when.method(GET).path(INDEX_PATH);
        then.status(200);
    }));
    // Next set up ABOUT_PATH, store in vector at ABOUT_KEY.
    endpoints.push(server.mock(|when, then| {
        when.method(GET).path(A_404_PATH);
        then.status(404);
    }));

    endpoints
}

// Build appropriate configuration for these tests.
fn common_build_configuration(server: &MockServer, custom: &mut Vec<&str>) -> GooseConfiguration {
    // Common elements in all our tests.
    let mut configuration = vec![
        "--users",
        "2",
        "--hatch-rate",
        "4",
        "--run-time",
        "2",
        "--no-reset-metrics",
    ];

    // Custom elements in some tests.
    configuration.append(custom);

    // Return the resulting configuration.
    common::build_configuration(&server, configuration)
}

// Helper to confirm all variations generate appropriate results.
fn validate_error(
    goose_metrics: &GooseMetrics,
    mock_endpoints: &[MockRef],
    configuration: &GooseConfiguration,
    test_type: TestType,
) {
    // Confirm that we loaded the mock endpoints.
    assert!(mock_endpoints[INDEX_KEY].hits() > 0);
    assert!(mock_endpoints[A_404_KEY].hits() > 0);

    // Get request metrics.
    let index_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", INDEX_PATH))
        .unwrap();
    let a_404_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", A_404_PATH))
        .unwrap();

    // Get error metrics.
    let a_404_errors = goose_metrics.errors.clone();

    // Confirm that the path and method are correct in the metrics.
    assert!(index_metrics.path == INDEX_PATH);
    assert!(index_metrics.method == GooseMethod::GET);
    assert!(a_404_metrics.path == A_404_PATH);
    assert!(a_404_metrics.method == GooseMethod::GET);

    // All requests to the index succeeded.
    mock_endpoints[INDEX_KEY].assert_hits(index_metrics.response_time_counter);
    mock_endpoints[INDEX_KEY].assert_hits(index_metrics.success_count);
    // All requests to the 404 page failed.
    mock_endpoints[A_404_KEY].assert_hits(a_404_metrics.response_time_counter);
    mock_endpoints[A_404_KEY].assert_hits(a_404_metrics.fail_count);

    match test_type {
        TestType::ErrorSummary => {
            // The 404 path was captured as an error.
            assert!(a_404_errors.len() == 1);
            // Extract the error from the BTreeMap.
            for error in a_404_errors {
                // The captured error was a GET request.
                assert!(error.1.method == GooseMethod::GET);
                // The captured error was for the 404 path.
                assert!(error.1.name == A_404_PATH);
                // The error was captured the number of times we requested the 404 path.
                assert!(error.1.occurrences == a_404_metrics.fail_count);
            }
        }
        TestType::NoErrorSummary => {
            // Goose was configured to not capture any errors.
            assert!(a_404_errors.is_empty());
        }
    }

    // The index always loaded successfully.
    assert!(index_metrics.fail_count == 0);

    // The 404 path was always an error.
    assert!(a_404_metrics.success_count == 0);

    // Verify that Goose started the correct number of users.
    assert!(goose_metrics.users == configuration.users.unwrap());
}

// Returns the appropriate taskset needed to build these tests.
fn get_tasks() -> GooseTaskSet {
    taskset!("LoadTest")
        .register_task(task!(get_index))
        .register_task(task!(get_404_path))
}

// Helper to run all standalone tests.
fn run_standalone_test(test_type: TestType) {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    let mut configuration_flags = match test_type {
        TestType::NoErrorSummary => vec!["--no-error-summary"],
        TestType::ErrorSummary => vec![],
    };

    // Build common configuration elements.
    let configuration = common_build_configuration(&server, &mut configuration_flags);

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(
        common::build_load_test(configuration.clone(), &get_tasks(), None, None),
        None,
    );

    // Confirm that the load test ran correctly.
    validate_error(&goose_metrics, &mock_endpoints, &configuration, test_type);
}

// Helper to run all standalone tests.
fn run_gaggle_test(test_type: TestType) {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Each worker has the same identical configuration.
    let worker_configuration = common::build_configuration(&server, vec!["--worker"]);

    // Build the load test for the Workers.
    let goose_attack = common::build_load_test(worker_configuration, &get_tasks(), None, None);

    // Workers launched in own threads, store thread handles.
    let worker_handles = common::launch_gaggle_workers(goose_attack, EXPECT_WORKERS);

    // Build common configuration elements, adding Manager Gaggle flags.
    let manager_configuration = match test_type {
        TestType::NoErrorSummary => common_build_configuration(
            &server,
            &mut vec![
                "--manager",
                "--expect-workers",
                &EXPECT_WORKERS.to_string(),
                "--no-error-summary",
            ],
        ),
        TestType::ErrorSummary => common_build_configuration(
            &server,
            &mut vec!["--manager", "--expect-workers", &EXPECT_WORKERS.to_string()],
        ),
    };

    // Build the load test for the Manager.
    let manager_goose_attack =
        common::build_load_test(manager_configuration.clone(), &get_tasks(), None, None);

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(manager_goose_attack, Some(worker_handles));

    // Confirm that the load test ran correctly.
    validate_error(
        &goose_metrics,
        &mock_endpoints,
        &manager_configuration,
        test_type,
    );
}

#[test]
// Confirm that errors show up in the summary when enabled.
fn test_error_summary() {
    run_standalone_test(TestType::ErrorSummary);
}

#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Confirm that errors show up in the summary when enabled, in Gaggle mode.
fn test_error_summary_gaggle() {
    run_gaggle_test(TestType::ErrorSummary);
}

#[test]
// Confirm that errors do not show up in the summary when --no-error-summary is enabled.
fn test_no_error_summary() {
    run_standalone_test(TestType::NoErrorSummary);
}

#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Confirm that errors do not show up in the summary when --no-error-summary is enabled,
// in Gaggle mode.
fn test_no_error_summary_gaggle() {
    run_gaggle_test(TestType::NoErrorSummary);
}
