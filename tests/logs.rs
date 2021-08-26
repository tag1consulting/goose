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
    // Test with tasks log enabled.
    Tasks,
    // Test with error log enabled.
    Error,
    // Test with debug log enabled.
    Debug,
    // Test with all log files enabled.
    All,
}

struct LogFiles<'a> {
    request_logs: &'a [String],
    task_logs: &'a [String],
    error_logs: &'a [String],
    debug_logs: &'a [String],
}

// Implement fmt::Display for TestType to uniquely name the log files generated
// by each test. This is necessary as tests run in parallel.
impl fmt::Display for TestType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let printable = match *self {
            TestType::Requests => "requests",
            TestType::Tasks => "tasks",
            TestType::Error => "error",
            TestType::Debug => "debug",
            TestType::All => "all",
        };
        write!(f, "{}", printable)
    }
}

// Test task.
pub async fn get_index(user: &mut GooseUser) -> GooseTaskResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// Test task.
pub async fn get_error(user: &mut GooseUser) -> GooseTaskResult {
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

// Returns the appropriate taskset, start_task and stop_task needed to build these tests.
fn get_tasks() -> GooseTaskSet {
    taskset!("LoadTest")
        .register_task(task!(get_index))
        .register_task(task!(get_error))
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
        TestType::Tasks => {
            // Tasks file must exist.
            assert!(!log_files.task_logs.is_empty());

            // Confirm the tasks log files actually exist.
            let mut tasks_file_lines = 0;
            for tasks_file in log_files.task_logs {
                assert!(std::path::Path::new(tasks_file).exists());
                tasks_file_lines += common::file_length(tasks_file);
            }
            // Tasks file must not be empty.
            assert!(tasks_file_lines > 0);
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
            // Tasks file must exist.
            assert!(!log_files.task_logs.is_empty());

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

            // Confirm the tasks log files actually exist.
            let mut tasks_file_lines = 0;
            for tasks_log in log_files.task_logs {
                assert!(std::path::Path::new(tasks_log).exists());
                tasks_file_lines += common::file_length(tasks_log);
            }
            // Task file must not be empty.
            assert!(tasks_file_lines > 0);
        }
    }
}

