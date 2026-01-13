//! Tests for Coordinated Omission metrics functionality introduced in PR #630.

use goose::metrics::GooseCoordinatedOmissionMitigation;
use goose::metrics::coordinated_omission::*;
use std::thread::sleep;
use std::time::Duration;

mod coordinated_omission_metrics_tests {
    use super::*;

    #[test]
    fn test_co_metrics_initialization() {
        let metrics = CoordinatedOmissionMetrics::new(GooseCoordinatedOmissionMitigation::Average);
        assert_eq!(metrics.actual_requests, 0);
        assert_eq!(metrics.synthetic_requests, 0);
        assert_eq!(metrics.synthetic_percentage, 0.0);
        assert!(metrics.co_events.is_empty());
        assert!(metrics.severity_histogram.is_empty());
        assert_eq!(
            metrics.mitigation_strategy,
            GooseCoordinatedOmissionMitigation::Average
        );
        assert!(metrics.started_secs > 0);

        // Test with different strategies
        let metrics_min =
            CoordinatedOmissionMetrics::new(GooseCoordinatedOmissionMitigation::Minimum);
        assert_eq!(
            metrics_min.mitigation_strategy,
            GooseCoordinatedOmissionMitigation::Minimum
        );

        let metrics_max =
            CoordinatedOmissionMetrics::new(GooseCoordinatedOmissionMitigation::Maximum);
        assert_eq!(
            metrics_max.mitigation_strategy,
            GooseCoordinatedOmissionMitigation::Maximum
        );

        let metrics_disabled =
            CoordinatedOmissionMetrics::new(GooseCoordinatedOmissionMitigation::Disabled);
        assert_eq!(
            metrics_disabled.mitigation_strategy,
            GooseCoordinatedOmissionMitigation::Disabled
        );
    }

    #[test]
    fn test_record_actual_request() {
        let mut metrics =
            CoordinatedOmissionMetrics::new(GooseCoordinatedOmissionMitigation::Average);

        // Initial state
        assert_eq!(metrics.actual_requests, 0);
        assert_eq!(metrics.synthetic_requests, 0);
        assert_eq!(metrics.synthetic_percentage, 0.0);

        // Record one actual request
        metrics.record_actual_request();
        assert_eq!(metrics.actual_requests, 1);
        assert_eq!(metrics.synthetic_requests, 0);
        assert_eq!(metrics.synthetic_percentage, 0.0);

        // Record more actual requests
        metrics.record_actual_request();
        metrics.record_actual_request();
        assert_eq!(metrics.actual_requests, 3);
        assert_eq!(metrics.synthetic_requests, 0);
        assert_eq!(metrics.synthetic_percentage, 0.0);

        // Verify no side effects
        assert!(metrics.co_events.is_empty());
        assert!(metrics.severity_histogram.is_empty());
    }

    #[test]
    fn test_record_synthetic_requests() {
        let mut metrics =
            CoordinatedOmissionMetrics::new(GooseCoordinatedOmissionMitigation::Average);

        // Initial state
        assert_eq!(metrics.actual_requests, 0);
        assert_eq!(metrics.synthetic_requests, 0);
        assert_eq!(metrics.synthetic_percentage, 0.0);

        // Record synthetic requests
        metrics.record_synthetic_requests(5);
        assert_eq!(metrics.actual_requests, 0);
        assert_eq!(metrics.synthetic_requests, 5);
        assert_eq!(metrics.synthetic_percentage, 100.0); // 5/5 = 100%

        // Record more synthetic requests
        metrics.record_synthetic_requests(3);
        assert_eq!(metrics.actual_requests, 0);
        assert_eq!(metrics.synthetic_requests, 8);
        assert_eq!(metrics.synthetic_percentage, 100.0); // 8/8 = 100%

        // Verify no side effects
        assert!(metrics.co_events.is_empty());
        assert!(metrics.severity_histogram.is_empty());
    }

