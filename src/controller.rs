//! Optionally launches telnet and WebSocket Controllers.
//!
//! By default, Goose launches both a telnet Controller and a WebSocket Controller, allowing
//! real-time control of the running load test.

use crate::metrics::GooseMetrics;
use crate::GooseConfiguration;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use regex::{Regex, RegexSet};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tungstenite::Message;

use std::io;
use std::str;

/// Goose currently supports two different Controller protocols: telnet and WebSocket.
#[derive(Clone, Debug)]
pub enum GooseControllerProtocol {
    /// Allows control of Goose via telnet.
    Telnet,
    /// Allows control of Goose via a WebSocket.
    WebSocket,
}

/// All commands recognized by the Goose Controllers.
///
/// Developer note: The order commands are defined here must match the order in which
/// the commands are defined in the
/// [`regex::RegexSet`](https://docs.rs/regex/*/regex/struct.RegexSet.html) in
/// [`controller_main()`](./fn.controller_main.html) as it is used to determine which
/// regex matched, if any. Any commands that require a second match to capture values
/// must be defined at the beginning of this enum.
#[derive(Clone, Debug, PartialEq)]
pub enum GooseControllerCommand {
    /// Change how quickly new [`GooseUser`](../goose/struct.GooseUser.html)s are launched.
    HatchRate,
    /// Display the current [`GooseConfiguration`](../struct.GooseConfiguration.html)s.
    Config,
    /// Display the current [`GooseConfiguration`](../struct.GooseConfiguration.html)s in json format.
    ConfigJson,
    /// Display the current [`GooseMetric`](../metrics/struct.GooseMetrics.html)s.
    Metrics,
    /// Display the current [`GooseMetric`](../metrics/struct.GooseMetrics.html)s in json format.
    MetricsJson,
    /// Displays a list of all supported commands.
    Help,
    /// Disconnect from the controller.
    Exit,
    /// Verify that the controller can talk to the parent process.
    Start,
    /// Stop a running test, putting it into an idle state.
    Stop,
    /// Tell the load test to shut down (which will disconnect the controller).
    Shutdown,
}

/// This structure is used to send commands and values to the parent process.
#[derive(Debug)]
pub struct GooseControllerRequestMessage {
    /// The command that is being sent to the parent.
    pub command: GooseControllerCommand,
    /// An optional value that is being sent to the parent.
    pub value: Option<String>,
}

/// An enumeration of all messages the parent can reply back to the controller thread.
#[derive(Debug)]
pub enum GooseControllerResponseMessage {
    /// A response containing a boolean value.
    Bool(bool),
    /// A response containing the load test configuration.
    Config(Box<GooseConfiguration>),
    /// A response containing current load test metrics.
    Metrics(Box<GooseMetrics>),
}

/// The actual request that's passed from the controller to the parent thread.
#[derive(Debug)]
pub struct GooseControllerRequest {
    /// Optional one-shot channel if a reply is required.
    pub response_channel: Option<tokio::sync::oneshot::Sender<GooseControllerResponse>>,
    /// An integer identifying which controller client is making the request.
    pub client_id: u32,
    /// The actual request message.
    pub request: GooseControllerRequestMessage,
}

/// The actual response that's passed from the parent to the controller.
#[derive(Debug)]
pub struct GooseControllerResponse {
    /// An integer identifying which controller the parent is responding to.
    pub client_id: u32,
    /// The actual response message.
    pub response: GooseControllerResponseMessage,
}

/// The required format for any request sent to the WebSocket Controller.
#[derive(Debug, Deserialize)]
pub struct GooseControllerWebSocketRequest {
    /// A valid command string.
    request: String,
}

/// The format of all responses returned by the WebSocket Controller.
#[derive(Debug, Serialize)]
pub struct GooseControllerWebSocketResponse {
    /// The response from the controller.
    response: String,
    /// Whether the request was successful or not.
    success: bool,
    /// If success is false, a description of the error.
    error: Option<String>,
}

/// Return type to indicate whether or not to exit the Controller thread.
type GooseControllerExit = bool;

/// Simplify the GooseController trait definition for WebSockets.
type GooseControllerWebSocketMessage =
    std::result::Result<tungstenite::Message, tungstenite::Error>;

/// Simplify the GooseControllerExecuteCommand trait definition for WebSockets.
type GooseControllerWebSocketSender = futures::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    tungstenite::Message,
>;

