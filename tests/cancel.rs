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
    // Cancel a load test that would otherwise run forever.
    NoRunTime,
    // Cancel a load test that would otherwise shut down automatically (with --run-time).
    RunTime,
    // Cancel a load test while the test plan is increasing the users.
    TestPlanIncrease,
    // Cancel a load test while the test plan is decreasing the users.
    TestPlanDecrease,
    // Cancel a load test while the test plan is maintaining the users.
    TestPlanMaintain,
    // Cancel a load test while running a finite number of iterations (--with iterations).
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
fn common_build_configuration(server: &MockServer, test_type: &TestType) -> GooseConfiguration {
    // In all cases throttle requests to allow asserting metrics precisely.
    let configuration = match test_type {
        TestType::RunTime => {
            // Hatch 3 users a second for 2 seconds, then run for 2 more seconds.
            vec![
                "--throttle-requests",
                "5",
                "--users",
                "10",
                "--hatch-rate",
                "5",
                "--run-time",
                "10",
                "--no-reset-metrics",
            ]
        }
        TestType::NoRunTime => {
            // Hatch 3 users a second for 2 seconds, then run until canceled.
            vec![
                "--throttle-requests",
                "5",
                "--users",
                "10",
                "--hatch-rate",
                "5",
                "--no-reset-metrics",
            ]
        }
        TestType::TestPlanIncrease => {
            // Launch 10 users in 1 minute, then run until cannceled.
            vec!["--throttle-requests", "5", "--test-plan", "10,1m"]
        }
        TestType::TestPlanDecrease => {
            // Launch 10 users in 1 minute, spend 1 minute reducing to 1 user, then run
            // until canceled.
            vec!["--throttle-requests", "5", "--test-plan", "10,1s;1,1m"]
        }
        TestType::TestPlanMaintain => {
            // Launch 10 users in 1 second, then maintain until canceled.
            vec!["--throttle-requests", "5", "--test-plan", "10,1s"]
        }
        TestType::Iterations => {
            // Hatch 3 users a second for 2 seconds, runing each for 5 iterations complete iterations then cancel.
            vec![
                "--throttle-requests",
                "5",
                "--users",
                "10",
                "--hatch-rate",
                "5",
                "--iterations",
                "5",
            ]
        }
    };

    // Build the resulting configuration.
    let mut configuration = common::build_configuration(server, configuration);

    // The common::build_configuration() function sets a few default options which have to be unset in
    // some the following configurations.
    match test_type {
        TestType::Iterations | TestType::NoRunTime => {
            // Do not set --run-time with --iterations or when testing not setting run time.
            configuration.run_time = "".to_string();
        }
        TestType::TestPlanIncrease | TestType::TestPlanDecrease | TestType::TestPlanMaintain => {
            // Do not set --run-time with --test-plan.
            configuration.run_time = "".to_string();
            // Do not set --hatch-rate with --test-plan.
            configuration.hatch_rate = None;
            // Do not set --users with --test-plan.
            configuration.users = None;
        }
        TestType::RunTime => {
            // No changes reuired for this test.
        }
    }

    configuration
}