    #[test]
    fn test_synthetic_percentage_calculation() {
        let mut metrics =
            CoordinatedOmissionMetrics::new(GooseCoordinatedOmissionMitigation::Average);

        // Test case: 0 actual, 0 synthetic = 0%
        assert_eq!(metrics.synthetic_percentage, 0.0);

        // Test case: 100 actual, 0 synthetic = 0%
        for _ in 0..100 {
            metrics.record_actual_request();
        }
        assert_eq!(metrics.synthetic_percentage, 0.0);

        // Test case: 100 actual, 10 synthetic = 9.09% (10/(100+10))
        metrics.record_synthetic_requests(10);
        assert!((metrics.synthetic_percentage - 9.090909).abs() < 0.001); // ~9.09%

        // Test case: add more to get 50/50 split
        metrics.record_synthetic_requests(40); // Total: 50 synthetic
        // 50/(100+50) = 33.33%
        assert!((metrics.synthetic_percentage - 33.333333).abs() < 0.001);

        // Add more synthetic to get exactly 50%: 100/(100+100) = 50%
        metrics.record_synthetic_requests(50); // Total: 100 synthetic
        assert_eq!(metrics.synthetic_percentage, 50.0); // 100/(100+100) = 50%
    }

    #[test]
    fn test_record_co_event() {
        let mut metrics =
            CoordinatedOmissionMetrics::new(GooseCoordinatedOmissionMitigation::Average);

        // Test minor severity (3x expected duration)
        let expected = Duration::from_millis(100);
        let actual = Duration::from_millis(300); // 3x
        metrics.record_co_event(expected, actual, 2, 1, "TestScenario".to_string());

        assert_eq!(metrics.co_events.len(), 1);
        assert_eq!(metrics.synthetic_requests, 2);
        assert_eq!(metrics.severity_histogram.get(&CoSeverity::Minor), Some(&1));

        let event = &metrics.co_events[0];
        assert_eq!(event.expected_cadence, expected);
        assert_eq!(event.actual_duration, actual);
        assert_eq!(event.synthetic_injected, 2);
        assert_eq!(event.user_id, 1);
        assert_eq!(event.scenario_name, "TestScenario");
        assert_eq!(event.severity, CoSeverity::Minor);
        assert!(event.timestamp_secs > 0);

        // Test moderate severity (7x expected duration)
        let actual_moderate = Duration::from_millis(700); // 7x
        metrics.record_co_event(expected, actual_moderate, 5, 2, "TestScenario2".to_string());

        assert_eq!(metrics.co_events.len(), 2);
        assert_eq!(metrics.synthetic_requests, 7); // 2 + 5
        assert_eq!(metrics.severity_histogram.get(&CoSeverity::Minor), Some(&1));
        assert_eq!(
            metrics.severity_histogram.get(&CoSeverity::Moderate),
            Some(&1)
        );

        // Test severe severity (15x expected duration)
        let actual_severe = Duration::from_millis(1500); // 15x
        metrics.record_co_event(expected, actual_severe, 10, 3, "TestScenario3".to_string());

        assert_eq!(metrics.co_events.len(), 3);
        assert_eq!(metrics.synthetic_requests, 17); // 2 + 5 + 10
        assert_eq!(
            metrics.severity_histogram.get(&CoSeverity::Severe),
            Some(&1)
        );

        // Test critical severity (25x expected duration)
        let actual_critical = Duration::from_millis(2500); // 25x
        metrics.record_co_event(
            expected,
            actual_critical,
            20,
            4,
            "TestScenario4".to_string(),
        );

        assert_eq!(metrics.co_events.len(), 4);
        assert_eq!(metrics.synthetic_requests, 37); // 2 + 5 + 10 + 20
        assert_eq!(
            metrics.severity_histogram.get(&CoSeverity::Critical),
            Some(&1)
        );

        let critical_event = &metrics.co_events[3];
        assert_eq!(critical_event.severity, CoSeverity::Critical);
    }
}

mod severity_classification_tests {
    use super::*;

