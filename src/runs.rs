//! Saved runs: every load test is saved by default to its own directory under
//! the runs directory (`goose-runs` unless `--runs-dir` says otherwise).

/// The JSON report of a saved run, also read by `--baseline-file <run dir>`.
pub(crate) const REPORT_JSON: &str = "report.json";
