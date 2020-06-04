use httpmock::Method::GET;
use httpmock::{mock, with_mock_server};

mod common;

use goose::prelude::*;

pub async fn get_index(client: &GooseClient) -> () {
    let _response = client.get("/").await;
}

pub async fn get_about(client: &GooseClient) -> () {
    let _response = client.get("/about.html").await;
}

#[test]
#[with_mock_server]
fn test_single_taskset() {
    let mock_index = mock(GET, "/").return_status(200).create();
    let mock_about = mock(GET, "/about.html")
        .return_status(200)
        .return_body("<HTML><BODY>about page</BODY></HTML>")
        .create();

    crate::GooseAttack::initialize_with_config(common::build_configuration())
        .setup()
        .register_taskset(
            taskset!("LoadTest")
                .register_task(task!(get_index).set_weight(9))
                .register_task(task!(get_about).set_weight(3)),
        )
        .execute();

    let called_index = mock_index.times_called();
    let called_about = mock_about.times_called();

    // Confirm that we loaded the mock endpoints.
    assert_ne!(called_index, 0);
    assert_ne!(called_about, 0);

    // Confirm that we loaded the index roughly three times as much as the about page.
    let one_third_index = called_index / 3;
    let difference = called_about as i32 - one_third_index as i32;
    assert!(difference >= -2 && difference <= 2);
}
