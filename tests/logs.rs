use httpmock::{Method::GET, Mock, MockServer};
use serial_test::serial;
use std::fmt;

mod common;

use goose::prelude::*;

// Paths used in load tests performed during these tests.
const INDEX_PATH: &str = "/";
const ERROR_PATH: &str = "/error";

// Indexes to the above paths.
const INDEX_KEY: usize = 0;
const ERROR_KEY: usize = 1;

// Load test configuration.
const EXPECT_WORKERS: usize = 2;

// There are multiple test variations in this file.
enum TestType {
    // Test with requests log enabled.
    Requests,
    // Test with transaction log enabled.
    Transactions,
    // Test with scenario log enabled.
    Scenarios,
    // Test with error log enabled.
    Error,
    // Test with debug log enabled.
    Debug,
    // Test with all log files enabled.
    All,
}

struct LogFiles<'a> {
    request_logs: &'a [String],
    transaction_logs: &'a [String],
    scenario_logs: &'a [String],
    error_logs: &'a [String],
    debug_logs: &'a [String],
}

// Implement fmt::Display for TestType to uniquely name the log files generated
// by each test. This is necessary as tests run in parallel.
impl fmt::Display for TestType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let printable = match *self {
            TestType::Requests => "requests",
            TestType::Transactions => "transactions",
            TestType::Scenarios => "scenarios",
            TestType::Error => "error",
            TestType::Debug => "debug",
            TestType::All => "all",
        };
        write!(f, "{}", printable)
    }
}

// Test transaction.
pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// Test transaction.
pub async fn get_error(user: &mut GooseUser) -> TransactionResult {
    let mut goose = user.get(ERROR_PATH).await?;

    if let Ok(r) = goose.response {
        let headers = &r.headers().clone();
        let status_code = r.status();
        if !status_code.is_success() {
            return user.set_failure(
                "loaded /error and got non-200 message",
                &mut goose.request,
                Some(headers),
                None,
            );
        }
    }
    Ok(())
}

// All tests in this file run against common endpoints.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<Mock> {
    vec![
        // First, set up INDEX_PATH, store in vector at INDEX_KEY.
        server.mock(|when, then| {
            when.method(GET).path(INDEX_PATH);
            then.status(200);
        }),
        // Next, set up ERROR_PATH, store in vector at ERROR_KEY.
        server.mock(|when, then| {
            when.method(GET).path(ERROR_PATH);
            then.status(503);
        }),
    ]
}

// Returns the appropriate scenario, start_transaction and stop_transaction needed to build these tests.
fn get_transactions() -> Scenario {
    scenario!("LoadTest")
        .register_transaction(transaction!(get_index))
        .register_transaction(transaction!(get_error))
}

