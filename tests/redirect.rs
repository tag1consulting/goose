use httpmock::Method::GET;
use httpmock::{mock, with_mock_server};

mod common;

use goose::prelude::*;

const INDEX_PATH: &str = "/";
const REDIRECT_PATH: &str = "/redirect";
const REDIRECT2_PATH: &str = "/redirect2";
const REDIRECT3_PATH: &str = "/redirect3";
const ABOUT_PATH: &str = "/about.php";

pub async fn get_index(client: &GooseClient) -> () {
    let _response = client.get(INDEX_PATH).await;
}

pub async fn get_redirect(client: &GooseClient) -> () {
    let mut response = client.get(REDIRECT_PATH).await;
    match response.response {
        Ok(r) => match r.text().await {
            Ok(html) => {
                // Confirm that we followed redirects and loaded the about page.
                if !html.contains("about page") {
                    eprintln!("about page body wrong");
                    client.set_failure(&mut response.request);
                }
            }
            Err(e) => {
                eprintln!("unexpected error parsing about page: {}", e);
                client.set_failure(&mut response.request);
            }
        },
        // Goose will catch this error.
        Err(_) => (),
    }
}

#[test]
#[with_mock_server]
fn test_redirect() {
    let mock_index = mock(GET, INDEX_PATH).return_status(200).create();
    let mock_redirect = mock(GET, REDIRECT_PATH)
        // Moved Permanently
        .return_status(301)
        .return_header("Location", "/redirect2")
        .create();
    let mock_redirect2 = mock(GET, REDIRECT2_PATH)
        // Found (Moved Temporarily)
        .return_status(302)
        .return_header("Location", "/redirect3")
        .create();
    let mock_redirect3 = mock(GET, REDIRECT3_PATH)
        // See Other
        .return_status(303)
        .return_header("Location", "/about.php")
        .create();
    let mock_about = mock(GET, ABOUT_PATH)
        .return_status(200)
        .return_body("<HTML><BODY>about page</BODY></HTML>")
        .create();

    crate::GooseAttack::initialize_with_config(common::build_configuration())
        .setup()
        .register_taskset(
            taskset!("LoadTest")
                // Load index directly.
                .register_task(task!(get_index))
                // Load redirect path, redirect to redirect2 path, redirect to
                // redirect3 path, redirect to about.
                .register_task(task!(get_redirect)),
        )
        .execute();

    let called_index = mock_index.times_called();
    let called_redirect = mock_redirect.times_called();
    let called_redirect2 = mock_redirect2.times_called();
    let called_redirect3 = mock_redirect3.times_called();
    let called_about = mock_about.times_called();

    // Confirm that we loaded the mock endpoints; while we never load the about page
    // directly, we should follow the redirects and load it.
    assert_ne!(called_index, 0);
    assert_ne!(called_redirect, 0);
    assert_ne!(called_redirect2, 0);
    assert_ne!(called_redirect3, 0);
    assert_ne!(called_about, 0);

    // We should have called all redirects the same number of times as we called the
    // final about page.
    assert!(called_redirect == called_redirect2);
    assert!(called_redirect == called_redirect3);
    assert!(called_redirect == called_about);
}
