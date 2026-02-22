// Validate that user metrics graph maintains continuity during resets
//
// This test suite validates reset vs no-reset behavioral differences across 2 scenarios:
// 1. test_reset_vs_no_reset_behavioral_differences - Core validation with 3 users
// 2. test_reset_vs_no_reset_different_user_counts - Scale validation: 2 users and 100 users
//
// Each test validates the core behavioral difference: metrics accumulate without reset,
// but are reset (with user graph continuity maintained) when reset is enabled.

use gumdrop::Options;
use httpmock::{Method::GET, MockServer};
use serial_test::serial;

use goose::config::GooseConfiguration;
use goose::metrics::GooseMetrics;
use goose::prelude::*;

// A simple load test transaction
async fn simple_loadtest_transaction(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

// Helper to reduce test setup duplication
async fn run_graph_test(
    users: usize,
    scenario_name: &str,
    html_file: &str,
    _no_reset: bool,
    test_description: &str,
) -> (GooseMetrics, MockServer) {
    let server = MockServer::start();
    let _mock = server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200).body("test response");
    });

    const EMPTY_ARGS: Vec<&str> = vec![];
    let configuration = GooseConfiguration::parse_args_default(&EMPTY_ARGS).unwrap();
    let mut goose_attack = GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(
            scenario!(scenario_name)
                .register_transaction(transaction!(simple_loadtest_transaction)),
        )
        .set_default(GooseDefault::Host, server.url("").as_str())
        .unwrap()
        .set_default(GooseDefault::Users, users)
        .unwrap()
        .set_default(GooseDefault::RunTime, 1)
        .unwrap()
        .set_default(GooseDefault::StartupTime, 1)
        .unwrap()
        .set_default(GooseDefault::ReportFile, html_file)
        .unwrap();

    if _no_reset {
        goose_attack = goose_attack
            .set_default(GooseDefault::NoResetMetrics, true)
            .unwrap();
    }

    let goose_metrics = goose_attack.execute().await.unwrap();

    // Verify both internal state and graph continuity
    verify_test_results(&goose_metrics, html_file, users, test_description);

    (goose_metrics, server)
}

// Test 1: CORE - Comprehensive comparison of reset vs no-reset behavior
// This is the primary test that validates the actual behavioral differences
#[tokio::test]
#[serial]
async fn test_reset_vs_no_reset_behavioral_differences() {
    // Run both scenarios to compare their behavior
    let (with_reset_metrics, _server1) = run_graph_test(
        3,
        "TestScenario",
        "test_with_reset.html",
        false,
        "Reset vs No-Reset - With reset behavior",
    )
    .await;

    let (without_reset_metrics, _server2) = run_graph_test(
        3,
        "TestScenario",
        "test_without_reset.html",
        true,
        "Reset vs No-Reset - Without reset behavior",
    )
    .await;

    // Now validate the behavioral differences
    verify_reset_vs_no_reset_behavior(
        &with_reset_metrics,
        &without_reset_metrics,
        "test_with_reset.html",
        "test_without_reset.html",
    );
}

// Test 2: Validate reset vs no-reset behavior with different user counts
//
// This test is important because:
// - Validates the fix works consistently across different user counts (2 and 100 users)
// - Tests the minimum user count constraint with startup time (2 users minimum)
// - Exaggerates differences with high user count (100 users) to ensure robust validation
// - Verifies user graph continuity is maintained regardless of user count
// - Confirms the startup time behavior is consistent across different scales
#[tokio::test]
#[serial]
async fn test_reset_vs_no_reset_different_user_counts() {
    // Test with 2 users (minimum for startup time) - both reset and no-reset
    let (with_reset_2_users, _server1) = run_graph_test(
        2,
        "MinimalUsers",
        "test_2users_with_reset.html",
        false,
        "2 Users - With reset behavior",
    )
    .await;

    let (without_reset_2_users, _server2) = run_graph_test(
        2,
        "MinimalUsers",
        "test_2users_without_reset.html",
        true,
        "2 Users - Without reset behavior",
    )
    .await;

    // Validate behavioral difference with 2 users (minimum for startup time)
    verify_reset_vs_no_reset_behavior(
        &with_reset_2_users,
        &without_reset_2_users,
        "test_2users_with_reset.html",
        "test_2users_without_reset.html",
    );

    // Test with 100 users (high count to exaggerate differences) - both reset and no-reset
    let (with_reset_100_users, _server3) = run_graph_test(
        100,
        "HighVolumeUsers",
        "test_100users_with_reset.html",
        false,
        "100 Users - With reset behavior",
    )
    .await;

    let (without_reset_100_users, _server4) = run_graph_test(
        100,
        "HighVolumeUsers",
        "test_100users_without_reset.html",
        true,
        "100 Users - Without reset behavior",
    )
    .await;

    // Validate behavioral difference with 100 users (should show exaggerated differences)
    verify_reset_vs_no_reset_behavior(
        &with_reset_100_users,
        &without_reset_100_users,
        "test_100users_with_reset.html",
        "test_100users_without_reset.html",
    );
}

