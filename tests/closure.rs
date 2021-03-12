use httpmock::{Method::GET, MockRef, MockServer};
use std::sync::Arc;

mod common;

use goose::goose::GooseMethod;
use goose::prelude::*;
use goose::GooseConfiguration;

// Paths used in load tests performed during these tests.
const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about.html";

// Load test configuration.
const EXPECT_WORKERS: usize = 2;

// Test task.
pub async fn get_index(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// Test task.
pub async fn get_about(user: &GooseUser) -> GooseTaskResult {
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
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<MockRef> {
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
fn common_build_configuration(
    server: &MockServer,
    users: usize,
    worker: Option<bool>,
    manager: Option<usize>,
) -> GooseConfiguration {
    if let Some(expect_workers) = manager {
        common::build_configuration(
            &server,
            vec![
                "--manager",
                "--expect-workers",
                &expect_workers.to_string(),
                "--no-reset-metrics",
                "--no-task-metrics",
                "--status-codes",
                "--users",
                &users.to_string(),
                "--hatch-rate",
                &(users * 2).to_string(),
            ],
        )
    } else if worker.is_some() {
        common::build_configuration(&server, vec!["--worker"])
    } else {
        common::build_configuration(
            &server,
            vec![
                "--no-reset-metrics",
                "--no-task-metrics",
                "--status-codes",
                "--users",
                &users.to_string(),
                "--hatch-rate",
                &(users * 2).to_string(),
            ],
        )
    }
}

// Dynamically build taskset.
fn build_taskset() -> GooseTaskSet {
    // Get common configuration for building endpoints and the load test itself.
    let test_endpoints = configure_mock_endpoints();

    let mut taskset = GooseTaskSet::new("LoadTest");
    for item in &test_endpoints {
        let path = item.path;
        let weight = item.weight;

        let closure: GooseTaskFunction = Arc::new(move |user| {
            Box::pin(async move {
                let _goose = user.get(path).await?;

                Ok(())
            })
        });

        let task = GooseTask::new(closure).set_weight(weight).unwrap();
        // We need to do the variable dance as taskset.register_task returns self and hence moves
        // self out of `taskset`. By storing it in a new local variable and then moving it over
        // we can avoid that error.
        let new_taskset = taskset.register_task(task);
        taskset = new_taskset;
    }

    taskset
}

// Common validation for the load tests in this file.
fn validate_closer_test(
    mock_endpoints: &[MockRef],
    goose_metrics: &GooseMetrics,
    configuration: &GooseConfiguration,
) {
    // Get the configuration that was used for building the load test.
    let test_endpoints = configure_mock_endpoints();

    // Ensure that the right paths have been called.
    for (idx, item) in test_endpoints.iter().enumerate() {
        let mock_endpoint = &mock_endpoints[idx];

        let format_item = |message, assert_item| {
            return format!("{} for item = {:#?}", message, assert_item);
        };

        // Confirm that we loaded the mock endpoint.
        assert!(
            mock_endpoint.hits() > 0,
            format_item("Endpoint was not called > 0", &item)
        );
        let expect_error = format_item("Item does not exist in goose_metrics", &item);
        let endpoint_metrics = goose_metrics
            .requests
            .get(&format!("GET {}", item.path))
            .expect(&expect_error);

        assert!(
            endpoint_metrics.path == item.path,
            format_item(
                &format!("{} != {}", endpoint_metrics.path, item.path),
                &item
            )
        );
        assert!(endpoint_metrics.method == GooseMethod::GET);

        // Confirm that Goose and the server saw the same number of page loads.
        let status_code: u16 = item.status_code;

        assert!(
            endpoint_metrics.response_time_counter == mock_endpoint.hits(),
            format_item("response_time_counter != hits()", &item)
        );
        assert!(
            endpoint_metrics.status_code_counts[&status_code] == mock_endpoint.hits(),
            format_item("status_code_counts != hits()", &item)
        );
        assert!(
            endpoint_metrics.success_count == mock_endpoint.hits(),
            format_item("success_count != hits()", &item)
        );
        assert!(
            endpoint_metrics.fail_count == 0,
            format_item("fail_count != 0", &item)
        );
    }

    // Test specific things directly access the mock endpoints here.
    let index = &mock_endpoints[0];
    let about = &mock_endpoints[1];

    // Confirm that we loaded the index roughly three times as much as the about page.
    let one_third_index = index.hits() / 3;
    let difference = about.hits() as i32 - one_third_index as i32;
    assert!(difference >= -2 && difference <= 2);

    // Verify that Goose started the correct number of users.
    assert!(goose_metrics.users == configuration.users.unwrap());
}

// Helper to run the test, takes a flag for indicating if running in standalone
// mode or Gaggle mode.
fn run_load_test(is_gaggle: bool) {
    // Start mock server.
    let server = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Get configuration for building the load test itself.
    let test_endpoints = configure_mock_endpoints();

    let (configuration, goose_metrics) = match is_gaggle {
        false => {
            // Build configuration.
            let configuration =
                common_build_configuration(&server, test_endpoints.len(), None, None);

            // Run the Goose Attack.
            let goose_metrics = common::run_load_test(
                common::build_load_test(configuration.clone(), &build_taskset(), None, None),
                None,
            );

            (configuration, goose_metrics)
        }
        true => {
            // Each worker has the same identical configuration.
            let worker_configuration =
                common_build_configuration(&server, test_endpoints.len(), Some(true), None);

            // Workers launched in own threads, store thread handles.
            let worker_handles = common::launch_gaggle_workers(
                common::build_load_test(worker_configuration, &build_taskset(), None, None),
                EXPECT_WORKERS,
            );

            // Build Manager configuration.
            let manager_configuration = common_build_configuration(
                &server,
                test_endpoints.len(),
                None,
                Some(EXPECT_WORKERS),
            );

            // Run the Goose Attack.
            let goose_metrics = common::run_load_test(
                common::build_load_test(
                    manager_configuration.clone(),
                    &build_taskset(),
                    None,
                    None,
                ),
                Some(worker_handles),
            );

            (manager_configuration, goose_metrics)
        }
    };

    // Confirm the load test ran correctly.
    validate_closer_test(&mock_endpoints, &goose_metrics, &configuration);
}

#[test]
// Load test with a single task set containing two weighted tasks setup via closure.
// Validate weighting and statistics.
fn test_single_taskset_closure() {
    // Run load test with is_gaggle set to false.
    run_load_test(false);
}

#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Spawn a gaggle of 1 manager and 2 workers each simulating one user. Run a load test,
// with a single task set containing two weighted tasks setup via closure. Validate
// that weighting and metrics are correctly merged to the Manager.
fn test_single_taskset_closure_gaggle() {
    // Run load test with is_gaggle set to true.
    run_load_test(true);
}
