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

/// Test test_start alone.
#[test]
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

    let mut configuration = common::build_configuration(&server);

    // Start manager instance of the load test.
    let mut master_configuration = configuration.clone();
    let master_handle = thread::spawn(move || {
        master_configuration.users = Some(2);
        master_configuration.hatch_rate = 4;
        master_configuration.manager = true;
        master_configuration.expect_workers = 1;
        master_configuration.run_time = "3".to_string();
        let _goose_attack = crate::GooseAttack::initialize_with_config(master_configuration)
            .setup()
            .unwrap()
            .register_taskset(taskset!("User1").register_task(task!(get_index)))
            .register_taskset(taskset!("User2").register_task(task!(get_about)))
            .execute()
            .unwrap();
    });

    // Start worker instance of the load test.
    let worker_handle = thread::spawn(move || {
        configuration.worker = true;
        configuration.host = "".to_string();
        configuration.users = None;
        configuration.no_stats = false;
        configuration.run_time = "".to_string();
        let _goose_attack = crate::GooseAttack::initialize_with_config(configuration)
            .setup()
            .unwrap()
            .register_taskset(taskset!("User1").register_task(task!(get_index)))
            .register_taskset(taskset!("User2").register_task(task!(get_about)))
            .execute()
            .unwrap();
    });

    // Wait for the load test to finish.
    let _ = worker_handle.join();
    let _ = master_handle.join();

    // Confirm the load test ran both tasksets.
    assert!(index.times_called() > 0);
    assert!(about.times_called() > 0);
}
