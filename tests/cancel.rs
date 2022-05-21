/// Validate that Goose properly shuts down when it receives SIGINT (control-c).
use httpmock::{Method::GET, Mock, MockServer};
use nix::sys::signal::{kill, SIGINT};
use nix::unistd::getpid;
use serial_test::serial;
use tokio::time::{sleep, Duration};

mod common;

use goose::config::GooseConfiguration;
use goose::goose::GooseMethod;
use goose::prelude::*;

// Paths used in load tests performed during these tests.
const INDEX_PATH: &str = "/";

// Indexes to the above paths.
const INDEX_KEY: usize = 0;

// Load test configuration.
const EXPECT_WORKERS: usize = 2;

// There are multiple test variations in this file.
enum TestType {
    // Runs until canceled.
    NoRunTime,
    // Runs a set amount of time (with --run-time) then exits.
    RunTime,
    // Run time is optionally configured through a test plan.
    TestPlan,
    // Runs a set number of iterations (with --iterations) then exits.
    Iterations,
}

// Test transaction.
pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// All tests in this file run against a common endpoint.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<Mock> {
    vec![
        // This load test only requests INDEX_PATH, which we store in vector at INDEX_KEY.
        server.mock(|when, then| {
            when.method(GET).path(INDEX_PATH);
            then.status(200);
        }),
    ]
}

// Build appropriate configuration for these tests.
fn common_build_configuration(
    server: &MockServer,
    test_type: &TestType,
    custom: &mut Vec<&str>,
) -> GooseConfiguration {
    // In all cases throttle requests to allow asserting metrics precisely.
    let mut configuration = match test_type {
        TestType::RunTime => {
            // Hatch 3 users a second for 2 seconds, then run for 2 more seconds.
            vec![
                "--throttle-requests",
                "5",
                "--users",
                "6",
                "--hatch-rate",
                "3",
                "--run-time",
                "10",
            ]
        }
        TestType::NoRunTime => {
            // Hatch 3 users a second for 2 seconds, then run until canceled.
            vec![
                "--throttle-requests",
                "5",
                "--users",
                "6",
                "--hatch-rate",
                "3",
            ]
        }
        TestType::TestPlan => {
            // Common configuration is the throttle, test-plan defined through `custom`.
            vec!["--throttle-requests", "5"]
        }
        TestType::Iterations => {
            // Hatch 3 users a second for 2 seconds, runing each for 5 iterations complete iterations then cancel.
            vec![
                "--throttle-requests",
                "5",
                "--users",
                "6",
                "--hatch-rate",
                "3",
                "iterations",
                "5",
            ]
        }
    };

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
    is_gaggle: bool,
) {
    // Confirm that we loaded the mock endpoints.
    assert!(mock_endpoints[INDEX_KEY].hits() > 0);

    // Get index and about out of goose metrics.
    let index_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", INDEX_PATH))
        .unwrap();

    // Confirm that the path and method are correct in the statistics.
    assert!(index_metrics.path == INDEX_PATH);
    assert!(index_metrics.method == GooseMethod::Get);

    // There should not have been any failures during this test.
    assert!(index_metrics.fail_count == 0);

    match test_type {
        TestType::RunTime => {
            assert!(goose_metrics.total_users == configuration.users.unwrap());
            if !is_gaggle {
                assert!(goose_metrics.history.len() == 4);
            }
        }
        TestType::NoRunTime => {
            assert!(goose_metrics.total_users == configuration.users.unwrap());
            if !is_gaggle {
                assert!(goose_metrics.history.len() == 4);
            }
        }
        TestType::TestPlan => {}
        TestType::Iterations => {}
    }
}

// Returns the appropriate scenario needed to build these tests.
fn get_transactions() -> Scenario {
    scenario!("LoadTest").register_transaction(transaction!(get_index).set_weight(9).unwrap())
}

async fn cancel_load_test(duration: Duration) {
    sleep(duration).await;
    kill(getpid(), SIGINT).expect("failed to send SIGNINT");
}