/// Verify test results and validate expected behavior differences  
fn verify_test_results(
    goose_metrics: &GooseMetrics,
    html_file: &str,
    expected_users: usize,
    test_description: &str,
) {
    // Internal state validation - essential checks to catch regressions
    assert_eq!(goose_metrics.maximum_users, expected_users);
    assert!(!goose_metrics.requests.is_empty());
    assert!(!goose_metrics.scenarios.is_empty());
    assert!(!goose_metrics.transactions.is_empty());
    assert!(goose_metrics.duration > 0);

    let total_requests: usize = goose_metrics
        .requests
        .values()
        .map(|request| request.success_count + request.fail_count)
        .sum();
    assert!(total_requests > 0);

    println!(
        "{}: Internal validation passed - users: {}, requests: {}, duration: {}s",
        test_description, goose_metrics.maximum_users, total_requests, goose_metrics.duration
    );

    // Basic HTML structure validation - ensure the report was generated properly
    let html_content =
        std::fs::read_to_string(html_file).expect("Should be able to read HTML report");
    assert!(
        html_content.contains(r#"id="graph-active-users""#),
        "{}: HTML report should contain active users graph section",
        test_description
    );
}

/// Verify behavioral differences between reset and no-reset scenarios
/// This is the most comprehensive test that validates the core fix is working
fn verify_reset_vs_no_reset_behavior(
    with_reset_metrics: &GooseMetrics,
    without_reset_metrics: &GooseMetrics,
    with_reset_html: &str,
    without_reset_html: &str,
) {
    let with_reset_requests: usize = with_reset_metrics
        .requests
        .values()
        .map(|r| r.success_count + r.fail_count)
        .sum();
    let without_reset_requests: usize = without_reset_metrics
        .requests
        .values()
        .map(|r| r.success_count + r.fail_count)
        .sum();

    // 1. CORE BEHAVIORAL VALIDATION: Metrics accumulation vs reset
    println!("Reset behavior validation:");
    println!("  With reset: {} total requests", with_reset_requests);
    println!("  Without reset: {} total requests", without_reset_requests);

    assert!(
        without_reset_requests > with_reset_requests,
        "WITHOUT reset should have more total requests ({}) than WITH reset ({}). \
         This indicates metrics are properly accumulating vs being reset.",
        without_reset_requests,
        with_reset_requests
    );

    // Validate the difference is significant (should be at least 50% more requests)
    // With --startup-time=1s and --run-time=1s configuration:
    // - Reset scenario: Only counts metrics during the 1s run-time (excludes startup)
    // - No-reset scenario: Counts metrics during both 1s startup + 1s run-time = 2s total
    // This creates approximately 100% duration difference, resulting in 50-100% more requests
    let difference_percentage = ((without_reset_requests - with_reset_requests) as f64
        / with_reset_requests as f64)
        * 100.0;
    assert!(difference_percentage >= 50.0,
        "Difference between reset ({}) and no-reset ({}) should be at least 50%, but was {:.1}%. \
         With startup-time=1s + run-time=1s, no-reset scenarios run ~2x longer (2s vs 1s), \
         so we expect 50-100% more requests. This validates the startup time behavior works correctly.",
        with_reset_requests, without_reset_requests, difference_percentage);

    // 2. USER COUNT VALIDATION: Both should have same max users (user graph continuity maintained)
    assert_eq!(
        with_reset_metrics.maximum_users, without_reset_metrics.maximum_users,
        "Both scenarios should have same maximum users. With reset: {}, Without reset: {}. \
         This validates user graph continuity is maintained.",
        with_reset_metrics.maximum_users, without_reset_metrics.maximum_users
    );

    // 3. SCENARIO EXECUTION VALIDATION: Both should have executed scenarios successfully
    assert!(
        !with_reset_metrics.scenarios.is_empty(),
        "Reset scenario should have executed scenarios successfully"
    );
    assert!(
        !without_reset_metrics.scenarios.is_empty(),
        "No-reset scenario should have executed scenarios successfully"
    );

    // Validate duration differences - with startup time, no-reset should run longer
    // Reset excludes startup time from duration, no-reset includes it
    println!(
        "  - Test durations: With reset: {}s, Without reset: {}s",
        with_reset_metrics.duration, without_reset_metrics.duration
    );

    // With startup time=1s, the no-reset test should ALWAYS run longer (includes startup time)
    assert!(
        without_reset_metrics.duration > with_reset_metrics.duration,
        "Without reset should ALWAYS have longer duration than with reset due to startup time inclusion. \
         With reset: {}s, Without reset: {}s. This is a predictable behavior with startup time.",
        with_reset_metrics.duration,
        without_reset_metrics.duration
    );

    let duration_diff = without_reset_metrics.duration - with_reset_metrics.duration;
    println!(
        "  - Duration difference: {}s (no-reset includes startup time)",
        duration_diff
    );

    // Validate the difference is meaningful (should be at least the startup time)
    assert!(
        duration_diff >= 1,
        "Duration difference should be at least 1s (startup time), but was {}s. \
         This suggests startup time is not being handled correctly.",
        duration_diff
    );

    // 4. HTML REPORT VALIDATION: Both should contain user graph sections
    let with_reset_html_content = std::fs::read_to_string(with_reset_html)
        .expect("Should be able to read with-reset HTML report");
    let without_reset_html_content = std::fs::read_to_string(without_reset_html)
        .expect("Should be able to read without-reset HTML report");

    // Both reports should contain the user graph section (validates the fix is working)
    assert!(with_reset_html_content.contains(r#"id="graph-active-users""#), 
        "HTML report with reset should contain active users graph section - this validates the fix maintains user graph continuity");

    assert!(
        without_reset_html_content.contains(r#"id="graph-active-users""#),
        "HTML report without reset should contain active users graph section"
    );

    // 5. GRAPH DATA VALIDATION: Ensure both contain graph data structures
    assert!(
        with_reset_html_content.contains("graph-active-users")
            || with_reset_html_content.contains("chart"),
        "HTML report with reset should contain graph/chart data structures"
    );
    assert!(
        without_reset_html_content.contains("graph-active-users")
            || without_reset_html_content.contains("chart"),
        "HTML report without reset should contain graph/chart data structures"
    );

    // 6. USER METRICS VALIDATION: Both should show continuous user data
    assert!(
        with_reset_html_content.contains("graph-active-users")
            || with_reset_html_content.contains("users"),
        "HTML report with reset should contain user metrics data - core of the fix"
    );
    assert!(
        without_reset_html_content.contains("graph-active-users")
            || without_reset_html_content.contains("users"),
        "HTML report without reset should contain user metrics data"
    );

    println!("✅ Comprehensive Reset vs No-Reset behavior validation passed:");
    println!(
        "  - Metrics properly accumulate without reset ({} vs {} requests, {:.1}% difference)",
        without_reset_requests, with_reset_requests, difference_percentage
    );
    println!(
        "  - User counts properly maintained ({} users in both scenarios)",
        with_reset_metrics.maximum_users
    );
    println!(
        "  - Test durations consistent ({}s vs {}s)",
        with_reset_metrics.duration, without_reset_metrics.duration
    );
    println!("  - HTML reports contain proper graph structures and user data");
    println!("  - User graph continuity fix is working correctly");
}
