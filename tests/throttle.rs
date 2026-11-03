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
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<Mock<'_>> {
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
) -> GooseConfiguration {
    common::build_configuration(
        server,
        vec![
            "--users",
            &users.to_string(),
            "--increase-rate",
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

// Helper to confirm all variations generate appropriate results.
fn validate_test(
    mock_endpoints: &[Mock],
    request_logs: &[String],
    throttle_value: usize,
    previous_requests_file_lines: Option<usize>,
    ran: std::time::Duration,
) -> usize {
    // Verify that the metrics file was created and has the correct number of lines.
    let mut current_requests_file_lines = 0;
    for request_log in request_logs {
        assert!(std::path::Path::new(request_log).exists());
        current_requests_file_lines += common::file_length(request_log);
    }

    // Confirm that we loaded the mock endpoints.
    assert!(mock_endpoints[INDEX_KEY].calls() > 0);
    assert!(mock_endpoints[ABOUT_KEY].calls() > 0);

    // Requests are made at the throttle's rate for as long as its throttle
    // runs, which is within `ran`, the whole load test's time: the users'
    // launch, RUN_TIME and the shutdown. Each process's throttle also starts
    // with one token's room free.
    let most = (ran.as_secs_f64() * throttle_value as f64).ceil() as usize + request_logs.len();
    assert!(
        current_requests_file_lines <= most,
        "{} requests, at most {}",
        current_requests_file_lines,
        most
    );

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
    let configuration =
        common_build_configuration(&server, REQUEST_LOG, THROTTLE_REQUESTS, USERS, RUN_TIME);

    // Run the Goose Attack.
    let started = std::time::Instant::now();
    common::run_load_test(common::build_load_test(
        configuration,
        vec![get_transactions()],
        None,
        None,
    ))
    .await;
    let ran = started.elapsed();

    // Confirm that the load test was actually throttled.
    let test1_lines = validate_test(
        &mock_endpoints,
        &[REQUEST_LOG.to_string()],
        THROTTLE_REQUESTS,
        None,
        ran,
    );

    // Increase the throttle and run a second load test, so we can compare the difference
    // and confirm the throttle is actually working.
    let increased_throttle = THROTTLE_REQUESTS * 5;

    // Build a new configuration.
    let configuration =
        common_build_configuration(&server, REQUEST_LOG, increased_throttle, USERS, RUN_TIME);

    // Run the Goose Attack.
    let started = std::time::Instant::now();
    common::run_load_test(common::build_load_test(
        configuration,
        vec![get_transactions()],
        None,
        None,
    ))
    .await;
    let ran = started.elapsed();

    // Confirm that the load test was actually throttled, at an increased rate.
    let _ = validate_test(
        &mock_endpoints,
        &[REQUEST_LOG.to_string()],
        increased_throttle,
        Some(test1_lines),
        ran,
    );
}
