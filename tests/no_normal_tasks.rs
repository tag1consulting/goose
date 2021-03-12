use httpmock::{
    Method::{GET, POST},
    MockRef, MockServer,
};

mod common;

use goose::goose::GooseTaskSet;
use goose::prelude::*;
use goose::GooseConfiguration;

// Paths used in load tests performed during these tests.
const LOGIN_PATH: &str = "/login";
const LOGOUT_PATH: &str = "/logout";

// Indexes to the above paths.
const LOGIN_KEY: usize = 0;
const LOGOUT_KEY: usize = 1;

// Load test configuration.
const EXPECT_WORKERS: usize = 2;
const USERS: usize = 5;
const RUN_TIME: usize = 2;

// Test task.
pub async fn login(user: &GooseUser) -> GooseTaskResult {
    let request_builder = user.goose_post(LOGIN_PATH).await?;
    let params = [("username", "me"), ("password", "s3crET!")];
    let _goose = user.goose_send(request_builder.form(&params), None).await?;
    Ok(())
}

// Test task.
pub async fn logout(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(LOGOUT_PATH).await?;
    Ok(())
}

// All tests in this file run against common endpoints.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<MockRef> {
    let mut endpoints: Vec<MockRef> = Vec::new();

    // First set up LOGIN_PATH, store in vector at LOGIN_KEY.
    endpoints.push(server.mock(|when, then| {
        when.method(POST).path(LOGIN_PATH);
        then.status(200);
    }));
    // Next set up LOGOUT_PATH, store in vector at LOGOUT_KEY.
    endpoints.push(server.mock(|when, then| {
        when.method(GET).path(LOGOUT_PATH);
        then.status(200);
    }));

    endpoints
}

// Build appropriate configuration for these tests.
fn common_build_configuration(
    server: &MockServer,
    worker: Option<bool>,
    manager: Option<usize>,
) -> GooseConfiguration {
    if let Some(expect_workers) = manager {
        common::build_configuration(
            &server,
            vec![
                "--manager",
                "--expect-workers",
                &expect_workers.to_string(),
                "--users",
                &USERS.to_string(),
                "--hatch-rate",
                &USERS.to_string(),
                "--run-time",
                &RUN_TIME.to_string(),
                "--no-reset-metrics",
            ],
        )
    } else if worker.is_some() {
        common::build_configuration(&server, vec!["--worker"])
    } else {
        common::build_configuration(
            &server,
            vec![
                "--users",
                &USERS.to_string(),
                "--hatch-rate",
                &USERS.to_string(),
                "--run-time",
                &RUN_TIME.to_string(),
                "--no-reset-metrics",
            ],
        )
    }
}

// Helper to confirm all variations generate appropriate results.
fn validate_test(mock_endpoints: &[MockRef]) {
    // Confirm that the on_start and on_exit tasks actually ran once per GooseUser.
    mock_endpoints[LOGIN_KEY].assert_hits(USERS);
    mock_endpoints[LOGOUT_KEY].assert_hits(USERS);
}

// Returns the appropriate taskset needed to build these tests.
fn get_tasks() -> GooseTaskSet {
    taskset!("LoadTest")
        .register_task(task!(login).set_on_start())
        .register_task(task!(logout).set_on_stop())
}

// Helper to run the test, takes a flag for indicating if running in standalone
// mode or Gaggle mode.
fn run_load_test(is_gaggle: bool) {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Configure and run test differently in standalone versus Gaggle mode.
    match is_gaggle {
        false => {
            // Build common configuration.
            let configuration = common_build_configuration(&server, None, None);

            // Run the Goose Attack.
            common::run_load_test(
                common::build_load_test(configuration, &get_tasks(), None, None),
                None,
            );
        }
        true => {
            // Build common configuration.
            let worker_configuration = common_build_configuration(&server, Some(true), None);

            // Workers launched in own threads, store thread handles.
            let worker_handles = common::launch_gaggle_workers(
                common::build_load_test(worker_configuration, &get_tasks(), None, None),
                EXPECT_WORKERS,
            );

            // Build Manager configuration.
            let manager_configuration =
                common_build_configuration(&server, None, Some(EXPECT_WORKERS));

            // Run the Goose Attack.
            common::run_load_test(
                common::build_load_test(manager_configuration, &get_tasks(), None, None),
                Some(worker_handles),
            );
        }
    }

    // Confirm the load test ran correctly.
    validate_test(&mock_endpoints);
}

#[test]
// Test taskset with only on_start() and on_stop() tasks.
fn test_no_normal_tasks() {
    // Run load test with is_gaggle set to false.
    run_load_test(false);
}

#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Test taskset with only on_start() and on_stop() tasks, in Gaggle mode.
fn test_no_normal_tasks_gaggle() {
    // Run load test with is_gaggle set to true.
    run_load_test(true);
}
