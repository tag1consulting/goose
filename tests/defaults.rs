use httpmock::Method::GET;
use httpmock::{Mock, MockRef, MockServer};
use std::io::{self, BufRead};
use std::thread;

mod common;

use goose::prelude::*;

const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about.html";

const USERS: usize = 3;
const RUN_TIME: usize = 2;
const HATCH_RATE: usize = 10;
const LOG_LEVEL: usize = 0;
const METRICS_FILE: &str = "metrics-test.log";
const DEBUG_FILE: &str = "debug-test.log";
const LOG_FORMAT: &str = "raw";
const THROTTLE_REQUESTS: usize = 10;

// Has tests:
// - GooseDefault::Host
// - GooseDefault::MetricsFile
// - GooseDefault::MetricsFormat
// - GooseDefault::DebugFile
// - GooseDefault::DebugFormat
// - GooseDefault::HatchRate
// - GooseDefault::RunTime
// - GooseDefault::ThrottleRequests
// - GooseDefault::Users
// - GooseDefault::Manager
// - GooseDefault::ExpectWorkers
// - GooseDefault::ManagerBindHost
// - GooseDefault::ManagerBindPort
// - GooseDefault::Worker
// - GooseDefault::ManagerHost
// - GooseDefault::ManagerPort

// Can't be tested:
// - GooseDefault::LogFile (logger can only be configured once)
// - GooseDefault::Verbose (logger can only be configured once)
// - GooseDefault::LogLevel (can't validate due to logger limitation)

// Doesn't have tests yet:
// - GooseDefault::OnlySummary
// - GooseDefault::NoResetMetrics
// - GooseDefault::NoMetrics
// - GooseDefault::NoTaskMetrics
// - GooseDefault::StatusCodes
// - GooseDefault::StickyFollow
// - GooseDefault::NoHashCheck

pub async fn get_index(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

pub async fn get_about(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(ABOUT_PATH).await?;
    Ok(())
}

// Note: we're not testing log_file as tests run in threads, and only one
// logger can be configured globally.

#[test]
/// Load test confirming that Goose respects configured defaults.
fn test_defaults() {
    // Multiple tests run together, so set a unique name.
    let metrics_file = "defaults-".to_string() + METRICS_FILE;
    let debug_file = "defaults-".to_string() + DEBUG_FILE;

    // Be sure there's no files left over from an earlier test.
    cleanup_files(vec![&metrics_file, &debug_file]);

    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server);
    let about = Mock::new()
        .expect_method(GET)
        .expect_path(ABOUT_PATH)
        .return_status(200)
        .create_on(&server);

    let mut config = common::build_configuration(&server);
    config.no_metrics = false;
    config.users = None;
    config.run_time = "".to_string();
    config.hatch_rate = 0;
    config.metrics_format = "".to_string();
    config.debug_format = "".to_string();

    config.no_reset_metrics = true;
    let host = std::mem::take(&mut config.host);
    let goose_metrics = crate::GooseAttack::initialize_with_config(config)
        .setup()
        .unwrap()
        .register_taskset(taskset!("Index").register_task(task!(get_index)))
        .register_taskset(taskset!("About").register_task(task!(get_about)))
        // Start two users, required to run both TaskSets.
        .set_default(GooseDefault::Host, host.as_str())
        .set_default(GooseDefault::Users, USERS)
        .set_default(GooseDefault::RunTime, RUN_TIME)
        .set_default(GooseDefault::HatchRate, HATCH_RATE)
        .set_default(GooseDefault::LogLevel, LOG_LEVEL)
        .set_default(GooseDefault::MetricsFile, metrics_file.as_str())
        .set_default(GooseDefault::MetricsFormat, LOG_FORMAT)
        .set_default(GooseDefault::DebugFile, debug_file.as_str())
        .set_default(GooseDefault::DebugFormat, LOG_FORMAT)
        .set_default(GooseDefault::ThrottleRequests, THROTTLE_REQUESTS)
        .execute()
        .unwrap();

    validate_test(goose_metrics, index, about, &[metrics_file], &[debug_file]);
}

