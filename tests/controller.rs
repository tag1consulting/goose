use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use gumdrop::Options;
use httpmock::{Method::GET, Mock, MockServer};
use std::{str, time};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use goose::config::GooseConfiguration;
use goose::controller::{
    ControllerCommand, ControllerWebSocketRequest, ControllerWebSocketResponse,
};
use goose::prelude::*;

mod common;

// Paths used in load tests performed during these tests.
const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about.html";

// Files generated during tests.
const REPORT_FILE: &str = "test-controller-report.html";

// Indexes to the above paths.
const INDEX_KEY: usize = 0;
const ABOUT_KEY: usize = 1;

const START_USERS: usize = 5;
const MAX_USERS: usize = 20;
const FINAL_USERS: usize = 10;
const HATCH_RATE: usize = 25;
const RUN_TIME: usize = 55;
const STARTUP_TIME: usize = 1;

// There are multiple test variations in this file.
#[derive(Clone, PartialEq)]
enum TestType {
    // Enable --no-telnet.
    WebSocket,
    // Enable --no-websocket.
    Telnet,
}

// State machine for tracking Controller state during tests.
struct TestState {
    // A buffer for the telnet Controller.
    buf: [u8; 2048],
    // Track iterations through ControllerCommands.
    position: usize,
    // Track the steps within a given iteration.
    step: usize,
    // The Controller command currently being tested.
    command: ControllerCommand,
    // A TCP socket if testing the telnet Controller.
    telnet_stream: Option<TcpStream>,
    // The receiver side of a split TCP socket if testing the WebSocket Controller.
    websocket_receiver: Option<SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>>,
    // The sender side of a split TCP socket if testing the WebSocket Controller.
    websocket_sender: Option<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>,
    // A flag indicating whether or not to wait for a reply.
    websocket_expect_reply: bool,
    // A flag indicating whether or not the WebSocket controller is being tested.
    websocket_controller: bool,
}

// Test transaction.
pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

// Test transaction.
pub async fn get_about(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(ABOUT_PATH).await?;
    Ok(())
}

// All tests in this file run against the following common endpoints.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<Mock> {
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
    let mut configuration: Vec<&str> =
        vec!["--quiet", "--no-autostart", "--co-mitigation", "disabled"];

    // Custom elements in some tests.
    configuration.append(custom);

    // Parse these options to generate a GooseConfiguration.
    GooseConfiguration::parse_args_default(&configuration)
        .expect("failed to parse options and generate a configuration")
}

// Helper to confirm all variations generate appropriate results.
fn validate_one_scenario(
    goose_metrics: &GooseMetrics,
    mock_endpoints: &[Mock],
    configuration: &GooseConfiguration,
    test_type: TestType,
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
    assert!(goose_metrics.total_users == MAX_USERS);

    // Host was not configured at start time.
    assert!(configuration.host.is_empty());

    // The load test was manually shut down instead of running to completion.
    assert!(goose_metrics.duration < RUN_TIME);

    // Increasing, Maintaining, Increasing, Maintaining, Decreasing, Maintaining, Canceling,
    // Finished, Finished.
    // Finished is logged twice because `stop` puts the test to Idle, and then `shutdown`
    // actually shuts down the test, and both are logged as "Finished".
    assert!(goose_metrics.history.len() == 9);

    // The Telnet test should create a report-file.
    if test_type == TestType::Telnet {
        // The report file must exist.
        assert!(std::path::Path::new(REPORT_FILE).exists());
        // The report file must not be empty.
        assert!(common::file_length(REPORT_FILE) > 0);
        // Now clean up the file.
        common::cleanup_files(vec![REPORT_FILE]);
    }
}

// Returns the appropriate scenario needed to build these tests.
fn get_transactions() -> Scenario {
    scenario!("LoadTest")
        .register_transaction(transaction!(get_index).set_weight(2).unwrap())
        .register_transaction(transaction!(get_about).set_weight(1).unwrap())
}