/// This state object is created in the main Controller thread and then passed to the specific
/// per-client thread.
pub struct GooseControllerState {
    /// Track which controller-thread this is.
    thread_id: u32,
    /// A shared channel for communicating with the parent process.
    channel_tx: flume::Sender<GooseControllerRequest>,
    /// A compiled set of regular expressions used for matching commands.
    commands: RegexSet,
    /// A compiled vector of regular expressions used for capturing values from commands.
    captures: Vec<Regex>,
    /// Which protocol this Controller understands.
    protocol: GooseControllerProtocol,
}
// Defines functions shared by all Controllers.
impl GooseControllerState {
    async fn accept_connections(self, mut socket: tokio::net::TcpStream) {
        let peer_addr = socket
            .peer_addr()
            .map_or("UNKNOWN ADDRESS".to_string(), |p| p.to_string());
        info!(
            "{:?} client [{}] connected from {}",
            self.protocol, self.thread_id, peer_addr
        );

        match self.protocol {
            GooseControllerProtocol::Telnet => {
                let mut buf: [u8; 1024] = [0; 1024];

                // Display initial goose> prompt.
                write_to_socket_raw(&mut socket, "goose> ").await;

                loop {
                    // Process data received from the client in a loop.
                    let n = socket
                        .read(&mut buf)
                        .await
                        .expect("failed to read data from socket");

                    // Invalid request, exit.
                    if n == 0 {
                        return;
                    }

                    // Extract the command string in a protocol-specific way.
                    if let Ok(command_string) = self.get_command_string(buf).await {
                        // Extract the command and value in a generic way.
                        if let Ok(request_message) = self.get_match(&command_string).await {
                            // Act on the commmand received.
                            if self.execute_command(&mut socket, request_message).await {
                                // If execute_command returns true, it's time to exit.
                                info!(
                                    "telnet client [{}] disconnected from {}",
                                    self.thread_id, peer_addr
                                );
                                break;
                            }
                        } else {
                            self.write_to_socket(
                                &mut socket,
                                "unrecognized command".to_string(),
                                None,
                            )
                            .await;
                        }
                    } else {
                        // Corrupted request from telnet client, exit.
                        info!(
                            "telnet client [{}] disconnected from {}",
                            self.thread_id, peer_addr
                        );
                        break;
                    }
                }
            }
            GooseControllerProtocol::WebSocket => {
                let stream = match tokio_tungstenite::accept_async(socket).await {
                    Ok(s) => s,
                    Err(e) => {
                        info!("invalid websocket handshake: {}", e);
                        return;
                    }
                };
                let (mut ws_sender, mut ws_receiver) = stream.split();

                loop {
                    // Wait until the client sends a command.
                    let data = ws_receiver
                        .next()
                        .await
                        .expect("failed to read data from socket");

                    // Extract the command string in a protocol-specific way.
                    if let Ok(command_string) = self.get_command_string(data).await {
                        // Extract the command and value in a generic way.
                        if let Ok(request_message) = self.get_match(&command_string).await {
                            if self.execute_command(&mut ws_sender, request_message).await {
                                // If execute_command() returns true, it's time to exit.
                                info!(
                                    "telnet client [{}] disconnected from {}",
                                    self.thread_id, peer_addr
                                );
                                break;
                            }
                        } else {
                            self.write_to_socket(
                                &mut ws_sender,
                                "unrecognized command".to_string(),
                                Some("unrecognized command".to_string()),
                            )
                            .await;
                        }
                    }
                }
            }
        }
    }

