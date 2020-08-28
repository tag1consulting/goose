use httpmock::Method::GET;
use httpmock::{Mock, MockRef, MockServer};
use std::io::{self, BufRead};

mod common;

use goose::prelude::*;

const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about.html";

const USERS: usize = 2;
const RUN_TIME: usize = 1;
const HATCH_RATE: usize = 4;
const LOG_LEVEL: usize = 0;
const METRICS_FILE: &str = "metrics-test.log";

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

    // Be sure there's no files left over from an earlier test.
    cleanup_file(&metrics_file);

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
        .execute()
        .unwrap();

    validate_test(goose_metrics, index, about, &metrics_file);
}

#[test]
/// Load test confirming that Goose respects CLI options.
fn test_no_defaults() {
    // Multiple tests run together, so set a unique name.
    let metrics_file = "nodefaults-".to_string() + METRICS_FILE;

    // Be sure there's no files left over from an earlier test.
    cleanup_file(&metrics_file);

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

    config.no_reset_metrics = true;
    let goose_metrics = crate::GooseAttack::initialize_with_config(config)
        .setup()
        .unwrap()
        .register_taskset(taskset!("Index").register_task(task!(get_index)))
        .register_taskset(taskset!("About").register_task(task!(get_about)))
        .execute()
        .unwrap();

    validate_test(goose_metrics, index, about, &metrics_file);
}

// Helper to delete test artifact, if existing.
fn cleanup_file(file: &str) {
    if std::path::Path::new(file).exists() {
        std::fs::remove_file(file).expect("failed to remove file");
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
fn validate_test(goose_metrics: GooseMetrics, index: MockRef, about: MockRef, metrics_file: &str) {
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
    assert!(std::path::Path::new(metrics_file).exists());
    assert!(file_length(metrics_file) == index.times_called() + about.times_called());

    // Verify that the test ran as long as it was supposed to.
    assert!(goose_metrics.duration == RUN_TIME);

    // Cleanup from test.
    cleanup_file(metrics_file);
}
