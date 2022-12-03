use httpmock::{
    Method::{DELETE, GET, HEAD, PATCH, POST, PUT},
    Mock, MockServer,
};
use serial_test::serial;

mod common;

use goose::config::GooseConfiguration;
use goose::prelude::*;

// Paths used in load tests performed during these tests.
const GET_PATH: &str = "/get";
const POST_PATH: &str = "/post";
const HEAD_PATH: &str = "/head";
const PATCH_PATH: &str = "/patch";
const PUT_PATH: &str = "/put";
const DELETE_PATH: &str = "/delete";

// Indexes to the above paths.
const GET_KEY: usize = 0;
const POST_KEY: usize = 1;
const HEAD_KEY: usize = 2;
const PATCH_KEY: usize = 3;
const PUT_KEY: usize = 4;
const DELETE_KEY: usize = 5;

// Load test configuration.
const EXPECT_WORKERS: usize = 2;
const USERS: usize = 5;
const RUN_TIME: usize = 2;

// Test transaction.
pub async fn get_builder(user: &mut GooseUser) -> TransactionResult {
    // Use builder to create a request.
    let goose_request = GooseRequest::builder()
        .path(GET_PATH)
        // Don't set method to `GooseMethod::Get` to confirm it's set by default.
        .build();

    // Make the configured request.
    let _goose = user.request(goose_request).await?;

    Ok(())
}

// Test transaction.
pub async fn get_nobuilder(user: &mut GooseUser) -> TransactionResult {
    // Make request.
    let _goose = user.get(GET_PATH).await?;

    Ok(())
}

// Test transaction.
pub async fn post_builder(user: &mut GooseUser) -> TransactionResult {
    // Use builder to create a request.
    let goose_request = GooseRequest::builder()
        .path(POST_PATH)
        .method(GooseMethod::Post)
        .build();

    // Make the configured request.
    let _goose = user.request(goose_request).await?;
    Ok(())
}

// Test transaction.
pub async fn post_nobuilder(user: &mut GooseUser) -> TransactionResult {
    // Make request.
    let _goose = user.post(POST_PATH, "post body").await?;

    Ok(())
}

// Test transaction.
pub async fn head_builder(user: &mut GooseUser) -> TransactionResult {
    // Use builder to create a request.
    let goose_request = GooseRequest::builder()
        .path(HEAD_PATH)
        .method(GooseMethod::Head)
        .build();

    // Make the configured request.
    let _goose = user.request(goose_request).await?;
    Ok(())
}

// Test transaction.
pub async fn head_nobuilder(user: &mut GooseUser) -> TransactionResult {
    // Make request.
    let _goose = user.head(HEAD_PATH).await?;

    Ok(())
}

// Test transaction.
pub async fn patch_builder(user: &mut GooseUser) -> TransactionResult {
    // Use builder to create a request.
    let goose_request = GooseRequest::builder()
        .path(PATCH_PATH)
        .method(GooseMethod::Patch)
        .build();

    // Make the configured request.
    let _goose = user.request(goose_request).await?;
    Ok(())
}

// Test transaction.
pub async fn put_builder(user: &mut GooseUser) -> TransactionResult {
    // Use builder to create a request.
    let goose_request = GooseRequest::builder()
        .path(PUT_PATH)
        .method(GooseMethod::Put)
        .build();

    // Make the configured request.
    let _goose = user.request(goose_request).await?;
    Ok(())
}

// Test transaction.
pub async fn delete_builder(user: &mut GooseUser) -> TransactionResult {
    // Use builder to create a request.
    let goose_request = GooseRequest::builder()
        .path(DELETE_PATH)
        .method(GooseMethod::Delete)
        .build();

    // Make the configured request.
    let _goose = user.request(goose_request).await?;
    Ok(())
}

// Test transaction.
pub async fn delete_nobuilder(user: &mut GooseUser) -> TransactionResult {
    // Make request.
    let _goose = user.delete(DELETE_PATH).await?;
    Ok(())
}