    #[test]
    fn test_calculate_severity_boundaries() {
        // Since calculate_severity is private, we test it indirectly through record_co_event
        let mut metrics =
            CoordinatedOmissionMetrics::new(GooseCoordinatedOmissionMitigation::Average);
        let expected = Duration::from_millis(100);

        // Test Minor boundaries (2-5x) - should be classified as Minor
        metrics.record_co_event(
            expected,
            Duration::from_millis(300),
            2,
            1,
            "Test".to_string(),
        ); // 3x
        assert_eq!(metrics.co_events[0].severity, CoSeverity::Minor);

        // Test Moderate boundaries (5-10x)
        metrics.record_co_event(
            expected,
            Duration::from_millis(700),
            5,
            2,
            "Test".to_string(),
        ); // 7x
        assert_eq!(metrics.co_events[1].severity, CoSeverity::Moderate);

        // Test Severe boundaries (10-20x)
        metrics.record_co_event(
            expected,
            Duration::from_millis(1500),
            10,
            3,
            "Test".to_string(),
        ); // 15x
        assert_eq!(metrics.co_events[2].severity, CoSeverity::Severe);

        // Test Critical boundaries (>20x)
        metrics.record_co_event(
            expected,
            Duration::from_millis(2500),
            20,
            4,
            "Test".to_string(),
        ); // 25x
        assert_eq!(metrics.co_events[3].severity, CoSeverity::Critical);

        // Verify severity histogram was updated correctly
        assert_eq!(metrics.severity_histogram.get(&CoSeverity::Minor), Some(&1));
        assert_eq!(
            metrics.severity_histogram.get(&CoSeverity::Moderate),
            Some(&1)
        );
        assert_eq!(
            metrics.severity_histogram.get(&CoSeverity::Severe),
            Some(&1)
        );
        assert_eq!(
            metrics.severity_histogram.get(&CoSeverity::Critical),
            Some(&1)
        );
    }

    #[test]
    fn test_severity_edge_cases() {
        // Test edge cases through CO event recording since calculate_severity is private
        let mut metrics =
            CoordinatedOmissionMetrics::new(GooseCoordinatedOmissionMitigation::Average);

        // Test with very small durations
        let small_expected = Duration::from_millis(1);
        metrics.record_co_event(
            small_expected,
            Duration::from_millis(3),
            1,
            1,
            "Test".to_string(),
        ); // 3x
        assert_eq!(metrics.co_events[0].severity, CoSeverity::Minor);

        // Test with zero expected duration (should not panic)
        let zero_expected = Duration::from_millis(0);
        metrics.record_co_event(
            zero_expected,
            Duration::from_millis(100),
            1,
            2,
            "Test".to_string(),
        );
        // Should be critical due to infinite ratio
        assert_eq!(metrics.co_events[1].severity, CoSeverity::Critical);

        // Test with zero actual duration (should not panic)
        let expected = Duration::from_millis(100);
        metrics.record_co_event(expected, Duration::from_millis(0), 1, 3, "Test".to_string());
        // Should be minor due to ratio near 0
        assert_eq!(metrics.co_events[2].severity, CoSeverity::Minor);

        // Test with very large durations
        let large_expected = Duration::from_secs(10);
        let large_actual = Duration::from_secs(300); // 30x
        metrics.record_co_event(large_expected, large_actual, 1, 4, "Test".to_string());
        assert_eq!(metrics.co_events[3].severity, CoSeverity::Critical);
    }
}

mod cadence_calculator_tests {
    use super::*;

    #[test]
    fn test_minimum_cadence_calculator() {
        let mut calc = MinimumCadence::new(3); // 3 warmup iterations

        assert_eq!(calc.name(), "minimum");
        assert!(calc.describe_approach().contains("minimum"));

        // During warmup - should return high duration to avoid false positives
        let measurements = vec![Duration::from_millis(100), Duration::from_millis(200)];
        let baseline = calc.calculate_baseline(&measurements);
        assert_eq!(baseline, Duration::from_secs(3600)); // High value during warmup

        // After warmup - should return minimum from measurements
        let measurements = vec![
            Duration::from_millis(100),
            Duration::from_millis(50),
            Duration::from_millis(200),
        ];
        let baseline = calc.calculate_baseline(&measurements);
        assert_eq!(baseline, Duration::from_millis(50)); // Minimum value

        // Test injection threshold (should trigger when elapsed > baseline * 2)
        assert!(
            !calc.should_inject_synthetic(Duration::from_millis(100), Duration::from_millis(50))
        ); // 100 == 50*2, not >
        assert!(
            !calc.should_inject_synthetic(Duration::from_millis(99), Duration::from_millis(50))
        ); // <2x
        assert!(
            calc.should_inject_synthetic(Duration::from_millis(101), Duration::from_millis(50))
        );
        // >2x
    }

