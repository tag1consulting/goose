mod common;

use httpmock::Method::GET;
use httpmock::{Mock, MockServer};
use std::thread;

use goose::prelude::*;

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
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Spawn a gaggle of 1 manager and 2 workers each simulating one user. Run a load test,
// synchronize statistics from the workers to the manager, and validate that Goose tracked
// the same statistics as the mock server.
fn test_gaggle() {
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
                .setup()
                .unwrap()
                .register_taskset(taskset!("User1").register_task(task!(get_index)))
                .register_taskset(taskset!("User2").register_task(task!(get_about)))
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
            "--users",
            "2",
            "--hatch-rate",
            "4",
            "--run-time",
            "3",
            // Enable statistics so we can validate they are merged to the manager correctly.
            "--no-reset-metrics",
        ],
    );
    let goose_metrics = crate::GooseAttack::initialize_with_config(manager_configuration)
        .unwrap()
        .setup()
        .unwrap()
        .register_taskset(taskset!("User1").register_task(task!(get_index)))
        .register_taskset(taskset!("User2").register_task(task!(get_about)))
        .execute()
        .unwrap();

    // Wait for both worker threads to finish and exit.
    for worker_handle in worker_handles {
        let _ = worker_handle.join();
    }

    // Confirm the load test ran both task sets.
    assert!(index.times_called() > 0);
    assert!(about.times_called() > 0);

    // Validate task metrics.
    assert!(goose_metrics.tasks[0][0].counter == index.times_called());
    assert!(goose_metrics.tasks[1][0].counter == about.times_called());

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
}
