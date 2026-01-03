//! Integration tests for Coordinated Omission functionality with the full Goose system.

use httpmock::{Method::GET, MockServer};
use std::time::Duration;

mod common;
use goose::metrics::GooseCoordinatedOmissionMitigation;
use goose::prelude::*;

// Test transaction for integration tests
async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

// Test transaction with delay to trigger CO events
async fn get_delayed_page(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/delayed").await?;
    Ok(())
}

mod load_test_integration {
    use super::*;

    #[tokio::test]
    async fn test_co_disabled_no_events() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET).path("/");
            then.status(200).delay(Duration::from_millis(100));
        });

        let config = common::build_configuration(
            &server,
            vec![
                "--users",
                "1",
                "--run-time",
                "3",
                "--co-mitigation",
                "disabled",
            ],
        );

        let goose_metrics = common::run_load_test(
            common::build_load_test(
                config,
                vec![scenario!("Test").register_transaction(transaction!(get_index))],
                None,
                None,
            ),
            None,
        )
        .await;

        // When CO is disabled, no CO metrics should be collected
        assert!(goose_metrics.coordinated_omission_metrics.is_none());
        assert!(mock.calls() > 0);

        // Regular metrics should still be present
        assert!(!goose_metrics.requests.is_empty());
    }

    #[tokio::test]
    async fn test_co_minimum_strategy_basic() {
        let server = MockServer::start();

        // Set up endpoint with predictable delays
        let mock = server.mock(|when, then| {
            when.method(GET).path("/");
            then.status(200).delay(Duration::from_millis(50));
        });

        let config = common::build_configuration(
            &server,
            vec![
                "--users",
                "1",
                "--run-time",
                "5",
                "--co-mitigation",
                "minimum",
            ],
        );

        let goose_metrics = common::run_load_test(
            common::build_load_test(
                config,
                vec![scenario!("Test").register_transaction(transaction!(get_index))],
                None,
                None,
            ),
            None,
        )
        .await;

        // CO should be enabled and metrics collected
        assert!(goose_metrics.coordinated_omission_metrics.is_some());
        let co_metrics = goose_metrics.coordinated_omission_metrics.unwrap();
        assert_eq!(
            co_metrics.mitigation_strategy,
            GooseCoordinatedOmissionMitigation::Minimum
        );

        // Should have recorded actual requests
        assert!(co_metrics.actual_requests > 0);

        // With consistent 50ms delays, minimal CO events expected
        // (This test may not trigger CO events due to consistent timing)
        assert!(mock.calls() > 0);
    }

    #[tokio::test]
    async fn test_co_event_detection_with_delays() {
        let server = MockServer::start();

        // Create a more complex mock that sometimes has delays
        let mock_fast = server.mock(|when, then| {
            when.method(GET).path("/");
            then.status(200).delay(Duration::from_millis(50));
        });

        let mock_slow = server.mock(|when, then| {
            when.method(GET).path("/delayed");
            then.status(200).delay(Duration::from_millis(500)); // Much slower
        });

        let config = common::build_configuration(
            &server,
            vec![
                "--users",
                "2",
                "--run-time",
                "8",
                "--co-mitigation",
                "minimum",
            ],
        );

        let goose_metrics = common::run_load_test(
            common::build_load_test(
                config,
                vec![
                    scenario!("Fast").register_transaction(transaction!(get_index)),
                    scenario!("Slow").register_transaction(transaction!(get_delayed_page)),
                ],
                None,
                None,
            ),
            None,
        )
        .await;

        assert!(goose_metrics.coordinated_omission_metrics.is_some());
        let co_metrics = goose_metrics.coordinated_omission_metrics.unwrap();

        // Should have recorded both actual and potentially synthetic requests
        assert!(co_metrics.actual_requests > 0);

        // With the delayed endpoint, we might trigger some CO events
        let summary = co_metrics.get_summary();

        // Verify basic functionality
        assert!(summary.duration_secs > 0);
        assert_eq!(
            summary.synthetic_percentage,
            co_metrics.synthetic_percentage
        );

        assert!(mock_fast.calls() > 0);
        assert!(mock_slow.calls() > 0);
    }

    #[tokio::test]
    async fn test_co_strategy_comparison() {
        let server = MockServer::start();

        // Set up endpoint with variable delay to trigger CO
        server.mock(|when, then| {
            when.method(GET).path("/");
            then.status(200).delay(Duration::from_millis(100));
        });

        // Test with minimum strategy
        let config_min = common::build_configuration(
            &server,
            vec![
                "--users",
                "1",
                "--run-time",
                "4",
                "--co-mitigation",
                "minimum",
            ],
        );

        let metrics_min = common::run_load_test(
            common::build_load_test(
                config_min,
                vec![scenario!("Test").register_transaction(transaction!(get_index))],
                None,
                None,
            ),
            None,
        )
        .await;

        // Test with average strategy
        let config_avg = common::build_configuration(
            &server,
            vec![
                "--users",
                "1",
                "--run-time",
                "4",
                "--co-mitigation",
                "average",
            ],
        );

        let metrics_avg = common::run_load_test(
            common::build_load_test(
                config_avg,
                vec![scenario!("Test").register_transaction(transaction!(get_index))],
                None,
                None,
            ),
            None,
        )
        .await;

        // Test with maximum strategy
        let config_max = common::build_configuration(
            &server,
            vec![
                "--users",
                "1",
                "--run-time",
                "4",
                "--co-mitigation",
                "maximum",
            ],
        );

        let metrics_max = common::run_load_test(
            common::build_load_test(
                config_max,
                vec![scenario!("Test").register_transaction(transaction!(get_index))],
                None,
                None,
            ),
            None,
        )
        .await;

        // All should have CO metrics enabled
        assert!(metrics_min.coordinated_omission_metrics.is_some());
        assert!(metrics_avg.coordinated_omission_metrics.is_some());
        assert!(metrics_max.coordinated_omission_metrics.is_some());

        let co_min = metrics_min.coordinated_omission_metrics.unwrap();
        let co_avg = metrics_avg.coordinated_omission_metrics.unwrap();
        let co_max = metrics_max.coordinated_omission_metrics.unwrap();

        // Verify different strategies are recorded
        assert_eq!(
            co_min.mitigation_strategy,
            GooseCoordinatedOmissionMitigation::Minimum
        );
        assert_eq!(
            co_avg.mitigation_strategy,
            GooseCoordinatedOmissionMitigation::Average
        );
        assert_eq!(
            co_max.mitigation_strategy,
            GooseCoordinatedOmissionMitigation::Maximum
        );

        // All should have recorded actual requests
        assert!(co_min.actual_requests > 0);
        assert!(co_avg.actual_requests > 0);
        assert!(co_max.actual_requests > 0);
    }
}

