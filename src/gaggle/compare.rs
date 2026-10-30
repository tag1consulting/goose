//! Field by field comparison of the metrics a Gaggle merge produces with
//! those of one process, for the shadow merge self check and the tests.

use crate::metrics::GooseMetrics;
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
/// omission timestamps, wall clock times that two runs never share; events
/// compare in order of their other fields, as workers interleave them.
pub(crate) fn metrics_diff(a: &GooseMetrics, b: &GooseMetrics) -> Vec<String> {
    let mut out = Vec::new();
    let keys: BTreeSet<&String> = a.requests.keys().chain(b.requests.keys()).collect();
    for key in keys {
        match (a.requests.get(key), b.requests.get(key)) {
            (Some(x), Some(y)) => {
                let field = |name: &str| format!("requests[{key}].{name}");
                check(&mut out, &field("path"), &x.path, &y.path);
                check(&mut out, &field("method"), &x.method, &y.method);
                check(
                    &mut out,
                    &field("custom_method"),
                    &x.custom_method,
                    &y.custom_method,
                );
                check(&mut out, &field("raw_data"), &x.raw_data, &y.raw_data);
                check(
                    &mut out,
                    &field("coordinated_omission_data"),
                    &x.coordinated_omission_data,
                    &y.coordinated_omission_data,
                );
                check(
                    &mut out,
                    &field("status_code_counts"),
                    &x.status_code_counts,
                    &y.status_code_counts,
                );
                check(
                    &mut out,
                    &field("status_code_timings"),
                    &x.status_code_timings,
                    &y.status_code_timings,
                );
                check(
                    &mut out,
                    &field("success_count"),
                    &x.success_count,
                    &y.success_count,
                );
                check(&mut out, &field("fail_count"), &x.fail_count, &y.fail_count);
                check(
                    &mut out,
                    &field("load_test_hash"),
                    &x.load_test_hash,
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
    check(&mut out, "transactions", &a.transactions, &b.transactions);
    check(&mut out, "scenarios", &a.scenarios, &b.scenarios);
    check(&mut out, "errors", &a.errors, &b.errors);
    check(
        &mut out,
        "transaction_errors",
        &a.transaction_errors,
        &b.transaction_errors,
    );
    check(&mut out, "hosts", &a.hosts, &b.hosts);
    match (
        a.coordinated_omission_metrics.as_ref(),
        b.coordinated_omission_metrics.as_ref(),
    ) {
        (None, None) => {}
        (Some(x), Some(y)) => {
            let field = |name: &str| format!("coordinated_omission_metrics.{name}");
            check(
                &mut out,
                &field("actual_requests"),
                &x.actual_requests,
                &y.actual_requests,
            );
            check(
                &mut out,
                &field("synthetic_requests"),
                &x.synthetic_requests,
                &y.synthetic_requests,
            );
            check(
                &mut out,
                &field("synthetic_percentage"),
                &x.synthetic_percentage,
                &y.synthetic_percentage,
            );
            check(
                &mut out,
                &field("severity_histogram"),
                &x.severity_histogram,
                &y.severity_histogram,
            );
            check(
                &mut out,
                &field("mitigation_strategy"),
                &x.mitigation_strategy,
                &y.mitigation_strategy,
            );
            let events =
                |metrics: &crate::metrics::coordinated_omission::CoordinatedOmissionMetrics| {
                    let mut events: Vec<String> = metrics
                        .co_events
                        .iter()
                        .map(|event| {
                            format!(
                                "{:?} {:?} {} {} {} {:?}",
                                event.expected_cadence,
                                event.actual_duration,
                                event.synthetic_injected,
                                event.user_id,
                                event.scenario_name,
                                event.severity
                            )
                        })
                        .collect();
                    events.sort();
                    events
                };
            check(&mut out, &field("co_events"), &events(x), &events(y));
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