    #[test]
    fn test_average_cadence_calculator() {
        let mut calc = AverageCadence::new(2, 2.5); // 2 warmup iterations, 2.5x deviation threshold

        assert_eq!(calc.name(), "average");
        assert!(calc.describe_approach().contains("average"));

        // During warmup
        let measurements = vec![Duration::from_millis(100)];
        let baseline = calc.calculate_baseline(&measurements);
        assert_eq!(baseline, Duration::from_secs(3600)); // High value during warmup

        // After warmup - should calculate average
        let measurements = vec![
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(300),
        ];
        let baseline = calc.calculate_baseline(&measurements);
        assert_eq!(baseline, Duration::from_millis(200)); // (100+200+300)/3 = 200

        // Test injection threshold (this test uses 2.5x threshold as configured above)
        let baseline = Duration::from_millis(100);
        // This calc was created with threshold 2.5, so ratio > 2.5 should trigger
        assert!(!calc.should_inject_synthetic(Duration::from_millis(250), baseline)); // 2.5x, not > 2.5
        assert!(calc.should_inject_synthetic(Duration::from_millis(251), baseline)); // >2.5x
        assert!(!calc.should_inject_synthetic(Duration::from_millis(249), baseline)); // <2.5x

        // Test with empty measurements after warmup
        let baseline = calc.calculate_baseline(&[]);
        assert_eq!(baseline, Duration::from_secs(1)); // Default fallback
    }

    #[test]
    fn test_maximum_cadence_calculator() {
        let mut calc = MaximumCadence::new(2); // 2 warmup iterations

        assert_eq!(calc.name(), "maximum");
        assert!(calc.describe_approach().contains("maximum"));

        // During warmup
        let measurements = vec![Duration::from_millis(100)];
        let baseline = calc.calculate_baseline(&measurements);
        assert_eq!(baseline, Duration::from_secs(3600)); // High value during warmup

        // After warmup - should return maximum from measurements
        let measurements = vec![
            Duration::from_millis(100),
            Duration::from_millis(50),
            Duration::from_millis(200),
        ];
        let baseline = calc.calculate_baseline(&measurements);
        assert_eq!(baseline, Duration::from_millis(200)); // Maximum value

        // Test injection threshold (should use 2x threshold: elapsed > baseline * 2)
        assert!(
            !calc.should_inject_synthetic(Duration::from_millis(400), Duration::from_millis(200))
        ); // 400 == 200*2, not >
        assert!(
            !calc.should_inject_synthetic(Duration::from_millis(399), Duration::from_millis(200))
        ); // <2x
        assert!(
            calc.should_inject_synthetic(Duration::from_millis(401), Duration::from_millis(200))
        ); // >2x
    }

