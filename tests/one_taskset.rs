use httpmock::{Method::GET, Mock, MockServer};
use serial_test::serial;

mod common;

use goose::config::GooseConfiguration;
use goose::goose::GooseMethod;
use goose::prelude::*;

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

// Test transaction.
pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// Test transaction.
pub async fn get_about(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(ABOUT_PATH).await?;
    Ok(())
}

// All tests in this file run against common endpoints.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<Mock> {
    vec![
        // First set up INDEX_PATH, store in vector at INDEX_KEY.
        server.mock(|when, then| {
            when.method(GET).path(INDEX_PATH);
            then.status(200);
        }),
        // Next set up ABOUT_PATH, store in vector at ABOUT_KEY.
        server.mock(|when, then| {
            when.method(GET).path(ABOUT_PATH);
            then.status(200);
        }),
    ]
}

// Build appropriate configuration for these tests.
fn common_build_configuration(server: &MockServer, custom: &mut Vec<&str>) -> GooseConfiguration {
    // Common elements in all our tests.
    let mut configuration = vec!["--users", "2", "--hatch-rate", "4", "--run-time", "2"];

    // Custom elements in some tests.
    configuration.append(custom);

    // Return the resulting configuration.
    common::build_configuration(server, configuration)
}

// Helper to confirm all variations generate appropriate results.
fn validate_one_scenario(
    goose_metrics: &GooseMetrics,
    mock_endpoints: &[Mock],
    configuration: &GooseConfiguration,
    test_type: TestType,
) {
    // Confirm that we loaded the mock endpoints.
    assert!(mock_endpoints[INDEX_KEY].hits() > 0);
    assert!(mock_endpoints[ABOUT_KEY].hits() > 0);

    // Confirm that we loaded the index roughly three times as much as the about page.
    let one_third_index = mock_endpoints[INDEX_KEY].hits() / 3;
    let difference = mock_endpoints[ABOUT_KEY].hits() as i32 - one_third_index as i32;
    assert!((-2..=2).contains(&difference));

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
    assert!(index_metrics.method == GooseMethod::Get);
    assert!(about_metrics.path == ABOUT_PATH);
    assert!(about_metrics.method == GooseMethod::Get);

    let status_code: u16 = 200;
    match test_type {
        TestType::ResetMetrics => {
            // Statistics were reset after all users were started, so Goose should report
            // fewer page loads than the server actually saw.
            println!(
                "raw_data.counter: {}, mock_endpoint_called: {}",
                index_metrics.raw_data.counter,
                mock_endpoints[INDEX_KEY].hits()
            );

            assert!(index_metrics.raw_data.counter < mock_endpoints[INDEX_KEY].hits());
            assert!(
                index_metrics.status_code_counts[&status_code] < mock_endpoints[INDEX_KEY].hits()
            );
            assert!(index_metrics.success_count < mock_endpoints[INDEX_KEY].hits());
            assert!(about_metrics.raw_data.counter < mock_endpoints[ABOUT_KEY].hits());
            assert!(
                about_metrics.status_code_counts[&status_code] < mock_endpoints[ABOUT_KEY].hits()
            );
            assert!(about_metrics.success_count < mock_endpoints[ABOUT_KEY].hits());
        }
        TestType::NoResetMetrics => {
            // Statistics were not reset, so Goose should report the same number of page
            // loads as the server actually saw.
            mock_endpoints[INDEX_KEY].assert_hits(index_metrics.raw_data.counter);
            mock_endpoints[INDEX_KEY].assert_hits(index_metrics.status_code_counts[&status_code]);
            mock_endpoints[INDEX_KEY].assert_hits(index_metrics.success_count);
            mock_endpoints[ABOUT_KEY].assert_hits(about_metrics.raw_data.counter);
            mock_endpoints[ABOUT_KEY].assert_hits(about_metrics.status_code_counts[&status_code]);
            mock_endpoints[ABOUT_KEY].assert_hits(about_metrics.success_count);
        }
    }

    // There should not have been any failures during this test.
    assert!(index_metrics.fail_count == 0);
    assert!(about_metrics.fail_count == 0);

    // Verify that Goose started the correct number of users.
    assert!(goose_metrics.total_users == configuration.users.unwrap());
}

// Returns the appropriate scenario needed to build these tests.
fn get_transactions() -> Scenario {
    scenario!("LoadTest")
        .register_transaction(transaction!(get_index).set_weight(9).unwrap())
        .register_transaction(transaction!(get_about).set_weight(3).unwrap())
}

// Helper to run all standalone tests.
async fn run_standalone_test(test_type: TestType) {
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
        common::build_load_test(configuration.clone(), vec![get_transactions()], None, None),
        None,
    )
    .await;

    // Confirm that the load test ran correctly.
    validate_one_scenario(&goose_metrics, &mock_endpoints, &configuration, test_type);
}

// Helper to run all standalone tests.
async fn run_gaggle_test(test_type: TestType) {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Each worker has the same identical configuration.
    let worker_configuration = common::build_configuration(&server, vec!["--worker"]);

    // Workers launched in own threads, store thread handles.
    let worker_handles = common::launch_gaggle_workers(EXPECT_WORKERS, || {
        common::build_load_test(
            worker_configuration.clone(),
            vec![get_transactions()],
            None,
            None,
        )
    });

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
    let manager_goose_attack = common::build_load_test(
        manager_configuration.clone(),
        vec![get_transactions()],
        None,
        None,
    );

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(manager_goose_attack, Some(worker_handles)).await;

    // Confirm that the load test ran correctly.
    validate_one_scenario(
        &goose_metrics,
        &mock_endpoints,
        &manager_configuration,
        test_type,
    );
}

#[tokio::test]
// Test a single scenario with multiple weighted transactions.
async fn test_one_scenario() {
    run_standalone_test(TestType::NoResetMetrics).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Test a single scenario with multiple weighted transactions, in Gaggle mode.
async fn test_one_scenario_gaggle() {
    run_gaggle_test(TestType::NoResetMetrics).await;
}

#[tokio::test]
// Test a single scenario with multiple weighted transactions, enable --no-reset-metrics.
async fn test_one_scenario_reset_metrics() {
    run_standalone_test(TestType::ResetMetrics).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Test a single scenario with multiple weighted transactions, enable --no-reset-metrics
// in Gaggle mode.
// @TODO: @FIXME: Goose is not resetting metrics when running in Gaggle mode.
// Issue: https://github.com/tag1consulting/goose/issues/193
async fn test_one_senario_reset_metrics_gaggle() {
    run_gaggle_test(TestType::ResetMetrics).await;
}