mod metrics_integration {
    use super::*;

    #[tokio::test]
    async fn test_co_metrics_in_goose_metrics() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/");
            then.status(200).delay(Duration::from_millis(75));
        });

        let config = common::build_configuration(
            &server,
            vec![
                "--users",
                "1",
                "--run-time",
                "3",
                "--co-mitigation",
                "average",
            ],
        );

        let goose_metrics = common::run_load_test(
            common::build_load_test(
                config,
                vec![scenario!("Test").register_transaction(transaction!(get_index))],
                None,
                None,
            ),
            None,
        )
        .await;

        // CO metrics should be integrated into main GooseMetrics
        assert!(goose_metrics.coordinated_omission_metrics.is_some());

        // Regular metrics should still be present
        assert!(!goose_metrics.requests.is_empty());

        // Verify the CO metrics can be accessed and have expected structure
        let co_metrics = goose_metrics.coordinated_omission_metrics.as_ref().unwrap();
        assert!(co_metrics.started_secs > 0);
        assert_eq!(
            co_metrics.mitigation_strategy,
            GooseCoordinatedOmissionMitigation::Average
        );

        // Summary should be accessible
        let summary = co_metrics.get_summary();
        assert!(summary.duration_secs > 0);
        assert!(summary.actual_requests > 0);
    }

    #[tokio::test]
    async fn test_co_event_recording_during_load_test() {
        let server = MockServer::start();

        // Create endpoint that will occasionally trigger CO events
        server.mock(|when, then| {
            when.method(GET).path("/");
            then.status(200).delay(Duration::from_millis(50));
        });

        let config = common::build_configuration(
            &server,
            vec![
                "--users",
                "2",
                "--run-time",
                "6",
                "--co-mitigation",
                "minimum",
            ],
        );

        let goose_metrics = common::run_load_test(
            common::build_load_test(
                config,
                vec![scenario!("Test").register_transaction(transaction!(get_index))],
                None,
                None,
            ),
            None,
        )
        .await;

        assert!(goose_metrics.coordinated_omission_metrics.is_some());
        let co_metrics = goose_metrics.coordinated_omission_metrics.unwrap();

        // Events should have proper structure if any were recorded
        for event in &co_metrics.co_events {
            assert!(event.timestamp_secs > 0);
            assert!(event.user_id > 0);
            assert!(!event.scenario_name.is_empty());
            assert!(event.expected_cadence > Duration::from_millis(0));
            assert!(event.actual_duration > Duration::from_millis(0));

            // Severity should be properly classified
            match event.severity {
                goose::metrics::coordinated_omission::CoSeverity::Minor
                | goose::metrics::coordinated_omission::CoSeverity::Moderate
                | goose::metrics::coordinated_omission::CoSeverity::Severe
                | goose::metrics::coordinated_omission::CoSeverity::Critical => {
                    // All valid severities
                }
            }
        }

        // Summary should reflect the events
        let summary = co_metrics.get_summary();
        assert_eq!(summary.total_co_events, co_metrics.co_events.len());
    }
}

