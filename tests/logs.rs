use httpmock::Method::GET;
use httpmock::{Mock, MockServer};

mod common;

use goose::prelude::*;

const INDEX_PATH: &str = "/";
const ERROR_PATH: &str = "/error";

pub async fn get_index(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

pub async fn get_error(user: &GooseUser) -> GooseTaskResult {
    let mut goose = user.get(ERROR_PATH).await?;

    if let Ok(r) = goose.response {
        let headers = &r.headers().clone();
        if r.text().await.is_err() {
            return user.set_failure(
                "there was an error",
                &mut goose.request,
                Some(headers),
                None,
            );
        }
    }
    Ok(())
}

fn cleanup_files(metrics_file: &str, debug_file: &str) {
    if std::path::Path::new(metrics_file).exists() {
        std::fs::remove_file(metrics_file).expect("failed to delete metrics log file");
    }
    if std::path::Path::new(debug_file).exists() {
        std::fs::remove_file(debug_file).expect("failed to delete debug log file");
    }
}

#[test]
fn test_metrics_logs_json() {
    const METRICS_FILE: &str = "metrics-json.log";
    const DEBUG_FILE: &str = "debug-json.log";

    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server);

    let mut config = common::build_configuration(&server);
    config.metrics_file = METRICS_FILE.to_string();
    config.no_metrics = false;
    let goose_metrics = crate::GooseAttack::initialize_with_config(config)
        .unwrap()
        .setup()
        .unwrap()
        .register_taskset(taskset!("LoadTest").register_task(task!(get_index)))
        .execute()
        .unwrap();

    // Confirm that we loaded the mock endpoints.
    assert!(index.times_called() > 0);

    // Confirm that the test duration was correct.
    assert!(goose_metrics.duration == 1);

    // Confirm only the metrics log file exists.
    assert!(std::path::Path::new(METRICS_FILE).exists());
    assert!(!std::path::Path::new(DEBUG_FILE).exists());

    cleanup_files(METRICS_FILE, DEBUG_FILE);
}

#[test]
fn test_metrics_logs_csv() {
    const METRICS_FILE: &str = "metrics-csv.log";
    const DEBUG_FILE: &str = "debug-csv.log";

    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server);

    let mut config = common::build_configuration(&server);
    config.metrics_file = METRICS_FILE.to_string();
    config.metrics_format = "csv".to_string();
    config.no_metrics = false;
    let _goose_metrics = crate::GooseAttack::initialize_with_config(config)
        .unwrap()
        .setup()
        .unwrap()
        .register_taskset(taskset!("LoadTest").register_task(task!(get_index)))
        .execute()
        .unwrap();

    // Confirm that we loaded the mock endpoints.
    assert!(index.times_called() > 0);

    // Confirm only the metrics log file exists.
    assert!(std::path::Path::new(METRICS_FILE).exists());
    assert!(!std::path::Path::new(DEBUG_FILE).exists());

    cleanup_files(METRICS_FILE, DEBUG_FILE);
}

#[test]
fn test_metrics_logs_raw() {
    const METRICS_FILE: &str = "metrics-raw.log";
    const DEBUG_FILE: &str = "debug-raw.log";

    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server);

    let mut config = common::build_configuration(&server);
    config.metrics_file = METRICS_FILE.to_string();
    config.metrics_format = "raw".to_string();
    config.no_metrics = false;
    let _goose_metrics = crate::GooseAttack::initialize_with_config(config)
        .unwrap()
        .setup()
        .unwrap()
        .register_taskset(taskset!("LoadTest").register_task(task!(get_index)))
        .execute()
        .unwrap();

    // Confirm that we loaded the mock endpoints.
    assert!(index.times_called() > 0);

    // Confirm only the metrics log file exists.
    assert!(std::path::Path::new(METRICS_FILE).exists());
    assert!(!std::path::Path::new(DEBUG_FILE).exists());

    cleanup_files(METRICS_FILE, DEBUG_FILE);
}

#[test]
fn test_debug_logs_raw() {
    const METRICS_FILE: &str = "metrics-raw2.log";
    const DEBUG_FILE: &str = "debug-raw2.log";

    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server);
    let error = Mock::new()
        .expect_method(GET)
        .expect_path(ERROR_PATH)
        .return_status(503)
        .create_on(&server);

    let mut config = common::build_configuration(&server);
    config.debug_file = DEBUG_FILE.to_string();
    config.debug_format = "raw".to_string();
    let _goose_metrics = crate::GooseAttack::initialize_with_config(config)
        .unwrap()
        .setup()
        .unwrap()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_index))
                .register_task(task!(get_error)),
        )
        .execute()
        .unwrap();

    // Confirm that we loaded the mock endpoints.
    assert!(index.times_called() > 0);
    assert!(error.times_called() > 0);

    // Confirm only the debug log file exists.
    assert!(std::path::Path::new(DEBUG_FILE).exists());
    assert!(!std::path::Path::new(METRICS_FILE).exists());

    cleanup_files(METRICS_FILE, DEBUG_FILE);
}

#[test]
fn test_debug_logs_json() {
    const METRICS_FILE: &str = "metrics-json2.log";
    const DEBUG_FILE: &str = "debug-json2.log";

    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server);
    let error = Mock::new()
        .expect_method(GET)
        .expect_path(ERROR_PATH)
        .return_status(503)
        .create_on(&server);

    let mut config = common::build_configuration(&server);
    config.debug_file = DEBUG_FILE.to_string();
    let _goose_metrics = crate::GooseAttack::initialize_with_config(config)
        .unwrap()
        .setup()
        .unwrap()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_index))
                .register_task(task!(get_error)),
        )
        .execute()
        .unwrap();

    // Confirm that we loaded the mock endpoints.
    assert!(index.times_called() > 0);
    assert!(error.times_called() > 0);

    // Confirm only the debug log file exists.
    assert!(!std::path::Path::new(METRICS_FILE).exists());
    assert!(std::path::Path::new(DEBUG_FILE).exists());

    cleanup_files(METRICS_FILE, DEBUG_FILE);
}

#[test]
fn test_metrics_and_debug_logs() {
    const METRICS_FILE: &str = "metrics-both.log";
    const DEBUG_FILE: &str = "debug-both.log";

    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server);
    let error = Mock::new()
        .expect_method(GET)
        .expect_path(ERROR_PATH)
        .return_status(503)
        .create_on(&server);

    let mut config = common::build_configuration(&server);
    config.metrics_file = METRICS_FILE.to_string();
    config.metrics_format = "raw".to_string();
    config.no_metrics = false;
    config.debug_file = DEBUG_FILE.to_string();
    let _goose_metrics = crate::GooseAttack::initialize_with_config(config)
        .unwrap()
        .setup()
        .unwrap()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_index))
                .register_task(task!(get_error)),
        )
        .execute()
        .unwrap();

    // Confirm that we loaded the mock endpoints.
    assert!(index.times_called() > 0);
    assert!(error.times_called() > 0);

    // Confirm both the metrics and debug logs exist.
    assert!(std::path::Path::new(METRICS_FILE).exists());
    assert!(std::path::Path::new(DEBUG_FILE).exists());

    cleanup_files(METRICS_FILE, DEBUG_FILE);
}
