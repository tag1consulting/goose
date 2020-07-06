use httpmock::Method::{GET, POST};
use httpmock::{mock, with_mock_server};

mod common;

use goose::prelude::*;

const INDEX_PATH: &str = "/";
const SETUP_PATH: &str = "/setup";
const TEARDOWN_PATH: &str = "/teardown";

pub async fn setup(user: &GooseUser) {
    let _response = user.post(SETUP_PATH, "setting up load test").await;
}

pub async fn teardown(user: &GooseUser) {
    let _response = user
        .post(TEARDOWN_PATH, "cleaning up after load test")
        .await;
}

pub async fn get_index(user: &GooseUser) {
    let _response = user.get(INDEX_PATH).await;
}

/// Test test_start alone.
#[test]
#[with_mock_server]
fn test_start() {
    let mock_setup = mock(POST, SETUP_PATH).return_status(201).create();
    let mock_teardown = mock(POST, TEARDOWN_PATH).return_status(205).create();
    let mock_index = mock(GET, INDEX_PATH).return_status(200).create();

    crate::GooseAttack::initialize_with_config(common::build_configuration())
        .setup()
        .test_start(GooseTask::new(setup))
        .register_taskset(GooseTaskSet::new("LoadTest").register_task(GooseTask::new(get_index).set_weight(9)))
        .execute();

    let called_setup = mock_setup.times_called();
    let called_index = mock_index.times_called();
    let called_teardown = mock_teardown.times_called();

    // Confirm the load test ran.
    assert_ne!(called_index, 0);

    // Confirm we ran setup one time.
    assert_eq!(called_setup, 1);

    // Confirm we did not run the teardown.
    assert_eq!(called_teardown, 0);
}

/// Test test_stop alone.
#[test]
#[with_mock_server]
fn test_stop() {
    let mock_setup = mock(POST, SETUP_PATH).return_status(201).create();
    let mock_teardown = mock(POST, TEARDOWN_PATH).return_status(205).create();
    let mock_index = mock(GET, INDEX_PATH).return_status(200).create();

    crate::GooseAttack::initialize_with_config(common::build_configuration())
        .setup()
        .test_stop(GooseTask::new(teardown))
        .register_taskset(GooseTaskSet::new("LoadTest").register_task(GooseTask::new(get_index).set_weight(9)))
        .execute();

    let called_setup = mock_setup.times_called();
    let called_index = mock_index.times_called();
    let called_teardown = mock_teardown.times_called();

    // Confirm the load test ran.
    assert_ne!(called_index, 0);

    // Confirm we did not run setup.
    assert_eq!(called_setup, 0);

    // Confirm we ran the teardown 1 time.
    assert_eq!(called_teardown, 1);
}

#[test]
#[with_mock_server]
fn test_setup_teardown() {
    let mock_setup = mock(POST, SETUP_PATH).return_status(201).create();
    let mock_teardown = mock(POST, TEARDOWN_PATH).return_status(205).create();
    let mock_index = mock(GET, INDEX_PATH).return_status(200).create();

    let mut configuration = common::build_configuration();
    // Launch several user threads, confirm we still only setup and teardown one time.
    configuration.users = Some(5);
    configuration.hatch_rate = 5;

    crate::GooseAttack::initialize_with_config(configuration)
        .setup()
        .test_start(GooseTask::new(setup))
        .register_taskset(GooseTaskSet::new("LoadTest").register_task(GooseTask::new(get_index).set_weight(9)))
        .test_stop(GooseTask::new(teardown))
        .execute();

    let called_setup = mock_setup.times_called();
    let called_index = mock_index.times_called();
    let called_teardown = mock_teardown.times_called();

    // Confirm the load test ran.
    assert_ne!(called_index, 0);

    // Confirm we ran setup one time.
    assert_eq!(called_setup, 1);

    // Confirm we ran teardown one time.
    assert_eq!(called_teardown, 1);
}
