use gumdrop::Options;
use httpmock::{Method::GET, MockRef, MockServer};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::{str, thread, time};
//use serial_test::serial;

mod common;

use goose::controller::GooseControllerCommand;
use goose::prelude::*;
use goose::GooseConfiguration;

// Paths used in load tests performed during these tests.
const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about.html";

// Indexes to the above paths.
const INDEX_KEY: usize = 0;
const ABOUT_KEY: usize = 1;

// Load test configuration.
//const EXPECT_WORKERS: usize = 2;

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
    //NoWebSocket,
}

// Internal state of test.
struct TestState {
    buf: [u8; 2048],
    position: usize,
    command: GooseControllerCommand,
    requested: bool,
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

// All tests in this file run against common endpoints.
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
// common::build_configuration() but for these tests we don't want a default
// configuration. We keep the signature the same, accepting the MockServer but
// not using it.
fn common_build_configuration(_server: &MockServer, custom: &mut Vec<&str>) -> GooseConfiguration {
    // Common elements in all our tests.
    let mut configuration: Vec<&str> = vec!["--no-autostart", "--status-codes",];

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
        //TestType::NoWebSocket => vec!["--no-websocket"],
    };

    // Build common configuration elements.
    let configuration = common_build_configuration(&server, &mut configuration_flags);

