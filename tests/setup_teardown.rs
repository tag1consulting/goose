use httpmock::Method::{GET, POST};
use httpmock::{Mock, MockServer};

mod common;

use goose::prelude::*;

const INDEX_PATH: &str = "/";
const SETUP_PATH: &str = "/setup";
const TEARDOWN_PATH: &str = "/teardown";

pub async fn setup(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.post(SETUP_PATH, "setting up load test").await?;
    Ok(())
}

pub async fn teardown(user: &GooseUser) -> GooseTaskResult {
    let _goose = user
        .post(TEARDOWN_PATH, "cleaning up after load test")
        .await?;
    Ok(())
}

pub async fn get_index(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

/// Test test_start alone.
#[test]
fn test_start() {
    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(201)
        .create_on(&server);
    let setup_path = Mock::new()
        .expect_method(POST)
        .expect_path(SETUP_PATH)
        .return_status(205)
        .create_on(&server);
    let teardown_path = Mock::new()
        .expect_method(POST)
        .expect_path(TEARDOWN_PATH)
        .return_status(200)
        .create_on(&server);

    let _goose_stats =
        crate::GooseAttack::initialize_with_config(common::build_configuration(&server))
            .unwrap()
            .setup()
            .unwrap()
            .test_start(task!(setup))
            .register_taskset(
                taskset!("LoadTest").register_task(task!(get_index).set_weight(9).unwrap()),
            )
            .execute()
            .unwrap();

    // Confirm the load test ran.
    assert!(index.times_called() > 0);

    // Confirm we ran setup one time.
    assert!(setup_path.times_called() == 1);

    // Confirm we did not run the teardown.
    assert!(teardown_path.times_called() == 0);
}

/// Test test_stop alone.
#[test]
fn test_stop() {
    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(201)
        .create_on(&server);
    let setup_path = Mock::new()
        .expect_method(POST)
        .expect_path(SETUP_PATH)
        .return_status(205)
        .create_on(&server);
    let teardown_path = Mock::new()
        .expect_method(POST)
        .expect_path(TEARDOWN_PATH)
        .return_status(200)
        .create_on(&server);

    let _goose_stats =
        crate::GooseAttack::initialize_with_config(common::build_configuration(&server))
            .unwrap()
            .setup()
            .unwrap()
            .test_stop(task!(teardown))
            .register_taskset(
                taskset!("LoadTest").register_task(task!(get_index).set_weight(9).unwrap()),
            )
            .execute()
            .unwrap();

    // Confirm the load test ran.
    assert!(index.times_called() > 0);

    // Confirm we did not run setup.
    assert!(setup_path.times_called() == 0);

    // Confirm we ran the teardown 1 time.
    assert!(teardown_path.times_called() == 1);
}

#[test]
fn test_setup_teardown() {
    let server = MockServer::start();

    let index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(201)
        .create_on(&server);
    let setup_path = Mock::new()
        .expect_method(POST)
        .expect_path(SETUP_PATH)
        .return_status(205)
        .create_on(&server);
    let teardown_path = Mock::new()
        .expect_method(POST)
        .expect_path(TEARDOWN_PATH)
        .return_status(200)
        .create_on(&server);

    let mut configuration = common::build_configuration(&server);
    // Launch several user threads, confirm we still only setup and teardown one time.
    configuration.users = Some(5);
    configuration.hatch_rate = 5;

    let _goose_stats = crate::GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .setup()
        .unwrap()
        .test_start(task!(setup))
        .register_taskset(
            taskset!("LoadTest").register_task(task!(get_index).set_weight(9).unwrap()),
        )
        .test_stop(task!(teardown))
        .execute()
        .unwrap();

    // Confirm the load test ran.
    assert!(index.times_called() != 0);

    // Confirm we ran setup one time.
    assert!(setup_path.times_called() == 1);

    // Confirm we ran teardown one time.
    assert!(teardown_path.times_called() == 1);
}