    // Both Controllers use a common function to identify commands.
    async fn get_match(&self, command_string: &str) -> Result<GooseControllerRequestMessage, ()> {
        let matches = self.commands.matches(&command_string);
        if matches.matched(GooseControllerCommand::Help as usize) {
            Ok(GooseControllerRequestMessage {
                command: GooseControllerCommand::Help,
                value: None,
            })
        } else if matches.matched(GooseControllerCommand::Exit as usize) {
            Ok(GooseControllerRequestMessage {
                command: GooseControllerCommand::Exit,
                value: None,
            })
        } else if matches.matched(GooseControllerCommand::Start as usize) {
            Ok(GooseControllerRequestMessage {
                command: GooseControllerCommand::Start,
                value: None,
            })
        } else if matches.matched(GooseControllerCommand::Stop as usize) {
            Ok(GooseControllerRequestMessage {
                command: GooseControllerCommand::Stop,
                value: None,
            })
        } else if matches.matched(GooseControllerCommand::Shutdown as usize) {
            Ok(GooseControllerRequestMessage {
                command: GooseControllerCommand::Shutdown,
                value: None,
            })
        } else if matches.matched(GooseControllerCommand::Config as usize) {
            Ok(GooseControllerRequestMessage {
                command: GooseControllerCommand::Config,
                value: None,
            })
        } else if matches.matched(GooseControllerCommand::ConfigJson as usize) {
            Ok(GooseControllerRequestMessage {
                command: GooseControllerCommand::ConfigJson,
                value: None,
            })
        } else if matches.matched(GooseControllerCommand::Metrics as usize) {
            Ok(GooseControllerRequestMessage {
                command: GooseControllerCommand::Metrics,
                value: None,
            })
        } else if matches.matched(GooseControllerCommand::MetricsJson as usize) {
            Ok(GooseControllerRequestMessage {
                command: GooseControllerCommand::MetricsJson,
                value: None,
            })
        } else if matches.matched(GooseControllerCommand::HatchRate as usize) {
            // Perform a second regex to capture the hatch_rate value.
            let caps = self.captures[GooseControllerCommand::HatchRate as usize]
                .captures(command_string)
                .unwrap();
            let hatch_rate = caps.get(2).map_or("", |m| m.as_str());
            Ok(GooseControllerRequestMessage {
                command: GooseControllerCommand::HatchRate,
                value: Some(hatch_rate.to_string()),
            })
        } else {
            Err(())
        }
    }

    // Use a rust match to enforce at compile time that all commands are supported.
    async fn process_command(
        &self,
        request_message: GooseControllerRequestMessage,
    ) -> Result<GooseControllerResponseMessage, String> {
        match self.send_to_parent_and_get_reply(request_message).await {
            Ok(r) => Ok(r),
            Err(e) => Err(format!("controller command failed: {}", e)),
        }
    }

    fn process_local_command(
        &self,
        request_message: &GooseControllerRequestMessage,
    ) -> Option<String> {
        match request_message.command {
            GooseControllerCommand::Help => Some(display_help()),
            GooseControllerCommand::Exit => Some("goodbye!".to_string()),
            _ => None,
        }
    }

    // Process the response received back from the parent process after running a command.
    fn process_response(
        &self,
        command: GooseControllerCommand,
        response: GooseControllerResponseMessage,
    ) -> Result<String, String> {
        match command {
            GooseControllerCommand::HatchRate => {
                if let GooseControllerResponseMessage::Bool(true) = response {
                    Ok("hatch_rate reconfigured".to_string())
                } else {
                    Err("failed to reconfigure hatch_rate".to_string())
                }
            }
            GooseControllerCommand::Config => {
                if let GooseControllerResponseMessage::Config(config) = response {
                    Ok(format!("{:#?}", config))
                } else {
                    Err("error loading configuration".to_string())
                }
            }
            GooseControllerCommand::ConfigJson => {
                if let GooseControllerResponseMessage::Config(config) = response {
                    Ok(serde_json::to_string(&config).expect("unexpected serde failure"))
                } else {
                    Err("error loading configuration".to_string())
                }
            }
            GooseControllerCommand::Metrics => {
                if let GooseControllerResponseMessage::Metrics(metrics) = response {
                    Ok(metrics.to_string())
                } else {
                    Err("error loading metrics".to_string())
                }
            }
            GooseControllerCommand::MetricsJson => {
                if let GooseControllerResponseMessage::Metrics(metrics) = response {
                    Ok(serde_json::to_string(&metrics).expect("unexpected serde failure"))
                } else {
                    Err("error loading metrics".to_string())
                }
            }
            GooseControllerCommand::Start => {
                if let GooseControllerResponseMessage::Bool(true) = response {
                    Ok("load test started".to_string())
                } else {
                    Err("load test not idle, failed to start".to_string())
                }
            }
            // This shouldn't work if the load test isn't running.
            GooseControllerCommand::Stop => {
                if let GooseControllerResponseMessage::Bool(true) = response {
                    Ok("load test stoped".to_string())
                } else {
                    Err("load test not running, failed to stop".to_string())
                }
            }
            GooseControllerCommand::Shutdown => {
                if let GooseControllerResponseMessage::Bool(true) = response {
                    Ok("load test shut down".to_string())
                } else {
                    Err("failed to shut down load test".to_string())
                }
            }
            // These commands are processed earlier so we should never get here.
            GooseControllerCommand::Help | GooseControllerCommand::Exit => {
                let e = "received an impossible HELP or EXIT command";
                error!("{}", e);
                Err(e.to_string())
            }
        }
    }

