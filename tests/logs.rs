use httpmock::Method::GET;
use httpmock::{mock, with_mock_server};

mod common;

use goose::prelude::*;

const INDEX_PATH: &str = "/";
const ERROR_PATH: &str = "/error";

const STATS_LOG_FILE: &str = "stats.log";
const DEBUG_LOG_FILE: &str = "debug.log";

pub async fn get_index(user: &GooseUser) {
    let _goose = user.get(INDEX_PATH).await;
}

pub async fn get_error(user: &GooseUser) {
    let goose = user.get(ERROR_PATH).await;
    if let Some(response) = goose.response {
        if let Ok(r) = response {
            let headers = &r.headers().clone();
            match r.text().await {
                Ok(_) => {}
                Err(_) => {
                    user.log_debug(
                        "there was an error",
                        Some(goose.request),
                        Some(headers),
                        None,
                    );
                }
            }
        }
    }
}

fn cleanup_files() {
    let _ = std::fs::remove_file(STATS_LOG_FILE);
    let _ = std::fs::remove_file(DEBUG_LOG_FILE);
}

#[test]
#[with_mock_server]
fn test_stat_logs_json() {
    cleanup_files();

    let mock_index = mock(GET, INDEX_PATH).return_status(200).create();

    let mut config = common::build_configuration();
    config.stats_log_file = STATS_LOG_FILE.to_string();
    config.no_stats = false;
    crate::GooseAttack::initialize_with_config(config)
        .setup()
        .register_taskset(taskset!("LoadTest").register_task(task!(get_index)))
        .execute();

    let called_index = mock_index.times_called();

    // Confirm that we loaded the mock endpoints.
    assert_ne!(called_index, 0);

    // Confirm only the stats log file exists.
    let stats_log_exists = std::path::Path::new(STATS_LOG_FILE).exists();
    let debug_log_exists = std::path::Path::new(DEBUG_LOG_FILE).exists();
    assert_eq!(stats_log_exists, true);
    assert_eq!(debug_log_exists, false);
}

#[test]
#[with_mock_server]
fn test_stat_logs_csv() {
    cleanup_files();

    let mock_index = mock(GET, INDEX_PATH).return_status(200).create();

    let mut config = common::build_configuration();
    config.stats_log_file = STATS_LOG_FILE.to_string();
    config.stats_log_format = "csv".to_string();
    config.no_stats = false;
    crate::GooseAttack::initialize_with_config(config)
        .setup()
        .register_taskset(taskset!("LoadTest").register_task(task!(get_index)))
        .execute();

    let called_index = mock_index.times_called();

    // Confirm that we loaded the mock endpoints.
    assert_ne!(called_index, 0);

    // Confirm only the stats log file exists.
    let stats_log_exists = std::path::Path::new(STATS_LOG_FILE).exists();
    let debug_log_exists = std::path::Path::new(DEBUG_LOG_FILE).exists();
    assert_eq!(stats_log_exists, true);
    assert_eq!(debug_log_exists, false);
}

#[test]
#[with_mock_server]
fn test_stat_logs_raw() {
    cleanup_files();

    let mock_index = mock(GET, INDEX_PATH).return_status(200).create();

    let mut config = common::build_configuration();
    config.stats_log_file = STATS_LOG_FILE.to_string();
    config.stats_log_format = "raw".to_string();
    config.no_stats = false;
    crate::GooseAttack::initialize_with_config(config)
        .setup()
        .register_taskset(taskset!("LoadTest").register_task(task!(get_index)))
        .execute();

    let called_index = mock_index.times_called();

    // Confirm that we loaded the mock endpoints.
    assert_ne!(called_index, 0);

    // Confirm only the stats log file exists.
    let stats_log_exists = std::path::Path::new(STATS_LOG_FILE).exists();
    let debug_log_exists = std::path::Path::new(DEBUG_LOG_FILE).exists();
    assert_eq!(stats_log_exists, true);
    assert_eq!(debug_log_exists, false);
}

#[test]
#[with_mock_server]
fn test_debug_logs_raw() {
    cleanup_files();

    let mock_index = mock(GET, INDEX_PATH).return_status(200).create();
    let mock_error = mock(GET, ERROR_PATH).return_status(503).create();

    let mut config = common::build_configuration();
    config.debug_log_file = DEBUG_LOG_FILE.to_string();
    config.debug_log_format = "raw".to_string();
    crate::GooseAttack::initialize_with_config(config)
        .setup()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_index))
                .register_task(task!(get_error)),
        )
        .execute();

    let called_index = mock_index.times_called();
    let called_error = mock_error.times_called();

    // Confirm that we loaded the mock endpoints.
    assert_ne!(called_index, 0);
    assert_ne!(called_error, 0);

    // Confirm only the debug log file exists.
    let stats_log_exists = std::path::Path::new(STATS_LOG_FILE).exists();
    let debug_log_exists = std::path::Path::new(DEBUG_LOG_FILE).exists();
    assert_eq!(stats_log_exists, false);
    assert_eq!(debug_log_exists, true);
}

#[test]
#[with_mock_server]
fn test_debug_logs_json() {
    cleanup_files();

    let mock_index = mock(GET, INDEX_PATH).return_status(200).create();
    let mock_error = mock(GET, ERROR_PATH).return_status(503).create();

    let mut config = common::build_configuration();
    config.debug_log_file = DEBUG_LOG_FILE.to_string();
    crate::GooseAttack::initialize_with_config(config)
        .setup()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_index))
                .register_task(task!(get_error)),
        )
        .execute();

    let called_index = mock_index.times_called();
    let called_error = mock_error.times_called();

    // Confirm that we loaded the mock endpoints.
    assert_ne!(called_index, 0);
    assert_ne!(called_error, 0);

    // Confirm only the debug log file exists.
    let stats_log_exists = std::path::Path::new(STATS_LOG_FILE).exists();
    let debug_log_exists = std::path::Path::new(DEBUG_LOG_FILE).exists();
    assert_eq!(stats_log_exists, false);
    assert_eq!(debug_log_exists, true);
}

#[test]
#[with_mock_server]
fn test_stats_and_debug_logs() {
    cleanup_files();

    let mock_index = mock(GET, INDEX_PATH).return_status(200).create();
    let mock_error = mock(GET, ERROR_PATH).return_status(503).create();

    let mut config = common::build_configuration();
    config.stats_log_file = STATS_LOG_FILE.to_string();
    config.stats_log_format = "raw".to_string();
    config.no_stats = false;
    config.debug_log_file = DEBUG_LOG_FILE.to_string();
    crate::GooseAttack::initialize_with_config(config)
        .setup()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_index))
                .register_task(task!(get_error)),
        )
        .execute();

    let called_index = mock_index.times_called();
    let called_error = mock_error.times_called();

    // Confirm that we loaded the mock endpoints.
    assert_ne!(called_index, 0);
    assert_ne!(called_error, 0);

    // Confirm both the stats and debug logs exist.
    let stats_log_exists = std::path::Path::new(STATS_LOG_FILE).exists();
    let debug_log_exists = std::path::Path::new(DEBUG_LOG_FILE).exists();
    assert_eq!(stats_log_exists, true);
    assert_eq!(debug_log_exists, true);
}
