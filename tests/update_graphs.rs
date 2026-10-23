// Validate that a request marked with set_failure() or set_success() counts
// once in the HTML report's requests and errors per second graphs.

use httpmock::{Method::GET, MockServer};

mod common;

use goose::prelude::*;

const OK_PATH: &str = "/ok";
const ERROR_PATH: &str = "/error";

// A 200 that set_failure() marks a failure.
async fn get_ok_then_set_failure(user: &mut GooseUser) -> TransactionResult {
    let mut goose = user.get(OK_PATH).await?;
    user.set_failure("intentional", &mut goose.request, None, None)
}

// A 404 that set_success() marks a success.
async fn get_error_then_set_success(user: &mut GooseUser) -> TransactionResult {
    let mut goose = user.get(ERROR_PATH).await?;
    user.set_success(&mut goose.request)
}

/// The run's final second when it ended partway through it, which the HTML
/// report's per second count graphs leave out, from the JSON report's
/// history: its first step to its last.
fn partial_final_second(json: &serde_json::Value) -> Option<usize> {
    let history = json["raw_metrics"]["history"].as_array().unwrap();
    let at = |step: &serde_json::Value| {
        chrono::DateTime::parse_from_rfc3339(step["timestamp"].as_str().unwrap()).unwrap()
    };
    let ran = (at(history.last().unwrap()) - at(&history[0]))
        .num_microseconds()
        .unwrap();
    (ran % 1_000_000 != 0).then_some((ran / 1_000_000) as usize)
}

/// The JSON report's count of `label` in graph `graph`, every second summed,
/// and in second `second` alone.
fn json_counts(
    json: &serde_json::Value,
    graph: &str,
    label: &str,
    second: Option<usize>,
) -> (u64, u64) {
    let series = &json["graphs"][graph][label];
    let origin = series["origin"].as_u64().unwrap() as usize;
    let data: Vec<u64> = serde_json::from_value(series["data"].clone()).unwrap();
    let at = second
        .and_then(|second| second.checked_sub(origin))
        .and_then(|index| data.get(index).copied())
        .unwrap_or(0);
    (data.iter().sum(), at)
}

/// The values of `label`'s line in the HTML report's graph `html_id`, summed.
/// Reads the markup `Graph::get_markup` writes: a line's `name: 'label',`,
/// then its `data: [[time, value], ...],` on one line.
fn graph_sum(html: &str, html_id: &str, label: &str) -> u64 {
    let graph = html
        .split(&format!(r#"<div id="{html_id}""#))
        .nth(1)
        .expect("graph in the report");
    let graph = &graph[..graph.find("</script>").unwrap()];
    let line = graph
        .split(&format!("name: '{label}',"))
        .nth(1)
        .expect("line in the graph");
    let data = &line[line.find("data: ").unwrap() + "data: ".len()..];
    let data = data[..data.find('\n').unwrap()].trim_end_matches(',');
    let points: Vec<(String, u64)> = serde_json::from_str(data).unwrap();
    points.iter().map(|(_, value)| value).sum()
}

#[tokio::test]
async fn updated_requests_count_once_in_report_graphs() {
    let server = MockServer::start();
    let ok = server.mock(|when, then| {
        when.method(GET).path(OK_PATH);
        then.status(200);
    });
    let error = server.mock(|when, then| {
        when.method(GET).path(ERROR_PATH);
        then.status(404);
    });

    let report = "update-graphs.html";
    let json_report = "update-graphs.json";
    common::cleanup_files(vec![report, json_report]);
    let configuration = common::build_configuration(
        &server,
        vec![
            "--users",
            "2",
            "--increase-rate",
            "2",
            "--run-time",
            "2",
            "--no-reset-metrics",
            "--report-file",
            report,
            "--report-file",
            json_report,
            "--report-graph-data",
        ],
    );
    let metrics = common::run_load_test(
        common::build_load_test(
            configuration,
            vec![scenario!("Updates")
                .register_transaction(transaction!(get_ok_then_set_failure))
                .register_transaction(transaction!(get_error_then_set_success))],
            None,
            None,
        ),
        None,
    )
    .await;
    let html = std::fs::read_to_string(report).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(json_report).unwrap()).unwrap();
    common::cleanup_files(vec![report, json_report]);

    let ok_metric = metrics.requests.get("GET /ok").unwrap();
    let error_metric = metrics.requests.get("GET /error").unwrap();
    // Every request sent was recorded, as a failure or a success.
    assert_eq!(ok_metric.fail_count, ok.calls());
    assert_eq!(ok_metric.success_count, 0);
    assert_eq!(error_metric.success_count, error.calls());
    assert_eq!(error_metric.fail_count, 0);
    assert!(ok.calls() > 0 && error.calls() > 0);

    // The per second data counts each request once, and the HTML graphs
    // draw all of it but the run's final, partial second.
    let partial = partial_final_second(&json);
    for (graph, html_id, label, sent) in [
        ("requests_per_second", "graph-rps", "GET /ok", ok.calls()),
        ("errors_per_second", "graph-eps", "GET /ok", ok.calls()),
        (
            "requests_per_second",
            "graph-rps",
            "GET /error",
            error.calls(),
        ),
    ] {
        let (all, final_second) = json_counts(&json, graph, label, partial);
        assert_eq!(all, sent as u64, "{graph} {label}");
        assert_eq!(
            graph_sum(&html, html_id, label),
            all - final_second,
            "{html_id} {label}"
        );
    }
    assert_eq!(graph_sum(&html, "graph-eps", "GET /error"), 0);
}
