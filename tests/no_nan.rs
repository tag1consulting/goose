//! A run whose users complete no iteration and make no request, and a
//! scenario no user runs, show 0, never NaN, in the console and every report;
//! and the users running a scenario are counted though they completed no
//! iteration.

use httpmock::{Method::GET, MockServer};
use std::time::Duration;

use goose::prelude::*;

mod common;

const REPORTS: [&str; 3] = ["test-no-nan.html", "test-no-nan.md", "test-no-nan.json"];

async fn start(_user: &mut GooseUser) -> TransactionResult {
    Ok(())
}

async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

#[tokio::test]
async fn test_no_iteration_no_request_no_nan() {
    let server = MockServer::start();
    let index = server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200);
    });

    // Two users start Waiting, then wait longer than the run before its
    // request: no iteration completes and no request is made. No user runs
    // Other.
    let mut flags = vec!["--users", "2", "--increase-rate", "2", "--run-time", "2"];
    flags.extend(["--scenarios", "waiting"]);
    for report in REPORTS {
        flags.extend(["--report-file", report]);
    }
    let configuration = common::build_configuration(&server, flags);
    let waiting = scenario!("Waiting")
        .set_wait_time(Duration::from_secs(20), Duration::from_secs(20))
        .unwrap()
        .register_transaction(transaction!(start))
        .register_transaction(transaction!(get_index));
    let other = scenario!("Other").register_transaction(transaction!(get_index));
    let goose_metrics = common::run_load_test(
        common::build_load_test(configuration, vec![waiting, other], None, None),
        None,
    )
    .await;

    assert_eq!(index.calls(), 0);
    assert_eq!(goose_metrics.scenarios[0].run_times.counter, 0);
    assert_eq!(goose_metrics.scenarios[0].users.len(), 2);
    assert!(goose_metrics.scenarios[1].users.is_empty());

    let console = goose_metrics.to_string();
    let reports: Vec<String> = REPORTS
        .iter()
        .map(|report| std::fs::read_to_string(report).unwrap())
        .collect();
    common::cleanup_files(REPORTS.to_vec());
    assert!(!console.contains("NaN"), "{}", console);
    for (report, text) in REPORTS.iter().zip(&reports) {
        assert!(!text.contains("NaN"), "{} has NaN", report);
    }
}