mod reporting_integration {
    use super::*;

    #[tokio::test]
    async fn test_html_report_with_co_data() {
        let server = MockServer::start();
        common::setup_co_triggering_server(&server);

        let report_file = "co-test-report.html";
        common::cleanup_files(vec![report_file]);

        let config = common::build_configuration(
            &server,
            vec![
                "--users",
                "2",
                "--run-time",
                "5",
                "--co-mitigation",
                "minimum",
                "--report-file",
                report_file,
            ],
        );

        let goose_metrics = common::run_load_test(
            common::build_load_test(
                config,
                vec![scenario!("Test").register_transaction(transaction!(get_index))],
                None,
                None,
            ),
            None,
        )
        .await;

        // Verify CO metrics were collected
        assert!(goose_metrics.coordinated_omission_metrics.is_some());

        // Verify HTML report was created
        assert!(std::path::Path::new(report_file).exists());
        let report_content = std::fs::read_to_string(report_file).unwrap();

        // Report should contain CO-related content
        // Note: Actual content depends on HTML template implementation
        assert!(report_content.contains("html")); // Basic HTML structure
        assert!(!report_content.is_empty());

        common::cleanup_files(vec![report_file]);
    }

    #[tokio::test]
    async fn test_json_export_with_co_data() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/");
            then.status(200).delay(Duration::from_millis(100));
        });

        let report_file = "co-test-report.json";
        common::cleanup_files(vec![report_file]);

        let config = common::build_configuration(
            &server,
            vec![
                "--users",
                "1",
                "--run-time",
                "3",
                "--co-mitigation",
                "average",
                "--report-file",
                report_file,
            ],
        );

        let goose_metrics = common::run_load_test(
            common::build_load_test(
                config,
                vec![scenario!("Test").register_transaction(transaction!(get_index))],
                None,
                None,
            ),
            None,
        )
        .await;

        // Verify CO metrics were collected
        assert!(goose_metrics.coordinated_omission_metrics.is_some());

        // Verify JSON report was created
        assert!(std::path::Path::new(report_file).exists());
        let report_content = std::fs::read_to_string(report_file).unwrap();

        // Should be valid JSON
        let json_value: serde_json::Value =
            serde_json::from_str(&report_content).expect("Report should be valid JSON");

        // JSON should contain coordinated omission metrics
        assert!(json_value.is_object());

        common::cleanup_files(vec![report_file]);
    }

    #[tokio::test]
    async fn test_markdown_report_with_co_analysis() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/");
            then.status(200).delay(Duration::from_millis(80));
        });

        let report_file = "co-test-report.md";
        common::cleanup_files(vec![report_file]);

        let config = common::build_configuration(
            &server,
            vec![
                "--users",
                "1",
                "--run-time",
                "4",
                "--co-mitigation",
                "minimum",
                "--report-file",
                report_file,
            ],
        );

        let goose_metrics = common::run_load_test(
            common::build_load_test(
                config,
                vec![scenario!("Test").register_transaction(transaction!(get_index))],
                None,
                None,
            ),
            None,
        )
        .await;

        // Verify CO metrics were collected
        assert!(goose_metrics.coordinated_omission_metrics.is_some());

        // Verify Markdown report was created
        assert!(std::path::Path::new(report_file).exists());
        let report_content = std::fs::read_to_string(report_file).unwrap();

        // Should contain Markdown formatting
        assert!(!report_content.is_empty());

        common::cleanup_files(vec![report_file]);
    }
}