// Helper to run all standalone tests.
async fn run_standalone_test(test_type: TestType) {
    // Start the mock server.
    let server = MockServer::start();
    let server_url = server.base_url();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    let mut configuration_flags = match &test_type {
        TestType::WebSocket => vec!["--no-telnet"],
        // Also test writing a report-file when stopping load test through controller.
        TestType::Telnet => vec!["--no-websocket", "--report-file", REPORT_FILE],
    };

    // Keep a copy for validation.
    let validate_test_type = test_type.clone();

    // Build common configuration elements.
    let configuration = common_build_configuration(&server, &mut configuration_flags);

    // Create a new thread from which to test the Controller.
    let _controller_handle = tokio::spawn(async move {
        // Sleep a half a second allowing the GooseAttack to start.
        tokio::time::sleep(time::Duration::from_millis(500)).await;

        // Initiailize the state engine.
        let mut test_state = update_state(None, &test_type).await;
        loop {
            // Process data received from the client in a loop.
            let response;
            let websocket_response: ControllerWebSocketResponse;
            if let Some(stream) = test_state.telnet_stream.as_mut() {
                let _ = match stream.read(&mut test_state.buf).await {
                    Ok(data) => data,
                    Err(_) => {
                        panic!("ERROR: server disconnected!");
                    }
                };
                response = str::from_utf8(&test_state.buf).unwrap();
            // Process data received from the client in a loop.
            } else if let Some(ws_receiver) = test_state.websocket_receiver.as_mut() {
                if !test_state.websocket_expect_reply {
                    response = "";
                    test_state.websocket_expect_reply = true;
                } else {
                    let message = ws_receiver.next().await.expect("error reading from server");
                    let r = message
                        .expect("error unwrapping message from server")
                        .into_text()
                        .expect("invalid message type");
                    // Keep response around for the entire loop.
                    websocket_response = match serde_json::from_str(&r) {
                        Ok(c) => c,
                        Err(e) => panic!("invalid response from server: {}", e),
                    };
                    response = &websocket_response.response;
                }
            } else {
                unreachable!();
            }

            //println!("{:?}: {}", test_state.command, response);
            match test_state.command {
                ControllerCommand::Exit => {
                    match test_state.step {
                        // Exit the Controller.
                        0 => {
                            make_request(&mut test_state, "exit\r\n").await;
                        }
                        // Confirm that the Controller exited.
                        _ => {
                            assert!(response.starts_with("goodbye!"));

                            // Re-connect to the Controller.
                            test_state = update_state(None, &test_type).await;

                            // Move onto the next command.
                            test_state = update_state(Some(test_state), &test_type).await;
                        }
                    }
                }
                ControllerCommand::Help => {
                    match test_state.step {
                        0 => {
                            // Request the help text.
                            make_request(&mut test_state, "help\r\n").await;
                        }
                        1 => {
                            // Be sure we actually received the help text.
                            assert!(response.contains("controller commands:"));

                            // Request the help text, using the short form.
                            make_request(&mut test_state, "?\r\n").await;
                        }
                        _ => {
                            // Be sure we actually received the help text.
                            assert!(response.contains("controller commands:"));

                            // Move onto the next command.
                            test_state = update_state(Some(test_state), &test_type).await;
                        }
                    }
                }
                ControllerCommand::Host => {
                    match test_state.step {
                        // Set the host to be load tested.
                        0 => {
                            make_request(&mut test_state, &["host ", &server_url, "\r\n"].concat())
                                .await;
                        }
                        // Confirm the host was configured.
                        1 => {
                            assert!(response.starts_with("host configured"));

                            // Then try and set an invalid host.
                            make_request(&mut test_state, "host foobar\r\n").await;
                        }
                        // Confirm that we can't configure an invalid host that doesn't
                        // match the regex.
                        2 => {
                            assert!(response.starts_with("unrecognized command"));

                            // Try again to set an invalid host.
                            make_request(&mut test_state, "host http://$[foo\r\n").await;
                        }
                        // Confirm that we can't configure an invalid host that does
                        // match the regex.
                        _ => {
                            assert!(response.starts_with("failed to reconfigure host"));

                            // Move onto the next command.
                            test_state = update_state(Some(test_state), &test_type).await;
                        }
                    }
                }
                ControllerCommand::TestPlan => {
                    match test_state.step {
                        // Try and set an invalid testplan.
                        0 => {
                            make_request(&mut test_state, "testplan 10\r\n").await;
                        }
                        1 => {
                            // Confirm that an invalid testplan fails.
                            assert!(response.starts_with("unrecognized command"));

                            // Set a valid test plan.
                            make_request(&mut test_state, "test_plan 10,2s;10,30m5s;0,1h2s\r\n")
                                .await;
                        }
                        _ => {
                            // Confirm that an invalid testplan fails.
                            assert!(response.starts_with("test-plan configured"));

                            // Move onto the next command.
                            test_state = update_state(Some(test_state), &test_type).await;
                        }
                    }
                }
                ControllerCommand::Users => {
                    match test_state.step {
                        // Reconfigure the number of users simulated by the load test.
                        0 => {
                            make_request(
                                &mut test_state,
                                &["users ", &START_USERS.to_string(), "\r\n"].concat(),
                            )
                            .await;
                        }
                        // Confirm that the users are reconfigured.
                        1 => {
                            assert!(response.starts_with("users configured"));

                            // Attempt to reconfigure users with bad data.
                            make_request(&mut test_state, "users 1.1\r\n").await;
                        }
                        // Confirm we can't configure users with a float.
                        _ => {
                            // The number of users started is verified when the load test finishes,
                            // so no further validation required here.
                            assert!(response.starts_with("unrecognized command"));

                            // Move onto the next command.
                            test_state = update_state(Some(test_state), &test_type).await;
                        }
                    }
                }
                ControllerCommand::HatchRate => {
                    match test_state.step {
                        // Configure a decimal hatch_rate.
                        0 => {
                            make_request(&mut test_state, "hatchrate .1\r\n").await;
                        }
                        // Confirm hatch_rate is configured.
                        1 => {
                            assert!(response.starts_with("hatch_rate configured"));

                            // Configure with leading and trailing zeros.
                            make_request(&mut test_state, "hatchrate 0.90\r\n").await;
                        }
                        // Confirm hatch_rate is configured.
                        2 => {
                            assert!(response.starts_with("hatch_rate configured"));

                            // Try to configure with an invalid decimal.
                            make_request(&mut test_state, "hatchrate 1.2.3\r\n").await;
                        }
                        // Confirm hatch_rate is not configured.
                        3 => {
                            assert!(response.starts_with("unrecognized command"));

                            // Configure hatch_rate with a single integer.
                            make_request(
                                &mut test_state,
                                &["hatchrate ", &HATCH_RATE.to_string(), "\r\n"].concat(),
                            )
                            .await;
                        }
                        // Confirm the final hatch_rate is configured.
                        _ => {
                            assert!(response.starts_with("hatch_rate configured"));

                            // The hatch_rate is verified when the load test finishes, so no
                            // further validation required here.

                            // Move onto the next command.
                            test_state = update_state(Some(test_state), &test_type).await;
                        }
                    }
                }
                ControllerCommand::StartupTime => {
                    match test_state.step {
                        // Try to configure a decimal StartupTime.
                        0 => {
                            make_request(&mut test_state, "startup-time .1\r\n").await;
                        }
                        // Configure a StartupTime with hms notation.
                        1 => {
                            assert!(response.starts_with("unrecognized command"));

                            make_request(&mut test_state, "startup-time 1h2m3s\r\n").await;
                        }
                        // Configure a proper StartupTime.
                        2 => {
                            assert!(response.starts_with("startup_time configured"));

                            make_request(
                                &mut test_state,
                                &["startup_time ", &STARTUP_TIME.to_string(), "\r\n"].concat(),
                            )
                            .await;
                        }
                        // Restore the HatchRate.
                        3 => {
                            assert!(response.starts_with("startup_time configured"));

                            // Configure hatch_rate with a single integer.
                            make_request(
                                &mut test_state,
                                &["hatchrate ", &HATCH_RATE.to_string(), "\r\n"].concat(),
                            )
                            .await;
                        }
                        // Confirm hatch_rate is configured.
                        _ => {
                            assert!(response.starts_with("hatch_rate configured"));

                            test_state = update_state(Some(test_state), &test_type).await;
                        }
                    }
                }
                ControllerCommand::RunTime => {
                    match test_state.step {
                        // Configure run_time using h:m:s format.
                        0 => {
                            // Set run_time with hours and minutes and seconds.
                            make_request(&mut test_state, "runtime 1h2m3s\r\n").await;
                        }
                        // Confirm the run_time is configured.
                        1 => {
                            assert!(response.starts_with("run_time configured"));

                            // Set run_time with hours and seconds.
                            make_request(&mut test_state, "run_time 1h2s\r\n").await;
                        }
                        // Confirm the run_time is configured.
                        2 => {
                            assert!(response.starts_with("run_time configured"));

                            // Set run_time with hours alone.
                            make_request(&mut test_state, "run-time 1h\r\n").await;
                        }
                        // Confirm the run_time is configured.
                        3 => {
                            assert!(response.starts_with("run_time configured"));

                            // Set run_time with seconds alone.
                            make_request(&mut test_state, "runtime 10s\r\n").await;
                        }
                        // Confirm the run_time is configured.
                        4 => {
                            assert!(response.starts_with("run_time configured"));

                            // Try to set run_time with unsupported value.
                            make_request(&mut test_state, "runtime 10d\r\n").await;
                        }
                        // Confirm the run_time is not configured.
                        5 => {
                            assert!(response.starts_with("unrecognized command"));

                            // Set run_time with seconds alone, and no "s".
                            make_request(
                                &mut test_state,
                                &["runtime ", &RUN_TIME.to_string(), "\r\n"].concat(),
                            )
                            .await;
                        }
                        // Confirm the run_time is configured.
                        _ => {
                            assert!(response.starts_with("run_time configured"));

                            // The run_time is verified when the load test finishes, so no
                            // further validation required here. Unfortunately, if this fails
                            // the load test could run forever.

                            // Move onto the next command.
                            test_state = update_state(Some(test_state), &test_type).await;
                        }
                    }
                }
                ControllerCommand::Config => {
                    match test_state.step {
                        // Request the configuration.
                        0 => {
                            make_request(&mut test_state, "config\r\n").await;
                        }
                        _ => {
                            // Confirm the configuration is returned in jsonformat.
                            if test_state.websocket_controller {
                                assert!(response
                                    .starts_with(r#"{"help":false,"version":false,"list":false,"#));
                            // Confirm the configuration object is returned.
                            } else {
                                assert!(response.starts_with(r"GooseConfiguration "));
                            }

                            // Move onto the next command.
                            test_state = update_state(Some(test_state), &test_type).await;
                        }
                    }
                }
                ControllerCommand::ConfigJson => {
                    match test_state.step {
                        // Request the configuration in json format.
                        0 => {
                            make_request(&mut test_state, "config-json\r\n").await;
                        }
                        // Confirm the configuration is returned in jsonformat.
                        _ => {
                            assert!(response
                                .starts_with(r#"{"help":false,"version":false,"list":false,"#));

                            // Move onto the next command.
                            test_state = update_state(Some(test_state), &test_type).await;
                        }
                    }
                }
                ControllerCommand::Metrics => {
                    match test_state.step {
                        // Request the running metrics.
                        0 => {
                            make_request(&mut test_state, "metrics\r\n").await;
                        }
                        _ => {
                            // Confirm the metrics are returned in json format.
                            if test_state.websocket_controller {
                                assert!(response.starts_with(r#"{"hash":0,"#));
                            }
                            // Confirm the metrics are returned and pretty-printed.
                            else {
                                assert!(response.contains("=== PER TRANSACTION METRICS ==="));
                            }

                            // Move onto the next command.
                            test_state = update_state(Some(test_state), &test_type).await;
                        }
                    }
                }
                ControllerCommand::MetricsJson => {
                    match test_state.step {
                        // Request the running metrics in json format.
                        0 => {
                            make_request(&mut test_state, "metrics-json\r\n").await;
                        }
                        // Confirm the metrics are returned in json format.
                        _ => {
                            assert!(response.starts_with(r#"{"hash":0,"#));

                            // Move onto the next command.
                            test_state = update_state(Some(test_state), &test_type).await;
                        }
                    }
                }
                ControllerCommand::Start => {
                    match test_state.step {
                        // Try to stop an idle load test.
                        0 => {
                            make_request(&mut test_state, "stop\r\n").await;
                        }
                        // Confirm an idle load test can not be stopped.
                        1 => {
                            assert!(response.starts_with("load test not running"));

                            // Send the start request.
                            make_request(&mut test_state, "start\r\n").await;
                        }
                        // Confirm an idle load test can be started.
                        2 => {
                            assert!(response.starts_with("load test started"));

                            // Send the start request again.
                            make_request(&mut test_state, "start\r\n").await;
                        }
                        // Confirm a running load test can not be started.
                        _ => {
                            assert!(response.starts_with("unable to start load test"));

                            // Move onto the next command.
                            test_state = update_state(Some(test_state), &test_type).await;
                        }
                    }
                }
                ControllerCommand::Stop => {
                    match test_state.step {
                        // Try to configure host on a running load test.
                        0 => {
                            make_request(&mut test_state, "host http://localhost/\r\n").await;
                        }
                        // Setting host fails, then reconfigure the number of users, increasing.
                        1 => {
                            assert!(response.starts_with("failed to reconfigure host"));

                            make_request(
                                &mut test_state,
                                &["users ", &MAX_USERS.to_string(), "\r\n"].concat(),
                            )
                            .await;
                        }
                        // Setting users succeeds, reconfigure the number of users, decreasing.
                        2 => {
                            assert!(response.starts_with("users configured"));

                            // Give Goose a second to increase users.
                            tokio::time::sleep(time::Duration::from_secs(1)).await;

                            make_request(
                                &mut test_state,
                                &["users ", &FINAL_USERS.to_string(), "\r\n"].concat(),
                            )
                            .await;
                        }
                        // Confirm host can not be configured on a running load test.
                        3 => {
                            assert!(response.starts_with("users configured"));

                            // Give Goose a second to decrease users.
                            tokio::time::sleep(time::Duration::from_secs(1)).await;

                            // Try to stop a running load test.
                            make_request(&mut test_state, "stop\r\n").await;
                        }
                        // Confirm a running load test can be stopped.
                        _ => {
                            assert!(response.starts_with("load test stopped"));

                            // Give Goose a half second to stop before moving on.
                            tokio::time::sleep(time::Duration::from_millis(500)).await;

                            // Move onto the next command.
                            test_state = update_state(Some(test_state), &test_type).await;
                        }
                    }
                }
                ControllerCommand::Shutdown => {
                    match test_state.step {
                        // Shut down the load test.
                        0 => {
                            make_request(&mut test_state, "shutdown\r\n").await;
                        }
                        // Confirm that the load test shut down.
                        _ => {
                            assert!(response.starts_with("load test shut down"));

                            // Move onto the next command.
                            test_state = update_state(Some(test_state), &test_type).await;
                        }
                    }
                }
            }
            // Flush the buffer.
            test_state.buf = [0; 2048];

            // Give the parent process time to catch up.
            tokio::time::sleep(time::Duration::from_millis(100)).await;
        }
    });

    // Run the Goose Attack. Add timeout to be sure Goose exits even if the controller tests fail.
    let goose_metrics = tokio::time::timeout(
        tokio::time::Duration::from_secs(60),
        common::run_load_test(
            common::build_load_test(configuration.clone(), vec![get_transactions()], None, None),
            None,
        ),
    )
    .await
    .expect("load test timed out");

    // Confirm that the load test ran correctly.
    validate_one_scenario(
        &goose_metrics,
        &mock_endpoints,
        &configuration,
        validate_test_type,
    );
}

// Update (or create) the current testing state. A simple state maching for
// navigating through all supported Controller commands and test states.
async fn update_state(test_state: Option<TestState>, test_type: &TestType) -> TestState {
    // The commands being tested, and the order they are tested.
    let commands_to_test = [
        ControllerCommand::Exit,
        ControllerCommand::Help,
        ControllerCommand::Host,
        ControllerCommand::TestPlan,
        ControllerCommand::Users,
        ControllerCommand::HatchRate,
        ControllerCommand::StartupTime,
        ControllerCommand::RunTime,
        ControllerCommand::Start,
        ControllerCommand::Config,
        ControllerCommand::ConfigJson,
        ControllerCommand::Metrics,
        ControllerCommand::MetricsJson,
        ControllerCommand::Stop,
        ControllerCommand::Shutdown,
    ];

    if let Some(mut state) = test_state {
        state.position += 1;
        state.step = 0;
        if let Some(command) = commands_to_test.get(state.position) {
            state.command = command.clone();
        }
        // Generate a new prompt.
        if let Some(stream) = state.telnet_stream.as_mut() {
            stream.write_all("\r\n".as_bytes()).await.unwrap();
        } else {
            state.websocket_expect_reply = false;
        }
        state
    } else {
        // Connect to telnet controller.
        let telnet_stream = match test_type {
            TestType::Telnet => Some(TcpStream::connect("127.0.0.1:5116").await.unwrap()),
            _ => None,
        };

        // Connect to WebSocket controller.
        let websocket_controller: bool;
        let (websocket_sender, websocket_receiver) = match test_type {
            TestType::WebSocket => {
                let (stream, _item) = tokio_tungstenite::connect_async("ws://127.0.0.1:5117")
                    .await
                    .unwrap();
                let (mut ws_sender, mut ws_receiver) = stream.split();

                // Send an empty message so the client performs a handshake.
                ws_sender.send(Message::Text("".into())).await.unwrap();

                // Ignore the error that comes back.
                let _ = ws_receiver.next().await.unwrap();

                websocket_controller = true;
                (Some(ws_sender), Some(ws_receiver))
            }
            TestType::Telnet => {
                websocket_controller = false;
                (None, None)
            }
        };

        TestState {
            buf: [0; 2048],
            position: 0,
            step: 0,
            command: commands_to_test.first().unwrap().clone(),
            telnet_stream,
            websocket_sender,
            websocket_receiver,
            websocket_expect_reply: false,
            websocket_controller,
        }
    }
}

async fn make_request(test_state: &mut TestState, command: &str) {
    //println!("making request: {}", command);
    if let Some(stream) = test_state.telnet_stream.as_mut() {
        match stream.write_all(command.as_bytes()).await {
            Ok(_) => (),
            Err(e) => panic!("failed to send {} command: {}", command, e),
        };
    } else if let Some(ws_sender) = test_state.websocket_sender.as_mut() {
        ws_sender
            .send(Message::Text(
                serde_json::to_string(&ControllerWebSocketRequest {
                    request: command.to_string(),
                })
                .unwrap(),
            ))
            .await
            .unwrap()
    }
    test_state.step += 1;
}

// Test controlling a load test with Telnet.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn test_telnet_controller() {
    run_standalone_test(TestType::Telnet).await;
}

// Test controlling a load test with WebSocket controller.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn test_websocket_controller() {
    run_standalone_test(TestType::WebSocket).await;
}