    #[test]
    fn test_percentile_cadence_calculator() {
        let mut calc = PercentileCadence::new(0.95, 2); // 95th percentile, 2 warmup iterations

        assert_eq!(calc.name(), "percentile");
        assert!(calc.describe_approach().contains("percentile"));

        // During warmup
        let measurements = vec![Duration::from_millis(100)];
        let baseline = calc.calculate_baseline(&measurements);
        assert_eq!(baseline, Duration::from_secs(3600)); // High value during warmup

        // After warmup - should calculate percentile
        let measurements: Vec<Duration> =
            (1..=100).map(|i| Duration::from_millis(i as u64)).collect();
        let baseline = calc.calculate_baseline(&measurements);
        // 95th percentile of 1-100 should be around 95
        assert!(baseline >= Duration::from_millis(94) && baseline <= Duration::from_millis(96));

        // Test injection threshold (uses 2x threshold: elapsed > baseline * 2)
        assert!(
            !calc.should_inject_synthetic(Duration::from_millis(200), Duration::from_millis(100))
        ); // 200 == 100*2, not >
        assert!(
            !calc.should_inject_synthetic(Duration::from_millis(199), Duration::from_millis(100))
        ); // <2x
        assert!(
            calc.should_inject_synthetic(Duration::from_millis(201), Duration::from_millis(100))
        ); // >2x

        // Test with empty measurements after warmup
        let baseline = calc.calculate_baseline(&[]);
        assert_eq!(baseline, Duration::from_secs(1)); // Default fallback
    }

    #[test]
    fn test_disabled_cadence_calculator() {
        // Test disabled cadence through the factory function since DisabledCadence is private
        let mut calc = create_cadence_calculator(&GooseCoordinatedOmissionMitigation::Disabled, 3);

        assert_eq!(calc.name(), "disabled");
        assert!(calc.describe_approach().contains("disabled"));

        // Should always return max duration
        let baseline = calc.calculate_baseline(&[Duration::from_millis(100)]);
        assert_eq!(baseline, Duration::from_secs(u64::MAX));

        // Should never trigger synthetic injection
        assert!(!calc.should_inject_synthetic(Duration::from_secs(1000), Duration::from_millis(1)));
        assert!(
            !calc.should_inject_synthetic(Duration::from_secs(u64::MAX), Duration::from_millis(1))
        );
    }

    #[test]
    fn test_cadence_calculator_factory() {
        let warmup_iterations = 5;

        // Test Average strategy
        let calc = create_cadence_calculator(
            &GooseCoordinatedOmissionMitigation::Average,
            warmup_iterations,
        );
        assert_eq!(calc.name(), "average");

        // Test Minimum strategy
        let calc = create_cadence_calculator(
            &GooseCoordinatedOmissionMitigation::Minimum,
            warmup_iterations,
        );
        assert_eq!(calc.name(), "minimum");

        // Test Maximum strategy
        let calc = create_cadence_calculator(
            &GooseCoordinatedOmissionMitigation::Maximum,
            warmup_iterations,
        );
        assert_eq!(calc.name(), "maximum");

        // Test Disabled strategy
        let calc = create_cadence_calculator(
            &GooseCoordinatedOmissionMitigation::Disabled,
            warmup_iterations,
        );
        assert_eq!(calc.name(), "disabled");
    }
}

mod co_summary_tests {
    use super::*;

    #[test]
    fn test_get_summary_basic() {
        let mut metrics =
            CoordinatedOmissionMetrics::new(GooseCoordinatedOmissionMitigation::Average);

        // Add some test data
        metrics.record_actual_request();
        metrics.record_actual_request();
        metrics.record_synthetic_requests(3);

        // Sleep to ensure duration is measured (use full second for clock resolution)
        sleep(Duration::from_secs(1));

        let summary = metrics.get_summary();

        assert_eq!(summary.total_co_events, 0); // No events recorded yet
        assert_eq!(summary.actual_requests, 2);
        assert_eq!(summary.synthetic_requests, 3);
        assert!((summary.synthetic_percentage - 60.0).abs() < 0.001); // 3/(2+3) = 60%
        assert!(summary.duration_secs > 0); // Should be > 0 after sleep
        assert_eq!(summary.events_per_minute, 0.0); // No events yet

        // Individual severity counts should all be 0
        assert_eq!(summary.minor_count, 0);
        assert_eq!(summary.moderate_count, 0);
        assert_eq!(summary.severe_count, 0);
        assert_eq!(summary.critical_count, 0);

        assert!(summary.per_user_events.is_empty());
        assert!(summary.per_scenario_events.is_empty());
    }

