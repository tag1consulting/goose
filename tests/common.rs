use gumdrop::Options;
use httpmock::MockServer;
use std::io::{self, BufRead};

use goose::goose::{GooseTask, GooseTaskSet};
use goose::metrics::GooseMetrics;
use goose::{GooseAttack, GooseConfiguration};

type WorkerHandles = Vec<std::thread::JoinHandle<()>>;

/// Not all functions are used by all tests, so we enable allow(dead_code) to avoid
/// compiler warnings during testing.

/// The following options are configured by default, if not set to a custom value
/// and if not building a Worker configuration:
///  --host <mock-server>
///  --users 1
///  --hatch-rate 1
///  --run-time 1
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
    }

    // Parse these options to generate a GooseConfiguration.
    GooseConfiguration::parse_args_default(&configuration)
        .expect("failed to parse options and generate a configuration")
}

/// Launch each Worker in its own thread, and return a vector of Worker handles.
#[allow(dead_code)]
pub fn launch_gaggle_workers(
    // A goose attack object which is cloned for each Worker.
    goose_attack: GooseAttack,
    // The number of Workers to launch.
    expect_workers: usize,
) -> WorkerHandles {
    // Launch each worker in its own thread, storing the join handles.
    let mut worker_handles = Vec::new();
    for _ in 0..expect_workers {
        let worker_goose_attack = goose_attack.clone();
        // Start worker instance of the load test.
        worker_handles.push(std::thread::spawn(move || {
            // Run the load test as configured.
            run_load_test(worker_goose_attack, None);
        }));
    }

    worker_handles
}

// Create a GooseAttack object from the configuration, taskset, and optional start and
// stop tasks.
#[allow(dead_code)]
pub fn build_load_test(
    configuration: GooseConfiguration,
    taskset: &GooseTaskSet,
    start_task: Option<&GooseTask>,
    stop_task: Option<&GooseTask>,
) -> GooseAttack {
    // First set up the common base configuration.
    let mut goose = crate::GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_taskset(taskset.clone());

    if let Some(task) = start_task {
        goose = goose.test_start(task.clone());
    }

    if let Some(task) = stop_task {
        goose = goose.test_stop(task.clone());
    }

    goose
}

/// Run the actual load test, returning the GooseMetrics.
pub fn run_load_test(
    goose_attack: GooseAttack,
    worker_handles: Option<WorkerHandles>,
) -> GooseMetrics {
    // Execute the load test.
    let goose_metrics = goose_attack.execute().unwrap();

    // If this is a Manager test, first wait for the Workers to exit to return.
    if let Some(handles) = worker_handles {
        // Wait for both worker threads to finish and exit.
        for handle in handles {
            let _ = handle.join();
        }
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
