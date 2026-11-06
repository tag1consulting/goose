use futures::{SinkExt, StreamExt};
use httpmock::{Method::GET, Mock, MockServer};
use std::time;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use goose::config::GooseConfiguration;
use goose::controller::{ControllerWebSocketRequest, ControllerWebSocketResponse};
use goose::prelude::*;

mod common;
#[cfg(feature = "mcp")]
mod mcp_client;

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
const INCREASE_RATE: usize = 25;
const RUN_TIME: usize = 55;
const INCREASE_TIME: usize = 1;

// The Controller each test variation drives, every one through the same
// table of commands below.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Protocol {
    // Enable --no-websocket: the telnet Controller on its fixed port 5116.
    Telnet,
    // Enable --websocket and --no-telnet: the WebSocket Controller on 5117.
    WebSocket,
    // Enable --mcp and --no-telnet: the MCP Controller on an ephemeral port.
    #[cfg(feature = "mcp")]
    Mcp,
}

impl Protocol {
    // Whether replies to config and metrics are json: WebSocket's always are,
    // and so are the MCP tools'.
    fn json(self) -> bool {
        self != Protocol::Telnet
    }
}

// The Controller commands tested, each in one or more steps.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Command {
    Exit,
    Help,
    Host,
    TestPlan,
    Users,
    IncreaseRate,
    IncreaseTime,
    DecreaseRate,
    DecreaseTime,
    RunTime,
    Throttle,
    Start,
    Config,
    ConfigJson,
    Metrics,
    MetricsJson,
    Stop,
    Shutdown,
}

impl Command {
    // Whether `protocol` has this command. The MCP Controller has no session
    // to exit and no help or unrecognized line: its tools/list is its help,
    // and tests/mcp_controller.rs tests an unknown tool.
    fn runs_on(self, protocol: Protocol) -> bool {
        #[cfg(feature = "mcp")]
        if protocol == Protocol::Mcp {
            return !matches!(self, Command::Exit | Command::Help);
        }
        let _ = protocol;
        true
    }
}

// A Controller's reply: the WebSocket and MCP success flag (telnet has none)
// and the text.
struct Reply {
    success: Option<bool>,
    text: String,
}

// What a step checks of its reply, with the protocol that sent it.
type Check = Box<dyn Fn(Protocol, &Reply) + Send>;

// One line sent to the Controller, the check of its reply, and how long to
// wait after it for Goose to catch up, or the users to wait for.
struct Step {
    line: String,
    check: Check,
    pause: time::Duration,
    reach: Option<usize>,
}

// A step whose reply must start with `prefix`.
fn starts(line: &str, prefix: &'static str) -> Step {
    Step {
        line: line.to_string(),
        check: Box::new(move |_, reply: &Reply| assert!(reply.text.starts_with(prefix))),
        pause: time::Duration::ZERO,
        reach: None,
    }
}

// A step with a check of its own.
fn step(line: &str, check: impl Fn(Protocol, &Reply) + Send + 'static) -> Step {
    Step {
        line: line.to_string(),
        check: Box::new(check),
        pause: time::Duration::ZERO,
        reach: None,
    }
}

impl Step {
    // This step, waiting `pause` after its reply.
    fn then_wait(mut self, pause: time::Duration) -> Self {
        self.pause = pause;
        self
    }

    // This step, then waiting until `users` run: a `users` reply means the
    // change was accepted, not that its users started or stopped.
    fn then_reach(mut self, users: usize) -> Self {
        self.reach = Some(users);
        self
    }
}