// Helper to confirm all variations generate appropriate results.
fn validate_test(
    goose_metrics: GooseMetrics,
    mock_endpoints: &[Mock],
    test_type: &TestType,
    log_files: &LogFiles,
) {
    // Confirm that we loaded the mock endpoints. This confirms that we started
    // both users, which also verifies that hatch_rate was properly set.
    // Confirm that we loaded the mock endpoints.
    assert!(mock_endpoints[INDEX_KEY].hits() > 0);
    assert!(mock_endpoints[ERROR_KEY].hits() > 0);

    // Confirm that the test duration was correct.
    assert!(goose_metrics.duration == 2);

    match test_type {
        TestType::Debug => {
            // Debug file must exist.
            assert!(!log_files.debug_logs.is_empty());

            // Confirm the debug log files actually exist.
            let mut debug_file_lines = 0;
            for debug_file in log_files.debug_logs {
                assert!(std::path::Path::new(debug_file).exists());
                debug_file_lines += common::file_length(debug_file);
            }
            // Debug file must not be empty.
            assert!(debug_file_lines > 0);
        }
        TestType::Requests => {
            // Requests file must exist.
            assert!(!log_files.request_logs.is_empty());

            // Confirm the requests log files actually exist.
            let mut requests_file_lines = 0;
            for request_log in log_files.request_logs {
                assert!(std::path::Path::new(request_log).exists());
                requests_file_lines += common::file_length(request_log);
            }
            // Metrics file must not be empty.
            assert!(requests_file_lines > 0);
        }
        TestType::Transactions => {
            // Transaction log must exist.
            assert!(!log_files.transaction_logs.is_empty());

            // Confirm the transaction log files actually exist.
            let mut transactions_file_lines = 0;
            for transactions_file in log_files.transaction_logs {
                assert!(std::path::Path::new(transactions_file).exists());
                transactions_file_lines += common::file_length(transactions_file);
            }
            // Transaction file must not be empty.
            assert!(transactions_file_lines > 0);
        }
        TestType::Scenarios => {
            // Scenario log must exist.
            assert!(!log_files.scenario_logs.is_empty());

            // Confirm the scneario log files actually exist.
            let mut scenarios_file_lines = 0;
            for scenarios_file in log_files.scenario_logs {
                assert!(std::path::Path::new(scenarios_file).exists());
                scenarios_file_lines += common::file_length(scenarios_file);
            }
            // Scenario file must not be empty.
            assert!(scenarios_file_lines > 0);
        }
        TestType::Error => {
            // Error file must exist.
            assert!(!log_files.error_logs.is_empty());

            // Confirm the error log files actually exist.
            let mut error_file_lines = 0;
            for error_file in log_files.error_logs {
                assert!(std::path::Path::new(error_file).exists());
                error_file_lines += common::file_length(error_file);
            }
            // Error file must not be empty.
            assert!(error_file_lines > 0);
        }
        TestType::All => {
            // Debug file must exist.
            assert!(!log_files.debug_logs.is_empty());
            // Error file must exist.
            assert!(!log_files.error_logs.is_empty());
            // Requests file must exist.
            assert!(!log_files.request_logs.is_empty());
            // Transactions file must exist.
            assert!(!log_files.transaction_logs.is_empty());
            // Scenarios file must exist.
            assert!(!log_files.scenario_logs.is_empty());

            // Confirm the debug log files actually exist.
            let mut debug_file_lines = 0;
            for debug_log in log_files.debug_logs {
                assert!(std::path::Path::new(debug_log).exists());
                debug_file_lines += common::file_length(debug_log);
            }
            // Debug file must not be empty.
            assert!(debug_file_lines > 0);

            // Confirm the error log files actually exist.
            let mut error_file_lines = 0;
            for error_log in log_files.error_logs {
                assert!(std::path::Path::new(error_log).exists());
                error_file_lines += common::file_length(error_log);
            }
            // Error file must not be empty.
            assert!(error_file_lines > 0);

            // Confirm the requests log files actually exist.
            let mut requests_file_lines = 0;
            for request_log in log_files.request_logs {
                assert!(std::path::Path::new(request_log).exists());
                requests_file_lines += common::file_length(request_log);
            }
            // Requests file must not be empty.
            assert!(requests_file_lines > 0);

            // Confirm the transaction log files actually exist.
            let mut transactions_file_lines = 0;
            for transactions_log in log_files.transaction_logs {
                assert!(std::path::Path::new(transactions_log).exists());
                transactions_file_lines += common::file_length(transactions_log);
            }
            // Transaction file must not be empty.
            assert!(transactions_file_lines > 0);

            // Confirm the scenario log files actually exist.
            let mut scenarios_file_lines = 0;
            for scenarios_log in log_files.scenario_logs {
                assert!(std::path::Path::new(scenarios_log).exists());
                scenarios_file_lines += common::file_length(scenarios_log);
            }
            // Scenario file must not be empty.
            assert!(scenarios_file_lines > 0);
        }
    }
}