// Helper to confirm all variations generate appropriate results.
fn validate_one_scenario(
    goose_metrics: &GooseMetrics,
    mock_endpoints: &[Mock],
    configuration: &GooseConfiguration,
    test_type: TestType,
    is_gaggle: bool,
) {
    // The throttle limits how many hits are registered.
    if is_gaggle {
        assert!(mock_endpoints[INDEX_KEY].hits() <= 40);
    } else {
        assert!(mock_endpoints[INDEX_KEY].hits() <= 20);
    }
    assert!(mock_endpoints[INDEX_KEY].hits() > 10);

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

    // The load test should run for 3 seconds before being canceled.
    if !is_gaggle {
        assert!(goose_metrics.duration == 3);
    }

    match test_type {
        TestType::RunTime => {
            assert!(goose_metrics.total_users == configuration.users.unwrap());
            if !is_gaggle {
                assert!(goose_metrics.history.len() == 4);
            }
            assert!(goose_metrics.maximum_users == 10);
            assert!(goose_metrics.total_users == 10);
        }
        TestType::NoRunTime => {
            assert!(goose_metrics.total_users == configuration.users.unwrap());
            if !is_gaggle {
                assert!(goose_metrics.history.len() == 4);
            }
            assert!(goose_metrics.maximum_users == 10);
            assert!(goose_metrics.total_users == 10);
        }
        TestType::TestPlanIncrease => {
            assert!(goose_metrics.history.len() == 3);
            assert!(goose_metrics.maximum_users < 10);
            assert!(goose_metrics.total_users < 10);
        }
        TestType::TestPlanDecrease => {
            assert!(goose_metrics.history.len() == 4);
            assert!(goose_metrics.maximum_users == 10);
            assert!(goose_metrics.total_users == 10);
        }
        TestType::TestPlanMaintain => {
            assert!(goose_metrics.history.len() == 4);
            assert!(goose_metrics.maximum_users == 10);
            assert!(goose_metrics.total_users == 10);
        }
        TestType::Iterations => {
            if !is_gaggle {
                assert!(goose_metrics.history.len() == 4);
            }
            assert!(goose_metrics.maximum_users == 10);
            assert!(goose_metrics.total_users == 10);
        }
    }
}

// Returns the appropriate scenario needed to build these tests.
fn get_transactions() -> Scenario {
    scenario!("LoadTest").register_transaction(transaction!(get_index).set_weight(9).unwrap())
}

// Sleeps for Duration then sends SIGINT (ctrl-c) to the load test process.
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

    // Build common configuration elements.
    let configuration = common_build_configuration(&server, &test_type);

    // Start a thread that will send a SIGINT to the running load test.
    // Don't await tokio::spawn, instead run in parallel and continue to start Goose Attakc.
    let _ignored_joinhandle = tokio::spawn(cancel_load_test(Duration::from_secs(3)));

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(
        common::build_load_test(configuration.clone(), vec![get_transactions()], None, None),
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
            vec![get_transactions()],
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
                "10",
                "--hatch-rate",
                "5",
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
                "10",
                "--hatch-rate",
                "5",
            ],
        ),
        TestType::Iterations => common::build_configuration(
            &server,
            vec![
                "--manager",
                "--expect-workers",
                &EXPECT_WORKERS.to_string(),
                "--users",
                "10",
                "--hatch-rate",
                "5",
            ],
        ),
        TestType::TestPlanIncrease | TestType::TestPlanDecrease | TestType::TestPlanMaintain => {
            panic!("test plan configuration not supported in gaggle mode")
        }
    };

    // Build the load test for the Manager.
    let manager_goose_attack = common::build_load_test(
        manager_configuration.clone(),
        vec![get_transactions()],
        None,
        None,
    );

    // Start a thread that will send a SIGINT to the running load test.
    // Don't await tokio::spawn, instead run in parallel and continue to start Goose Attakc.
    let _ignored_joinhandle = tokio::spawn(cancel_load_test(Duration::from_secs(3)));

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

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
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

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Cancel a scenario without --run-time configured, in Gaggle mode.
async fn test_cancel_noruntime_gaggle() {
    run_gaggle_test(TestType::NoRunTime).await;
}

/* With --iterations */

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Cancel a scenario with --iterations configured.
async fn test_cancel_iterations() {
    run_standalone_test(TestType::Iterations).await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Cancel a scenario with --iterations configured, in Gaggle mode.
async fn test_cancel_iterations_gaggle() {
    run_gaggle_test(TestType::Iterations).await;
}

/* With --test--plan */

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Cancel a scenario with --iterations configured.
async fn test_cancel_testplan_increase() {
    run_standalone_test(TestType::TestPlanIncrease).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Cancel a scenario with --iterations configured.
async fn test_cancel_testplan_decrease() {
    run_standalone_test(TestType::TestPlanDecrease).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Cancel a scenario with --iterations configured.
async fn test_cancel_testplan_maintain() {
    run_standalone_test(TestType::TestPlanMaintain).await;
}
