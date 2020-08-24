use gumdrop::Options;
use httpmock::MockServer;

use goose::GooseConfiguration;

pub fn build_configuration(server: &MockServer) -> GooseConfiguration {
    // Using default options, except for those specified below
    GooseConfiguration::parse_args_default(&[
        // Set --host to server URL
        "--host",
        &server.url("/"),
        // Set --users to 1
        "--users",
        "1",
        // Set --hatch-rate to 1
        "--hatch-rate",
        "1",
        // Set --run-time to 1
        "--run-time",
        "1",
        // Set --no-metrics flag
        "--no-metrics",
        // Set --no-task-metrics flag
        "--no-task-metrics",
    ])
    .unwrap()
}