// Helper to run all standalone tests.
async fn run_standalone_test(test_type: TestType, format: &str) {
    let request_log = test_type.to_string() + "-request-log." + format;
    let transaction_log = test_type.to_string() + "-transaction-log." + format;
    let scenario_log = test_type.to_string() + "-scenario-log." + format;
    let debug_log = test_type.to_string() + "-debug-log." + format;
    let error_log = test_type.to_string() + "-error-log." + format;

    let server = MockServer::start();

    let mock_endpoints = setup_mock_server_endpoints(&server);

    let mut configuration_flags = match test_type {
        TestType::Debug => vec!["--debug-log", &debug_log, "--debug-format", format],
        TestType::Error => vec!["--error-log", &error_log, "--error-format", format],
        TestType::Requests => vec!["--request-log", &request_log, "--request-format", format],
        TestType::Transactions => vec![
            "--transaction-log",
            &transaction_log,
            "--transaction-format",
            format,
        ],
        TestType::Scenarios => vec!["--scenario-log", &scenario_log, "--scenario-format", format],
        TestType::All => vec![
            "--request-log",
            &request_log,
            "--request-format",
            format,
            "--transaction-log",
            &transaction_log,
            "--transaction-format",
            format,
            "--scenario-log",
            &scenario_log,
            "--scenario-format",
            format,
            "--error-log",
            &error_log,
            "--error-format",
            format,
            "--debug-log",
            &debug_log,
            "--debug-format",
            format,
        ],
    };
    configuration_flags.extend(vec!["--users", "4", "--hatch-rate", "4", "--run-time", "2"]);
    let configuration = common::build_configuration(&server, configuration_flags);

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(
        common::build_load_test(configuration, vec![get_transactions()], None, None),
        None,
    )
    .await;

    let log_files = LogFiles {
        request_logs: &[request_log.to_string()],
        transaction_logs: &[transaction_log.to_string()],
        scenario_logs: &[scenario_log.to_string()],
        error_logs: &[error_log.to_string()],
        debug_logs: &[debug_log.to_string()],
    };

    validate_test(goose_metrics, &mock_endpoints, &test_type, &log_files);

    common::cleanup_files(vec![
        &request_log,
        &transaction_log,
        &scenario_log,
        &error_log,
        &debug_log,
    ]);
}

// Helper to run all gaggle tests.
async fn run_gaggle_test(test_type: TestType, format: &str) {
    let requests_file = test_type.to_string() + "-gaggle-request-log." + format;
    let transactions_file = test_type.to_string() + "-gaggle-transaction-log." + format;
    let scenarios_file = test_type.to_string() + "-gaggle-scenario-log." + format;
    let error_file = test_type.to_string() + "-gaggle-error-log." + format;
    let debug_file = test_type.to_string() + "-gaggle-debug-log." + format;

    let server = MockServer::start();

    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Launch each worker in its own thread, storing the join handles.
    let mut worker_handles = Vec::new();
    let mut requests_files = Vec::new();
    let mut transactions_files = Vec::new();
    let mut scenarios_files = Vec::new();
    let mut error_files = Vec::new();
    let mut debug_files = Vec::new();
    for i in 0..EXPECT_WORKERS {
        // Name files different per-Worker thread.
        let worker_requests_file = requests_file.clone() + &i.to_string();
        let worker_transactions_file = transactions_file.clone() + &i.to_string();
        let worker_scenarios_file = scenarios_file.clone() + &i.to_string();
        let worker_error_file = error_file.clone() + &i.to_string();
        let worker_debug_file = debug_file.clone() + &i.to_string();
        // Store filenames to cleanup at end of test.
        requests_files.push(worker_requests_file.clone());
        transactions_files.push(worker_transactions_file.clone());
        scenarios_files.push(worker_scenarios_file.clone());
        error_files.push(worker_error_file.clone());
        debug_files.push(worker_debug_file.clone());
        // Build appropriate configuration.
        let worker_configuration_flags = match test_type {
            TestType::Debug => vec![
                "--worker",
                "--debug-log",
                &worker_debug_file,
                "--debug-format",
                format,
            ],
            TestType::Error => vec![
                "--worker",
                "--error-log",
                &worker_error_file,
                "--error-format",
                format,
            ],
            TestType::Requests => vec![
                "--worker",
                "--request-log",
                &worker_requests_file,
                "--request-format",
                format,
            ],
            TestType::Transactions => vec![
                "--worker",
                "--transaction-log",
                &worker_transactions_file,
                "--transaction-format",
                format,
            ],
            TestType::Scenarios => vec![
                "--worker",
                "--scenario-log",
                &worker_scenarios_file,
                "--scenario-format",
                format,
            ],
            TestType::All => vec![
                "--worker",
                "--request-log",
                &worker_requests_file,
                "--request-format",
                format,
                "--transaction-log",
                &worker_transactions_file,
                "--transaction-format",
                format,
                "--scenario-log",
                &worker_scenarios_file,
                "--scenario-format",
                format,
                "--error-log",
                &worker_error_file,
                "--error-format",
                format,
                "--debug-log",
                &worker_debug_file,
                "--debug-format",
                format,
            ],
        };
        let worker_configuration = common::build_configuration(&server, worker_configuration_flags);
        let worker_goose_attack = common::build_load_test(
            worker_configuration.clone(),
            vec![get_transactions()],
            None,
            None,
        );
        // Start worker instance of the load test.
        worker_handles.push(tokio::spawn(common::run_load_test(
            worker_goose_attack,
            None,
        )));
    }

    let manager_configuration = common::build_configuration(
        &server,
        vec![
            "--manager",
            "--expect-workers",
            &EXPECT_WORKERS.to_string(),
            "--users",
            "4",
            "--hatch-rate",
            "4",
            "--run-time",
            "2",
        ],
    );

    // Build the load test for the Manager.
    let manager_goose_attack =
        common::build_load_test(manager_configuration, vec![get_transactions()], None, None);

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(manager_goose_attack, Some(worker_handles)).await;

    let log_files = LogFiles {
        request_logs: &requests_files,
        transaction_logs: &transactions_files,
        scenario_logs: &scenarios_files,
        error_logs: &error_files,
        debug_logs: &debug_files,
    };

    validate_test(goose_metrics, &mock_endpoints, &test_type, &log_files);

    for file in requests_files {
        common::cleanup_files(vec![&file]);
    }
    for file in transactions_files {
        common::cleanup_files(vec![&file]);
    }
    for file in scenarios_files {
        common::cleanup_files(vec![&file]);
    }
    for file in error_files {
        common::cleanup_files(vec![&file]);
    }
    for file in debug_files {
        common::cleanup_files(vec![&file]);
    }
}

