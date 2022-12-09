use httpmock::{Method::GET, Mock, MockServer};
use serial_test::serial;
use tokio::time::{sleep, Duration};

mod common;

use goose::config::GooseConfiguration;
use goose::prelude::*;

// Paths used in load tests performed during these tests.
const ONE_PATH: &str = "/one";
const TWO_PATH: &str = "/two";
const THREE_PATH: &str = "/three";
const START_ONE_PATH: &str = "/start/one";
const STOP_ONE_PATH: &str = "/stop/one";

// Indexes to the above paths.
const ONE_KEY: usize = 0;
const TWO_KEY: usize = 1;
const THREE_KEY: usize = 2;
const START_ONE_KEY: usize = 3;
const STOP_ONE_KEY: usize = 4;

// Load test configuration.
const EXPECT_WORKERS: usize = 2;
const USERS: usize = 4;
const RUN_TIME: usize = 2;

// There are multiple test variations in this file.
#[derive(Clone)]
enum TestType {
    // No sequences defined in load test.
    NotSequenced,
    // Sequences defined in load test, scheduled round robin.
    SequencedRoundRobin,
    // Sequences defined in load test, scheduled serially.
    SequencedSerial,
}

// Test transaction.
pub async fn one(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(ONE_PATH).await?;

    Ok(())
}

// Test transaction.
pub async fn two_with_delay(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(TWO_PATH).await?;

    // "Run out the clock" on the load test when this function runs. Sleep for
    // the total duration the test is to run plus 2 seconds to be sure no
    // additional transactions will run after this one.
    sleep(Duration::from_secs(RUN_TIME as u64 + 2)).await;

    Ok(())
}

// Test transaction.
pub async fn three(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(THREE_PATH).await?;

    Ok(())
}

// Used as a test_start() function, which always runs one time.
pub async fn start_one(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(START_ONE_PATH).await?;

    Ok(())
}

// Used as a test_stop() function, which always runs one time.
pub async fn stop_one(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(STOP_ONE_PATH).await?;

    Ok(())
}

// All tests in this file run against common endpoints.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<Mock> {
    vec![
        // First set up ONE_PATH, store in vector at ONE_KEY.
        server.mock(|when, then| {
            when.method(GET).path(ONE_PATH);
            then.status(200);
        }),
        // Next set up TWO_PATH, store in vector at TWO_KEY.
        server.mock(|when, then| {
            when.method(GET).path(TWO_PATH);
            then.status(200);
        }),
        // Next set up THREE_PATH, store in vector at THREE_KEY.
        server.mock(|when, then| {
            when.method(GET).path(THREE_PATH);
            then.status(200);
        }),
        // Next set up START_ONE_PATH, store in vector at START_ONE_KEY.
        server.mock(|when, then| {
            when.method(GET).path(START_ONE_PATH);
            then.status(200);
        }),
        // Next set up STOP_ONE_PATH, store in vector at STOP_ONE_KEY.
        server.mock(|when, then| {
            when.method(GET).path(STOP_ONE_PATH);
            then.status(200);
        }),
    ]
}

// Build appropriate configuration for these tests.
fn common_build_configuration(
    server: &MockServer,
    worker: Option<bool>,
    manager: Option<usize>,
) -> GooseConfiguration {
    if let Some(expect_workers) = manager {
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
                "--no-reset-metrics",
            ],
        )
    } else if worker.is_some() {
        common::build_configuration(server, vec!["--worker"])
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
                "--no-reset-metrics",
            ],
        )
    }
}

