//! Compact dashboard snapshot DTO and pure builder.
//!
//! Always compiled (no axum dependency). Used by `MetricsCommand::GetDashboardSnapshot`
//! and later by the HTTP dashboard server.

use super::{
    calculate_response_time_percentile, merge_times, per_second_calculations, update_max_time,
    update_min_time, GooseErrorMetricAggregate, GooseMetrics, GooseRequestMetricAggregate,
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

/// Wire format versioned so UI and server can evolve independently.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct DashboardSnapshot {
    pub version: u32,
    pub generated_at: DateTime<Utc>,
    pub goose_version: String,

    pub phase: String,
    pub duration_secs: u64,
    pub active_users: u64,
    pub maximum_users: u64,
    pub total_users: u64,
    pub hosts: Vec<String>,

    pub aggregate: AggregateMetrics,
    pub requests: Vec<RequestRow>,
    pub errors: Vec<ErrorRow>,
    pub series: SeriesWindow,

    pub flags: SnapshotFlags,
}

#[derive(Debug, Clone, Serialize)]
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
}

#[derive(Debug, Clone, Serialize)]
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
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ErrorRow {
    pub method: String,
    pub name: String,
    pub error: String,
    pub occurrences: u64,
}

/// Trailing per-second aggregate series for dashboard charts.
#[derive(Debug, Clone, Serialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize)]
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
pub(crate) struct SnapshotFlags {
    pub metrics_disabled: bool,
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
    pub total_users: usize,
    pub phase: String,
    pub series_window_secs: u32,
    pub no_status_codes: bool,
    pub metrics_disabled: bool,
}

/// Build a compact [`DashboardSnapshot`] from live metrics + series window.
pub(crate) fn build_dashboard_snapshot(input: DashboardSnapshotInput<'_>) -> DashboardSnapshot {
    // With `--no-metrics`, document an honest empty state: no tables, no series,
    // and no truncation flags. Runtime still reports phase/user counts.
    if input.metrics_disabled {
        let mut hosts: Vec<String> = input.metrics.hosts.iter().cloned().collect();
        hosts.sort();
        return DashboardSnapshot {
            version: 1,
            generated_at: Utc::now(),
            goose_version: env!("CARGO_PKG_VERSION").to_string(),
            phase: input.phase,
            duration_secs: input.metrics.duration as u64,
            active_users: input.active_users as u64,
            maximum_users: input.maximum_users as u64,
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
            },
            requests: Vec::new(),
            errors: Vec::new(),
            series: SeriesWindow::empty(),
            flags: SnapshotFlags {
                metrics_disabled: true,
                requests_truncated: false,
                errors_truncated: false,
                series_seconds: input.series_window_secs,
            },
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
        aggregate_times = merge_times(aggregate_times, request.raw_data.times.clone());
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
        .map(|(request, _)| request_row_from_aggregate(request, duration, input.no_status_codes))
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

    let co_active = input
        .metrics
        .coordinated_omission_metrics
        .as_ref()
        .map(|m| m.has_events())
        .unwrap_or(false);

    let mut hosts: Vec<String> = input.metrics.hosts.iter().cloned().collect();
    hosts.sort();

    DashboardSnapshot {
        version: 1,
        generated_at: Utc::now(),
        goose_version: env!("CARGO_PKG_VERSION").to_string(),
        phase: input.phase,
        duration_secs: duration as u64,
        active_users: input.active_users as u64,
        maximum_users: input.maximum_users as u64,
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
        },
        requests: request_rows,
        errors: error_rows,
        series: input.series,
        flags: SnapshotFlags {
            metrics_disabled: false,
            requests_truncated,
            errors_truncated,
            series_seconds: input.series_window_secs,
        },
    }
}

fn request_row_from_aggregate(
    request: &GooseRequestMetricAggregate,
    duration: usize,
    no_status_codes: bool,
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
    use crate::goose::GooseMethod;
    use crate::metrics::coordinated_omission::CoordinatedOmissionMetrics;
    use crate::metrics::GooseCoordinatedOmissionMitigation;
    use crate::metrics::GooseRequestMetricAggregate;
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
            total_users: 10,
            phase: "idle".to_string(),
            series_window_secs: SERIES_WINDOW_SECS,
            no_status_codes: false,
            metrics_disabled: false,
        });
        assert_eq!(snap.version, 1);
        assert_eq!(snap.phase, "idle");
        assert_eq!(snap.maximum_users, 10);
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
            total_users: 5,
            phase: "maintain".to_string(),
            series_window_secs: 60,
            no_status_codes: true,
            metrics_disabled: false,
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
            total_users: 0,
            phase: "maintain".to_string(),
            series_window_secs: 300,
            no_status_codes: false,
            metrics_disabled: false,
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
            total_users: 0,
            phase: "maintain".to_string(),
            series_window_secs: 300,
            no_status_codes: false,
            metrics_disabled: false,
        });
        assert!(snap.aggregate.co_active);
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
            total_users: 10,
            phase: "increase".to_string(),
            series_window_secs: 300,
            no_status_codes: false,
            metrics_disabled: false,
        });

        assert_eq!(snap.aggregate.total_requests, 15);
        assert_eq!(snap.aggregate.total_failures, 2);
        assert!((snap.aggregate.requests_per_second - 1.5).abs() < 0.001);
        assert!((snap.aggregate.failure_rate - (2.0 / 15.0)).abs() < 0.0001);
        assert_eq!(snap.requests.len(), 2);
        assert_eq!(snap.requests[0].name, "/a");
        assert_eq!(snap.requests[0].request_count, 10);
        assert_eq!(snap.active_users, 3);
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
            total_users: 10,
            phase: "maintain".to_string(),
            series_window_secs: 120,
            no_status_codes: false,
            metrics_disabled: true,
        });

        assert!(snap.flags.metrics_disabled);
        assert!(!snap.flags.requests_truncated);
        assert!(!snap.flags.errors_truncated);
        assert_eq!(snap.flags.series_seconds, 120);
        assert!(snap.requests.is_empty());
        assert!(snap.errors.is_empty());
        assert_eq!(snap.series, SeriesWindow::empty());
        assert_eq!(snap.aggregate.total_requests, 0);
        assert_eq!(snap.aggregate.total_failures, 0);
        assert!(!snap.aggregate.co_active);
        // Runtime context is still honest.
        assert_eq!(snap.phase, "maintain");
        assert_eq!(snap.duration_secs, 42);
        assert_eq!(snap.active_users, 7);
        assert_eq!(snap.hosts, vec!["https://example.com".to_string()]);
    }
}
