use httpmock::Method::GET;
use httpmock::{Mock, MockServer};
use std::io::{self, BufRead};

mod common;

use goose::prelude::*;

const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about.html";
const METRICS_FILE: &str = "throttle-metrics.log";

const THROTTLE_REQUESTS: usize = 25;
const USERS: usize = 5;
const RUN_TIME: usize = 3;
const EXPECT_WORKERS: usize = 2;

pub async fn get_index(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

pub async fn get_about(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(ABOUT_PATH).await?;
    Ok(())
}

#[test]
// Verify that the throttle limits the number of requests per second, and that increasing
// the throttle increases the number of requests per second.
fn test_throttle() {
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

    let config = common::build_configuration(
        &server,
        vec![
            // Record all requests so we can confirm throttle is working.
            "--metrics-file",
            METRICS_FILE,
            // Enable the throttle.
            "--throttle-requests",
            &THROTTLE_REQUESTS.to_string(),
            "--users",
            &USERS.to_string(),
            "--hatch-rate",
            &USERS.to_string(),
            // Run for a few seconds to be sure throttle really works.
            "--run-time",
            &RUN_TIME.to_string(),
        ],
    );
    let _goose_metrics = crate::GooseAttack::initialize_with_config(config)
        .unwrap()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_about))
                .register_task(task!(get_index)),
        )
        .execute()
        .unwrap();

    // Confirm that we loaded the mock endpoints.
    assert!(index.times_called() > 0);
    assert!(about.times_called() > 0);

    let test1_lines: usize;
    if let Ok(metrics_log) = std::fs::File::open(std::path::Path::new(METRICS_FILE)) {
        test1_lines = io::BufReader::new(metrics_log).lines().count();
    } else {
        test1_lines = 0;
    }

    // Requests are made while GooseUsers are hatched, and then for RUN_TIME seconds.
    assert!(test1_lines <= (RUN_TIME + 1) * THROTTLE_REQUESTS);

    // Cleanup log file.
    std::fs::remove_file(METRICS_FILE).expect("failed to delete metrics log file");

    // Increase the throttle and run a second load test, so we can compare the difference
    // and confirm the throttle is actually working.
    let increased_throttle = THROTTLE_REQUESTS * 5;

    let config = common::build_configuration(
        &server,
        vec![
            // Record all requests so we can confirm throttle is working.
            "--metrics-file",
            METRICS_FILE,
            "--throttle-requests",
            &increased_throttle.to_string(),
            "--users",
            &USERS.to_string(),
            "--hatch-rate",
            &USERS.to_string(),
            "--run-time",
            &RUN_TIME.to_string(),
        ],
    );
    let _goose_metrics = crate::GooseAttack::initialize_with_config(config)
        .unwrap()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_about))
                .register_task(task!(get_index)),
        )
        .execute()
        .unwrap();

    // Confirm that we loaded the mock endpoints.
    assert!(index.times_called() > 0);
    assert!(about.times_called() > 0);

    let lines: usize;
    if let Ok(metrics_log) = std::fs::File::open(std::path::Path::new(METRICS_FILE)) {
        lines = io::BufReader::new(metrics_log).lines().count();
    } else {
        lines = 0;
    }

    // Requests are made while GooseUsers are hatched, and then for RUN_TIME seconds.
    assert!(lines <= (RUN_TIME + 1) * increased_throttle);
    // Verify the second load test generated more than 4x the load of the first test.
    assert!(lines > test1_lines * 4);
    // Verify the second load test generated less than 6x the load of the first test.
    assert!(lines < test1_lines * 6);

    // Cleanup log file.
    std::fs::remove_file(METRICS_FILE).expect("failed to delete metrics log file");
}

