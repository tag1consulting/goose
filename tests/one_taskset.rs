use httpmock::Method::GET;
use httpmock::{Mock, MockRef, MockServer};
use serial_test::serial;

mod common;

use goose::goose::GooseMethod;
use goose::prelude::*;
use goose::GooseConfiguration;

// Paths used in load tests performed during these tests.
const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about.html";

// Indexes to the above paths.
const INDEX_KEY: usize = 0;
const ABOUT_KEY: usize = 1;

// Load test configuration.
const EXPECT_WORKERS: usize = 2;

// There are multiple test variations in this file.
#[derive(Clone)]
enum TestType {
    // Enable --no-reset-metrics.
    NoResetMetrics,
    // Do not enable --no-reset-metrics.
    ResetMetrics,
}

// Test task.
pub async fn get_index(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// Test task.
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
        "--status-codes",
    ];

    // Custom elements in some tests.
    configuration.append(custom);

    // Return the resulting configuration.
    common::build_configuration(&server, configuration)
}

// Helper to confirm all variations generate appropriate results.
fn validate_one_taskset(
    goose_metrics: &GooseMetrics,
    mock_endpoints: &[MockRef],
    configuration: &GooseConfiguration,
    test_type: TestType,
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
    match test_type {
        TestType::ResetMetrics => {
            // Statistics were reset after all users were started, so Goose should report
            // fewer page loads than the server actually saw.
            println!(
                "response_time_counter: {}, mock_endpoint_called: {}",
                index_metrics.response_time_counter,
                mock_endpoints[INDEX_KEY].times_called()
            );

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
        }
        TestType::NoResetMetrics => {
            // Statistics were not reset, so Goose should report the same number of page
            // loads as the server actually saw.
            assert!(
                index_metrics.response_time_counter == mock_endpoints[INDEX_KEY].times_called()
            );
            assert!(
                index_metrics.status_code_counts[&status_code]
                    == mock_endpoints[INDEX_KEY].times_called()
            );
            assert!(index_metrics.success_count == mock_endpoints[INDEX_KEY].times_called());
            assert!(
                about_metrics.response_time_counter == mock_endpoints[ABOUT_KEY].times_called()
            );
            assert!(
                about_metrics.status_code_counts[&status_code]
                    == mock_endpoints[ABOUT_KEY].times_called()
            );
            assert!(about_metrics.success_count == mock_endpoints[ABOUT_KEY].times_called());
        }
    }

    // There should not have been any failures during this test.
    assert!(index_metrics.fail_count == 0);
    assert!(about_metrics.fail_count == 0);

    // Verify that Goose started the correct number of users.
    assert!(goose_metrics.users == configuration.users.unwrap());
}

// Returns the appropriate taskset needed to build these tests.
fn get_tasks() -> GooseTaskSet {
    taskset!("LoadTest")
        .register_task(task!(get_index).set_weight(9).unwrap())
        .register_task(task!(get_about).set_weight(3).unwrap())
}

// Helper to run all standalone tests.
fn run_standalone_test(test_type: TestType) {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    let mut configuration_flags = match test_type {
        TestType::NoResetMetrics => vec!["--no-reset-metrics"],
        TestType::ResetMetrics => vec![],
    };

    // Build common configuration elements.
    let configuration = common_build_configuration(&server, &mut configuration_flags);

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(
        common::build_load_test(configuration.clone(), &get_tasks(), None, None),
        None,
    );

    // Confirm that the load test ran correctly.
    validate_one_taskset(&goose_metrics, &mock_endpoints, &configuration, test_type);
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
        TestType::NoResetMetrics => common_build_configuration(
            &server,
            &mut vec![
                "--manager",
                "--expect-workers",
                &EXPECT_WORKERS.to_string(),
                "--no-reset-metrics",
            ],
        ),
        TestType::ResetMetrics => common_build_configuration(
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
    validate_one_taskset(
        &goose_metrics,
        &mock_endpoints,
        &manager_configuration,
        test_type,
    );
}

#[test]
// Test a single task set with multiple weighted tasks.
fn test_one_taskset() {
    run_standalone_test(TestType::NoResetMetrics);
}

#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Test a single task set with multiple weighted tasks, in Gaggle mode.
fn test_one_taskset_gaggle() {
    run_gaggle_test(TestType::NoResetMetrics);
}

#[test]
// Test a single task set with multiple weighted tasks, enable --no-reset-metrics.
fn test_one_taskset_reset_metrics() {
    run_standalone_test(TestType::ResetMetrics);
}

/* @TODO: @FIXME: Goose is not resetting metrics when running in Gaggle mode.
 * Issue: https://github.com/tag1consulting/goose/issues/193
#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Test a single task set with multiple weighted tasks, enable --no-reset-metrics
// in Gaggle mode.
fn test_one_taskset_reset_metrics_gaggle() {
    run_gaggle_test(TestType::ResetMetrics);
}
*/