    /// Send a message to parent thread, with or without an optional value, and wait for
    /// a reply.
    async fn send_to_parent_and_get_reply(
        &self,
        request: GooseControllerRequestMessage,
    ) -> Result<GooseControllerResponseMessage, String> {
        // Create a one-shot channel to allow the parent to reply to our request. As flume
        // doesn't implement a one-shot channel, we use tokio for this temporary channel.
        let (response_tx, response_rx): (
            tokio::sync::oneshot::Sender<GooseControllerResponse>,
            tokio::sync::oneshot::Receiver<GooseControllerResponse>,
        ) = tokio::sync::oneshot::channel();

        if self
            .channel_tx
            .try_send(GooseControllerRequest {
                response_channel: Some(response_tx),
                client_id: self.thread_id,
                request,
            })
            .is_err()
        {
            return Err("parent process has closed the controller channel".to_string());
        }

        // Await response from parent.
        match response_rx.await {
            Ok(value) => Ok(value.response),
            Err(e) => Err(format!("one-shot channel dropped without reply: {}", e)),
        }
    }
}

/// Controller-protocol-specific functions, necessary to manage the different way each
/// Controller protocol communicates with a client.
#[async_trait]
trait GooseController<T> {
    // Extract the command string from a Controller client request.
    async fn get_command_string(&self, raw_value: T) -> Result<String, String>;
}
#[async_trait]
impl GooseController<[u8; 1024]> for GooseControllerState {
    // Extract the command string from a telnet Controller client request.
    async fn get_command_string(&self, raw_value: [u8; 1024]) -> Result<String, String> {
        let command_string = match str::from_utf8(&raw_value) {
            Ok(m) => {
                if let Some(c) = m.lines().next() {
                    c
                } else {
                    ""
                }
            }
            Err(e) => {
                let error = format!("ignoring unexpected input from telnet controller: {}", e);
                info!("{}", error);
                return Err(error);
            }
        };

        Ok(command_string.to_string())
    }
}
#[async_trait]
impl GooseController<GooseControllerWebSocketMessage> for GooseControllerState {
    // Extract the command string from a WebSocket Controller client request.
    async fn get_command_string(
        &self,
        raw_value: GooseControllerWebSocketMessage,
    ) -> Result<String, String> {
        if let Ok(request) = raw_value {
            if request.is_text() {
                if let Ok(request) = request.into_text() {
                    debug!("websocket request: {:?}", request.trim());
                    let command_string: GooseControllerWebSocketRequest =
                        match serde_json::from_str(&request) {
                            Ok(c) => c,
                            Err(_) => {
                                return Err("unrecognized json request, refer to Goose README.md"
                                    .to_string())
                            }
                        };
                    return Ok(command_string.request);
                }
            }
        }

        Err(
            "failed to get command string from WebSocket request, refer to Goose README.md"
                .to_string(),
        )
    }
}
#[async_trait]
trait GooseControllerExecuteCommand<T> {
    // Run the command received from a Controller request.
    async fn execute_command(
        &self,
        socket: &mut T,
        request_message: GooseControllerRequestMessage,
    ) -> GooseControllerExit;

    // Send response to Controller client.
    async fn write_to_socket(
        &self,
        socket: &mut T,
        response_message: String,
        error: Option<String>,
    );
}
#[async_trait]
impl GooseControllerExecuteCommand<tokio::net::TcpStream> for GooseControllerState {
    // Run the command received from a telnet Controller request.
    async fn execute_command(
        &self,
        socket: &mut tokio::net::TcpStream,
        request_message: GooseControllerRequestMessage,
    ) -> GooseControllerExit {
        // First handle commands that don't require interaction with the parent process.
        if let Some(message) = self.process_local_command(&request_message) {
            self.write_to_socket(socket, message, None).await;
            // Return true if it's time to exit, false if not.
            return request_message.command == GooseControllerCommand::Exit;
        }

        // Retain a copy of the command used when processing the parent response.
        let command = request_message.command.clone();

        // Now handle commands that require interaction with the parent process.
        let response = match self.process_command(request_message).await {
            Ok(r) => r,
            Err(e) => {
                self.write_to_socket(socket, e, None).await;
                return false;
            }
        };

        // Determine whether or not to exit controller.
        let exit_controller = command == GooseControllerCommand::Shutdown;

        // Always write the returned String to the socket, whether or not the command
        // was successful.
        match self.process_response(command.clone(), response) {
            Ok(message) => self.write_to_socket(socket, message, None).await,
            Err(message) => self.write_to_socket(socket, message, None).await,
        }

        exit_controller
    }

