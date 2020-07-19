use httpmock::Method::GET;
use httpmock::{mock, with_mock_server};

mod common;

use goose::prelude::*;

const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about.html";
const STATS_LOG_FILE: &str = "throttle-stats.log";

pub async fn get_index(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

pub async fn get_about(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(ABOUT_PATH).await?;
    Ok(())
}

#[test]
#[with_mock_server]
fn test_throttle() {
    use std::io::{self, BufRead};

    let mock_index = mock(GET, INDEX_PATH).return_status(200).create();
    let mock_about = mock(GET, ABOUT_PATH).return_status(200).create();

    let mut throttle_requests = 25;
    let users = 5;
    let run_time = 3;

    let mut config = common::build_configuration();
    // Record all requests so we can confirm throttle is working.
    config.stats_log_file = STATS_LOG_FILE.to_string();
    config.no_stats = false;
    // Enable the throttle.
    config.throttle_requests = Some(throttle_requests);
    config.users = Some(users);
    // Start all users in half a second.
    config.hatch_rate = users;
    // Run for a few seconds to be sure throttle really works.
    config.run_time = run_time.to_string();
    crate::GooseAttack::initialize_with_config(config)
        .setup()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_about))
                .register_task(task!(get_index)),
        )
        .execute();

    let called_index = mock_index.times_called();
    let called_about = mock_about.times_called();

    // Confirm that we loaded the mock endpoints.
    assert_ne!(called_index, 0);
    assert_ne!(called_about, 0);

    let test1_lines: usize;
    if let Ok(stats_log) = std::fs::File::open(std::path::Path::new(STATS_LOG_FILE)) {
        test1_lines = io::BufReader::new(stats_log).lines().count();
    } else {
        test1_lines = 0;
    }

    // Requests are made while GooseUsers are hatched, and then for run_time seconds.
    assert!(test1_lines <= (run_time + 1) * throttle_requests);

    // Increase the throttle and run a second load test, so we can compare the difference
    // and confirm the throttle is actually working.
    throttle_requests *= 5;

    let mut config = common::build_configuration();
    // Record all requests so we can confirm throttle is working.
    config.stats_log_file = STATS_LOG_FILE.to_string();
    config.no_stats = false;
    // Enable the throttle.
    config.throttle_requests = Some(throttle_requests);
    config.users = Some(users);
    // Start all users in half a second.
    config.hatch_rate = users;
    config.run_time = run_time.to_string();
    crate::GooseAttack::initialize_with_config(config)
        .setup()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_about))
                .register_task(task!(get_index)),
        )
        .execute();

    let called_index = mock_index.times_called();
    let called_about = mock_about.times_called();

    // Confirm that we loaded the mock endpoints.
    assert_ne!(called_index, 0);
    assert_ne!(called_about, 0);

    let lines: usize;
    if let Ok(stats_log) = std::fs::File::open(std::path::Path::new(STATS_LOG_FILE)) {
        lines = io::BufReader::new(stats_log).lines().count();
    } else {
        lines = 0;
    }

    // Requests are made while GooseUsers are hatched, and then for run_time seconds.
    assert!(lines <= (run_time + 1) * throttle_requests);
    // Verify the second load test generated more than 4x the load of the first test.
    assert!(lines > test1_lines * 4);
}
