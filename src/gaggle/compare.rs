//! Field by field comparison of the metrics a Gaggle merge produces with
//! those of one process, for the shadow merge self check and the tests.

use crate::metrics::coordinated_omission::{CoEvent, CoordinatedOmissionMetrics};
use crate::metrics::{GooseMetrics, GooseRequestMetricAggregate};
use std::collections::BTreeSet;
use std::fmt::Debug;

/// Push `field: a != b` when they differ.
fn check<T: PartialEq + Debug>(out: &mut Vec<String>, field: &str, a: &T, b: &T) {
    if a != b {
        out.push(format!("{field}: {a:?} != {b:?}"));
    }
}

/// Every difference between the merged figures of two `GooseMetrics`, as
/// `field: a != b`: requests (timing buckets, coordinated omission data,
/// status codes and their timings, counts), transactions, scenarios (users
/// included), request and transaction errors, hosts, and coordinated
/// omission metrics. Figures no worker sends (the duration, user counts,
/// history, thresholds and hash) are left out, and so are coordinated
/// omission event timestamps, wall clock times that two runs never share;
/// events compare in order of their other fields, as workers interleave
/// them.
pub(crate) fn metrics_diff(a: &GooseMetrics, b: &GooseMetrics) -> Vec<String> {
    // Every field named, so a field added to any of these fails to compile
    // here until it is compared or left out on purpose.
    let GooseMetrics {
        // No worker sends it: the manager's own.
        hash: _,
        // As `hash`.
        history: _,
        // As `hash`.
        duration: _,
        // As `hash`.
        maximum_users: _,
        // As `hash`.
        total_users: _,
        requests,
        transactions,
        scenarios,
        errors,
        transaction_errors,
        hosts,
        coordinated_omission_metrics,
        // Judged on the merged metrics, by the tests that compare reports.
        thresholds: _,
        // As `thresholds`.
        threshold_abort: _,
        // As `thresholds`.
        rps_window: _,
        // The manager's own display flags.
        final_metrics: _,
        // As `final_metrics`.
        display_status_codes: _,
        // As `final_metrics`.
        display_metrics: _,
    } = a;
    let mut out = Vec::new();
    let keys: BTreeSet<&String> = requests.keys().chain(b.requests.keys()).collect();
    for key in keys {
        match (requests.get(key), b.requests.get(key)) {
            (Some(x), Some(y)) => {
                let GooseRequestMetricAggregate {
                    path,
                    method,
                    raw_data,
                    coordinated_omission_data,
                    status_code_counts,
                    status_code_timings,
                    success_count,
                    fail_count,
                    load_test_hash,
                    custom_method,
                } = x;
                let field = |name: &str| format!("requests[{key}].{name}");
                check(&mut out, &field("path"), path, &y.path);
                check(&mut out, &field("method"), method, &y.method);
                check(
                    &mut out,
                    &field("custom_method"),
                    custom_method,
                    &y.custom_method,
                );
                check(&mut out, &field("raw_data"), raw_data, &y.raw_data);
                check(
                    &mut out,
                    &field("coordinated_omission_data"),
                    coordinated_omission_data,
                    &y.coordinated_omission_data,
                );
                check(
                    &mut out,
                    &field("status_code_counts"),
                    status_code_counts,
                    &y.status_code_counts,
                );
                check(
                    &mut out,
                    &field("status_code_timings"),
                    status_code_timings,
                    &y.status_code_timings,
                );
                check(
                    &mut out,
                    &field("success_count"),
                    success_count,
                    &y.success_count,
                );
                check(&mut out, &field("fail_count"), fail_count, &y.fail_count);
                check(
                    &mut out,
                    &field("load_test_hash"),
                    load_test_hash,
                    &y.load_test_hash,
                );
            }
            (x, y) => out.push(format!(
                "requests[{key}]: present {} != present {}",
                x.is_some(),
                y.is_some()
            )),
        }
    }
    // Whole aggregates, compared with their own `PartialEq`, every field.
    check(&mut out, "transactions", transactions, &b.transactions);
    check(&mut out, "scenarios", scenarios, &b.scenarios);
    check(&mut out, "errors", errors, &b.errors);
    check(
        &mut out,
        "transaction_errors",
        transaction_errors,
        &b.transaction_errors,
    );
    check(&mut out, "hosts", hosts, &b.hosts);
    match (
        coordinated_omission_metrics.as_ref(),
        b.coordinated_omission_metrics.as_ref(),
    ) {
        (None, None) => {}
        (Some(x), Some(y)) => {
            let CoordinatedOmissionMetrics {
                actual_requests,
                synthetic_requests,
                synthetic_percentage,
                co_events,
                severity_histogram,
                mitigation_strategy,
                started_secs,
            } = x;
            let field = |name: &str| format!("coordinated_omission_metrics.{name}");
            check(
                &mut out,
                &field("actual_requests"),
                actual_requests,
                &y.actual_requests,
            );
            check(
                &mut out,
                &field("synthetic_requests"),
                synthetic_requests,
                &y.synthetic_requests,
            );
            check(
                &mut out,
                &field("synthetic_percentage"),
                synthetic_percentage,
                &y.synthetic_percentage,
            );
            check(
                &mut out,
                &field("severity_histogram"),
                severity_histogram,
                &y.severity_histogram,
            );
            check(
                &mut out,
                &field("started_secs"),
                started_secs,
                &y.started_secs,
            );
            check(
                &mut out,
                &field("mitigation_strategy"),
                mitigation_strategy,
                &y.mitigation_strategy,
            );
            let events = |events: &[CoEvent]| {
                let mut events: Vec<String> = events
                    .iter()
                    .map(|event| {
                        let CoEvent {
                            // Wall clock times, which two runs never share.
                            timestamp_secs: _,
                            expected_cadence,
                            actual_duration,
                            synthetic_injected,
                            user_id,
                            scenario_name,
                            severity,
                        } = event;
                        format!(
                            "{:?} {:?} {} {} {} {:?}",
                            expected_cadence,
                            actual_duration,
                            synthetic_injected,
                            user_id,
                            scenario_name,
                            severity
                        )
                    })
                    .collect();
                events.sort();
                events
            };
            check(
                &mut out,
                &field("co_events"),
                &events(co_events),
                &events(&y.co_events),
            );
        }
        (x, y) => out.push(format!(
            "coordinated_omission_metrics: kept {} != kept {}",
            x.is_some(),
            y.is_some()
        )),
    }
    out
}

/// Copy into `merged` the figures no worker sends, which the manager keeps
/// itself: the hash, history, duration, user counts, thresholds and display
/// flags of `run`.
pub(crate) fn adopt_unmerged(merged: &mut GooseMetrics, run: &GooseMetrics) {
    merged.hash = run.hash;
    merged.history = run.history.clone();
    merged.duration = run.duration;
    merged.maximum_users = run.maximum_users;
    merged.total_users = run.total_users;
    merged.thresholds = run.thresholds.clone();
    merged.threshold_abort = run.threshold_abort.clone();
    merged.rps_window = run.rps_window.clone();
    merged.final_metrics = run.final_metrics;
    merged.display_status_codes = run.display_status_codes;
    merged.display_metrics = run.display_metrics;
}
