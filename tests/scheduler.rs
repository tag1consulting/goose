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
const EXPECT_WORKERS: usize = 4;
// Users needs to be an even number.
const USERS: usize = 18;
const RUN_TIME: usize = 3;

// There are two test variations in this file.
#[derive(Clone)]
enum TestType {
    // Schedule multiple task sets.
    TaskSets,
    // Schedule multiple tasks.
    Tasks,
}

// Test task.
pub async fn one_with_delay(user: &mut GooseUser) -> GooseTaskResult {
    let _goose = user.get(ONE_PATH).await?;

    // "Run out the clock" on the load test when this function runs. Sleep for
    // the total duration the test is to run plus 2 second to be sure no
    // additional tasks will run after this one.
    sleep(Duration::from_secs(RUN_TIME as u64 + 2)).await;

    Ok(())
}

// Test task.
pub async fn two_with_delay(user: &mut GooseUser) -> GooseTaskResult {
    let _goose = user.get(TWO_PATH).await?;

    // "Run out the clock" on the load test when this function runs. Sleep for
    // the total duration the test is to run plus 2 second to be sure no
    // additional tasks will run after this one.
    sleep(Duration::from_secs(RUN_TIME as u64 + 2)).await;

    Ok(())
}

// Test task.
pub async fn three(user: &mut GooseUser) -> GooseTaskResult {
    let _goose = user.get(THREE_PATH).await?;

    Ok(())
}

// Used as a test_start() function, which always runs one time.
pub async fn start_one(user: &mut GooseUser) -> GooseTaskResult {
    let _goose = user.get(START_ONE_PATH).await?;

    Ok(())
}

