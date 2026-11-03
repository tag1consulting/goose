use httpmock::{Method::GET, Mock, MockServer};
use std::sync::Arc;

mod common;

use goose::config::GooseConfiguration;
use goose::goose::GooseMethod;
use goose::prelude::*;

// Paths used in load tests performed during these tests.
const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about.html";

// Test transaction.
pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// Test transaction.
pub async fn get_about(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(ABOUT_PATH).await?;
    Ok(())
}

// Structure defining a load test endpoint.
#[derive(Debug)]
struct LoadtestEndpoint<'a> {
    pub path: &'a str,
    pub status_code: u16,
    pub weight: usize,
}

// Configure endpoints to test.
fn configure_mock_endpoints<'a>() -> Vec<LoadtestEndpoint<'a>> {
    vec![
        LoadtestEndpoint {
            path: INDEX_PATH,
            status_code: 200,
            weight: 9,
        },
        LoadtestEndpoint {
            path: ABOUT_PATH,
            status_code: 200,
            weight: 3,
        },
    ]
}

// All tests in this file run against common endpoints.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<Mock<'_>> {
    // Get common configuration for building endpoints and the load test itself.
    let test_endpoints = configure_mock_endpoints();

    // Setup mock endpoints.
    let mut mock_endpoints = Vec::with_capacity(test_endpoints.len());
    for (idx, item) in test_endpoints.iter().enumerate() {
        let path = item.path;
        let mock_endpoint = server.mock(|when, then| {
            when.method(GET).path(path);
            then.status(item.status_code);
        });

        // Ensure the index matches.
        assert!(idx == mock_endpoints.len());
        mock_endpoints.push(mock_endpoint);
    }

    mock_endpoints
}

// Build load test configuration.
fn common_build_configuration(server: &MockServer, users: usize) -> GooseConfiguration {
    common::build_configuration(
        server,
        vec![
            "--no-reset-metrics",
            "--no-transaction-metrics",
            "--users",
            &users.to_string(),
            "--increase-rate",
            &(users * 2).to_string(),
        ],
    )
}

// Dynamically build scenario.
fn build_scenario() -> Scenario {
    // Get common configuration for building endpoints and the load test itself.
    let test_endpoints = configure_mock_endpoints();

    let mut scenario = Scenario::new("LoadTest");
    for item in &test_endpoints {
        let path = item.path;
        let weight = item.weight;

        let closure: TransactionFunction = Arc::new(move |user| {
            Box::pin(async move {
                let _goose = user.get(path).await?;

                Ok(())
            })
        });

        let transaction = Transaction::new(closure).set_weight(weight).unwrap();
        // We need to do the variable dance as scenario.register_transaction returns self and hence moves
        // self out of `scenario`. By storing it in a new local variable and then moving it over
        // we can avoid that error.
        let new_scenario = scenario.register_transaction(transaction);
        scenario = new_scenario;
    }

    scenario
}

// Common validation for the load tests in this file.
fn validate_closer_test(
    mock_endpoints: &[Mock],
    goose_metrics: &GooseMetrics,
    configuration: &GooseConfiguration,
) {
    // Get the configuration that was used for building the load test.
    let test_endpoints = configure_mock_endpoints();

    // Ensure that the right paths have been called.
    for (idx, item) in test_endpoints.iter().enumerate() {
        let mock_endpoint = &mock_endpoints[idx];

        // Confirm that we loaded the mock endpoint.
        assert!(
            mock_endpoint.calls() > 0,
            "Endpoint was not called > 0 for item: {:#?}",
            item
        );
        let expect_error = format!("Item does not exist in goose_metrics: {:#?}", item);
        let endpoint_metrics = goose_metrics
            .requests
            .get(&format!("GET {}", item.path))
            .expect(&expect_error);

        assert!(
            endpoint_metrics.path == item.path,
            "{} != {} for item: {:#?}",
            endpoint_metrics.path,
            item.path,
            item
        );
        assert!(endpoint_metrics.method == GooseMethod::Get);

        // Confirm that Goose and the server saw the same number of page loads.
        let status_code: u16 = item.status_code;

        assert!(
            endpoint_metrics.raw_data.counter == mock_endpoint.calls(),
            "response_time_counter != hits() for item: {:#?}",
            item
        );
        assert!(
            endpoint_metrics.status_code_counts[&status_code] == mock_endpoint.calls(),
            "status_code_counts != hits() for item: {:#?}",
            item
        );
        assert!(
            endpoint_metrics.success_count == mock_endpoint.calls(),
            "success_count != hits() for item: {:#?}",
            item
        );
        assert!(
            endpoint_metrics.fail_count == 0,
            "fail_count != 0 for item: {:#?}",
            item
        );
    }

    // Test specific things directly access the mock endpoints here.
    let index = &mock_endpoints[0];
    let about = &mock_endpoints[1];

    // Confirm that we loaded the index roughly three times as much as the about page.
    let one_third_index = index.calls() / 3;
    let difference = about.calls() as i32 - one_third_index as i32;
    assert!((-2..=2).contains(&difference));

    // Verify that Goose started the correct number of users.
    assert!(goose_metrics.total_users == configuration.users.unwrap());
}

// Helper to run the test.
async fn run_load_test() {
    // Start mock server.
    let server = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Get configuration for building the load test itself.
    let test_endpoints = configure_mock_endpoints();

    // Build configuration.
    let configuration = common_build_configuration(&server, test_endpoints.len());

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(common::build_load_test(
        configuration.clone(),
        vec![build_scenario()],
        None,
        None,
    ))
    .await;

    // Confirm the load test ran correctly.
    validate_closer_test(&mock_endpoints, &goose_metrics, &configuration);
}

#[tokio::test]
// Load test with a single scenario containing two weighted transactions setup via closure.
// Validate weighting and statistics.
async fn test_single_scenario_closure() {
    run_load_test().await;
}
