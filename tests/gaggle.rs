mod common;

use httpmock::Method::GET;
use httpmock::{mock, with_mock_server};
use std::thread;

use goose::prelude::*;

const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about.html";

pub async fn get_index(client: &GooseClient) {
    let _response = client.get(INDEX_PATH).await;
}

pub async fn get_about(client: &GooseClient) {
    let _response = client.get(ABOUT_PATH).await;
}

/// Test test_start alone.
#[test]
#[with_mock_server]
fn test_gaggle() {
    let mock_index = mock(GET, INDEX_PATH).return_status(200).create();
    let mock_about = mock(GET, ABOUT_PATH).return_status(200).create();

    let configuration = common::build_configuration();

    // Start a manager instance of the load test.
    let mut master_configuration = configuration.clone();
    let master_handle = thread::spawn(move || {
        master_configuration.clients = Some(2);
        master_configuration.hatch_rate = 4;
        master_configuration.manager = true;
        master_configuration.expect_workers = 2;
        master_configuration.run_time = "3".to_string();
        crate::GooseAttack::initialize_with_config(master_configuration)
            .setup()
            .register_taskset(
                taskset!("User1")
                    .register_task(task!(get_index))
            )
            .register_taskset(
                taskset!("User2")
                    .register_task(task!(get_about))
            )
            .execute();
    });

    // Start a manager instance of the load test.
    let mut worker1_configuration = configuration.clone();
    let worker1_handle = thread::spawn(move || {
        worker1_configuration.worker = true;
        worker1_configuration.host = "".to_string();
        worker1_configuration.clients = None;
        worker1_configuration.no_stats = false;
        worker1_configuration.run_time = "".to_string();
        crate::GooseAttack::initialize_with_config(worker1_configuration)
            .setup()
            .register_taskset(
                taskset!("User1")
                    .register_task(task!(get_index))
            )
            .register_taskset(
                taskset!("User2")
                    .register_task(task!(get_about))
            )
            .execute();
    });

    // Start a manager instance of the load test.
    let mut worker2_configuration = configuration.clone();
    let worker2_handle = thread::spawn(move || {
        worker2_configuration.worker = true;
        worker2_configuration.host = "".to_string();
        worker2_configuration.clients = None;
        worker2_configuration.no_stats = false;
        worker2_configuration.run_time = "".to_string();
        crate::GooseAttack::initialize_with_config(worker2_configuration)
            .setup()
            .register_taskset(
                taskset!("User1")
                    .register_task(task!(get_index))
            )
            .register_taskset(
                taskset!("User2")
                    .register_task(task!(get_about))
            )
            .execute();
    });

    // Wait for the load test to finish.
    let _ = worker2_handle.join();
    let _ = worker1_handle.join();
    let _ = master_handle.join();

    let called_index = mock_index.times_called();
    let called_about = mock_about.times_called();

    eprintln!("index: {}, about: {}", called_index, called_about);

    // Confirm the load test ran both users.
    assert_ne!(called_index, 0);
    assert_ne!(called_about, 0);
}
