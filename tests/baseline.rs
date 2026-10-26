//! A JSON report loads as a `--baseline-file`, whatever status codes its run
//! received.

use httpmock::{Method::GET, MockServer};
use serial_test::serial;
use std::path::PathBuf;

mod common;

use goose::prelude::*;

const OK_PATH: &str = "/ok";
const MISSING_PATH: &str = "/missing";

/// Two paths reported under one name, so the request `GET page` receives
/// both a 200 and a 404 and gets per status code breakdown rows.
pub async fn get_page(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get_named(OK_PATH, "page").await?;
    let _goose = user.get_named(MISSING_PATH, "page").await;
    Ok(())
}

fn setup_mock_server_endpoints(server: &MockServer) {
    server.mock(|when, then| {
        when.method(GET).path(OK_PATH);
        then.status(200).body("ok");
    });
    server.mock(|when, then| {
        when.method(GET).path(MISSING_PATH);
        then.status(404);
    });
}

/// A new empty folder for one test.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "goose-baseline-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run one load test with `args` against `server`.
async fn run(server: &MockServer, args: Vec<&str>) -> Result<GooseMetrics, GooseError> {
    let configuration = common::build_configuration(server, args);
    common::build_load_test(
        configuration,
        vec![scenario!("LoadTest").register_transaction(transaction!(get_page))],
        None,
        None,
    )
    .execute()
    .await
}

/// The JSON report of a run that received more than one status code holds no
/// breakdown rows, loads as a baseline, and the next run's report shows
/// deltas against it; the HTML report keeps its breakdown rows.
#[tokio::test]
#[serial]
async fn a_report_with_two_status_codes_loads_as_a_baseline() {
    let server = MockServer::start();
    setup_mock_server_endpoints(&server);
    let dir = temp_dir("status-codes");
    let json = dir.join("report.json").to_string_lossy().into_owned();
    let html = dir.join("report.html").to_string_lossy().into_owned();
    run(
        &server,
        vec!["--report-file", &json, "--report-file", &html],
    )
    .await
    .unwrap();

    let md = dir.join("against.md").to_string_lossy().into_owned();
    run(
        &server,
        vec!["--baseline-file", &json, "--report-file", &md],
    )
    .await
    .expect("the report loads as a baseline");
    let md = std::fs::read_to_string(&md).unwrap();
    assert!(md.contains("(+") || md.contains("(-"), "deltas: {}", md);

    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    let names: Vec<&str> = report["raw_request_metrics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["page", "Aggregated"]);
    let codes = &report["raw_metrics"]["requests"]["GET page"]["status_code_counts"];
    assert!(
        codes.get("200").is_some() && codes.get("404").is_some(),
        "{}",
        codes
    );
    let html_text = std::fs::read_to_string(&html).unwrap();
    assert!(html_text.contains("200 (") && html_text.contains("404 ("));

    // A report rendered from the JSON report rebuilds the breakdown rows.
    let rendered = dir.join("rendered.md").to_string_lossy().into_owned();
    let configuration = goose::config::GooseConfiguration::parse_args_default(&[
        "--render-report",
        &json,
        "--report-file",
        &rendered,
    ])
    .unwrap();
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("LoadTest").register_transaction(transaction!(get_page)))
        .execute()
        .await
        .unwrap();
    let rendered = std::fs::read_to_string(&rendered).unwrap();
    assert!(
        rendered.contains("200 (") && rendered.contains("404 ("),
        "{}",
        rendered
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// A JSON report written by Goose 0.18.1 still loads as a baseline.
#[tokio::test]
#[serial]
async fn a_0_18_1_report_still_loads_as_a_baseline() {
    let server = MockServer::start();
    setup_mock_server_endpoints(&server);
    let dir = temp_dir("0.18.1");
    let fixture = format!(
        "{}/tests/fixtures/report-0.18.1.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let md = dir.join("against.md").to_string_lossy().into_owned();
    run(
        &server,
        vec!["--baseline-file", &fixture, "--report-file", &md],
    )
    .await
    .expect("the 0.18.1 report loads as a baseline");
    assert!(std::fs::read_to_string(&md).unwrap().contains("Aggregated"));
    std::fs::remove_dir_all(&dir).ok();
}
