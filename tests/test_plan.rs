use std::str::FromStr;

mod common;

use goose::GooseError;
use goose::test_plan::TestPlan;

// Test valid test plan parsing with various formats
#[test]
fn test_valid_test_plan_parsing() {
    // Simple single step
    let plan = TestPlan::from_str("10,30s").unwrap();
    assert_eq!(plan.steps, vec![(10, 30_000)]);
    assert_eq!(plan.current, 0);

    // Multiple steps with semicolon separator
    let plan = TestPlan::from_str("5,30s;10,1m;0,10s").unwrap();
    assert_eq!(plan.steps, vec![(5, 30_000), (10, 60_000), (0, 10_000)]);

    // Complex time formats
    let plan = TestPlan::from_str("10,1h30m10s").unwrap();
    assert_eq!(plan.steps, vec![(10, 5_410_000)]); // 1*3600 + 30*60 + 10 = 5410 seconds

    // Integer seconds without suffix
    let plan = TestPlan::from_str("15,45").unwrap();
    assert_eq!(plan.steps, vec![(15, 45_000)]);

    // Hours only
    let plan = TestPlan::from_str("8,2h").unwrap();
    assert_eq!(plan.steps, vec![(8, 7_200_000)]); // 2*3600 = 7200 seconds

    // Minutes only
    let plan = TestPlan::from_str("12,45m").unwrap();
    assert_eq!(plan.steps, vec![(12, 2_700_000)]); // 45*60 = 2700 seconds

    // Hours and minutes
    let plan = TestPlan::from_str("20,1h20m").unwrap();
    assert_eq!(plan.steps, vec![(20, 4_800_000)]); // 1*3600 + 20*60 = 4800 seconds
}

// Test whitespace handling
#[test]
fn test_test_plan_whitespace_handling() {
    // Extra spaces around numbers and separators
    let plan = TestPlan::from_str("  10  ,  30s  ").unwrap();
    assert_eq!(plan.steps, vec![(10, 30_000)]);

    // Spaces in multi-step plans
    let plan = TestPlan::from_str(" 5 , 30s ; 10 , 1m ; 0 , 10s ").unwrap();
    assert_eq!(plan.steps, vec![(5, 30_000), (10, 60_000), (0, 10_000)]);

    // Mixed whitespace
    let plan = TestPlan::from_str("\t5,30s;\n10,1m").unwrap();
    assert_eq!(plan.steps, vec![(5, 30_000), (10, 60_000)]);
}

// Test zero values
#[test]
fn test_test_plan_zero_values() {
    // Zero users
    let plan = TestPlan::from_str("0,30s").unwrap();
    assert_eq!(plan.steps, vec![(0, 30_000)]);

    // Zero time
    let plan = TestPlan::from_str("10,0").unwrap();
    assert_eq!(plan.steps, vec![(10, 0)]);

    // Both zero
    let plan = TestPlan::from_str("0,0").unwrap();
    assert_eq!(plan.steps, vec![(0, 0)]);
}

// Test large numbers
#[test]
fn test_test_plan_large_numbers() {
    // Large user count
    let plan = TestPlan::from_str("10000,1h").unwrap();
    assert_eq!(plan.steps, vec![(10000, 3_600_000)]);

    // Large time values
    let plan = TestPlan::from_str("100,24h").unwrap();
    assert_eq!(plan.steps, vec![(100, 86_400_000)]); // 24*3600 = 86400 seconds
}

// Test complex multi-step scenarios
#[test]
fn test_complex_multi_step_scenarios() {
    // Ramp up, maintain, ramp down pattern
    let plan = TestPlan::from_str("0,0;50,2m;50,5m;0,1m").unwrap();
    assert_eq!(
        plan.steps,
        vec![
            (0, 0),
            (50, 120_000), // 2 minutes
            (50, 300_000), // 5 minutes
            (0, 60_000)    // 1 minute
        ]
    );

    // Complex scaling pattern
    let plan = TestPlan::from_str("10,30s;25,1m;50,2m;25,30s;0,10s").unwrap();
    assert_eq!(
        plan.steps,
        vec![
            (10, 30_000),
            (25, 60_000),
            (50, 120_000),
            (25, 30_000),
            (0, 10_000)
        ]
    );
}

// Test invalid test plan formats
#[test]
fn test_invalid_test_plan_formats() {
    // Missing comma
    let result = TestPlan::from_str("10 30s");
    assert!(result.is_err());
    if let Err(GooseError::InvalidOption { option, value, .. }) = result {
        assert_eq!(option, "`configuration.test_plan");
        assert_eq!(value, "10 30s");
    } else {
        panic!("Expected InvalidOption error");
    }

    // Missing time unit in complex format
    let result = TestPlan::from_str("10,1h30");
    assert!(result.is_err());

    // Invalid characters
    let result = TestPlan::from_str("10,30x");
    assert!(result.is_err());

    // Negative numbers - regex should not match
    let result = TestPlan::from_str("-10,30s");
    assert!(result.is_err());

    // Extra comma
    let result = TestPlan::from_str("10,,30s");
    assert!(result.is_err());

    // Empty string
    let result = TestPlan::from_str("");
    assert!(result.is_err());

    // Just a semicolon
    let result = TestPlan::from_str(";");
    assert!(result.is_err());

    // Invalid time format with wrong order
    let result = TestPlan::from_str("10,30m1h");
    assert!(result.is_err());

    // Missing users
    let result = TestPlan::from_str(",30s");
    assert!(result.is_err());

    // Missing time - this actually succeeds with 0 time
    let result = TestPlan::from_str("10,");
    // This is actually valid - it parses as 10 users for 0 seconds
    assert!(result.is_ok());
    if let Ok(plan) = result {
        assert_eq!(plan.steps, vec![(10, 0)]);
    }
}