// Used as a test_stop() function, which always runs one time.
pub async fn stop_one(user: &mut GooseUser) -> GooseTaskResult {
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
fn validate_test(test_type: &TestType, scheduler: &GooseScheduler, mock_endpoints: &[Mock]) {
    // START_ONE_PATH is loaded one and only one time on all variations.
    mock_endpoints[START_ONE_KEY].assert_hits(1);

    match test_type {
        TestType::TaskSets => {
            // Now validate scheduler-specific counters.
            match scheduler {
                GooseScheduler::RoundRobin => {
                    // We launch an equal number of each task set, so we call both endpoints
                    // an equal number of times.
                    mock_endpoints[TWO_KEY].assert_hits(mock_endpoints[ONE_KEY].hits());
                    mock_endpoints[ONE_KEY].assert_hits(USERS / 2);
                }
                GooseScheduler::Serial => {
                    // As we only launch as many users as the weight of the first task set, we only
                    // call the first endpoint, never the second endpoint.
                    mock_endpoints[ONE_KEY].assert_hits(USERS);
                    mock_endpoints[TWO_KEY].assert_hits(0);
                }
                GooseScheduler::Random => {
                    // When scheduling task sets randomly, we don't know how many of each will get
                    // launched, but we do now that added together they will equal the total number
                    // of users.
                    assert!(
                        mock_endpoints[ONE_KEY].hits() + mock_endpoints[TWO_KEY].hits() == USERS
                    );
                }
            }
        }
        TestType::Tasks => {
            // Now validate scheduler-specific counters.
            match scheduler {
                GooseScheduler::RoundRobin => {
                    // Tests are allocated round robin THREE, TWO, ONE. There's no delay
                    // in THREE, so the test runs THREE and TWO which then times things out
                    // and prevents ONE from running.
                    mock_endpoints[ONE_KEY].assert_hits(0);
                    mock_endpoints[TWO_KEY].assert_hits(USERS);
                    mock_endpoints[THREE_KEY].assert_hits(USERS);
                }
                GooseScheduler::Serial => {
                    // Tests are allocated sequentally THREE, TWO, ONE. There's no delay
                    // in THREE and it has a weight of 2, so the test runs THREE twice and
                    // TWO which then times things out and prevents ONE from running.
                    mock_endpoints[ONE_KEY].assert_hits(0);
                    mock_endpoints[TWO_KEY].assert_hits(USERS);
                    mock_endpoints[THREE_KEY].assert_hits(USERS * 2);
                }
                GooseScheduler::Random => {
                    // When scheduling task sets randomly, we don't know how many of each will get
                    // launched, but we do now that added together they will equal the total number
                    // of users (THREE_KEY isn't counted as there's no delay).
                    assert!(
                        mock_endpoints[ONE_KEY].hits() + mock_endpoints[TWO_KEY].hits() == USERS
                    );
                }
            }
        }
    }

    // STOP_ONE_PATH is loaded one and only one time on all variations.
    mock_endpoints[STOP_ONE_KEY].assert_hits(1);
}

// Returns the appropriate taskset, start_task and stop_task needed to build these tests.
fn get_tasksets() -> (Scenario, Scenario, GooseTask, GooseTask) {
    (
        scenario!("TaskSetOne")
            .register_task(task!(one_with_delay))
            .set_weight(USERS)
            .unwrap(),
        scenario!("TaskSetTwo")
            .register_task(task!(two_with_delay))
            // Add one to the weight to avoid this getting reduced by gcd.
            .set_weight(USERS + 1)
            .unwrap(),
        // Start runs before all other tasks, regardless of where defined.
        task!(start_one),
        // Stop runs after all other tasks, regardless of where defined.
        task!(stop_one),
    )
}

// Returns a single Scenario with two GooseTasks, a start_task, and a stop_task.
fn get_tasks() -> (Scenario, GooseTask, GooseTask) {
    (
        scenario!("TaskSet")
            .register_task(task!(three).set_weight(USERS * 2).unwrap())
            .register_task(task!(two_with_delay).set_weight(USERS).unwrap())
            .register_task(task!(one_with_delay).set_weight(USERS).unwrap()),
        // Start runs before all other tasks, regardless of where defined.
        task!(start_one),
        // Stop runs after all other tasks, regardless of where defined.
        task!(stop_one),
    )
}

// Helper to run all standalone tests.
async fn run_standalone_test(test_type: &TestType, scheduler: &GooseScheduler) {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let configuration = common_build_configuration(&server, None, None);

    let goose_attack = match test_type {
        TestType::TaskSets => {
            // Get the tasksets, start and stop tasks to build a load test.
            let (taskset1, taskset2, start_task, stop_task) = get_tasksets();
            // Set up the common base configuration.
            crate::GooseAttack::initialize_with_config(configuration)
                .unwrap()
                .register_scenario(taskset1)
                .register_scenario(taskset2)
                .test_start(start_task)
                .test_stop(stop_task)
                .set_scheduler(scheduler.clone())
        }
        TestType::Tasks => {
            // Get the taskset, start and stop tasks to build a load test.
            let (taskset1, start_task, stop_task) = get_tasks();
            // Set up the common base configuration.
            crate::GooseAttack::initialize_with_config(configuration)
                .unwrap()
                .register_scenario(taskset1)
                .test_start(start_task)
                .test_stop(stop_task)
                .set_scheduler(scheduler.clone())
        }
    };

    // Run the Goose Attack.
    common::run_load_test(goose_attack, None).await;

    // Confirm the load test ran correctly.
    validate_test(test_type, scheduler, &mock_endpoints);
}

// Helper to run all gaggle tests.
async fn run_gaggle_test(test_type: &TestType, scheduler: &GooseScheduler) {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the mock endpoints needed for this test.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Build common configuration.
    let worker_configuration = common_build_configuration(&server, Some(true), None);

    // Workers launched in own threads, store thread handles.
    let worker_handles = match test_type {
        TestType::TaskSets => {
            // Get the tasksets, start and stop tasks to build a load test.
            let (taskset1, taskset2, start_task, stop_task) = get_tasksets();
            common::launch_gaggle_workers(EXPECT_WORKERS, || {
                crate::GooseAttack::initialize_with_config(worker_configuration.clone())
                    .unwrap()
                    .register_scenario(taskset1.clone())
                    .register_scenario(taskset2.clone())
                    .test_start(start_task.clone())
                    .test_stop(stop_task.clone())
                    .set_scheduler(scheduler.clone())
            })
        }
        TestType::Tasks => {
            // Get the taskset, start and stop tasks to build a load test.
            let (taskset1, start_task, stop_task) = get_tasks();
            common::launch_gaggle_workers(EXPECT_WORKERS, || {
                crate::GooseAttack::initialize_with_config(worker_configuration.clone())
                    .unwrap()
                    .register_scenario(taskset1.clone())
                    .test_start(start_task.clone())
                    .test_stop(stop_task.clone())
                    .set_scheduler(scheduler.clone())
            })
        }
    };

    // Build Manager configuration.
    let manager_configuration = common_build_configuration(&server, None, Some(EXPECT_WORKERS));

    let manager_goose_attack = match test_type {
        TestType::TaskSets => {
            // Get the tasksets, start and stop tasks to build a load test.
            let (taskset1, taskset2, start_task, stop_task) = get_tasksets();
            // Build the load test for the Manager.
            crate::GooseAttack::initialize_with_config(manager_configuration)
                .unwrap()
                .register_scenario(taskset1)
                .register_scenario(taskset2)
                .test_start(start_task)
                .test_stop(stop_task)
                .set_scheduler(scheduler.clone())
        }
        TestType::Tasks => {
            // Get the taskset, start and stop tasks to build a load test.
            let (taskset1, start_task, stop_task) = get_tasks();
            // Build the load test for the Manager.
            crate::GooseAttack::initialize_with_config(manager_configuration)
                .unwrap()
                .register_scenario(taskset1)
                .test_start(start_task)
                .test_stop(stop_task)
                .set_scheduler(scheduler.clone())
        }
    };

    // Run the Goose Attack.
    common::run_load_test(manager_goose_attack, Some(worker_handles)).await;

    // Confirm the load test ran correctly.
    validate_test(test_type, scheduler, &mock_endpoints);
}

#[tokio::test]
// Load test with multiple tasks allocating Scenarios in round robin order.
async fn test_round_robin_taskset() {
    run_standalone_test(&TestType::TaskSets, &GooseScheduler::RoundRobin).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Load test with multiple tasks allocating Scenarios in round robin order, in
// Gaggle mode.
async fn test_round_robin_taskset_gaggle() {
    run_gaggle_test(&TestType::TaskSets, &GooseScheduler::RoundRobin).await;
}

#[tokio::test]
// Load test with multiple GooseTasks allocated in round robin order.
async fn test_round_robin_task() {
    run_standalone_test(&TestType::Tasks, &GooseScheduler::RoundRobin).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Load test with multiple GooseTasks allocated in round robin order, in
// Gaggle mode.
async fn test_round_robin_task_gaggle() {
    run_gaggle_test(&TestType::Tasks, &GooseScheduler::RoundRobin).await;
}

#[tokio::test]
// Load test with multiple tasks allocating Scenarios in serial order.
async fn test_serial_taskset() {
    run_standalone_test(&TestType::TaskSets, &GooseScheduler::Serial).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Load test with multiple tasks allocating Scenarios in serial order, in
// Gaggle mode.
async fn test_serial_taskset_gaggle() {
    run_gaggle_test(&TestType::TaskSets, &GooseScheduler::Serial).await;
}

#[tokio::test]
// Load test with multiple GooseTasks allocated in serial order.
async fn test_serial_tasks() {
    run_standalone_test(&TestType::Tasks, &GooseScheduler::Serial).await;
}

#[tokio::test]
// Load test with multiple tasks allocating Scenarios in random order.
async fn test_random_taskset() {
    run_standalone_test(&TestType::TaskSets, &GooseScheduler::Random).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Load test with multiple tasks allocating Scenarios in random order, in
// Gaggle mode.
async fn test_random_taskset_gaggle() {
    run_gaggle_test(&TestType::TaskSets, &GooseScheduler::Random).await;
}

#[tokio::test]
// Load test with multiple tasks allocating Scenarios in random order.
async fn test_random_tasks() {
    run_standalone_test(&TestType::Tasks, &GooseScheduler::Random).await;
}
