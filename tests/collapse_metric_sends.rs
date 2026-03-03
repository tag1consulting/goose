use httpmock::{Method::GET, MockServer};
use serial_test::serial;

mod common;

use goose::prelude::*;

// Verify that the GooseMetric::All optimization produces correct aggregated
// metrics. A single-transaction, single-request scenario is the best case
// for the optimisation: the request, transaction, and scenario metrics are
// combined into one channel message instead of three.

const INDEX_PATH: &str = "/";

pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

fn setup_mock(server: &MockServer) -> httpmock::Mock<'_> {
    server.mock(|when, then| {
        when.method(GET).path(INDEX_PATH);
        then.status(200);
    })
}

// Run a short load test with 1 user, 1 scenario, 1 transaction and validate
// that request, transaction, and scenario metrics are all correctly recorded.
#[tokio::test]
#[serial]
async fn single_request_per_scenario_metrics_correct() {
    let server = MockServer::start();
    let mock = setup_mock(&server);

    let config = common::build_configuration(
        &server,
        vec!["--users", "1", "--hatch-rate", "1", "--run-time", "2"],
    );

    let goose = common::build_load_test(
        config,
        vec![scenario!("Single").register_transaction(transaction!(get_index))],
        None,
        None,
    );

    let metrics = common::run_load_test(goose, None).await;

    // The mock endpoint should have been hit.
    assert!(mock.calls() > 0, "mock endpoint should be called");

    // Request metrics: there should be exactly one request key (GET /).
    assert_eq!(metrics.requests.len(), 1, "expected one request key");
    let req = metrics.requests.values().next().unwrap();
    assert!(req.success_count > 0, "should have successful requests");
    assert_eq!(req.fail_count, 0, "should have no failed requests");

    // Transaction metrics: one scenario with one transaction.
    assert_eq!(metrics.transactions.len(), 1, "expected one scenario");
    assert_eq!(
        metrics.transactions[0].len(),
        1,
        "expected one transaction in the scenario"
    );
    let txn = &metrics.transactions[0][0];
    assert!(txn.success_count > 0, "should have successful transactions");

    // Scenario metrics: one scenario with iterations.
    assert_eq!(metrics.scenarios.len(), 1, "expected one scenario metric");
    let scn = &metrics.scenarios[0];
    assert!(scn.counter > 0, "scenario should have counted iterations");
}

// Verify metrics correctness with multiple transactions per scenario (the
// fallback path where not all three metrics can be combined into All).
pub async fn get_other(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/other").await?;
    Ok(())
}

#[tokio::test]
#[serial]
async fn multi_transaction_scenario_metrics_correct() {
    let server = MockServer::start();
    let _mock_index = setup_mock(&server);
    let _mock_other = server.mock(|when, then| {
        when.method(GET).path("/other");
        then.status(200);
    });

    let config = common::build_configuration(
        &server,
        vec!["--users", "1", "--hatch-rate", "1", "--run-time", "2"],
    );

    let goose = common::build_load_test(
        config,
        vec![scenario!("Multi")
            .register_transaction(transaction!(get_index))
            .register_transaction(transaction!(get_other))],
        None,
        None,
    );

    let metrics = common::run_load_test(goose, None).await;

    // Two request keys: GET / and GET /other.
    assert_eq!(metrics.requests.len(), 2, "expected two request keys");
    for req in metrics.requests.values() {
        assert!(req.success_count > 0, "each request key should have hits");
    }

    // Transaction metrics: one scenario with two transactions.
    assert_eq!(metrics.transactions.len(), 1);
    assert_eq!(metrics.transactions[0].len(), 2);
    for txn in &metrics.transactions[0] {
        assert!(txn.success_count > 0, "each transaction should have hits");
    }

    // Scenario metrics.
    assert_eq!(metrics.scenarios.len(), 1);
    assert!(metrics.scenarios[0].counter > 0);
}

// Verify that set_success / set_failure still work correctly with the
// buffered metric pipeline.
pub async fn get_and_override_success(user: &mut GooseUser) -> TransactionResult {
    let mut goose = user.get("/fail").await?;
    // The server returns 500, so Goose marks it as failure. Override to success.
    if let Ok(_response) = &goose.response {
        return user.set_success(&mut goose.request);
    }
    Ok(())
}

#[tokio::test]
#[serial]
async fn set_success_with_buffered_metrics() {
    let server = MockServer::start();
    let _mock = server.mock(|when, then| {
        when.method(GET).path("/fail");
        then.status(500);
    });

    let config = common::build_configuration(
        &server,
        vec![
            "--users",
            "1",
            "--hatch-rate",
            "1",
            "--run-time",
            "2",
            // Disable error_on_fail so the test doesn't abort on 500.
        ],
    );

    let goose = common::build_load_test(
        config,
        vec![scenario!("Override").register_transaction(transaction!(get_and_override_success))],
        None,
        None,
    );

    let metrics = common::run_load_test(goose, None).await;

    // With set_success, the request should be counted as a success.
    let req = metrics.requests.values().next().unwrap();
    assert!(
        req.success_count > 0,
        "set_success should mark requests as successful"
    );
}

// Verify that --no-transaction-metrics still works (the fallback path where
// transaction metrics are disabled but request/scenario metrics are recorded).
#[tokio::test]
#[serial]
async fn no_transaction_metrics_flag() {
    let server = MockServer::start();
    let _mock = setup_mock(&server);

    let config = common::build_configuration(
        &server,
        vec![
            "--users",
            "1",
            "--hatch-rate",
            "1",
            "--run-time",
            "2",
            "--no-transaction-metrics",
        ],
    );

    let goose = common::build_load_test(
        config,
        vec![scenario!("NoTxn").register_transaction(transaction!(get_index))],
        None,
        None,
    );

    let metrics = common::run_load_test(goose, None).await;

    // Request metrics should still be recorded.
    assert!(!metrics.requests.is_empty());
    let req = metrics.requests.values().next().unwrap();
    assert!(req.success_count > 0);

    // Transaction metrics should be empty (counters are zero).
    for scenario_txns in &metrics.transactions {
        for txn in scenario_txns {
            assert_eq!(txn.success_count, 0);
            assert_eq!(txn.fail_count, 0);
        }
    }
}
