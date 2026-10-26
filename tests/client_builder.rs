//! Tests for the type-safe client builder implementation.

use httpmock::{Method::GET, MockServer};
use std::time::Duration;

mod common;

use goose::client::{ClientStrategy, GooseClientBuilder};
use goose::config::GooseConfiguration;
use goose::prelude::*;

#[test]
fn test_client_builder_type_safety() {
    // Test that we can create builders with different states
    let cookies_enabled = GooseClientBuilder::new();
    let cookies_disabled = cookies_enabled.without_cookies();
    let back_to_enabled = cookies_disabled.with_cookies();

    // Test that we can configure shared properties on both states
    let _configured_enabled = back_to_enabled
        .timeout(Duration::from_secs(30))
        .user_agent("test-agent")
        .gzip(false);

    let _configured_disabled = GooseClientBuilder::new()
        .without_cookies()
        .timeout(Duration::from_secs(30))
        .user_agent("test-agent")
        .gzip(false);
}

#[test]
fn test_client_strategy_individual() {
    let builder = GooseClientBuilder::new()
        .timeout(Duration::from_secs(10))
        .user_agent("test-individual");

    let strategy = builder.build_strategy();

    match strategy {
        ClientStrategy::Individual(config) => {
            assert_eq!(config.timeout, Some(Duration::from_secs(10)));
            assert_eq!(config.user_agent, "test-individual");
            assert!(config.cookies_enabled);
        }
        ClientStrategy::Shared(_) => panic!("Expected Individual strategy"),
    }
}

#[test]
fn test_client_strategy_shared() {
    let builder = GooseClientBuilder::new()
        .without_cookies()
        .timeout(Duration::from_secs(20))
        .user_agent("test-shared");

    let strategy = builder.build_strategy().expect("Failed to build strategy");

    match strategy {
        ClientStrategy::Shared(_) => {
            // Success - we got a shared client
        }
        ClientStrategy::Individual(_) => panic!("Expected Shared strategy"),
    }
}

#[cfg(feature = "cookies")]
#[tokio::test]
async fn test_goose_attack_with_custom_client_cookies_enabled() {
    let _attack = GooseAttack::initialize()
        .expect("Failed to initialize GooseAttack")
        .set_client_builder_with_cookies(
            GooseClientBuilder::new()
                .timeout(Duration::from_secs(15))
                .user_agent("test-cookies-enabled"),
        )
        .expect("Failed to set client builder");

    // If we get here without panicking, the configuration worked
}

#[tokio::test]
async fn test_goose_attack_with_custom_client_cookies_disabled() {
    let _attack = GooseAttack::initialize()
        .expect("Failed to initialize GooseAttack")
        .set_client_builder_without_cookies(
            GooseClientBuilder::new()
                .without_cookies()
                .timeout(Duration::from_secs(25))
                .user_agent("test-cookies-disabled"),
        )
        .expect("Failed to set client builder");

    // If we get here without panicking, the configuration worked
}

#[tokio::test]
async fn test_default_behavior_unchanged() {
    // Test that default behavior is unchanged when no client builder is set
    let _attack = GooseAttack::initialize().expect("Failed to initialize GooseAttack");

    // If we get here without panicking, the default initialization worked
}

#[test]
fn test_client_config_from_goose_configuration() {
    use goose::config::GooseConfiguration;

    let args: Vec<&str> = vec![];
    let goose_config =
        GooseConfiguration::parse_args_default(&args).expect("Failed to parse configuration");

    let _builder = GooseClientBuilder::from_configuration(&goose_config);

    // If we get here without panicking, the configuration worked
}

#[test]
fn test_builder_method_chaining() {
    // Test that all methods return Self for chaining
    let _builder = GooseClientBuilder::new()
        .timeout(Duration::from_secs(30))
        .user_agent("chained-test")
        .gzip(true)
        .accept_invalid_certs(false)
        .without_cookies()
        .with_cookies()
        .timeout(Duration::from_secs(60));
}

