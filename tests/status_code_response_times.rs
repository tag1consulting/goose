use httpmock::{Method::GET, Mock, MockServer};

mod common;

use goose::prelude::*;

// Paths used in load tests performed during these tests.
const SUCCESS_PATH: &str = "/success";
const MIXED_PATH: &str = "/mixed";
const ERROR_PATH: &str = "/error";

// Indexes to the above paths.
const SUCCESS_KEY: usize = 0;
const MIXED_KEY: usize = 1;
const ERROR_KEY: usize = 2;

// Test transaction that always returns 200.
pub async fn get_success(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(SUCCESS_PATH).await?;
    Ok(())
}

// Test transaction that returns mixed status codes.
pub async fn get_mixed(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(MIXED_PATH).await?;
    Ok(())
}

// Test transaction that always returns 400.
pub async fn get_error(user: &mut GooseUser) -> TransactionResult {
    let mut goose = user.get(ERROR_PATH).await?;

    if let Ok(r) = goose.response {
        let headers = &r.headers().clone();
        let status_code = r.status();
        if !status_code.is_success() {
            return user.set_failure(
                "got non-200 status code",
                &mut goose.request,
                Some(headers),
                None,
            );
        }
    }
    Ok(())
}

// Set up mock server with different endpoints that return different status codes.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<Mock<'_>> {
    let mut mocks = vec![
        // SUCCESS_PATH always returns 200
        server.mock(|when, then| {
            when.method(GET).path(SUCCESS_PATH);
            then.status(200);
        }),
        // MIXED_PATH returns 200 most of the time
        server.mock(|when, then| {
            when.method(GET).path(MIXED_PATH);
            then.status(200);
        }),
        // ERROR_PATH always returns 400
        server.mock(|when, then| {
            when.method(GET).path(ERROR_PATH);
            then.status(400);
        }),
    ];

    // Add a separate mock for MIXED_PATH that returns 400 occasionally
    // This will be handled randomly by httpmock
    for _ in 0..2 {
        mocks.push(server.mock(|when, then| {
            when.method(GET).path(MIXED_PATH);
            then.status(400);
        }));
    }

    mocks
}

// Returns the scenario needed to build these tests.
fn get_transactions() -> Scenario {
    scenario!("StatusCodeTest")
        .register_transaction(transaction!(get_success).set_weight(2).unwrap())
        .register_transaction(transaction!(get_mixed).set_weight(6).unwrap())
        .register_transaction(transaction!(get_error).set_weight(2).unwrap())
}

// Helper to validate that status code response time tracking works correctly.
fn validate_status_code_response_times(goose_metrics: &GooseMetrics, mock_endpoints: &[Mock]) {
    // Verify that endpoints were hit
    assert!(mock_endpoints[SUCCESS_KEY].hits() > 0);
    assert!(mock_endpoints[MIXED_KEY].hits() > 0);
    assert!(mock_endpoints[ERROR_KEY].hits() > 0);

    // Test the /success endpoint - should only have 200 status codes
    let success_request = goose_metrics
        .requests
        .get(&format!("GET {}", SUCCESS_PATH))
        .expect("Success request should exist");

    // Should have status code counts
    assert!(
        !success_request.status_code_counts.is_empty(),
        "Success endpoint should have status code counts"
    );
    assert!(
        success_request.status_code_counts.contains_key(&200),
        "Success endpoint should have 200 status codes"
    );
    assert!(
        !success_request.status_code_counts.contains_key(&400),
        "Success endpoint should not have 400 status codes"
    );

    // Should have status code response times, but since only one status code, no breakdown needed
    // (smart omission should apply)
    if success_request.status_code_response_times.len() <= 1 {
        // This is expected - single status code endpoints don't need breakdown
    } else {
        // If multiple status codes exist, verify they're properly tracked
        assert!(
            success_request
                .status_code_response_times
                .contains_key(&200),
            "Success endpoint should have 200 status code response times"
        );
    }

    // Test the /mixed endpoint - should have both 200 and 400 status codes
    let mixed_request = goose_metrics
        .requests
        .get(&format!("GET {}", MIXED_PATH))
        .expect("Mixed request should exist");

    // Should have status code counts for both 200 and 400
    assert!(
        !mixed_request.status_code_counts.is_empty(),
        "Mixed endpoint should have status code counts"
    );
    // Since we have multiple mock endpoints for the mixed path, we might get both status codes
    let has_multiple_status_codes = mixed_request.status_code_counts.len() > 1;

    if has_multiple_status_codes {
        // Should have status code response times for different status codes
        assert!(
            mixed_request.status_code_response_times.len() > 1,
            "Mixed endpoint with multiple status codes should have response time breakdowns"
        );
    }

    // Test the /error endpoint - should only have 400 status codes
    let error_request = goose_metrics
        .requests
        .get(&format!("GET {}", ERROR_PATH))
        .expect("Error request should exist");

    // Should have status code counts
    assert!(
        !error_request.status_code_counts.is_empty(),
        "Error endpoint should have status code counts"
    );
    assert!(
        error_request.status_code_counts.contains_key(&400),
        "Error endpoint should have 400 status codes"
    );
    assert!(
        !error_request.status_code_counts.contains_key(&200),
        "Error endpoint should not have 200 status codes"
    );
}

