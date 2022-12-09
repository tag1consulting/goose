/// Validate that Goose only runs the selected Scenario filtered by --scenarios.
use httpmock::{Method::GET, Mock, MockServer};
use serial_test::serial;

mod common;

use goose::config::GooseConfiguration;
use goose::goose::GooseMethod;
use goose::prelude::*;

// Paths used in load tests performed during these tests.
const SCENARIOA1: &str = "/path/a/1";
const SCENARIOA2: &str = "/path/a/2";
const SCENARIOB1: &str = "/path/b/1";
const SCENARIOB2: &str = "/path/b/2";

// Indexes to the above paths.
const SCENARIOA1_KEY: usize = 0;
const SCENARIOA2_KEY: usize = 1;
const SCENARIOB1_KEY: usize = 2;
const SCENARIOB2_KEY: usize = 3;

// Load test configuration.
const EXPECT_WORKERS: usize = 2;

// There are multiple test variations in this file.
enum TestType {
    // Limit scenarios with --scenarios option.
    ScenariosOption,
    // Limit scenarios with GooseDefault::Scenarios.
    ScenariosDefault,
}

// Test transaction.
pub async fn get_scenarioa1(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(SCENARIOA1).await?;
    Ok(())
}

// Test transaction.
pub async fn get_scenarioa2(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(SCENARIOA2).await?;
    Ok(())
}

// Test transaction.
pub async fn get_scenariob1(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(SCENARIOB1).await?;
    Ok(())
}

// Test transaction.
pub async fn get_scenariob2(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(SCENARIOB2).await?;
    Ok(())
}

// All tests in this file run against a common endpoint.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<Mock> {
    vec![
        // SCENARIOA1 is stored in vector at SCENARIOA1_KEY.
        server.mock(|when, then| {
            when.method(GET).path(SCENARIOA1);
            then.status(200);
        }),
        // SCENARIOA2 is stored in vector at SCENARIOA2_KEY.
        server.mock(|when, then| {
            when.method(GET).path(SCENARIOA2);
            then.status(200);
        }),
        // SCENARIOB1 is stored in vector at SCENARIOB1_KEY.
        server.mock(|when, then| {
            when.method(GET).path(SCENARIOB1);
            then.status(200);
        }),
        // SCENARIOB2 is stored in vector at SCENARIOB2_KEY.
        server.mock(|when, then| {
            when.method(GET).path(SCENARIOB2);
            then.status(200);
        }),
    ]
}

// Build appropriate configuration for these tests.
fn common_build_configuration(server: &MockServer, test_type: &TestType) -> GooseConfiguration {
    // In all cases throttle requests to allow asserting metrics precisely.
    let configuration = match test_type {
        TestType::ScenariosOption => {
            // Start 10 users in 1 second, then run for 1 more second.
            vec![
                "--users",
                "10",
                "--hatch-rate",
                "10",
                "--run-time",
                "1",
                "--no-reset-metrics",
                // Only run Scenario A1 and Scenario A2
                "--scenarios",
                "scenarioa",
            ]
        }
        TestType::ScenariosDefault => {
            // Start 10 users in 1 second, then run for 1 more second.
            vec![
                "--users",
                "10",
                "--hatch-rate",
                "10",
                "--run-time",
                "1",
                "--no-reset-metrics",
            ]
        }
    };

    // Build the resulting configuration.
    common::build_configuration(server, configuration)
}

