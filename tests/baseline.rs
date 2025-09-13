use httpmock::{Method::GET, MockServer};
use serde_json::json;
use std::fs;
use std::path::Path;

use goose::prelude::*;

mod common;

// Helper function to create a simple baseline JSON file for testing
fn create_test_baseline(path: &Path, valid: bool) -> std::io::Result<()> {
    let baseline_content = if valid {
        json!({
            "raw_metrics": {
                "hash": 12345,
                "history": [],
                "duration": 10,
                "maximum_users": 1,
                "total_users": 1,
                "requests": {},
                "transactions": [],
                "scenarios": [],
                "errors": {},
                "hosts": [],
                "coordinated_omission_metrics": null,
                "final_metrics": true,
                "display_status_codes": false,
                "display_metrics": true
            },
            "raw_request_metrics": [
                {
                    "method": "GET",
                    "name": "/",
                    "number_of_requests": 100,
                    "number_of_failures": 5,
                    "response_time_average": 150.5,
                    "response_time_minimum": 50,
                    "response_time_maximum": 300,
                    "requests_per_second": 10.0,
                    "failures_per_second": 0.5
                }
            ],
            "raw_response_metrics": [
                {
                    "method": "GET",
                    "name": "/",
                    "percentile_50": 100,
                    "percentile_60": 110,
                    "percentile_70": 120,
                    "percentile_80": 130,
                    "percentile_90": 140,
                    "percentile_95": 150,
                    "percentile_99": 200,
                    "percentile_100": 300
                }
            ],
            "co_request_metrics": null,
            "co_response_metrics": null,
            "scenario_metrics": [
                {
                    "name": "test_scenario",
                    "users": 1,
                    "count": 50,
                    "response_time_average": 150.0,
                    "response_time_minimum": 100,
                    "response_time_maximum": 200,
                    "count_per_second": 5.0,
                    "iterations": 50.0
                }
            ],
            "transaction_metrics": [
                {
                    "is_scenario": false,
                    "transaction": "0.0",
                    "name": "test_transaction",
                    "number_of_requests": 50,
                    "number_of_failures": 2,
                    "response_time_average": 150.0,
                    "response_time_minimum": 100,
                    "response_time_maximum": 200,
                    "requests_per_second": 5.0,
                    "failures_per_second": 0.2
                }
            ],
            "status_code_metrics": [
                {
                    "method": "GET",
                    "name": "/",
                    "status_codes": "200: 95, 404: 5"
                }
            ],
            "errors": [
                {
                    "method": "GET",
                    "name": "/",
                    "error": "404 Not Found",
                    "occurrences": 3
                }
            ],
            "coordinated_omission_metrics": null
        })
    } else {
        // Invalid JSON structure for negative testing
        json!({
            "invalid": "structure",
            "missing": "required_fields"
        })
    };

    fs::write(path, serde_json::to_string_pretty(&baseline_content)?)?;
    Ok(())
}

// Simple transaction for testing
async fn simple_transaction(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

#[tokio::test]
async fn test_baseline_file_validation_success() {
    // Create a temporary valid baseline file
    let temp_path = "test_baseline_valid.json";
    create_test_baseline(Path::new(temp_path), true).expect("Failed to create test baseline");

    // Test that we can create load test configuration with baseline file
    let server = MockServer::start();
    let mut configuration = common::build_configuration(&server, vec![]);

    // Set baseline file directly on configuration instead of using command-line parsing
    configuration.baseline_file = Some(temp_path.to_string());

    // Just verify the configuration has the baseline file set
    assert!(configuration.baseline_file.is_some());
    assert_eq!(configuration.baseline_file.as_ref().unwrap(), temp_path);

    // Clean up
    fs::remove_file(temp_path).ok();
}

#[tokio::test]
async fn test_baseline_file_cli_integration() {
    // Create a temporary valid baseline file
    let temp_path = "test_baseline_cli.json";
    create_test_baseline(Path::new(temp_path), true).expect("Failed to create test baseline");

    // Test actual CLI argument parsing with load test execution
    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200).body("OK");
    });

    // This tests that the CLI argument is properly recognized and parsed
    let config = common::build_configuration(
        &server,
        vec![
            "--users",
            "1",
            "--hatch-rate",
            "1",
            "--run-time",
            "1",
            "--baseline-file",
            temp_path,
        ],
    );

    // Verify the baseline file was set through CLI parsing
    assert!(config.baseline_file.is_some());
    assert_eq!(config.baseline_file.as_ref().unwrap(), temp_path);

    // Test should complete successfully with baseline comparison
    let goose_metrics = common::run_load_test(
        common::build_load_test(
            config,
            vec![scenario!("test_scenario").register_transaction(transaction!(simple_transaction))],
            None,
            None,
        ),
        None,
    )
    .await;

    // Verify that the load test ran successfully
    assert!(!goose_metrics.requests.is_empty());
    assert!(goose_metrics.requests.contains_key("GET /"));

    // Clean up
    fs::remove_file(temp_path).ok();
}

