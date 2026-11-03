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

/// What `--baseline-file` and `--render-report` say of a JSON report from
/// Goose 0.18 or earlier.
const GOOSE_0_18_REPORT: &str = "this JSON report is from Goose 0.18 or earlier, \
which this Goose cannot read; convert it to a new JSON report with Goose 0.19.1, \
the last release that reads it: --render-report <old.json> --report-file <new.json>";

/// The path of the report `fixture` in tests/fixtures.
fn fixture(name: &str) -> String {
    format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// Write the reports `report_files` name from the JSON report `report`
/// with `--render-report`.
async fn render(report: &str, report_files: &[&str]) -> Result<GooseMetrics, GooseError> {
    let mut args = vec!["--render-report", report];
    for file in report_files {
        args.extend(["--report-file", *file]);
    }
    let configuration = goose::config::GooseConfiguration::parse_args_default(&args).unwrap();
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("LoadTest").register_transaction(transaction!(get_page)))
        .execute()
        .await
}

/// `--baseline-file` refuses a JSON report written by Goose 0.18.1, with the
/// command that converts it.
#[tokio::test]
#[serial]
async fn a_0_18_1_report_is_refused_as_a_baseline() {
    let server = MockServer::start();
    setup_mock_server_endpoints(&server);
    let dir = temp_dir("0.18.1");
    let md = dir.join("against.md").to_string_lossy().into_owned();
    let error = run(
        &server,
        vec![
            "--baseline-file",
            &fixture("report-0.18.1.json"),
            "--report-file",
            &md,
        ],
    )
    .await
    .expect_err("the 0.18.1 report is refused as a baseline");
    match &error {
        GooseError::InvalidOption { option, detail, .. } => {
            assert_eq!(option, "--baseline-file");
            assert_eq!(detail, GOOSE_0_18_REPORT);
        }
        other => panic!("{:?}", other),
    }
    // What a `main` returning the error prints after `Error: `.
    let printed = format!("{:?}", error);
    assert!(printed.contains(GOOSE_0_18_REPORT), "{}", printed);
    assert!(!std::path::Path::new(&md).exists());
    std::fs::remove_dir_all(&dir).ok();
}

/// `--render-report` refuses a JSON report written by Goose 0.18.1, with the
/// same error as `--baseline-file`, and writes nothing.
#[tokio::test]
#[serial]
async fn render_report_refuses_a_0_18_1_report() {
    let dir = temp_dir("convert");
    let converted = dir.join("converted.json").to_string_lossy().into_owned();
    let error = render(&fixture("report-0.18.1.json"), &[&converted])
        .await
        .expect_err("the 0.18.1 report is refused");
    match &error {
        GooseError::InvalidOption { option, detail, .. } => {
            assert_eq!(option, "--render-report");
            assert_eq!(detail, GOOSE_0_18_REPORT);
        }
        other => panic!("{:?}", other),
    }
    // What a `main` returning the error prints after `Error: `.
    let printed = format!("{:?}", error);
    assert!(printed.contains(GOOSE_0_18_REPORT), "{}", printed);
    assert!(!std::path::Path::new(&converted).exists());
    std::fs::remove_dir_all(&dir).ok();
}

/// The time fields of each table's rows.
const TIMES: &[&str] = &[
    "response_time_average",
    "response_time_minimum",
    "response_time_maximum",
];
const PERCENTILES: &[&str] = &[
    "percentile_50",
    "percentile_60",
    "percentile_70",
    "percentile_80",
    "percentile_90",
    "percentile_95",
    "percentile_99",
    "percentile_100",
];

/// Every row of the JSON report `report` writes each time once, as an
/// integer number of microseconds under its `_us` name, and never under its
/// old name. A scenario's heading row in the transactions table has no
/// average, so writes `null`.
fn assert_microsecond_rows_only(report: &serde_json::Value) {
    for (table, fields) in [
        ("raw_request_metrics", TIMES),
        ("raw_response_metrics", PERCENTILES),
        ("transaction_metrics", TIMES),
        ("scenario_metrics", TIMES),
    ] {
        let rows = report[table].as_array().unwrap();
        assert!(!rows.is_empty(), "{}", table);
        for row in rows {
            for field in fields {
                assert!(row.get(*field).is_none(), "{} {}: {}", table, field, row);
                let field_us = format!("{field}_us");
                let heading = row["is_scenario"] == true && *field == "response_time_average";
                assert!(
                    row[&field_us].is_u64() || (heading && row[&field_us].is_null()),
                    "{} {}: {}",
                    table,
                    field_us,
                    row
                );
            }
        }
    }
}

/// The JSON report `name`, written by Goose `version`, loads as a baseline
/// and renders with `--render-report`: the JSON report rendered from it
/// writes its rows in microseconds only, rebuilt from its raw timing data,
/// so a 0.59 ms p50 is 590 µs whatever its rows held.
async fn assert_report_loads(name: &str, version: &str) {
    let server = MockServer::start();
    setup_mock_server_endpoints(&server);
    let dir = temp_dir(name);
    let path = fixture(name);
    let old: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(old["goose_version"], version);

    let md = dir.join("against.md").to_string_lossy().into_owned();
    run(
        &server,
        vec!["--baseline-file", &path, "--report-file", &md],
    )
    .await
    .unwrap_or_else(|e| panic!("{} loads as a baseline: {}", name, e));
    assert!(std::fs::read_to_string(&md).unwrap().contains("Aggregated"));

    let json = dir.join("rendered.json").to_string_lossy().into_owned();
    let rendered_md = dir.join("rendered.md").to_string_lossy().into_owned();
    render(&path, &[&json, &rendered_md])
        .await
        .unwrap_or_else(|e| panic!("{} renders: {}", name, e));
    let new: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    assert_microsecond_rows_only(&new);
    let aggregated = new["raw_response_metrics"]
        .as_array()
        .unwrap()
        .last()
        .unwrap();
    assert_eq!(aggregated["name"], "Aggregated");
    let p50 = aggregated["percentile_50_us"].as_u64().unwrap();
    assert!((584..=596).contains(&p50), "{}", p50);
    for (key, request) in new["raw_metrics"]["requests"].as_object().unwrap() {
        assert!(request["raw_data"]["times_us"].is_object(), "{}", key);
    }
    assert!(std::fs::read_to_string(&rendered_md)
        .unwrap()
        .contains("Aggregated"));
    std::fs::remove_dir_all(&dir).ok();
}

/// A JSON report from Goose 0.19.0, whose rows hold whole milliseconds only.
#[tokio::test]
#[serial]
async fn a_0_19_0_report_loads_as_a_baseline_and_renders() {
    assert_report_loads("report-0.19.0.json", "0.19.0").await;
}

/// A JSON report from Goose 0.19.1, whose rows hold whole milliseconds and
/// microseconds.
#[tokio::test]
#[serial]
async fn a_0_19_1_report_loads_as_a_baseline_and_renders() {
    assert_report_loads("report-0.19.1.json", "0.19.1-dev").await;
}

/// A JSON report from Goose 0.20, whose rows hold microseconds only.
#[tokio::test]
#[serial]
async fn a_0_20_report_loads_as_a_baseline_and_renders() {
    assert_report_loads("report-0.20.json", "0.20.0-dev").await;
}
