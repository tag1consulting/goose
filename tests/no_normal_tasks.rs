use httpmock::Method::{GET, POST};
use httpmock::{Mock, MockServer};

mod common;

use goose::prelude::*;

const LOGIN_PATH: &str = "/login";
const LOGOUT_PATH: &str = "/logout";

pub async fn login(user: &GooseUser) -> GooseTaskResult {
    let request_builder = user.goose_post(LOGIN_PATH).await?;
    let params = [("username", "me"), ("password", "s3crET!")];
    let _goose = user.goose_send(request_builder.form(&params), None).await?;
    Ok(())
}

pub async fn logout(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(LOGOUT_PATH).await?;
    Ok(())
}

#[test]
fn test_no_normal_tasks() {
    let server = MockServer::start();

    let login_path = Mock::new()
        .expect_method(POST)
        .expect_path(LOGIN_PATH)
        .return_status(200)
        .create_on(&server);
    let logout_path = Mock::new()
        .expect_method(GET)
        .expect_path(LOGOUT_PATH)
        .return_status(200)
        .create_on(&server);

    let _goose_stats = crate::GooseAttack::initialize_with_config(common::build_configuration(
        &server,
        vec!["--no-metrics"],
    ))
    .unwrap()
    .setup()
    .unwrap()
    .register_taskset(
        taskset!("LoadTest")
            .register_task(task!(login).set_on_start())
            .register_task(task!(logout).set_on_stop()),
    )
    .execute()
    .unwrap();

    // Confirm that the on_start and on_exit tasks actually ran.
    assert!(login_path.times_called() == 1);
    assert!(logout_path.times_called() == 1);
}