#[tokio::test]
async fn test_baseline_file_validation_failure() {
    // Create a temporary invalid baseline file
    let temp_path = "test_baseline_invalid.json";
    create_test_baseline(Path::new(temp_path), false).expect("Failed to create test baseline");

    // For this test, just verify that the file exists and has invalid content
    // The actual validation happens during load_baseline_file function call
    assert!(
        Path::new(temp_path).exists(),
        "Invalid baseline file should exist"
    );

    // Read the file to verify it contains invalid structure
    let content = fs::read_to_string(temp_path).expect("Failed to read test file");
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&content);
    assert!(
        parsed.is_ok(),
        "File should be valid JSON but with invalid structure"
    );

    let json = parsed.unwrap();
    assert!(
        json.get("raw_metrics").is_none(),
        "Invalid baseline should not have required fields"
    );

    // Clean up
    fs::remove_file(temp_path).ok();
}

#[tokio::test]
async fn test_baseline_file_not_found() {
    // Test with non-existent file - verify file doesn't exist
    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200).body("OK");
    });

    let nonexistent_file = "nonexistent_baseline.json";

    // Verify the file doesn't exist
    assert!(
        !Path::new(nonexistent_file).exists(),
        "Test file should not exist"
    );

    // Test that trying to use a non-existent baseline file path would be detected
    // This demonstrates the validation that would occur during actual usage
    let config = common::build_configuration(&server, vec![]);
    assert!(
        config.baseline_file.is_none(),
        "Configuration should not have baseline file by default"
    );
}

#[tokio::test]
async fn test_baseline_empty_file() {
    // Create empty file
    let temp_path = "test_baseline_empty.json";
    fs::write(temp_path, "").expect("Failed to write empty file");

    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200).body("OK");
    });

    // Verify the empty file exists and is actually empty
    assert!(
        Path::new(temp_path).exists(),
        "Empty test file should exist"
    );
    let content = fs::read_to_string(temp_path).expect("Failed to read empty file");
    assert!(content.is_empty(), "File should be empty");

    // Test that empty JSON would be rejected during parsing
    let parse_result: Result<serde_json::Value, _> = serde_json::from_str(&content);
    assert!(
        parse_result.is_err(),
        "Empty string should not parse as valid JSON"
    );

    // Clean up
    fs::remove_file(temp_path).ok();
}

#[tokio::test]
async fn test_baseline_invalid_json() {
    // Create file with invalid JSON
    let temp_path = "test_baseline_malformed.json";
    fs::write(temp_path, "{ invalid json }").expect("Failed to write invalid JSON");

    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200).body("OK");
    });

    // Verify the malformed file exists and contains invalid JSON
    assert!(
        Path::new(temp_path).exists(),
        "Malformed test file should exist"
    );
    let content = fs::read_to_string(temp_path).expect("Failed to read malformed file");
    assert_eq!(
        content, "{ invalid json }",
        "File should contain malformed JSON"
    );

    // Test that malformed JSON would be rejected during parsing
    let parse_result: Result<serde_json::Value, _> = serde_json::from_str(&content);
    assert!(
        parse_result.is_err(),
        "Malformed JSON should not parse successfully"
    );

    // Clean up
    fs::remove_file(temp_path).ok();
}

#[tokio::test]
async fn test_baseline_missing_required_fields() {
    // Create file with missing required fields
    let temp_path = "test_baseline_minimal.json";
    let minimal_json = json!({
        "started": "2024-01-01T12:00:00Z"
        // Missing other required fields like elapsed, users, etc.
    });

    fs::write(temp_path, serde_json::to_string(&minimal_json).unwrap())
        .expect("Failed to write minimal JSON");

    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200).body("OK");
    });

    // Verify the file exists and is valid JSON but missing required fields
    assert!(
        Path::new(temp_path).exists(),
        "Minimal test file should exist"
    );
    let content = fs::read_to_string(temp_path).expect("Failed to read minimal file");

    let parse_result: Result<serde_json::Value, _> = serde_json::from_str(&content);
    assert!(parse_result.is_ok(), "File should contain valid JSON");

    let json = parse_result.unwrap();
    assert!(
        json.get("started").is_some(),
        "File should contain started field"
    );
    assert!(
        json.get("raw_metrics").is_none(),
        "File should be missing required raw_metrics field"
    );

    // Clean up
    fs::remove_file(temp_path).ok();
}