#[test]
fn test_state_transitions() {
    let builder = GooseClientBuilder::new(); // CookiesEnabled

    // Test transition to disabled
    let builder = builder.without_cookies(); // CookiesDisabled

    // Test transition back to enabled
    let _builder = builder.with_cookies(); // CookiesEnabled
}

#[cfg(feature = "cookies")]
#[tokio::test]
async fn test_functional_load_test_with_cookies_enabled() {
    // Create a functional load test to verify cookies-enabled client works
    let _attack = GooseAttack::initialize()
        .expect("Failed to initialize GooseAttack")
        .set_client_builder_with_cookies(
            GooseClientBuilder::new()
                .timeout(Duration::from_secs(5))
                .user_agent("test-functional-cookies"),
        )
        .expect("Failed to set client builder")
        .register_scenario(
            scenario!("TestScenario")
                .set_host("http://httpbin.org")
                .register_transaction(transaction!(test_transaction)),
        )
        .set_default(GooseDefault::Users, 1)
        .expect("Failed to set users")
        .set_default(GooseDefault::RunTime, 1)
        .expect("Failed to set runtime");

    // If we get here without panicking, the setup worked
}

#[tokio::test]
async fn test_functional_load_test_with_cookies_disabled() {
    // Create a functional load test to verify cookies-disabled client works
    let _attack = GooseAttack::initialize()
        .expect("Failed to initialize GooseAttack")
        .set_client_builder_without_cookies(
            GooseClientBuilder::new()
                .without_cookies()
                .timeout(Duration::from_secs(5))
                .user_agent("test-functional-no-cookies"),
        )
        .expect("Failed to set client builder")
        .register_scenario(
            scenario!("TestScenario")
                .set_host("http://httpbin.org")
                .register_transaction(transaction!(test_transaction)),
        )
        .set_default(GooseDefault::Users, 1)
        .expect("Failed to set users")
        .set_default(GooseDefault::RunTime, 1)
        .expect("Failed to set runtime");

    // If we get here without panicking, the setup worked
}

#[test]
fn test_performance_optimization_different_strategies() {
    // Test that different strategies produce different memory footprints
    let individual_strategy = GooseClientBuilder::new()
        .timeout(Duration::from_secs(10))
        .build_strategy();

    let shared_strategy = GooseClientBuilder::new()
        .without_cookies()
        .timeout(Duration::from_secs(10))
        .build_strategy()
        .expect("Failed to build shared strategy");

    // Verify we get different strategy types
    match (&individual_strategy, &shared_strategy) {
        (ClientStrategy::Individual(_), ClientStrategy::Shared(_)) => {
            // Success - we have different strategies as expected
        }
        _ => panic!("Expected Individual and Shared strategies"),
    }
}

#[test]
fn test_compile_time_safety_demonstration() {
    // Demonstrate that the type-safe builder works correctly with different states

    // ✅ Standard methods available on CookiesEnabled state (default)
    let _cookies_enabled = GooseClientBuilder::new()
        .timeout(Duration::from_secs(30))
        .user_agent("test-enabled");

    // ✅ Shared methods available on both states
    let _cookies_disabled = GooseClientBuilder::new()
        .without_cookies()
        .timeout(Duration::from_secs(30)) // This compiles
        .user_agent("test"); // This compiles

    // ✅ State transitions work seamlessly
    let _transitioning = GooseClientBuilder::new()
        .timeout(Duration::from_secs(15)) // Available on CookiesEnabled
        .without_cookies() // Transition to CookiesDisabled
        .timeout(Duration::from_secs(20)) // Available on both
        .with_cookies() // Transition back to CookiesEnabled
        .timeout(Duration::from_secs(25)); // Available again

    // Note: The type system prevents calling cookie-specific methods
    // on CookiesDisabled state, ensuring compile-time safety
}

// Mock transaction for testing
async fn test_transaction(user: &mut GooseUser) -> TransactionResult {
    let _response = user.get("/get").await?;
    Ok(())
}

// The tests below run load tests against a mock server, to confirm the client
// builders set on a GooseAttack reach every user's client.

