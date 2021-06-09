use gumdrop::Options;
use httpmock::{Method::GET, MockRef, MockServer};
use serial_test::serial;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::{str, thread, time};

use goose::controller::GooseControllerCommand;
use goose::prelude::*;
use goose::GooseConfiguration;

mod common;

// Paths used in load tests performed during these tests.
const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about.html";

// Indexes to the above paths.
const INDEX_KEY: usize = 0;
const ABOUT_KEY: usize = 1;

const USERS: usize = 5;
const HATCH_RATE: usize = 10;
const RUN_TIME: usize = 10;

// There are multiple test variations in this file.
#[derive(Clone)]
enum TestType {
    // Both controllers enabled.
    Both,
    // Enable --no-telnet.
    //NoTelnet,
    // Enable --no-websocket.
    NoWebSocket,
}

// State machine for tracking Controller state during tests.
struct TestState {
    buf: [u8; 2048],
    position: usize,
    step: usize,
    command: GooseControllerCommand,
    stream: TcpStream,
}

// Test task.
pub async fn get_index(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// Test task.
pub async fn get_about(user: &GooseUser) -> GooseTaskResult {
    let _goose = user.get(ABOUT_PATH).await?;
    Ok(())
}

// All tests in this file run against the following common endpoints.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<MockRef> {
    vec![
        // First set up INDEX_PATH, store in vector at INDEX_KEY.
        server.mock(|when, then| {
            when.method(GET).path(INDEX_PATH);
            then.status(200);
        }),
        // Next set up ABOUT_PATH, store in vector at ABOUT_KEY.
        server.mock(|when, then| {
            when.method(GET).path(ABOUT_PATH);
            then.status(200);
        }),
    ]
}

// Build appropriate configuration for these tests. Normally this also calls
// common::build_configuration() to get defaults most all tests needs, but
// for these tests we don't want a default configuration. We keep the signature
// the same to simplify reuse, accepting the MockServer but not using it.
fn common_build_configuration(_server: &MockServer, custom: &mut Vec<&str>) -> GooseConfiguration {
    // Common elements in all our tests.
    let mut configuration: Vec<&str> = vec!["--no-autostart"];

    // Custom elements in some tests.
    configuration.append(custom);

    // Parse these options to generate a GooseConfiguration.
    GooseConfiguration::parse_args_default(&configuration)
        .expect("failed to parse options and generate a configuration")
}

// Helper to confirm all variations generate appropriate results.
fn validate_one_taskset(
    goose_metrics: &GooseMetrics,
    mock_endpoints: &[MockRef],
    configuration: &GooseConfiguration,
    _test_type: TestType,
) {
    //println!("goose_metrics: {:#?}", goose_metrics);
    //println!("configuration: {:#?}", configuration);

    // Confirm that we loaded the mock endpoints.
    assert!(mock_endpoints[INDEX_KEY].hits() > 0);
    assert!(mock_endpoints[ABOUT_KEY].hits() > 0);

    // Get index and about out of goose metrics.
    let index_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", INDEX_PATH))
        .unwrap();
    let about_metrics = goose_metrics
        .requests
        .get(&format!("GET {}", ABOUT_PATH))
        .unwrap();

    // There should not have been any failures during this test.
    assert!(index_metrics.fail_count == 0);
    assert!(about_metrics.fail_count == 0);

    // Users were correctly configured through the controller.
    assert!(goose_metrics.users == USERS);

    // Host was not configured at start time.
    assert!(configuration.host.is_empty());

    // The load test was manually shut down instead of running to completion.
    assert!(goose_metrics.duration < RUN_TIME);
}

// Returns the appropriate taskset needed to build these tests.
fn get_tasks() -> GooseTaskSet {
    taskset!("LoadTest")
        .register_task(task!(get_index).set_weight(2).unwrap())
        .register_task(task!(get_about).set_weight(1).unwrap())
}