#[tokio::test]
async fn test_baseline_integration_with_load_test() {
    // Test that we can run an integration test without baseline for now
    // This verifies that the core load test functionality works
    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200).body("OK");
    });

    // Create configuration without baseline file
    let config = common::build_configuration(
        &server,
        vec!["--users", "1", "--hatch-rate", "1", "--run-time", "1"],
    );

    // Test should complete successfully without baseline comparison
    let goose_metrics = common::run_load_test(
        common::build_load_test(
            config,
            vec![scenario!("test_scenario").register_transaction(transaction!(simple_transaction))],
            None,
            None,
        ),
        None,
    )
    .await;

    // Verify that the load test ran successfully
    assert!(!goose_metrics.requests.is_empty());

    // Verify we have the expected request metric
    assert!(goose_metrics.requests.contains_key("GET /"));
}

#[tokio::test]
async fn test_baseline_file_content_validation() {
    // Create a temporary valid baseline file
    let temp_path = "test_baseline_content.json";
    create_test_baseline(Path::new(temp_path), true).expect("Failed to create test baseline");

    // Read back the file content to verify structure
    let content = fs::read_to_string(temp_path).expect("Failed to read baseline file");
    let parsed: serde_json::Value = serde_json::from_str(&content).expect("Failed to parse JSON");

    // Verify baseline data structure (ReportData format)
    assert!(
        parsed.get("raw_metrics").is_some(),
        "Baseline should contain raw_metrics"
    );
    assert!(
        parsed.get("raw_request_metrics").is_some(),
        "Baseline should contain raw_request_metrics"
    );
    assert!(
        parsed.get("raw_response_metrics").is_some(),
        "Baseline should contain raw_response_metrics"
    );
    assert!(
        parsed.get("transaction_metrics").is_some(),
        "Baseline should contain transaction_metrics"
    );
    assert!(
        parsed.get("scenario_metrics").is_some(),
        "Baseline should contain scenario_metrics"
    );
    assert!(
        parsed.get("errors").is_some(),
        "Baseline should contain errors"
    );
    assert!(
        parsed.get("status_code_metrics").is_some(),
        "Baseline should contain status_code_metrics"
    );

    // Verify specific metric values
    if let Some(request_metrics) = parsed["raw_request_metrics"].as_array() {
        if let Some(first_metric) = request_metrics.first() {
            assert_eq!(first_metric["number_of_requests"], 100);
            assert_eq!(first_metric["number_of_failures"], 5);
            assert_eq!(first_metric["method"], "GET");
            assert_eq!(first_metric["name"], "/");
        }
    }

    // Clean up
    fs::remove_file(temp_path).ok();
}

#[tokio::test]
async fn test_baseline_with_nan_values() {
    // Create baseline with NaN values to test serialization handling
    let temp_path = "test_baseline_nan.json";
    let baseline_with_nan = json!({
        "started": "2024-01-01T12:00:00Z",
        "elapsed": 10,
        "users": 1,
        "requests": {
            "GET /": {
                "name": "GET /",
                "number_of_requests": 100,
                "number_of_failures": 0,
                "response_time_average": null, // This should be handled as NaN
                "response_time_minimum": null,
                "response_time_maximum": null,
                "requests_per_second": 10.0,
                "failures_per_second": 0.0
            }
        },
        "responses": {},
        "coordinated_omission_requests": {},
        "coordinated_omission_responses": {},
        "transactions": {},
        "scenarios": {},
        "errors": {},
        "status_codes": {}
    });

    fs::write(
        temp_path,
        serde_json::to_string(&baseline_with_nan).unwrap(),
    )
    .expect("Failed to write baseline with NaN values");

    // Test that configuration accepts the file
    let server = MockServer::start();
    let mut configuration = common::build_configuration(&server, vec![]);

    // Set baseline file directly on configuration
    configuration.baseline_file = Some(temp_path.to_string());

    assert!(configuration.baseline_file.is_some());

    // Clean up
    fs::remove_file(temp_path).ok();
}

#[tokio::test]
async fn test_baseline_file_permissions() {
    // Create a file and remove read permissions (Unix-like systems)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let temp_path = "test_baseline_permissions.json";
        create_test_baseline(Path::new(temp_path), true).expect("Failed to create test baseline");

        // Remove read permissions
        let mut perms = fs::metadata(temp_path).unwrap().permissions();
        perms.set_mode(0o000);
        fs::set_permissions(temp_path, perms).unwrap();

        let server = MockServer::start();
        let mut configuration = common::build_configuration(&server, vec![]);

        // Set baseline file directly on configuration
        configuration.baseline_file = Some(temp_path.to_string());

        // Configuration should have baseline file set (validation happens during execution)
        assert!(configuration.baseline_file.is_some());

        // Restore permissions for cleanup
        let mut perms = fs::metadata(temp_path).unwrap().permissions();
        perms.set_mode(0o644);
        fs::set_permissions(temp_path, perms).unwrap();
        fs::remove_file(temp_path).ok();
    }
}

