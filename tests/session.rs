use httpmock::{Method::GET, Method::POST, Mock, MockServer};
use reqwest::header;

mod common;

use goose::config::GooseConfiguration;
use goose::prelude::*;

// In this test the SessionData is a simple String.
struct SessionData(String);

// The actual session data that is set and later validated.
const SESSION_DATA: &str = "This is my session data.";

// Paths used in load tests performed during these tests.
const SESSION_PATH: &str = "/session";
const COOKIE_PATH: &str = "/cookie";

// Indexes for valid requests of above paths, used to validate tests.
const POST_SESSION_KEY: usize = 0;
const GET_SESSION_KEY: usize = 1;
const POST_COOKIE_KEY_0: usize = 2;
const GET_COOKIE_KEY_0: usize = 6;
const POST_COOKIE_KEY_1: usize = 7;
const GET_COOKIE_KEY_1: usize = 11;
const POST_COOKIE_KEY_2: usize = 12;
const GET_COOKIE_KEY_2: usize = 16;
const POST_COOKIE_KEY_3: usize = 17;
const GET_COOKIE_KEY_3: usize = 21;

// How many users to simulate, each with their own session.
const SESSION_USERS: &str = "10";

// How many users to simulate, each with their own cookie.
const COOKIE_USERS: &str = "4";

// There are multiple test variations in this file.
#[derive(Clone)]
enum TestType {
    // Test sessions.
    Session,
    // Test cookies.
    Cookie,
}

// Create a unqiue session per-user.
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

// Verify that the per-user session data is correct.
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

// Set a cookie that is unique per-user.
pub async fn set_cookie(user: &mut GooseUser) -> TransactionResult {
    // Per-user cookie name.
    let cookie_name = format!("TestCookie{}", user.weighted_users_index);

    // Per-user cookie path.
    let cookie_path = format!("{}{}", COOKIE_PATH, user.weighted_users_index);

    // Set the Cookie.
    let request_builder = user
        .get_request_builder(&GooseMethod::Post, &cookie_path)?
        .header("Cookie", format!("{}=foo", cookie_name));
    let goose_request = GooseRequest::builder()
        .set_request_builder(request_builder)
        .build();
    let goose = user.request(goose_request).await?;
    let response = goose.response.expect("there must be a response");
    let cookie: reqwest::cookie::Cookie = response.cookies().next().expect("cookie must be set");
    assert!(cookie.name() == cookie_name);

    Ok(())
}

// Verify that the per-user cookie is correct.
pub async fn validate_cookie(user: &mut GooseUser) -> TransactionResult {
    // Per-user cookie path.
    let cookie_path = format!("{}{}", COOKIE_PATH, user.weighted_users_index);

    // Load COOKIE_PATH, the mock endpoint will validate that the proper Cookie is set.
    // Each GooseUser launched has a unique user.weighted_users_index (from 0 to 3),
    // and each user has a unique Cookie name which is TestCookie# where # is the index.
    // Reqwest doesn't expose the cookie data it tracks, so we set up a per-user path
    // and validate the cookie on the mock server side. A 200 will be returned if the
    // correct cookie is passed in by the client. A 404 will be returned if not.
    let _goose = user.get(&cookie_path).await?;

    Ok(())
}

