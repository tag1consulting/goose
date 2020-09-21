use httpmock::Method::GET;
use httpmock::{Mock, MockRef, MockServer};
use serial_test::serial;

mod common;

use goose::prelude::*;
use goose::GooseConfiguration;

const INDEX_PATH: &str = "/";
const REDIRECT_PATH: &str = "/redirect";
const REDIRECT2_PATH: &str = "/redirect2";
const REDIRECT3_PATH: &str = "/redirect3";
const ABOUT_PATH: &str = "/about.php";

const INDEX_KEY: usize = 0;
const REDIRECT_KEY: usize = 1;
const REDIRECT_KEY2: usize = 2;
const REDIRECT_KEY3: usize = 3;
const ABOUT_KEY: usize = 4;

const SERVER1_INDEX_KEY: usize = 0;
const SERVER1_ABOUT_KEY: usize = 1;
const SERVER1_REDIRECT_KEY: usize = 2;
const SERVER2_INDEX_KEY: usize = 3;
const SERVER2_ABOUT_KEY: usize = 4;

const EXPECT_WORKERS: usize = 2;

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

// Task function, load REDIRECT_PATH and follow redirects to ABOUT_PATH.
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

// Task function, load REDIRECT_PATH and follow redirect to new domain.
pub async fn get_domain_redirect(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(REDIRECT_PATH).await?;
    Ok(())
}

// Defines the different types of redirects being tested.
#[derive(Clone)]
enum TestType {
    // Chains many different redirects together.
    Chain,
    // Redirects between domains.
    Domain,
    // Permanently redirects between domains.
    Sticky,
}

// Sets up the endpoints used to test redirects.
fn setup_mock_server_endpoints<'a>(
    test_type: &TestType,
    server: &'a MockServer,
    server2: Option<&'a MockServer>,
) -> Vec<MockRef<'a>> {
    let mut endpoints: Vec<MockRef> = Vec::new();

    match test_type {
        TestType::Chain => {
            // First set up INDEX_PATH, store in vector at INDEX_KEY.
            endpoints.push(
                Mock::new()
                    .expect_method(GET)
                    .expect_path(INDEX_PATH)
                    .return_status(200)
                    .create_on(&server),
            );
            // Next set up REDIRECT_PATH, store in vector at REDIRECT_KEY.
            endpoints.push(
                Mock::new()
                    .expect_method(GET)
                    .expect_path(REDIRECT_PATH)
                    .return_status(301)
                    .return_header("Location", REDIRECT2_PATH)
                    .create_on(&server),
            );
            // Next set up REDIRECT2_PATH, store in vector at REDIRECT2_KEY.
            endpoints.push(
                Mock::new()
                    .expect_method(GET)
                    .expect_path(REDIRECT2_PATH)
                    .return_status(302)
                    .return_header("Location", REDIRECT3_PATH)
                    .create_on(&server),
            );
            // Next set up REDIRECT3_PATH, store in vector at REDIRECT3_KEY.
            endpoints.push(
                Mock::new()
                    .expect_method(GET)
                    .expect_path(REDIRECT3_PATH)
                    .return_status(303)
                    .return_header("Location", ABOUT_PATH)
                    .create_on(&server),
            );
            // Next set up ABOUT_PATH, store in vector at ABOUT_KEY.
            endpoints.push(
                Mock::new()
                    .expect_method(GET)
                    .expect_path(ABOUT_PATH)
                    .return_status(200)
                    .return_body("<HTML><BODY>about page</BODY></HTML>")
                    .create_on(&server),
            );
        }
        TestType::Domain | TestType::Sticky => {
            // First set up INDEX_PATH, store in vector at SERVER1_INDEX_KEY.
            endpoints.push(
                Mock::new()
                    .expect_method(GET)
                    .expect_path(INDEX_PATH)
                    .return_status(200)
                    .create_on(&server),
            );
            // Next set up ABOUT_PATH, store in vector at SERVER1_ABOUT_KEY.
            endpoints.push(
                Mock::new()
                    .expect_method(GET)
                    .expect_path(ABOUT_PATH)
                    .return_status(200)
                    .return_body("<HTML><BODY>about page</BODY></HTML>")
                    .create_on(&server),
            );
            // Next set up REDIRECT_PATH, store in vector at SERVER1_REDIRECT_KEY.
            endpoints.push(
                Mock::new()
                    .expect_method(GET)
                    .expect_path(REDIRECT_PATH)
                    .return_status(301)
                    .return_header("Location", &server2.unwrap().url(INDEX_PATH))
                    .create_on(&server),
            );
            // Next set up INDEX_PATH on server 2, store in vector at SERVER2_INDEX_KEY.
            endpoints.push(
                Mock::new()
                    .expect_method(GET)
                    .expect_path(INDEX_PATH)
                    .return_status(200)
                    .create_on(&server2.unwrap()),
            );
            // Next set up ABOUT_PATH on server 2, store in vector at SERVER2_ABOUT_KEY.
            endpoints.push(
                Mock::new()
                    .expect_method(GET)
                    .expect_path(ABOUT_PATH)
                    .return_status(200)
                    .create_on(&server2.unwrap()),
            );
        }
    }

    endpoints
}

