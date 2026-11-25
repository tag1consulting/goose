//! `GET /api/v1/metrics`: the latest dashboard snapshot in the Prometheus
//! text exposition format (version 0.0.4), for Prometheus and anything that
//! scrapes it, such as Grafana Agent or VictoriaMetrics.
//!
//! Rendered from the snapshot the hub already built, so a scrape costs the
//! same however many requests the load test makes. The `goose_run_*` series
//! cover every request. The series by `method` and `name` are the snapshot's
//! request rows, the dashboard's table of at most
//! [`MAX_REQUEST_ROWS`](crate::metrics::dashboard_snapshot::MAX_REQUEST_ROWS)
//! names by count: a name outside it has no series until it is back in it,
//! and its counters then go on from where they were, so no counter ever
//! goes down within a run.

use crate::metrics::dashboard_snapshot::MicrosecondTimes;
use crate::metrics::DashboardSnapshot;
use std::fmt::Write;

/// Content type of the Prometheus text format.
pub(super) const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// `value` escaped for a label value: `\` to `\\`, `"` to `\"` and a newline
/// to `\n`.
fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            c => escaped.push(c),
        }
    }
    escaped
}

/// The `# HELP` and `# TYPE` lines of a metric.
fn header(out: &mut String, name: &str, kind: &str, help: &str) {
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} {kind}");
}

/// Seconds from microseconds.
fn seconds(us: u64) -> f64 {
    us as f64 / 1_000_000.0
}

/// The samples of one response time summary: its quantiles, when there are
/// times, then `_sum` and `_count` of the measured times. `labels` are as
/// written between braces, empty for none.
fn summary(
    out: &mut String,
    name: &str,
    labels: &str,
    times: Option<&MicrosecondTimes>,
    count: u64,
    total_us: u64,
) {
    let (open, close, comma) = if labels.is_empty() {
        ("", "", "")
    } else {
        ("{", "}", ",")
    };
    if let Some(times) = times {
        for (quantile, us) in [("0.5", times.p50), ("0.95", times.p95), ("0.99", times.p99)] {
            let _ = writeln!(
                out,
                "{name}{{{labels}{comma}quantile=\"{quantile}\"}} {}",
                seconds(us)
            );
        }
    }
    let _ = writeln!(out, "{name}_sum{open}{labels}{close} {}", seconds(total_us));
    let _ = writeln!(out, "{name}_count{open}{labels}{close} {count}");
}