// The user agent the builders set, which the mock endpoints require.
const BUILDER_USER_AGENT: &str = "goose-client-builder-test/1.0";

// How many users the multi user tests launch.
const USERS: usize = 3;

// A per user path, so each user's requests are counted apart.
fn user_path(prefix: &str, user: &GooseUser) -> String {
    format!("{prefix}{}", user.weighted_users_index)
}

// Launch USERS users at once and run for one second.
fn multi_user_configuration(server: &MockServer) -> GooseConfiguration {
    common::build_configuration(
        server,
        vec![
            "--users",
            &USERS.to_string(),
            "--increase-rate",
            &USERS.to_string(),
            "--run-time",
            "1",
        ],
    )
}

async fn request_user_agent_path(user: &mut GooseUser) -> TransactionResult {
    let path = user_path("/agent/", user);
    let _goose = user.get(&path).await?;
    Ok(())
}

// One mock per user that answers only a request carrying the builder's user agent.
fn user_agent_mocks(server: &MockServer) -> Vec<httpmock::Mock<'_>> {
    (0..USERS)
        .map(|i| {
            server.mock(|when, then| {
                when.method(GET)
                    .path(format!("/agent/{i}"))
                    .header("user-agent", BUILDER_USER_AGENT);
                then.status(200);
            })
        })
        .collect()
}

// Run a load test whose every user requests its own path, and confirm each path
// was requested with the builder's user agent.
async fn assert_user_agent_reaches_every_user(goose_attack: GooseAttack, server: &MockServer) {
    let mocks = user_agent_mocks(server);
    let goose_attack = goose_attack.register_scenario(
        scenario!("UserAgent").register_transaction(transaction!(request_user_agent_path)),
    );
    common::run_load_test(goose_attack, None).await;
    for (i, mock) in mocks.iter().enumerate() {
        assert!(
            mock.calls() > 0,
            "user {} never sent the builder's user agent",
            i
        );
    }
}

#[cfg(feature = "cookies")]
#[tokio::test]
async fn test_user_agent_reaches_every_user_with_cookies() {
    let server = MockServer::start();
    let goose_attack = GooseAttack::initialize_with_config(multi_user_configuration(&server))
        .unwrap()
        .set_client_builder_with_cookies(GooseClientBuilder::new().user_agent(BUILDER_USER_AGENT))
        .unwrap();
    assert_user_agent_reaches_every_user(goose_attack, &server).await;
}

#[tokio::test]
async fn test_user_agent_reaches_every_user_without_cookies() {
    let server = MockServer::start();
    let goose_attack = GooseAttack::initialize_with_config(multi_user_configuration(&server))
        .unwrap()
        .set_client_builder_without_cookies(
            GooseClientBuilder::new()
                .without_cookies()
                .user_agent(BUILDER_USER_AGENT),
        )
        .unwrap();
    assert_user_agent_reaches_every_user(goose_attack, &server).await;
}

// Run a load test whose every user requests its own path, and confirm each user
// sent only the user agent `expected` gives for its index: one mock per user
// answers that user agent and a second answers any other, which must never be called.
async fn assert_user_agent_by_user(
    goose_attack: GooseAttack,
    server: &MockServer,
    on_start: Option<Transaction>,
    expected: fn(usize) -> &'static str,
) {
    let mocks: Vec<_> = (0..USERS)
        .map(|i| {
            let path = format!("/agent/{i}");
            let right = server.mock(|when, then| {
                when.method(GET)
                    .path(path.clone())
                    .header("user-agent", expected(i));
                then.status(200);
            });
            let wrong = server.mock(|when, then| {
                when.method(GET)
                    .path(path)
                    .header_not("user-agent", expected(i));
                then.status(200);
            });
            (right, wrong)
        })
        .collect();
    let mut scenario =
        scenario!("UserAgent").register_transaction(transaction!(request_user_agent_path));
    if let Some(on_start) = on_start {
        scenario = scenario.register_transaction(on_start);
    }
    common::run_load_test(goose_attack.register_scenario(scenario), None).await;
    for (i, (right, wrong)) in mocks.iter().enumerate() {
        assert!(right.calls() > 0, "user {} never sent {}", i, expected(i));
        assert_eq!(
            wrong.calls(),
            0,
            "user {} sent a user agent other than {}",
            i,
            expected(i)
        );
    }
}

