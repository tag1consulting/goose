use httpmock::{Method::GET, Mock, MockServer};
use serial_test::serial;

mod common;

use goose::config::GooseConfiguration;
use goose::prelude::*;

// Paths used in load tests performed during these tests.
const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about.html";

// Indexes to the above paths.
const INDEX_KEY: usize = 0;
const ABOUT_KEY: usize = 1;

// Load test configuration.
const REQUEST_LOG: &str = "throttle-metrics.log";
const THROTTLE_REQUESTS: usize = 25;
const USERS: usize = 5;
const RUN_TIME: usize = 3;
const EXPECT_WORKERS: usize = 2;

// Test transaction.
pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// Test transaction.
pub async fn get_about(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(ABOUT_PATH).await?;
    Ok(())
}

// All tests in this file run against common endpoints.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<Mock> {
    vec![
        // First set up INDEX_PATH, store in vector at INDEX_KEY.
        server.mock(|when, then| {
            when.method(GET).path(INDEX_PATH);
            then.status(200);
        }),
        // Next set up ABOUT_PATH, store in vector at ABOUT_KEY.
        server.mock(|when, then| {
            when.method(GET).path(ABOUT_PATH);
            then.status(200);
        }),
    ]
}

// Build appropriate configuration for these tests.
fn common_build_configuration(
    server: &MockServer,
    request_log: &str,
    throttle_requests: usize,
    users: usize,
    run_time: usize,
    worker: Option<bool>,
    manager: Option<usize>,
) -> GooseConfiguration {
    if let Some(expect_workers) = manager {
        common::build_configuration(
            server,
            vec![
                "--manager",
                "--expect-workers",
                &expect_workers.to_string(),
                "--users",
                &users.to_string(),
                "--hatch-rate",
                &users.to_string(),
                // Run the load test long enough to confirm the throttle is working correctly.
                "--run-time",
                &run_time.to_string(),
            ],
        )
    } else if worker.is_some() {
        common::build_configuration(
            server,
            vec![
                "--worker",
                // Limit the maximum requests per second.
                "--throttle-requests",
                &throttle_requests.to_string(),
                // Write requests to file to confirm throttle is working.
                "--request-log",
                request_log,
            ],
        )
    } else {
        common::build_configuration(
            server,
            vec![
                "--users",
                &users.to_string(),
                "--hatch-rate",
                &users.to_string(),
                // Run the load test long enough to confirm the throttle is working correctly.
                "--run-time",
                &run_time.to_string(),
                // Limit the maximum requests per second.
                "--throttle-requests",
                &throttle_requests.to_string(),
                // Write requests to file to confirm throttle is working.
                "--request-log",
                request_log,
            ],
        )
    }
}

// Helper to confirm all variations generate appropriate results.
fn validate_test(
    mock_endpoints: &[Mock],
    request_logs: &[String],
    throttle_value: usize,
    previous_requests_file_lines: Option<usize>,
) -> usize {
    // Verify that the metrics file was created and has the correct number of lines.
    let mut current_requests_file_lines = 0;
    for request_log in request_logs {
        assert!(std::path::Path::new(request_log).exists());
        current_requests_file_lines += common::file_length(request_log);
    }

    // Confirm that we loaded the mock endpoints.
    assert!(mock_endpoints[INDEX_KEY].hits() > 0);
    assert!(mock_endpoints[ABOUT_KEY].hits() > 0);

    // Requests are made while GooseUsers are hatched, and then for RUN_TIME seconds.
    assert!(current_requests_file_lines <= (RUN_TIME + 1) * throttle_value);

    if let Some(previous_lines) = previous_requests_file_lines {
        // Verify the second load test generated more than 4x the load of the first test.
        assert!(current_requests_file_lines > previous_lines * 4);
        // Verify the second load test generated less than 6x the load of the first test.
        assert!(current_requests_file_lines < previous_lines * 6);
    }

    // Cleanup log file.
    for request_log in request_logs {
        std::fs::remove_file(request_log).expect("failed to delete metrics log file");
    }

    // Return the number of lines in the current metrics file, allowing comparisons between
    // multiple tests with different throttle values.
    current_requests_file_lines
}

// Returns the appropriate scenario needed to build these tests.
fn get_transactions() -> Scenario {
    scenario!("LoadTest")
        .register_transaction(transaction!(get_index))
        .register_transaction(transaction!(get_about))
}

