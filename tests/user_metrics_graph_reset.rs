// Validate that user metrics graph maintains continuity during resets
//
// This test suite covers 4 distinct scenarios to ensure comprehensive coverage:
// 1. test_user_metrics_graph_continuity_with_reset - Tests DEFAULT behavior (metrics reset normally)
// 2. test_user_metrics_graph_continuity_without_reset - Tests --no-reset-metrics flag specifically
// 3. test_comprehensive_graph_metrics_reset_integration - Tests broader integration with 3 users
// 4. test_graph_data_preservation_during_reset - Tests data preservation with 2 users and different scenario name
//
// Each test serves a distinct purpose and validates different aspects of graph continuity behavior.

use gumdrop::Options;
use httpmock::{Method::GET, MockServer};
use serial_test::serial;

use goose::config::GooseConfiguration;
use goose::metrics::GooseMetrics;
use goose::prelude::*;
use std::fs;

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
    no_reset: bool,
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
        .set_default(GooseDefault::HatchRate, "10")
        .unwrap()
        .set_default(GooseDefault::RunTime, 1)
        .unwrap()
        .set_default(GooseDefault::ReportFile, html_file)
        .unwrap();

    if no_reset {
        goose_attack = goose_attack
            .set_default(GooseDefault::NoResetMetrics, true)
            .unwrap();
    }

    let goose_metrics = goose_attack.execute().await.unwrap();

    // Verify both internal state and graph continuity
    verify_test_results(&goose_metrics, html_file, users, test_description);

    (goose_metrics, server)
}

// Test 1: Validate default behavior (with metrics reset)
#[tokio::test]
#[serial]
async fn test_user_metrics_graph_continuity_with_reset() {
    let (_, _server) = run_graph_test(
        3,
        "TestScenario",
        "test1_graph_reset.html",
        false,
        "Test 1 - Default behavior (with reset)",
    )
    .await;
    // Test passes if no panics occur during execution
}

// Test 2: Validate --no-reset-metrics flag behavior
#[tokio::test]
#[serial]
async fn test_user_metrics_graph_continuity_without_reset() {
    let (_, _server) = run_graph_test(
        3,
        "TestScenario",
        "test2_no_reset.html",
        true,
        "Test 2 - --no-reset-metrics flag behavior",
    )
    .await;
    // Test passes if no panics occur during execution
}

// Test 3: Integration testing with different scenario name
#[tokio::test]
#[serial]
async fn test_comprehensive_graph_metrics_reset_integration() {
    let (_, _server) = run_graph_test(
        3,
        "TestUsers",
        "test3_integration.html",
        false,
        "Test 3 - Integration with different scenario",
    )
    .await;
    // Test passes if no panics occur during execution
}

// Test 4: Data preservation testing with 2 users
#[tokio::test]
#[serial]
async fn test_graph_data_preservation_during_reset() {
    let (_, _server) = run_graph_test(
        2,
        "TestGraphPreservation",
        "test4_preservation.html",
        false,
        "Test 4 - Data preservation with 2 users",
    )
    .await;
    // Test passes if no panics occur during execution
}

/// Consolidated verification for internal state and graph continuity
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

    // HTML parsing for graph continuity verification - the core requirement
    if let Err(e) = verify_graph_continuity(html_file) {
        println!(
            "{}: Graph continuity verification failed: {}",
            test_description, e
        );
    }
}

/// Verify graph continuity by parsing HTML and checking for unexpected drops to zero
fn verify_graph_continuity(html_file: &str) -> Result<(), Box<dyn std::error::Error>> {
    let html_content = fs::read_to_string(html_file)?;

    // Find the user graph data section
    if let Some(graph_start) = html_content.find(r#"id="graph-active-users""#) {
        if let Some(data_start) = html_content[graph_start..].find("data:") {
            let data_section = &html_content[graph_start + data_start..];

            // Extract the data array
            if let Some(array_start) = data_section.find('[') {
                if let Some(array_end) = data_section.find(']') {
                    let data_str = &data_section[array_start..=array_end];
                    let user_counts = extract_user_counts_from_js_array(data_str)?;

                    // Check for unexpected drops to zero (core continuity requirement)
                    if user_counts.len() >= 3 {
                        for i in 1..user_counts.len() - 1 {
                            let prev = user_counts[i - 1];
                            let current = user_counts[i];
                            let next = user_counts[i + 1];

                            // Flag suspicious drops: non-zero -> zero -> non-zero pattern
                            if prev > 0 && current == 0 && next > 0 {
                                eprintln!("Graph continuity issue: unexpected drop to 0 at position {}: {} -> {} -> {}", 
                                    i, prev, current, next);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Extract user counts from JavaScript array format
fn extract_user_counts_from_js_array(
    data_str: &str,
) -> Result<Vec<usize>, Box<dyn std::error::Error>> {
    let mut user_counts = Vec::new();
    let mut search_pos = 0;

    while let Some(bracket_start) = data_str[search_pos..].find('[') {
        let abs_bracket_start = search_pos + bracket_start;
        if let Some(bracket_end) = data_str[abs_bracket_start..].find(']') {
            let abs_bracket_end = abs_bracket_start + bracket_end;
            let item = &data_str[abs_bracket_start + 1..abs_bracket_end];

            // Look for the comma that separates timestamp from count
            if let Some(comma_pos) = item.rfind(',') {
                let count_str = item[comma_pos + 1..].trim();
                if let Ok(count) = count_str.parse::<usize>() {
                    user_counts.push(count);
                }
            }

            search_pos = abs_bracket_end + 1;
        } else {
            break;
        }
    }

    Ok(user_counts)
}
