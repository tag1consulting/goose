use httpmock::Method::{GET, POST};
use httpmock::{mock, with_mock_server};

mod common;

use goose::prelude::*;

const LOGIN_PATH: &str = "/login";
const LOGOUT_PATH: &str = "/logout";

pub async fn login(user: &GooseUser) {
    let request_builder = user.goose_post(LOGIN_PATH).await;
    let params = [("username", "me"), ("password", "s3crET!")];
    let _response = user.goose_send(request_builder.form(&params), None).await;
}

pub async fn logout(user: &GooseUser) {
    let _response = user.get(LOGOUT_PATH).await;
}

#[test]
#[with_mock_server]
fn test_no_normal_tasks() {
    let mock_login = mock(POST, LOGIN_PATH).return_status(200).create();
    let mock_logout = mock(GET, LOGOUT_PATH).return_status(200).create();

    crate::GooseAttack::initialize_with_config(common::build_configuration())
        .setup()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(login).set_on_start())
                .register_task(task!(logout).set_on_stop()),
        )
        .execute();

    let called_login = mock_login.times_called();
    let called_logout = mock_logout.times_called();

    // Confirm that the on_start and on_exit tasks actually ran.
    assert_eq!(called_login, 1);
    assert_eq!(called_logout, 1);
}