// Helper to run all standalone tests.
fn run_standalone_test(test_type: TestType, format: &str) {
    let request_log = test_type.to_string() + "-request-log." + format;
    let task_log = test_type.to_string() + "-task-log." + format;
    let debug_log = test_type.to_string() + "-debug-log." + format;
    let error_log = test_type.to_string() + "-error-log." + format;

    let server = MockServer::start();

    let mock_endpoints = setup_mock_server_endpoints(&server);

    let mut configuration_flags = match test_type {
        TestType::Debug => vec!["--debug-log", &debug_log, "--debug-format", format],
        TestType::Error => vec!["--error-log", &error_log, "--error-format", format],
        TestType::Requests => vec!["--request-log", &request_log, "--request-format", format],
        TestType::Tasks => vec!["--task-log", &task_log, "--task-format", format],
        TestType::All => vec![
            "--request-log",
            &request_log,
            "--request-format",
            format,
            "--task-log",
            &task_log,
            "--task-format",
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
        common::build_load_test(configuration, &get_tasks(), None, None),
        None,
    );

    let log_files = LogFiles {
        request_logs: &[request_log.to_string()],
        task_logs: &[task_log.to_string()],
        error_logs: &[error_log.to_string()],
        debug_logs: &[debug_log.to_string()],
    };

    validate_test(goose_metrics, &mock_endpoints, &test_type, &log_files);

    common::cleanup_files(vec![&request_log, &task_log, &error_log, &debug_log]);
}

// Helper to run all gaggle tests.
fn run_gaggle_test(test_type: TestType, format: &str) {
    let requests_file = test_type.to_string() + "-gaggle-request-log." + format;
    let tasks_file = test_type.to_string() + "-gaggle-task-log." + format;
    let error_file = test_type.to_string() + "-gaggle-error-log." + format;
    let debug_file = test_type.to_string() + "-gaggle-debug-log." + format;

    let server = MockServer::start();

    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Launch each worker in its own thread, storing the join handles.
    let mut worker_handles = Vec::new();
    let mut requests_files = Vec::new();
    let mut tasks_files = Vec::new();
    let mut error_files = Vec::new();
    let mut debug_files = Vec::new();
    for i in 0..EXPECT_WORKERS {
        // Name files different per-Worker thread.
        let worker_requests_file = requests_file.clone() + &i.to_string();
        let worker_tasks_file = tasks_file.clone() + &i.to_string();
        let worker_error_file = error_file.clone() + &i.to_string();
        let worker_debug_file = debug_file.clone() + &i.to_string();
        // Store filenames to cleanup at end of test.
        requests_files.push(worker_requests_file.clone());
        tasks_files.push(worker_tasks_file.clone());
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
            TestType::Tasks => vec![
                "--worker",
                "--task-log",
                &worker_tasks_file,
                "--task-format",
                format,
            ],
            TestType::All => vec![
                "--worker",
                "--request-log",
                &worker_requests_file,
                "--request-format",
                format,
                "--task-log",
                &worker_tasks_file,
                "--task-format",
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
        let worker_goose_attack =
            common::build_load_test(worker_configuration.clone(), &get_tasks(), None, None);
        // Start worker instance of the load test.
        worker_handles.push(std::thread::spawn(move || {
            // Run the load test as configured.
            common::run_load_test(worker_goose_attack, None);
        }));
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
        common::build_load_test(manager_configuration, &get_tasks(), None, None);

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(manager_goose_attack, Some(worker_handles));

    let log_files = LogFiles {
        request_logs: &requests_files,
        task_logs: &tasks_files,
        error_logs: &error_files,
        debug_logs: &debug_files,
    };

    validate_test(goose_metrics, &mock_endpoints, &test_type, &log_files);

    for file in requests_files {
        common::cleanup_files(vec![&file]);
    }
    for file in tasks_files {
        common::cleanup_files(vec![&file]);
    }
    for file in error_files {
        common::cleanup_files(vec![&file]);
    }
    for file in debug_files {
        common::cleanup_files(vec![&file]);
    }
}

#[test]
// Enable json-formatted requests log.
fn test_requests_logs_json() {
    run_standalone_test(TestType::Requests, "json");
}

#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable json-formatted requests log, in Gaggle mode.
fn test_requests_logs_json_gaggle() {
    run_gaggle_test(TestType::Requests, "json");
}

#[test]
// Enable csv-formatted requests log.
fn test_requests_logs_csv() {
    run_standalone_test(TestType::Requests, "csv");
}

#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable csv-formatted requests log, in Gaggle mode.
fn test_requests_logs_csv_gaggle() {
    run_gaggle_test(TestType::Requests, "csv");
}

#[test]
// Enable raw-formatted requests log.
fn test_requests_logs_raw() {
    run_standalone_test(TestType::Requests, "raw");
}

#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable raw-formatted requests log, in Gaggle mode.
fn test_requests_logs_raw_gaggle() {
    run_gaggle_test(TestType::Requests, "raw");
}

#[test]
// Enable pretty-formatted requests log.
fn test_requests_logs_pretty() {
    run_standalone_test(TestType::Requests, "pretty");
}

#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable pretty-formatted requests log, in Gaggle mode.
fn test_requests_logs_pretty_gaggle() {
    run_gaggle_test(TestType::Requests, "pretty");
}

#[test]
// Enable json-formatted tasks log.
fn test_tasks_logs_json() {
    run_standalone_test(TestType::Tasks, "json");
}

#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable json-formatted tasks log, in Gaggle mode.
fn test_tasks_logs_json_gaggle() {
    run_gaggle_test(TestType::Tasks, "json");
}

#[test]
// Enable csv-formatted tasks log.
fn test_tasks_logs_csv() {
    run_standalone_test(TestType::Tasks, "csv");
}

#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable csv-formatted tasks log, in Gaggle mode.
fn test_tasks_logs_csv_gaggle() {
    run_gaggle_test(TestType::Tasks, "csv");
}

#[test]
// Enable raw-formatted tasks log.
fn test_tasks_logs_raw() {
    run_standalone_test(TestType::Tasks, "raw");
}

#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable raw-formatted tasks log, in Gaggle mode.
fn test_tasks_logs_raw_gaggle() {
    run_gaggle_test(TestType::Tasks, "raw");
}

#[test]
// Enable raw-formatted error log.
fn test_error_logs_raw() {
    run_standalone_test(TestType::Error, "raw");
}

#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable raw-formatted error log, in Gaggle mode.
fn test_error_logs_raw_gaggle() {
    run_gaggle_test(TestType::Error, "raw");
}

#[test]
// Enable json-formatted error log.
fn test_error_logs_json() {
    run_standalone_test(TestType::Error, "json");
}

#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable json-formatted error log, in Gaggle mode.
fn test_error_logs_json_gaggle() {
    run_gaggle_test(TestType::Error, "json");
}

#[test]
// Enable csv-formatted error log.
fn test_error_logs_csv() {
    run_standalone_test(TestType::Error, "csv");
}

#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable csv-formatted error log, in Gaggle mode.
fn test_error_logs_csv_gaggle() {
    run_gaggle_test(TestType::Error, "csv");
}

#[test]
// Enable raw-formatted debug log.
fn test_debug_logs_raw() {
    run_standalone_test(TestType::Debug, "raw");
}

#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable raw-formatted debug log, in Gaggle mode.
fn test_debug_logs_raw_gaggle() {
    run_gaggle_test(TestType::Debug, "raw");
}

#[test]
// Enable json-formatted debug log.
fn test_debug_logs_json() {
    run_standalone_test(TestType::Debug, "json");
}

#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable json-formatted debug log, in Gaggle mode.
fn test_debug_logs_json_gaggle() {
    run_gaggle_test(TestType::Debug, "json");
}

#[test]
// Enable csv-formatted debug log.
fn test_debug_logs_csv() {
    run_standalone_test(TestType::Debug, "csv");
}

#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable csv-formatted debug log, in Gaggle mode.
fn test_debug_logs_csv_gaggle() {
    run_gaggle_test(TestType::Debug, "csv");
}

#[test]
// Enable raw-formatted logs.
fn test_all_logs_raw() {
    run_standalone_test(TestType::All, "raw");
}

#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable raw-formatted logs, in Gaggle mode.
fn test_all_logs_raw_gaggle() {
    run_gaggle_test(TestType::All, "raw");
}

#[test]
// Enable pretty-formatted logs.
fn test_all_logs_pretty() {
    run_standalone_test(TestType::All, "pretty");
}

#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable pretty-formatted logs, in Gaggle mode.
fn test_all_logs_pretty_gaggle() {
    run_gaggle_test(TestType::All, "pretty");
}