// Helper to run all standalone tests.
fn run_standalone_test(test_type: TestType) {
    // Start the mock server.
    let server = MockServer::start();
    let server_url = server.base_url();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    let mut configuration_flags = match test_type {
        TestType::Both => vec![],
        //TestType::NoTelnet => vec!["--no-telnet"],
        TestType::NoWebSocket => vec!["--no-websocket"],
    };

    // Build common configuration elements.
    let configuration = common_build_configuration(&server, &mut configuration_flags);

    // Create a new thread from which to test the Controller.
    let _controller_handle = thread::spawn(move || {
        // Sleep a quarter of a second allowing the GooseAttack to start.
        thread::sleep(time::Duration::from_millis(250));

        // Initiailize the state engine.
        let mut test_state = update_state(None);
        loop {
            // Process data received from the client in a loop.
            let _ = match test_state.stream.read(&mut test_state.buf) {
                Ok(data) => data,
                Err(_) => {
                    panic!("ERROR: server disconnected!");
                }
            };

            let response = str::from_utf8(&test_state.buf).unwrap();
            match test_state.command {
                GooseControllerCommand::Help => {
                    match test_state.step {
                        0 => {
                            // Request the help text.
                            make_request(&mut test_state, "help\r\n");
                        }
                        1 => {
                            // Be sure we actually received the help text.
                            assert!(response.contains("controller commands:"));

                            // Request the help text, using the short form.
                            make_request(&mut test_state, "?\r\n");
                        }
                        _ => {
                            // Be sure we actually received the help text.
                            assert!(response.contains("controller commands:"));

                            // Move onto the next command.
                            test_state = update_state(Some(test_state));
                        }
                    }
                }
                GooseControllerCommand::Host => {
                    match test_state.step {
                        // Set the host to be load tested.
                        0 => {
                            make_request(&mut test_state, &["host ", &server_url, "\r\n"].concat());
                        }
                        // Confirm the host was configured.
                        1 => {
                            assert!(response.starts_with("host configured"));

                            // Then try and set an invalid host.
                            make_request(&mut test_state, "host foobar\r\n");
                        }
                        // Confirm that we can't configure an invalid host that doesn't
                        // match the regex.
                        2 => {
                            assert!(response.starts_with("unrecognized command"));

                            // Try again to set an invalid host.
                            make_request(&mut test_state, "host http://$[foo\r\n");
                        }
                        // Confirm that we can't configure an invalid host that does
                        // match the regex.
                        _ => {
                            assert!(response.starts_with("unrecognized command"));

                            // Move onto the next command.
                            test_state = update_state(Some(test_state));
                        }
                    }
                }
                GooseControllerCommand::Users => {
                    match test_state.step {
                        // Reconfigure the number of users simulated by the load test.
                        0 => {
                            make_request(
                                &mut test_state,
                                &["users ", &USERS.to_string(), "\r\n"].concat(),
                            );
                        }
                        // Confirm that the users are reconfigured.
                        1 => {
                            assert!(response.starts_with("users configured"));

                            // Attempt to reconfigure users with bad data.
                            make_request(&mut test_state, "users 1.1\r\n");
                        }
                        // Confirm we can't configure users with a float.
                        _ => {
                            // The number of users started is verified when the load test finishes,
                            // so no further validation required here.
                            assert!(response.starts_with("unrecognized command"));

                            // Move onto the next command.
                            test_state = update_state(Some(test_state));
                        }
                    }
                }
                GooseControllerCommand::HatchRate => {
                    match test_state.step {
                        // Configure a decimal hatch_rate.
                        0 => {
                            make_request(&mut test_state, "hatchrate .1\r\n");
                        }
                        // Confirm hatch_rate is configured.
                        1 => {
                            assert!(response.starts_with("hatch_rate configured"));

                            // Configure with leading and trailing zeros.
                            make_request(&mut test_state, "hatchrate 0.90\r\n");
                        }
                        // Confirm hatch_rate is configured.
                        2 => {
                            assert!(response.starts_with("hatch_rate configured"));

                            // Try to configure with an invalid decimal.
                            make_request(&mut test_state, "hatchrate 1.2.3\r\n");
                        }
                        // Confirm hatch_rate is not configured.
                        3 => {
                            assert!(response.starts_with("unrecognized command"));

                            // Configure hatch_rate with a single integer.
                            make_request(
                                &mut test_state,
                                &["hatchrate ", &HATCH_RATE.to_string(), "\r\n"].concat(),
                            );
                        }
                        // Confirm the final hatch_rate is configured.
                        _ => {
                            assert!(response.starts_with("hatch_rate configured"));

                            // The hatch_rate is verified when the load test finishes, so no
                            // further validation required here.

                            // Move onto the next command.
                            test_state = update_state(Some(test_state));
                        }
                    }
                }
                GooseControllerCommand::RunTime => {
                    match test_state.step {
                        // Configure run_time using h:m:s format.
                        0 => {
                            // Set run_time with hours and minutes and seconds.
                            make_request(&mut test_state, "runtime 1h2m3s\r\n");
                        }
                        // Confirm the run_time is configured.
                        1 => {
                            assert!(response.starts_with("run_time configured"));

                            // Set run_time with hours and seconds.
                            make_request(&mut test_state, "run_time 1h2s\r\n");
                        }
                        // Confirm the run_time is configured.
                        2 => {
                            assert!(response.starts_with("run_time configured"));

                            // Set run_time with hours alone.
                            make_request(&mut test_state, "run-time 1h\r\n");
                        }
                        // Confirm the run_time is configured.
                        3 => {
                            assert!(response.starts_with("run_time configured"));

                            // Set run_time with seconds alone.
                            make_request(&mut test_state, "runtime 10s\r\n");
                        }
                        // Confirm the run_time is configured.
                        4 => {
                            assert!(response.starts_with("run_time configured"));

                            // Try to set run_time with unsupported value.
                            make_request(&mut test_state, "runtime 10d\r\n");
                        }
                        // Confirm the run_time is not configured.
                        5 => {
                            assert!(response.starts_with("unrecognized command"));

                            // Set run_time with seconds alone, and no "s".
                            make_request(
                                &mut test_state,
                                &["runtime ", &RUN_TIME.to_string(), "\r\n"].concat(),
                            );
                        }
                        // Confirm the run_time is configured.
                        _ => {
                            assert!(response.starts_with("run_time configured"));

                            // The run_time is verified when the load test finishes, so no
                            // further validation required here. Unfortunately, if this fails
                            // the load test could run forever.

                            // Move onto the next command.
                            test_state = update_state(Some(test_state));
                        }
                    }
                }
                GooseControllerCommand::Config => {
                    match test_state.step {
                        // Request the configuration.
                        0 => {
                            make_request(&mut test_state, "config\r\n");
                        }
                        // Confirm the configuration object is returned.
                        _ => {
                            assert!(response.starts_with(r"GooseConfiguration "));

                            // Move onto the next command.
                            test_state = update_state(Some(test_state));
                        }
                    }
                }
                GooseControllerCommand::ConfigJson => {
                    match test_state.step {
                        // Request the configuration in json format.
                        0 => {
                            make_request(&mut test_state, "config-json\r\n");
                        }
                        // Confirm the configuration is returned in jsonformat.
                        _ => {
                            assert!(response
                                .starts_with(r#"{"help":false,"version":false,"list":false,"#));

                            // Move onto the next command.
                            test_state = update_state(Some(test_state));
                        }
                    }
                }
                GooseControllerCommand::Metrics => {
                    match test_state.step {
                        // Request the running metrics.
                        0 => {
                            make_request(&mut test_state, "metrics\r\n");
                        }
                        // Confirm the metrics are returned and pretty-printed.
                        _ => {
                            assert!(response.contains("=== PER TASK METRICS ==="));

                            // Move onto the next command.
                            test_state = update_state(Some(test_state));
                        }
                    }
                }
                GooseControllerCommand::MetricsJson => {
                    match test_state.step {
                        // Request the running metrics in json format.
                        0 => {
                            make_request(&mut test_state, "metrics-json\r\n");
                        }
                        // Confirm the metrics are returned in json format.
                        _ => {
                            assert!(response.starts_with(r#"{"hash":0,"#));

                            // Move onto the next command.
                            test_state = update_state(Some(test_state));
                        }
                    }
                }
                GooseControllerCommand::Start => {
                    match test_state.step {
                        // Try to stop an idle load test.
                        0 => {
                            make_request(&mut test_state, "stop\r\n");
                        }
                        // Confirm an idle load test can not be stopped.
                        1 => {
                            assert!(response.starts_with("load test not running"));

                            // Send the start request.
                            make_request(&mut test_state, "start\r\n");
                        }
                        // Confirm an idle load test can be started.
                        2 => {
                            assert!(response.starts_with("load test started"));

                            // Send the start request again.
                            make_request(&mut test_state, "start\r\n");
                        }
                        // Confirm a running load test can not be started.
                        _ => {
                            assert!(response.starts_with("unable to start load test"));

                            // Move onto the next command.
                            test_state = update_state(Some(test_state));
                        }
                    }
                }
                GooseControllerCommand::Stop => {
                    match test_state.step {
                        // Try to configure users on a running load test.
                        0 => {
                            make_request(&mut test_state, "users 1\r\n");
                        }
                        // Confirm users can not be configured on a running load test.
                        1 => {
                            assert!(response.starts_with("load test not idle"));

                            // Try to configure host on a running load test.
                            make_request(&mut test_state, "host http://localhost/\r\n");
                        }
                        // Confirm host can not be configured on a running load test.
                        2 => {
                            assert!(response.starts_with("failed to reconfigure host"));

                            // Try to stop a running load test.
                            make_request(&mut test_state, "stop\r\n");
                        }
                        // Confirm a running load test can be stopped.
                        _ => {
                            assert!(response.starts_with("load test stopped"));

                            // Give Goose a half second to stop before moving on.
                            thread::sleep(time::Duration::from_millis(500));

                            // Move onto the next command.
                            test_state = update_state(Some(test_state));
                        }
                    }
                }
                GooseControllerCommand::Shutdown => {
                    match test_state.step {
                        // Shut down the load test.
                        0 => {
                            make_request(&mut test_state, "shutdown\r\n");
                        }
                        // Confirm that the load test shut down.
                        _ => {
                            assert!(response.starts_with("load test shut down"));

                            // Move onto the next command.
                            test_state = update_state(Some(test_state));
                        }
                    }
                }
                GooseControllerCommand::Exit => {
                    // @TODO: this command is not currently tested.
                    unreachable!()
                }
            }
            // Flush the buffer.
            test_state.buf = [0; 2048];

            // Give the parent process time to catch up.
            thread::sleep(time::Duration::from_millis(100));
        }
    });

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(
        common::build_load_test(configuration.clone(), &get_tasks(), None, None),
        None,
    );

    // Confirm that the load test ran correctly.
    validate_one_taskset(&goose_metrics, &mock_endpoints, &configuration, test_type);
}

// Update (or create) the current testing state. A simple state maching for
// navigating through all supported Controller commands and test states.
fn update_state(test_state: Option<TestState>) -> TestState {
    // The commands being tested, and the order they are tested.
    let commands_to_test = [
        GooseControllerCommand::Help,
        GooseControllerCommand::Host,
        GooseControllerCommand::Users,
        GooseControllerCommand::HatchRate,
        GooseControllerCommand::RunTime,
        GooseControllerCommand::Start,
        GooseControllerCommand::Config,
        GooseControllerCommand::ConfigJson,
        GooseControllerCommand::Metrics,
        GooseControllerCommand::MetricsJson,
        GooseControllerCommand::Stop,
        GooseControllerCommand::Shutdown,
    ];

    if let Some(mut state) = test_state {
        state.position += 1;
        state.step = 0;
        if let Some(command) = commands_to_test.get(state.position) {
            state.command = command.clone();
        }
        // Generate a new prompt.
        state.stream.write_all("\r\n".as_bytes()).unwrap();
        state
    } else {
        TestState {
            buf: [0; 2048],
            position: 0,
            step: 0,
            command: commands_to_test.first().unwrap().clone(),
            // Connect to the TCP Controller.
            stream: TcpStream::connect("127.0.0.1:5116").unwrap(),
        }
    }
}

fn make_request(test_state: &mut TestState, command: &str) {
    //println!("making request: {}", command);
    test_state.stream.write_all(command.as_bytes()).unwrap();
    test_state.step += 1;
}

#[test]
#[serial]
// Test controlling a load test with Telnet and WebSockets both.
// @TODO: start controllers on a custom port to avoid having to test serially.
fn test_both_controllers() {
    run_standalone_test(TestType::Both);
}

#[test]
#[serial]
// @TODO: start controller on a custom port to avoid having to test serially.
// Test controlling a load test with Telnet, when the WebSocket controller is disabled.
fn test_telnet_controller() {
    run_standalone_test(TestType::NoWebSocket);
}

/*
#[test]
// Test controlling a load test with Telnet controller.
#[test]
// Test controlling a load test with WebSocket controller.
fn test_websocket_controller() {
    run_standalone_test(TestType::NoTelnet);
}
*/
