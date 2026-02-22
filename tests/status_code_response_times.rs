use httpmock::Method::GET;
use httpmock::MockServer;

mod common;

use goose::prelude::*;

// Paths used in load tests.
const OK_PATH: &str = "/ok";
const ERROR_PATH: &str = "/error";

// Transaction that hits both OK and ERROR paths but tracks them under the
// same request name, producing multiple status codes for one metric key.
async fn get_mixed(user: &mut GooseUser) -> TransactionResult {
    // Two out of three requests succeed.
    let _goose = user.get_named(OK_PATH, "mixed endpoint").await?;
    let _goose = user.get_named(OK_PATH, "mixed endpoint").await?;
    // One out of three requests fails.
    let _goose = user.get_named(ERROR_PATH, "mixed endpoint").await?;
    Ok(())
}

// Transaction that only hits the OK path (single status code).
async fn get_success(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(OK_PATH).await?;
    Ok(())
}

/// Verify that per-status-code timing data is recorded and that
/// `status_code_breakdowns()` returns data when multiple status codes exist.
#[tokio::test]
async fn test_status_code_response_time_tracking() {
    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(GET).path(OK_PATH);
        then.status(200);
    });
    server.mock(|when, then| {
        when.method(GET).path(ERROR_PATH);
        then.status(404);
    });

    let config = common::build_configuration(
        &server,
        vec!["--users", "2", "--hatch-rate", "2", "--run-time", "2"],
    );

    let metrics = common::run_load_test(
        common::build_load_test(
            config,
            vec![
                scenario!("Mixed")
                    .register_transaction(transaction!(get_mixed).set_weight(3).unwrap()),
                scenario!("Success")
                    .register_transaction(transaction!(get_success).set_weight(1).unwrap()),
            ],
            None,
            None,
        ),
        None,
    )
    .await;

    // --- "mixed endpoint" should have both 200 and 404 status codes ---
    let mixed = metrics
        .requests
        .get("GET mixed endpoint")
        .expect("mixed endpoint request should exist");

    assert!(
        mixed.status_code_counts.contains_key(&200),
        "mixed endpoint should have 200 status codes"
    );
    assert!(
        mixed.status_code_counts.contains_key(&404),
        "mixed endpoint should have 404 status codes"
    );

    // Timing summaries should exist for both status codes.
    assert_eq!(
        mixed.status_code_timings.len(),
        2,
        "mixed endpoint should have timing data for 2 status codes"
    );
    assert!(
        mixed.status_code_timings.contains_key(&200),
        "mixed endpoint should have 200 timing data"
    );
    assert!(
        mixed.status_code_timings.contains_key(&404),
        "mixed endpoint should have 404 timing data"
    );

    // Verify timing summary counts are consistent.
    let timing_200 = &mixed.status_code_timings[&200];
    let timing_404 = &mixed.status_code_timings[&404];
    assert!(timing_200.count > 0, "200 timing should have recordings");
    assert!(timing_404.count > 0, "404 timing should have recordings");
    assert!(timing_200.min_time > 0, "200 timing min should be non-zero");
    assert!(
        timing_200.max_time >= timing_200.min_time,
        "200 timing max should be >= min"
    );

    // The breakdown helper should return data.
    let breakdowns = mixed
        .status_code_breakdowns()
        .expect("mixed endpoint should have breakdowns");
    assert_eq!(breakdowns.len(), 2);
    assert_eq!(breakdowns[0].status_code, 200);
    assert_eq!(breakdowns[1].status_code, 404);
    // Percentages should be roughly 2/3 and 1/3.
    assert!(
        breakdowns[0].percentage > 50.0,
        "200 should be the majority: {:.1}%",
        breakdowns[0].percentage
    );
    assert!(
        breakdowns[1].percentage < 50.0,
        "404 should be the minority: {:.1}%",
        breakdowns[1].percentage
    );

    // --- "/ok" endpoint (from the Success scenario) should have only one status code ---
    let success = metrics
        .requests
        .get(&format!("GET {OK_PATH}"))
        .expect("success endpoint request should exist");

    assert_eq!(
        success.status_code_timings.len(),
        1,
        "single-status-code endpoint should have exactly one timing entry"
    );
    // Breakdown helper should return None for single status code.
    assert!(
        success.status_code_breakdowns().is_none(),
        "single status code endpoint should not have breakdowns"
    );
}

/// Verify that breakdown rows appear in HTML and Markdown reports.
#[tokio::test]
async fn test_status_code_breakdowns_in_reports() {
    let html_file = "status-code-report-test.html";
    let md_file = "status-code-report-test.md";
    common::cleanup_files(vec![html_file, md_file]);

    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(GET).path(OK_PATH);
        then.status(200);
    });
    server.mock(|when, then| {
        when.method(GET).path(ERROR_PATH);
        then.status(404);
    });

    let config = common::build_configuration(
        &server,
        vec![
            "--users",
            "2",
            "--hatch-rate",
            "2",
            "--run-time",
            "2",
            "--report-file",
            html_file,
            "--report-file",
            md_file,
        ],
    );

    let _metrics = common::run_load_test(
        common::build_load_test(
            config,
            vec![scenario!("Mixed")
                .register_transaction(transaction!(get_mixed).set_weight(3).unwrap())],
            None,
            None,
        ),
        None,
    )
    .await;

    // --- Verify HTML report contains breakdown rows ---
    assert!(
        std::path::Path::new(html_file).exists(),
        "HTML report should be generated"
    );
    let html = std::fs::read_to_string(html_file).unwrap();
    assert!(
        html.contains("status-breakdown"),
        "HTML report should contain status-breakdown CSS class"
    );
    assert!(
        html.contains("└─"),
        "HTML report should contain tree-style breakdown prefix"
    );

    // --- Verify Markdown report contains breakdown rows ---
    assert!(
        std::path::Path::new(md_file).exists(),
        "Markdown report should be generated"
    );
    let md = std::fs::read_to_string(md_file).unwrap();
    assert!(
        md.contains("└─"),
        "Markdown report should contain tree-style breakdown prefix"
    );
    // Breakdown rows should contain status code and percentage.
    assert!(
        md.contains("200") && md.contains("404"),
        "Markdown report should contain both status codes"
    );

    common::cleanup_files(vec![html_file, md_file]);
}
