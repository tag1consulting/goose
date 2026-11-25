//! `GET /api/v1/metrics`: the latest dashboard snapshot in the Prometheus
//! text exposition format (version 0.0.4), for Prometheus and anything that
//! scrapes it, such as Grafana Agent or VictoriaMetrics.
//!
//! Rendered from the snapshot the hub already built, so a scrape costs the
//! same however many requests the load test makes. Requests are the
//! snapshot's request rows, the dashboard's table of at most
//! [`MAX_REQUEST_ROWS`](crate::metrics::dashboard_snapshot::MAX_REQUEST_ROWS)
//! names by count; past it, the rest are folded into one series labelled
//! `name="other"` with no `method` label.

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

/// One request series: its labels, as written between braces, and figures.
struct RequestSeries {
    labels: String,
    requests: u64,
    failures: u64,
    sum_seconds: f64,
    /// p50, p95 and p99 in microseconds; `None` for the folded series and a
    /// request without times.
    quantiles: Option<[u64; 3]>,
}

/// `snapshot` in the Prometheus text format.
pub(super) fn render(snapshot: &DashboardSnapshot) -> String {
    let mut requests: Vec<RequestSeries> = snapshot
        .requests
        .iter()
        .map(|row| RequestSeries {
            labels: format!(
                "method=\"{}\",name=\"{}\"",
                escape(&row.method),
                escape(&row.name)
            ),
            requests: row.request_count,
            failures: row.failure_count,
            sum_seconds: row.response_time_avg_ms * row.request_count as f64 / 1000.0,
            quantiles: row
                .response_time_us
                .as_ref()
                .map(|us| [us.p50, us.p95, us.p99]),
        })
        .collect();
    if snapshot.flags.requests_truncated {
        let aggregate = &snapshot.aggregate;
        let listed = |figure: fn(&RequestSeries) -> u64| {
            requests
                .iter()
                .fold(0u64, |total, row| total.saturating_add(figure(row)))
        };
        let listed_sum: f64 = requests.iter().map(|row| row.sum_seconds).sum();
        let total_sum = aggregate.response_time_avg_ms * aggregate.total_requests as f64 / 1000.0;
        let other = RequestSeries {
            labels: "name=\"other\"".to_string(),
            requests: aggregate
                .total_requests
                .saturating_sub(listed(|row| row.requests)),
            failures: aggregate
                .total_failures
                .saturating_sub(listed(|row| row.failures)),
            sum_seconds: (total_sum - listed_sum).max(0.0),
            quantiles: None,
        };
        requests.push(other);
    }

    let mut out = String::with_capacity(512 + requests.len() * 512);

    header(&mut out, "goose_info", "gauge", "Goose version, always 1.");
    let _ = writeln!(
        out,
        "goose_info{{version=\"{}\"}} 1",
        escape(&snapshot.goose_version)
    );

    header(&mut out, "goose_users", "gauge", "Users currently running.");
    let _ = writeln!(out, "goose_users {}", snapshot.active_users);

    header(
        &mut out,
        "goose_requests_total",
        "counter",
        "Requests made since the run started.",
    );
    for row in &requests {
        let _ = writeln!(
            out,
            "goose_requests_total{{{}}} {}",
            row.labels, row.requests
        );
    }

    header(
        &mut out,
        "goose_request_failures_total",
        "counter",
        "Failed requests since the run started.",
    );
    for row in &requests {
        let _ = writeln!(
            out,
            "goose_request_failures_total{{{}}} {}",
            row.labels, row.failures
        );
    }

    header(
        &mut out,
        "goose_response_time_seconds",
        "summary",
        "Response times since the run started.",
    );
    for row in &requests {
        if let Some(quantiles) = row.quantiles {
            for (quantile, us) in ["0.5", "0.95", "0.99"].iter().zip(quantiles) {
                let _ = writeln!(
                    out,
                    "goose_response_time_seconds{{{},quantile=\"{quantile}\"}} {}",
                    row.labels,
                    seconds(us)
                );
            }
        }
        let _ = writeln!(
            out,
            "goose_response_time_seconds_sum{{{}}} {}",
            row.labels, row.sum_seconds
        );
        let _ = writeln!(
            out,
            "goose_response_time_seconds_count{{{}}} {}",
            row.labels, row.requests
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
            "Transactions run since the run started.",
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
            "Failed transactions since the run started.",
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
        AggregateMetrics, MicrosecondTimes, Percentiles, RequestRow, SeriesWindow, SnapshotFlags,
        SnapshotReport, SnapshotSettings, TransactionRow,
    };

    fn snapshot(requests: Vec<RequestRow>, truncated: bool) -> DashboardSnapshot {
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
                total_requests: 0,
                total_failures: 0,
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
                response_time_us: None,
                co_active: false,
                co_adjusted: None,
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

    fn request(method: &str, name: &str, count: u64, failures: u64, avg_ms: f64) -> RequestRow {
        RequestRow {
            method: method.into(),
            name: name.into(),
            request_count: count,
            failure_count: failures,
            requests_per_second: 0.0,
            failures_per_second: 0.0,
            response_time_avg_ms: avg_ms,
            response_time_min_ms: 0,
            response_time_max_ms: 0,
            percentile_ms: Percentiles {
                p50: 0,
                p95: 0,
                p99: 0,
            },
            response_time_us: Some(MicrosecondTimes {
                min: 1_000,
                max: 90_000,
                p50: 2_000,
                p95: 15_000,
                p99: 80_000,
            }),
            status_codes: vec![],
            co_adjusted: None,
            has_series: false,
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
                request("GET", "/", 40, 2, 12.5),
                request("POST", "login", 10, 0, 100.0),
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
# HELP goose_requests_total Requests made since the run started.
# TYPE goose_requests_total counter
goose_requests_total{method=\"GET\",name=\"/\"} 40
goose_requests_total{method=\"POST\",name=\"login\"} 10
# HELP goose_request_failures_total Failed requests since the run started.
# TYPE goose_request_failures_total counter
goose_request_failures_total{method=\"GET\",name=\"/\"} 2
goose_request_failures_total{method=\"POST\",name=\"login\"} 0
# HELP goose_response_time_seconds Response times since the run started.
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
# HELP goose_transactions_total Transactions run since the run started.
# TYPE goose_transactions_total counter
goose_transactions_total{scenario=\"Website\",name=\"login\",transaction=\"1.1\"} 10
goose_transactions_total{scenario=\"Website\",name=\"\",transaction=\"1.2\"} 40
# HELP goose_transaction_failures_total Failed transactions since the run started.
# TYPE goose_transaction_failures_total counter
goose_transaction_failures_total{scenario=\"Website\",name=\"login\",transaction=\"1.1\"} 0
goose_transaction_failures_total{scenario=\"Website\",name=\"\",transaction=\"1.2\"} 2
";
        assert_eq!(render(&snapshot), expected);
    }

    #[test]
    fn escapes_label_values() {
        let snapshot = snapshot(vec![request("GET", "a\\b\"c\nd", 1, 0, 1.0)], false);
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

    #[test]
    fn folds_names_past_the_cap_into_other() {
        let mut snapshot = snapshot(
            vec![
                request("GET", "/a", 50, 5, 10.0),
                request("GET", "/b", 30, 1, 20.0),
            ],
            true,
        );
        // Two more names the snapshot does not list: 20 requests, 4 failed,
        // 2 s of response time between them.
        snapshot.aggregate.total_requests = 100;
        snapshot.aggregate.total_failures = 10;
        snapshot.aggregate.response_time_avg_ms = 31.0;
        let text = render(&snapshot);
        assert!(
            text.contains("goose_requests_total{name=\"other\"} 20\n"),
            "{}",
            text
        );
        assert!(
            text.contains("goose_request_failures_total{name=\"other\"} 4\n"),
            "{}",
            text
        );
        assert!(
            text.contains("goose_response_time_seconds_sum{name=\"other\"} 2\n"),
            "{}",
            text
        );
        assert!(
            text.contains("goose_response_time_seconds_count{name=\"other\"} 20\n"),
            "{}",
            text
        );
        assert!(
            !text.contains("{name=\"other\",quantile="),
            "the folded series has no quantiles: {}",
            text
        );
        assert_eq!(text.matches("goose_requests_total{").count(), 3);
    }

    #[test]
    fn empty_run_is_valid() {
        let text = render(&snapshot(vec![], false));
        assert!(text.contains("\ngoose_users 0\n"), "{}", text);
        assert!(text.contains("goose_info{version=\"0.20.0-dev\"} 1\n"));
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