// All tests in this file run against common endpoints.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<Mock> {
    let cookie_path_0 = format!("{}0", COOKIE_PATH);
    let cookie_path_1 = format!("{}1", COOKIE_PATH);
    let cookie_path_2 = format!("{}2", COOKIE_PATH);
    let cookie_path_3 = format!("{}3", COOKIE_PATH);
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
        // CookiePath0: TestCookie0=foo
        server.mock(|when, then| {
            when.method(POST).path(&cookie_path_0);
            then.status(200)
                .header(header::SET_COOKIE.as_str(), "TestCookie0=foo");
        }),
        // Be sure TestCookie1 doesn't exist for user0.
        server.mock(|when, then| {
            when.method(GET)
                .path(&cookie_path_0)
                .cookie_exists("TestCookie1");
            then.status(500);
        }),
        // Be sure TestCookie2 doesn't exist for user0.
        server.mock(|when, then| {
            when.method(GET)
                .path(&cookie_path_0)
                .cookie_exists("TestCookie2");
            then.status(500);
        }),
        // Be sure TestCookie3 doesn't exist for user0.
        server.mock(|when, then| {
            when.method(GET)
                .path(&cookie_path_0)
                .cookie_exists("TestCookie3");
            then.status(500);
        }),
        // TestCookie0 should only exist for user0.
        server.mock(|when, then| {
            when.method(GET)
                .path(cookie_path_0)
                .cookie_exists("TestCookie0");
            then.status(200);
        }),
        // CookiePath1: TestCookie1=foo
        server.mock(|when, then| {
            when.method(POST).path(&cookie_path_1);
            then.status(200)
                .header(header::SET_COOKIE.as_str(), "TestCookie1=foo");
        }),
        // Be sure TestCookie0 doesn't exist for user1.
        server.mock(|when, then| {
            when.method(GET)
                .path(&cookie_path_1)
                .cookie_exists("TestCookie0");
            then.status(500);
        }),
        // Be sure TestCookie2 doesn't exist for user1.
        server.mock(|when, then| {
            when.method(GET)
                .path(&cookie_path_1)
                .cookie_exists("TestCookie2");
            then.status(500);
        }),
        // Be sure TestCookie3 doesn't exist for user1.
        server.mock(|when, then| {
            when.method(GET)
                .path(&cookie_path_1)
                .cookie_exists("TestCookie3");
            then.status(500);
        }),
        // TestCookie1 should only exist for user1.
        server.mock(|when, then| {
            when.method(GET)
                .path(cookie_path_1)
                .cookie_exists("TestCookie1");
            then.status(200);
        }),
        // CookiePath2: TestCookie2=foo
        server.mock(|when, then| {
            when.method(POST).path(&cookie_path_2);
            then.status(200)
                .header(header::SET_COOKIE.as_str(), "TestCookie2=foo");
        }),
        // Be sure TestCookie0 doesn't exist for user2.
        server.mock(|when, then| {
            when.method(GET)
                .path(&cookie_path_2)
                .cookie_exists("TestCookie0");
            then.status(500);
        }),
        // Be sure TestCookie1 doesn't exist for user2.
        server.mock(|when, then| {
            when.method(GET)
                .path(&cookie_path_2)
                .cookie_exists("TestCookie1");
            then.status(500);
        }),
        // Be sure TestCookie3 doesn't exist for user2.
        server.mock(|when, then| {
            when.method(GET)
                .path(&cookie_path_2)
                .cookie_exists("TestCookie3");
            then.status(500);
        }),
        // TestCookie2 should only exist for user0.
        server.mock(|when, then| {
            when.method(GET)
                .path(cookie_path_2)
                .cookie_exists("TestCookie2");
            then.status(200);
        }),
        // CookiePath3: TestCookie3=foo
        server.mock(|when, then| {
            when.method(POST).path(&cookie_path_3);
            then.status(200)
                .header(header::SET_COOKIE.as_str(), "TestCookie3=foo");
        }),
        // Be sure TestCookie0 doesn't exist for user3.
        server.mock(|when, then| {
            when.method(GET)
                .path(&cookie_path_3)
                .cookie_exists("TestCookie0");
            then.status(500);
        }),
        // Be sure TestCookie1 doesn't exist for user3.
        server.mock(|when, then| {
            when.method(GET)
                .path(&cookie_path_3)
                .cookie_exists("TestCookie1");
            then.status(500);
        }),
        // Be sure TestCookie2 doesn't exist for user3.
        server.mock(|when, then| {
            when.method(GET)
                .path(&cookie_path_3)
                .cookie_exists("TestCookie2");
            then.status(500);
        }),
        // TestCookie3 should only exist for user3.
        server.mock(|when, then| {
            when.method(GET)
                .path(cookie_path_3)
                .cookie_exists("TestCookie3");
            then.status(200);
        }),
    ]
}

