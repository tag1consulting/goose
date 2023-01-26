use httpmock::{Method::GET, Mock, MockServer};
use serial_test::serial;

mod common;

use goose::config::GooseConfiguration;
use goose::goose::Scenario;
use goose::prelude::*;

// Paths used in load tests performed during these tests.
const INDEX_PATH: &str = "/";
const REDIRECT_PATH: &str = "/redirect";
const REDIRECT2_PATH: &str = "/redirect2";
const REDIRECT3_PATH: &str = "/redirect3";
const ABOUT_PATH: &str = "/about.php";

// Indexes to the above paths.
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

// Load test configuration.
const EXPECT_WORKERS: usize = 4;
const USERS: usize = 9;
const RUN_TIME: usize = 3;

// There are multiple test variations in this file.
#[derive(Clone)]
enum TestType {
    // Chain many different redirects together.
    Chain,
    // Redirect between domains.
    Domain,
    // Permanently redirect between domains.
    Sticky,
}

// Test transaction.
pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// Test transaction.
pub async fn get_about(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(ABOUT_PATH).await?;
    Ok(())
}

// Test transaction.
pub async fn get_redirect(user: &mut GooseUser) -> TransactionResult {
    // Load REDIRECT_PATH and follow redirects to ABOUT_PATH.
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

// Test transaction.
pub async fn get_domain_redirect(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(REDIRECT_PATH).await?;
    Ok(())
}

// Sets up the endpoints used to test redirects.
fn setup_mock_server_endpoints<'a>(
    test_type: &TestType,
    server: &'a MockServer,
    server2: Option<&'a MockServer>,
) -> Vec<Mock<'a>> {
    match test_type {
        TestType::Chain => {
            vec![
                // First set up INDEX_PATH, store in vector at INDEX_KEY.
                server.mock(|when, then| {
                    when.method(GET).path(INDEX_PATH);
                    then.status(200);
                }),
                // Next set up REDIRECT_PATH, store in vector at REDIRECT_KEY.
                server.mock(|when, then| {
                    when.method(GET).path(REDIRECT_PATH);
                    then.status(301).header("Location", REDIRECT2_PATH);
                }),
                // Next set up REDIRECT2_PATH, store in vector at REDIRECT2_KEY.
                server.mock(|when, then| {
                    when.method(GET).path(REDIRECT2_PATH);
                    then.status(302).header("Location", REDIRECT3_PATH);
                }),
                // Next set up REDIRECT3_PATH, store in vector at REDIRECT3_KEY.
                server.mock(|when, then| {
                    when.method(GET).path(REDIRECT3_PATH);
                    then.status(303).header("Location", ABOUT_PATH);
                }),
                // Next set up ABOUT_PATH, store in vector at ABOUT_KEY.
                server.mock(|when, then| {
                    when.method(GET).path(ABOUT_PATH);
                    then.status(200)
                        .body("<HTML><BODY>about page</BODY></HTML>");
                }),
            ]
        }
        TestType::Domain | TestType::Sticky => {
            vec![
                // First set up INDEX_PATH, store in vector at SERVER1_INDEX_KEY.
                server.mock(|when, then| {
                    when.method(GET).path(INDEX_PATH);
                    then.status(200);
                }),
                // Next set up ABOUT_PATH, store in vector at SERVER1_ABOUT_KEY.
                server.mock(|when, then| {
                    when.method(GET).path(ABOUT_PATH);
                    then.status(200)
                        .body("<HTML><BODY>about page</BODY></HTML>");
                }),
                // Next set up REDIRECT_PATH, store in vector at SERVER1_REDIRECT_KEY.
                server.mock(|when, then| {
                    when.method(GET).path(REDIRECT_PATH);
                    then.status(301)
                        .header("Location", server2.unwrap().url(INDEX_PATH));
                }),
                // Next set up INDEX_PATH on server 2, store in vector at SERVER2_INDEX_KEY.
                server2.unwrap().mock(|when, then| {
                    when.method(GET).path(INDEX_PATH);
                    then.status(200);
                }),
                // Next set up ABOUT_PATH on server 2, store in vector at SERVER2_ABOUT_KEY.
                server2.unwrap().mock(|when, then| {
                    when.method(GET).path(ABOUT_PATH);
                    then.status(200);
                }),
            ]
        }
    }
}

