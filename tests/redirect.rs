use httpmock::Method::GET;
use httpmock::{mock, with_mock_server};

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
#[with_mock_server]
/// Simulate a load test which includes a page with a redirect chain, confirms
/// all redirects are correctly followed.
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

#[test]
#[with_mock_server]
/// Simulate a load test which includes a page with a redirect to another domain
/// (which in this case is a second mock server running on a different path).
/// all redirects are correctly followed.
fn test_domain_redirect() {
    let mock_index = mock(GET, INDEX_PATH).return_status(200).create();
    let mock_about = mock(GET, ABOUT_PATH).return_status(200).create();
    let alternate_domain = &mockito::server_url();
    let mock_redirect = mock(GET, REDIRECT_PATH)
        // Moved Permanently
        .return_status(301)
        .return_header(
            "Location",
            format!("{}{}", alternate_domain, INDEX_PATH).as_str(),
        )
        .create();
    let mock_index_alt = mockito::mock("GET", INDEX_PATH)
        .with_status(200)
        .expect_at_least(1)
        .create();
    let mock_about_alt = mockito::mock("GET", ABOUT_PATH)
        .with_status(200)
        .expect(0)
        .create();

    crate::GooseAttack::initialize_with_config(common::build_configuration())
        .setup()
        .register_taskset(
            taskset!("LoadTest")
                // First load redirect, takes this request only to another domain.
                .register_task(task!(get_domain_redirect).set_on_start())
                // Load index directly.
                .register_task(task!(get_index))
                // Load about directly, always on original domain.
                .register_task(task!(get_about)),
        )
        .execute();

    let called_index = mock_index.times_called();
    let called_about = mock_about.times_called();
    let called_redirect = mock_redirect.times_called();

    // Confirm that we load the index, about and redirect pages on the orginal domain.
    assert_ne!(called_index, 0);
    assert_ne!(called_about, 0);
    assert_ne!(called_redirect, 0);

    // Confirm that the redirect sends us to the second domain (mocked using a
    // server on a different port).
    mock_index_alt.assert();

    // Confirm the we never loaded the about page on the second domain.
    mock_about_alt.assert();
}

#[test]
#[with_mock_server]
fn test_sticky_domain_redirect() {
    let mock_index = mock(GET, INDEX_PATH).return_status(200).create();
    let mock_about = mock(GET, ABOUT_PATH).return_status(200).create();
    let alternate_domain = &mockito::server_url();
    eprintln!("alternate_domain: {}", &alternate_domain);
    let mock_redirect = mock(GET, REDIRECT_PATH)
        .return_status(301)
        .return_header(
            "Location",
            format!("{}{}", alternate_domain, INDEX_PATH).as_str(),
        )
        .create();
    let mock_index_alt = mockito::mock("GET", INDEX_PATH)
        .with_status(200)
        .expect_at_least(1)
        .create();
    let mock_about_alt = mockito::mock("GET", ABOUT_PATH)
        .with_status(200)
        .expect_at_least(1)
        .create();

    // Enable sticky_follow option.
    let mut configuration = common::build_configuration();
    configuration.sticky_follow = true;
    crate::GooseAttack::initialize_with_config(configuration)
        .setup()
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
        .execute();

    let called_index = mock_index.times_called();
    let called_about = mock_about.times_called();
    let called_redirect = mock_redirect.times_called();

    // Confirm we redirect on startup, and never load index or about.
    assert_eq!(called_redirect, 1);
    assert_eq!(called_index, 0);
    assert_eq!(called_about, 0);

    // Confirm that we load the alternative index and about pages (mocked using
    // a server on a different port).)
    mock_index_alt.assert();
    mock_about_alt.assert();
}