// Build configuration for a load test.
fn common_build_configuration(
    server: &MockServer,
    sticky: bool,
    worker: Option<bool>,
    manager: Option<usize>,
) -> GooseConfiguration {
    if let Some(expect_workers) = manager {
        if sticky {
            common::build_configuration(
                &server,
                vec![
                    "--sticky-follow",
                    "--manager",
                    "--expect-workers",
                    &expect_workers.to_string(),
                    "--users",
                    &expect_workers.to_string(),
                    "--hatch-rate",
                    &expect_workers.to_string(),
                ],
            )
        } else {
            common::build_configuration(
                &server,
                vec![
                    "--manager",
                    "--expect-workers",
                    &expect_workers.to_string(),
                    "--users",
                    &expect_workers.to_string(),
                    "--hatch-rate",
                    &expect_workers.to_string(),
                ],
            )
        }
    } else if worker.is_some() {
        common::build_configuration(&server, vec!["--worker"])
    } else {
        if sticky {
            common::build_configuration(&server, vec!["--sticky-follow"])
        } else {
            common::build_configuration(&server, vec![])
        }
    }
}

// Common validation for the load tests in this file.
fn validate_redirect(test_type: &TestType, mock_endpoints: &Vec<MockRef>) {
    match test_type {
        TestType::Chain => {
            // Confirm that we loaded the mock endpoints; while we never load the about page
            // directly, we should follow the redirects and load it.
            assert!(mock_endpoints[INDEX_KEY].times_called() > 0);
            assert!(mock_endpoints[REDIRECT_KEY].times_called() > 0);
            assert!(mock_endpoints[REDIRECT_KEY2].times_called() > 0);
            assert!(mock_endpoints[REDIRECT_KEY3].times_called() > 0);
            assert!(mock_endpoints[ABOUT_KEY].times_called() > 0);

            // We should have called all redirects the same number of times as we called the
            // final about page.
            assert!(
                mock_endpoints[REDIRECT_KEY].times_called()
                    == mock_endpoints[REDIRECT_KEY2].times_called()
            );
            assert!(
                mock_endpoints[REDIRECT_KEY].times_called()
                    == mock_endpoints[REDIRECT_KEY3].times_called()
            );
            assert!(
                mock_endpoints[REDIRECT_KEY].times_called()
                    == mock_endpoints[ABOUT_KEY].times_called()
            );
        }
        TestType::Domain => {
            // Confirm that we load the index, about and redirect pages on the original domain.
            assert!(mock_endpoints[SERVER1_INDEX_KEY].times_called() > 0);
            assert!(mock_endpoints[SERVER1_REDIRECT_KEY].times_called() > 0);
            assert!(mock_endpoints[SERVER1_ABOUT_KEY].times_called() > 0);

            // Confirm that the redirect sends us to the second domain (mocked using a
            // server on a different port).
            assert!(mock_endpoints[SERVER2_INDEX_KEY].times_called() > 0);

            // Confirm the we never loaded the about page on the second domain.
            assert!(mock_endpoints[SERVER2_ABOUT_KEY].times_called() == 0);
        }
        TestType::Sticky => {
            // Confirm we redirect on startup, and never load index or about.
            assert!(mock_endpoints[SERVER1_INDEX_KEY].times_called() == 0);
            assert!(mock_endpoints[SERVER1_REDIRECT_KEY].times_called() == 1);
            assert!(mock_endpoints[SERVER1_ABOUT_KEY].times_called() == 0);

            // Confirm that we load the alternative index and about pages (mocked using
            // a server on a different port).
            assert!(mock_endpoints[SERVER2_INDEX_KEY].times_called() > 0);
            assert!(mock_endpoints[SERVER2_ABOUT_KEY].times_called() > 0);
        }
    }
}

// Run the actual load test. Validation is done server-side, so no need to
// return the GooseMetrics.
fn run_load_test(test_type: &TestType, configuration: &GooseConfiguration) {
    // First, initialize an empty load test with the provided configuration.
    let goose = crate::GooseAttack::initialize_with_config(configuration.clone()).unwrap();

    // Now, add task sets as required by the specifying test_type.
    let goose = match test_type {
        TestType::Chain => {
            goose.register_taskset(
                taskset!("LoadTest")
                    // Load index directly.
                    .register_task(task!(get_index))
                    // Load redirect path, redirect to redirect2 path, redirect to
                    // redirect3 path, redirect to about.
                    .register_task(task!(get_redirect)),
            )
        }
        TestType::Domain => {
            goose.register_taskset(
                taskset!("LoadTest")
                    // First load redirect, takes this request only to another domain.
                    .register_task(task!(get_domain_redirect).set_on_start())
                    // Load index directly.
                    .register_task(task!(get_index))
                    // Load about directly, always on original domain.
                    .register_task(task!(get_about)),
            )
        }
        TestType::Sticky => {
            goose.register_taskset(
                taskset!("LoadTest")
                    // First load redirect, due to stick_follow the load test stays on the
                    // new domain for all subsequent requests.
                    .register_task(task!(get_domain_redirect).set_on_start())
                    // Due to sticky follow, we should always load the alternative index.
                    .register_task(task!(get_index))
                    // Due to sticky follow, we should always load the alternative about.
                    .register_task(task!(get_about)),
            )
        }
    };

    // Finally, execute the load test.
    let _goose_metrics = goose.execute().unwrap();
}