#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Verify that the throttle limits the number of requests per second even when running
// in Gaggle distributed load test, and that increasing the throttle increases the
// number of requests per second across the Gaggle.
fn test_throttle_gaggle() {
    use std::thread;

    // Multiple tests run together, so set a unique name.
    let metrics_file = "gaggle-".to_string() + METRICS_FILE;

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

    // Launch workers in their own threads, storing the thread handle.
    let mut worker_handles = Vec::new();
    // Each worker has the same identical configuration.
    let mut worker_configuration = common::build_configuration(
        &server,
        vec![
            "--worker",
            // Enable the throttle.
            "--throttle-requests",
            &THROTTLE_REQUESTS.to_string(),
        ],
    );

    // Unset options set in common.rs as they can't be set on the Worker.
    worker_configuration.users = None;
    worker_configuration.run_time = "".to_string();
    worker_configuration.hatch_rate = None;

    for _ in 0..EXPECT_WORKERS {
        let configuration = worker_configuration.clone();
        // Start worker instance of the load test.
        worker_handles.push(thread::spawn(move || {
            let _goose_metrics = crate::GooseAttack::initialize_with_config(configuration)
                .unwrap()
                .register_taskset(
                    taskset!("LoadTest")
                        .register_task(task!(get_index))
                        .register_task(task!(get_about)),
                )
                .execute()
                .unwrap();
        }));
    }

    // Start manager instance in current thread and run a distributed load test.
    let manager_configuration = common::build_configuration(
        &server,
        vec![
            "--manager",
            "--expect-workers",
            &EXPECT_WORKERS.to_string(),
            // Record all requests so we can confirm throttle is working.
            "--metrics-file",
            &metrics_file.to_string(),
            "--users",
            &USERS.to_string(),
            "--hatch-rate",
            &USERS.to_string(),
            // Run for a few seconds to be sure throttle really works.
            "--run-time",
            &RUN_TIME.to_string(),
        ],
    );

    let _goose_metrics = crate::GooseAttack::initialize_with_config(manager_configuration)
        .unwrap()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_index))
                .register_task(task!(get_about)),
        )
        .execute()
        .unwrap();

    // Confirm that we loaded the mock endpoints.
    assert!(index.times_called() > 0);
    assert!(about.times_called() > 0);

    let test1_lines: usize;
    if let Ok(metrics_log) = std::fs::File::open(std::path::Path::new(&metrics_file)) {
        test1_lines = io::BufReader::new(metrics_log).lines().count();
    } else {
        test1_lines = 0;
    }

    // Requests are made while GooseUsers are hatched, and then for run_time seconds.
    assert!(test1_lines <= (RUN_TIME + 1) * THROTTLE_REQUESTS);

    // Cleanup log file.
    std::fs::remove_file(&metrics_file).expect("failed to delete metrics log file");

    // Increase the throttle and run a second load test, so we can compare the difference
    // and confirm the throttle is actually working.
    let increased_throttle = THROTTLE_REQUESTS * 5;

    // Clear vector to launch workers again in their own threads, storing the thread handle.
    worker_handles.clear();
    // Each worker has the same identical configuration.
    let mut worker_configuration = common::build_configuration(
        &server,
        vec![
            "--worker",
            // Enable the throttle.
            "--throttle-requests",
            &increased_throttle.to_string(),
        ],
    );

    // Unset options set in common.rs as they can't be set on the Worker.
    worker_configuration.users = None;
    worker_configuration.run_time = "".to_string();
    worker_configuration.hatch_rate = None;

    for _ in 0..EXPECT_WORKERS {
        let configuration = worker_configuration.clone();
        // Start worker instance of the load test.
        worker_handles.push(thread::spawn(move || {
            let _goose_metrics = crate::GooseAttack::initialize_with_config(configuration)
                .unwrap()
                .register_taskset(
                    taskset!("LoadTest")
                        .register_task(task!(get_index))
                        .register_task(task!(get_about)),
                )
                .execute()
                .unwrap();
        }));
    }

    // Start manager instance in current thread and run a distributed load test.
    let manager_configuration = common::build_configuration(
        &server,
        vec![
            "--manager",
            "--expect-workers",
            &EXPECT_WORKERS.to_string(),
            // Record all requests so we can confirm throttle is working.
            "--metrics-file",
            &metrics_file.to_string(),
            "--users",
            &USERS.to_string(),
            "--hatch-rate",
            &USERS.to_string(),
            // Run for a few seconds to be sure throttle really works.
            "--run-time",
            &RUN_TIME.to_string(),
        ],
    );

    let _goose_metrics = crate::GooseAttack::initialize_with_config(manager_configuration)
        .unwrap()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_index))
                .register_task(task!(get_about)),
        )
        .execute()
        .unwrap();

    // Confirm that we loaded the mock endpoints.
    assert!(index.times_called() > 0);
    assert!(about.times_called() > 0);

    let lines: usize;
    if let Ok(metrics_log) = std::fs::File::open(std::path::Path::new(&metrics_file)) {
        lines = io::BufReader::new(metrics_log).lines().count();
    } else {
        lines = 0;
    }

    // Requests are made while GooseUsers are hatched, and then for run_time seconds.
    assert!(lines <= (RUN_TIME + 1) * increased_throttle);
    // Verify the second load test generated more than 4x the load of the first test.
    assert!(lines > test1_lines * 4);
    // Verify the second load test generated less than 6x the load of the first test.
    assert!(lines < test1_lines * 6);

    // Cleanup log file.
    std::fs::remove_file(&metrics_file).expect("failed to delete metrics log file");
}