// Helper to confirm all variations generate appropriate results.
fn validate_loadtest(
    goose_metrics: &GooseMetrics,
    mock_endpoints: &[Mock],
    configuration: &GooseConfiguration,
    test_type: TestType,
) {
    assert!(goose_metrics.total_users == configuration.users.unwrap());
    assert!(goose_metrics.maximum_users == 10);
    assert!(goose_metrics.total_users == 10);

    match test_type {
        TestType::ScenariosOption => {
            // Get scenarioa1 metrics.
            let scenarioa1_metrics = goose_metrics
                .requests
                .get(&format!("GET {}", SCENARIOA1))
                .unwrap();
            // Confirm that the path and method are correct in the statistics.
            assert!(scenarioa1_metrics.path == SCENARIOA1);
            assert!(scenarioa1_metrics.method == GooseMethod::Get);
            // There should not have been any failures during this test.
            assert!(scenarioa1_metrics.fail_count == 0);
            // Confirm Goose and the mock endpoint agree on the number of requests made.
            assert!(mock_endpoints[SCENARIOA1_KEY].hits() <= scenarioa1_metrics.success_count);

            // Get scenarioa2 metrics.
            let scenarioa2_metrics = goose_metrics
                .requests
                .get(&format!("GET {}", SCENARIOA2))
                .unwrap();
            // Confirm that the path and method are correct in the statistics.
            assert!(scenarioa2_metrics.path == SCENARIOA2);
            assert!(scenarioa2_metrics.method == GooseMethod::Get);
            // There should not have been any failures during this test.
            assert!(scenarioa2_metrics.fail_count == 0);
            // Confirm Goose and the mock endpoint agree on the number of requests made.
            assert!(mock_endpoints[SCENARIOA2_KEY].hits() <= scenarioa2_metrics.success_count);

            // scenariob1 and scenariob2 should not have been loaded due to `--scenarios scenarioa`.
            assert!(mock_endpoints[SCENARIOB1_KEY].hits() == 0);
            assert!(mock_endpoints[SCENARIOB2_KEY].hits() == 0);
        }
        TestType::ScenariosDefault => {
            // scenarioa1 and scenarioa2 should not have been loaded due to `GooseDefault::Scenarios`.
            assert!(mock_endpoints[SCENARIOA1_KEY].hits() == 0);
            assert!(mock_endpoints[SCENARIOA2_KEY].hits() == 0);
            assert!(mock_endpoints[SCENARIOB1_KEY].hits() > 0);
            assert!(mock_endpoints[SCENARIOB2_KEY].hits() > 0);
        }
    }
}

// Returns the appropriate scenarios needed to build these tests.
fn get_scenarios() -> Vec<Scenario> {
    vec![
        scenario!("Scenario A1").register_transaction(transaction!(get_scenarioa1)),
        scenario!("Scenario A2").register_transaction(transaction!(get_scenarioa2)),
        scenario!("Scenario B1").register_transaction(transaction!(get_scenariob1)),
        scenario!("Scenario B2").register_transaction(transaction!(get_scenariob2)),
    ]
}

// Helper to run all standalone tests.
async fn run_standalone_test(test_type: TestType) {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration elements.
    let configuration = common_build_configuration(&server, &test_type);

    let mut goose = common::build_load_test(configuration.clone(), get_scenarios(), None, None);

    // By default, only run scenarios starting with `scenariob`.
    goose = *goose
        .set_default(GooseDefault::Scenarios, "scenariob")
        .unwrap();

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(goose, None).await;

    // Confirm that the load test ran correctly.
    validate_loadtest(&goose_metrics, &mock_endpoints, &configuration, test_type);
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
        common::build_load_test(worker_configuration.clone(), get_scenarios(), None, None)
    });

    // Build common configuration elements, adding Manager Gaggle flags.
    let manager_configuration = match test_type {
        TestType::ScenariosOption => common::build_configuration(
            &server,
            vec![
                "--manager",
                "--expect-workers",
                &EXPECT_WORKERS.to_string(),
                "--no-reset-metrics",
                "--users",
                "10",
                "--hatch-rate",
                "10",
                "--run-time",
                "1",
                "--scenarios",
                "scenarioa",
            ],
        ),
        TestType::ScenariosDefault => common::build_configuration(
            &server,
            vec![
                "--manager",
                "--expect-workers",
                &EXPECT_WORKERS.to_string(),
                "--no-reset-metrics",
                "--users",
                "10",
                "--hatch-rate",
                "10",
                "--run-time",
                "1",
            ],
        ),
    };

    // Build the load test for the Manager.
    let mut manager_goose_attack =
        common::build_load_test(manager_configuration.clone(), get_scenarios(), None, None);

    // By default, only run scenarios starting with `scenariob`.
    manager_goose_attack = *manager_goose_attack
        .set_default(GooseDefault::Scenarios, "scenariob")
        .unwrap();

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(manager_goose_attack, Some(worker_handles)).await;

    // Confirm that the load test ran correctly.
    validate_loadtest(
        &goose_metrics,
        &mock_endpoints,
        &manager_configuration,
        test_type,
    );
}

/* With `--scenarios` */

#[tokio::test]
// Run only half the configured scenarios.
async fn test_scenarios_option() {
    run_standalone_test(TestType::ScenariosOption).await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Run only half the configured scenarios, in Gaggle mode.
async fn test_scenarios_option_gaggle() {
    run_gaggle_test(TestType::ScenariosOption).await;
}

/* With `GooseDefault::Scenarios` */

#[tokio::test]
// Run only half the configured scenarios.
async fn test_scenarios_default() {
    run_standalone_test(TestType::ScenariosDefault).await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Run only half the configured scenarios, in Gaggle mode.
async fn test_scenarios_default_gaggle() {
    run_gaggle_test(TestType::ScenariosDefault).await;
}