#[tokio::test]
#[serial]
// Enable throttle to confirm it limits the number of request per second.
// Increase the throttle and confirm it increases the number of requests
// per second.
async fn test_throttle() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build configuration.
    let configuration = common_build_configuration(
        &server,
        REQUEST_LOG,
        THROTTLE_REQUESTS,
        USERS,
        RUN_TIME,
        None,
        None,
    );

    // Run the Goose Attack.
    common::run_load_test(
        common::build_load_test(configuration, vec![get_transactions()], None, None),
        None,
    )
    .await;

    // Confirm that the load test was actually throttled.
    let test1_lines = validate_test(
        &mock_endpoints,
        &[REQUEST_LOG.to_string()],
        THROTTLE_REQUESTS,
        None,
    );

    // Increase the throttle and run a second load test, so we can compare the difference
    // and confirm the throttle is actually working.
    let increased_throttle = THROTTLE_REQUESTS * 5;

    // Build a new configuration.
    let configuration = common_build_configuration(
        &server,
        REQUEST_LOG,
        increased_throttle,
        USERS,
        RUN_TIME,
        None,
        None,
    );

    // Run the Goose Attack.
    common::run_load_test(
        common::build_load_test(configuration, vec![get_transactions()], None, None),
        None,
    )
    .await;

    // Confirm that the load test was actually throttled, at an increased rate.
    let _ = validate_test(
        &mock_endpoints,
        &[REQUEST_LOG.to_string()],
        increased_throttle,
        Some(test1_lines),
    );
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Enable throttle to confirm it limits the number of request per second, in
// Gaggle mode. Increase the throttle and confirm it increases the number of
// requests per second, in Gaggle mode.
async fn test_throttle_gaggle() {
    // Multiple tests run together, so set a unique name.
    let request_log = "gaggle-".to_string() + REQUEST_LOG;

    // Start the mock server.
    let server = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Each worker has the same identical configuration.
    let configuration =
        common_build_configuration(&server, "", THROTTLE_REQUESTS, 0, 0, Some(true), None);

    // Build the load test for the Workers.
    // Launch each worker in its own thread, storing the join handles.
    let mut worker_handles = Vec::new();
    let mut request_logs = Vec::new();
    for i in 0..EXPECT_WORKERS {
        let mut worker_configuration = configuration.clone();
        worker_configuration.request_log = request_log.clone() + &i.to_string();
        request_logs.push(worker_configuration.request_log.clone());
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

    // Start manager instance in current thread and run a distributed load test.
    let manager_configuration = common_build_configuration(
        &server,
        &request_log,
        0,
        USERS,
        RUN_TIME,
        None,
        Some(EXPECT_WORKERS),
    );

    // Build the load test for the Manager.
    let manager_goose_attack = common::build_load_test(
        manager_configuration.clone(),
        vec![get_transactions()],
        None,
        None,
    );

    // Run the Goose Attack.
    common::run_load_test(manager_goose_attack, Some(worker_handles)).await;

    // Confirm that the load test was actually throttled.
    let test1_lines = validate_test(
        &mock_endpoints,
        &request_logs,
        // Throttle is configured per-worker, so multiply by EXPECT_WORKERS.
        THROTTLE_REQUESTS * EXPECT_WORKERS,
        None,
    );

    // Increase the throttle and run a second load test, so we can compare the difference
    // and confirm the throttle is actually working.
    let increased_throttle = THROTTLE_REQUESTS * 5;

    // Each worker has the same identical configuration.
    let configuration =
        common_build_configuration(&server, "", increased_throttle, 0, 0, Some(true), None);

    // Build the load test for the Workers.
    // Launch each worker in its own thread, storing the join handles.
    let mut worker_handles = Vec::new();
    let mut request_logs = Vec::new();
    for i in 0..EXPECT_WORKERS {
        let mut worker_configuration = configuration.clone();
        worker_configuration.request_log = request_log.clone() + &i.to_string();
        request_logs.push(worker_configuration.request_log.clone());
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

    // Build the load test for the Manager.
    let manager_goose_attack =
        common::build_load_test(manager_configuration, vec![get_transactions()], None, None);

    // Run the Goose Attack.
    common::run_load_test(manager_goose_attack, Some(worker_handles)).await;

    // Confirm that the load test was actually throttled, at an increased rate.
    let _ = validate_test(
        &mock_endpoints,
        &request_logs,
        // Throttle is configured per-worker, so multiply by EXPECT_WORKERS.
        increased_throttle * EXPECT_WORKERS,
        Some(test1_lines),
    );
}