/// `snapshot` in the Prometheus text format.
pub(super) fn render(snapshot: &DashboardSnapshot) -> String {
    let labels: Vec<String> = snapshot
        .requests
        .iter()
        .map(|row| {
            format!(
                "method=\"{}\",name=\"{}\"",
                escape(&row.method),
                escape(&row.name)
            )
        })
        .collect();
    let mut out = String::with_capacity(1024 + labels.len() * 512);

    header(&mut out, "goose_info", "gauge", "Goose version, always 1.");
    let _ = writeln!(
        out,
        "goose_info{{version=\"{}\"}} 1",
        escape(&snapshot.goose_version)
    );

    header(&mut out, "goose_users", "gauge", "Users currently running.");
    let _ = writeln!(out, "goose_users {}", snapshot.active_users);

    let aggregate = &snapshot.aggregate;
    header(
        &mut out,
        "goose_run_requests_total",
        "counter",
        "Requests of every name since metrics were last reset.",
    );
    let _ = writeln!(out, "goose_run_requests_total {}", aggregate.total_requests);
    header(
        &mut out,
        "goose_run_request_failures_total",
        "counter",
        "Failed requests of every name since metrics were last reset.",
    );
    let _ = writeln!(
        out,
        "goose_run_request_failures_total {}",
        aggregate.total_failures
    );
    header(
        &mut out,
        "goose_run_response_time_seconds",
        "summary",
        "Measured response times of every request since metrics were last reset.",
    );
    summary(
        &mut out,
        "goose_run_response_time_seconds",
        "",
        aggregate.response_time_us.as_ref(),
        aggregate.measured_count,
        aggregate.measured_total_us,
    );

    header(
        &mut out,
        "goose_requests_total",
        "counter",
        "Requests since metrics were last reset.",
    );
    for (row, labels) in snapshot.requests.iter().zip(&labels) {
        let _ = writeln!(
            out,
            "goose_requests_total{{{labels}}} {}",
            row.request_count
        );
    }

    header(
        &mut out,
        "goose_request_failures_total",
        "counter",
        "Failed requests since metrics were last reset.",
    );
    for (row, labels) in snapshot.requests.iter().zip(&labels) {
        let _ = writeln!(
            out,
            "goose_request_failures_total{{{labels}}} {}",
            row.failure_count
        );
    }

    header(
        &mut out,
        "goose_response_time_seconds",
        "summary",
        "Measured response times since metrics were last reset.",
    );
    for (row, labels) in snapshot.requests.iter().zip(&labels) {
        summary(
            &mut out,
            "goose_response_time_seconds",
            labels,
            row.response_time_us.as_ref(),
            row.measured_count,
            row.measured_total_us,
        );
    }

    if !snapshot.transactions.is_empty() {
        // `transaction`, the transaction's number counted from 1, keeps two
        // transactions with the same scenario and name apart.
        let labels: Vec<String> = snapshot
            .transactions
            .iter()
            .map(|row| {
                format!(
                    "scenario=\"{}\",name=\"{}\",transaction=\"{}.{}\"",
                    escape(&row.scenario_name),
                    escape(&row.transaction_name),
                    row.scenario_index.saturating_add(1),
                    row.transaction_index.saturating_add(1)
                )
            })
            .collect();
        header(
            &mut out,
            "goose_transactions_total",
            "counter",
            "Transactions run since metrics were last reset.",
        );
        for (row, labels) in snapshot.transactions.iter().zip(&labels) {
            let _ = writeln!(
                out,
                "goose_transactions_total{{{labels}}} {}",
                row.run_count
            );
        }
        header(
            &mut out,
            "goose_transaction_failures_total",
            "counter",
            "Failed transactions since metrics were last reset.",
        );
        for (row, labels) in snapshot.transactions.iter().zip(&labels) {
            let _ = writeln!(
                out,
                "goose_transaction_failures_total{{{labels}}} {}",
                row.failure_count
            );
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::dashboard_snapshot::{
        AggregateMetrics, Percentiles, RequestRow, SeriesWindow, SnapshotFlags, SnapshotReport,
        SnapshotSettings, TransactionRow,
    };

    fn times() -> MicrosecondTimes {
        MicrosecondTimes {
            min: 1_000,
            max: 90_000,
            p50: 2_000,
            p95: 15_000,
            p99: 80_000,
        }
    }

    /// A snapshot holding `requests`, its aggregate their totals.
    fn snapshot(requests: Vec<RequestRow>, truncated: bool) -> DashboardSnapshot {
        let sum = |figure: fn(&RequestRow) -> u64| requests.iter().map(figure).sum::<u64>();
        DashboardSnapshot {
            version: 1,
            generated_at: jiff::Timestamp::now(),
            goose_version: "0.20.0-dev".into(),
            phase: "maintain".into(),
            stopping: false,
            duration_secs: 10,
            active_users: 0,
            maximum_users: 0,
            target_users: 0,
            total_users: 0,
            hosts: vec![],
            aggregate: AggregateMetrics {
                total_requests: sum(|row| row.request_count),
                total_failures: sum(|row| row.failure_count),
                requests_per_second: 0.0,
                failures_per_second: 0.0,
                failure_rate: 0.0,
                response_time_avg_ms: 0.0,
                response_time_min_ms: 0,
                response_time_max_ms: 0,
                percentile_ms: Percentiles {
                    p50: 0,
                    p95: 0,
                    p99: 0,
                },
                response_time_us: (!requests.is_empty()).then(times),
                co_active: false,
                co_adjusted: None,
                measured_count: sum(|row| row.measured_count),
                measured_total_us: sum(|row| row.measured_total_us),
            },
            requests,
            errors: vec![],
            transaction_errors: vec![],
            scenarios: vec![],
            transactions: vec![],
            series: SeriesWindow::empty(),
            flags: SnapshotFlags {
                metrics_disabled: false,
                transaction_metrics_disabled: false,
                scenario_metrics_disabled: false,
                requests_truncated: truncated,
                errors_truncated: false,
                transaction_errors_truncated: false,
                series_seconds: 300,
                percentiles_over_time: false,
            },
            report: SnapshotReport::default(),
            settings: SnapshotSettings::idle(),
            build_time: std::time::Duration::ZERO,
            gaggle: None,
        }
    }

    /// A request counted `count` times, `failures` of them failed, with
    /// `count` measured times totalling `total_us`.
    fn request(method: &str, name: &str, count: u64, failures: u64, total_us: u64) -> RequestRow {
        RequestRow {
            method: method.into(),
            name: name.into(),
            request_count: count,
            failure_count: failures,
            requests_per_second: 0.0,
            failures_per_second: 0.0,
            response_time_avg_ms: 0.0,
            response_time_min_ms: 0,
            response_time_max_ms: 0,
            percentile_ms: Percentiles {
                p50: 0,
                p95: 0,
                p99: 0,
            },
            response_time_us: Some(times()),
            status_codes: vec![],
            co_adjusted: None,
            has_series: false,
            measured_count: count,
            measured_total_us: total_us,
        }
    }

    fn transaction(
        scenario: &str,
        index: u64,
        name: &str,
        runs: u64,
        fails: u64,
    ) -> TransactionRow {
        TransactionRow {
            scenario_index: 0,
            scenario_name: scenario.into(),
            transaction_index: index,
            transaction_name: name.into(),
            run_count: runs,
            failure_count: fails,
            runs_per_second: 0.0,
            failures_per_second: 0.0,
            response_time_avg_ms: 0.0,
            response_time_min_ms: 0,
            response_time_max_ms: 0,
            percentile_ms: Percentiles {
                p50: 0,
                p95: 0,
                p99: 0,
            },
            response_time_us: MicrosecondTimes::default(),
        }
    }

    #[test]
    fn renders_two_requests() {
        let mut snapshot = snapshot(
            vec![
                request("GET", "/", 40, 2, 500_000),
                request("POST", "login", 10, 0, 1_000_000),
            ],
            false,
        );
        snapshot.active_users = 5;
        snapshot.transactions = vec![
            transaction("Website", 0, "login", 10, 0),
            transaction("Website", 1, "", 40, 2),
        ];
        let expected = "\
# HELP goose_info Goose version, always 1.
# TYPE goose_info gauge
goose_info{version=\"0.20.0-dev\"} 1
# HELP goose_users Users currently running.
# TYPE goose_users gauge
goose_users 5
# HELP goose_run_requests_total Requests of every name since metrics were last reset.
# TYPE goose_run_requests_total counter
goose_run_requests_total 50
# HELP goose_run_request_failures_total Failed requests of every name since metrics were last reset.
# TYPE goose_run_request_failures_total counter
goose_run_request_failures_total 2
# HELP goose_run_response_time_seconds Measured response times of every request since metrics were last reset.
# TYPE goose_run_response_time_seconds summary
goose_run_response_time_seconds{quantile=\"0.5\"} 0.002
goose_run_response_time_seconds{quantile=\"0.95\"} 0.015
goose_run_response_time_seconds{quantile=\"0.99\"} 0.08
goose_run_response_time_seconds_sum 1.5
goose_run_response_time_seconds_count 50
# HELP goose_requests_total Requests since metrics were last reset.
# TYPE goose_requests_total counter
goose_requests_total{method=\"GET\",name=\"/\"} 40
goose_requests_total{method=\"POST\",name=\"login\"} 10
# HELP goose_request_failures_total Failed requests since metrics were last reset.
# TYPE goose_request_failures_total counter
goose_request_failures_total{method=\"GET\",name=\"/\"} 2
goose_request_failures_total{method=\"POST\",name=\"login\"} 0
# HELP goose_response_time_seconds Measured response times since metrics were last reset.
# TYPE goose_response_time_seconds summary
goose_response_time_seconds{method=\"GET\",name=\"/\",quantile=\"0.5\"} 0.002
goose_response_time_seconds{method=\"GET\",name=\"/\",quantile=\"0.95\"} 0.015
goose_response_time_seconds{method=\"GET\",name=\"/\",quantile=\"0.99\"} 0.08
goose_response_time_seconds_sum{method=\"GET\",name=\"/\"} 0.5
goose_response_time_seconds_count{method=\"GET\",name=\"/\"} 40
goose_response_time_seconds{method=\"POST\",name=\"login\",quantile=\"0.5\"} 0.002
goose_response_time_seconds{method=\"POST\",name=\"login\",quantile=\"0.95\"} 0.015
goose_response_time_seconds{method=\"POST\",name=\"login\",quantile=\"0.99\"} 0.08
goose_response_time_seconds_sum{method=\"POST\",name=\"login\"} 1
goose_response_time_seconds_count{method=\"POST\",name=\"login\"} 10
# HELP goose_transactions_total Transactions run since metrics were last reset.
# TYPE goose_transactions_total counter
goose_transactions_total{scenario=\"Website\",name=\"login\",transaction=\"1.1\"} 10
goose_transactions_total{scenario=\"Website\",name=\"\",transaction=\"1.2\"} 40
# HELP goose_transaction_failures_total Failed transactions since metrics were last reset.
# TYPE goose_transaction_failures_total counter
goose_transaction_failures_total{scenario=\"Website\",name=\"login\",transaction=\"1.1\"} 0
goose_transaction_failures_total{scenario=\"Website\",name=\"\",transaction=\"1.2\"} 2
";
        assert_eq!(render(&snapshot), expected);
    }

    #[test]
    fn escapes_label_values() {
        let snapshot = snapshot(vec![request("GET", "a\\b\"c\nd", 1, 0, 1_000)], false);
        let text = render(&snapshot);
        assert!(
            text.contains("goose_requests_total{method=\"GET\",name=\"a\\\\b\\\"c\\nd\"} 1\n"),
            "{}",
            text
        );
        // Every sample is one line: the newline never reaches the output.
        for line in text.lines() {
            assert!(
                line.starts_with('#') || line.starts_with("goose_"),
                "{}",
                line
            );
        }
    }

    /// Past the table's cap, the names it leaves out have no series of
    /// their own and are not summed by subtraction, which would make a
    /// counter go down when a name moves in or out of the table; the
    /// `goose_run_*` series count them.
    #[test]
    fn names_past_the_cap_are_counted_in_the_run_totals() {
        let listed = vec![
            request("GET", "/a", 50, 5, 500_000),
            request("GET", "/b", 30, 1, 600_000),
        ];
        let mut snapshot = snapshot(listed, true);
        // Two more names the snapshot does not list: 20 requests, 4 failed,
        // 2 s of response time between them.
        snapshot.aggregate.total_requests += 20;
        snapshot.aggregate.total_failures += 4;
        snapshot.aggregate.measured_count += 20;
        snapshot.aggregate.measured_total_us += 2_000_000;
        let text = render(&snapshot);
        assert!(
            text.contains("\ngoose_run_requests_total 100\n"),
            "{}",
            text
        );
        assert!(
            text.contains("\ngoose_run_request_failures_total 10\n"),
            "{}",
            text
        );
        assert!(
            text.contains("\ngoose_run_response_time_seconds_sum 3.1\n"),
            "{}",
            text
        );
        assert!(
            text.contains("\ngoose_run_response_time_seconds_count 100\n"),
            "{}",
            text
        );
        assert!(!text.contains("other"), "{}", text);
        assert_eq!(text.matches("\ngoose_requests_total{").count(), 2);
    }

    /// Coordinated omission mitigation counts its synthetic requests, as
    /// the dashboard's tables do, but the summary holds only the times that
    /// were measured.
    #[test]
    fn summary_holds_only_measured_times() {
        let mut row = request("GET", "/", 15, 0, 50_000);
        row.measured_count = 10;
        let snapshot = snapshot(vec![row], false);
        let text = render(&snapshot);
        assert!(
            text.contains("goose_requests_total{method=\"GET\",name=\"/\"} 15\n"),
            "{}",
            text
        );
        assert!(
            text.contains("goose_response_time_seconds_count{method=\"GET\",name=\"/\"} 10\n"),
            "{}",
            text
        );
        assert!(
            text.contains("goose_response_time_seconds_sum{method=\"GET\",name=\"/\"} 0.05\n"),
            "{}",
            text
        );
        assert!(
            text.contains("\ngoose_run_response_time_seconds_count 10\n"),
            "{}",
            text
        );
    }

    #[test]
    fn empty_run_is_valid() {
        let text = render(&snapshot(vec![], false));
        assert!(text.contains("\ngoose_users 0\n"), "{}", text);
        assert!(text.contains("goose_info{version=\"0.20.0-dev\"} 1\n"));
        assert!(text.contains("\ngoose_run_requests_total 0\n"));
        assert!(text.contains("\ngoose_run_response_time_seconds_count 0\n"));
        assert!(!text.contains("quantile"), "no quantiles without times");
        assert!(!text.contains("goose_transactions_total"));
        for line in text.lines() {
            let mut parts = line.split(' ');
            match parts.next() {
                Some("#") => {
                    assert!(
                        matches!(parts.next(), Some("HELP") | Some("TYPE")),
                        "{}",
                        line
                    )
                }
                _ => assert!(
                    line.rsplit(' ')
                        .next()
                        .and_then(|v| v.parse::<f64>().ok())
                        .is_some(),
                    "{}",
                    line
                ),
            }
        }
        assert!(text.ends_with('\n'));
    }
}