    #[test]
    fn test_get_summary_per_user_breakdown() {
        let mut metrics =
            CoordinatedOmissionMetrics::new(GooseCoordinatedOmissionMitigation::Average);

        // Create events for multiple users with different severities
        let expected = Duration::from_millis(100);

        // User 1: 2 minor events
        metrics.record_co_event(
            expected,
            Duration::from_millis(300),
            2,
            1,
            "Scenario1".to_string(),
        );
        metrics.record_co_event(
            expected,
            Duration::from_millis(400),
            3,
            1,
            "Scenario1".to_string(),
        );

        // User 2: 1 moderate, 1 severe event
        metrics.record_co_event(
            expected,
            Duration::from_millis(600),
            5,
            2,
            "Scenario2".to_string(),
        );
        metrics.record_co_event(
            expected,
            Duration::from_millis(1200),
            10,
            2,
            "Scenario2".to_string(),
        );

        // User 3: 1 critical event
        metrics.record_co_event(
            expected,
            Duration::from_millis(3000),
            25,
            3,
            "Scenario3".to_string(),
        );

        let summary = metrics.get_summary();

        assert_eq!(summary.total_co_events, 5);
        assert_eq!(summary.per_user_events.len(), 3);

        // Find user 1's breakdown
        let user1_data = summary
            .per_user_events
            .iter()
            .find(|(user_id, _, _)| *user_id == 1)
            .expect("User 1 should be present");
        assert_eq!(user1_data.1, 2); // 2 events
        assert!(user1_data.2.contains("Minor: 2")); // 2 minor events

        // Find user 2's breakdown
        let user2_data = summary
            .per_user_events
            .iter()
            .find(|(user_id, _, _)| *user_id == 2)
            .expect("User 2 should be present");
        assert_eq!(user2_data.1, 2); // 2 events
        assert!(user2_data.2.contains("Moderate: 1"));
        assert!(user2_data.2.contains("Severe: 1"));

        // Find user 3's breakdown
        let user3_data = summary
            .per_user_events
            .iter()
            .find(|(user_id, _, _)| *user_id == 3)
            .expect("User 3 should be present");
        assert_eq!(user3_data.1, 1); // 1 event
        assert!(user3_data.2.contains("Critical: 1"));
    }

    #[test]
    fn test_get_summary_per_scenario_breakdown() {
        let mut metrics =
            CoordinatedOmissionMetrics::new(GooseCoordinatedOmissionMitigation::Average);

        // Create events for multiple scenarios
        let expected = Duration::from_millis(100);

        // Scenario1: 2 events, 5 synthetic requests total
        metrics.record_co_event(
            expected,
            Duration::from_millis(300),
            2,
            1,
            "Scenario1".to_string(),
        );
        metrics.record_co_event(
            expected,
            Duration::from_millis(400),
            3,
            2,
            "Scenario1".to_string(),
        );

        // Scenario2: 1 event, 10 synthetic requests
        metrics.record_co_event(
            expected,
            Duration::from_millis(1200),
            10,
            3,
            "Scenario2".to_string(),
        );

        let summary = metrics.get_summary();

        assert_eq!(summary.per_scenario_events.len(), 2);

        // Find Scenario1's breakdown
        let scenario1_data = summary
            .per_scenario_events
            .iter()
            .find(|(scenario, _, _)| scenario == "Scenario1")
            .expect("Scenario1 should be present");
        assert_eq!(scenario1_data.1, 2); // 2 events
        assert_eq!(scenario1_data.2, 5); // 2 + 3 = 5 synthetic requests

        // Find Scenario2's breakdown
        let scenario2_data = summary
            .per_scenario_events
            .iter()
            .find(|(scenario, _, _)| scenario == "Scenario2")
            .expect("Scenario2 should be present");
        assert_eq!(scenario2_data.1, 1); // 1 event
        assert_eq!(scenario2_data.2, 10); // 10 synthetic requests
    }