    // Send response to Controller client.
    async fn write_to_socket(
        &self,
        socket: &mut tokio::net::TcpStream,
        response_message: String,
        _error: Option<String>,
    ) {
        socket
            // Add a linefeed to the end of the message.
            .write_all([&response_message, "\ngoose> "].concat().as_bytes())
            .await
            .expect("failed to write data to socket");
    }
}
#[async_trait]
impl GooseControllerExecuteCommand<GooseControllerWebSocketSender> for GooseControllerState {
    // Run the command received from a WebSocket Controller request.
    async fn execute_command(
        &self,
        socket: &mut GooseControllerWebSocketSender,
        request_message: GooseControllerRequestMessage,
    ) -> GooseControllerExit {
        // First handle commands that don't require interaction with the parent process.
        if let Some(message) = self.process_local_command(&request_message) {
            self.write_to_socket(socket, message, None).await;

            // Set exit_controller to true if Exit command was received.
            let exit_controller = request_message.command == GooseControllerCommand::Exit;

            // Notify the WebSocket client that this connection is closing.
            if exit_controller {
                socket
                    .send(Message::Close(Some(tungstenite::protocol::CloseFrame {
                        code: tungstenite::protocol::frame::coding::CloseCode::Normal,
                        reason: std::borrow::Cow::Borrowed("exit"),
                    })))
                    .await
                    .expect("failed to write data to stream");
            }

            return exit_controller;
        }

        // GooseController always returns JSON
        let command = if request_message.command == GooseControllerCommand::Config {
            GooseControllerCommand::ConfigJson
        } else if request_message.command == GooseControllerCommand::Metrics {
            GooseControllerCommand::MetricsJson
        } else {
            request_message.command.clone()
        };

        // Now handle commands that require interaction with the parent process.
        let response = match self.process_command(request_message).await {
            Ok(r) => r,
            Err(e) => {
                self.write_to_socket(socket, e, None).await;
                return false;
            }
        };

        // Determine whether or not to exit controller.
        let exit_controller = command == GooseControllerCommand::Shutdown;

        match self.process_response(command, response) {
            // Command was processed successfully.
            Ok(message) => {
                self.write_to_socket(socket, message, None).await;
            }
            // Command failed.
            Err(error) => {
                self.write_to_socket(socket, error.clone(), Some(error))
                    .await;
            }
        }

        // Notify the WebSocket client that this connection is closing.
        if exit_controller {
            socket
                .send(Message::Close(Some(tungstenite::protocol::CloseFrame {
                    code: tungstenite::protocol::frame::coding::CloseCode::Normal,
                    reason: std::borrow::Cow::Borrowed("shutdown"),
                })))
                .await
                .expect("failed to write data to stream");
        }

        // Return true if it's time to exit the controller.
        exit_controller
    }

    // Send a json-formatted response to the WebSocket.
    async fn write_to_socket(
        &self,
        socket: &mut GooseControllerWebSocketSender,
        response: String,
        error: Option<String>,
    ) {
        if let Err(e) = socket
            .send(Message::Text(
                match serde_json::to_string(&GooseControllerWebSocketResponse {
                    response,
                    // Success is true if there is no error, false if there is an error.
                    success: error.is_none(),
                    error,
                }) {
                    Ok(json) => json,
                    Err(e) => {
                        warn!("failed to json encode response: {}", e);
                        return;
                    }
                },
            ))
            .await
        {
            warn!("failed to write data to websocket: {}", e);
        }
    }
}

