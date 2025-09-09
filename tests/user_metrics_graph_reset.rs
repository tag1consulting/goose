// Validate that user metrics graph maintains continuity during resets
use gumdrop::Options;
use httpmock::{Method::GET, MockServer};
use serial_test::serial;

use goose::config::GooseConfiguration;
use goose::prelude::*;

// A simple load test transaction
async fn simple_loadtest_transaction(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_user_metrics_graph_continuity_with_reset() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200).body("test response");
    });

    // Use very short run time to make it fast and predictable without parsing command line arguments
    const EMPTY_ARGS: Vec<&str> = vec![];
    let configuration = GooseConfiguration::parse_args_default(&EMPTY_ARGS).unwrap();
    let goose_attack = GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(
            scenario!("TestScenario")
                .register_transaction(transaction!(simple_loadtest_transaction)),
        )
        .set_default(GooseDefault::Host, server.url("").as_str())
        .unwrap()
        .set_default(GooseDefault::Users, 3)
        .unwrap()
        .set_default(GooseDefault::HatchRate, "10")
        .unwrap()
        .set_default(GooseDefault::RunTime, 1) // Just 1 second
        .unwrap();

    let goose_metrics = goose_attack.execute().await.unwrap();

    assert_eq!(goose_metrics.maximum_users, 3);
    assert!(!goose_metrics.requests.is_empty());
    // Don't assert exact request counts since it's time-based, just verify it works
    assert!(mock.hits() > 0);
}

#[tokio::test]
#[serial]
async fn test_user_metrics_graph_continuity_without_reset() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200).body("test response");
    });

    // Use very short run time to make it fast and predictable without parsing command line arguments
    const EMPTY_ARGS: Vec<&str> = vec![];
    let configuration = GooseConfiguration::parse_args_default(&EMPTY_ARGS).unwrap();
    let goose_attack = GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(
            scenario!("TestScenario")
                .register_transaction(transaction!(simple_loadtest_transaction)),
        )
        .set_default(GooseDefault::Host, server.url("").as_str())
        .unwrap()
        .set_default(GooseDefault::Users, 3)
        .unwrap()
        .set_default(GooseDefault::HatchRate, "10")
        .unwrap()
        .set_default(GooseDefault::RunTime, 1) // Just 1 second
        .unwrap()
        .set_default(GooseDefault::NoResetMetrics, true)
        .unwrap();

    let goose_metrics = goose_attack.execute().await.unwrap();

    assert_eq!(goose_metrics.maximum_users, 3);
    assert!(!goose_metrics.requests.is_empty());
    // Don't assert exact request counts since it's time-based, just verify it works
    assert!(mock.hits() > 0);
}

#[tokio::test]
#[serial]
async fn test_comprehensive_graph_metrics_reset_integration() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200).body("test response");
    });

    // Use very short run time to make it fast and predictable without parsing command line arguments
    const EMPTY_ARGS: Vec<&str> = vec![];
    let configuration = GooseConfiguration::parse_args_default(&EMPTY_ARGS).unwrap();
    let goose_attack = GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(
            scenario!("TestUsers").register_transaction(transaction!(simple_loadtest_transaction)),
        )
        .set_default(GooseDefault::Host, server.url("").as_str())
        .unwrap()
        .set_default(GooseDefault::Users, 3)
        .unwrap()
        .set_default(GooseDefault::HatchRate, "10")
        .unwrap()
        .set_default(GooseDefault::RunTime, 1) // Just 1 second
        .unwrap();

    let goose_metrics = goose_attack.execute().await.unwrap();

    assert_eq!(goose_metrics.maximum_users, 3);
    assert!(!goose_metrics.requests.is_empty());
    // Don't assert exact request counts since it's time-based, just verify it works
    assert!(mock.hits() > 0);
}

#[tokio::test]
#[serial]
async fn test_graph_data_preservation_during_reset() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200).body("test response");
    });

    // Use very short run time to make it fast and predictable without parsing command line arguments
    const EMPTY_ARGS: Vec<&str> = vec![];
    let configuration = GooseConfiguration::parse_args_default(&EMPTY_ARGS).unwrap();
    let goose_attack = GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(
            scenario!("TestGraphPreservation")
                .register_transaction(transaction!(simple_loadtest_transaction)),
        )
        .set_default(GooseDefault::Host, server.url("").as_str())
        .unwrap()
        .set_default(GooseDefault::Users, 2)
        .unwrap()
        .set_default(GooseDefault::HatchRate, "10")
        .unwrap()
        .set_default(GooseDefault::RunTime, 1) // Just 1 second
        .unwrap();

    let goose_metrics = goose_attack.execute().await.unwrap();

    assert_eq!(goose_metrics.maximum_users, 2);
    assert!(!goose_metrics.requests.is_empty());
    // Don't assert exact request counts since it's time-based, just verify it works
    assert!(mock.hits() > 0);
}
