use futures::future::join_all;
use gumdrop::Options;
use httpmock::MockServer;
use std::io::{self, BufRead};

use goose::config::GooseConfiguration;
use goose::goose::{Scenario, Transaction};
use goose::metrics::GooseMetrics;
use goose::GooseAttack;

type WorkerHandles = Vec<tokio::task::JoinHandle<GooseMetrics>>;

/// Not all functions are used by all tests, so we enable allow(dead_code) to avoid
/// compiler warnings during testing.

/// The following options are configured by default, if not set to a custom value
/// and if not building a Worker configuration:
///  --host <mock-server>
///  --users 1
///  --hatch-rate 1
///  --run-time 1
///  --co-mitigation disabled
#[allow(dead_code)]
pub fn build_configuration(server: &MockServer, custom: Vec<&str>) -> GooseConfiguration {
    // Start with an empty configuration.
    let mut configuration: Vec<&str> = vec![];
    // Declare server_url here no matter what, so its lifetime is sufficient when needed.
    let server_url = server.base_url();

    // Merge in all custom options first.
    configuration.extend_from_slice(&custom);

    // If not building a Worker configuration, set some defaults.
    if !configuration.contains(&"--worker") {
        // Default to using mock server if not otherwise configured.
        if !configuration.contains(&"--host") {
            configuration.extend_from_slice(&["--host", &server_url]);
        }

        // Default to testing with 1 user if not otherwise configured.
        if !configuration.contains(&"--users") {
            configuration.extend_from_slice(&["--users", "1"]);
        }

        // Default to hatch 1 user per second if not otherwise configured.
        if !configuration.contains(&"--hatch-rate") {
            configuration.extend_from_slice(&["--hatch-rate", "1"]);
        }

        // Default to running for 1 second if not otherwise configured.
        if !configuration.contains(&"--run-time") {
            configuration.extend_from_slice(&["--run-time", "1"]);
        }

        // Default to disabling coordinated omission mitigation if not otherwise configured
        if !configuration.contains(&"--co-mitigation") {
            configuration.extend_from_slice(&["--co-mitigation", "disabled"]);
        }
    }

    // Disable verbose output when running tests.
    configuration.extend_from_slice(&["--quiet"]);

    // Parse these options to generate a GooseConfiguration.
    GooseConfiguration::parse_args_default(&configuration)
        .expect("failed to parse options and generate a configuration")
}

/// Launch each Worker in its own thread, and return a vector of Worker handles.
#[allow(dead_code)]
pub fn launch_gaggle_workers<F: Fn() -> GooseAttack>(
    // The number of Workers to launch.
    expect_workers: usize,
    // A goose attack object which is cloned for each Worker.
    goose_attack_provider: F,
) -> WorkerHandles {
    // Launch each worker in its own thread, storing the join handles.
    let mut worker_handles = Vec::new();
    for _ in 0..expect_workers {
        let worker_goose_attack = goose_attack_provider();
        // Start worker instance of the load test.
        worker_handles.push(tokio::spawn(run_load_test(worker_goose_attack, None)));
    }

    worker_handles
}

// Create a GooseAttack object from the configuration, Scenarios, and optional start and
// stop Transactions.
#[allow(dead_code)]
pub fn build_load_test(
    configuration: GooseConfiguration,
    scenarios: Vec<Scenario>,
    start_transaction: Option<&Transaction>,
    stop_transaction: Option<&Transaction>,
) -> GooseAttack {
    // First set up the common base configuration.
    let mut goose = crate::GooseAttack::initialize_with_config(configuration).unwrap();

    for scenario in scenarios {
        goose = goose.register_scenario(scenario.clone());
    }

    if let Some(transaction) = start_transaction {
        goose = goose.test_start(transaction.clone());
    }

    if let Some(transaction) = stop_transaction {
        goose = goose.test_stop(transaction.clone());
    }

    goose
}

/// Run the actual load test, returning the GooseMetrics.
pub async fn run_load_test(
    goose_attack: GooseAttack,
    worker_handles: Option<WorkerHandles>,
) -> GooseMetrics {
    // Execute the load test.
    let goose_metrics = goose_attack.execute().await.unwrap();

    // If this is a Manager test, first wait for the Workers to exit to return.
    if let Some(handles) = worker_handles {
        // Wait for both worker threads to finish and exit.
        join_all(handles).await;
    }

    goose_metrics
}

/// Helper to count the number of lines in a test artifact.
#[allow(dead_code)]
pub fn file_length(file_name: &str) -> usize {
    if let Ok(file) = std::fs::File::open(std::path::Path::new(file_name)) {
        io::BufReader::new(file).lines().count()
    } else {
        0
    }
}

/// Helper to delete test artifacts, if existing.
#[allow(dead_code)]
pub fn cleanup_files(files: Vec<&str>) {
    for file in files {
        if std::path::Path::new(file).exists() {
            std::fs::remove_file(file).expect("failed to remove file");
        }
    }
}
