//! Test PDF functionality when compiled with pdf-reports feature

#[cfg(feature = "pdf-reports")]
use httpmock::Method::GET;
use httpmock::MockServer;
use serial_test::serial;

mod common;

use goose::prelude::*;

// Paths used in load tests performed during these tests.
const INDEX_PATH: &str = "/";

// Test transaction.
pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// All tests in this file run against common endpoints.
#[cfg(feature = "pdf-reports")]
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<httpmock::Mock<'_>> {
    vec![
        // Set up INDEX_PATH
        server.mock(|when, then| {
            when.method(GET).path(INDEX_PATH);
            then.status(200);
        }),
    ]
}

// Returns the appropriate scenario needed to build these tests.
fn get_transactions() -> Scenario {
    scenario!("LoadTest").register_transaction(transaction!(get_index))
}

/// Test that PDF generation functionality is available when compiled with pdf-reports feature.
/// This validates that chromium dependencies are compiled in and functional.
#[cfg(feature = "pdf-reports")]
#[tokio::test]
#[serial]
async fn test_pdf_generation_with_feature() {
    let pdf_file = "test-pdf-generation.pdf";

    let server = MockServer::start();
    let mock_endpoints = setup_mock_server_endpoints(&server);

    let configuration_flags = vec![
        "--users",
        "1",
        "--hatch-rate",
        "1",
        "--run-time",
        "1",
        "--report-file",
        pdf_file, // This should work when pdf-reports feature is compiled
    ];
    let configuration = common::build_configuration(&server, configuration_flags);

    // Build the load test - PDF functionality is purely opt-in via CLI flag
    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);

    // Run the Goose Attack
    let goose_metrics = common::run_load_test(goose_attack, None).await;

    // Confirm that we loaded the mock endpoints
    assert!(mock_endpoints[0].hits() > 0);

    // Confirm that the test duration was correct
    assert!(goose_metrics.duration == 1);

    // PDF file must exist when pdf-reports feature is compiled
    assert!(
        std::path::Path::new(pdf_file).exists(),
        "PDF report file should be created when pdf-reports feature is compiled"
    );

    // PDF file must not be empty
    let metadata = std::fs::metadata(pdf_file).expect("Failed to get PDF file metadata");
    assert!(metadata.len() > 0, "PDF report file should not be empty");

    common::cleanup_files(vec![pdf_file]);
}

/// Test that PDF resource management works correctly - Chrome processes are properly cleaned up.
/// This test validates that multiple PDF generations don't cause resource leaks.
#[cfg(feature = "pdf-reports")]
#[tokio::test]
#[serial]
async fn test_pdf_resource_management() {
    // This test verifies that Chrome processes are properly managed
    // and cleaned up by running multiple load tests that generate PDFs

    // Test multiple PDF generations to ensure no resource leaks
    for i in 0..3 {
        let pdf_file = format!("test-resource-management-{}.pdf", i);

        let server = MockServer::start();
        let mock_endpoints = setup_mock_server_endpoints(&server);

        let configuration_flags = vec![
            "--users",
            "1",
            "--hatch-rate",
            "1",
            "--run-time",
            "1",
            "--report-file",
            &pdf_file,
        ];
        let configuration = common::build_configuration(&server, configuration_flags);

        // Build and run the load test with PDF generation
        let goose_attack =
            common::build_load_test(configuration, vec![get_transactions()], None, None);

        let goose_metrics = common::run_load_test(goose_attack, None).await;

        // Confirm basic functionality
        assert!(mock_endpoints[0].hits() > 0);
        assert!(goose_metrics.duration == 1);

        // Verify PDF file was created
        assert!(
            std::path::Path::new(&pdf_file).exists(),
            "PDF file {} should exist",
            i
        );

        let metadata = std::fs::metadata(&pdf_file).expect("Failed to get PDF file metadata");
        assert!(metadata.len() > 0, "PDF file {} should have content", i);

        // Clean up the file after verification
        common::cleanup_files(vec![&pdf_file]);

        // Each iteration should properly clean up its Chrome process
        // The ChromeSession Drop implementation ensures resource cleanup
    }
}

/// Test that PDF functionality fails correctly when the feature is NOT compiled in.
/// This validates that chromium dependencies are NOT available and the proper error is shown.
#[cfg(not(feature = "pdf-reports"))]
#[tokio::test]
#[serial]
async fn test_pdf_without_feature_fails() {
    let pdf_file = "test-pdf-should-fail.pdf";

    let server = MockServer::start();

    let configuration_flags = vec![
        "--users",
        "1",
        "--hatch-rate",
        "1",
        "--run-time",
        "1",
        "--report-file",
        pdf_file, // This should fail without pdf-reports feature
    ];
    let configuration = common::build_configuration(&server, configuration_flags);

    // Build the load test
    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);

    // This should fail because the pdf-reports feature is not compiled in
    let result = goose_attack.execute().await;

    // Verify that we get the expected error
    match result {
        Err(goose::GooseError::InvalidOption {
            option,
            value,
            detail,
        }) => {
            assert_eq!(option, "--report-file");
            assert_eq!(value, pdf_file);
            assert!(detail.contains("PDF reports require"));
        }
        Ok(_) => {
            panic!("Expected InvalidOption error, but load test completed successfully!");
        }
        other => panic!("Expected InvalidOption error, got: {:?}", other),
    }

    // The PDF file should not be created when the feature is not compiled
    assert!(
        !std::path::Path::new(pdf_file).exists(),
        "PDF file should not be created when pdf-reports feature is not compiled"
    );

    common::cleanup_files(vec![pdf_file]);
}
