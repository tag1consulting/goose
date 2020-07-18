mod common;

use httpmock::Method::GET;
use httpmock::{mock, with_mock_server};
use std::thread;

use goose::prelude::*;

const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about.html";

pub async fn get_index(user: &GooseUser) -> Result<(), ()> {
    let _goose = user.get(INDEX_PATH).await;
    Ok(())
}

pub async fn get_about(user: &GooseUser) -> Result<(), ()> {
    let _goose = user.get(ABOUT_PATH).await;
    Ok(())
}

/// Test test_start alone.
#[test]
#[with_mock_server]
fn test_gaggle() {
    let mock_index = mock(GET, INDEX_PATH).return_status(200).create();
    let mock_about = mock(GET, ABOUT_PATH).return_status(200).create();

    let mut configuration = common::build_configuration();

    // Start manager instance of the load test.
    let mut master_configuration = configuration.clone();
    let master_handle = thread::spawn(move || {
        master_configuration.users = Some(2);
        master_configuration.hatch_rate = 4;
        master_configuration.manager = true;
        master_configuration.expect_workers = 1;
        master_configuration.run_time = "3".to_string();
        crate::GooseAttack::initialize_with_config(master_configuration)
            .setup()
            .register_taskset(taskset!("User1").register_task(task!(get_index)))
            .register_taskset(taskset!("User2").register_task(task!(get_about)))
            .execute();
    });

    // Start worker instance of the load test.
    let worker_handle = thread::spawn(move || {
        configuration.worker = true;
        configuration.host = "".to_string();
        configuration.users = None;
        configuration.no_stats = false;
        configuration.run_time = "".to_string();
        crate::GooseAttack::initialize_with_config(configuration)
            .setup()
            .register_taskset(taskset!("User1").register_task(task!(get_index)))
            .register_taskset(taskset!("User2").register_task(task!(get_about)))
            .execute();
    });

    // Wait for the load test to finish.
    let _ = worker_handle.join();
    let _ = master_handle.join();

    let called_index = mock_index.times_called();
    let called_about = mock_about.times_called();

    // Confirm the load test ran both tasksets.
    assert_ne!(called_index, 0);
    assert_ne!(called_about, 0);
}