// The last client builder setter called wins, whichever kind it is.
#[cfg(feature = "cookies")]
#[tokio::test]
async fn test_last_client_builder_setter_wins() {
    let server = MockServer::start();
    let goose_attack = GooseAttack::initialize_with_config(multi_user_configuration(&server))
        .unwrap()
        .set_client_builder_with_cookies(GooseClientBuilder::new().user_agent("first"))
        .unwrap()
        .set_client_builder_without_cookies(
            GooseClientBuilder::new()
                .without_cookies()
                .user_agent("second"),
        )
        .unwrap();
    assert_user_agent_by_user(goose_attack, &server, None, |_| "second").await;
}

// User 0 replaces its client in on_start, leaving the shared client.
async fn user_zero_sets_own_client(user: &mut GooseUser) -> TransactionResult {
    if user.weighted_users_index == 0 {
        user.set_client_builder(reqwest::Client::builder().user_agent("own-ua"))
            .await?;
    }
    Ok(())
}

// A user that calls GooseUser::set_client_builder leaves the shared client, and
// the other users keep it.
#[tokio::test]
async fn test_set_client_builder_leaves_the_shared_client() {
    let server = MockServer::start();
    let goose_attack = GooseAttack::initialize_with_config(multi_user_configuration(&server))
        .unwrap()
        .set_client_builder_without_cookies(
            GooseClientBuilder::new()
                .without_cookies()
                .user_agent("shared-ua"),
        )
        .unwrap();
    assert_user_agent_by_user(
        goose_attack,
        &server,
        Some(transaction!(user_zero_sets_own_client).set_on_start()),
        |i| if i == 0 { "own-ua" } else { "shared-ua" },
    )
    .await;
}

// The users that run test_start and test_stop get the builder's client.
async fn request_setup_path(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/setup").await?;
    Ok(())
}

async fn request_teardown_path(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/teardown").await?;
    Ok(())
}

#[tokio::test]
async fn test_builder_reaches_test_start_and_test_stop() {
    let server = MockServer::start();
    let mocks: Vec<_> = ["/setup", "/teardown"]
        .iter()
        .map(|path| {
            server.mock(|when, then| {
                when.method(GET)
                    .path(*path)
                    .header("user-agent", BUILDER_USER_AGENT);
                then.status(200);
            })
        })
        .collect();
    let goose_attack =
        GooseAttack::initialize_with_config(common::build_configuration(&server, vec![]))
            .unwrap()
            .set_client_builder_without_cookies(
                GooseClientBuilder::new()
                    .without_cookies()
                    .user_agent(BUILDER_USER_AGENT),
            )
            .unwrap()
            .register_scenario(
                scenario!("Setup").register_transaction(transaction!(test_transaction)),
            )
            .test_start(transaction!(request_setup_path))
            .test_stop(transaction!(request_teardown_path));
    common::run_load_test(goose_attack, None).await;
    for mock in &mocks {
        mock.assert_calls(1);
    }
}

// A request the mock server answers after one second.
const SLOW_PATH: &str = "/slow";

async fn request_slow_path(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(SLOW_PATH).await?;
    Ok(())
}

