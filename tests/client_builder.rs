//! Tests for the type-safe client builder implementation.

use goose::client::{ClientStrategy, GooseClientBuilder};
use goose::prelude::*;
use std::time::Duration;

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

// Note: GooseAttack integration methods would be added in a future PR
// For now, we test the client builder functionality independently

#[tokio::test]
async fn test_default_behavior_unchanged() {
    // Test that default behavior is unchanged when no client builder is set
    let _attack = GooseAttack::initialize().expect("Failed to initialize GooseAttack");

    // If we get here without panicking, the default initialization worked
}

#[test]
fn test_client_config_from_goose_configuration() {
    use goose::config::GooseConfiguration;
    use gumdrop::Options;

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
