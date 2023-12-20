use httpmock::{Method::GET, Method::POST, Mock, MockServer};

mod common;

use goose::config::GooseConfiguration;
use goose::prelude::*;

// In this test the SessionData is a simple String.
struct SessionData(String);

// The actual session data that is set and later validated.
const SESSION_DATA: &str = "This is my session data.";

// Paths used in load tests performed during these tests.
const SESSION_PATH: &str = "/session";

// Indexes to the above paths.
const POST_SESSION_KEY: usize = 0;
const GET_SESSION_KEY: usize = 1;

// How many users to simulate, each with their own session.
const USERS: &str = "10";

// Test transaction.
pub async fn set_session_data(user: &mut GooseUser) -> TransactionResult {
    // Confirm that we start with empty session data.
    let session_data = user.get_session_data::<SessionData>();
    assert!(session_data.is_none());

    // We don't really have to make a request here, but we can...
    let _goose = user.post(SESSION_PATH, SESSION_DATA).await?;

    // Store data in the session, unique per user.
    user.set_session_data(SessionData(format!(
        "{}.{}",
        SESSION_DATA, user.weighted_users_index
    )));

    // Confirm that we now have session data.
    let session_data = user.get_session_data::<SessionData>();
    assert!(session_data.is_some());

    Ok(())
}

// Test transaction.
pub async fn validate_session_data(user: &mut GooseUser) -> TransactionResult {
    // We don't really have to make a request here, but we can...
    let _goose = user.get(SESSION_PATH).await?;

    // Confirm that we now have session data.
    let session_data = user.get_session_data::<SessionData>();
    assert!(session_data.is_some());

    // Confirm tht the session data is valid.
    if let Some(data) = session_data {
        // Validate that session data is unique-per-user.
        assert!(data.0 == format!("{}.{}", SESSION_DATA, user.weighted_users_index));
    } else {
        panic!("no session data !?");
    }

    Ok(())
}

// All tests in this file run against common endpoints.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<Mock> {
    vec![
        // Set up SESSION_PATH, store in vector at POST_SESSION_KEY.
        server.mock(|when, then| {
            when.method(POST).path(SESSION_PATH);
            then.status(200);
        }),
        // Set up SESSION_PATH, store in vector at GET_SESSION_KEY.
        server.mock(|when, then| {
            when.method(GET).path(SESSION_PATH);
            then.status(200);
        }),
    ]
}

// Build appropriate configuration for these tests.
fn common_build_configuration(server: &MockServer, custom: &mut Vec<&str>) -> GooseConfiguration {
    // Common elements in all our tests.
    let mut configuration = vec!["--users", USERS, "--hatch-rate", USERS, "--run-time", "2"];

    // Custom elements in some tests.
    configuration.append(custom);

    // Return the resulting configuration.
    common::build_configuration(server, configuration)
}

// Helper to confirm all variations generate appropriate results.
fn validate_requests(goose_metrics: &GooseMetrics, mock_endpoints: &[Mock]) {
    // Convert USERS to a usize.
    let users = USERS.parse::<usize>().expect("usize");

    // Confirm that we loaded the mock endpoints.
    assert!(mock_endpoints[POST_SESSION_KEY].hits() == users);
    assert!(mock_endpoints[GET_SESSION_KEY].hits() > users);

    // Extract the POST and GET requests out of goose metrics.
    let post_metrics = goose_metrics
        .requests
        .get(&format!("POST {}", SESSION_PATH))
        .unwrap();
    let get_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", SESSION_PATH))
        .unwrap();

    // We POST and GET the same path.
    assert!(post_metrics.path == get_metrics.path);
    // We made POST requests.
    assert!(post_metrics.method == GooseMethod::Post);
    // We made GET requests.
    assert!(get_metrics.method == GooseMethod::Get);
    // We made only 1 POST request per user.
    assert!(post_metrics.success_count == users);
    // We made more than 1 GET request per user.
    assert!(get_metrics.success_count > users);
    // There were no POST errors.
    assert!(post_metrics.fail_count == 0);
    // There were no GET errors.
    assert!(get_metrics.fail_count == 0);
}

// Returns the appropriate scenario needed to build these tests.
fn get_transactions() -> Scenario {
    scenario!("LoadTest")
        // Set up the sesssion only one time
        .register_transaction(transaction!(set_session_data).set_on_start())
        // Validate the session repeateldy.
        .register_transaction(transaction!(validate_session_data))
}

// Helper to run all standalone tests.
async fn run_standalone_test() {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    let mut configuration_flags = vec!["--no-reset-metrics"];

    // Build common configuration elements.
    let configuration = common_build_configuration(&server, &mut configuration_flags);

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(
        common::build_load_test(configuration.clone(), vec![get_transactions()], None, None),
        None,
    )
    .await;

    // Confirm that the load test ran correctly.
    validate_requests(&goose_metrics, &mock_endpoints);
}

#[tokio::test]
// Test a single scenario with multiple weighted transactions.
async fn test_session() {
    run_standalone_test().await;
}