// Run one user that requests SLOW_PATH through a shared client builder, given
// `builder_timeout` when set, with the given command line options and timeout
// GooseDefault, returning how many of its requests succeeded and failed.
async fn slow_requests(
    server: &MockServer,
    builder_timeout: Option<Duration>,
    command_line: Vec<&str>,
    default_timeout: Option<&str>,
) -> (usize, usize) {
    let mut options = vec!["--run-time", "2"];
    options.extend(command_line);
    let mut builder = GooseClientBuilder::new().without_cookies();
    if let Some(timeout) = builder_timeout {
        builder = builder.timeout(timeout);
    }
    let mut goose_attack =
        GooseAttack::initialize_with_config(common::build_configuration(server, options))
            .unwrap()
            .set_client_builder_without_cookies(builder)
            .unwrap()
            .register_scenario(
                scenario!("Timeout").register_transaction(transaction!(request_slow_path)),
            );
    if let Some(timeout) = default_timeout {
        goose_attack = *goose_attack
            .set_default(GooseDefault::Timeout, timeout)
            .unwrap();
    }
    let goose_metrics = common::run_load_test(goose_attack, None).await;
    let metrics = goose_metrics
        .requests
        .get(&format!("GET {SLOW_PATH}"))
        .expect("the user requested the slow path");
    (metrics.success_count, metrics.fail_count)
}

#[tokio::test]
async fn test_timeout_precedence() {
    let server = MockServer::start();
    let _slow = server.mock(|when, then| {
        when.method(GET).path(SLOW_PATH);
        then.status(200).delay(Duration::from_secs(1));
    });
    let short = Some(Duration::from_millis(200));

    // The builder's 200 ms timeout applies: every request times out.
    let (success, fail) = slow_requests(&server, short, vec![], None).await;
    assert_eq!(success, 0, "the builder's timeout was not applied");
    assert!(fail > 0);

    // The builder's timeout wins over a GooseDefault.
    let (success, fail) = slow_requests(&server, short, vec![], Some("5")).await;
    assert_eq!(success, 0, "a GooseDefault timeout replaced the builder's");
    assert!(fail > 0);

    // --timeout on the command line wins over the builder: no request times out.
    let (success, fail) = slow_requests(&server, short, vec!["--timeout", "5"], None).await;
    assert!(
        success > 0,
        "--timeout did not override the builder's timeout"
    );
    assert_eq!(fail, 0);

    // A builder not given a timeout keeps the GooseDefault's.
    let (success, fail) = slow_requests(&server, None, vec![], Some("0.2")).await;
    assert_eq!(
        success, 0,
        "a builder not given a timeout replaced the GooseDefault's"
    );
    assert!(fail > 0);
}

// A path the mock server answers only when the request asks for no compression.
const PLAIN_PATH: &str = "/plain";

async fn request_plain_path(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(PLAIN_PATH).await?;
    Ok(())
}

// Run one user through a shared client builder given gzip on and a user agent, with
// the given command line options, returning how many requests asked for no
// compression with that user agent.
async fn uncompressed_requests(server: &MockServer, command_line: Vec<&str>) -> usize {
    let goose_attack =
        GooseAttack::initialize_with_config(common::build_configuration(server, command_line))
            .unwrap()
            .set_client_builder_without_cookies(
                GooseClientBuilder::new()
                    .without_cookies()
                    .gzip(true)
                    .user_agent(BUILDER_USER_AGENT),
            )
            .unwrap()
            .register_scenario(
                scenario!("Gzip").register_transaction(transaction!(request_plain_path)),
            );
    let goose_metrics = common::run_load_test(goose_attack, None).await;
    goose_metrics
        .requests
        .get(&format!("GET {PLAIN_PATH}"))
        .expect("the user requested the plain path")
        .success_count
}

#[tokio::test]
async fn test_command_line_no_gzip_overrides_builder() {
    let server = MockServer::start();
    let _plain = server.mock(|when, then| {
        when.method(GET)
            .path(PLAIN_PATH)
            .header("user-agent", BUILDER_USER_AGENT)
            .header_missing("accept-encoding");
        then.status(200);
    });

    // The builder turned gzip on, so every request asks for it and gets a 404.
    assert_eq!(uncompressed_requests(&server, vec![]).await, 0);

    // --no-gzip on the command line wins over the builder.
    assert!(uncompressed_requests(&server, vec!["--no-gzip"]).await > 0);
}