/// The control loop listens for connections on the configured TCP port. Each connection
/// spawns a new thread so multiple clients can connect. Handles incoming connections for
/// both telnet and WebSocket clients.
///  -  @TODO: optionally limit how many controller connections are allowed
///  -  @TODO: optionally require client authentication
///  -  @TODO: optionally ssl-encrypt client communication
pub async fn controller_main(
    // Expose load test configuration to controller thread.
    configuration: GooseConfiguration,
    // For sending requests to the parent process.
    channel_tx: flume::Sender<GooseControllerRequest>,
    // Which type of controller to launch.
    protocol: GooseControllerProtocol,
) -> io::Result<()> {
    // Build protocol-appropriate address.
    let address = match &protocol {
        GooseControllerProtocol::Telnet => format!(
            "{}:{}",
            configuration.telnet_host, configuration.telnet_port
        ),
        GooseControllerProtocol::WebSocket => format!(
            "{}:{}",
            configuration.websocket_host, configuration.websocket_port
        ),
    };

    // All controllers use a TcpListener port.
    debug!(
        "preparing to bind {:?} controller to: {}",
        protocol, address
    );
    let listener = TcpListener::bind(&address).await?;
    info!("{:?} controller listening on: {}", protocol, address);

    // These first regular expressions are compiled twice. Once as part of a set used to match
    // against a command. The second time to capture specific matched values. This is a
    // limitiation of RegexSet as documented at:
    // https://docs.rs/regex/1.5.4/regex/struct.RegexSet.html#limitations
    let hatchrate_regex = r"(?i)^(hatchrate|hatch_rate) ([0-9]*(\.[0-9]*)?){1}$";
    // @TODO: enable when the parent process processes it properly.
    //let users_regex = r"(?i)^users (\d+)$";

    // The following RegexSet is matched against all commands received through the controller.
    // Developer note: The order commands are defined here must match the order in which
    // the commands are defined in the GooseControllerCommand enum, as it is used to determine
    // which expression matched, if any.
    let commands = RegexSet::new(&[
        // Modify how quickly users hatch (or exit if users are reduced).
        hatchrate_regex,
        // Display the current load test configuration.
        r"(?i)^config$",
        // Display the current load test configuration in json.
        r"(?i)^config-json$",
        // Display running metrics for the currently active load test.
        r"(?i)^(metrics|stats)$",
        // Display running metrics for the currently active load test in json.
        r"(?i)^(metrics-json|stats-json)$",
        // Provide a list of possible commands.
        r"(?i)^(help|\?)$",
        // Exit/quit the controller connection, does not affect load test.
        r"(?i)^(exit|quit)$",
        // Start an idle load test.
        r"(?i)^start$",
        // Stop an idle load test.
        r"(?i)^stop$",
        // Shutdown the load test (which will cause the controller connection to quit).
        r"(?i)^shutdown$",
    ])
    .unwrap();

    // The following regular expressions are used when matching against certain commands
    // to then capture a matched value.
    let captures = vec![Regex::new(hatchrate_regex).unwrap()];

    // Counter increments each time a controller client connects with this protocol.
    let mut thread_id: u32 = 0;

    // Wait for a connection.
    while let Ok((stream, _)) = listener.accept().await {
        thread_id += 1;

        let controller_state = GooseControllerState {
            thread_id,
            channel_tx: channel_tx.clone(),
            commands: commands.clone(),
            captures: captures.clone(),
            protocol: protocol.clone(),
        };

        // Spawn a new thread to communicate with a client. The returned JoinHandle is
        // ignored as the thread simply runs until the client exits or Goose shuts down.
        let _ = tokio::spawn(controller_state.accept_connections(stream));
    }

    Ok(())
}

/// Send a message to the client TcpStream, no prompt or line feed.
async fn write_to_socket_raw(socket: &mut tokio::net::TcpStream, message: &str) {
    socket
        // Add a linefeed to the end of the message.
        .write_all(message.as_bytes())
        .await
        .expect("failed to write data to socket");
}

// A controller help screen.
// @TODO: document `users` when enabled:
// users INT          set number of simulated users
fn display_help() -> String {
    format!(
        r"{} {} controller commands:
 help (?)           this help
 exit (quit)        exit controller
 start              start an idle load test
 stop               stop a running load test and return to idle state
 shutdown           shutdown running load test (and exit controller)
 hatchrate FLOAT    set per-second rate users hatch
 config             display load test configuration
 config-json        display load test configuration in json format
 metrics            display metrics for current load test
 metrics-json       display metrics for current load test in json format",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    )
}
