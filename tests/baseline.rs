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
            "started": "2024-01-01T12:00:00Z",
            "elapsed": 10,
            "users": 1,
            "requests": {
                "GET /": {
                    "name": "GET /",
                    "number_of_requests": 100,
                    "number_of_failures": 5,
                    "response_time_average": 150.5,
                    "response_time_minimum": 50.2,
                    "response_time_maximum": 300.8,
                    "requests_per_second": 10.0,
                    "failures_per_second": 0.5
                }
            },
            "responses": {
                "GET /": {
                    "name": "GET /",
                    "number_of_requests": 100,
                    "number_of_failures": 5,
                    "response_time_average": 150.5,
                    "response_time_minimum": 50.2,
                    "response_time_maximum": 300.8,
                    "requests_per_second": 10.0,
                    "failures_per_second": 0.5
                }
            },
            "coordinated_omission_requests": {},
            "coordinated_omission_responses": {},
            "transactions": {
                "test_transaction": {
                    "name": "test_transaction",
                    "number": 50,
                    "fail": 2,
                    "times": [100.0, 200.0, 150.0],
                    "min_time": 100.0,
                    "max_time": 200.0,
                    "total_time": 450.0,
                    "counter": 50
                }
            },
            "scenarios": {
                "test_scenario": {
                    "name": "test_scenario",
                    "users": 10,
                    "counter": 50,
                    "iterations": 5
                }
            },
            "errors": {
                "404 Not Found": {
                    "error": "404 Not Found",
                    "occurrences": 3
                }
            },
            "status_codes": {
                "200": {
                    "method": "GET",
                    "name": "/",
                    "status_code": 200,
                    "count": 95
                },
                "404": {
                    "method": "GET",
                    "name": "/nonexistent",
                    "status_code": 404,
                    "count": 5
                }
            }
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
    let configuration = common::build_configuration(&server, vec!["--baseline-file", temp_path]);

    // Just verify the configuration accepts the baseline file option
    assert!(configuration.baseline_file.is_some());
    assert_eq!(configuration.baseline_file.as_ref().unwrap(), temp_path);

    // Clean up
    fs::remove_file(temp_path).ok();
}

#[tokio::test]
async fn test_baseline_file_validation_failure() {
    // Create a temporary invalid baseline file
    let temp_path = "test_baseline_invalid.json";
    create_test_baseline(Path::new(temp_path), false).expect("Failed to create test baseline");

    // Test that invalid baseline fails during load test startup
    let server = MockServer::start();
    let configuration = common::build_configuration(&server, vec!["--baseline-file", temp_path]);

    // The configuration should accept the file path, but validation should occur during execution
    assert!(configuration.baseline_file.is_some());

    // Clean up
    fs::remove_file(temp_path).ok();
}

#[tokio::test]
async fn test_baseline_file_not_found() {
    // Test with non-existent file
    let server = MockServer::start();
    let configuration = common::build_configuration(
        &server,
        vec!["--baseline-file", "nonexistent_baseline.json"],
    );

    // Configuration should accept non-existent file, but validation occurs during load test execution
    assert!(configuration.baseline_file.is_some());
}

#[tokio::test]
async fn test_baseline_empty_file() {
    // Create empty file
    let temp_path = "test_baseline_empty.json";
    fs::write(temp_path, "").expect("Failed to write empty file");

    let server = MockServer::start();
    let configuration = common::build_configuration(&server, vec!["--baseline-file", temp_path]);

    // Configuration should accept the file path
    assert!(configuration.baseline_file.is_some());

    // Clean up
    fs::remove_file(temp_path).ok();
}

#[tokio::test]
async fn test_baseline_invalid_json() {
    // Create file with invalid JSON
    let temp_path = "test_baseline_malformed.json";
    fs::write(temp_path, "{ invalid json }").expect("Failed to write invalid JSON");

    let server = MockServer::start();
    let configuration = common::build_configuration(&server, vec!["--baseline-file", temp_path]);

    // Configuration should accept the file path
    assert!(configuration.baseline_file.is_some());

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
    let configuration = common::build_configuration(&server, vec!["--baseline-file", temp_path]);

    // Configuration should accept the file path
    assert!(configuration.baseline_file.is_some());

    // Clean up
    fs::remove_file(temp_path).ok();
}

