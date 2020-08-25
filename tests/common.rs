use gumdrop::Options;
use httpmock::MockServer;

use goose::GooseConfiguration;

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
    let server_url = server.url("/");

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
