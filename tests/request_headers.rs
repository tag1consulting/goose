use httpmock::{Method::GET, Mock, MockServer};
use serial_test::serial;
use std::sync::Mutex;

mod common;

use goose::prelude::*;

// Path used in load tests performed during these tests, always returns an error so the
// error log is written to.
const ERROR_PATH: &str = "/error";

// Header set on each request, in addition to an authorization header.
const HEADER_NAME: &str = "x-goose-test";
const HEADER_VALUE: &str = "639";

// The headers as captured in `GooseRawRequest.headers`, formatted the same as all
// earlier releases of Goose so log output is unchanged.
const EXPECTED_HEADERS: [&str; 2] = [
    "(\"authorization\", Sensitive)",
    "(\"x-goose-test\", \"639\")",
];

// The same headers as they appear in a JSON formatted log.
const EXPECTED_JSON: &str =
    r#""headers":["(\"authorization\", Sensitive)","(\"x-goose-test\", \"639\")"]"#;

// Every `GooseRawRequest.headers` seen by the test transaction, validated after the
// load test completes. Tests in this file run serially, so they can share this.
static CAPTURED_HEADERS: Mutex<Vec<Vec<String>>> = Mutex::new(Vec::new());

// There are multiple test variations in this file.
enum TestType {
    // Test with no log enabled.
    NoLog,
    // Test with request log enabled.
    Requests,
    // Test with debug log enabled.
    Debug,
    // Test with error log enabled.
    Error,
}

// Test transaction, makes a request that sets headers and fails.
pub async fn get_error_with_headers(user: &mut GooseUser) -> TransactionResult {
    let request_builder = user
        .get_request_builder(&GooseMethod::Get, ERROR_PATH)?
        .bearer_auth("0123456789abcdef0123456789abcdef01234567")
        .header(HEADER_NAME, HEADER_VALUE);
    let goose_request = GooseRequest::builder()
        .set_request_builder(request_builder)
        .build();
    let mut goose = user.request(goose_request).await?;

    CAPTURED_HEADERS
        .lock()
        .unwrap()
        .push(goose.request.raw.headers.clone());

    if let Ok(r) = goose.response {
        let headers = &r.headers().clone();
        if !r.status().is_success() {
            // Also writes the request to the debug log, if enabled.
            return user.set_failure(
                "loaded /error and got non-200 message",
                &mut goose.request,
                Some(headers),
                None,
            );
        }
    }
    Ok(())
}

// All tests in this file run against a common endpoint.
fn setup_mock_server_endpoint(server: &MockServer) -> Mock<'_> {
    server.mock(|when, then| {
        when.method(GET).path(ERROR_PATH);
        then.status(503);
    })
}

// Helper to run each test variation.
async fn run_header_test(test_type: TestType) {
    let log_file = match test_type {
        TestType::NoLog => "",
        TestType::Requests => "request-headers-request-log.json",
        TestType::Debug => "request-headers-debug-log.json",
        TestType::Error => "request-headers-error-log.json",
    };

    CAPTURED_HEADERS.lock().unwrap().clear();

    let server = MockServer::start();
    let mock_endpoint = setup_mock_server_endpoint(&server);

    let mut configuration_flags = match test_type {
        TestType::NoLog => vec![],
        TestType::Requests => vec!["--request-log", log_file, "--request-format", "json"],
        TestType::Debug => vec!["--debug-log", log_file, "--debug-format", "json"],
        TestType::Error => vec!["--error-log", log_file, "--error-format", "json"],
    };
    configuration_flags.extend(vec![
        "--users",
        "1",
        "--increase-rate",
        "1",
        "--run-time",
        "1",
    ]);
    let configuration = common::build_configuration(&server, configuration_flags);

    common::run_load_test(
        common::build_load_test(
            configuration,
            vec![scenario!("LoadTest").register_transaction(transaction!(get_error_with_headers))],
            None,
            None,
        ),
        None,
    )
    .await;

    assert!(mock_endpoint.calls() > 0);

    let captured = CAPTURED_HEADERS.lock().unwrap().clone();
    assert!(!captured.is_empty());

    match test_type {
        TestType::NoLog => {
            // Nothing reads the headers, so they are not captured.
            for headers in &captured {
                assert!(headers.is_empty(), "unexpected headers: {:?}", headers);
            }
        }
        _ => {
            // A log reads the headers, so they are captured in the historic format.
            for headers in &captured {
                assert_eq!(headers, &EXPECTED_HEADERS);
            }
            // And written to the log exactly as before.
            let log = std::fs::read_to_string(log_file).unwrap();
            assert!(!log.is_empty());
            for line in log.lines() {
                assert!(
                    line.contains(EXPECTED_JSON),
                    "log line missing headers: {}",
                    line
                );
            }
            common::cleanup_files(vec![log_file]);
        }
    }
}

#[tokio::test]
#[serial]
/// Request headers are not captured when no log needs them.
async fn test_request_headers_no_log() {
    run_header_test(TestType::NoLog).await;
}

#[tokio::test]
#[serial]
/// Request headers are captured and logged when the request log is enabled.
async fn test_request_headers_request_log() {
    run_header_test(TestType::Requests).await;
}

#[tokio::test]
#[serial]
/// Request headers are captured and logged when the debug log is enabled.
async fn test_request_headers_debug_log() {
    run_header_test(TestType::Debug).await;
}

#[tokio::test]
#[serial]
/// Request headers are captured and logged when the error log is enabled.
async fn test_request_headers_error_log() {
    run_header_test(TestType::Error).await;
}