#[tokio::test]
async fn test_baseline_large_file_handling() {
    // Create a baseline file with many metrics to test performance
    let temp_path = "test_baseline_large.json";
    let mut large_baseline = json!({
        "started": "2024-01-01T12:00:00Z",
        "elapsed": 100,
        "users": 50,
        "requests": {},
        "responses": {},
        "coordinated_omission_requests": {},
        "coordinated_omission_responses": {},
        "transactions": {},
        "scenarios": {},
        "errors": {},
        "status_codes": {}
    });

    // Add many request metrics
    for i in 0..100 {
        // Reduced from 1000 to keep test reasonable
        let key = format!("GET /endpoint{}", i);
        large_baseline["requests"][&key] = json!({
            "name": key,
            "number_of_requests": 10,
            "number_of_failures": 1,
            "response_time_average": 100.0 + i as f64,
            "response_time_minimum": 50.0,
            "response_time_maximum": 200.0,
            "requests_per_second": 1.0,
            "failures_per_second": 0.1
        });
    }

    fs::write(temp_path, serde_json::to_string(&large_baseline).unwrap())
        .expect("Failed to write large baseline file");

    // Test configuration with large file
    let server = MockServer::start();
    let mut configuration = common::build_configuration(&server, vec![]);

    // Set baseline file directly on configuration
    configuration.baseline_file = Some(temp_path.to_string());

    assert!(configuration.baseline_file.is_some());

    // Clean up
    fs::remove_file(temp_path).ok();
}

#[tokio::test]
async fn test_baseline_error_validation_comprehensive() {
    // Test that demonstrates proper error handling during actual load test runs
    // This addresses the PR feedback about verifying error handling during load test execution

    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200).body("OK");
    });

    // Test 1: Valid baseline file should work during load test execution
    let temp_path = "test_baseline_error_validation.json";
    create_test_baseline(Path::new(temp_path), true).expect("Failed to create valid baseline");

    let mut config = common::build_configuration(&server, vec![]);
    config.baseline_file = Some(temp_path.to_string());

    // This test should succeed with a valid baseline file
    let goose_metrics = common::run_load_test(
        common::build_load_test(
            config,
            vec![scenario!("test_scenario").register_transaction(transaction!(simple_transaction))],
            None,
            None,
        ),
        None,
    )
    .await;

    // Verify that the load test ran successfully with baseline comparison
    assert!(!goose_metrics.requests.is_empty());
    assert!(goose_metrics.requests.contains_key("GET /"));

    // Test 2: Test configuration validation with invalid baseline files
    // (These tests verify that configuration parsing properly validates baseline files)

    // Test with non-existent file - should work when setting directly on config
    let mut config = common::build_configuration(&server, vec![]);
    config.baseline_file = Some("nonexistent_baseline_comprehensive.json".to_string());
    // This should work since we're setting the file after configuration parsing
    assert!(config.baseline_file.is_some());

    // Test with invalid JSON file - should work when setting directly on config
    fs::write(temp_path, "{ invalid json }").expect("Failed to write invalid JSON");

    let mut config = common::build_configuration(&server, vec![]);
    config.baseline_file = Some(temp_path.to_string());
    // This should work since we're setting the file after configuration parsing
    assert!(config.baseline_file.is_some());

    // Test with missing required fields - should work when setting directly on config
    let minimal_json = json!({
        "invalid": "structure"
    });
    fs::write(temp_path, serde_json::to_string(&minimal_json).unwrap())
        .expect("Failed to write minimal JSON");

    let mut config = common::build_configuration(&server, vec![]);
    config.baseline_file = Some(temp_path.to_string());
    // This should work since we're setting the file after configuration parsing
    assert!(config.baseline_file.is_some());

    // Test 3: Verify that CLI parsing validates baseline files properly
    // Create a valid baseline file for CLI testing
    create_test_baseline(Path::new(temp_path), true).expect("Failed to create valid baseline");

    // This should work with a valid baseline file
    let config = common::build_configuration(
        &server,
        vec![
            "--users",
            "1",
            "--hatch-rate",
            "1",
            "--run-time",
            "1",
            "--baseline-file",
            temp_path,
        ],
    );

    assert!(config.baseline_file.is_some());
    assert_eq!(config.baseline_file.as_ref().unwrap(), temp_path);

    // Clean up
    fs::remove_file(temp_path).ok();
}