// Build appropriate configuration for these tests.
fn common_build_configuration(
    server: &MockServer,
    sticky: bool,
    worker: Option<bool>,
    manager: Option<usize>,
) -> GooseConfiguration {
    if let Some(expect_workers) = manager {
        if sticky {
            common::build_configuration(
                server,
                vec![
                    "--sticky-follow",
                    "--manager",
                    "--expect-workers",
                    &expect_workers.to_string(),
                    "--users",
                    &USERS.to_string(),
                    "--hatch-rate",
                    &USERS.to_string(),
                    "--run-time",
                    &RUN_TIME.to_string(),
                ],
            )
        } else {
            common::build_configuration(
                server,
                vec![
                    "--manager",
                    "--expect-workers",
                    &expect_workers.to_string(),
                    "--users",
                    &USERS.to_string(),
                    "--hatch-rate",
                    &USERS.to_string(),
                    "--run-time",
                    &RUN_TIME.to_string(),
                ],
            )
        }
    } else if worker.is_some() {
        common::build_configuration(server, vec!["--worker"])
    } else if sticky {
        common::build_configuration(
            server,
            vec![
                "--sticky-follow",
                "--users",
                &USERS.to_string(),
                "--hatch-rate",
                &USERS.to_string(),
                "--run-time",
                &RUN_TIME.to_string(),
            ],
        )
    } else {
        common::build_configuration(
            server,
            vec![
                "--users",
                &USERS.to_string(),
                "--hatch-rate",
                &USERS.to_string(),
                "--run-time",
                &RUN_TIME.to_string(),
            ],
        )
    }
}

// Helper to confirm all variations generate appropriate results.
fn validate_redirect(test_type: &TestType, mock_endpoints: &[Mock]) {
    match test_type {
        TestType::Chain => {
            // Confirm that all pages are loaded, even those not requested directly but
            // that are only loaded due to redirects.
            assert!(mock_endpoints[INDEX_KEY].hits() > 0);
            assert!(mock_endpoints[REDIRECT_KEY].hits() > 0);
            assert!(mock_endpoints[REDIRECT_KEY2].hits() > 0);
            assert!(mock_endpoints[REDIRECT_KEY3].hits() > 0);
            assert!(mock_endpoints[ABOUT_KEY].hits() > 0);

            // Confirm the entire redirect chain is loaded the same number of times.
            mock_endpoints[REDIRECT_KEY].assert_hits(mock_endpoints[REDIRECT_KEY2].hits());
            mock_endpoints[REDIRECT_KEY].assert_hits(mock_endpoints[REDIRECT_KEY3].hits());
            mock_endpoints[REDIRECT_KEY].assert_hits(mock_endpoints[ABOUT_KEY].hits());
        }
        TestType::Domain => {
            // All pages on Server1 are loaded.
            assert!(mock_endpoints[SERVER1_INDEX_KEY].hits() > 0);
            assert!(mock_endpoints[SERVER1_REDIRECT_KEY].hits() > 0);
            assert!(mock_endpoints[SERVER1_ABOUT_KEY].hits() > 0);

            // GooseUsers are redirected to Server2 correctly.
            assert!(mock_endpoints[SERVER2_INDEX_KEY].hits() > 0);

            // GooseUsers do not stick to Server2 and load the other page.
            mock_endpoints[SERVER2_ABOUT_KEY].assert_hits(0);
        }
        TestType::Sticky => {
            // Each GooseUser loads the redirect on Server1 one time.
            println!(
                "SERVER1_REDIRECT: {}, USERS: {}",
                mock_endpoints[SERVER1_REDIRECT_KEY].hits(),
                USERS,
            );
            println!(
                "SERVER1_INDEX: {}, SERVER1_ABOUT: {}",
                mock_endpoints[SERVER1_INDEX_KEY].hits(),
                mock_endpoints[SERVER1_ABOUT_KEY].hits(),
            );
            println!(
                "SERVER2_INDEX: {}, SERVER2_ABOUT: {}",
                mock_endpoints[SERVER2_INDEX_KEY].hits(),
                mock_endpoints[SERVER2_ABOUT_KEY].hits(),
            );
            mock_endpoints[SERVER1_REDIRECT_KEY].assert_hits(USERS);

            // Redirected to Server2, no user load anything else on Server1.
            mock_endpoints[SERVER1_INDEX_KEY].assert_hits(0);
            mock_endpoints[SERVER1_ABOUT_KEY].assert_hits(0);

            // All GooseUsers go on to load pages on Server2.
            assert!(mock_endpoints[SERVER2_INDEX_KEY].hits() > 0);
            assert!(mock_endpoints[SERVER2_ABOUT_KEY].hits() > 0);
        }
    }
}

