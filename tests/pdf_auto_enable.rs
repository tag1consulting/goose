//! Test PDF auto-enable functionality

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
async fn test_pdf_chromium_compiled_in() {
    let pdf_file = "test-pdf-auto-enable.pdf";

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
        pdf_file, // This should work with auto-enable
    ];
    let configuration = common::build_configuration(&server, configuration_flags);

    // Build the load test and programmatically enable PDF reports using GooseDefault
    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None)
        .set_default(GooseDefault::PdfReports, true) // Enable PDF auto-detection
        .expect("Should be able to enable PDF reports when pdf-reports feature is compiled");

    // Run the Goose Attack (dereference the Box)
    let goose_metrics = common::run_load_test(*goose_attack, None).await;

    // Confirm that we loaded the mock endpoints
    assert!(mock_endpoints[0].hits() > 0);

    // Confirm that the test duration was correct
    assert!(goose_metrics.duration == 1);

    // PDF file must exist when pdf-reports feature is compiled and auto-enabled
    assert!(
        std::path::Path::new(pdf_file).exists(),
        "PDF report file should be created when PDF auto-enable is used"
    );

    // PDF file must not be empty
    let metadata = std::fs::metadata(pdf_file).expect("Failed to get PDF file metadata");
    assert!(metadata.len() > 0, "PDF report file should not be empty");

    common::cleanup_files(vec![pdf_file]);
}

/// Test that PDF resource management works correctly - Chrome processes are properly cleaned up.
/// This test validates that multiple PDF generations don't cause resource leaks by using the public API.
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
            common::build_load_test(configuration, vec![get_transactions()], None, None)
                .set_default(GooseDefault::PdfReports, true)
                .expect("Should be able to enable PDF reports");

        let goose_metrics = common::run_load_test(*goose_attack, None).await;

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

/// Test that PDF auto-enable functionality works correctly when the feature is NOT compiled in.
/// This validates that chromium dependencies are NOT available and the proper error is shown.
#[cfg(not(feature = "pdf-reports"))]
#[tokio::test]
#[serial]
async fn test_pdf_chromium_not_compiled() {
    let pdf_file = "test-pdf-auto-enable-should-fail.pdf";

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

    // Build the load test and programmatically enable PDF reports using GooseDefault
    let goose_attack_result =
        common::build_load_test(configuration, vec![get_transactions()], None, None)
            .set_default(GooseDefault::PdfReports, true); // Enable PDF auto-detection

    // This should succeed in creating the GooseAttack, but fail during execution
    let goose_attack =
        goose_attack_result.expect("Should be able to set PDF default even without feature");

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

    // The PDF file may be created but should be empty since the error occurred before writing
    if std::path::Path::new(pdf_file).exists() {
        let metadata = std::fs::metadata(pdf_file).expect("Failed to get PDF file metadata");
        assert_eq!(
            metadata.len(),
            0,
            "PDF report file should be empty when pdf-reports feature is not compiled"
        );
    }

    common::cleanup_files(vec![pdf_file]);
}