// Helper to confirm all variations generate appropriate results.
fn validate_test(test_type: &TestType, mock_endpoints: &[Mock]) {
    // START_ONE_PATH is loaded one and only one time on all variations.
    mock_endpoints[START_ONE_KEY].assert_hits(1);

    // Now confirm TestType-specific counters.
    match test_type {
        TestType::NotSequenced => {
            // All transactions run one time, as they are launched RoundRobin in the order
            // defined (and importantly three is defined before two in this test).
            mock_endpoints[ONE_KEY].assert_hits(USERS);
            mock_endpoints[THREE_KEY].assert_hits(USERS);
            mock_endpoints[TWO_KEY].assert_hits(USERS);
        }
        TestType::SequencedRoundRobin => {
            // Transaction ONE runs twice as it's scheduled first with a weight of 2. It then
            // runs one more time in the next scheduling as it then round robins between
            // ONE and TWO. When TWO runs it runs out the clock.
            mock_endpoints[ONE_KEY].assert_hits(USERS * 3);
            // Two runs out the clock, so three never runs.
            mock_endpoints[TWO_KEY].assert_hits(USERS);
            mock_endpoints[THREE_KEY].assert_hits(0);
        }
        TestType::SequencedSerial => {
            // Transaction ONE runs twice as it's scheduled first with a weight of 2. It then
            // runs two more times in the next scheduling as runs transaction serially as
            // defined.
            mock_endpoints[ONE_KEY].assert_hits(USERS * 4);
            // Two runs out the clock, so three never runs.
            mock_endpoints[TWO_KEY].assert_hits(USERS);
            mock_endpoints[THREE_KEY].assert_hits(0);
        }
    }

    // STOP_ONE_PATH is loaded one and only one time on all variations.
    mock_endpoints[STOP_ONE_KEY].assert_hits(1);
}

// Returns the appropriate scenario, start_transaction and stop_transaction needed to build these tests.
fn get_transactions(test_type: &TestType) -> (Scenario, Transaction, Transaction) {
    match test_type {
        // No sequence declared, so transactions run in default RoundRobin order: 1, 3, 2, 1...
        TestType::NotSequenced => (
            scenario!("LoadTest")
                .register_transaction(transaction!(one).set_weight(2).unwrap())
                .register_transaction(transaction!(three))
                .register_transaction(transaction!(two_with_delay)),
            // Start runs before all other transactions, regardless of where defined.
            transaction!(start_one),
            // Stop runs after all other transactions, regardless of where defined.
            transaction!(stop_one),
        ),
        // Sequence added, so transactions run in the declared sequence order: 1, 1, 2, 3...
        TestType::SequencedRoundRobin => (
            scenario!("LoadTest")
                .register_transaction(transaction!(one).set_sequence(1).set_weight(2).unwrap())
                .register_transaction(transaction!(three).set_sequence(3))
                .register_transaction(transaction!(one).set_sequence(2).set_weight(2).unwrap())
                .register_transaction(transaction!(two_with_delay).set_sequence(2)),
            // Start runs before all other transactions, regardless of where defined.
            transaction!(start_one),
            // Stop runs after all other transactions, regardless of where defined.
            transaction!(stop_one),
        ),
        TestType::SequencedSerial => (
            scenario!("LoadTest")
                .register_transaction(transaction!(one).set_sequence(1).set_weight(2).unwrap())
                .register_transaction(transaction!(three).set_sequence(3))
                .register_transaction(transaction!(one).set_sequence(2).set_weight(2).unwrap())
                .register_transaction(transaction!(two_with_delay).set_sequence(2)),
            // Start runs before all other transactions, regardless of where defined.
            transaction!(start_one),
            // Stop runs after all other transactions, regardless of where defined.
            transaction!(stop_one),
        ),
    }
}

// Helper to run all standalone tests.
async fn run_standalone_test(test_type: TestType) {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let configuration = common_build_configuration(&server, None, None);

    // Get the scenario, start and stop transactions to build a load test.
    let (scenario, start_transaction, stop_transaction) = get_transactions(&test_type);

    let goose_attack = match test_type {
        TestType::NotSequenced | TestType::SequencedRoundRobin => {
            // Set up the common base configuration.
            crate::GooseAttack::initialize_with_config(configuration)
                .unwrap()
                .register_scenario(scenario)
                .test_start(start_transaction)
                .test_stop(stop_transaction)
                .set_scheduler(GooseScheduler::RoundRobin)
        }
        TestType::SequencedSerial => {
            // Set up the common base configuration.
            crate::GooseAttack::initialize_with_config(configuration)
                .unwrap()
                .register_scenario(scenario)
                .test_start(start_transaction)
                .test_stop(stop_transaction)
                .set_scheduler(GooseScheduler::Serial)
        }
    };

    // Run the Goose Attack.
    common::run_load_test(goose_attack, None).await;

    // Confirm the load test ran correctly.
    validate_test(&test_type, &mock_endpoints);
}