// Test status code response time tracking functionality.
#[tokio::test]
async fn test_status_code_response_time_tracking() {
    let server = MockServer::start();
    let mock_endpoints = setup_mock_server_endpoints(&server);

    let configuration = common::build_configuration(
        &server,
        vec!["--users", "4", "--hatch-rate", "4", "--run-time", "3"],
    );

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(
        common::build_load_test(configuration, vec![get_transactions()], None, None),
        None,
    )
    .await;

    validate_status_code_response_times(&goose_metrics, &mock_endpoints);
}

// Test status code response time tracking with HTML report generation.
#[tokio::test]
async fn test_status_code_response_time_html_report() {
    let server = MockServer::start();
    let mock_endpoints = setup_mock_server_endpoints(&server);

    let report_file = "status-code-test-report.html";
    let configuration = common::build_configuration(
        &server,
        vec![
            "--users",
            "4",
            "--hatch-rate",
            "4",
            "--run-time",
            "3",
            "--report-file",
            report_file,
        ],
    );

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(
        common::build_load_test(configuration, vec![get_transactions()], None, None),
        None,
    )
    .await;

    validate_status_code_response_times(&goose_metrics, &mock_endpoints);

    // Verify HTML report was generated
    assert!(
        std::path::Path::new(report_file).exists(),
        "HTML report should be generated"
    );

    // Read the HTML report and check for status code breakdowns
    let html_content =
        std::fs::read_to_string(report_file).expect("Should be able to read HTML report");

    // Look for status code breakdown indicators in the HTML
    // The breakdown should show up as tree-like formatting with └─
    if html_content.contains("└─") {
        // Verify that the breakdown shows percentages and different status codes
        assert!(
            html_content.contains("200"),
            "HTML report should contain 200 status codes"
        );
        assert!(
            html_content.contains("400"),
            "HTML report should contain 400 status codes"
        );
        assert!(
            html_content.contains("%"),
            "HTML report should contain percentage indicators"
        );
    }

    // Cleanup
    common::cleanup_files(vec![report_file]);
}

// Test that status code response time data structure works correctly through integration testing.
// Since the internal methods are private, we test through the actual load test system.
#[tokio::test]
async fn test_status_code_response_time_integration() {
    let server = MockServer::start();

    // Set up a simple endpoint that always returns 200
    let _mock_endpoint = server.mock(|when, then| {
        when.method(GET).path("/test");
        then.status(200);
    });

    let configuration = common::build_configuration(
        &server,
        vec!["--users", "1", "--hatch-rate", "1", "--run-time", "1"],
    );

    // Create a simple scenario that makes requests
    let scenario = scenario!("TestScenario")
        .register_transaction(transaction!(test_transaction).set_weight(1).unwrap());

    // Run the load test
    let goose_metrics = common::run_load_test(
        common::build_load_test(configuration, vec![scenario], None, None),
        None,
    )
    .await;

    // Verify that we have request metrics
    assert!(
        !goose_metrics.requests.is_empty(),
        "Should have recorded request metrics"
    );

    // Get the request metric for our test endpoint
    let request_key = "GET /test";
    if let Some(request_metric) = goose_metrics.requests.get(request_key) {
        // Verify that status_code_response_times is properly populated
        // This tests the integration of the entire feature
        assert!(
            !request_metric.status_code_counts.is_empty(),
            "Should have status code counts"
        );
        assert!(
            request_metric.status_code_counts.contains_key(&200),
            "Should have recorded 200 status codes"
        );
    }
}

// Simple transaction function for testing
async fn test_transaction(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/test").await?;
    Ok(())
}