// All tests in this file run against common endpoints.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<Mock> {
    vec![
        // First set up GET_PATH, store in vector at GET_KEY.
        server.mock(|when, then| {
            when.method(GET).path(GET_PATH);
            then.status(200);
        }),
        // Next set up POST_PATH, store in vector at POST_KEY.
        server.mock(|when, then| {
            when.method(POST).path(POST_PATH);
            then.status(200);
        }),
        // Next set up HEAD_PATH, store in vector at HEAD_KEY.
        server.mock(|when, then| {
            when.method(HEAD).path(HEAD_PATH);
            then.status(200);
        }),
        // Next set up PATCH_PATH, store in vector at PATCH_KEY.
        server.mock(|when, then| {
            when.method(PATCH).path(PATCH_PATH);
            then.status(200);
        }),
        // Next set up PUT_PATH, store in vector at PUT_KEY.
        server.mock(|when, then| {
            when.method(PUT).path(PUT_PATH);
            then.status(200);
        }),
        // Next set up DELETE_PATH, store in vector at DELETE_KEY.
        server.mock(|when, then| {
            when.method(DELETE).path(DELETE_PATH);
            then.status(200);
        }),
    ]
}

// Build appropriate configuration for these tests.
fn common_build_configuration(
    server: &MockServer,
    worker: Option<bool>,
    manager: Option<usize>,
) -> GooseConfiguration {
    if let Some(expect_workers) = manager {
        common::build_configuration(
            server,
            vec![
                "--manager",
                "--expect-workers",
                &expect_workers.to_string(),
                "--users",
                &USERS.to_string(),
                "--hatch-rate",
                &USERS.to_string(),
                "--run-time",
                &RUN_TIME.to_string(),
                "--no-reset-metrics",
            ],
        )
    } else if worker.is_some() {
        common::build_configuration(server, vec!["--worker"])
    } else {
        common::build_configuration(
            server,
            vec![
                "--users",
                &USERS.to_string(),
                "--hatch-rate",
                &USERS.to_string(),
                "--run-time",
                &RUN_TIME.to_string(),
                "--no-reset-metrics",
            ],
        )
    }
}

// Helper to confirm all variations generate appropriate results.
fn validate_test(is_builder: bool, mock_endpoints: &[Mock], goose_metrics: GooseMetrics) {
    // Confirm that the on_start and on_exit transactions actually ran once per GooseUser.
    assert!(mock_endpoints[GET_KEY].hits() > 0);
    assert!(mock_endpoints[POST_KEY].hits() > 0);
    assert!(mock_endpoints[HEAD_KEY].hits() > 0);
    assert!(mock_endpoints[DELETE_KEY].hits() > 0);

    // PATCH and PUT are only possible when using builder.
    if is_builder {
        assert!(mock_endpoints[PATCH_KEY].hits() > 0);
        assert!(mock_endpoints[PUT_KEY].hits() > 0);
    }

    // Validate GET requests.
    let get_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", GET_PATH))
        .unwrap();
    assert!(get_metrics.path == GET_PATH);
    assert!(get_metrics.method == GooseMethod::Get);
    assert!(get_metrics.fail_count == 0);
    assert!(get_metrics.success_count > 0);

    // Validate POST requests.
    let post_metrics = goose_metrics
        .requests
        .get(&format!("POST {}", POST_PATH))
        .unwrap();
    assert!(post_metrics.path == POST_PATH);
    assert!(post_metrics.method == GooseMethod::Post);
    assert!(post_metrics.fail_count == 0);
    assert!(post_metrics.success_count > 0);

    // Validate HEAD requests.
    let head_metrics = goose_metrics
        .requests
        .get(&format!("HEAD {}", HEAD_PATH))
        .unwrap();
    assert!(head_metrics.path == HEAD_PATH);
    assert!(head_metrics.method == GooseMethod::Head);
    assert!(head_metrics.fail_count == 0);
    assert!(head_metrics.success_count > 0);

    // Validate DELETE requests.
    let patch_metrics = goose_metrics
        .requests
        .get(&format!("DELETE {}", DELETE_PATH))
        .unwrap();
    assert!(patch_metrics.path == DELETE_PATH);
    assert!(patch_metrics.method == GooseMethod::Delete);
    assert!(patch_metrics.fail_count == 0);
    assert!(patch_metrics.success_count > 0);

    // PATCH and PUT are only possible when using builder.
    if is_builder {
        // Validate PATCH requests.
        let patch_metrics = goose_metrics
            .requests
            .get(&format!("PATCH {}", PATCH_PATH))
            .unwrap();
        assert!(patch_metrics.path == PATCH_PATH);
        assert!(patch_metrics.method == GooseMethod::Patch);
        assert!(patch_metrics.fail_count == 0);
        assert!(patch_metrics.success_count > 0);

        // Validate PUT requests.
        let patch_metrics = goose_metrics
            .requests
            .get(&format!("PUT {}", PUT_PATH))
            .unwrap();
        assert!(patch_metrics.path == PUT_PATH);
        assert!(patch_metrics.method == GooseMethod::Put);
        assert!(patch_metrics.fail_count == 0);
        assert!(patch_metrics.success_count > 0);
    }
}

