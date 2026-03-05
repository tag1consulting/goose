use httpmock::Method::GET;
use httpmock::MockServer;
use std::time::Duration;

mod common;

use goose::prelude::*;

// Paths used in load tests.
const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about";
const ERROR_PATH: &str = "/error";
const SLOW_PATH: &str = "/slow";

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

/// Verify that atomic counters are properly reset when metrics are reset after ramp-up.
///
/// By default (without `--no-reset-metrics`), Goose resets metrics after all users have
/// been spawned. The atomic counters must be zeroed alongside the aggregate data, otherwise
/// the final counts would include requests from the ramp-up phase.
#[tokio::test]
async fn test_atomic_counters_metrics_reset() {
    let server = MockServer::start();

    let mock_endpoint = server.mock(|when, then| {
        when.method(GET).path(INDEX_PATH);
        then.status(200);
    });

    // Use slow hatch-rate to create a meaningful ramp-up phase.
    // 2 users at 1/sec = 2 second ramp-up, then 2 seconds of stable load.
    let config = common::build_configuration(
        &server,
        vec!["--users", "2", "--hatch-rate", "1", "--run-time", "2"],
    );

    let metrics = common::run_load_test(
        common::build_load_test(
            config,
            vec![scenario!("ResetScenario").register_transaction(transaction!(get_index))],
            None,
            None,
        ),
        None,
    )
    .await;

    let index = metrics
        .requests
        .get("GET /")
        .expect("index request should exist in metrics");

    // The server saw all requests (ramp-up + stable), but Goose metrics were reset
    // after ramp-up, so Goose should report fewer than the server.
    assert!(
        index.success_count < mock_endpoint.calls(),
        "with reset, Goose success_count ({}) should be less than server calls ({})",
        index.success_count,
        mock_endpoint.calls()
    );

    // The critical invariant: atomic counters must stay consistent with raw_data
    // even after a reset.
    assert_eq!(
        index.success_count + index.fail_count,
        index.raw_data.counter,
        "success + fail should equal raw_data counter after reset"
    );
    assert_eq!(index.fail_count, 0, "no failures expected");
}

// Slow transaction that triggers Coordinated Omission backfilling.
async fn get_slow(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(SLOW_PATH).await?;
    Ok(())
}

/// Verify that Coordinated Omission synthetic requests are counted in atomic counters.
///
/// When CO mitigation is enabled and a slow response causes backfilling, the parent
/// thread generates synthetic request metrics. These must be counted via
/// `increment_registry_counter()` so that success_count includes them.
#[tokio::test]
async fn test_atomic_counters_coordinated_omission() {
    let server = MockServer::start();

    // Slow endpoint that triggers CO events: 100ms delay creates a cadence violation
    // when the user's expected cadence is much shorter.
    server.mock(|when, then| {
        when.method(GET).path(SLOW_PATH);
        then.status(200).delay(Duration::from_millis(100));
    });

    let config = common::build_configuration(
        &server,
        vec![
            "--users",
            "1",
            "--hatch-rate",
            "1",
            "--run-time",
            "3",
            "--co-mitigation",
            "average",
            "--no-reset-metrics",
        ],
    );

    let metrics = common::run_load_test(
        common::build_load_test(
            config,
            vec![scenario!("CoScenario").register_transaction(transaction!(get_slow))],
            None,
            None,
        ),
        None,
    )
    .await;

    let slow = metrics
        .requests
        .get("GET /slow")
        .expect("slow request should exist in metrics");

    // With CO mitigation, synthetic requests are injected, so raw_data.counter
    // (which counts both real and synthetic timing records) should be >= success_count.
    // The key invariant is that atomic counters stay consistent.
    assert!(
        slow.success_count > 0,
        "should have successful slow requests"
    );
    assert_eq!(
        slow.success_count + slow.fail_count,
        slow.raw_data.counter,
        "success + fail should equal raw_data counter with CO mitigation"
    );

    // If CO events actually occurred, the coordinated_omission_data should exist.
    if let Some(co_data) = &slow.coordinated_omission_data {
        // CO synthetic requests should have been recorded (counter > 0 means
        // timing records were generated beyond just the actual requests).
        assert!(
            co_data.counter > 0,
            "CO data counter should reflect synthetic requests"
        );
    }
}

/// Verify that atomic counters are properly isolated per request key.
///
/// Failures on one path should not affect another path's counters.
#[tokio::test]
async fn test_atomic_counters_isolation() {
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
        vec!["--users", "1", "--hatch-rate", "1", "--run-time", "2"],
    );

    let metrics = common::run_load_test(
        common::build_load_test(
            config,
            vec![scenario!("IsolationScenario")
                .register_transaction(transaction!(get_index).set_weight(2).unwrap())
                .register_transaction(transaction!(get_about).set_weight(2).unwrap())
                .register_transaction(transaction!(get_error).set_weight(1).unwrap())],
            None,
            None,
        ),
        None,
    )
    .await;

    // Index: all successes, zero failures.
    let index = metrics
        .requests
        .get("GET /")
        .expect("index request should exist");
    assert!(index.success_count > 0, "index should have successes");
    assert_eq!(index.fail_count, 0, "index should have zero failures");
    assert_eq!(
        index.success_count + index.fail_count,
        index.raw_data.counter,
        "index counter invariant"
    );

    // About: all successes, zero failures.
    let about = metrics
        .requests
        .get("GET /about")
        .expect("about request should exist");
    assert!(about.success_count > 0, "about should have successes");
    assert_eq!(about.fail_count, 0, "about should have zero failures");
    assert_eq!(
        about.success_count + about.fail_count,
        about.raw_data.counter,
        "about counter invariant"
    );

    // Error: all failures, zero successes — isolated from the successes above.
    let error = metrics
        .requests
        .get("GET /error")
        .expect("error request should exist");
    assert!(error.fail_count > 0, "error should have failures");
    assert_eq!(
        error.success_count, 0,
        "error should have zero successes (404 = failure)"
    );
    assert_eq!(
        error.success_count + error.fail_count,
        error.raw_data.counter,
        "error counter invariant"
    );
}

// Transaction that hits the error path without any override (records as failure).
async fn get_error(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(ERROR_PATH).await?;
    Ok(())
}
