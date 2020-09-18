use httpmock::Method::GET;
use httpmock::{Mock, MockServer};

mod common;

use goose::goose::GooseMethod;
use goose::prelude::*;
use std::thread;

const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about.html";

pub async fn get_index(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

pub async fn get_about(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(ABOUT_PATH).await?;
    Ok(())
}

// @todo Move out to common.rs.
#[derive(Debug)]
struct LoadtestEndpoint<'a> {
    pub path: &'a str,
    pub status_code: u16,
    pub weight: usize,
}

#[test]
// Load test with a single task set containing two weighted tasks setup via closure.
// Validate weighting and statistics.
fn test_single_taskset_closure() {
    use std::sync::Arc;

    // Configure endpoints to test.
    let test_endpoints = vec![
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
    ];

    // Start mock server.
    let server = MockServer::start();

    // Build configuration.
    let config = common::build_configuration(
        &server,
        vec![
            "--no-reset-metrics",
            "--no-task-metrics",
            "--status-codes",
            "--users",
            &test_endpoints.len().to_string(),
            "--hatch-rate",
            &(2 * test_endpoints.len()).to_string(),
        ],
    );

    // Setup mock endpoints.
    let mut mock_endpoints = Vec::with_capacity(test_endpoints.len());
    for (idx, item) in test_endpoints.iter().enumerate() {
        let path = item.path;
        let mock_endpoint = Mock::new()
            .expect_method(GET)
            .expect_path(path)
            .return_status(item.status_code.into())
            .create_on(&server);

        // Ensure the index matches.
        assert!(idx == mock_endpoints.len());
        mock_endpoints.push(mock_endpoint);
    }

    // Build dynamic taskset.
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

    // Run the loadtest.
    let goose_metrics = crate::GooseAttack::initialize_with_config(config.clone())
        .unwrap()
        .register_taskset(taskset)
        .execute()
        .unwrap();

    // Ensure that the right paths have been called.
    for (idx, item) in test_endpoints.iter().enumerate() {
        let mock_endpoint = &mock_endpoints[idx];

        let format_item = |message, assert_item| {
            return format!("{} for item = {:#?}", message, assert_item);
        };

        // Confirm that we loaded the mock endpoint.
        assert!(
            mock_endpoint.times_called() > 0,
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
            endpoint_metrics.response_time_counter == mock_endpoint.times_called(),
            format_item("response_time_counter != times_called()", &item)
        );
        assert!(
            endpoint_metrics.status_code_counts[&status_code] == mock_endpoint.times_called(),
            format_item("status_code_counts != times_called()", &item)
        );
        assert!(
            endpoint_metrics.success_count == mock_endpoint.times_called(),
            format_item("success_count != times_called()", &item)
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
    let one_third_index = index.times_called() / 3;
    let difference = about.times_called() as i32 - one_third_index as i32;
    assert!(difference >= -2 && difference <= 2);

    // Verify that Goose started the correct number of users.
    assert!(goose_metrics.users == config.users.unwrap());
}

#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Spawn a gaggle of 1 manager and 2 workers each simulating one user. Run a load test,
// with a single task set containing two weighted tasks setup via closure. Validate
// that weighting and metrics are correctly merged to the Manager.
fn test_single_taskset_closure_gaggle() {
    use std::sync::Arc;

    // Configure endpoints to test.
    let test_endpoints = vec![
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
    ];

    // Start mock server.
    let server = MockServer::start();

    // Setup mock endpoints.
    let mut mock_endpoints = Vec::with_capacity(test_endpoints.len());
    for (idx, item) in test_endpoints.iter().enumerate() {
        let path = item.path;
        let mock_endpoint = Mock::new()
            .expect_method(GET)
            .expect_path(path)
            .return_status(item.status_code.into())
            .create_on(&server);

        // Ensure the index matches.
        assert!(idx == mock_endpoints.len());
        mock_endpoints.push(mock_endpoint);
    }

    // Launch workers in their own threads, storing the thread handle.
    let mut worker_handles = Vec::new();
    // Each worker has the same identical configuration.
    let mut worker_configuration = common::build_configuration(&server, vec!["--worker"]);

    // Unset options set in common.rs as they can't be set on the Worker.
    worker_configuration.users = None;
    worker_configuration.run_time = "".to_string();
    worker_configuration.hatch_rate = None;

    // Build dynamic taskset.
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

    for _ in 0..2 {
        // Clone configuration and taskset for each Worker.
        let worker_config = worker_configuration.clone();
        let worker_taskset = taskset.clone();
        // Start worker instance of the load test.
        worker_handles.push(thread::spawn(move || {
            // Run the loadtest.
            let _goose_metrics = crate::GooseAttack::initialize_with_config(worker_config)
                .unwrap()
                .register_taskset(worker_taskset)
                .execute()
                .unwrap();
        }));
    }

    // Start manager instance in current thread and run a distributed load test.
    let manager_configuration = common::build_configuration(
        &server,
        vec![
            "--manager",
            "--expect-workers",
            "2",
            "--run-time",
            "3",
            "--users",
            &test_endpoints.len().to_string(),
            // Start users in .5 seconds.
            "--hatch-rate",
            &(2 * test_endpoints.len()).to_string(),
            // Verify that metrics are merged correctly on the manager.
            "--status-codes",
            "--no-reset-metrics",
            "--no-task-metrics",
        ],
    );

    // Run the loadtest.
    let goose_metrics = crate::GooseAttack::initialize_with_config(manager_configuration.clone())
        .unwrap()
        .register_taskset(taskset)
        .execute()
        .unwrap();

    // Wait for both worker threads to finish and exit.
    for worker_handle in worker_handles {
        let _ = worker_handle.join();
    }

    // Ensure that the right paths have been called.
    for (idx, item) in test_endpoints.iter().enumerate() {
        let mock_endpoint = &mock_endpoints[idx];

        let format_item = |message, assert_item| {
            return format!("{} for item = {:#?}", message, assert_item);
        };

        // Confirm that we loaded the mock endpoint.
        assert!(
            mock_endpoint.times_called() > 0,
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
            endpoint_metrics.response_time_counter == mock_endpoint.times_called(),
            format_item("response_time_counter != times_called()", &item)
        );
        assert!(
            endpoint_metrics.status_code_counts[&status_code] == mock_endpoint.times_called(),
            format_item("status_code_counts != times_called()", &item)
        );
        assert!(
            endpoint_metrics.success_count == mock_endpoint.times_called(),
            format_item("success_count != times_called()", &item)
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
    let one_third_index = index.times_called() / 3;
    let difference = about.times_called() as i32 - one_third_index as i32;
    assert!(difference >= -2 && difference <= 2);

    // Verify that Goose started the correct number of users.
    assert!(goose_metrics.users == manager_configuration.users.unwrap());

    // Confirm the load test ran both task sets.
    assert!(index.times_called() > 0);
    assert!(about.times_called() > 0);

    // Validate request metrics.
    let index_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", INDEX_PATH))
        .unwrap();
    let about_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", ABOUT_PATH))
        .unwrap();
    assert!(index_metrics.response_time_counter == index.times_called());
    assert!(about_metrics.response_time_counter == about.times_called());

    // Confirm that the path and method are correct in the statistics.
    assert!(index_metrics.path == INDEX_PATH);
    assert!(index_metrics.method == GooseMethod::GET);
    assert!(about_metrics.path == ABOUT_PATH);
    assert!(about_metrics.method == GooseMethod::GET);

    // Confirm that Goose and the server saw the same number of page loads.
    let status_code: u16 = 200;
    assert!(index_metrics.response_time_counter == index.times_called());
    assert!(index_metrics.status_code_counts[&status_code] == index.times_called());
    assert!(index_metrics.success_count == index.times_called());
    assert!(index_metrics.fail_count == 0);
    assert!(about_metrics.response_time_counter == about.times_called());
    assert!(about_metrics.status_code_counts[&status_code] == about.times_called());
    assert!(about_metrics.success_count == about.times_called());
    assert!(about_metrics.fail_count == 0);

    // Verify that Goose started the correct number of users.
    assert!(goose_metrics.users == manager_configuration.users.unwrap());
}
