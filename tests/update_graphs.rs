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
    common::cleanup_files(vec![report]);
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
    common::cleanup_files(vec![report]);

    let ok_metric = metrics.requests.get("GET /ok").unwrap();
    let error_metric = metrics.requests.get("GET /error").unwrap();
    // Every request sent was recorded, as a failure or a success.
    assert_eq!(ok_metric.fail_count, ok.calls());
    assert_eq!(ok_metric.success_count, 0);
    assert_eq!(error_metric.success_count, error.calls());
    assert_eq!(error_metric.fail_count, 0);
    assert!(ok.calls() > 0 && error.calls() > 0);

    assert_eq!(graph_sum(&html, "graph-rps", "GET /ok"), ok.calls() as u64);
    assert_eq!(graph_sum(&html, "graph-eps", "GET /ok"), ok.calls() as u64);
    assert_eq!(
        graph_sum(&html, "graph-rps", "GET /error"),
        error.calls() as u64
    );
    assert_eq!(graph_sum(&html, "graph-eps", "GET /error"), 0);
}
