use httpmock::Method::GET;
use httpmock::{Mock, MockRef, MockServer};
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
    // Test with metrics log enabled.
    Metrics,
    // Test with debug log enabled.
    Debug,
    // Test with metrics log and debug log both enabled.
    MetricsAndDebug,
}

// Implement fmt::Display for TestType to uniquely name the log files generated
// by each test. This is necessary as tests run in parallel.
impl fmt::Display for TestType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let printable = match *self {
            TestType::Metrics => "metrics",
            TestType::Debug => "debug",
            TestType::MetricsAndDebug => "metrics-and-debug",
        };
        write!(f, "{}", printable)
    }
}

// Test task.
pub async fn get_index(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// Test task.
pub async fn get_error(user: &GooseUser) -> GooseTaskResult {
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
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<MockRef> {
    let mut endpoints: Vec<MockRef> = Vec::new();

    // First, set up INDEX_PATH, store in vector at INDEX_KEY.
    endpoints.push(
        Mock::new()
            .expect_method(GET)
            .expect_path(INDEX_PATH)
            .return_status(200)
            .create_on(&server),
    );
    // Next, set up ERROR_PATH, store in vector at ERROR_KEY.
    endpoints.push(
        Mock::new()
            .expect_method(GET)
            .expect_path(ERROR_PATH)
            .return_status(503)
            .create_on(&server),
    );

    endpoints
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
    mock_endpoints: &[MockRef],
    test_type: &TestType,
    metrics_files: &[String],
    debug_files: &[String],
) {
    // Confirm that we loaded the mock endpoints. This confirms that we started
    // both users, which also verifies that hatch_rate was properly set.
    // Confirm that we loaded the mock endpoints.
    assert!(mock_endpoints[INDEX_KEY].times_called() > 0);
    assert!(mock_endpoints[ERROR_KEY].times_called() > 0);

    // Confirm that the test duration was correct.
    assert!(goose_metrics.duration == 2);

    match test_type {
        TestType::Debug => {
            // Debug file must exist.
            assert!(!debug_files.is_empty());

            // Confirm the debug log files actually exist.
            let mut debug_file_lines = 0;
            for debug_file in debug_files {
                assert!(std::path::Path::new(debug_file).exists());
                debug_file_lines += common::file_length(debug_file);
                println!(
                    "debug_file_lines: {}, debug_file: {}",
                    debug_file_lines, debug_file
                );
            }
            // Debug file must not be empty.
            assert!(debug_file_lines > 0);
        }
        TestType::Metrics => {
            // Metrics file must exist.
            assert!(!metrics_files.is_empty());

            // Confirm the metrics log files actually exist.
            let mut metrics_file_lines = 0;
            for metrics_file in metrics_files {
                assert!(std::path::Path::new(metrics_file).exists());
                metrics_file_lines += common::file_length(metrics_file);
                println!(
                    "metrics_file_lines: {}, metrics_file: {}",
                    metrics_file_lines, metrics_file
                );
            }
            // Metrics file must not be empty.
            assert!(metrics_file_lines > 0);
        }
        TestType::MetricsAndDebug => {
            // Debug file must exist.
            assert!(!debug_files.is_empty());
            // Metrics file must exist.
            assert!(!metrics_files.is_empty());

            // Confirm the debug log files actually exist.
            let mut debug_file_lines = 0;
            for debug_file in debug_files {
                assert!(std::path::Path::new(debug_file).exists());
                debug_file_lines += common::file_length(debug_file);
                println!(
                    "debug_file_lines: {}, debug_file: {}",
                    debug_file_lines, debug_file
                );
            }
            // Debug file must not be empty.
            assert!(debug_file_lines > 0);

            // Confirm the metrics log files actually exist.
            let mut metrics_file_lines = 0;
            for metrics_file in metrics_files {
                assert!(std::path::Path::new(metrics_file).exists());
                metrics_file_lines += common::file_length(metrics_file);
            }
            // Metrics file must not be empty.
            assert!(metrics_file_lines > 0);
        }
    }
}

// Helper to run all standalone tests.
fn run_standalone_test(test_type: TestType, format: &str) {
    let metrics_file = test_type.to_string() + "-metrics-log." + format;
    let debug_file = test_type.to_string() + "-debug-log." + format;

    let server = MockServer::start();

    let mock_endpoints = setup_mock_server_endpoints(&server);

    let mut configuration_flags = match test_type {
        TestType::Debug => vec!["--debug-file", &debug_file, "--debug-format", format],
        TestType::Metrics => vec!["--metrics-file", &metrics_file, "--metrics-format", format],
        TestType::MetricsAndDebug => vec![
            "--metrics-file",
            &metrics_file,
            "--metrics-format",
            format,
            "--debug-file",
            &debug_file,
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

    validate_test(
        goose_metrics,
        &mock_endpoints,
        &test_type,
        &[metrics_file.to_string()],
        &[debug_file.to_string()],
    );

    common::cleanup_files(vec![&metrics_file, &debug_file]);
}

// Helper to run all gaggle tests.
fn run_gaggle_test(test_type: TestType, format: &str) {
    let metrics_file = test_type.to_string() + "-gaggle-metrics-log." + format;
    let debug_file = test_type.to_string() + "-gaggle-debug-log." + format;

    let server = MockServer::start();

    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Launch each worker in its own thread, storing the join handles.
    let mut worker_handles = Vec::new();
    let mut metrics_files = Vec::new();
    let mut debug_files = Vec::new();
    for i in 0..EXPECT_WORKERS {
        // Name files different per-Worker thread.
        let worker_metrics_file = metrics_file.clone() + &i.to_string();
        let worker_debug_file = debug_file.clone() + &i.to_string();
        // Store filenames to cleanup at end of test.
        metrics_files.push(worker_metrics_file.clone());
        debug_files.push(worker_debug_file.clone());
        // Build appropriate configuration.
        let worker_configuration_flags = match test_type {
            TestType::Debug => vec![
                "--worker",
                "--debug-file",
                &worker_debug_file,
                "--debug-format",
                format,
                "--verbose",
            ],
            TestType::Metrics => vec![
                "--worker",
                "--metrics-file",
                &worker_metrics_file,
                "--metrics-format",
                format,
                "--verbose",
            ],
            TestType::MetricsAndDebug => vec![
                "--worker",
                "--metrics-file",
                &worker_metrics_file,
                "--metrics-format",
                format,
                "--debug-file",
                &worker_debug_file,
                "--debug-format",
                format,
                "--verbose",
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
            "--verbose",
        ],
    );

    // Build the load test for the Manager.
    let manager_goose_attack =
        common::build_load_test(manager_configuration, &get_tasks(), None, None);

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(manager_goose_attack, Some(worker_handles));

    validate_test(
        goose_metrics,
        &mock_endpoints,
        &test_type,
        &metrics_files,
        &debug_files,
    );

    for file in metrics_files {
        common::cleanup_files(vec![&file]);
    }
    for file in debug_files {
        common::cleanup_files(vec![&file]);
    }
}

#[test]
// Enable json-formatted metrics log.
fn test_metrics_logs_json() {
    run_standalone_test(TestType::Metrics, "json");
}

#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable json-formatted metrics log, in Gaggle mode.
fn test_metrics_logs_json_gaggle() {
    run_gaggle_test(TestType::Metrics, "json");
}

#[test]
// Enable csv-formatted metrics log.
fn test_metrics_logs_csv() {
    run_standalone_test(TestType::Metrics, "csv");
}

#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable csv-formatted metrics log, in Gaggle mode.
fn test_metrics_logs_csv_gaggle() {
    run_gaggle_test(TestType::Metrics, "csv");
}

#[test]
// Enable raw-formatted metrics log.
fn test_metrics_logs_raw() {
    run_standalone_test(TestType::Metrics, "raw");
}

#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable raw-formatted metrics log, in Gaggle mode.
fn test_metrics_logs_raw_gaggle() {
    run_gaggle_test(TestType::Metrics, "raw");
}

#[test]
// Enable raw-formatted debug log.
fn test_debug_logs_raw() {
    run_standalone_test(TestType::Debug, "raw");
}

/* @TODO: FIXME: https://github.com/tag1consulting/goose/issues/194
#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable raw-formatted debug log, in Gaggle mode.
fn test_debug_logs_raw_gaggle() {
    run_gaggle_test(TestType::Debug, "raw");
}
*/

#[test]
// Enable json-formatted debug log.
fn test_debug_logs_json() {
    run_standalone_test(TestType::Debug, "json");
}

/* @TODO: FIXME: https://github.com/tag1consulting/goose/issues/194
#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable json-formatted debug log, in Gaggle mode.
fn test_debug_logs_json_gaggle() {
    run_gaggle_test(TestType::Debug, "json");
}
*/

#[test]
// Enable raw-formatted debug log and metrics log.
fn test_metrics_and_debug_logs() {
    run_standalone_test(TestType::MetricsAndDebug, "raw");
}

/* @TODO: FIXME: https://github.com/tag1consulting/goose/issues/194
#[test]
#[serial]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Enable raw-formatted debug log and metrics log, in Gaggle mode.
fn test_metrics_and_debug_logs_gaggle() {
    run_gaggle_test(TestType::MetricsAndDebug, "raw");
}
*/
