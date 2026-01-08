# Baseline Processing Implementation (PR2)

## Overview

This document covers the second phase of the baseline comparison feature implementation: core processing logic and delta calculation infrastructure. This builds upon the foundation established in [PR1 (baseline-infrastructure)](https://github.com/tag1consulting/goose/pull/659).

## PR Context

**Base Branch**: `baseline-infrastructure` (not `main`)  
**Purpose**: Core processing logic for baseline comparison  

## What This PR Accomplishes

This PR implements the core algorithmic and data structure components required for baseline comparison:

### 1. Delta Calculation Infrastructure (`src/metrics/delta.rs`)

**DeltaValue Trait**
- Core trait for types that can calculate deltas against baseline values
- Associated `Delta` type defines the result type for delta calculations
- Implementations for primitive numeric types (`usize`, `f32`, `f64`, `u64`, `u16`)
- Each implementation includes overflow protection using `ISIZE_MIN_ABS` constant

**Value<T> Enum**
- Represents either a plain value or a value with an associated delta to baseline
- `Plain(T)` variant for standalone values
- `WithDelta { value: T, delta: T::Delta }` for values with baseline comparison
- Generic over any type implementing `DeltaValue`

**Overflow Protection**
- `ISIZE_MIN_ABS` constant for safe arithmetic operations
- Prevents integer overflow in delta calculations
- Used in all numeric delta implementations

### 2. Enhanced NullableFloat (`src/metrics/nullable.rs`)

Building on the basic `NullableFloat` from PR1, this PR adds:

**DeltaValue Implementation**
- Enables `NullableFloat` to participate in baseline comparisons
- Custom delta calculation handling NaN values appropriately
- Preserves NaN semantics during baseline comparison operations

### 3. Data Structure Enhancements (`src/metrics/common.rs`)

**ReportOptions Structure**
- Consolidates report generation configuration
- Fields: `no_transaction_metrics`, `no_scenario_metrics`, `no_status_codes`
- Provides clean interface for conditional report generation

**ReportData Structure**
- Container for processed metrics data ready for report generation
- Organizes different metric types for consistent report processing
- Foundation for baseline-enhanced reporting

**Baseline Processing Functions**
- `load_baseline_file()`: Loads and validates baseline JSON files
- `prepare_data_with_baseline()`: Processes metrics with baseline comparison
- Error handling for invalid baseline files and data mismatches

### 4. Module Organization

**Public API Exposure**
- `DeltaValue` trait made public for external implementations
- `Value<T>` enum exported for use in report generation
- Clean module boundaries between delta calculation and data processing

**Integration Points**
- Delta calculation integrated into existing metrics processing pipeline
- Baseline loading functions ready for integration with CLI options
- Report data structures prepared for template integration

## Why This PR Stops Here

This PR contains only the processing logic without report generation integration. This separation:

- Enables focused testing of delta calculation algorithms
- Provides clean API boundaries for report generation
- Allows validation of baseline file handling independently
- Maintains manageable PR size for effective code review

## Key Files Added/Modified

- `src/metrics/delta.rs`: Complete delta calculation infrastructure
- `src/metrics/nullable.rs`: Enhanced with DeltaValue implementation  
- `src/metrics/common.rs`: Baseline processing and data structures
- `src/metrics.rs`: Module organization and public API exports

## Technical Implementation Details

### Delta Calculation Algorithm

For numeric types, delta calculation follows the pattern:
```rust
fn delta(&self, baseline: &Self) -> Self::Delta {
    if baseline_value > self_value {
        -(baseline_value - self_value) // Negative delta (improvement)
    } else {
        self_value - baseline_value    // Positive delta (regression)
    }
}
```

### NaN Handling Strategy

`NullableFloat` delta calculation preserves NaN semantics:
- `NaN` compared to any value results in `NaN` delta
- Maintains consistency with IEEE 754 floating-point standards
- Enables graceful handling of missing baseline data

### Error Handling

Baseline processing includes comprehensive error handling:
- Invalid JSON file format detection
- Missing baseline file handling
- Data type mismatch validation
- Clear error messages for debugging

## Next Phase: PR3 (Reports Integration)

The third PR will integrate this processing logic with the report generation system:
- HTML template updates to display delta comparisons
- Markdown report integration
- Report formatting functions using the `Value<T>` enum
- End-to-end baseline comparison workflow

## Dependencies

This PR builds directly on the infrastructure from:
- [PR1: baseline-infrastructure](https://github.com/tag1consulting/goose/pull/659) - CLI options and basic configuration