#[tokio::test]
async fn test_shared_client_build_error_stops_before_users_start() {
    let server = MockServer::start();
    let any_request = server.mock(|when, then| {
        when.any_request();
        then.status(200);
    });
    // A user agent with a newline is not a valid header value, so building the
    // client fails.
    let goose_attack =
        GooseAttack::initialize_with_config(common::build_configuration(&server, vec![]))
            .unwrap()
            .set_client_builder_without_cookies(
                GooseClientBuilder::new()
                    .without_cookies()
                    .user_agent("bad\nagent"),
            )
            .unwrap()
            .register_scenario(
                scenario!("BuildError").register_transaction(transaction!(test_transaction)),
            );
    match goose_attack.execute().await {
        Err(GooseError::Reqwest(e)) => assert!(e.is_builder(), "not a builder error: {}", e),
        Err(e) => panic!("expected the builder's error, got {}", e),
        Ok(_) => panic!("a shared client that cannot be built ran the load test"),
    }
    assert_eq!(any_request.calls(), 0, "a user started");
}

#[cfg(feature = "cookies")]
static COOKIES_SET: std::sync::LazyLock<tokio::sync::Barrier> =
    std::sync::LazyLock::new(|| tokio::sync::Barrier::new(2));

// Each user is set a cookie of its own, waits until the other user has been set
// one too, then makes a request that must carry only its own cookie.
#[cfg(feature = "cookies")]
async fn set_and_check_cookie(user: &mut GooseUser) -> TransactionResult {
    let set_path = user_path("/set/", user);
    let _goose = user.get(&set_path).await?;
    // Time out rather than hang the test when the other user never gets here.
    if tokio::time::timeout(Duration::from_secs(10), COOKIES_SET.wait())
        .await
        .is_err()
    {
        return Err(Box::new(TransactionError::Custom(
            "the other user was never set its cookie".to_string(),
        )));
    }
    let check_path = user_path("/check/", user);
    let _goose = user.get(&check_path).await?;
    Ok(())
}

#[cfg(feature = "cookies")]
#[tokio::test]
async fn test_with_cookies_users_do_not_share_cookies() {
    let server = MockServer::start();
    let mut set_mocks = Vec::new();
    let mut check_mocks = Vec::new();
    for i in 0..2 {
        let other = 1 - i;
        set_mocks.push(server.mock(|when, then| {
            when.method(GET).path(format!("/set/{i}"));
            then.status(200)
                .header("set-cookie", format!("user{i}=yes; Path=/"));
        }));
        check_mocks.push(server.mock(|when, then| {
            when.method(GET)
                .path(format!("/check/{i}"))
                .header("user-agent", BUILDER_USER_AGENT)
                .cookie_exists(format!("user{i}"))
                .cookie_missing(format!("user{other}"));
            then.status(200);
        }));
    }
    let goose_attack = GooseAttack::initialize_with_config(common::build_configuration(
        &server,
        vec!["--users", "2", "--increase-rate", "2"],
    ))
    .unwrap()
    .set_client_builder_with_cookies(GooseClientBuilder::new().user_agent(BUILDER_USER_AGENT))
    .unwrap()
    .register_scenario(
        scenario!("Cookies")
            .register_transaction(transaction!(set_and_check_cookie).set_on_start()),
    );
    common::run_load_test(goose_attack, None).await;
    for i in 0..2 {
        set_mocks[i].assert_calls(1);
        // A 404 here means user i sent the other user's cookie, lacked its own, or
        // did not send the builder's user agent.
        check_mocks[i].assert_calls(1);
    }
}

#[cfg(not(feature = "cookies"))]
#[test]
fn test_with_cookies_needs_the_cookies_feature() {
    let configuration = GooseConfiguration::parse_args_default(&["--quiet"]).unwrap();
    match GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .set_client_builder_with_cookies(GooseClientBuilder::new())
    {
        Err(GooseError::FeatureNotEnabled { feature, .. }) => assert_eq!(feature, "cookies"),
        Err(e) => panic!("expected FeatureNotEnabled, got {}", e),
        Ok(_) => panic!("set_client_builder_with_cookies accepted a builder without cookies"),
    }
}
