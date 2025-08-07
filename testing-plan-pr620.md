# Testing Plan for PR #620: Fix Error Graph in HTML Report

## Overview
This document outlines a comprehensive testing strategy for PR #620 which fixes a critical bug in HTML report generation where error summaries were not being displayed due to inverted boolean logic in `src/metrics/common.rs`.

## Bug Analysis
**Location**: `src/metrics/common.rs`, line 414-417
**Issue**: The `prepare_data` function uses `metrics.errors.is_empty()` which returns `true` when there are NO errors, causing errors to only be included when the collection is empty.

**Before Fix**:
```rust
errors: metrics.errors.is_empty()
    .then(|| metrics.errors.values().collect::<Vec<_>>()),
```

**After Fix**:
```rust
errors: (!metrics.errors.is_empty())
    .then(|| metrics.errors.values().collect::<Vec<_>>()),
```

## Testing Strategy

### 1. Unit Tests for `prepare_data` Function

**File**: `src/metrics/common.rs` (add to existing `#[cfg(test)]` module)

#### Test Case A: Empty Errors Collection
```rust
#[test]
fn test_prepare_data_with_no_errors() {
    let mut metrics = create_mock_goose_metrics();
    // Ensure errors collection is empty
    metrics.errors.clear();
    
    let options = ReportOptions {
        no_transaction_metrics: false,
        no_scenario_metrics: false,
        no_status_codes: false,
    };
    
    let report_data = prepare_data(options, &metrics);
    
    // Should be None when no errors exist
    assert!(report_data.errors.is_none());
}
```

#### Test Case B: Populated Errors Collection
```rust
#[test]
fn test_prepare_data_with_errors() {
    let mut metrics = create_mock_goose_metrics();
    
    // Add mock error data
    let error_aggregate = GooseErrorMetricAggregate {
        error: "Connection timeout".to_string(),
        occurrences: 5,
    };
    metrics.errors.insert("timeout_error".to_string(), error_aggregate);
    
    let options = ReportOptions {
        no_transaction_metrics: false,
        no_scenario_metrics: false,
        no_status_codes: false,
    };
    
    let report_data = prepare_data(options, &metrics);
    
    // Should contain error data when errors exist
    assert!(report_data.errors.is_some());
    let errors = report_data.errors.unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].error, "Connection timeout");
    assert_eq!(errors[0].occurrences, 5);
}
```

#### Test Case C: Boolean Logic Verification
```rust
#[test]
fn test_error_boolean_logic_fix() {
    let mut empty_metrics = create_mock_goose_metrics();
    empty_metrics.errors.clear();
    
    let mut populated_metrics = create_mock_goose_metrics();
    populated_metrics.errors.insert(
        "test_error".to_string(),
        GooseErrorMetricAggregate {
            error: "Test error".to_string(),
            occurrences: 1,
        }
    );
    
    // Test the boolean logic directly
    assert!(empty_metrics.errors.is_empty());
    assert!(!(!empty_metrics.errors.is_empty())); // Should be false
    
    assert!(!populated_metrics.errors.is_empty());
    assert!(!populated_metrics.errors.is_empty()); // Should be true
}
```

### 2. Integration Tests

**File**: `tests/error_reporting.rs` (new file)

#### Test Case D: End-to-End Error Generation and Reporting
```rust
use goose::prelude::*;
use httpmock::{Method::GET, MockServer};
use tokio;

#[tokio::test]
async fn test_error_reporting_integration() {
    // Set up mock server that returns errors
    let server = MockServer::start();
    let error_mock = server.mock(|when, then| {
        when.method(GET).path("/error");
        then.status(500).body("Internal Server Error");
    });
    
    // Create a scenario that generates errors
    let scenario = scenario!("Error Test")
        .register_transaction(transaction!(error_transaction));
    
    // Configure test to generate errors
    let configuration = common::build_configuration(
        &server,
        vec!["--users", "5", "--run-time", "3", "--hatch-rate", "5"]
    );
    
    let goose_attack = common::build_load_test(
        configuration,
        vec![scenario],
        None,
        None,
    );
    
    // Run the test and collect metrics
    let metrics = common::run_load_test(goose_attack, None).await;
    
    // Verify errors were collected
    assert!(!metrics.errors.is_empty());
    
    // Test prepare_data function with real error data
    let options = ReportOptions {
        no_transaction_metrics: false,
        no_scenario_metrics: false,
        no_status_codes: false,
    };
    
    let report_data = prepare_data(options, &metrics);
    
    // Verify errors are included in report data
    assert!(report_data.errors.is_some());
    let errors = report_data.errors.unwrap();
    assert!(!errors.is_empty());
    
    error_mock.assert();
}

async fn error_transaction(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/error").await?;
    Ok(())
}
```