    #[test]
    fn test_synthetic_threshold_detection() {
        let mut metrics =
            CoordinatedOmissionMetrics::new(GooseCoordinatedOmissionMitigation::Average);

        // Test below threshold: 0.5% with 1% threshold → false
        metrics.record_actual_request(); // 1 actual
        metrics.record_actual_request(); // 2 actual
        metrics.record_synthetic_requests(1); // 1 synthetic = 1/(2+1) = 33.33%
        // Wait, let's make it actually 0.5%: need 199 actual, 1 synthetic = 0.5%
        for _ in 0..197 {
            metrics.record_actual_request(); // Total 199 actual
        }
        // Now: 1/(199+1) = 0.5%
        assert!(!metrics.is_synthetic_threshold_exceeded(1.0)); // 0.5% < 1%

        // Test at threshold: add more synthetic to reach exactly 1.0%
        metrics.record_synthetic_requests(1); // Total 2 synthetic = 2/(199+2) ≈ 1.0%
        assert!(!metrics.is_synthetic_threshold_exceeded(1.0)); // 1.0% == 1%, should be false

        // Test above threshold: add one more synthetic
        metrics.record_synthetic_requests(1); // Total 3 synthetic = 3/(199+3) ≈ 1.49%
        assert!(metrics.is_synthetic_threshold_exceeded(1.0)); // 1.49% > 1%
    }
}

mod display_formatting_tests {
    use super::*;

    #[test]
    fn test_co_metrics_display() {
        let mut metrics =
            CoordinatedOmissionMetrics::new(GooseCoordinatedOmissionMitigation::Average);

        // Add some test data
        metrics.record_actual_request();
        metrics.record_synthetic_requests(2);

        let expected = Duration::from_millis(100);
        metrics.record_co_event(
            expected,
            Duration::from_millis(300),
            2,
            1,
            "TestScenario".to_string(),
        );

        let display_output = format!("{metrics}");

        // Check that key information is present
        assert!(display_output.contains("COORDINATED OMISSION METRICS"));
        assert!(display_output.contains("Total CO Events: 1"));
        assert!(display_output.contains("Actual requests: 1"));
        assert!(display_output.contains("Synthetic requests: 4")); // 2 recorded + 2 from CO event = 4 total
        assert!(display_output.contains("Minor: 1"));
        assert!(display_output.contains("Events per minute:"));
    }

    #[test]
    fn test_co_summary_display() {
        let mut metrics =
            CoordinatedOmissionMetrics::new(GooseCoordinatedOmissionMitigation::Average);

        // Add various severity events
        let expected = Duration::from_millis(100);
        metrics.record_co_event(
            expected,
            Duration::from_millis(300),
            2,
            1,
            "Test1".to_string(),
        ); // Minor
        metrics.record_co_event(
            expected,
            Duration::from_millis(700),
            5,
            2,
            "Test2".to_string(),
        ); // Moderate
        metrics.record_co_event(
            expected,
            Duration::from_millis(1500),
            10,
            3,
            "Test3".to_string(),
        ); // Severe
        metrics.record_co_event(
            expected,
            Duration::from_millis(2500),
            20,
            4,
            "Test4".to_string(),
        ); // Critical

        let summary = metrics.get_summary();
        let display_output = format!("{summary}");

        // Verify all severity types are displayed
        assert!(display_output.contains("Minor: 1"));
        assert!(display_output.contains("Moderate: 1"));
        assert!(display_output.contains("Severe: 1"));
        assert!(display_output.contains("Critical: 1"));

        // Check totals
        assert!(display_output.contains("Total CO Events: 4"));
        assert!(display_output.contains("Synthetic requests: 37")); // 2+5+10+20

        // Check formatting handles large numbers correctly
        assert!(display_output.contains("Events per minute:"));
    }

    #[test]
    fn test_display_handles_zero_values() {
        let metrics = CoordinatedOmissionMetrics::new(GooseCoordinatedOmissionMitigation::Average);
        let display_output = format!("{metrics}");

        // Should handle zero values gracefully
        assert!(display_output.contains("Total CO Events: 0"));
        assert!(display_output.contains("Actual requests: 0"));
        assert!(display_output.contains("Synthetic requests: 0 (0.0%)"));
        assert!(display_output.contains("Events per minute: 0.00"));
    }
}
