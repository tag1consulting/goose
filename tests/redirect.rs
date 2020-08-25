use httpmock::Method::GET;
use httpmock::{Mock, MockServer};

mod common;

use goose::prelude::*;

const INDEX_PATH: &str = "/";
const REDIRECT_PATH: &str = "/redirect";
const REDIRECT2_PATH: &str = "/redirect2";
const REDIRECT3_PATH: &str = "/redirect3";
const ABOUT_PATH: &str = "/about.php";

// Task function, load INDEX_PATH.
pub async fn get_index(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// Task function, load ABOUT PATH
pub async fn get_about(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(ABOUT_PATH).await?;
    Ok(())
}

// Task function, load REDRECT_PATH and follow redirects to ABOUT_PATH.
pub async fn get_redirect(user: &GooseUser) -> GooseTaskResult {
    let mut goose = user.get(REDIRECT_PATH).await?;

    if let Ok(r) = goose.response {
        match r.text().await {
            Ok(html) => {
                // Confirm that we followed redirects and loaded the about page.
                if !html.contains("about page") {
                    return user.set_failure(
                        "about page body wrong",
                        &mut goose.request,
                        None,
                        None,
                    );
                }
            }
            Err(e) => {
                return user.set_failure(
                    format!("unexpected error parsing about page: {}", e).as_str(),
                    &mut goose.request,
                    None,
                    None,
                );
            }
        }
    }
    Ok(())
}

// Task function, load REDRECT_PATH and follow redirect to new domain.
pub async fn get_domain_redirect(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(REDIRECT_PATH).await?;
    Ok(())
}

#[test]
/// Simulate a load test which includes a page with a redirect chain, confirms
/// all redirects are correctly followed.
fn test_redirect() {
    let server1 = MockServer::start();

    let server1_index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server1);
    let server1_redirect = Mock::new()
        .expect_method(GET)
        .expect_path(REDIRECT_PATH)
        .return_status(301)
        .return_header("Location", REDIRECT2_PATH)
        .create_on(&server1);
    let server1_redirect2 = Mock::new()
        .expect_method(GET)
        .expect_path(REDIRECT2_PATH)
        .return_status(302)
        .return_header("Location", REDIRECT3_PATH)
        .create_on(&server1);
    let server1_redirect3 = Mock::new()
        .expect_method(GET)
        .expect_path(REDIRECT3_PATH)
        .return_status(303)
        .return_header("Location", ABOUT_PATH)
        .create_on(&server1);
    let server1_about = Mock::new()
        .expect_method(GET)
        .expect_path(ABOUT_PATH)
        .return_status(200)
        .return_body("<HTML><BODY>about page</BODY></HTML>")
        .create_on(&server1);

    let _goose_stats =
        crate::GooseAttack::initialize_with_config(common::build_configuration(&server1))
            .unwrap()
            .setup()
            .unwrap()
            .register_taskset(
                taskset!("LoadTest")
                    // Load index directly.
                    .register_task(task!(get_index))
                    // Load redirect path, redirect to redirect2 path, redirect to
                    // redirect3 path, redirect to about.
                    .register_task(task!(get_redirect)),
            )
            .execute()
            .unwrap();

    // Confirm that we loaded the mock endpoints; while we never load the about page
    // directly, we should follow the redirects and load it.
    assert!(server1_index.times_called() > 0);
    assert!(server1_redirect.times_called() > 0);
    assert!(server1_redirect2.times_called() > 0);
    assert!(server1_redirect3.times_called() > 0);
    assert!(server1_about.times_called() > 0);

    // We should have called all redirects the same number of times as we called the
    // final about page.
    assert!(server1_redirect.times_called() == server1_redirect2.times_called());
    assert!(server1_redirect.times_called() == server1_redirect3.times_called());
    assert!(server1_redirect.times_called() == server1_about.times_called());
}

#[test]
/// Simulate a load test which includes a page with a redirect to another domain
/// (which in this case is a second mock server running on a different path).
/// all redirects are correctly followed.
fn test_domain_redirect() {
    let server1 = MockServer::start();
    let server2 = MockServer::start();

    let server1_index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server1);
    let server1_about = Mock::new()
        .expect_method(GET)
        .expect_path(ABOUT_PATH)
        .return_status(200)
        .create_on(&server1);
    let server1_redirect = Mock::new()
        .expect_method(GET)
        .expect_path(REDIRECT_PATH)
        .return_status(301)
        .return_header("Location", &server2.url(INDEX_PATH))
        .create_on(&server1);

    let server2_index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server2);
    let server2_about = Mock::new()
        .expect_method(GET)
        .expect_path(ABOUT_PATH)
        .return_status(200)
        .create_on(&server2);

    let _goose_stats =
        crate::GooseAttack::initialize_with_config(common::build_configuration(&server1))
            .unwrap()
            .setup()
            .unwrap()
            .register_taskset(
                taskset!("LoadTest")
                    // First load redirect, takes this request only to another domain.
                    .register_task(task!(get_domain_redirect).set_on_start())
                    // Load index directly.
                    .register_task(task!(get_index))
                    // Load about directly, always on original domain.
                    .register_task(task!(get_about)),
            )
            .execute()
            .unwrap();

    // Confirm that we load the index, about and redirect pages on the orginal domain.
    assert!(server1_index.times_called() > 0);
    assert!(server1_redirect.times_called() > 0);
    assert!(server1_about.times_called() > 0);

    // Confirm that the redirect sends us to the second domain (mocked using a
    // server on a different port).
    assert!(server2_index.times_called() > 0);

    // Confirm the we never loaded the about page on the second domain.
    assert!(server2_about.times_called() == 0);
}

#[test]
fn test_sticky_domain_redirect() {
    let server1 = MockServer::start();
    let server2 = MockServer::start();

    let server1_index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server1);
    let server1_about = Mock::new()
        .expect_method(GET)
        .expect_path(ABOUT_PATH)
        .return_status(200)
        .create_on(&server1);
    let server1_redirect = Mock::new()
        .expect_method(GET)
        .expect_path(REDIRECT_PATH)
        .return_status(301)
        .return_header("Location", &server2.url(INDEX_PATH))
        .create_on(&server1);

    let server2_index = Mock::new()
        .expect_method(GET)
        .expect_path(INDEX_PATH)
        .return_status(200)
        .create_on(&server2);
    let server2_about = Mock::new()
        .expect_method(GET)
        .expect_path(ABOUT_PATH)
        .return_status(200)
        .create_on(&server2);

    // Enable sticky_follow option.
    let mut configuration = common::build_configuration(&server1);
    configuration.sticky_follow = true;
    let _goose_stats = crate::GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .setup()
        .unwrap()
        .register_taskset(
            taskset!("LoadTest")
                // First load redirect, due to stick_follow the load test stays on the
                // new domain for all subsequent requests.
                .register_task(task!(get_domain_redirect).set_on_start())
                // Due to sticky follow, we should always load the alternative index.
                .register_task(task!(get_index))
                // Due to sticky follow, we should always load the alternative about.
                .register_task(task!(get_about)),
        )
        .execute()
        .unwrap();

    // Confirm we redirect on startup, and never load index or about.
    assert!(server1_redirect.times_called() == 1);
    assert!(server1_index.times_called() == 0);
    assert!(server1_about.times_called() == 0);

    // Confirm that we load the alternative index and about pages (mocked using
    // a server on a different port).
    assert!(server2_index.times_called() > 0);
    assert!(server2_about.times_called() > 0);
}