#[test]
/// Simulate a load test which includes a page with a redirect chain, confirms
/// all redirects are correctly followed.
fn test_redirect() {
    // Start the mock server.
    let server = MockServer::start();

    // Define the type of redirect being tested.
    let test_type = TestType::Chain;

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&test_type, &server, None);

    // Build configuration.
    let configuration = common_build_configuration(&server, false, None, None);

    // Run the load test as configured.
    run_load_test(&test_type, &configuration);

    // Confirm that the load test was actually throttled.
    validate_redirect(&test_type, &mock_endpoints);
}

#[test]
// Only run gaggle tests if the feature is compiled into the codebase.
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Gaggle tests have to be running serially instead of in parallel.
#[serial]
/// Simulate a distributed load test which includes a page with a redirect chain,
/// confirms all redirects are correctly followed.
fn test_redirect_gaggle() {
    // Start the mock server.
    let server = MockServer::start();

    // Define the type of redirect being tested.
    let test_type = TestType::Chain;

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&test_type, &server, None);

    // Build Worker configuration.
    let configuration = common_build_configuration(&server, false, Some(true), None);

    // Workers launched in own threads, store thread handles.
    let mut worker_handles = Vec::new();

    // Launch Workers in threads.
    for _ in 0..EXPECT_WORKERS {
        let worker_test_type = test_type.clone();
        let worker_configuration = configuration.clone();
        // Start worker instance of the load test.
        worker_handles.push(std::thread::spawn(move || {
            // Run the load test as configured.
            run_load_test(&worker_test_type, &worker_configuration);
        }));
    }

    // Build Manager configuration.
    let manager_configuration =
        common_build_configuration(&server, false, None, Some(EXPECT_WORKERS));

    // Run the load test as configured.
    run_load_test(&test_type, &manager_configuration);

    // Confirm that the load test was actually throttled.
    validate_redirect(&test_type, &mock_endpoints);
}

#[test]
/// Simulate a load test which includes a page with a redirect to another domain
/// (which in this case is a second mock server running on a different path).
/// Confirm all redirects are correctly followed.
fn test_domain_redirect() {
    // Start the mock servers.
    let server1 = MockServer::start();
    let server2 = MockServer::start();

    // Define the type of redirect being tested.
    let test_type = TestType::Domain;

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&test_type, &server1, Some(&server2));

    // Build configuration.
    let configuration = common_build_configuration(&server1, false, None, None);

    // Run the load test as configured.
    run_load_test(&test_type, &configuration);

    // Confirm that the load test was actually throttled.
    validate_redirect(&test_type, &mock_endpoints);
}

#[test]
// Only run gaggle tests if the feature is compiled into the codebase.
#[cfg_attr(not(feature = "gaggle"), ignore)]
// Gaggle tests have to be running serially instead of in parallel.
#[serial]
/// Simulate a distributed load test which includes a page with a redirect to
/// another domain (which in this case is a second mock server running on a
/// different path). Confirm all redirects are correctly followed.
fn test_domain_redirect_gaggle() {
    // Start the mock servers.
    let server1 = MockServer::start();
    let server2 = MockServer::start();

    // Define the type of redirect being tested.
    let test_type = TestType::Domain;

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&test_type, &server1, Some(&server2));

    // Build Worker configuration.
    let configuration = common_build_configuration(&server1, false, Some(true), None);

    // Workers launched in own threads, store thread handles.
    let mut worker_handles = Vec::new();

    // Launch Workers in threads.
    for _ in 0..EXPECT_WORKERS {
        let worker_test_type = test_type.clone();
        let worker_configuration = configuration.clone();
        // Start worker instance of the load test.
        worker_handles.push(std::thread::spawn(move || {
            // Run the load test as configured.
            run_load_test(&worker_test_type, &worker_configuration);
        }));
    }

    // Build Manager configuration.
    let manager_configuration =
        common_build_configuration(&server1, false, None, Some(EXPECT_WORKERS));

    // Run the load test as configured.
    run_load_test(&test_type, &manager_configuration);

    // Confirm that the load test was actually throttled.
    validate_redirect(&test_type, &mock_endpoints);
}

#[test]
fn test_sticky_domain_redirect() {
    // Start the mock servers.
    let server1 = MockServer::start();
    let server2 = MockServer::start();

    // Define the type of redirect being tested.
    let test_type = TestType::Sticky;

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&test_type, &server1, Some(&server2));

    // Build configuration, enabling --sticky-follow.
    let configuration = common_build_configuration(&server1, true, None, None);

    // Run the load test as configured.
    run_load_test(&test_type, &configuration);

    // Confirm that the load test was actually throttled.
    validate_redirect(&test_type, &mock_endpoints);
}