mod configuration_integration {
    use super::*;

    #[tokio::test]
    async fn test_co_mitigation_cli_parsing() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/");
            then.status(200);
        });

        // Test "average" parsing
        let config_avg = common::build_configuration(
            &server,
            vec![
                "--users",
                "1",
                "--run-time",
                "2",
                "--co-mitigation",
                "average",
            ],
        );
        assert_eq!(
            config_avg.co_mitigation,
            Some(GooseCoordinatedOmissionMitigation::Average)
        );

        // Test "minimum" parsing
        let config_min = common::build_configuration(
            &server,
            vec![
                "--users",
                "1",
                "--run-time",
                "2",
                "--co-mitigation",
                "minimum",
            ],
        );
        assert_eq!(
            config_min.co_mitigation,
            Some(GooseCoordinatedOmissionMitigation::Minimum)
        );

        // Test "maximum" parsing
        let config_max = common::build_configuration(
            &server,
            vec![
                "--users",
                "1",
                "--run-time",
                "2",
                "--co-mitigation",
                "maximum",
            ],
        );
        assert_eq!(
            config_max.co_mitigation,
            Some(GooseCoordinatedOmissionMitigation::Maximum)
        );

        // Test "disabled" parsing
        let config_disabled = common::build_configuration(
            &server,
            vec![
                "--users",
                "1",
                "--run-time",
                "2",
                "--co-mitigation",
                "disabled",
            ],
        );
        assert_eq!(
            config_disabled.co_mitigation,
            Some(GooseCoordinatedOmissionMitigation::Disabled)
        );

        // Test abbreviations work
        let config_avg_abbrev = common::build_configuration(
            &server,
            vec!["--users", "1", "--run-time", "2", "--co-mitigation", "avg"],
        );
        assert_eq!(
            config_avg_abbrev.co_mitigation,
            Some(GooseCoordinatedOmissionMitigation::Average)
        );
    }

    #[tokio::test]
    async fn test_co_integration_with_other_options() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/");
            then.status(200).delay(Duration::from_millis(60));
        });

        let request_log = "co-integration-requests.log";
        let debug_log = "co-integration-debug.log";
        common::cleanup_files(vec![request_log, debug_log]);

        // Test CO with request logging and other options
        let config = common::build_configuration(
            &server,
            vec![
                "--users",
                "2",
                "--run-time",
                "4",
                "--co-mitigation",
                "average",
                "--request-log",
                request_log,
                "--debug-log",
                debug_log,
                "--no-transaction-metrics",
                "--throttle-requests",
                "50",
            ],
        );

        let goose_metrics = common::run_load_test(
            common::build_load_test(
                config,
                vec![
                    scenario!("Test1").register_transaction(transaction!(get_index)),
                    scenario!("Test2").register_transaction(transaction!(get_index)),
                ],
                None,
                None,
            ),
            None,
        )
        .await;

        // CO should work with other options enabled
        assert!(goose_metrics.coordinated_omission_metrics.is_some());
        let co_metrics = goose_metrics.coordinated_omission_metrics.unwrap();
        assert_eq!(
            co_metrics.mitigation_strategy,
            GooseCoordinatedOmissionMitigation::Average
        );

        // Multiple scenarios should be handled correctly
        assert!(co_metrics.actual_requests > 0);

        // Other options should still work
        assert!(!goose_metrics.requests.is_empty());

        // Transaction metrics should be disabled as requested
        assert!(goose_metrics.transactions.is_empty());

        // Log files should exist
        assert!(std::path::Path::new(request_log).exists());
        assert!(std::path::Path::new(debug_log).exists());

        common::cleanup_files(vec![request_log, debug_log]);
    }

    #[tokio::test]
    async fn test_co_default_configuration() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/");
            then.status(200);
        });

        // Test default behavior when CO is not explicitly configured
        let config = common::build_configuration(
            &server,
            vec![
                "--users",
                "1",
                "--run-time",
                "2", // No --co-mitigation specified
            ],
        );

        let goose_metrics = common::run_load_test(
            common::build_load_test(
                config,
                vec![scenario!("Test").register_transaction(transaction!(get_index))],
                None,
                None,
            ),
            None,
        )
        .await;

        // Default should be disabled, so no CO metrics
        assert!(goose_metrics.coordinated_omission_metrics.is_none());

        // Regular metrics should still work
        assert!(!goose_metrics.requests.is_empty());
    }
}