// Helper to run all gaggle tests.
async fn run_gaggle_test(test_type: TestType) {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let worker_configuration = common_build_configuration(&server, Some(true), None);

    // Get the scenario, start and stop transactions to build a load test.
    let (scenario, start_transaction, stop_transaction) = get_transactions(&test_type);

    // Workers launched in own threads, store thread handles.
    let worker_handles = match test_type {
        TestType::NotSequenced | TestType::SequencedRoundRobin => {
            // Set up the common base configuration.
            common::launch_gaggle_workers(EXPECT_WORKERS, || {
                crate::GooseAttack::initialize_with_config(worker_configuration.clone())
                    .unwrap()
                    .register_scenario(scenario.clone())
                    .test_start(start_transaction.clone())
                    .test_stop(stop_transaction.clone())
                    // Unnecessary as this is the default.
                    .set_scheduler(GooseScheduler::RoundRobin)
            })
        }
        TestType::SequencedSerial => common::launch_gaggle_workers(EXPECT_WORKERS, || {
            crate::GooseAttack::initialize_with_config(worker_configuration.clone())
                .unwrap()
                .register_scenario(scenario.clone())
                .test_start(start_transaction.clone())
                .test_stop(stop_transaction.clone())
                .set_scheduler(GooseScheduler::Serial)
        }),
    };

    // Build Manager configuration.
    let manager_configuration = common_build_configuration(&server, None, Some(EXPECT_WORKERS));

    let manager_goose_attack = match test_type {
        TestType::NotSequenced | TestType::SequencedRoundRobin => {
            // Set up the common base configuration.
            crate::GooseAttack::initialize_with_config(manager_configuration)
                .unwrap()
                .register_scenario(scenario)
                .test_start(start_transaction)
                .test_stop(stop_transaction)
                // Unnecessary as this is the default.
                .set_scheduler(GooseScheduler::RoundRobin)
        }
        TestType::SequencedSerial => {
            // Set up the common base configuration.
            crate::GooseAttack::initialize_with_config(manager_configuration)
                .unwrap()
                .register_scenario(scenario)
                .test_start(start_transaction)
                .test_stop(stop_transaction)
                .set_scheduler(GooseScheduler::Serial)
        }
    };

    // Run the Goose Attack.
    common::run_load_test(manager_goose_attack, Some(worker_handles)).await;

    // Confirm the load test ran correctly.
    validate_test(&test_type, &mock_endpoints);
}

#[tokio::test]
// Load test with multiple transactions and no sequences defined.
async fn test_not_sequenced() {
    run_standalone_test(TestType::NotSequenced).await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Load test with multiple transactions and no sequences defined, in Gaggle mode.
async fn test_not_sequenced_gaggle() {
    run_gaggle_test(TestType::NotSequenced).await;
}

#[tokio::test]
// Load test with multiple transactions and sequences defined, using the
// round robin scheduler.
async fn test_sequenced_round_robin() {
    run_standalone_test(TestType::SequencedRoundRobin).await;
}

#[tokio::test]
// Load test with multiple transactions and sequences defined, using the
// sequential scheduler.
async fn test_sequenced_sequential() {
    run_standalone_test(TestType::SequencedSerial).await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Load test with multiple transactions and sequences defined, using the
// round robin scheduler, in Gaggle mode.
async fn test_sequenced_round_robin_gaggle() {
    run_gaggle_test(TestType::SequencedRoundRobin).await;
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
// Load test with multiple transactions and sequences defined, using the
// sequential scheduler, in Gaggle mode.
async fn test_sequenced_sequential_gaggle() {
    run_gaggle_test(TestType::SequencedSerial).await;
}
