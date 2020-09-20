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

#[test]
// Load test with a single task set containing two weighted tasks. Validate
// weighting and statistics.
fn test_single_taskset() {
    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server);
    let about = Mock::new()
        .expect_method(GET)
        .expect_path(ABOUT_PATH)
        .return_status(200)
        .create_on(&server);

    let config = common::build_configuration(
        &server,
        vec![
            "--users",
            "2",
            // Start users in .5 seconds.
            "--hatch-rate",
            "4",
            "--status-codes",
            "--no-reset-metrics",
        ],
    );
    let goose_metrics = crate::GooseAttack::initialize_with_config(config.clone())
        .unwrap()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_index).set_weight(9).unwrap())
                .register_task(task!(get_about).set_weight(3).unwrap()),
        )
        .execute()
        .unwrap();

    // Confirm that we loaded the mock endpoints.
    assert!(index.times_called() > 0);
    assert!(about.times_called() > 0);

    // Confirm that we loaded the index roughly three times as much as the about page.
    let one_third_index = index.times_called() / 3;
    let difference = about.times_called() as i32 - one_third_index as i32;
    assert!(difference >= -2 && difference <= 2);

    let index_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", INDEX_PATH))
        .unwrap();
    let about_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", ABOUT_PATH))
        .unwrap();

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
    assert!(goose_metrics.users == config.users.unwrap());
}

#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Spawn a gaggle of 1 manager and 2 workers each simulating one user. Run a load test,
// synchronize metrics from the workers to the manager, and validate that Goose tracked
// the same metrics as the mock server.
fn test_single_taskset_gaggle() {
    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server);
    let about = Mock::new()
        .expect_method(GET)
        .expect_path(ABOUT_PATH)
        .return_status(200)
        .create_on(&server);

    // Launch workers in their own threads, storing the thread handle.
    let mut worker_handles = Vec::new();
    // Each worker has the same identical configuration.
    let worker_configuration = common::build_configuration(&server, vec!["--worker"]);

    for _ in 0..2 {
        let configuration = worker_configuration.clone();
        // Start worker instance of the load test.
        worker_handles.push(thread::spawn(move || {
            let _goose_metrics = crate::GooseAttack::initialize_with_config(configuration)
                .unwrap()
                .register_taskset(
                    taskset!("LoadTest")
                        .register_task(task!(get_index).set_weight(9).unwrap())
                        .register_task(task!(get_about).set_weight(3).unwrap()),
                )
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
            "2",
            // Start users in .5 seconds.
            "--hatch-rate",
            "4",
            // Verify that metrics are merged correctly on the manager.
            "--status-codes",
            "--no-reset-metrics",
        ],
    );
    let goose_metrics = crate::GooseAttack::initialize_with_config(manager_configuration.clone())
        .unwrap()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_index).set_weight(9).unwrap())
                .register_task(task!(get_about).set_weight(3).unwrap()),
        )
        .execute()
        .unwrap();

    // Wait for both worker threads to finish and exit.
    for worker_handle in worker_handles {
        let _ = worker_handle.join();
    }

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

#[test]
// Load test with a single task set containing two weighted tasks. Validate
// weighting and statistics after resetting metrics.
fn test_single_taskset_reset_metrics() {
    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server);
    let about = Mock::new()
        .expect_method(GET)
        .expect_path(ABOUT_PATH)
        .return_status(200)
        .create_on(&server);

    let config = common::build_configuration(
        &server,
        vec![
            "--no-task-metrics",
            "--status-codes",
            "--users",
            "2",
            "--hatch-rate",
            "4",
        ],
    );
    let goose_metrics = crate::GooseAttack::initialize_with_config(config.clone())
        .unwrap()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_index).set_weight(9).unwrap())
                .register_task(task!(get_about).set_weight(3).unwrap()),
        )
        .execute()
        .unwrap();

    // Confirm that we loaded the mock endpoints.
    assert!(index.times_called() > 0);
    assert!(about.times_called() > 0);

    // Confirm that we loaded the index roughly three times as much as the about page.
    let one_third_index = index.times_called() / 3;
    let difference = about.times_called() as i32 - one_third_index as i32;
    assert!(difference >= -2 && difference <= 2);

    let index_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", INDEX_PATH))
        .unwrap();
    let about_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", ABOUT_PATH))
        .unwrap();

    // Confirm that the path and method are correct in the statistics.
    assert!(index_metrics.path == INDEX_PATH);
    assert!(index_metrics.method == GooseMethod::GET);
    assert!(about_metrics.path == ABOUT_PATH);
    assert!(about_metrics.method == GooseMethod::GET);

    // Confirm that Goose saw fewer page loads than the server, as the statistics
    // were reset after .5 seconds.
    let status_code: u16 = 200;
    assert!(index_metrics.response_time_counter < index.times_called());
    assert!(index_metrics.status_code_counts[&status_code] < index.times_called());
    assert!(index_metrics.success_count < index.times_called());
    assert!(index_metrics.fail_count == 0);
    assert!(about_metrics.response_time_counter < about.times_called());
    assert!(about_metrics.status_code_counts[&status_code] < about.times_called());
    assert!(about_metrics.success_count < about.times_called());
    assert!(about_metrics.fail_count == 0);

    // Verify that Goose started the correct number of users.
    assert!(goose_metrics.users == config.users.unwrap());
}