// Test total_users calculation
#[test]
fn test_total_users_calculation() {
    // Single step
    let plan = TestPlan::from_str("10,30s").unwrap();
    assert_eq!(plan.total_users(), 10);

    // Multiple steps with increases only
    let plan = TestPlan::from_str("5,30s;10,1m;15,30s").unwrap();
    assert_eq!(plan.total_users(), 15); // 5 + (10-5) + (15-10) = 15

    // Steps with increases and decreases
    let plan = TestPlan::from_str("0,0;10,30s;5,30s;15,30s;0,10s").unwrap();
    assert_eq!(plan.total_users(), 20); // 0 + 10 + 0 + 10 + 0 = 20 (users added when increasing)

    // Starting from zero
    let plan = TestPlan::from_str("0,10s;20,1m;0,10s").unwrap();
    assert_eq!(plan.total_users(), 20); // 0 + 20 + 0 = 20

    // Complex scenario
    let plan = TestPlan::from_str("10,30s;25,1m;50,30s;25,30s;0,10s").unwrap();
    assert_eq!(plan.total_users(), 50); // 10 + 15 + 25 = 50

    // No increases (maintain same level)
    let plan = TestPlan::from_str("10,30s;10,1m;10,30s").unwrap();
    assert_eq!(plan.total_users(), 10);
}

// Test edge cases for time parsing
#[test]
fn test_time_parsing_edge_cases() {
    // Maximum single digit hours/minutes/seconds
    let plan = TestPlan::from_str("5,9h9m9s").unwrap();
    assert_eq!(plan.steps, vec![(5, 32_949_000)]); // 9*3600 + 9*60 + 9 = 32949 seconds

    // Multi-digit time components
    let plan = TestPlan::from_str("10,12h45m30s").unwrap();
    assert_eq!(plan.steps, vec![(10, 45_930_000)]); // 12*3600 + 45*60 + 30 = 45930 seconds

    // Just seconds with large number
    let plan = TestPlan::from_str("10,3661s").unwrap();
    assert_eq!(plan.steps, vec![(10, 3_661_000)]);

    // Zero time components
    let plan = TestPlan::from_str("10,0h0m30s").unwrap();
    assert_eq!(plan.steps, vec![(10, 30_000)]);

    let plan = TestPlan::from_str("10,1h0m0s").unwrap();
    assert_eq!(plan.steps, vec![(10, 3_600_000)]);
}

// Test current step initialization
#[test]
fn test_current_step_initialization() {
    let plan = TestPlan::from_str("10,30s;20,1m").unwrap();
    assert_eq!(plan.current, 0); // Should always start at 0

    // Empty new plan
    let plan = TestPlan::new();
    assert_eq!(plan.current, 0);
    assert!(plan.steps.is_empty());
}

// Test partial time formats
#[test]
fn test_partial_time_formats() {
    // Only hours and seconds (no minutes)
    let plan = TestPlan::from_str("10,1h30s").unwrap();
    assert_eq!(plan.steps, vec![(10, 3_630_000)]); // 1*3600 + 30 = 3630 seconds

    // Only minutes and seconds (no hours)
    let plan = TestPlan::from_str("10,5m45s").unwrap();
    assert_eq!(plan.steps, vec![(10, 345_000)]); // 5*60 + 45 = 345 seconds
}

// Test realistic load test scenarios
#[test]
fn test_realistic_load_test_scenarios() {
    // Typical web application load test
    let plan = TestPlan::from_str("0,0;10,1m;50,2m;100,5m;50,1m;0,30s").unwrap();
    assert_eq!(
        plan.steps,
        vec![
            (0, 0),         // Start
            (10, 60_000),   // Ramp to 10 users over 1 minute
            (50, 120_000),  // Ramp to 50 users over 2 minutes
            (100, 300_000), // Ramp to 100 users over 5 minutes
            (50, 60_000),   // Reduce to 50 users over 1 minute
            (0, 30_000)     // Shut down over 30 seconds
        ]
    );
    assert_eq!(plan.total_users(), 100);

    // Simple spike test
    let plan = TestPlan::from_str("1,10s;100,30s;1,10s").unwrap();
    assert_eq!(
        plan.steps,
        vec![
            (1, 10_000),   // Start with 1 user
            (100, 30_000), // Spike to 100 users
            (1, 10_000)    // Return to 1 user
        ]
    );
    assert_eq!(plan.total_users(), 100);
}