/* Request logs */

#[tokio::test]
// Enable json-formatted requests log.
async fn test_requests_logs_json() {
    run_standalone_test(TestType::Requests, "json").await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable json-formatted requests log, in Gaggle mode.
async fn test_requests_logs_json_gaggle() {
    run_gaggle_test(TestType::Requests, "json").await;
}

#[tokio::test]
// Enable csv-formatted requests log.
async fn test_requests_logs_csv() {
    run_standalone_test(TestType::Requests, "csv").await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable csv-formatted requests log, in Gaggle mode.
async fn test_requests_logs_csv_gaggle() {
    run_gaggle_test(TestType::Requests, "csv").await;
}

#[tokio::test]
// Enable raw-formatted requests log.
async fn test_requests_logs_raw() {
    run_standalone_test(TestType::Requests, "raw").await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable raw-formatted requests log, in Gaggle mode.
async fn test_requests_logs_raw_gaggle() {
    run_gaggle_test(TestType::Requests, "raw").await;
}

#[tokio::test]
// Enable pretty-formatted requests log.
async fn test_requests_logs_pretty() {
    run_standalone_test(TestType::Requests, "pretty").await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable pretty-formatted requests log, in Gaggle mode.
async fn test_requests_logs_pretty_gaggle() {
    run_gaggle_test(TestType::Requests, "pretty").await;
}

/* Transaction logs */

#[tokio::test]
// Enable json-formatted transaction log.
async fn test_transactions_logs_json() {
    run_standalone_test(TestType::Transactions, "json").await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable json-formatted transaction log, in Gaggle mode.
async fn test_transactions_logs_json_gaggle() {
    run_gaggle_test(TestType::Transactions, "json").await;
}

#[tokio::test]
// Enable csv-formatted transaction log.
async fn test_transactions_logs_csv() {
    run_standalone_test(TestType::Transactions, "csv").await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable csv-formatted transaction log, in Gaggle mode.
async fn test_transactions_logs_csv_gaggle() {
    run_gaggle_test(TestType::Transactions, "csv").await;
}

#[tokio::test]
// Enable raw-formatted transaction log.
async fn test_transactions_logs_raw() {
    run_standalone_test(TestType::Transactions, "raw").await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable raw-formatted transaction log, in Gaggle mode.
async fn test_transactions_logs_raw_gaggle() {
    run_gaggle_test(TestType::Transactions, "raw").await;
}

/* Scenario logs */

#[tokio::test]
// Enable json-formatted scenario log.
async fn test_scenarios_logs_json() {
    run_standalone_test(TestType::Scenarios, "json").await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable json-formatted scenario log, in Gaggle mode.
async fn test_scenarios_logs_json_gaggle() {
    run_gaggle_test(TestType::Scenarios, "json").await;
}

#[tokio::test]
// Enable csv-formatted scenario log.
async fn test_scenarios_logs_csv() {
    run_standalone_test(TestType::Scenarios, "csv").await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable csv-formatted scenario log, in Gaggle mode.
async fn test_scenarios_logs_csv_gaggle() {
    run_gaggle_test(TestType::Scenarios, "csv").await;
}

#[tokio::test]
// Enable raw-formatted scenario log.
async fn test_scenarios_logs_raw() {
    run_standalone_test(TestType::Scenarios, "raw").await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable raw-formatted scenario log, in Gaggle mode.
async fn test_scenarios_logs_raw_gaggle() {
    run_gaggle_test(TestType::Scenarios, "raw").await;
}

/* Error logs */

#[tokio::test]
// Enable raw-formatted error log.
async fn test_error_logs_raw() {
    run_standalone_test(TestType::Error, "raw").await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable raw-formatted error log, in Gaggle mode.
async fn test_error_logs_raw_gaggle() {
    run_gaggle_test(TestType::Error, "raw").await;
}

#[tokio::test]
// Enable json-formatted error log.
async fn test_error_logs_json() {
    run_standalone_test(TestType::Error, "json").await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable json-formatted error log, in Gaggle mode.
async fn test_error_logs_json_gaggle() {
    run_gaggle_test(TestType::Error, "json").await;
}

#[tokio::test]
// Enable csv-formatted error log.
async fn test_error_logs_csv() {
    run_standalone_test(TestType::Error, "csv").await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable csv-formatted error log, in Gaggle mode.
async fn test_error_logs_csv_gaggle() {
    run_gaggle_test(TestType::Error, "csv").await;
}

/* Debug logs */

#[tokio::test]
// Enable raw-formatted debug log.
async fn test_debug_logs_raw() {
    run_standalone_test(TestType::Debug, "raw").await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable raw-formatted debug log, in Gaggle mode.
async fn test_debug_logs_raw_gaggle() {
    run_gaggle_test(TestType::Debug, "raw").await;
}

#[tokio::test]
// Enable json-formatted debug log.
async fn test_debug_logs_json() {
    run_standalone_test(TestType::Debug, "json").await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable json-formatted debug log, in Gaggle mode.
async fn test_debug_logs_json_gaggle() {
    run_gaggle_test(TestType::Debug, "json").await;
}

#[tokio::test]
// Enable csv-formatted debug log.
async fn test_debug_logs_csv() {
    run_standalone_test(TestType::Debug, "csv").await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable csv-formatted debug log, in Gaggle mode.
async fn test_debug_logs_csv_gaggle() {
    run_gaggle_test(TestType::Debug, "csv").await;
}

/* All logs */

#[tokio::test]
// Enable raw-formatted logs.
async fn test_all_logs_raw() {
    run_standalone_test(TestType::All, "raw").await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable raw-formatted logs, in Gaggle mode.
async fn test_all_logs_raw_gaggle() {
    run_gaggle_test(TestType::All, "raw").await;
}

#[tokio::test]
// Enable pretty-formatted logs.
async fn test_all_logs_pretty() {
    run_standalone_test(TestType::All, "pretty").await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable pretty-formatted logs, in Gaggle mode.
async fn test_all_logs_pretty_gaggle() {
    run_gaggle_test(TestType::All, "pretty").await;
}

#[test]
fn test_csv_row_macro() {
    let row = goose::logger::format_csv_row!(1, '"', "hello , ");
    assert_eq!(r#"1,"""","hello , ""#, row);

    let row = goose::logger::format_csv_row!(format!("{:?}", (1, 2)), "你好", "A\nNew Day",);
    assert_eq!("\"(1, 2)\",你好,\"A\nNew Day\"", row);
}
