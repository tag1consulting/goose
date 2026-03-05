use httpmock::Method::GET;
use httpmock::MockServer;

mod common;

use goose::prelude::*;

// Paths used in load tests.
const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about";
const ERROR_PATH: &str = "/error";

// Simple successful transaction.
async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// Transaction that hits a different path.
async fn get_about(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(ABOUT_PATH).await?;
    Ok(())
}

// Transaction that hits an error path, then uses set_success() to override.
async fn get_error_then_set_success(user: &mut GooseUser) -> TransactionResult {
    let mut goose = user.get(ERROR_PATH).await?;
    // Override: mark this 404 as a success.
    user.set_success(&mut goose.request)?;
    Ok(())
}

// Transaction that hits a success path, then uses set_failure() to override.
async fn get_success_then_set_failure(user: &mut GooseUser) -> TransactionResult {
    let mut goose = user.get_named(INDEX_PATH, "forced_fail").await?;
    // Override: mark this 200 as a failure.
    user.set_failure("intentional", &mut goose.request, None, None)?;
    Ok(())
}

/// Verify that atomic counters produce correct success_count and fail_count
/// values in the final metrics, matching the expected request outcomes.
#[tokio::test]
async fn test_atomic_counters_basic() {
    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(GET).path(INDEX_PATH);
        then.status(200);
    });
    server.mock(|when, then| {
        when.method(GET).path(ABOUT_PATH);
        then.status(200);
    });
    server.mock(|when, then| {
        when.method(GET).path(ERROR_PATH);
        then.status(404);
    });

    let config = common::build_configuration(
        &server,
        vec!["--users", "2", "--hatch-rate", "4", "--run-time", "2"],
    );

    let metrics = common::run_load_test(
        common::build_load_test(
            config,
            vec![scenario!("IndexScenario")
                .register_transaction(transaction!(get_index).set_weight(2).unwrap())
                .register_transaction(transaction!(get_about).set_weight(1).unwrap())],
            None,
            None,
        ),
        None,
    )
    .await;

    // Verify index requests: all should be successes.
    let index = metrics
        .requests
        .get("GET /")
        .expect("index request should exist in metrics");
    assert!(
        index.success_count > 0,
        "should have successful index requests"
    );
    assert_eq!(
        index.fail_count, 0,
        "index requests should have zero failures"
    );
    // Total should equal success + fail.
    assert_eq!(
        index.success_count + index.fail_count,
        index.raw_data.counter,
        "success + fail should equal raw_data counter for index"
    );

    // Verify about requests: all should be successes.
    let about = metrics
        .requests
        .get("GET /about")
        .expect("about request should exist in metrics");
    assert!(
        about.success_count > 0,
        "should have successful about requests"
    );
    assert_eq!(
        about.fail_count, 0,
        "about requests should have zero failures"
    );
    assert_eq!(
        about.success_count + about.fail_count,
        about.raw_data.counter,
        "success + fail should equal raw_data counter for about"
    );
}

/// Verify that set_success() correctly adjusts atomic counters from failure to success.
#[tokio::test]
async fn test_atomic_counters_set_success() {
    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(GET).path(ERROR_PATH);
        then.status(404);
    });

    let config = common::build_configuration(
        &server,
        vec!["--users", "1", "--hatch-rate", "1", "--run-time", "2"],
    );

    let metrics = common::run_load_test(
        common::build_load_test(
            config,
            vec![scenario!("SetSuccessScenario")
                .register_transaction(transaction!(get_error_then_set_success))],
            None,
            None,
        ),
        None,
    )
    .await;

    // The 404 was originally recorded as a failure, then set_success() was called.
    // The final count should show all requests as successes with zero failures.
    let error_req = metrics
        .requests
        .get("GET /error")
        .expect("error request should exist in metrics");
    assert!(
        error_req.success_count > 0,
        "set_success should have converted failures to successes"
    );
    assert_eq!(
        error_req.fail_count, 0,
        "set_success should have zeroed fail_count"
    );
}

/// Verify that set_failure() correctly adjusts atomic counters from success to failure.
#[tokio::test]
async fn test_atomic_counters_set_failure() {
    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(GET).path(INDEX_PATH);
        then.status(200);
    });

    let config = common::build_configuration(
        &server,
        vec!["--users", "1", "--hatch-rate", "1", "--run-time", "2"],
    );

    let metrics = common::run_load_test(
        common::build_load_test(
            config,
            vec![scenario!("SetFailureScenario")
                .register_transaction(transaction!(get_success_then_set_failure))],
            None,
            None,
        ),
        None,
    )
    .await;

    // The 200 was originally recorded as a success, then set_failure() was called.
    // The final count should show all requests as failures with zero successes.
    let forced_fail = metrics
        .requests
        .get("GET forced_fail")
        .expect("forced_fail request should exist in metrics");
    assert!(
        forced_fail.fail_count > 0,
        "set_failure should have converted successes to failures"
    );
    assert_eq!(
        forced_fail.success_count, 0,
        "set_failure should have zeroed success_count"
    );
}