// Helper to run all standalone tests.
async fn run_standalone_test(test_type: TestType) {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    let mut configuration_flags = match test_type {
        TestType::RunTime => vec!["--no-reset-metrics"],
        TestType::NoRunTime => vec!["--no-reset-metrics"],
        TestType::TestPlan => vec![],
        TestType::Iterations => vec![],
    };

    // Build common configuration elements.
    let configuration = common_build_configuration(&server, &test_type, &mut configuration_flags);

    let cancel_delay = match test_type {
        TestType::RunTime => Duration::from_secs(3),
        TestType::NoRunTime => Duration::from_secs(2),
        TestType::TestPlan => Duration::from_secs(2),
        TestType::Iterations => Duration::from_secs(2),
    };

    // Start a thread that will send a SIGINT to the running load test.
    let _ = tokio::spawn(cancel_load_test(cancel_delay));

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(
        common::build_load_test(configuration.clone(), &get_transactions(), None, None),
        None,
    )
    .await;

    // Confirm that the load test ran correctly.
    validate_one_scenario(
        &goose_metrics,
        &mock_endpoints,
        &configuration,
        test_type,
        false,
    );
}

// Helper to run all standalone tests.
async fn run_gaggle_test(test_type: TestType) {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Each worker has the same identical configuration.
    let worker_configuration =
        common::build_configuration(&server, vec!["--worker", "--throttle-requests", "5"]);

    // Workers launched in own threads, store thread handles.
    let worker_handles = common::launch_gaggle_workers(EXPECT_WORKERS, || {
        common::build_load_test(
            worker_configuration.clone(),
            &get_transactions(),
            None,
            None,
        )
    });

    // Build common configuration elements, adding Manager Gaggle flags.
    let manager_configuration = match test_type {
        TestType::RunTime => common::build_configuration(
            &server,
            vec![
                "--manager",
                "--expect-workers",
                &EXPECT_WORKERS.to_string(),
                "--no-reset-metrics",
                "--users",
                "6",
                "--hatch-rate",
                "3",
                "--run-time",
                "10",
            ],
        ),
        TestType::NoRunTime => common::build_configuration(
            &server,
            vec![
                "--manager",
                "--expect-workers",
                &EXPECT_WORKERS.to_string(),
                "--no-reset-metrics",
                "--users",
                "6",
                "--hatch-rate",
                "3",
            ],
        ),
        TestType::Iterations => common::build_configuration(
            &server,
            vec!["--manager", "--expect-workers", &EXPECT_WORKERS.to_string()],
        ),
        TestType::TestPlan => panic!("test plan configuration not supported in gaggle mode"),
    };

    // Build the load test for the Manager.
    let manager_goose_attack = common::build_load_test(
        manager_configuration.clone(),
        &get_transactions(),
        None,
        None,
    );

    let cancel_delay = match test_type {
        TestType::RunTime => Duration::from_secs(3),
        TestType::NoRunTime => Duration::from_secs(2),
        TestType::TestPlan => Duration::from_secs(2),
        TestType::Iterations => Duration::from_secs(2),
    };

    // Start a thread that will send a SIGINT to the running load test.
    let _ = tokio::spawn(cancel_load_test(cancel_delay));

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(manager_goose_attack, Some(worker_handles)).await;

    // Confirm that the load test ran correctly.
    validate_one_scenario(
        &goose_metrics,
        &mock_endpoints,
        &manager_configuration,
        test_type,
        true,
    );
}

/* With --run-time */

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Cancel a scenario with --run-time configured before it times out.
async fn test_cancel_runtime() {
    run_standalone_test(TestType::RunTime).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Cancel a scenario with --run-time configured before it times out, in Gaggle mode.
async fn test_cancel_runtime_gaggle() {
    run_gaggle_test(TestType::RunTime).await;
}

/* Without --run-time */

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Cancel a scenario without --run-time configured.
async fn test_cancel_noruntime() {
    run_standalone_test(TestType::NoRunTime).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Cancel a scenario without --run-time configured, in Gaggle mode.
async fn test_cancel_noruntime_gaggle() {
    run_gaggle_test(TestType::NoRunTime).await;
}