// Every command tested, in order, with its steps: the same lines and the same
// expected replies for every protocol.
fn commands(server_url: &str) -> Vec<(Command, Vec<Step>)> {
    vec![
        // Exit the Controller, then connect again.
        (Command::Exit, vec![starts("exit", "goodbye!")]),
        (
            Command::Help,
            vec![
                // Request the help text.
                step("help", |_, reply| {
                    assert!(reply.text.contains("controller commands:"))
                }),
                // Request the help text, using the short form.
                step("?", |_, reply| {
                    assert!(reply.success.unwrap_or(true), "help must succeed");
                    assert!(reply.text.contains("controller commands:"));
                }),
                // Send a line that is not a command.
                step("foo", |protocol, reply| {
                    let websocket = protocol == Protocol::WebSocket;
                    assert_eq!(reply.success, websocket.then_some(false));
                    // The WebSocket reply also points to the book.
                    if websocket {
                        assert!(reply
                            .text
                            .starts_with("unrecognized command, see Goose book"));
                    } else {
                        assert!(reply.text.starts_with("unrecognized command\ngoose> "));
                    }
                }),
            ],
        ),
        (
            Command::Host,
            vec![
                // Set the host to be load tested.
                starts(&["host ", server_url].concat(), "host configured"),
                // An invalid host is refused.
                starts("host foobar", "host must be a valid url"),
                // A host that is not a valid url is refused.
                starts("host http://$[foo", "host must be a valid url"),
            ],
        ),
        (
            Command::TestPlan,
            vec![
                // An invalid testplan is refused.
                starts("testplan 10", "test-plan must be"),
                // A valid test plan is configured.
                starts("test_plan 10,2s;10,30m5s;0,1h2s", "test-plan configured"),
            ],
        ),
        (
            Command::Users,
            vec![
                // Reconfigure the number of users simulated by the load test.
                starts(
                    &["users ", &START_USERS.to_string()].concat(),
                    "users configured",
                ),
                // Zero users is refused.
                starts("users 0", "users must be an integer between 1 and 100000"),
                // More than the maximum is refused.
                starts(
                    "users 100001",
                    "users must be an integer between 1 and 100000",
                ),
                // Users can't be configured with a float. The number of users
                // started is verified when the load test finishes.
                starts("users 1.1", "users must be an integer between 1 and 100000"),
            ],
        ),
        (
            Command::IncreaseRate,
            vec![
                // A decimal increase_rate.
                starts("increaserate .1", "increase_rate configured"),
                // With leading and trailing zeros.
                starts("increaserate 0.90", "increase_rate configured"),
                // An invalid decimal is refused.
                starts(
                    "increaserate 1.2.3",
                    "increase_rate must be a number from 0.01 to 100000",
                ),
                // A single integer: the increase_rate is verified when the
                // load test finishes.
                starts(
                    &["increaserate ", &INCREASE_RATE.to_string()].concat(),
                    "increase_rate configured",
                ),
            ],
        ),
        (
            Command::IncreaseTime,
            vec![
                // A decimal IncreaseTime is refused.
                starts("increase-time .1", "increase_time must be a time span"),
                // An IncreaseTime with hms notation.
                starts("increase-time 1h2m3s", "increase_time configured"),
                // A proper IncreaseTime.
                starts(
                    &["increase_time ", &INCREASE_TIME.to_string()].concat(),
                    "increase_time configured",
                ),
                // Restore the IncreaseRate.
                starts(
                    &["increaserate ", &INCREASE_RATE.to_string()].concat(),
                    "increase_rate configured",
                ),
            ],
        ),
        (
            Command::DecreaseRate,
            vec![
                // A decimal decrease_rate.
                starts("decreaserate .5", "decrease_rate configured"),
                // An invalid decimal is refused.
                starts(
                    "decreaserate 1.2.3",
                    "decrease_rate must be a number from 0.01 to 100000",
                ),
                // A single integer.
                starts("decreaserate 2", "decrease_rate configured"),
            ],
        ),
        (
            Command::DecreaseTime,
            vec![
                // A decimal DecreaseTime is refused.
                starts("decrease-time .1", "decrease_time must be a time span"),
                // A DecreaseTime with hms notation.
                starts("decrease-time 1h2m3s", "decrease_time configured"),
                // A proper DecreaseTime.
                starts("decrease_time 30", "decrease_time configured"),
                // Restore increase_rate for subsequent tests.
                starts(
                    &["increaserate ", &INCREASE_RATE.to_string()].concat(),
                    "increase_rate configured",
                ),
            ],
        ),
        (
            Command::RunTime,
            vec![
                // Hours and minutes and seconds.
                starts("runtime 1h2m3s", "run_time configured"),
                // Hours and seconds.
                starts("run_time 1h2s", "run_time configured"),
                // Hours alone.
                starts("run-time 1h", "run_time configured"),
                // Seconds alone.
                starts("runtime 10s", "run_time configured"),
                // An unsupported value is refused.
                starts("runtime 10d", "run_time must be a time span"),
                // Seconds alone, and no "s". The run_time is verified when the
                // load test finishes; if this fails the load test could run
                // forever.
                starts(
                    &["runtime ", &RUN_TIME.to_string()].concat(),
                    "run_time configured",
                ),
            ],
        ),
        (
            Command::Throttle,
            vec![
                // A throttle past the bound is refused.
                starts("throttle 1000001", "throttle must be an integer"),
                // Set a throttle while idle, for the next Start.
                starts("throttle 100000", "throttle configured"),
                // 0 clears it while idle.
                starts("throttle-requests 0", "throttle cleared"),
                // Leave a throttle no request reaches, so the run's counts
                // are unchanged.
                starts("throttle_requests 100000", "throttle configured"),
            ],
        ),
        (
            Command::Start,
            vec![
                // An idle load test can not be stopped.
                starts("stop", "load test not running"),
                // An idle load test can be started.
                starts("start", "load test started"),
                // A running load test can not be started.
                starts("start", "unable to start load test, it is not idle"),
            ],
        ),
        (
            Command::Config,
            vec![step("config", |protocol, reply| {
                assert!(reply.success.unwrap_or(true), "config must succeed");
                // The configuration is returned in json format.
                if protocol.json() {
                    assert!(reply.text.starts_with(r#"{"host":"#));
                // The configuration object is returned.
                } else {
                    assert!(reply.text.starts_with(r"GooseConfiguration "));
                }
            })],
        ),
        (
            Command::ConfigJson,
            vec![starts("config-json", r#"{"host":"#)],
        ),
        (
            Command::Metrics,
            vec![step("metrics", |protocol, reply| {
                // The metrics are returned in json format.
                if protocol.json() {
                    assert!(reply.text.starts_with(r#"{"hash":0,"#));
                }
                // The metrics are returned and pretty-printed.
                else {
                    assert!(reply.text.contains("=== PER TRANSACTION METRICS ==="));
                }
            })],
        ),
        (
            // The reply is longer than a telnet read: the telnet client reads
            // the rest of it, to the prompt that ends it.
            Command::MetricsJson,
            vec![starts("metrics-json", r#"{"hash":0,"#)],
        ),
        (
            Command::Stop,
            vec![
                // Setting host on a running load test fails.
                starts(
                    "host http://localhost/",
                    "failed to reconfigure host, load test is not idle",
                ),
                // A user count too large for a usize is refused and the load
                // test keeps running.
                starts(
                    "users 99999999999999999999",
                    "users must be an integer between 1 and 100000",
                ),
                // Reconfigure the number of users, increasing; wait until
                // they all run.
                starts(
                    &["users ", &MAX_USERS.to_string()].concat(),
                    "users configured",
                )
                .then_reach(MAX_USERS),
                // Reconfigure the number of users, decreasing; wait until
                // only they run.
                starts(
                    &["users ", &FINAL_USERS.to_string()].concat(),
                    "users configured",
                )
                .then_reach(FINAL_USERS),
                // A running load test can be stopped; give Goose a half
                // second to stop before moving on.
                starts("stop", "load test stopped").then_wait(time::Duration::from_millis(500)),
            ],
        ),
        (
            Command::Shutdown,
            vec![starts("shutdown", "load test shut down")],
        ),
    ]
}

type WebSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

// A connected Controller client: what differs between the protocols is only
// how a line is sent and its reply read.
enum Client {
    Telnet(TcpStream),
    WebSocket(Box<WebSocket>),
    #[cfg(feature = "mcp")]
    Mcp(mcp_client::Mcp),
}

impl Client {
    // Connect to the `protocol` Controller, the MCP Controller on `mcp_port`.
    async fn connect(protocol: Protocol, mcp_port: u16) -> Self {
        let _ = mcp_port;
        match protocol {
            Protocol::Telnet => {
                let mut client =
                    Client::Telnet(TcpStream::connect("127.0.0.1:5116").await.unwrap());
                // The prompt the telnet Controller greets a client with.
                client.read_telnet().await;
                client
            }
            Protocol::WebSocket => {
                let (mut stream, _item) = tokio_tungstenite::connect_async("ws://127.0.0.1:5117")
                    .await
                    .unwrap();
                // Send an empty message so the client performs a handshake.
                stream.send(Message::Text("".into())).await.unwrap();
                // Ignore the error that comes back.
                let _ = stream.next().await.unwrap();
                Client::WebSocket(Box::new(stream))
            }
            #[cfg(feature = "mcp")]
            Protocol::Mcp => Client::Mcp(mcp_client::Mcp::new(mcp_port)),
        }
    }

    // A telnet reply, to the prompt that ends it, prompt included.
    async fn read_telnet(&mut self) -> String {
        let Client::Telnet(stream) = self else {
            unreachable!("only telnet has a prompt");
        };
        let mut buf = [0u8; 2048];
        let mut reply = Vec::new();
        while !reply.ends_with(b"goose> ") {
            let read = stream.read(&mut buf).await.unwrap();
            assert!(read > 0, "the controller closed mid reply");
            reply.extend_from_slice(&buf[..read]);
        }
        String::from_utf8(reply).unwrap()
    }

    // Send `line` and read its reply.
    async fn send(&mut self, line: &str) -> Reply {
        match self {
            Client::Telnet(stream) => {
                if let Err(e) = stream.write_all(format!("{line}\r\n").as_bytes()).await {
                    panic!("failed to send {} command: {}", line, e);
                }
                Reply {
                    success: None,
                    text: self.read_telnet().await,
                }
            }
            Client::WebSocket(stream) => {
                let request = serde_json::to_string(&ControllerWebSocketRequest {
                    request: format!("{line}\r\n"),
                })
                .unwrap();
                stream.send(Message::Text(request.into())).await.unwrap();
                let message = stream.next().await.expect("error reading from server");
                let text = message
                    .expect("error unwrapping message from server")
                    .into_text()
                    .expect("invalid message type");
                let reply: ControllerWebSocketResponse = match serde_json::from_str(&text) {
                    Ok(reply) => reply,
                    Err(e) => panic!("invalid response from server: {}", e),
                };
                Reply {
                    success: Some(reply.success),
                    text: reply.response,
                }
            }
            #[cfg(feature = "mcp")]
            Client::Mcp(mcp) => {
                let (ok, text) = mcp.send_line(line).await;
                Reply {
                    success: Some(ok),
                    text,
                }
            }
        }
    }

    // Poll the Controller's own metrics, at most every 100 ms for up to 30
    // s, until its history's last row maintains `users`: the users a change
    // asked for are all running.
    async fn wait_for_users(&mut self, users: usize) {
        let deadline = time::Instant::now() + time::Duration::from_secs(30);
        loop {
            let reply = self.send("metrics-json").await;
            let text = &reply.text;
            let json = &text[text.find('{').unwrap()..=text.rfind('}').unwrap()];
            let metrics: serde_json::Value = serde_json::from_str(json).unwrap();
            let last = metrics["history"].as_array().and_then(|rows| rows.last());
            if last.is_some_and(|row| row["action"] == "Maintaining" && row["users"] == users) {
                return;
            }
            assert!(
                time::Instant::now() < deadline,
                "{users} users were not running 30 s after the change: {last:?}"
            );
            tokio::time::sleep(time::Duration::from_millis(100)).await;
        }
    }

    // Before the next command, a telnet client sends an empty line for a
    // fresh prompt, and reads the reply to it.
    async fn new_prompt(&mut self) {
        if let Client::Telnet(stream) = self {
            stream.write_all("\r\n".as_bytes()).await.unwrap();
            self.read_telnet().await;
        }
    }
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
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<Mock<'_>> {
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
    protocol: Protocol,
) {
    // Confirm that we loaded the mock endpoints.
    assert!(mock_endpoints[INDEX_KEY].calls() > 0);
    assert!(mock_endpoints[ABOUT_KEY].calls() > 0);

    // Get index and about out of goose metrics.
    let index_metrics = goose_metrics
        .requests
        .get(&format!("GET {INDEX_PATH}"))
        .unwrap();
    let about_metrics = goose_metrics
        .requests
        .get(&format!("GET {ABOUT_PATH}"))
        .unwrap();

    // There should not have been any failures during this test.
    assert!(index_metrics.fail_count == 0);
    assert!(about_metrics.fail_count == 0);

    // Users were correctly configured through the controller.
    assert!(goose_metrics.total_users == MAX_USERS);

    // Host was not configured at start time.
    assert!(configuration.host.is_none());

    // The load test was manually shut down instead of running to completion.
    assert!(goose_metrics.duration < RUN_TIME);

    // Increasing, Maintaining, Increasing, Maintaining, Decreasing, Maintaining, Canceling,
    // Finished, Finished.
    // Finished is logged twice because `stop` puts the test to Idle, and then `shutdown`
    // actually shuts down the test, and both are logged as "Finished".
    assert!(goose_metrics.history.len() == 9);

    // The Telnet test should create a report-file.
    if protocol == Protocol::Telnet {
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

// Drive the `protocol` Controller through every command it has, checking
// every reply.
async fn drive(protocol: Protocol, server_url: String, mcp_port: u16) {
    // Sleep a half a second allowing the GooseAttack to start.
    tokio::time::sleep(time::Duration::from_millis(500)).await;

    let mut client = Client::connect(protocol, mcp_port).await;
    for (command, steps) in commands(&server_url) {
        if !command.runs_on(protocol) {
            continue;
        }
        for step in steps {
            let reply = client.send(&step.line).await;
            (step.check)(protocol, &reply);
            if let Some(users) = step.reach {
                client.wait_for_users(users).await;
            }
            tokio::time::sleep(step.pause).await;
            // Give the parent process time to catch up.
            tokio::time::sleep(time::Duration::from_millis(100)).await;
        }
        if command == Command::Exit {
            // Re-connect to the Controller.
            client = Client::connect(protocol, mcp_port).await;
        }
        if command != Command::Shutdown {
            client.new_prompt().await;
        }
    }
}

// Helper to run all standalone tests.
async fn run_standalone_test(protocol: Protocol) {
    // Start the mock server.
    let server = MockServer::start();
    let server_url = server.base_url();

    // Setup the endpoints needed for this test on the mock server.
    let mock_endpoints = setup_mock_server_endpoints(&server);

    #[cfg(feature = "mcp")]
    let mcp_port = mcp_client::free_port();
    #[cfg(not(feature = "mcp"))]
    let mcp_port = 0;
    #[cfg(feature = "mcp")]
    let mcp_port_text = mcp_port.to_string();
    let mut configuration_flags = match protocol {
        Protocol::WebSocket => vec!["--websocket", "--no-telnet"],
        // Also test writing a report-file when stopping load test through controller.
        Protocol::Telnet => vec!["--no-websocket", "--report-file", REPORT_FILE],
        #[cfg(feature = "mcp")]
        Protocol::Mcp => vec![
            "--mcp",
            "--mcp-set-host",
            "--mcp-port",
            &mcp_port_text,
            "--no-telnet",
        ],
    };

    // Build common configuration elements.
    let configuration = common_build_configuration(&server, &mut configuration_flags);

    // Drive the Controller from another task, as Goose runs here.
    let controller = tokio::spawn(drive(protocol, server_url, mcp_port));

    // Run the Goose Attack. Add timeout to be sure Goose exits even if the controller tests fail.
    let goose_metrics = tokio::time::timeout(
        tokio::time::Duration::from_secs(60),
        common::run_load_test(common::build_load_test(
            configuration.clone(),
            vec![get_transactions()],
            None,
            None,
        )),
    )
    .await;
    // A failed check in the client is the failure to report, before a load
    // test that timed out because of it.
    if goose_metrics.is_ok() || controller.is_finished() {
        controller.await.expect("the controller client failed");
    }
    let goose_metrics = goose_metrics.expect("load test timed out");

    // Confirm that the load test ran correctly.
    validate_one_scenario(&goose_metrics, &mock_endpoints, &configuration, protocol);
}

// Test controlling a load test with Telnet.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn test_telnet_controller() {
    tokio::time::timeout(
        time::Duration::from_secs(120),
        run_standalone_test(Protocol::Telnet),
    )
    .await
    .expect("test timed out");
}

// Test controlling a load test with WebSocket controller.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn test_websocket_controller() {
    tokio::time::timeout(
        time::Duration::from_secs(120),
        run_standalone_test(Protocol::WebSocket),
    )
    .await
    .expect("test timed out");
}

// Test controlling a load test with the MCP Controller, through the same
// table of commands.
#[cfg(feature = "mcp")]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn test_mcp_controller() {
    tokio::time::timeout(
        time::Duration::from_secs(120),
        run_standalone_test(Protocol::Mcp),
    )
    .await
    .expect("test timed out");
}