// Returns the appropriate scenario needed to build these tests.
fn get_transactions(test_type: &TestType) -> Scenario {
    match test_type {
        TestType::Chain => {
            scenario!("LoadTest")
                // Load index directly.
                .register_transaction(transaction!(get_index))
                // Load redirect path, redirect to redirect2 path, redirect to
                // redirect3 path, redirect to about.
                .register_transaction(transaction!(get_redirect))
        }
        TestType::Domain | TestType::Sticky => {
            scenario!("LoadTest")
                // First load redirect, takes this request only to another domain.
                .register_transaction(transaction!(get_domain_redirect))
                // Load index.
                .register_transaction(transaction!(get_index))
                // Load about.
                .register_transaction(transaction!(get_about))
        }
    }
}

// Helper to run all standalone tests.
async fn run_standalone_test(test_type: TestType) {
    // Start the mock servers.
    let server1 = MockServer::start();
    let server2 = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&test_type, &server1, Some(&server2));

    // Build appropriate configuration.
    let sticky = match test_type {
        TestType::Sticky => true,
        TestType::Chain | TestType::Domain => false,
    };
    let configuration = common_build_configuration(&server1, sticky, None, None);

    // Run the Goose Attack.
    common::run_load_test(
        common::build_load_test(
            configuration,
            vec![get_transactions(&test_type)],
            None,
            None,
        ),
        None,
    )
    .await;

    // Confirm that the load test was actually redirected.
    validate_redirect(&test_type, &mock_endpoints);
}

// Helper to run all standalone tests.
async fn run_gaggle_test(test_type: TestType) {
    // Start the mock servers.
    let server1 = MockServer::start();
    let server2 = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&test_type, &server1, Some(&server2));

    // Build appropriate Worker configuration.
    let sticky = match test_type {
        TestType::Sticky => true,
        TestType::Chain | TestType::Domain => false,
    };
    let worker_configuration = common_build_configuration(&server1, sticky, Some(true), None);

    // Workers launched in own threads, store thread handles.
    let worker_handles = common::launch_gaggle_workers(EXPECT_WORKERS, || {
        common::build_load_test(
            worker_configuration.clone(),
            vec![get_transactions(&test_type)],
            None,
            None,
        )
    });

    // Build Manager configuration.
    let manager_configuration =
        common_build_configuration(&server1, sticky, None, Some(EXPECT_WORKERS));

    // Build the load test for the Workers.
    let manager_goose_attack = common::build_load_test(
        manager_configuration,
        vec![get_transactions(&test_type)],
        None,
        None,
    );

    // Run the Goose Attack.
    common::run_load_test(manager_goose_attack, Some(worker_handles)).await;

    // Confirm that the load test was actually redirected.
    validate_redirect(&test_type, &mock_endpoints);
}

#[tokio::test]
// Request a page that redirects multiple times with different redirect headers.
async fn test_redirect() {
    run_standalone_test(TestType::Chain).await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
#[serial]
// Request a page that redirects multiple times with different redirect headers,
// in Gaggle mode.
async fn test_redirect_gaggle() {
    run_gaggle_test(TestType::Chain).await;
}

#[tokio::test]
// Request a page that redirects to another domain.
// Different domains are simulated with multiple mock servers running on different
// ports.
async fn test_domain_redirect() {
    run_standalone_test(TestType::Domain).await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Request a page that redirects to another domain, in Gaggle mode.
// Different domains are simulated with multiple mock servers running on different
// ports.
async fn test_domain_redirect_gaggle() {
    run_gaggle_test(TestType::Domain).await;
}

#[tokio::test]
// Request a page that redirects to another domain with --sticky-follow enabled.
// Different domains are simulated with multiple mock servers running on different
// ports.
async fn test_sticky_domain_redirect() {
    run_standalone_test(TestType::Sticky).await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Request a page that redirects to another domain with --sticky-follow enabled, in
// Gaggle mode.
// Different domains are simulated with multiple mock servers running on different
// ports.
async fn test_sticky_domain_redirect_gaggle() {
    run_gaggle_test(TestType::Sticky).await;
}