// Returns the appropriate scenario needed to build these tests.
fn get_transactions(is_builder: bool) -> Scenario {
    if is_builder {
        scenario!("LoadTest")
            .register_transaction(transaction!(get_builder))
            .register_transaction(transaction!(post_builder))
            .register_transaction(transaction!(head_builder))
            .register_transaction(transaction!(patch_builder))
            .register_transaction(transaction!(put_builder))
            .register_transaction(transaction!(delete_builder))
    } else {
        scenario!("LoadTest")
            .register_transaction(transaction!(get_nobuilder))
            .register_transaction(transaction!(post_nobuilder))
            .register_transaction(transaction!(head_nobuilder))
            .register_transaction(transaction!(delete_nobuilder))
    }
}

// Helper to run the test, takes a flag for indicating if running in standalone
// mode or Gaggle mode.
async fn run_load_test(is_builder: bool, is_gaggle: bool) {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Configure and run test differently in standalone versus Gaggle mode.
    let metrics = match is_gaggle {
        false => {
            // Build common configuration.
            let configuration = common_build_configuration(&server, None, None);

            // Run the Goose Attack.
            common::run_load_test(
                common::build_load_test(
                    configuration,
                    vec![get_transactions(is_builder)],
                    None,
                    None,
                ),
                None,
            )
            .await
        }
        true => {
            // Build common configuration.
            let worker_configuration = common_build_configuration(&server, Some(true), None);

            // Workers launched in own threads, store thread handles.
            let worker_handles = common::launch_gaggle_workers(EXPECT_WORKERS, || {
                common::build_load_test(
                    worker_configuration.clone(),
                    vec![get_transactions(is_builder)],
                    None,
                    None,
                )
            });

            // Build Manager configuration.
            let manager_configuration =
                common_build_configuration(&server, None, Some(EXPECT_WORKERS));

            // Run the Goose Attack.
            common::run_load_test(
                common::build_load_test(
                    manager_configuration,
                    vec![get_transactions(is_builder)],
                    None,
                    None,
                ),
                Some(worker_handles),
            )
            .await
        }
    };

    // Confirm the load test ran correctly.
    validate_test(is_builder, &mock_endpoints, metrics);
}

#[tokio::test]
// Test scenario using GooseRequest::builder().
async fn test_request_builder() {
    // Run load test with is_builder set to true and is_gaggle set to false.
    run_load_test(true, false).await;
}

#[tokio::test]
// Test scenario without using GooseRequest::builder().
async fn test_request_no_builder() {
    // Run load test with is_builder set to false and is_gaggle set to false.
    run_load_test(false, false).await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Test scenario using GooseRequest::builder().
async fn test_request_builder_gaggle() {
    // Run load test with is_builder set to true and is_gaggle set to true.
    run_load_test(true, true).await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Test scenario without using GooseRequest::builder().
async fn test_request_no_builder_gaggle() {
    // Run load test with is_builder set to false and is_gaggle set to true.
    run_load_test(false, true).await;
}
