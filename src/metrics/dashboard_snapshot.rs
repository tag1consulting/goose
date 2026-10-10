//! Compact dashboard snapshot DTO and pure builder.
//!
//! Always compiled (no axum dependency). Used by `MetricsCommand::GetDashboardSnapshot`
//! and later by the HTTP dashboard server.

use super::{
    calculate_response_time_percentile, merge_times, per_second_calculations, update_max_time,
    update_min_time, GooseErrorMetricAggregate, GooseMetrics, GooseRequestMetricAggregate,
    GooseRequestMetricTimingData, ScenarioMetricAggregate, TransactionMetricAggregate,
};
use chrono::prelude::*;
use serde::Serialize;
use std::collections::BTreeMap;

/// Maximum per-request rows included in a snapshot table.
pub(crate) const MAX_REQUEST_ROWS: usize = 100;
/// Maximum error rows included in a snapshot table.
pub(crate) const MAX_ERROR_ROWS: usize = 50;
/// Default trailing series window length in seconds (5 minutes).
///
/// Consumed by the dashboard HTTP server when issuing GetDashboardSnapshot.
#[cfg_attr(not(feature = "dashboard"), allow(dead_code))]
pub(crate) const SERIES_WINDOW_SECS: u32 = 300;

/// Wire format version of [`DashboardSnapshot`]. The dashboard client refuses a
/// snapshot with any other version; bump it on a breaking change to the shape.
pub(crate) const SNAPSHOT_VERSION: u32 = 1;

/// Wire format versioned so UI and server can evolve independently.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct DashboardSnapshot {
    pub version: u32,
    pub generated_at: DateTime<Utc>,
    pub goose_version: String,

    pub phase: String,
    /// True while a cancel (Stop, `shutdown`, Ctrl-C) ramps the run down,
    /// until it reaches idle. A test plan's own ramp down leaves it false, so
    /// clients can tell the two `decrease` phases apart.
    pub stopping: bool,
    pub duration_secs: u64,
    /// Users currently running (main-loop active count).
    pub active_users: u64,
    /// Peak concurrent users observed during this run (high-water mark).
    pub maximum_users: u64,
    /// Current test-plan step target — users the attack is increasing or
    /// decreasing toward (not the historical peak).
    pub target_users: u64,
    pub total_users: u64,
    pub hosts: Vec<String>,

    pub aggregate: AggregateMetrics,
    pub requests: Vec<RequestRow>,
    pub errors: Vec<ErrorRow>,
    /// One row per registered scenario, in registration order. Never truncated.
    pub scenarios: Vec<ScenarioRow>,
    /// One row per registered transaction, grouped by scenario, each group in
    /// registration order. Never truncated.
    pub transactions: Vec<TransactionRow>,
    pub series: SeriesWindow,

    pub flags: SnapshotFlags,
    /// Whether runs are saved, and the last run Goose saved or tried to save.
    pub save: SnapshotSave,
}

/// Saving runs, as the dashboard shows it. Also the data of the SSE `closed`
/// event, so a page learns where the last run was saved when Goose exits.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct SnapshotSave {
    /// `on`, `off` (`--no-save` or `--no-metrics`), or `failed` (the runs
    /// directory or the run's directory couldn't be created).
    #[cfg_attr(test, ts(type = "\"on\" | \"off\" | \"failed\""))]
    pub state: String,
    /// Why the last run couldn't be saved, or why this run isn't saved.
    pub reason: Option<String>,
    /// The runs directory as given, never resolved to an absolute path.
    pub dir: String,
    /// The id of the last run Goose saved or tried to save.
    pub last_run: Option<String>,
}