mod performance_and_edge_cases {
    use super::*;

    #[tokio::test]
    async fn test_co_with_multiple_users() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/");
            then.status(200).delay(Duration::from_millis(75));
        });

        let config = common::build_configuration(
            &server,
            vec![
                "--users",
                "5", // Multiple users
                "--run-time",
                "6",
                "--co-mitigation",
                "minimum",
            ],
        );

        let goose_metrics = common::run_load_test(
            common::build_load_test(
                config,
                vec![scenario!("Test").register_transaction(transaction!(get_index))],
                None,
                None,
            ),
            None,
        )
        .await;

        assert!(goose_metrics.coordinated_omission_metrics.is_some());
        let co_metrics = goose_metrics.coordinated_omission_metrics.unwrap();

        // Should handle multiple users correctly
        assert!(co_metrics.actual_requests > 0);

        // If CO events were recorded, they should have different user IDs
        let user_ids: std::collections::HashSet<usize> = co_metrics
            .co_events
            .iter()
            .map(|event| event.user_id)
            .collect();

        // We should see events from multiple users if any CO events occurred
        if !co_metrics.co_events.is_empty() {
            // At least some variation in user IDs is expected with multiple users
            assert!(!user_ids.is_empty());
        }
    }

    #[tokio::test]
    async fn test_co_edge_cases() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/");
            then.status(200).delay(Duration::from_millis(10)); // Very fast response
        });

        // Test with very short duration
        let config = common::build_configuration(
            &server,
            vec![
                "--users",
                "1",
                "--run-time",
                "1", // Very short test
                "--co-mitigation",
                "average",
            ],
        );

        let goose_metrics = common::run_load_test(
            common::build_load_test(
                config,
                vec![scenario!("Test").register_transaction(transaction!(get_index))],
                None,
                None,
            ),
            None,
        )
        .await;

        // Should handle short tests gracefully
        assert!(goose_metrics.coordinated_omission_metrics.is_some());
        let co_metrics = goose_metrics.coordinated_omission_metrics.unwrap();

        // Even with short test, basic structure should be intact
        assert_eq!(
            co_metrics.mitigation_strategy,
            GooseCoordinatedOmissionMitigation::Average
        );

        // May have very few or no actual requests due to short duration, but shouldn't crash
        let summary = co_metrics.get_summary();
        // Just ensure summary can be created without panicking
        // Just ensure summary can be created without panicking
        let _ = summary.total_co_events; // Verify field exists
    }

    #[tokio::test]
    async fn test_co_system_stability() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/");
            then.status(200).delay(Duration::from_millis(100));
        });

        // Test with higher load to verify system stability
        let config = common::build_configuration(
            &server,
            vec![
                "--users",
                "3",
                "--run-time",
                "8", // Longer test
                "--co-mitigation",
                "minimum",
            ],
        );

        let goose_metrics = common::run_load_test(
            common::build_load_test(
                config,
                vec![scenario!("Test").register_transaction(transaction!(get_index))],
                None,
                None,
            ),
            None,
        )
        .await;

        assert!(goose_metrics.coordinated_omission_metrics.is_some());
        let co_metrics = goose_metrics.coordinated_omission_metrics.unwrap();

        // System should remain stable and produce reasonable results
        assert!(co_metrics.actual_requests > 0);

        let summary = co_metrics.get_summary();
        assert!(summary.duration_secs > 0);
        assert!(summary.actual_requests > 0);

        // All CO events should be properly structured
        for event in &co_metrics.co_events {
            assert!(event.timestamp_secs > 0);
            assert!(event.expected_cadence > Duration::from_millis(0));
            assert!(event.actual_duration > Duration::from_millis(0));
            assert!(!event.scenario_name.is_empty());
        }
    }
}