// Build appropriate configuration for these tests.
fn common_build_configuration(
    test_type: &TestType,
    server: &MockServer,
    custom: &mut Vec<&str>,
) -> GooseConfiguration {
    // Common elements in all our tests.
    let mut configuration = match test_type {
        TestType::Session => vec![
            "--users",
            SESSION_USERS,
            "--hatch-rate",
            SESSION_USERS,
            "--run-time",
            "2",
        ],
        TestType::Cookie => vec![
            "--users",
            COOKIE_USERS,
            "--hatch-rate",
            COOKIE_USERS,
            "--run-time",
            "2",
        ],
    };

    // Custom elements in some tests.
    configuration.append(custom);

    // Return the resulting configuration.
    common::build_configuration(server, configuration)
}

// Helper to confirm all variations generate appropriate results.
fn validate_requests(test_type: TestType, goose_metrics: &GooseMetrics, mock_endpoints: &[Mock]) {
    // Convert USERS to a usize.
    let users = match test_type {
        TestType::Session => SESSION_USERS
            .parse::<usize>()
            .expect("must be a valid usize"),
        TestType::Cookie => COOKIE_USERS
            .parse::<usize>()
            .expect("must be a valid usize"),
    };

    match test_type {
        TestType::Session => {
            // Confirm that each user set a session one and only one time.
            assert!(mock_endpoints[POST_SESSION_KEY].hits() == users);
            // Confirm that each user validated their session multiple times.
            assert!(mock_endpoints[GET_SESSION_KEY].hits() > users);
        }
        TestType::Cookie => {
            // Confirm that each user set a cookie one and only one time.
            assert!(mock_endpoints[POST_COOKIE_KEY_0].hits() == 1);
            assert!(mock_endpoints[POST_COOKIE_KEY_1].hits() == 1);
            assert!(mock_endpoints[POST_COOKIE_KEY_2].hits() == 1);
            assert!(mock_endpoints[POST_COOKIE_KEY_3].hits() == 1);
            // Confirm that each user validated their cookie multiple times.
            assert!(mock_endpoints[GET_COOKIE_KEY_0].hits() > 1);
            assert!(mock_endpoints[GET_COOKIE_KEY_1].hits() > 1);
            assert!(mock_endpoints[GET_COOKIE_KEY_2].hits() > 1);
            assert!(mock_endpoints[GET_COOKIE_KEY_3].hits() > 1);
        }
    }

    // Extract the POST requests out of goose metrics.
    let post_metrics = match test_type {
        TestType::Session => goose_metrics.requests.get("POST create session").unwrap(),
        TestType::Cookie => goose_metrics.requests.get("POST create cookie").unwrap(),
    };

    // Extract the GET requests out of goose metrics.
    let get_metrics = match test_type {
        TestType::Session => goose_metrics.requests.get("GET read session").unwrap(),
        TestType::Cookie => goose_metrics.requests.get("GET read cookie").unwrap(),
    };

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
fn get_scenarios(test_type: &TestType) -> Scenario {
    match test_type {
        TestType::Session => {
            scenario!("Sessions")
                // Set up the sesssion only one time
                .register_transaction(
                    transaction!(set_session_data)
                        .set_on_start()
                        .set_name("create session"),
                )
                // Validate the session repeateldy.
                .register_transaction(transaction!(validate_session_data).set_name("read session"))
        }
        TestType::Cookie => {
            scenario!("Cookie")
                // Create the cookie only one time
                .register_transaction(
                    transaction!(set_cookie)
                        .set_on_start()
                        .set_name("create cookie"),
                )
                // Validate the cookie repeateldy.
                .register_transaction(transaction!(validate_cookie).set_name("read cookie"))
        }
    }
}

// Helper to run all standalone tests.
async fn run_standalone_test(test_type: TestType) {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    let mut configuration_flags = vec!["--no-reset-metrics"];

    // Build common configuration elements.
    let configuration = common_build_configuration(&test_type, &server, &mut configuration_flags);

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(
        common::build_load_test(
            configuration.clone(),
            vec![get_scenarios(&test_type)],
            None,
            None,
        ),
        None,
    )
    .await;

    // Confirm that the load test ran correctly.
    validate_requests(test_type, &goose_metrics, &mock_endpoints);
}

#[tokio::test]
// Test to confirm sessions are unique per GooseUser and last their lifetime.
async fn test_session() {
    run_standalone_test(TestType::Session).await;
}

#[tokio::test]
// Test to confirm cookies are unique per GooseUser and last their lifetime.
async fn test_cookie() {
    run_standalone_test(TestType::Cookie).await;
}
