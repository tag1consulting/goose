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

fn cleanup_files(stats_log_file: &str, debug_log_file: &str) {
    if std::path::Path::new(stats_log_file).exists() {
        std::fs::remove_file(stats_log_file).expect("failed to delete stats log file");
    }
    if std::path::Path::new(debug_log_file).exists() {
        std::fs::remove_file(debug_log_file).expect("failed to delete debug log file");
    }
}

#[test]
fn test_stats_logs_json() {
    const STATS_LOG_FILE: &str = "stats-json.log";
    const DEBUG_LOG_FILE: &str = "debug-json.log";

    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server);

    let mut config = common::build_configuration(&server);
    config.stats_log_file = STATS_LOG_FILE.to_string();
    config.no_stats = false;
    let goose_stats = crate::GooseAttack::initialize_with_config(config)
        .setup()
        .unwrap()
        .register_taskset(taskset!("LoadTest").register_task(task!(get_index)))
        .execute()
        .unwrap();

    // Confirm that we loaded the mock endpoints.
    assert!(index.times_called() > 0);

    // Confirm that the test duration was correct.
    assert!(goose_stats.duration == 1);

    // Confirm only the stats log file exists.
    assert!(std::path::Path::new(STATS_LOG_FILE).exists());
    assert!(!std::path::Path::new(DEBUG_LOG_FILE).exists());

    cleanup_files(STATS_LOG_FILE, DEBUG_LOG_FILE);
}

#[test]
fn test_stats_logs_csv() {
    const STATS_LOG_FILE: &str = "stats-csv.log";
    const DEBUG_LOG_FILE: &str = "debug-csv.log";

    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server);

    let mut config = common::build_configuration(&server);
    config.stats_log_file = STATS_LOG_FILE.to_string();
    config.stats_log_format = "csv".to_string();
    config.no_stats = false;
    let _goose_stats = crate::GooseAttack::initialize_with_config(config)
        .setup()
        .unwrap()
        .register_taskset(taskset!("LoadTest").register_task(task!(get_index)))
        .execute()
        .unwrap();

    // Confirm that we loaded the mock endpoints.
    assert!(index.times_called() > 0);

    // Confirm only the stats log file exists.
    assert!(std::path::Path::new(STATS_LOG_FILE).exists());
    assert!(!std::path::Path::new(DEBUG_LOG_FILE).exists());

    cleanup_files(STATS_LOG_FILE, DEBUG_LOG_FILE);
}

#[test]
fn test_stats_logs_raw() {
    const STATS_LOG_FILE: &str = "stats-raw.log";
    const DEBUG_LOG_FILE: &str = "debug-raw.log";

    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server);

    let mut config = common::build_configuration(&server);
    config.stats_log_file = STATS_LOG_FILE.to_string();
    config.stats_log_format = "raw".to_string();
    config.no_stats = false;
    let _goose_stats = crate::GooseAttack::initialize_with_config(config)
        .setup()
        .unwrap()
        .register_taskset(taskset!("LoadTest").register_task(task!(get_index)))
        .execute()
        .unwrap();

    // Confirm that we loaded the mock endpoints.
    assert!(index.times_called() > 0);

    // Confirm only the stats log file exists.
    assert!(std::path::Path::new(STATS_LOG_FILE).exists());
    assert!(!std::path::Path::new(DEBUG_LOG_FILE).exists());

    cleanup_files(STATS_LOG_FILE, DEBUG_LOG_FILE);
}

#[test]
fn test_debug_logs_raw() {
    const STATS_LOG_FILE: &str = "stats-raw2.log";
    const DEBUG_LOG_FILE: &str = "debug-raw2.log";

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
    config.debug_log_file = DEBUG_LOG_FILE.to_string();
    config.debug_log_format = "raw".to_string();
    let _goose_stats = crate::GooseAttack::initialize_with_config(config)
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
    assert!(std::path::Path::new(DEBUG_LOG_FILE).exists());
    assert!(!std::path::Path::new(STATS_LOG_FILE).exists());

    cleanup_files(STATS_LOG_FILE, DEBUG_LOG_FILE);
}

#[test]
fn test_debug_logs_json() {
    const STATS_LOG_FILE: &str = "stats-json2.log";
    const DEBUG_LOG_FILE: &str = "debug-json2.log";

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
    config.debug_log_file = DEBUG_LOG_FILE.to_string();
    let _goose_stats = crate::GooseAttack::initialize_with_config(config)
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
    assert!(!std::path::Path::new(STATS_LOG_FILE).exists());
    assert!(std::path::Path::new(DEBUG_LOG_FILE).exists());

    cleanup_files(STATS_LOG_FILE, DEBUG_LOG_FILE);
}

#[test]
fn test_stats_and_debug_logs() {
    const STATS_LOG_FILE: &str = "stats-both.log";
    const DEBUG_LOG_FILE: &str = "debug-both.log";

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
    config.stats_log_file = STATS_LOG_FILE.to_string();
    config.stats_log_format = "raw".to_string();
    config.no_stats = false;
    config.debug_log_file = DEBUG_LOG_FILE.to_string();
    let _goose_stats = crate::GooseAttack::initialize_with_config(config)
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

    // Confirm both the stats and debug logs exist.
    assert!(std::path::Path::new(STATS_LOG_FILE).exists());
    assert!(std::path::Path::new(DEBUG_LOG_FILE).exists());

    cleanup_files(STATS_LOG_FILE, DEBUG_LOG_FILE);
}