    let _controller_handle = thread::spawn(move || {
        // Sleep a quarter of a second allowing the GooseAttack to start.
        thread::sleep(time::Duration::from_millis(250));

        // Initiailize the state engine.
        let mut test_state = update_state(None, None);
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
                    if test_state.requested {
                        // When requesting help, the response should include the following text:
                        //println!("message: {}", response);
                        assert!(response.contains("controller commands:"));

                        // Move onto the next command.
                        test_state = update_state(Some(test_state), None);
                    } else {
                        // Make the request.
                        test_state = update_state(Some(test_state), Some("help\r\n"));
                    }
                }
                GooseControllerCommand::Host => {
                    if test_state.requested {
                        // The load test won't run unless this is correctly configured, so no
                        // further validation required here.
                        //println!("host response: {}", response);

                        // Move onto the next command.
                        test_state = update_state(Some(test_state), None);
                    } else {
                        // Make the request.
                        test_state = update_state(
                            Some(test_state),
                            Some(&["host ", &server_url, "\r\n"].concat()),
                        );
                    }
                }
                GooseControllerCommand::Users => {
                    if test_state.requested {
                        // The number of users started is verified when the load test finishes,
                        // so no further validation required here.
                        //println!("users response: {}", response);

                        // Move onto the next command.
                        test_state = update_state(Some(test_state), None);
                    } else {
                        // Make the request.
                        test_state = update_state(
                            Some(test_state),
                            Some(&["users ", &USERS.to_string(), "\r\n"].concat()),
                        );
                    }
                }
                GooseControllerCommand::HatchRate => {
                    if test_state.requested {
                        // The hatch_rate is verified when the load test finishes, so no
                        // further validation required here.
                        //println!("hatch_rate response: {}", response);

                        // Move onto the next command.
                        test_state = update_state(Some(test_state), None);
                    } else {
                        // Make the request.
                        test_state = update_state(
                            Some(test_state),
                            Some(&["hatchrate ", &HATCH_RATE.to_string(), "\r\n"].concat()),
                        );
                    }
                }
                GooseControllerCommand::RunTime => {
                    if test_state.requested {
                        // The run_time is verified when the load test finishes, so no
                        // further validation required here. Unfortunately, if this fails
                        // the load test could run forever.
                        //println!("run_time response: {}", response);

                        // Move onto the next command.
                        test_state = update_state(Some(test_state), None);
                    } else {
                        // Make the request.
                        test_state = update_state(
                            Some(test_state),
                            Some(&["runtime ", &RUN_TIME.to_string(), "\r\n"].concat()),
                        );
                    }
                }
                GooseControllerCommand::Config => {
                    if test_state.requested {
                        // Confirm that the response starts with the following text.
                        //println!("config response: '{}'", response);
                        assert!(response.starts_with("GooseConfiguration {"));

                        // Move onto the next command.
                        test_state = update_state(Some(test_state), None);
                    } else {
                        // Make the request.
                        test_state = update_state(Some(test_state), Some("config\r\n"));
                    }
                }
                GooseControllerCommand::ConfigJson => {
                    if test_state.requested {
                        // Confirm that the response starts with the following text.
                        //println!("config-json response: '{}'", response);
                        assert!(response.starts_with(r#"{"help":false,"version":false,"list":false,"#));

                        // Move onto the next command.
                        test_state = update_state(Some(test_state), None);
                    } else {
                        // Make the request.
                        test_state = update_state(Some(test_state), Some("config-json\r\n"));
                    }
                }
                GooseControllerCommand::Metrics => {
                    if test_state.requested {
                        // Confirm that the response starts with the following text.
                        //println!("metrics response: {}", response);
                        assert!(response.contains("=== PER TASK METRICS ==="));

                        // Move onto the next command.
                        test_state = update_state(Some(test_state), None);
                    } else {
                        // Make the request.
                        test_state = update_state(Some(test_state), Some("metrics\r\n"));
                    }
                }
                GooseControllerCommand::MetricsJson => {
                    if test_state.requested {
                        // Confirm that the response starts with the following text.
                        //println!("metrics-json response: '{}'", response);
                        assert!(response.starts_with(r#"{"hash":0,"#));

                        // Move onto the next command.
                        test_state = update_state(Some(test_state), None);
                    } else {
                        // Make the request.
                        test_state = update_state(Some(test_state), Some("metrics-json\r\n"));
                    }
                }
                GooseControllerCommand::Start => {
                    if test_state.requested {
                        // Confirm that the response starts with the following text.
                        //println!("start response: {}", response);
                        assert!(response.starts_with("load test started"));

                        thread::sleep(time::Duration::from_millis(500));

                        // Move onto the next command.
                        test_state = update_state(Some(test_state), None);
                    } else {
                        // Make the request.
                        test_state = update_state(Some(test_state), Some("start\r\n"));
                    }
                }
                GooseControllerCommand::Stop => {
                    if test_state.requested {
                        // Confirm that the response starts with the following text.
                        //println!("stop response: '{}'", response);
                        assert!(response.starts_with("load test stopped"));

                        thread::sleep(time::Duration::from_millis(500));

                        // Move onto the next command.
                        test_state = update_state(Some(test_state), None);
                    } else {
                        // Make the request.
                        test_state = update_state(Some(test_state), Some("stop\r\n"));
                    }
                }
                GooseControllerCommand::Shutdown => {
                    if test_state.requested {
                        // Confirm that the response starts with the following text.
                        //println!("stop response: '{}'", response);
                        assert!(response.starts_with("load test shut down"));

                        // Move onto the next command.
                        test_state = update_state(Some(test_state), None);
                    } else {
                        // Make the request.
                        test_state = update_state(Some(test_state), Some("shutdown\r\n"));
                    }
                }
                GooseControllerCommand::Exit => {
                    // @TODO: this command is not currently tested.
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

// Update (or create) the current testing state. In order to navigate through all supported
// Controller commands and variations we need a simple state machine.
fn update_state(test_state: Option<TestState>, command: Option<&str>) -> TestState {
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
        // If a command has been passed in, run it and update the state accordingly.
        if let Some(c) = command {
            //println!("running command: {}", c);
            state.stream.write_all(c.as_bytes()).unwrap();
            state.requested = true;
            state
        // Otherwise move onto the next command.
        } else {
            state.position += 1;
            if let Some(command) = commands_to_test.get(state.position) {
                state.command = command.clone();
            }
            state.requested = false;
            // Generate a new prompt.
            state.stream.write_all("\r\n".as_bytes()).unwrap();
            state
        }
    } else {
        TestState {
            buf: [0; 2048],
            position: 0,
            command: commands_to_test.first().unwrap().clone(),
            requested: false,
            // Connect to the TCP Controller.
            stream: TcpStream::connect("127.0.0.1:5116").unwrap(),
        }
    }
}

/*
// Helper to run all distributed tests.
fn run_gaggle_test(test_type: TestType) {
    // Start the mock server.
    let server = MockServer::start();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    // Each worker has the same identical configuration.
    let worker_configuration = common::build_configuration(&server, vec!["--worker"]);

    // Build the load test for the Workers.
    let goose_attack = common::build_load_test(worker_configuration, &get_tasks(), None, None);

    // Workers launched in own threads, store thread handles.
    let worker_handles = common::launch_gaggle_workers(goose_attack, EXPECT_WORKERS);

    // Build common configuration elements, adding Manager Gaggle flags.
    let manager_configuration = match test_type {
        TestType::Both => common_build_configuration(
            &server,
            &mut vec!["--manager", "--expect-workers", &EXPECT_WORKERS.to_string()],
        ),
        TestType::NoTelnet => common_build_configuration(
            &server,
            &mut vec![
                "--manager",
                "--expect-workers",
                &EXPECT_WORKERS.to_string(),
                "--no-telnet",
            ],
        ),
        TestType::NoWebSocket => common_build_configuration(
            &server,
            &mut vec![
                "--manager",
                "--expect-workers",
                &EXPECT_WORKERS.to_string(),
                "--no-websocket",
            ],
        ),
    };

    // Build the load test for the Manager.
    let manager_goose_attack =
        common::build_load_test(manager_configuration.clone(), &get_tasks(), None, None);

    // Run the Goose Attack.
    let goose_metrics = common::run_load_test(manager_goose_attack, Some(worker_handles));

    // Confirm that the load test ran correctly.
    validate_one_taskset(
        &goose_metrics,
        &mock_endpoints,
        &manager_configuration,
        test_type,
    );
}
*/

#[test]
// Test controlling a load test with Telnet and WebSockets both.
fn test_both_controllers() {
    run_standalone_test(TestType::Both);
}

/* @TODO: @FIXME: Goose does not support Controllers in Gaggle mode currently.
#[test]
#[cfg_attr(not(feature = "gaggle"), ignore)]
#[serial]
// Test controlling a load test with Telnet and WebSockets both, in Gaggle mode.
fn test_both_controllers() {
    run_gaggle_test(TestType::Both);
}

#[test]
// Test controlling a load test with Telnet controller.
fn test_telnet_controller() {
    run_standalone_test(TestType::NoWebSocket);
}

#[test]
// Test controlling a load test with WebSocket controller.
fn test_websocket_controller() {
    run_standalone_test(TestType::NoTelnet);
}
*/
