# Baseline Feature Implementation Strategy

## Overview

The baseline comparison feature for Goose load testing framework was implemented using a sequential three-branch strategy to break down a large feature (~2,100 lines across 18 files) into manageable, focused pull requests.

### Current State

The `baseline-infrastructure` branch has been completed and provides:

- CLI integration for `--baseline-file` option
- Configuration structure ready for baseline file paths
- Foundation for subsequent processing and reporting logic
- Clean compilation without warnings or errors

The infrastructure is now ready for the processing logic implementation in the next PR.

## Three-Branch Sequential Strategy

### Branch 1: `baseline-infrastructure` (THIS BRANCH)

**Base**: `main`
**Purpose**: Foundation layer for baseline comparison functionality

#### What This PR Accomplishes

This PR establishes the basic infrastructure required for baseline file processing:

1. **CLI Option Addition**
   - Added `--baseline-file` command-line option to `GooseConfiguration`
   - Option accepts path to a baseline JSON report file
   - Integrated with existing CLI parsing infrastructure using `gumdrop`

2. **Configuration Structure Updates**
   - Added `baseline_file: Option<String>` field to `GooseConfiguration` struct
   - Added corresponding field to `GooseDefaults` struct for programmatic configuration
   - Implemented proper configuration precedence handling

3. **Dead Code Warning Resolution**
   - Applied `#[allow(dead_code)]` attribute to `baseline_file` field in `GooseDefaults`
   - This suppresses warnings since the field is defined but not yet used in this PR
   - Field usage is implemented in subsequent PRs

4. **NullableFloat Infrastructure**
   - Added `src/metrics/nullable.rs` with `NullableFloat` struct
   - Handles JSON deserialization where `null` values become `NaN` in Rust
   - Implements necessary traits: `Deref`, `DerefMut`, `From<f32>`, `Display`
   - Custom serde deserializer converts `Option<f32>` to `NullableFloat` with NaN fallback
   - Foundation for handling baseline comparison data with missing/null values

#### Why This PR Stops Here

This PR intentionally contains only the configuration infrastructure without any processing logic. This approach:

- Provides a clean foundation for subsequent work
- Allows for focused review of CLI and configuration changes
- Prevents compilation warnings that would occur if configuration was added without usage
- Establishes the interface contract before implementing the business logic

#### Key Files Modified

- `src/config.rs`: Added baseline_file configuration fields and CLI option
- `src/metrics/nullable.rs`: Added NullableFloat struct for JSON null handling
- Configuration validation and help text integration

### Branch 2: `baseline-processing` (ANOTHER BRANCH)

**Base**: `baseline-infrastructure` (not `main`)
**Purpose**: Core processing logic for baseline comparison

#### Planned Content

- `DeltaValue` trait implementation for calculating differences between metrics
- `Value<T>` enum for representing plain values or values with deltas
- `NullableFloat` struct for handling NaN values in JSON serialization
- Baseline file loading and validation logic
- Delta calculation algorithms

### Branch 3: `baseline-reports` (ANOTHER BRANCH)

**Base**: `baseline-processing` (not `main` or `baseline-infrastructure`)
**Purpose**: Integration with report generation systems

#### Planned Content

- HTML report templates updated to display baseline comparisons
- Markdown report integration with delta display
- Report formatting functions for baseline data
- Integration with existing report generation pipeline

## Technical Rationale

### Sequential Dependency Chain

```
main → baseline-infrastructure → baseline-processing → baseline-reports
```

This linear progression ensures:

1. **Clean Separation**: Each PR addresses one logical layer of functionality
2. **Incremental Building**: Each branch builds upon the previous work
3. **Manageable Review Size**: Smaller, focused PRs instead of 2,100+ line monolith
4. **Independent Testing**: Each layer can be tested before proceeding to the next
