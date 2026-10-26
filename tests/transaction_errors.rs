//! A transaction that returns an error records it: in the transaction log's
//! `error` field, in the error log, and in the errors tables of the console
//! summary and the reports.

use httpmock::{Method::GET, MockServer};
use serial_test::serial;
use std::path::{Path, PathBuf};

mod common;

use goose::prelude::*;

const INDEX_PATH: &str = "/";

/// Every other run of this transaction returns `TransactionError::Custom`.
pub async fn boom(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    if user.get_iterations() % 2 == 1 {
        return Err(Box::new(TransactionError::Custom("boom".to_string())));
    }
    Ok(())
}

/// Marks its request failed, returning `TransactionError::RequestFailed`.
pub async fn fail_request(user: &mut GooseUser) -> TransactionResult {
    let mut goose = user.get(INDEX_PATH).await?;
    user.set_failure("rejected by the test", &mut goose.request, None, None)
}

/// Makes one request after another, throttled, until the run ends.
pub async fn throttled(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

/// Fails when its request can not be sent: `.response?` returns the
/// `TransactionError::Reqwest` the request path already recorded.
pub async fn unreachable(user: &mut GooseUser) -> TransactionResult {
    let _response = user.get(INDEX_PATH).await?.response?;
    Ok(())
}

/// Makes a request that fails, then returns `TransactionError::Custom`: one
/// request error and one transaction error.
pub async fn both_errors(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(MISSING_PATH).await?;
    Err(Box::new(TransactionError::Custom("boom".to_string())))
}

const MISSING_PATH: &str = "/missing";

fn setup_mock_server_endpoints(server: &MockServer) {
    server.mock(|when, then| {
        when.method(GET).path(INDEX_PATH);
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
        "goose-transaction-errors-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn path(dir: &Path, name: &str) -> String {
    dir.join(name).to_string_lossy().into_owned()
}

/// Each line of a JSON log.
fn json_lines(path: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn read_json(path: &str) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Run one load test of `transaction` with `args` against `server`.
async fn run(
    server: &MockServer,
    transaction: Transaction,
    args: Vec<&str>,
) -> Result<GooseMetrics, GooseError> {
    let configuration = common::build_configuration(server, args);
    common::build_load_test(
        configuration,
        vec![scenario!("LoadTest").register_transaction(transaction)],
        None,
        None,
    )
    .execute()
    .await
}

const BOOM: &str = "TransactionError: custom error (boom)";

/// `TransactionError::Custom("boom")` appears in the error log, the
/// transaction log's `error` field, the console summary and the JSON and HTML
/// reports, and a report holding it loads as a baseline.
#[tokio::test]
#[serial]
async fn a_custom_error_is_recorded() {
    let server = MockServer::start();
    setup_mock_server_endpoints(&server);
    let dir = temp_dir("custom");
    let (error_log, transaction_log) = (path(&dir, "error.log"), path(&dir, "transaction.log"));
    let (json, html) = (path(&dir, "report.json"), path(&dir, "report.html"));
    let goose_metrics = run(
        &server,
        transaction!(boom).set_name("boom"),
        vec![
            "--no-reset-metrics",
            "--users",
            "2",
            "--increase-rate",
            "2",
            "--run-time",
            "2",
            "--error-log",
            &error_log,
            "--transaction-log",
            &transaction_log,
            "--report-file",
            &json,
            "--report-file",
            &html,
        ],
    )
    .await
    .unwrap();

    let failed = goose_metrics.transactions[0][0].fail_count;
    assert!(failed > 0);
    let errors: Vec<_> = goose_metrics.transaction_errors.values().collect();
    assert_eq!(errors.len(), 1, "{:?}", errors);
    assert_eq!(errors[0].occurrences, failed);
    assert_eq!(errors[0].error, BOOM);
    assert_eq!(errors[0].scenario_name, "LoadTest");
    assert_eq!(errors[0].transaction_name, "boom");
    assert!(goose_metrics.errors.is_empty());

    // One error log entry for each failed transaction, shaped as a transaction.
    let entries = json_lines(&error_log);
    assert_eq!(entries.len(), failed);
    for entry in &entries {
        assert_eq!(entry["error"], BOOM);
        // The error log names the scenario by its machine name, as the
        // request log does.
        assert_eq!(entry["scenario_name"], "loadtest");
        assert_eq!(entry["transaction_name"], "boom");
        assert_eq!(entry["transaction_index"], 0);
        assert!(entry.get("raw").is_none(), "{}", entry);
    }

    // The transaction log names the error of each failed transaction only.
    let lines = json_lines(&transaction_log);
    assert!(lines.iter().any(|line| line["success"] == true));
    for line in &lines {
        let expected = if line["success"] == true { "" } else { BOOM };
        assert_eq!(line["error"], expected, "{}", line);
    }

    // The console summary's transaction errors table.
    let printed = goose_metrics.to_string();
    assert!(
        printed.contains("=== TRANSACTION ERRORS ==="),
        "{}",
        printed
    );
    assert!(
        printed.contains(&format!("1.1 LoadTest boom: {BOOM}")),
        "{}",
        printed
    );

    // The JSON report's.
    let report = read_json(&json);
    let rows = report["transaction_errors"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["transaction"], "1.1");
    assert_eq!(rows[0]["scenario"], "LoadTest");
    assert_eq!(rows[0]["name"], "boom");
    assert_eq!(rows[0]["error"], BOOM);
    assert_eq!(rows[0]["occurrences"], failed);

    // The HTML report's.
    let html_text = std::fs::read_to_string(&html).unwrap();
    assert!(html_text.contains("<h2>Transaction Errors</h2>"));
    assert!(html_text.contains(BOOM));

    // The report loads as a baseline, and the next report compares with it.
    let md = path(&dir, "against.md");
    run(
        &server,
        transaction!(boom).set_name("boom"),
        vec![
            "--users",
            "2",
            "--increase-rate",
            "2",
            "--run-time",
            "2",
            "--baseline-file",
            &json,
            "--report-file",
            &md,
        ],
    )
    .await
    .expect("a report with transaction errors loads as a baseline");
    let md = std::fs::read_to_string(&md).unwrap();
    let section = md
        .split("## Transaction Error Metrics")
        .nth(1)
        .unwrap_or_else(|| panic!("no transaction errors section: {}", md));
    let row = section
        .lines()
        .find(|line| line.contains(BOOM))
        .unwrap_or_else(|| panic!("no transaction error row: {}", section));
    assert!(row.contains("(+") || row.contains("(-"), "{}", row);

    std::fs::remove_dir_all(&dir).ok();
}

/// `--no-error-summary` leaves transaction errors out of the tables, not out
/// of the logs.
#[tokio::test]
#[serial]
async fn no_error_summary_leaves_transaction_errors_out_of_the_tables() {
    let server = MockServer::start();
    setup_mock_server_endpoints(&server);
    let dir = temp_dir("no-summary");
    let (error_log, json) = (path(&dir, "error.log"), path(&dir, "report.json"));
    let goose_metrics = run(
        &server,
        transaction!(boom),
        vec![
            "--no-error-summary",
            "--error-log",
            &error_log,
            "--report-file",
            &json,
        ],
    )
    .await
    .unwrap();
    assert!(goose_metrics.transactions[0][0].fail_count > 0);
    assert!(goose_metrics.transaction_errors.is_empty());
    assert!(read_json(&json).get("transaction_errors").is_none());
    assert!(!json_lines(&error_log).is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

/// A transaction that returns `RequestFailed` adds no row to any errors table:
/// its request error is counted once, and the transaction log names it. Errors
/// are kept when metrics are reset after the ramp up, and transaction counts
/// are not, so they are compared without a reset.
#[tokio::test]
#[serial]
async fn a_failed_request_is_not_counted_twice() {
    let server = MockServer::start();
    setup_mock_server_endpoints(&server);
    let dir = temp_dir("request-failed");
    let (error_log, transaction_log) = (path(&dir, "error.log"), path(&dir, "transaction.log"));
    let json = path(&dir, "report.json");
    let goose_metrics = run(
        &server,
        transaction!(fail_request),
        vec![
            "--no-reset-metrics",
            "--error-log",
            &error_log,
            "--transaction-log",
            &transaction_log,
            "--report-file",
            &json,
        ],
    )
    .await
    .unwrap();

    assert!(goose_metrics.transaction_errors.is_empty());
    assert_eq!(goose_metrics.errors.len(), 1);
    let request_errors = goose_metrics.errors.values().next().unwrap();
    let failed = goose_metrics.transactions[0][0].fail_count;
    assert_eq!(request_errors.occurrences, failed);
    let report = read_json(&json);
    assert!(report.get("transaction_errors").is_none());
    assert_eq!(report["errors"].as_array().unwrap().len(), 1);

    // Every error log entry is a request error.
    for entry in json_lines(&error_log) {
        assert!(entry.get("raw").is_some(), "{}", entry);
    }
    for line in json_lines(&transaction_log) {
        assert_eq!(line["success"], false);
        assert_eq!(line["error"], "request failed: GET /: rejected by the test");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// Requests still waiting on the throttle when the run stops are canceled;
/// their transactions fail with `RequestCanceled`, which is no error anywhere.
#[tokio::test]
#[serial]
async fn a_canceled_request_is_no_error() {
    let server = MockServer::start();
    setup_mock_server_endpoints(&server);
    let dir = temp_dir("canceled");
    let (error_log, transaction_log) = (path(&dir, "error.log"), path(&dir, "transaction.log"));
    let goose_metrics = run(
        &server,
        transaction!(throttled),
        vec![
            "--users",
            "10",
            "--increase-rate",
            "100",
            "--run-time",
            "2",
            "--throttle-requests",
            "2",
            "--error-log",
            &error_log,
            "--transaction-log",
            &transaction_log,
        ],
    )
    .await
    .unwrap();

    assert!(goose_metrics.transaction_errors.is_empty());
    assert!(goose_metrics.errors.is_empty());
    assert!(json_lines(&error_log).is_empty());
    let lines = json_lines(&transaction_log);
    // The users waiting on the throttle when the run stopped: their
    // transactions failed, canceled, and say no error.
    assert!(
        lines.iter().any(|line| line["success"] == false),
        "no transaction was canceled"
    );
    for line in lines {
        assert_eq!(line["error"], "", "{}", line);
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A request that can not be sent is a request error; the transaction that
/// returns its `TransactionError::Reqwest` adds no second row, and the
/// transaction log names it.
#[tokio::test]
#[serial]
async fn a_request_that_can_not_be_sent_is_not_counted_twice() {
    let server = MockServer::start();
    let dir = temp_dir("unreachable");
    let (error_log, transaction_log) = (path(&dir, "error.log"), path(&dir, "transaction.log"));
    let goose_metrics = run(
        &server,
        transaction!(unreachable),
        vec![
            // Nothing listens on the discard port.
            "--host",
            "http://127.0.0.1:9",
            "--no-reset-metrics",
            "--error-log",
            &error_log,
            "--transaction-log",
            &transaction_log,
        ],
    )
    .await
    .unwrap();

    assert!(goose_metrics.transaction_errors.is_empty());
    assert_eq!(goose_metrics.errors.len(), 1, "{:?}", goose_metrics.errors);
    let failed = goose_metrics.transactions[0][0].fail_count;
    assert!(failed > 0);
    assert_eq!(
        goose_metrics.errors.values().next().unwrap().occurrences,
        failed
    );
    for entry in json_lines(&error_log) {
        assert!(entry.get("raw").is_some(), "{}", entry);
    }
    for line in json_lines(&transaction_log) {
        assert_eq!(line["success"], false);
        let error = line["error"].as_str().unwrap();
        assert!(
            error.starts_with("TransactionError: reqwest::Error"),
            "{}",
            error
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// `--no-transaction-metrics` writes no transaction log, but still records
/// transaction errors, as it still records request errors.
#[tokio::test]
#[serial]
async fn no_transaction_metrics_still_records_transaction_errors() {
    let server = MockServer::start();
    setup_mock_server_endpoints(&server);
    let dir = temp_dir("no-transaction-metrics");
    let error_log = path(&dir, "error.log");
    let goose_metrics = run(
        &server,
        transaction!(boom).set_name("boom"),
        vec!["--no-transaction-metrics", "--error-log", &error_log],
    )
    .await
    .unwrap();
    let errors: Vec<_> = goose_metrics.transaction_errors.values().collect();
    assert_eq!(errors.len(), 1, "{:?}", errors);
    assert_eq!(errors[0].error, BOOM);
    assert!(errors[0].occurrences > 0);
    let entries = json_lines(&error_log);
    assert!(!entries.is_empty());
    assert!(entries.iter().all(|entry| entry["error"] == BOOM));
    std::fs::remove_dir_all(&dir).ok();
}

/// An error log in CSV holding request errors and transaction errors reads
/// with a strict CSV reader: every row has the header's columns.
#[tokio::test]
#[serial]
async fn the_error_log_csv_reads_strictly() {
    let server = MockServer::start();
    setup_mock_server_endpoints(&server);
    let dir = temp_dir("csv");
    let error_log = path(&dir, "error.csv");
    run(
        &server,
        transaction!(both_errors),
        vec!["--error-log", &error_log, "--error-format", "csv"],
    )
    .await
    .unwrap();

    let mut reader = csv::ReaderBuilder::new()
        .flexible(false)
        .from_path(&error_log)
        .unwrap();
    let header = reader.headers().unwrap().clone();
    assert_eq!(header.len(), 14);
    let (mut requests, mut transactions) = (0, 0);
    for record in reader.records() {
        let record = record.expect("every row has the header's columns");
        let cell = |name: &str| &record[header.iter().position(|h| h == name).unwrap()];
        if cell("raw").is_empty() {
            transactions += 1;
            assert_eq!(cell("error"), BOOM);
            assert_eq!(cell("transaction_index"), "0");
        } else {
            requests += 1;
            assert_eq!(cell("error"), "404 Not Found: /missing");
            assert_eq!(cell("transaction_name"), "");
        }
    }
    assert!(
        requests > 0 && transactions > 0,
        "{} {}",
        requests,
        transactions
    );
    std::fs::remove_dir_all(&dir).ok();
}