#[test]
/// Load test confirming that Goose respects CLI options.
fn test_no_defaults() {
    // Multiple tests run together, so set a unique name.
    let metrics_file = "nodefaults-".to_string() + METRICS_FILE;
    let debug_file = "nodefaults-".to_string() + DEBUG_FILE;

    // Be sure there's no files left over from an earlier test.
    cleanup_files(vec![&metrics_file, &debug_file]);

    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server);
    let about = Mock::new()
        .expect_method(GET)
        .expect_path(ABOUT_PATH)
        .return_status(200)
        .create_on(&server);

    let mut config = common::build_configuration(&server);
    config.no_metrics = false;
    config.users = Some(USERS);
    config.run_time = RUN_TIME.to_string();
    config.hatch_rate = HATCH_RATE;
    config.log_level = LOG_LEVEL as u8;
    config.metrics_file = metrics_file.to_string();
    config.metrics_format = LOG_FORMAT.to_string();
    config.debug_file = debug_file.to_string();
    config.debug_format = LOG_FORMAT.to_string();
    config.throttle_requests = Some(THROTTLE_REQUESTS);

    config.no_reset_metrics = true;
    let goose_metrics = crate::GooseAttack::initialize_with_config(config)
        .setup()
        .unwrap()
        .register_taskset(taskset!("Index").register_task(task!(get_index)))
        .register_taskset(taskset!("About").register_task(task!(get_about)))
        .execute()
        .unwrap();

    validate_test(goose_metrics, index, about, &[metrics_file], &[debug_file]);
}

#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
/// Load test confirming that Goose respects configured gaggle-related defaults.
fn test_gaggle_defaults() {
    // Multiple tests run together, so set a unique name.
    let metrics_file = "gaggle-".to_string() + METRICS_FILE;
    let debug_file = "gaggle-".to_string() + DEBUG_FILE;

    // Be sure there's no files left over from an earlier test.
    for i in 0..USERS {
        let file = metrics_file.to_string() + &i.to_string();
        cleanup_files(vec![&file]);
        let file = debug_file.to_string() + &i.to_string();
        cleanup_files(vec![&file]);
    }

    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server);
    let about = Mock::new()
        .expect_method(GET)
        .expect_path(ABOUT_PATH)
        .return_status(200)
        .create_on(&server);

    const HOST: &str = "127.0.0.1";
    const PORT: usize = 9988;

    let mut configuration = common::build_configuration(&server);
    let host = std::mem::take(&mut configuration.host);
    configuration.no_metrics = false;
    configuration.no_task_metrics = false;
    configuration.users = None;
    configuration.run_time = "".to_string();
    configuration.hatch_rate = 0;
    configuration.metrics_format = "".to_string();
    configuration.debug_format = "".to_string();

    // Launch workers in their own threads, storing the thread handle.
    let mut worker_handles = Vec::new();
    for i in 0..USERS {
        let worker_configuration = configuration.clone();
        let worker_host = host.clone();
        let worker_metrics_file = metrics_file.clone() + &i.to_string();
        let worker_debug_file = debug_file.clone() + &i.to_string();
        //  //let worker_metrics_file = metrics_file.clone();
        worker_handles.push(thread::spawn(move || {
            let _ = crate::GooseAttack::initialize_with_config(worker_configuration)
                .setup()
                .unwrap()
                .register_taskset(taskset!("Index").register_task(task!(get_index)))
                .register_taskset(taskset!("About").register_task(task!(get_about)))
                // Start two users, required to run both TaskSets.
                .set_default(GooseDefault::Host, worker_host.as_str())
                .set_default(GooseDefault::Users, USERS)
                .set_default(GooseDefault::RunTime, RUN_TIME)
                .set_default(GooseDefault::HatchRate, HATCH_RATE)
                .set_default(GooseDefault::ThrottleRequests, THROTTLE_REQUESTS)
                .set_default(GooseDefault::DebugFile, worker_debug_file.as_str())
                .set_default(GooseDefault::DebugFormat, LOG_FORMAT)
                .set_default(GooseDefault::MetricsFile, worker_metrics_file.as_str())
                .set_default(GooseDefault::MetricsFormat, LOG_FORMAT)
                // Worker configuration using defaults instead of run-time options.
                .set_default(GooseDefault::Worker, true)
                .set_default(GooseDefault::ManagerHost, HOST)
                .set_default(GooseDefault::ManagerPort, PORT)
                .execute()
                .unwrap();
        }));
    }

    // @TODO: configure through defaults:
    configuration.no_reset_metrics = true;

    // Start manager instance in current thread and run a distributed load test.
    let goose_metrics = crate::GooseAttack::initialize_with_config(configuration)
        .setup()
        .unwrap()
        .register_taskset(taskset!("Index").register_task(task!(get_index)))
        .register_taskset(taskset!("About").register_task(task!(get_about)))
        // Start two users, required to run both TaskSets.
        .set_default(GooseDefault::Host, host.as_str())
        .set_default(GooseDefault::Users, USERS)
        .set_default(GooseDefault::RunTime, RUN_TIME)
        .set_default(GooseDefault::HatchRate, HATCH_RATE)
        // Manager configuration using defaults instead of run-time options.
        .set_default(GooseDefault::Manager, true)
        .set_default(GooseDefault::ExpectWorkers, USERS)
        .set_default(GooseDefault::ManagerBindHost, HOST)
        .set_default(GooseDefault::ManagerBindPort, PORT)
        .execute()
        .unwrap();

    // Wait for both worker threads to finish and exit.
    for worker_handle in worker_handles {
        let _ = worker_handle.join();
    }

    let mut metrics_files: Vec<String> = vec![];
    let mut debug_files: Vec<String> = vec![];
    for i in 0..USERS {
        let file = metrics_file.to_string() + &i.to_string();
        metrics_files.push(file);
        let file = debug_file.to_string() + &i.to_string();
        debug_files.push(file);
    }
    validate_test(goose_metrics, index, about, &metrics_files, &debug_files);
}

