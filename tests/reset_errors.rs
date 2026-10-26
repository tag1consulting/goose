//! The errors tables restart at the metrics reset after ramp up, as every
//! other table does, and `--no-reset-metrics` keeps the ramp up's errors.

use httpmock::{Method::GET, MockServer};
use serial_test::serial;

mod common;

use goose::prelude::*;

const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about/";

/// Users launched; the last one launched never fails.
const USERS: usize = 3;

/// Each user but the last one launched requests a page that is not found
/// and fails its transaction when it starts, so every error happens during
/// the ramp up.
pub async fn fail_while_ramping(user: &mut GooseUser) -> TransactionResult {
    if user.weighted_users_index + 1 < USERS {
        let _goose = user.get(ABOUT_PATH).await?;
        return Err(Box::new(TransactionError::Custom("ramping up".to_string())));
    }
    Ok(())
}

/// After the ramp up every request succeeds.
pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

async fn run(reset: bool) -> GooseMetrics {
    let server = MockServer::start();
    let about = server.mock(|when, then| {
        when.method(GET).path(ABOUT_PATH);
        then.status(404);
    });
    server.mock(|when, then| {
        when.method(GET).path(INDEX_PATH);
        then.status(200);
    });

    let users = USERS.to_string();
    let mut options = vec!["--users", &users, "--increase-rate", "1", "--run-time", "2"];
    if !reset {
        options.push("--no-reset-metrics");
    }
    let configuration = common::build_configuration(&server, options);
    let scenario = scenario!("LoadTest")
        .register_transaction(transaction!(fail_while_ramping).set_on_start())
        .register_transaction(transaction!(get_index));
    let metrics = common::run_load_test(
        common::build_load_test(configuration, vec![scenario], None, None),
        None,
    )
    .await;
    // Both users that failed did so before the last user launched.
    about.assert_calls(USERS - 1);
    metrics
}

/// After the reset no error from the ramp up is counted, and the summary
/// prints no errors tables.
#[tokio::test]
#[serial]
async fn errors_restart_at_reset() {
    let metrics = run(true).await;
    assert!(metrics.errors.is_empty(), "{:?}", metrics.errors);
    assert!(
        metrics.transaction_errors.is_empty(),
        "{:?}",
        metrics.transaction_errors
    );
    let summary = metrics.to_string();
    assert!(!summary.contains("=== ERRORS ==="), "{}", summary);
    assert!(
        !summary.contains("=== TRANSACTION ERRORS ==="),
        "{}",
        summary
    );
}

/// With `--no-reset-metrics` the ramp up's errors are kept.
#[tokio::test]
#[serial]
async fn no_reset_keeps_ramp_errors() {
    let metrics = run(false).await;
    let occurrences: Vec<(&str, usize)> = metrics
        .errors
        .values()
        .map(|error| (error.name.as_str(), error.occurrences))
        .collect();
    assert_eq!(occurrences, vec![(ABOUT_PATH, USERS - 1)]);
    let transaction_errors: Vec<(bool, usize)> = metrics
        .transaction_errors
        .values()
        .map(|error| (error.error.contains("ramping up"), error.occurrences))
        .collect();
    assert_eq!(transaction_errors, vec![(true, USERS - 1)]);
    let summary = metrics.to_string();
    assert!(summary.contains("=== ERRORS ==="), "{}", summary);
    assert!(
        summary.contains("=== TRANSACTION ERRORS ==="),
        "{}",
        summary
    );
}
