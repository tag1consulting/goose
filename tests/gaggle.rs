use gumdrop::Options;
use httpmock::{Method::GET, Mock, MockServer};
use std::{str, time};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use goose::config::GooseConfiguration;
use goose::prelude::*;
//use goose::util::parse_timespan;

mod common;

// Paths used in load tests performed during these tests.
const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about.html";

// Indexes to the above paths.
const INDEX_KEY: usize = 0;
const ABOUT_KEY: usize = 1;

// Used to build a test plan.
const STARTUP_TIME: &str = "2s";
const USERS: usize = 100;
const RUNNING_TIME: &str = "2s";

// There are multiple test variations in this file.
#[derive(Clone, PartialEq)]
enum TestType {
    // Expect a single worker.
    Workers1,
    // Expect multiple workers/
    _Workers2,
}

// State machine for tracking Controller state during tests.
struct TestState {
    // A buffer for the telnet Controller.
    buf: [u8; 2048],
    // A TCP socket for the telnet Controller.
    telnet_stream: TcpStream,
}

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

// All tests in this file run against the following common endpoints.
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

// Build appropriate configuration for these tests. Normally this also calls
// common::build_configuration() to get defaults most all tests needs, but
// for these tests we don't want a default configuration. We keep the signature
// the same to simplify reuse, accepting the MockServer but not using it.
fn common_build_configuration(_server: &MockServer, custom: &mut Vec<&str>) -> GooseConfiguration {
    // Common elements in all our tests.
    let mut configuration: Vec<&str> =
        //vec!["--quiet", "--no-autostart", "--co-mitigation", "disabled"];
        vec!["--no-autostart", "--co-mitigation", "disabled"];

    // Custom elements in some tests.
    configuration.append(custom);

    // Parse these options to generate a GooseConfiguration.
    GooseConfiguration::parse_args_default(&configuration)
        .expect("failed to parse options and generate a configuration")
}

// Helper to confirm all variations generate appropriate results.
fn validate_one_scenario(
    _goose_metrics: &GooseMetrics,
    mock_endpoints: &[Mock],
    configuration: &GooseConfiguration,
    _test_type: TestType,
) {
    //println!("goose_metrics: {:#?}", goose_metrics);
    //println!("configuration: {:#?}", configuration);

    assert!(configuration.manager);
    assert!(!configuration.worker);

    // Confirm that we did not actually load the mock endpoints.
    assert!(mock_endpoints[INDEX_KEY].hits() == 0);
    assert!(mock_endpoints[ABOUT_KEY].hits() == 0);

    // Get index and about out of goose metrics.
    /*
    let index_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", INDEX_PATH))
        .unwrap();
    let about_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", ABOUT_PATH))
        .unwrap();

    // There should not have been any failures during this test.
    assert!(index_metrics.fail_count == 0);
    assert!(about_metrics.fail_count == 0);
    */

    // Host was not configured at start time.
    assert!(configuration.host.is_empty());

    // The load test was manually shut down instead of running to completion.
    //assert!(goose_metrics.duration >= parse_timespan(RUNNING_TIME));

    // Increasing, Maintaining, Increasing, Maintaining, Decreasing, Maintaining, Canceling,
    // Finished, Finished.
    // Finished is logged twice because `stop` puts the test to Idle, and then `shutdown`
    // actually shuts down the test, and both are logged as "Finished".
    //assert!(goose_metrics.history.len() == 9);
}

// Returns the appropriate scenario needed to build these tests.
fn get_transactions() -> Scenario {
    scenario!("LoadTest")
        .register_transaction(transaction!(get_index).set_weight(2).unwrap())
        .register_transaction(transaction!(get_about).set_weight(1).unwrap())
}

// Helper to run all standalone tests.
async fn run_standalone_test(test_type: TestType) {
    // Start the mock server.
    let server = MockServer::start();
    let server_url = server.base_url();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    let mut configuration_flags = match &test_type {
        // Manager with 1 Worker process.
        TestType::Workers1 => vec!["--manager", "--expect-workers", "1"],
        // Manager with multiple Worker processes.
        TestType::_Workers2 => vec!["--manager", "--expect-workers", "2"],
    };

    // Keep a copy for validation.
    let validate_test_type = test_type.clone();

    // Build common configuration elements.
    let configuration = common_build_configuration(&server, &mut configuration_flags);

    // Create a new thread from which to test the Controller.
    let _controller_handle = tokio::spawn(async move {
        // Sleep a half a second allowing the GooseAttack to start.
        tokio::time::sleep(time::Duration::from_millis(500)).await;

        let commands = [
            // Run load test against mock server.
            (format!("host {}\r\n", server_url), "host configured"),
            // Start USERS in STARTUP_TIME, run USERS for RUNNING_TIME, then shut down as quickly as possible.
            (
                format!(
                    "test_plan {},{};{},{};{},{}\r\n",
                    USERS, STARTUP_TIME, USERS, RUNNING_TIME, 0, 0
                ),
                "test-plan configured",
            ),
            // Start the Manager process.
            //("start\r\n".to_string(), "load test started"),
            // All done, shut down.
            ("shutdown\r\n".to_string(), "load test shut down"),
        ];

        // Maintain a test state for looping through commands.
        let mut test_state = TestState {
            buf: [0; 2048],
            telnet_stream: TcpStream::connect("127.0.0.1:5115").await.unwrap(),
        };
        let response = get_response(&mut test_state).await;
        assert!(response.starts_with("goose> "));

        for command in commands {
            log::info!(">-> Sending request: `{}`", command.0);
            let response = make_request(&mut test_state, &command.0).await;
            log::info!("<-< Received response: `{}`", response);
            assert!(response.starts_with(command.1));
        }
    });

    // Run the Goose Attack. Add timeout to be sure Goose exits even if the controller tests fail.
    let goose_metrics = tokio::time::timeout(
        tokio::time::Duration::from_secs(60),
        common::run_load_test(
            common::build_load_test(configuration.clone(), vec![get_transactions()], None, None),
            None,
        ),
    )
    .await
    .expect("load test timed out");

    // Confirm that the load test ran correctly.
    validate_one_scenario(
        &goose_metrics,
        &mock_endpoints,
        &configuration,
        validate_test_type,
    );
}

// Helper to send request to Controller and get response back.
async fn make_request(test_state: &mut TestState, command: &str) -> String {
    //println!("making request: {}", command);
    match test_state.telnet_stream.write_all(command.as_bytes()).await {
        Ok(_) => (),
        Err(e) => panic!("failed to send {} command: {}", command, e),
    };
    get_response(test_state).await.to_string()
}

// Helper to read response from Controller.
async fn get_response(test_state: &mut TestState) -> &str {
    let _ = match test_state.telnet_stream.read(&mut test_state.buf).await {
        Ok(data) => data,
        Err(e) => {
            panic!("ERROR: server disconnected: {}", e);
        }
    };
    str::from_utf8(&test_state.buf).unwrap()
}

// Test a load test simulating 1 Worker.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn test_worker1() {
    run_standalone_test(TestType::Workers1).await;
}