// Helper to delete test artifact, if existing.
fn cleanup_files(files: Vec<&str>) {
    for file in files {
        if std::path::Path::new(file).exists() {
            std::fs::remove_file(file).expect("failed to remove file");
        }
    }
}

// Helper to count the number of lines in a test artifact.
fn file_length(file_name: &str) -> usize {
    if let Ok(file) = std::fs::File::open(std::path::Path::new(file_name)) {
        io::BufReader::new(file).lines().count()
    } else {
        0
    }
}

/// Helper that validates test results are the same regardless of if setting
/// run-time options, or defaults.
fn validate_test(
    goose_metrics: GooseMetrics,
    index: MockRef,
    about: MockRef,
    metrics_files: &[String],
    debug_files: &[String],
) {
    // Confirm that we loaded the mock endpoints. This confirms that we started
    // both users, which also verifies that hatch_rate was properly set.
    assert!(index.times_called() > 0);
    assert!(about.times_called() > 0);

    let index_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", INDEX_PATH))
        .unwrap();
    let about_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", ABOUT_PATH))
        .unwrap();

    // Confirm that Goose and the server saw the same number of page loads.
    assert!(index_metrics.response_time_counter == index.times_called());
    assert!(index_metrics.success_count == index.times_called());
    assert!(index_metrics.fail_count == 0);
    assert!(about_metrics.response_time_counter == about.times_called());
    assert!(about_metrics.success_count == about.times_called());
    assert!(about_metrics.fail_count == 0);

    // Verify that Goose started the correct number of users.
    assert!(goose_metrics.users == USERS);

    // Verify that the metrics file was created and has the correct number of lines.
    let mut metrics_lines = 0;
    for metrics_file in metrics_files {
        assert!(std::path::Path::new(metrics_file).exists());
        metrics_lines += file_length(metrics_file);
    }
    assert!(metrics_lines == index.times_called() + about.times_called());

    // Verify that the debug file was created and is empty.
    for debug_file in debug_files {
        assert!(std::path::Path::new(debug_file).exists());
        assert!(file_length(debug_file) == 0);
    }

    // Requests are made while GooseUsers are hatched, and then for run_time seconds.
    // Verify that the test ran as long as it was supposed to.
    assert!(goose_metrics.duration == RUN_TIME);

    // Be sure there were no more requests made than the throttle should allow.
    // In the case of a gaggle, there's multiple processes running with the same
    // throttle.
    let number_of_processes = metrics_files.len();
    assert!(metrics_lines <= (RUN_TIME + 1) * THROTTLE_REQUESTS * number_of_processes);

    // Cleanup from test.
    for file in metrics_files {
        cleanup_files(vec![file]);
    }
    for file in debug_files {
        cleanup_files(vec![file]);
    }
}