impl SnapshotSave {
    /// Not saving, for a snapshot built where the save state is not known.
    #[cfg(test)]
    pub(crate) fn off(dir: &str) -> Self {
        SnapshotSave {
            state: "off".to_string(),
            reason: None,
            dir: dir.to_string(),
            last_run: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct AggregateMetrics {
    pub total_requests: u64,
    pub total_failures: u64,
    pub requests_per_second: f64,
    pub failures_per_second: f64,
    pub failure_rate: f64,
    pub response_time_avg_ms: f64,
    pub response_time_min_ms: u64,
    pub response_time_max_ms: u64,
    pub percentile_ms: Percentiles,
    /// True iff coordinated-omission metrics object is present AND has recorded
    /// at least one CO event (or synthetic request).
    pub co_active: bool,
    pub co_adjusted: Option<CoAdjusted>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RequestRow {
    pub method: String,
    pub name: String,
    pub request_count: u64,
    pub failure_count: u64,
    pub requests_per_second: f64,
    pub failures_per_second: f64,
    pub response_time_avg_ms: f64,
    pub response_time_min_ms: u64,
    pub response_time_max_ms: u64,
    pub percentile_ms: Percentiles,
    pub status_codes: Vec<(u16, u64)>,
    pub co_adjusted: Option<CoAdjusted>,
}

/// Metrics for one registered scenario.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct ScenarioRow {
    /// Goose's own scenario index, counted from 0 in registration order. The
    /// dashboard shows it counted from 1.
    pub scenario_index: u64,
    pub scenario_name: String,
    /// Distinct users that have finished at least one run of this scenario
    /// since the last metrics reset; not the users running it now.
    pub users: u64,
    /// Completed passes through the scenario's weighted transactions.
    pub run_count: u64,
    pub runs_per_second: f64,
    pub response_time_avg_ms: f64,
    pub response_time_min_ms: u64,
    pub response_time_max_ms: u64,
    pub percentile_ms: Percentiles,
}

/// Metrics for one registered transaction.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct TransactionRow {
    /// Goose's own scenario index, counted from 0 in registration order. The
    /// dashboard shows it counted from 1.
    pub scenario_index: u64,
    pub scenario_name: String,
    /// Goose's own transaction index within its scenario, counted from 0 in
    /// registration order. The dashboard shows it counted from 1.
    pub transaction_index: u64,
    /// Empty when the transaction has no name.
    pub transaction_name: String,
    /// Successful plus failed runs.
    pub run_count: u64,
    pub failure_count: u64,
    pub runs_per_second: f64,
    pub failures_per_second: f64,
    pub response_time_avg_ms: f64,
    pub response_time_min_ms: u64,
    pub response_time_max_ms: u64,
    pub percentile_ms: Percentiles,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct ErrorRow {
    pub method: String,
    pub name: String,
    pub error: String,
    pub occurrences: u64,
}

/// Trailing per-second aggregate series for dashboard charts.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct SeriesWindow {
    /// First exported bucket index in seconds-from-test-start.
    pub start_second: u64,
    pub rps: Vec<f64>,
    pub fps: Vec<f64>,
    pub users: Vec<u64>,
    pub avg_latency_ms: Vec<f64>,
}

impl SeriesWindow {
    /// Empty window (no series data recorded yet).
    pub(crate) fn empty() -> Self {
        SeriesWindow {
            start_second: 0,
            rps: Vec::new(),
            fps: Vec::new(),
            users: Vec::new(),
            avg_latency_ms: Vec::new(),
        }
    }
}

/// Response times including the synthetic times coordinated omission mitigation adds. Present only when mitigation has recorded at least one event.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CoAdjusted {
    pub response_time_avg_ms: f64,
    pub response_time_max_ms: u64,
    pub percentile_ms: Percentiles,
}

impl CoAdjusted {
    fn from_timing_data(data: &GooseRequestMetricTimingData) -> Self {
        CoAdjusted {
            response_time_avg_ms: average_ms(data.total_time, data.counter),
            response_time_max_ms: data.maximum_time as u64,
            percentile_ms: Percentiles::from_times(
                &data.times,
                data.counter,
                data.minimum_time,
                data.maximum_time,
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct Percentiles {
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
}

impl Percentiles {
    fn zero() -> Self {
        Percentiles {
            p50: 0,
            p95: 0,
            p99: 0,
        }
    }

    fn from_times(
        times: &BTreeMap<usize, usize>,
        total_requests: usize,
        min: usize,
        max: usize,
    ) -> Self {
        if total_requests == 0 {
            return Self::zero();
        }
        Percentiles {
            p50: calculate_response_time_percentile(times, total_requests, min, max, 0.5) as u64,
            p95: calculate_response_time_percentile(times, total_requests, min, max, 0.95) as u64,
            p99: calculate_response_time_percentile(times, total_requests, min, max, 0.99) as u64,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct SnapshotFlags {
    pub metrics_disabled: bool,
    /// True under `--no-transaction-metrics`, and under `--no-metrics` too;
    /// `transactions` is then empty.
    pub transaction_metrics_disabled: bool,
    /// True under `--no-scenario-metrics`, and under `--no-metrics` too;
    /// `scenarios` is then empty.
    pub scenario_metrics_disabled: bool,
    pub requests_truncated: bool,
    pub errors_truncated: bool,
    pub series_seconds: u32,
}

/// Inputs supplied by the metrics processor / main loop when building a snapshot.
pub(crate) struct DashboardSnapshotInput<'a> {
    pub metrics: &'a GooseMetrics,
    pub series: SeriesWindow,
    pub active_users: usize,
    pub maximum_users: usize,
    /// Current plan-step / configured target (see [`DashboardSnapshot::target_users`]).
    pub target_users: usize,
    pub total_users: usize,
    pub phase: String,
    /// See [`DashboardSnapshot::stopping`].
    pub stopping: bool,
    pub series_window_secs: u32,
    pub no_status_codes: bool,
    pub metrics_disabled: bool,
    /// `--no-transaction-metrics`.
    pub no_transaction_metrics: bool,
    /// `--no-scenario-metrics`.
    pub no_scenario_metrics: bool,
    /// See [`DashboardSnapshot::save`].
    pub save: SnapshotSave,
}

/// Build a compact [`DashboardSnapshot`] from live metrics + series window.
pub(crate) fn build_dashboard_snapshot(input: DashboardSnapshotInput<'_>) -> DashboardSnapshot {
    // With `--no-metrics`, document an honest empty state: no tables, no series,
    // and no truncation flags. Runtime still reports phase/user counts.
    if input.metrics_disabled {
        let mut hosts: Vec<String> = input.metrics.hosts.iter().cloned().collect();
        hosts.sort();
        return DashboardSnapshot {
            version: SNAPSHOT_VERSION,
            generated_at: Utc::now(),
            goose_version: env!("CARGO_PKG_VERSION").to_string(),
            phase: input.phase,
            stopping: input.stopping,
            duration_secs: input.metrics.duration as u64,
            active_users: input.active_users as u64,
            maximum_users: input.maximum_users as u64,
            target_users: input.target_users as u64,
            total_users: input.total_users as u64,
            hosts,
            aggregate: AggregateMetrics {
                total_requests: 0,
                total_failures: 0,
                requests_per_second: 0.0,
                failures_per_second: 0.0,
                failure_rate: 0.0,
                response_time_avg_ms: 0.0,
                response_time_min_ms: 0,
                response_time_max_ms: 0,
                percentile_ms: Percentiles::zero(),
                co_active: false,
                co_adjusted: None,
            },
            requests: Vec::new(),
            errors: Vec::new(),
            scenarios: Vec::new(),
            transactions: Vec::new(),
            series: SeriesWindow::empty(),
            flags: SnapshotFlags {
                metrics_disabled: true,
                transaction_metrics_disabled: true,
                scenario_metrics_disabled: true,
                requests_truncated: false,
                errors_truncated: false,
                series_seconds: input.series_window_secs,
            },
            save: input.save,
        };
    }

    let duration = input.metrics.duration;

    let mut aggregate_total_count: usize = 0;
    let mut aggregate_fail_count: usize = 0;
    let mut aggregate_response_time_counter: usize = 0;
    let mut aggregate_response_time_total: usize = 0;
    let mut aggregate_min: usize = 0;
    let mut aggregate_max: usize = 0;
    let mut aggregate_times: BTreeMap<usize, usize> = BTreeMap::new();

    let co_active = input
        .metrics
        .coordinated_omission_metrics
        .as_ref()
        .map(|m| m.has_events())
        .unwrap_or(false);
    // Adjusted times across every request, built only while mitigation has
    // events. A request without adjusted data contributes its measured times,
    // which for it are the same thing. This deliberately differs from the
    // report's adjusted aggregate, which leaves such requests out.
    let mut co_aggregate: Option<GooseRequestMetricTimingData> =
        co_active.then(|| GooseRequestMetricTimingData::new(None));

    // Rank by count first; only materialize full table rows for the top N so
    // high request-name cardinality does not pay percentile/status work ~1 Hz
    // for rows the UI never shows.
    let mut request_rank: Vec<(&GooseRequestMetricAggregate, usize)> =
        Vec::with_capacity(input.metrics.requests.len());
    for request in input.metrics.requests.values() {
        let total_count = request.success_count + request.fail_count;
        aggregate_total_count += total_count;
        aggregate_fail_count += request.fail_count;
        aggregate_response_time_counter += request.raw_data.counter;
        aggregate_response_time_total += request.raw_data.total_time;
        aggregate_min = update_min_time(aggregate_min, request.raw_data.minimum_time);
        aggregate_max = update_max_time(aggregate_max, request.raw_data.maximum_time);
        aggregate_times = merge_times(aggregate_times, &request.raw_data.times);
        if let Some(co_aggregate) = co_aggregate.as_mut() {
            co_aggregate.merge(
                request
                    .coordinated_omission_data
                    .as_ref()
                    .unwrap_or(&request.raw_data),
            );
        }
        request_rank.push((request, total_count));
    }

    request_rank.sort_by(|(a, a_count), (b, b_count)| {
        b_count
            .cmp(a_count)
            .then_with(|| a.method_label().cmp(b.method_label()))
            .then_with(|| a.path.cmp(&b.path))
    });
    let requests_truncated = request_rank.len() > MAX_REQUEST_ROWS;
    request_rank.truncate(MAX_REQUEST_ROWS);
    let request_rows: Vec<RequestRow> = request_rank
        .into_iter()
        .map(|(request, _)| {
            request_row_from_aggregate(request, duration, input.no_status_codes, co_active)
        })
        .collect();

    let mut error_rank: Vec<&GooseErrorMetricAggregate> = input.metrics.errors.values().collect();
    error_rank.sort_by(|a, b| {
        b.occurrences
            .cmp(&a.occurrences)
            .then_with(|| a.method_label().cmp(b.method_label()))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.error.cmp(&b.error))
    });
    let errors_truncated = error_rank.len() > MAX_ERROR_ROWS;
    error_rank.truncate(MAX_ERROR_ROWS);
    let error_rows: Vec<ErrorRow> = error_rank
        .into_iter()
        .map(error_row_from_aggregate)
        .collect();

    // Every registered scenario and transaction gets a row: their number is
    // fixed by the load test's code, so there is no cap.
    let scenario_rows: Vec<ScenarioRow> = if input.no_scenario_metrics {
        Vec::new()
    } else {
        input
            .metrics
            .scenarios
            .iter()
            .map(|scenario| scenario_row_from_aggregate(scenario, duration))
            .collect()
    };
    let transaction_rows: Vec<TransactionRow> = if input.no_transaction_metrics {
        Vec::new()
    } else {
        input
            .metrics
            .transactions
            .iter()
            .flatten()
            .map(|transaction| transaction_row_from_aggregate(transaction, duration))
            .collect()
    };

    let (rps, fps) = per_second_calculations(duration, aggregate_total_count, aggregate_fail_count);
    let failure_rate = if aggregate_total_count > 0 {
        aggregate_fail_count as f64 / aggregate_total_count as f64
    } else {
        0.0
    };
    let response_time_avg_ms = if aggregate_response_time_counter > 0 {
        aggregate_response_time_total as f64 / aggregate_response_time_counter as f64
    } else {
        0.0
    };

    let mut hosts: Vec<String> = input.metrics.hosts.iter().cloned().collect();
    hosts.sort();

    DashboardSnapshot {
        version: SNAPSHOT_VERSION,
        generated_at: Utc::now(),
        goose_version: env!("CARGO_PKG_VERSION").to_string(),
        phase: input.phase,
        stopping: input.stopping,
        duration_secs: duration as u64,
        active_users: input.active_users as u64,
        maximum_users: input.maximum_users as u64,
        target_users: input.target_users as u64,
        total_users: input.total_users as u64,
        hosts,
        aggregate: AggregateMetrics {
            total_requests: aggregate_total_count as u64,
            total_failures: aggregate_fail_count as u64,
            requests_per_second: rps as f64,
            failures_per_second: fps as f64,
            failure_rate,
            response_time_avg_ms,
            response_time_min_ms: aggregate_min as u64,
            response_time_max_ms: aggregate_max as u64,
            percentile_ms: Percentiles::from_times(
                &aggregate_times,
                aggregate_response_time_counter,
                aggregate_min,
                aggregate_max,
            ),
            co_active,
            co_adjusted: co_aggregate.as_ref().map(CoAdjusted::from_timing_data),
        },
        requests: request_rows,
        errors: error_rows,
        scenarios: scenario_rows,
        transactions: transaction_rows,
        series: input.series,
        flags: SnapshotFlags {
            metrics_disabled: false,
            transaction_metrics_disabled: input.no_transaction_metrics,
            scenario_metrics_disabled: input.no_scenario_metrics,
            requests_truncated,
            errors_truncated,
            series_seconds: input.series_window_secs,
        },
        save: input.save,
    }
}

fn request_row_from_aggregate(
    request: &GooseRequestMetricAggregate,
    duration: usize,
    no_status_codes: bool,
    co_active: bool,
) -> RequestRow {
    let total_count = request.success_count + request.fail_count;
    let (rps, fps) = per_second_calculations(duration, total_count, request.fail_count);
    let avg = if request.raw_data.counter > 0 {
        request.raw_data.total_time as f64 / request.raw_data.counter as f64
    } else {
        0.0
    };
    let status_codes = if no_status_codes {
        Vec::new()
    } else {
        let mut codes: Vec<(u16, u64)> = request
            .status_code_counts
            .iter()
            .map(|(&code, &count)| (code, count as u64))
            .collect();
        codes.sort_by_key(|(code, _)| *code);
        codes
    };

    RequestRow {
        method: request.method_label().to_string(),
        name: request.path.clone(),
        request_count: total_count as u64,
        failure_count: request.fail_count as u64,
        requests_per_second: rps as f64,
        failures_per_second: fps as f64,
        response_time_avg_ms: avg,
        response_time_min_ms: request.raw_data.minimum_time as u64,
        response_time_max_ms: request.raw_data.maximum_time as u64,
        percentile_ms: Percentiles::from_times(
            &request.raw_data.times,
            request.raw_data.counter,
            request.raw_data.minimum_time,
            request.raw_data.maximum_time,
        ),
        status_codes,
        co_adjusted: if co_active {
            request
                .coordinated_omission_data
                .as_ref()
                .map(CoAdjusted::from_timing_data)
        } else {
            None
        },
    }
}

/// Average time in milliseconds, or 0 when nothing has been timed.
fn average_ms(total_time: usize, counter: usize) -> f64 {
    if counter > 0 {
        total_time as f64 / counter as f64
    } else {
        0.0
    }
}

fn scenario_row_from_aggregate(scenario: &ScenarioMetricAggregate, duration: usize) -> ScenarioRow {
    let (runs_per_second, _) = per_second_calculations(duration, scenario.counter, 0);
    ScenarioRow {
        scenario_index: scenario.index as u64,
        scenario_name: scenario.name.to_string(),
        users: scenario.users.len() as u64,
        run_count: scenario.counter as u64,
        runs_per_second: runs_per_second as f64,
        response_time_avg_ms: average_ms(scenario.total_time, scenario.counter),
        response_time_min_ms: scenario.min_time as u64,
        response_time_max_ms: scenario.max_time as u64,
        percentile_ms: Percentiles::from_times(
            &scenario.times,
            scenario.counter,
            scenario.min_time,
            scenario.max_time,
        ),
    }
}

fn transaction_row_from_aggregate(
    transaction: &TransactionMetricAggregate,
    duration: usize,
) -> TransactionRow {
    let run_count = transaction.success_count + transaction.fail_count;
    let (runs_per_second, failures_per_second) =
        per_second_calculations(duration, run_count, transaction.fail_count);
    TransactionRow {
        scenario_index: transaction.scenario_index as u64,
        scenario_name: transaction.scenario_name.to_string(),
        transaction_index: transaction.transaction_index as u64,
        transaction_name: transaction
            .transaction_name
            .name_for_transaction()
            .to_string(),
        run_count: run_count as u64,
        failure_count: transaction.fail_count as u64,
        runs_per_second: runs_per_second as f64,
        failures_per_second: failures_per_second as f64,
        response_time_avg_ms: average_ms(transaction.total_time, transaction.counter),
        response_time_min_ms: transaction.min_time as u64,
        response_time_max_ms: transaction.max_time as u64,
        percentile_ms: Percentiles::from_times(
            &transaction.times,
            transaction.counter,
            transaction.min_time,
            transaction.max_time,
        ),
    }
}

fn error_row_from_aggregate(error: &GooseErrorMetricAggregate) -> ErrorRow {
    ErrorRow {
        method: error.method_label().to_string(),
        name: error.name.clone(),
        error: error.error.clone(),
        occurrences: error.occurrences as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goose::{GooseMethod, TransactionName};
    use crate::metrics::coordinated_omission::CoordinatedOmissionMetrics;
    use crate::metrics::GooseCoordinatedOmissionMitigation;
    use crate::metrics::GooseRequestMetricAggregate;
    use std::sync::Arc;
    use std::time::Duration;

    fn empty_metrics() -> GooseMetrics {
        GooseMetrics::default()
    }

    fn make_request(
        path: &str,
        successes: usize,
        failures: usize,
        times: &[(usize, usize)],
    ) -> GooseRequestMetricAggregate {
        let mut agg = GooseRequestMetricAggregate::new(path, GooseMethod::Get, 0, "");
        agg.success_count = successes;
        agg.fail_count = failures;
        for &(time, count) in times {
            for _ in 0..count {
                agg.raw_data.record_time(time as u64);
            }
        }
        agg
    }

    #[test]
    fn builder_empty_metrics() {
        let metrics = empty_metrics();
        let snap = build_dashboard_snapshot(DashboardSnapshotInput {
            metrics: &metrics,
            series: SeriesWindow::empty(),
            active_users: 0,
            maximum_users: 10,
            target_users: 10,
            total_users: 10,
            phase: "idle".to_string(),
            stopping: false,
            series_window_secs: SERIES_WINDOW_SECS,
            no_status_codes: false,
            metrics_disabled: false,
            no_transaction_metrics: false,
            no_scenario_metrics: false,
            save: SnapshotSave::off("goose-runs"),
        });
        assert_eq!(snap.version, 1);
        assert_eq!(snap.phase, "idle");
        assert_eq!(snap.maximum_users, 10);
        assert_eq!(snap.target_users, 10);
        assert_eq!(snap.aggregate.total_requests, 0);
        assert!(!snap.aggregate.co_active);
        assert!(snap.requests.is_empty());
        assert!(snap.errors.is_empty());
        assert!(!snap.flags.requests_truncated);
        assert!(!snap.flags.errors_truncated);
        assert_eq!(snap.flags.series_seconds, SERIES_WINDOW_SECS);
    }

    #[test]
    fn builder_caps_request_and_error_rows() {
        let mut metrics = empty_metrics();
        metrics.duration = 10;
        for i in 0..(MAX_REQUEST_ROWS + 25) {
            let path = format!("/path-{i}");
            // Higher index → more requests so sort order is deterministic.
            let count = i + 1;
            metrics.requests.insert(
                format!("GET {path}"),
                make_request(&path, count, 0, &[(10, count)]),
            );
        }
        for i in 0..(MAX_ERROR_ROWS + 10) {
            let key = format!("err-{i}");
            let mut err = GooseErrorMetricAggregate::new(
                GooseMethod::Get,
                format!("/e{i}"),
                format!("error {i}"),
                "",
            );
            err.occurrences = i + 1;
            metrics.errors.insert(key, err);
        }

        let snap = build_dashboard_snapshot(DashboardSnapshotInput {
            metrics: &metrics,
            series: SeriesWindow::empty(),
            active_users: 5,
            maximum_users: 5,
            target_users: 5,
            total_users: 5,
            phase: "maintain".to_string(),
            stopping: false,
            series_window_secs: 60,
            no_status_codes: true,
            metrics_disabled: false,
            no_transaction_metrics: false,
            no_scenario_metrics: false,
            save: SnapshotSave::off("goose-runs"),
        });

        assert_eq!(snap.requests.len(), MAX_REQUEST_ROWS);
        assert!(snap.flags.requests_truncated);
        // Top row should be the highest count.
        assert_eq!(
            snap.requests[0].request_count,
            (MAX_REQUEST_ROWS + 25) as u64
        );
        assert!(snap.requests[0].status_codes.is_empty());

        assert_eq!(snap.errors.len(), MAX_ERROR_ROWS);
        assert!(snap.flags.errors_truncated);
        assert_eq!(snap.errors[0].occurrences, (MAX_ERROR_ROWS + 10) as u64);
    }

    #[test]
    fn co_active_requires_events_not_just_enabled() {
        let mut metrics = empty_metrics();
        // Mitigation enabled but zero events → co_active false.
        metrics.coordinated_omission_metrics = Some(CoordinatedOmissionMetrics::new(
            GooseCoordinatedOmissionMitigation::Average,
        ));
        let snap = build_dashboard_snapshot(DashboardSnapshotInput {
            metrics: &metrics,
            series: SeriesWindow::empty(),
            active_users: 0,
            maximum_users: 0,
            target_users: 0,
            total_users: 0,
            phase: "maintain".to_string(),
            stopping: false,
            series_window_secs: 300,
            no_status_codes: false,
            metrics_disabled: false,
            no_transaction_metrics: false,
            no_scenario_metrics: false,
            save: SnapshotSave::off("goose-runs"),
        });
        assert!(!snap.aggregate.co_active);

        // Record a CO event → co_active true.
        metrics
            .coordinated_omission_metrics
            .as_mut()
            .unwrap()
            .record_co_event(
                Duration::from_millis(100),
                Duration::from_millis(1000),
                5,
                0,
                "scenario".to_string(),
            );
        let snap = build_dashboard_snapshot(DashboardSnapshotInput {
            metrics: &metrics,
            series: SeriesWindow::empty(),
            active_users: 0,
            maximum_users: 0,
            target_users: 0,
            total_users: 0,
            phase: "maintain".to_string(),
            stopping: false,
            series_window_secs: 300,
            no_status_codes: false,
            metrics_disabled: false,
            no_transaction_metrics: false,
            no_scenario_metrics: false,
            save: SnapshotSave::off("goose-runs"),
        });
        assert!(snap.aggregate.co_active);
    }

    fn co_input(metrics: &GooseMetrics) -> DashboardSnapshotInput<'_> {
        DashboardSnapshotInput {
            metrics,
            series: SeriesWindow::empty(),
            active_users: 0,
            maximum_users: 0,
            target_users: 0,
            total_users: 0,
            phase: "maintain".to_string(),
            stopping: false,
            series_window_secs: 300,
            no_status_codes: false,
            metrics_disabled: false,
            no_transaction_metrics: false,
            no_scenario_metrics: false,
            save: SnapshotSave::off("goose-runs"),
        }
    }

    /// "/a" has 20 real times of 10 ms and 2 synthetic times of 5000 ms; "/b"
    /// has 10 real times of 20 ms and never needed backfill.
    fn add_co_requests(metrics: &mut GooseMetrics) {
        let mut a = GooseRequestMetricAggregate::new("/a", GooseMethod::Get, 0, "");
        a.success_count = 20;
        for _ in 0..20 {
            a.record_time(10, false);
        }
        a.record_time(5000, true);
        a.record_time(5000, true);
        metrics.requests.insert("GET /a".to_string(), a);
        metrics
            .requests
            .insert("GET /b".to_string(), make_request("/b", 10, 0, &[(20, 10)]));
    }

    fn record_one_co_event(metrics: &mut GooseMetrics) {
        metrics
            .coordinated_omission_metrics
            .as_mut()
            .unwrap()
            .record_co_event(
                Duration::from_millis(100),
                Duration::from_millis(1000),
                5,
                0,
                "scenario".to_string(),
            );
    }

    #[test]
    fn co_adjusted_carries_synthetic_times_and_percentile_ms_stays_measured() {
        let mut metrics = empty_metrics();
        metrics.coordinated_omission_metrics = Some(CoordinatedOmissionMetrics::new(
            GooseCoordinatedOmissionMitigation::Average,
        ));
        record_one_co_event(&mut metrics);
        add_co_requests(&mut metrics);
        let snap = build_dashboard_snapshot(co_input(&metrics));
        assert!(snap.aggregate.co_active);

        // Row "/a": adjusted includes the synthetic times, measured does not.
        let a = snap.requests.iter().find(|r| r.name == "/a").unwrap();
        assert_eq!(a.percentile_ms.p95, 10);
        assert_eq!(a.percentile_ms.p99, 10);
        assert_eq!(a.response_time_avg_ms, 10.0);
        assert_eq!(a.response_time_max_ms, 10);
        let a_co = a.co_adjusted.as_ref().expect("/a has adjusted data");
        assert_eq!(a_co.percentile_ms.p50, 10);
        assert_eq!(a_co.percentile_ms.p95, 5000);
        assert_eq!(a_co.percentile_ms.p99, 5000);
        assert_eq!(a_co.response_time_max_ms, 5000);
        assert!((a_co.response_time_avg_ms - 10_200.0 / 22.0).abs() < 1e-9);

        // Row "/b" never needed backfill, so it has no adjusted values.
        let b = snap.requests.iter().find(|r| r.name == "/b").unwrap();
        assert!(b.co_adjusted.is_none());

        // The aggregate: measured percentiles stay measured.
        assert_eq!(snap.aggregate.percentile_ms.p99, 20);
        assert_eq!(snap.aggregate.response_time_max_ms, 20);
        // Adjusted aggregate: "/a" adjusted times plus "/b" raw times (the
        // fallback), 32 times totalling 10400 ms.
        let agg_co = snap.aggregate.co_adjusted.as_ref().expect("adjusted");
        assert_eq!(agg_co.percentile_ms.p50, 10);
        assert_eq!(agg_co.percentile_ms.p95, 20);
        assert_eq!(agg_co.percentile_ms.p99, 5000);
        assert_eq!(agg_co.response_time_max_ms, 5000);
        assert!((agg_co.response_time_avg_ms - 10_400.0 / 32.0).abs() < 1e-9);
    }

    #[test]
    fn co_adjusted_absent_without_mitigation_events() {
        // Mitigation off.
        let mut metrics = empty_metrics();
        add_co_requests(&mut metrics);
        let snap = build_dashboard_snapshot(co_input(&metrics));
        assert!(snap.aggregate.co_adjusted.is_none());
        assert!(snap.requests.iter().all(|r| r.co_adjusted.is_none()));

        // Mitigation on, no events recorded.
        metrics.coordinated_omission_metrics = Some(CoordinatedOmissionMetrics::new(
            GooseCoordinatedOmissionMitigation::Average,
        ));
        let snap = build_dashboard_snapshot(co_input(&metrics));
        assert!(!snap.aggregate.co_active);
        assert!(snap.aggregate.co_adjusted.is_none());
        assert!(snap.requests.iter().all(|r| r.co_adjusted.is_none()));
    }

    #[test]
    fn builder_aggregates_request_counts_and_percentiles() {
        let mut metrics = empty_metrics();
        metrics.duration = 10;
        metrics
            .requests
            .insert("GET /a".to_string(), make_request("/a", 8, 2, &[(20, 10)]));
        metrics
            .requests
            .insert("GET /b".to_string(), make_request("/b", 5, 0, &[(40, 5)]));

        let snap = build_dashboard_snapshot(DashboardSnapshotInput {
            metrics: &metrics,
            series: SeriesWindow::empty(),
            active_users: 3,
            maximum_users: 10,
            target_users: 10,
            total_users: 10,
            phase: "increase".to_string(),
            stopping: false,
            series_window_secs: 300,
            no_status_codes: false,
            metrics_disabled: false,
            no_transaction_metrics: false,
            no_scenario_metrics: false,
            save: SnapshotSave::off("goose-runs"),
        });

        assert_eq!(snap.aggregate.total_requests, 15);
        assert_eq!(snap.aggregate.total_failures, 2);
        assert!((snap.aggregate.requests_per_second - 1.5).abs() < 0.001);
        assert!((snap.aggregate.failure_rate - (2.0 / 15.0)).abs() < 0.0001);
        assert_eq!(snap.requests.len(), 2);
        assert_eq!(snap.requests[0].name, "/a");
        assert_eq!(snap.requests[0].request_count, 10);
        assert_eq!(snap.active_users, 3);
        assert_eq!(snap.target_users, 10);
        assert!(snap.aggregate.response_time_avg_ms > 0.0);
        assert!(snap.aggregate.percentile_ms.p50 > 0);
    }

    #[test]
    fn metrics_disabled_forces_empty_tables_and_series() {
        let mut metrics = empty_metrics();
        metrics.duration = 42;
        metrics.hosts.insert("https://example.com".to_string());
        metrics
            .requests
            .insert("GET /a".to_string(), make_request("/a", 8, 2, &[(20, 10)]));
        let mut err = GooseErrorMetricAggregate::new(
            GooseMethod::Get,
            "/a".to_string(),
            "boom".to_string(),
            "",
        );
        err.occurrences = 3;
        metrics.errors.insert("err".to_string(), err);
        add_scenario_metrics(&mut metrics);

        let series = SeriesWindow {
            start_second: 10,
            rps: vec![1.0, 2.0],
            fps: vec![0.0, 0.5],
            users: vec![1, 2],
            avg_latency_ms: vec![10.0, 20.0],
        };

        let snap = build_dashboard_snapshot(DashboardSnapshotInput {
            metrics: &metrics,
            series,
            active_users: 7,
            maximum_users: 10,
            target_users: 10,
            total_users: 10,
            phase: "maintain".to_string(),
            stopping: true,
            series_window_secs: 120,
            no_status_codes: false,
            metrics_disabled: true,
            no_transaction_metrics: false,
            no_scenario_metrics: false,
            save: SnapshotSave::off("goose-runs"),
        });

        assert!(snap.flags.metrics_disabled);
        assert!(!snap.flags.requests_truncated);
        assert!(!snap.flags.errors_truncated);
        assert_eq!(snap.flags.series_seconds, 120);
        assert!(snap.requests.is_empty());
        assert!(snap.errors.is_empty());
        assert!(snap.scenarios.is_empty());
        assert!(snap.transactions.is_empty());
        assert!(snap.flags.transaction_metrics_disabled);
        assert!(snap.flags.scenario_metrics_disabled);
        assert_eq!(snap.series, SeriesWindow::empty());
        assert_eq!(snap.aggregate.total_requests, 0);
        assert_eq!(snap.aggregate.total_failures, 0);
        assert!(!snap.aggregate.co_active);
        // Runtime context is still honest.
        assert_eq!(snap.phase, "maintain");
        assert!(snap.stopping);
        assert_eq!(snap.duration_secs, 42);
        assert_eq!(snap.active_users, 7);
        assert_eq!(snap.target_users, 10);
        assert_eq!(snap.hosts, vec!["https://example.com".to_string()]);
    }

    fn make_transaction(
        scenario_index: usize,
        scenario_name: &str,
        transaction_index: usize,
        transaction_name: &str,
        times: &[(u64, bool)],
    ) -> TransactionMetricAggregate {
        let mut agg = TransactionMetricAggregate::new(
            scenario_index,
            Arc::from(scenario_name),
            transaction_index,
            TransactionName::TransactionOnly(Arc::from(transaction_name)),
        );
        for &(time, success) in times {
            agg.set_time(time, success);
        }
        agg
    }

    fn make_scenario(index: usize, name: &str, runs: &[(u64, usize)]) -> ScenarioMetricAggregate {
        let mut agg = ScenarioMetricAggregate::new(index, Arc::from(name));
        for &(time, user) in runs {
            agg.update(time, user);
        }
        agg
    }

    /// Two scenarios of two transactions each, the second transaction of the
    /// first scenario with failures.
    fn add_scenario_metrics(metrics: &mut GooseMetrics) {
        metrics.scenarios = vec![
            make_scenario(0, "Alpha", &[(100, 0), (200, 1), (300, 0), (400, 1)]),
            make_scenario(1, "Beta", &[(50, 2), (70, 2)]),
        ];
        metrics.transactions = vec![
            vec![
                make_transaction(
                    0,
                    "Alpha",
                    0,
                    "login",
                    &[(10, true), (20, true), (30, true)],
                ),
                make_transaction(
                    0,
                    "Alpha",
                    1,
                    "",
                    &[(40, true), (60, false), (80, false), (100, true)],
                ),
            ],
            vec![
                make_transaction(1, "Beta", 0, "front", &[(5, true)]),
                make_transaction(1, "Beta", 1, "about", &[]),
            ],
        ];
    }

    fn scenario_input(metrics: &GooseMetrics) -> DashboardSnapshotInput<'_> {
        DashboardSnapshotInput {
            metrics,
            series: SeriesWindow::empty(),
            active_users: 3,
            maximum_users: 3,
            target_users: 3,
            total_users: 3,
            phase: "maintain".to_string(),
            stopping: false,
            series_window_secs: SERIES_WINDOW_SECS,
            no_status_codes: false,
            metrics_disabled: false,
            no_transaction_metrics: false,
            no_scenario_metrics: false,
            save: SnapshotSave::off("goose-runs"),
        }
    }

    #[test]
    fn builder_scenario_and_transaction_rows() {
        let mut metrics = empty_metrics();
        metrics.duration = 4;
        add_scenario_metrics(&mut metrics);
        let snap = build_dashboard_snapshot(scenario_input(&metrics));

        assert!(!snap.flags.scenario_metrics_disabled);
        assert!(!snap.flags.transaction_metrics_disabled);

        let names: Vec<&str> = snap
            .scenarios
            .iter()
            .map(|s| s.scenario_name.as_str())
            .collect();
        assert_eq!(names, vec!["Alpha", "Beta"]);
        let alpha = &snap.scenarios[0];
        assert_eq!(alpha.scenario_index, 0);
        assert_eq!(alpha.users, 2);
        assert_eq!(alpha.run_count, 4);
        assert!((alpha.runs_per_second - 1.0).abs() < 1e-9);
        assert!((alpha.response_time_avg_ms - 250.0).abs() < 1e-9);
        assert_eq!(alpha.response_time_min_ms, 100);
        assert_eq!(alpha.response_time_max_ms, 400);
        assert_eq!(alpha.percentile_ms.p50, 200);
        assert_eq!(alpha.percentile_ms.p95, 400);
        assert_eq!(alpha.percentile_ms.p99, 400);
        let beta = &snap.scenarios[1];
        assert_eq!(beta.scenario_index, 1);
        assert_eq!(beta.users, 1);
        assert_eq!(beta.run_count, 2);
        assert!((beta.response_time_avg_ms - 60.0).abs() < 1e-9);

        let order: Vec<(u64, u64)> = snap
            .transactions
            .iter()
            .map(|t| (t.scenario_index, t.transaction_index))
            .collect();
        assert_eq!(order, vec![(0, 0), (0, 1), (1, 0), (1, 1)]);

        let login = &snap.transactions[0];
        assert_eq!(login.scenario_name, "Alpha");
        assert_eq!(login.transaction_name, "login");
        assert_eq!(login.run_count, 3);
        assert_eq!(login.failure_count, 0);
        assert!((login.runs_per_second - 0.75).abs() < 1e-6);
        assert_eq!(login.failures_per_second, 0.0);
        assert!((login.response_time_avg_ms - 20.0).abs() < 1e-9);
        assert_eq!(login.response_time_min_ms, 10);
        assert_eq!(login.response_time_max_ms, 30);
        assert_eq!(login.percentile_ms.p50, 20);
        assert_eq!(login.percentile_ms.p99, 30);

        let failing = &snap.transactions[1];
        assert_eq!(failing.transaction_name, "");
        assert_eq!(failing.run_count, 4);
        assert_eq!(failing.failure_count, 2);
        assert!((failing.runs_per_second - 1.0).abs() < 1e-6);
        assert!((failing.failures_per_second - 0.5).abs() < 1e-6);
        assert!((failing.response_time_avg_ms - 70.0).abs() < 1e-9);
        assert_eq!(failing.response_time_min_ms, 40);
        assert_eq!(failing.response_time_max_ms, 100);
        assert_eq!(failing.percentile_ms.p50, 60);
        assert_eq!(failing.percentile_ms.p95, 100);

        let never_run = &snap.transactions[3];
        assert_eq!(never_run.scenario_name, "Beta");
        assert_eq!(never_run.transaction_name, "about");
        assert_eq!(never_run.run_count, 0);
        assert_eq!(never_run.response_time_avg_ms, 0.0);
        assert_eq!(never_run.percentile_ms.p50, 0);
    }

    #[test]
    fn never_run_scenario_serializes_without_null() {
        let mut metrics = empty_metrics();
        metrics.scenarios = vec![make_scenario(0, "Idle", &[])];
        metrics.transactions = vec![vec![make_transaction(0, "Idle", 0, "t", &[])]];
        let snap = build_dashboard_snapshot(scenario_input(&metrics));
        let scenario = &snap.scenarios[0];
        assert_eq!(scenario.run_count, 0);
        assert_eq!(scenario.users, 0);
        assert_eq!(scenario.runs_per_second, 0.0);
        assert_eq!(scenario.response_time_avg_ms, 0.0);
        assert_eq!(scenario.percentile_ms.p50, 0);
        assert_eq!(snap.transactions[0].response_time_avg_ms, 0.0);
        // Only the scenario and transaction rows: other parts of the snapshot
        // hold nulls on purpose (`save`, and `co_adjusted` without events).
        let value = serde_json::to_value(&snap).expect("serialize snapshot");
        for rows in ["scenarios", "transactions"] {
            let json = value[rows].to_string();
            assert!(!json.contains("null"), "{}: {}", rows, json);
        }
    }

    #[test]
    fn no_transaction_metrics_empties_only_transactions() {
        let mut metrics = empty_metrics();
        metrics.duration = 4;
        add_scenario_metrics(&mut metrics);
        let mut input = scenario_input(&metrics);
        input.no_transaction_metrics = true;
        let snap = build_dashboard_snapshot(input);
        assert!(snap.transactions.is_empty());
        assert!(snap.flags.transaction_metrics_disabled);
        assert!(!snap.flags.scenario_metrics_disabled);
        assert!(!snap.flags.metrics_disabled);
        assert_eq!(snap.scenarios.len(), 2);
    }

    #[test]
    fn no_scenario_metrics_empties_only_scenarios() {
        let mut metrics = empty_metrics();
        metrics.duration = 4;
        add_scenario_metrics(&mut metrics);
        let mut input = scenario_input(&metrics);
        input.no_scenario_metrics = true;
        let snap = build_dashboard_snapshot(input);
        assert!(snap.scenarios.is_empty());
        assert!(snap.flags.scenario_metrics_disabled);
        assert!(!snap.flags.transaction_metrics_disabled);
        assert!(!snap.flags.metrics_disabled);
        assert_eq!(snap.transactions.len(), 4);
    }

    /// Committed TypeScript declarations of the snapshot structs, compiled
    /// with the dashboard client (`src/dashboard/static/tsconfig.json`).
    const DASHBOARD_TYPES_PATH: &str = "src/dashboard/static/snapshot.d.ts";

    /// Set to rewrite [`DASHBOARD_TYPES_PATH`] instead of comparing against it.
    const UPDATE_DASHBOARD_TYPES_ENV: &str = "GOOSE_UPDATE_DASHBOARD_TYPES";

    /// TypeScript declarations for every struct in the snapshot, generated by
    /// `ts-rs`. They are global `type` aliases, so every client module in
    /// `src/dashboard/static/src/` uses them without an import. Every `u64`
    /// is emitted as `number`, the type `JSON.parse` yields, rather than
    /// `ts-rs`'s default `bigint`.
    fn dashboard_types_ts() -> String {
        use std::any::TypeId;
        use ts_rs::{TypeVisitor, TS};

        /// Collects the declaration of every struct reachable from the
        /// snapshot, so a new nested struct cannot be left out of the file.
        struct Declarations {
            cfg: ts_rs::Config,
            seen: Vec<TypeId>,
            decls: Vec<String>,
        }
        impl TypeVisitor for Declarations {
            fn visit<T: TS + 'static + ?Sized>(&mut self) {
                // Primitives and std containers have no output path and no
                // declaration of their own.
                if T::output_path().is_none() || self.seen.contains(&TypeId::of::<T>()) {
                    return;
                }
                self.seen.push(TypeId::of::<T>());
                self.decls.push(T::decl(&self.cfg));
                T::visit_dependencies(self);
            }
        }

        let mut declarations = Declarations {
            cfg: ts_rs::Config::new().with_large_int("number"),
            seen: Vec::new(),
            decls: Vec::new(),
        };
        declarations.visit::<DashboardSnapshot>();
        // The derive's visiting order is not stable from one build to the
        // next; sort by type name so the file is.
        declarations.decls.sort();
        let mut out = format!(
            "// Generated from src/metrics/dashboard_snapshot.rs by the unit test\n\
             // `dashboard_types_match_rust`. Do not edit. To regenerate, run:\n\
             //   {UPDATE_DASHBOARD_TYPES_ENV}=1 cargo test --lib dashboard_types_match_rust\n\
             \n\
             /** The only `DashboardSnapshot.version` this client accepts. */\n\
             type DashboardSnapshotVersion = {SNAPSHOT_VERSION};\n"
        );
        for decl in declarations.decls {
            out.push('\n');
            // `ts-rs` leaves a space after the comma ahead of a doc comment.
            for line in decl.lines() {
                out.push_str(line.trim_end());
                out.push('\n');
            }
        }
        out
    }

    #[test]
    fn dashboard_types_match_rust() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(DASHBOARD_TYPES_PATH);
        let generated = dashboard_types_ts();
        if std::env::var_os(UPDATE_DASHBOARD_TYPES_ENV).is_some() {
            // Never let a stray variable turn the CI check into a rewrite.
            assert!(
                std::env::var_os("CI").is_none(),
                "{} must not be set in CI",
                UPDATE_DASHBOARD_TYPES_ENV
            );
            std::fs::write(&path, &generated).expect("write dashboard types");
            return;
        }
        let committed = std::fs::read_to_string(&path).expect("read dashboard types");
        assert!(
            committed == generated,
            "{} does not match the snapshot structs. Regenerate it with \
             `{}=1 cargo test --lib dashboard_types_match_rust`, then type check \
             the client (`npm run check` in src/dashboard/static).\n--- generated ---\n{}",
            DASHBOARD_TYPES_PATH,
            UPDATE_DASHBOARD_TYPES_ENV,
            generated
        );
    }

    #[test]
    fn snapshot_json_comes_from_plain_derives() {
        // snapshot.d.ts describes the JSON only while serde's derive writes it
        // from the fields as declared. ts-rs does not model attributes such as
        // `serialize_with`, and cannot see a hand-written `impl Serialize`, so
        // either could change the JSON without changing the declarations.
        // `serde(` also catches one inside `cfg_attr`.
        let source = include_str!("dashboard_snapshot.rs");
        let structs = source.split("\nmod tests {").next().expect("source");
        for pattern in ["serde(", "impl Serialize", "impl serde::Serialize"] {
            assert!(
                !structs.contains(pattern),
                "`{}` in dashboard_snapshot.rs: snapshot.d.ts may no longer \
                 match the JSON; check the generated types by hand",
                pattern
            );
        }
    }
}