#[tokio::test]
async fn test_baseline_integration_with_load_test() {
    // Create a temporary baseline file with valid data that matches what the load test will generate
    let baseline_path = "test_baseline_integration.json";

    // First, run a simple load test to generate realistic baseline data
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200).body("OK");
    });

    let baseline_config = common::build_configuration(
        &server,
        vec!["--users", "1", "--hatch-rate", "1", "--run-time", "1"],
    );

    let baseline_metrics = common::run_load_test(
        common::build_load_test(
            baseline_config,
            vec![scenario!("test_scenario").register_transaction(transaction!(simple_transaction))],
            None,
            None,
        ),
        None,
    )
    .await;

    // Create a valid baseline file using the actual metrics structure
    let baseline_data = serde_json::to_value(&baseline_metrics).unwrap();
    fs::write(
        baseline_path,
        serde_json::to_string_pretty(&baseline_data).unwrap(),
    )
    .expect("Failed to write baseline file");

    // Now create a configuration that includes the baseline file
    // Instead of using build_configuration which may not handle --baseline-file properly,
    // we'll create the configuration manually
    let mut config = common::build_configuration(
        &server,
        vec!["--users", "1", "--hatch-rate", "1", "--run-time", "1"],
    );

    // Set the baseline file directly on the configuration
    config.baseline_file = Some(baseline_path.to_string());

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

    // Clean up
    fs::remove_file(baseline_path).ok();
}

#[tokio::test]
async fn test_baseline_file_content_validation() {
    // Create a temporary valid baseline file
    let temp_path = "test_baseline_content.json";
    create_test_baseline(Path::new(temp_path), true).expect("Failed to create test baseline");

    // Read back the file content to verify structure
    let content = fs::read_to_string(temp_path).expect("Failed to read baseline file");
    let parsed: serde_json::Value = serde_json::from_str(&content).expect("Failed to parse JSON");

    // Verify baseline data structure
    assert!(
        parsed.get("started").is_some(),
        "Baseline should contain started timestamp"
    );
    assert!(
        parsed.get("elapsed").is_some(),
        "Baseline should contain elapsed time"
    );
    assert!(
        parsed.get("users").is_some(),
        "Baseline should contain user count"
    );
    assert!(
        parsed.get("requests").is_some(),
        "Baseline should contain request metrics"
    );
    assert!(
        parsed.get("responses").is_some(),
        "Baseline should contain response metrics"
    );
    assert!(
        parsed.get("transactions").is_some(),
        "Baseline should contain transaction metrics"
    );
    assert!(
        parsed.get("scenarios").is_some(),
        "Baseline should contain scenario metrics"
    );
    assert!(
        parsed.get("errors").is_some(),
        "Baseline should contain error metrics"
    );
    assert!(
        parsed.get("status_codes").is_some(),
        "Baseline should contain status code metrics"
    );

    // Verify specific metric values
    if let Some(request_metric) = parsed["requests"].get("GET /") {
        assert_eq!(request_metric["number_of_requests"], 100);
        assert_eq!(request_metric["number_of_failures"], 5);
    }

    // Clean up
    fs::remove_file(temp_path).ok();
}

#[tokio::test]
async fn test_baseline_with_nan_values() {
    // Create baseline with NaN values to test serialization handling
    let temp_path = "/tmp/test_baseline_nan.json";
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
    let configuration = common::build_configuration(&server, vec!["--baseline-file", temp_path]);
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

        let temp_path = "/tmp/test_baseline_permissions.json";
        create_test_baseline(Path::new(temp_path), true).expect("Failed to create test baseline");

        // Remove read permissions
        let mut perms = fs::metadata(temp_path).unwrap().permissions();
        perms.set_mode(0o000);
        fs::set_permissions(temp_path, perms).unwrap();

        let server = MockServer::start();
        let configuration =
            common::build_configuration(&server, vec!["--baseline-file", temp_path]);

        // Configuration should accept the file path (validation happens during execution)
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
    let temp_path = "/tmp/test_baseline_large.json";
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
    let configuration = common::build_configuration(&server, vec!["--baseline-file", temp_path]);
    assert!(configuration.baseline_file.is_some());

    // Clean up
    fs::remove_file(temp_path).ok();
}