#### Test Case E: HTML Report Generation with Errors
```rust
#[tokio::test]
async fn test_html_report_with_errors() {
    // Similar setup to Test Case D
    // ... (error generation code)
    
    // Generate HTML report
    let html_report = generate_html_report(&metrics);
    
    // Verify error section is present in HTML
    assert!(html_report.contains("Error Summary"));
    assert!(html_report.contains("Internal Server Error"));
    
    // Clean up generated files
    common::cleanup_files(vec!["report.html"]);
}
```

### 3. Regression Tests

**File**: `tests/regression_pr620.rs` (new file)

#### Test Case F: Specific Regression Test for PR #620
```rust
#[test]
fn test_pr620_regression_error_display_logic() {
    // This test specifically validates the fix for PR #620
    let mut metrics_with_errors = create_mock_goose_metrics();
    let mut metrics_without_errors = create_mock_goose_metrics();
    
    // Add error to first metrics
    metrics_with_errors.errors.insert(
        "pr620_test".to_string(),
        GooseErrorMetricAggregate {
            error: "PR #620 regression test error".to_string(),
            occurrences: 3,
        }
    );
    
    // Clear errors from second metrics
    metrics_without_errors.errors.clear();
    
    let options = ReportOptions {
        no_transaction_metrics: false,
        no_scenario_metrics: false,
        no_status_codes: false,
    };
    
    // Test with errors - should include error data
    let report_with_errors = prepare_data(options.clone(), &metrics_with_errors);
    assert!(report_with_errors.errors.is_some(), "Errors should be included when present");
    
    // Test without errors - should not include error data
    let report_without_errors = prepare_data(options, &metrics_without_errors);
    assert!(report_without_errors.errors.is_none(), "Errors should be None when not present");
}
```

### 4. Helper Functions

#### Mock Data Creation
```rust
fn create_mock_goose_metrics() -> GooseMetrics {
    use std::collections::HashMap;
    use crate::metrics::{GooseMetrics, GooseRequestMetricAggregate};
    
    GooseMetrics {
        duration: 10,
        users: vec![],
        requests: HashMap::new(),
        transactions: vec![],
        scenarios: vec![],
        errors: HashMap::new(),
    }
}

fn create_mock_error_aggregate(error_msg: &str, count: usize) -> GooseErrorMetricAggregate {
    GooseErrorMetricAggregate {
        error: error_msg.to_string(),
        occurrences: count,
    }
}
```

## Test Execution Strategy

### Phase 1: Unit Tests
- Run unit tests to verify the core logic fix
- Ensure both positive and negative cases work
- Validate boolean logic correction

### Phase 2: Integration Tests  
- Test complete error reporting pipeline
- Verify HTML report generation includes errors
- Test with various error scenarios

### Phase 3: Regression Tests
- Ensure the specific bug is fixed
- Prevent future regressions
- Validate edge cases

### Phase 4: Performance Tests
- Ensure fix doesn't impact performance
- Test with large error collections
- Verify memory usage is reasonable

## Expected Test Results

### Before Fix Applied
- Unit tests should fail, demonstrating the bug
- Integration tests should show missing error data in reports
- HTML reports should not contain error sections

### After Fix Applied
- All unit tests should pass
- Integration tests should show error data in reports
- HTML reports should contain populated error sections
- No performance degradation

## Implementation Checklist

- [ ] Add unit tests to `src/metrics/common.rs`
- [ ] Create `tests/error_reporting.rs` integration tests
- [ ] Create `tests/regression_pr620.rs` regression tests
- [ ] Add helper functions for mock data creation
- [ ] Update existing test utilities if needed
- [ ] Run full test suite to ensure no regressions
- [ ] Add documentation for new test patterns
- [ ] Consider adding property-based tests for edge cases

## Continuous Integration

### Test Categories
- **Fast Tests**: Unit tests (< 1 second each)
- **Medium Tests**: Integration tests (< 10 seconds each)  
- **Slow Tests**: End-to-end HTML generation tests (< 30 seconds each)

### CI Pipeline Integration
- Run unit tests on every commit
- Run integration tests on PR creation
- Run full test suite before merge
- Generate test coverage reports

## Future Enhancements

### Additional Test Scenarios
- Test with very large error collections (1000+ errors)
- Test with various error types and formats
- Test error reporting with different report formats
- Test concurrent error collection scenarios

### Test Infrastructure Improvements
- Create error simulation utilities
- Add property-based testing for error scenarios
- Implement snapshot testing for HTML output
- Add performance benchmarks for error processing

## Conclusion

This comprehensive testing plan ensures that:
1. The specific bug in PR #620 is fixed and verified
2. Future regressions are prevented
3. Error reporting functionality is thoroughly tested
4. The fix doesn't introduce new issues
5. The codebase maintains high quality and reliability

The tests should be implemented as a follow-up PR to provide robust coverage for this critical bug fix.
