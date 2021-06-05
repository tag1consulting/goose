//! Optionally launches telnet and WebSocket Controllers.
//!
//! By default, Goose launches both a telnet Controller and a WebSocket Controller, allowing
//! real-time control of the running load test.

use crate::metrics::GooseMetrics;
use crate::util;
use crate::{AttackPhase, GooseAttack, GooseAttackRunState, GooseConfiguration, GooseError};

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use regex::{Regex, RegexSet};
use serde::{Deserialize, Serialize};
use std::io;
use std::str;
use std::str::FromStr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tungstenite::Message;

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
///
/// @TODO: Document the steps necessary to add a new Controller command.
#[derive(Clone, Debug, PartialEq)]
pub enum GooseControllerCommand {
    /// Configure the host to load test, for example http://localhost/.
    Host,
    /// Configure how many [`GooseUser`](../goose/struct.GooseUser.html)s are launched.
    Users,
    /// Configure how quickly new [`GooseUser`](../goose/struct.GooseUser.html)s are launched.
    HatchRate,
    /// Configure how long the load test should run before stopping.
    RunTime,
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
}

/// Return type to indicate whether or not to exit the Controller thread.
type GooseControllerExit = bool;

/// The telnet Controller message buffer.
type GooseControllerTelnetMessage = [u8; 1024];

/// The WebSocket Controller message buffer.
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
    /// Track the ip and port of the connected TCP client.
    peer_address: String,
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
        info!(
            "{:?} client [{}] connected from {}",
            self.protocol, self.thread_id, self.peer_address
        );
        match self.protocol {
            GooseControllerProtocol::Telnet => {
                let mut buf: GooseControllerTelnetMessage = [0; 1024];

                // Display initial goose> prompt.
                write_to_socket_raw(&mut socket, "goose> ").await;

                loop {
                    // Process data received from the client in a loop.
                    let n = match socket.read(&mut buf).await {
                        Ok(data) => data,
                        Err(_) => {
                            info!(
                                "Telnet client [{}] disconnected from {}",
                                self.thread_id, self.peer_address
                            );
                            break;
                        }
                    };

                    // Invalid request, exit.
                    if n == 0 {
                        info!(
                            "Telnet client [{}] disconnected from {}",
                            self.thread_id, self.peer_address
                        );
                        break;
                    }

                    // Extract the command string in a protocol-specific way.
                    if let Ok(command_string) = self.get_command_string(buf).await {
                        // Extract the command and value in a generic way.
                        if let Ok(request_message) = self.get_match(&command_string).await {
                            // Act on the commmand received.
                            if self.execute_command(&mut socket, request_message).await {
                                // If execute_command returns true, it's time to exit.
                                info!(
                                    "Telnet client [{}] disconnected from {}",
                                    self.thread_id, self.peer_address
                                );
                                break;
                            }
                        } else {
                            self.write_to_socket(
                                &mut socket,
                                Err("unrecognized command".to_string()),
                            )
                            .await;
                        }
                    } else {
                        // Corrupted request from telnet client, exit.
                        info!(
                            "Telnet client [{}] disconnected from {}",
                            self.thread_id, self.peer_address
                        );
                        break;
                    }
                }
            }
            GooseControllerProtocol::WebSocket => {
                let stream = match tokio_tungstenite::accept_async(socket).await {
                    Ok(s) => s,
                    Err(e) => {
                        info!("invalid WebSocket handshake: {}", e);
                        return;
                    }
                };
                let (mut ws_sender, mut ws_receiver) = stream.split();

                loop {
                    // Wait until the client sends a command.
                    let data = match ws_receiver.next().await {
                        Some(d) => (d),
                        None => {
                            // Returning with no data means the client disconnected.
                            info!(
                                "Telnet client [{}] disconnected from {}",
                                self.thread_id, self.peer_address
                            );
                            break;
                        }
                    };

                    // Extract the command string in a protocol-specific way.
                    if let Ok(command_string) = self.get_command_string(data).await {
                        // Extract the command and value in a generic way.
                        if let Ok(request_message) = self.get_match(&command_string).await {
                            if self.execute_command(&mut ws_sender, request_message).await {
                                // If execute_command() returns true, it's time to exit.
                                info!(
                                    "Telnet client [{}] disconnected from {}",
                                    self.thread_id, self.peer_address
                                );
                                break;
                            }
                        } else {
                            self.write_to_socket(
                                &mut ws_sender,
                                Err("unrecognized command, see Goose README.md".to_string()),
                            )
                            .await;
                        }
                    } else {
                        self.write_to_socket(
                            &mut ws_sender,
                            Err("unable to parse json, see Goose README.md".to_string()),
                        )
                        .await;
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
        } else if matches.matched(GooseControllerCommand::Host as usize) {
            // Perform a second regex to capture the host value.
            let caps = self.captures[GooseControllerCommand::Host as usize]
                .captures(command_string)
                .unwrap();
            let host = caps.get(2).map_or("", |m| m.as_str());
            // The Regex that captures the host only validates that the host starts with
            // http:// or https://. Now use a library to properly validate that this is
            // a valid host before sending to the parent process.
            if util::is_valid_host(host).is_ok() {
                Ok(GooseControllerRequestMessage {
                    command: GooseControllerCommand::Host,
                    value: Some(host.to_string()),
                })
            } else {
                debug!("invalid host: {}", host);
                Err(())
            }
        } else if matches.matched(GooseControllerCommand::Users as usize) {
            // Perform a second regex to capture the users value.
            let caps = self.captures[GooseControllerCommand::Users as usize]
                .captures(command_string)
                .unwrap();
            let users = caps.get(2).map_or("", |m| m.as_str());
            Ok(GooseControllerRequestMessage {
                command: GooseControllerCommand::Users,
                value: Some(users.to_string()),
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
        } else if matches.matched(GooseControllerCommand::RunTime as usize) {
            // Perform a second regex to capture the run_time value.
            let caps = self.captures[GooseControllerCommand::RunTime as usize]
                .captures(command_string)
                .unwrap();
            let run_time = caps.get(2).map_or("", |m| m.as_str());
            Ok(GooseControllerRequestMessage {
                command: GooseControllerCommand::RunTime,
                value: Some(run_time.to_string()),
            })
        } else {
            Err(())
        }
    }

    /// Process a request entirely within the Controller thread, without sending a message
    /// to the parent thread.
    fn process_local_command(
        &self,
        request_message: &GooseControllerRequestMessage,
    ) -> Option<String> {
        match request_message.command {
            GooseControllerCommand::Help => Some(display_help()),
            GooseControllerCommand::Exit => Some("goodbye!".to_string()),
            // All other commands require sending the request to the parent thread.
            _ => None,
        }
    }

    /// Send a message to parent thread, with or without an optional value, and wait for
    /// a reply.
    async fn process_command(
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

    // Process the response received back from the parent process after running a command.
    fn process_response(
        &self,
        command: GooseControllerCommand,
        response: GooseControllerResponseMessage,
    ) -> Result<String, String> {
        match command {
            GooseControllerCommand::Host => {
                if let GooseControllerResponseMessage::Bool(true) = response {
                    Ok("host configured".to_string())
                } else {
                    Err("load test not idle, failed to reconfigure host".to_string())
                }
            }
            GooseControllerCommand::Users => {
                if let GooseControllerResponseMessage::Bool(true) = response {
                    Ok("users configured".to_string())
                } else {
                    Err("load test not idle, failed to reconfigure users".to_string())
                }
            }
            GooseControllerCommand::HatchRate => {
                if let GooseControllerResponseMessage::Bool(true) = response {
                    Ok("hatch_rate configured".to_string())
                } else {
                    Err("failed to configure hatch_rate".to_string())
                }
            }
            GooseControllerCommand::RunTime => {
                if let GooseControllerResponseMessage::Bool(true) = response {
                    Ok("run_time configured".to_string())
                } else {
                    Err("failed to configure run_time".to_string())
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
}

/// Controller-protocol-specific functions, necessary to manage the different way each
/// Controller protocol communicates with a client.
#[async_trait]
trait GooseController<T> {
    // Extract the command string from a Controller client request.
    async fn get_command_string(&self, raw_value: T) -> Result<String, String>;
}
#[async_trait]
impl GooseController<GooseControllerTelnetMessage> for GooseControllerState {
    // Extract the command string from a telnet Controller client request.
    async fn get_command_string(
        &self,
        raw_value: GooseControllerTelnetMessage,
    ) -> Result<String, String> {
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
                } else {
                    // Failed to consume the WebSocket message and convert it to a String.
                    return Err("unsupported string format".to_string());
                }
            } else {
                // Received a non-text WebSocket message.
                return Err("unsupported format, requests must be sent as text".to_string());
            }
        }
        // Improper WebSocket handshake.
        Err("WebSocket handshake error".to_string())
    }
}
#[async_trait]
trait GooseControllerExecuteCommand<T> {
    // Run the command received from a Controller request. Returns a boolean, if true exit.
    async fn execute_command(
        &self,
        socket: &mut T,
        request_message: GooseControllerRequestMessage,
    ) -> GooseControllerExit;

    // Send response to Controller client. The response is wrapped in a Result to indicate
    // if the request was successful or not.
    async fn write_to_socket(&self, socket: &mut T, response_message: Result<String, String>);
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
            self.write_to_socket(socket, Ok(message)).await;
            // If Exit was received return true to exit, otherwise return false.
            return request_message.command == GooseControllerCommand::Exit;
        }

        // Retain a copy of the command used when processing the parent response.
        let command = request_message.command.clone();

        // Now handle commands that require interaction with the parent process.
        let response = match self.process_command(request_message).await {
            Ok(r) => r,
            Err(e) => {
                // Receiving an error here means the parent closed the communication
                // channel. Write the error to the Controller client and then return
                // true to exit.
                self.write_to_socket(socket, Err(e)).await;
                return true;
            }
        };

        // If Shutdown command was received return true to exit, otherwise return false.
        let exit_controller = command == GooseControllerCommand::Shutdown;

        // Write the response to the Controller client socket.
        self.write_to_socket(socket, self.process_response(command, response))
            .await;

        // Return true if it's time to exit the Controller.
        exit_controller
    }

    // Send response to telnet Controller client.
    async fn write_to_socket(
        &self,
        socket: &mut tokio::net::TcpStream,
        message: Result<String, String>,
    ) {
        // Send result to telnet Controller client, whether Ok() or Err().
        let response_message = match message {
            Ok(m) => m,
            Err(e) => e,
        };
        if socket
            // Add a linefeed to the end of the message, followed by a prompt.
            .write_all([&response_message, "\ngoose> "].concat().as_bytes())
            .await
            .is_err()
        {
            warn!("failed to write data to socker");
        };
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
            self.write_to_socket(socket, Ok(message)).await;

            // If Exit was received return true to exit, otherwise return false.
            let exit_controller = request_message.command == GooseControllerCommand::Exit;
            // If exiting, notify the WebSocket client that this connection is closing.
            if exit_controller
                && socket
                    .send(Message::Close(Some(tungstenite::protocol::CloseFrame {
                        code: tungstenite::protocol::frame::coding::CloseCode::Normal,
                        reason: std::borrow::Cow::Borrowed("exit"),
                    })))
                    .await
                    .is_err()
            {
                warn!("failed to write data to stream");
            }

            return exit_controller;
        }

        // WebSocket Controller always returns JSON, convert command where necessary.
        let command = match request_message.command {
            GooseControllerCommand::Config => GooseControllerCommand::ConfigJson,
            GooseControllerCommand::Metrics => GooseControllerCommand::MetricsJson,
            _ => request_message.command.clone(),
        };

        // Now handle commands that require interaction with the parent process.
        let response = match self.process_command(request_message).await {
            Ok(r) => r,
            Err(e) => {
                // Receiving an error here means the parent closed the communication
                // channel. Write the error to the Controller client and then return
                // true to exit.
                self.write_to_socket(socket, Err(e)).await;
                return true;
            }
        };

        // If Shutdown command was received return true to exit, otherwise return false.
        let exit_controller = command == GooseControllerCommand::Shutdown;

        // Write the response to the Controller client socket.
        self.write_to_socket(socket, self.process_response(command, response))
            .await;

        // If exiting, notify the WebSocket client that this connection is closing.
        if exit_controller
            && socket
                .send(Message::Close(Some(tungstenite::protocol::CloseFrame {
                    code: tungstenite::protocol::frame::coding::CloseCode::Normal,
                    reason: std::borrow::Cow::Borrowed("shutdown"),
                })))
                .await
                .is_err()
        {
            warn!("failed to write data to stream");
        }

        // Return true if it's time to exit the Controller.
        exit_controller
    }

    // Send a json-formatted response to the WebSocket.
    async fn write_to_socket(
        &self,
        socket: &mut GooseControllerWebSocketSender,
        response_result: Result<String, String>,
    ) {
        let success;
        let response = match response_result {
            Ok(m) => {
                success = true;
                m
            }
            Err(e) => {
                success = false;
                e
            }
        };
        if let Err(e) = socket
            .send(Message::Text(
                match serde_json::to_string(&GooseControllerWebSocketResponse {
                    response,
                    // Success is true if there is no error, false if there is an error.
                    success,
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
            info!("failed to write data to websocket: {}", e);
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
    let host_regex = r"(?i)^(host|hostname|host_name|host-name) ((https?)://.+)$";
    let users_regex = r"(?i)^(users?) (\d+)$";
    let hatchrate_regex = r"(?i)^(hatchrate|hatch_rate|hatch-rate) ([0-9]*(\.[0-9]*)?){1}$";
    let runtime_regex =
        r"(?i)^(run|runtime|run_time|run-time|) (\d+|((\d+?)h)?((\d+?)m)?((\d+?)s)?)$";

    // The following RegexSet is matched against all commands received through the controller.
    // Developer note: The order commands are defined here must match the order in which
    // the commands are defined in the GooseControllerCommand enum, as it is used to determine
    // which expression matched, if any.
    let commands = RegexSet::new(&[
        // Modify the host the load test runs against.
        host_regex,
        // Modify how many users hatch.
        users_regex,
        // Modify how quickly users hatch.
        hatchrate_regex,
        // Modify how long the load test will run.
        runtime_regex,
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
    let captures = vec![
        Regex::new(host_regex).unwrap(),
        Regex::new(users_regex).unwrap(),
        Regex::new(hatchrate_regex).unwrap(),
        Regex::new(runtime_regex).unwrap(),
    ];

    // Counter increments each time a controller client connects with this protocol.
    let mut thread_id: u32 = 0;

    // Wait for a connection.
    while let Ok((stream, _)) = listener.accept().await {
        thread_id += 1;

        // Identify the client ip and port, used primarily for debug logging.
        let peer_address = stream
            .peer_addr()
            .map_or("UNKNOWN ADDRESS".to_string(), |p| p.to_string());

        // Create a per-client Controller state.
        let controller_state = GooseControllerState {
            thread_id,
            peer_address,
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
    if socket
        // Add a linefeed to the end of the message.
        .write_all(message.as_bytes())
        .await
        .is_err()
    {
        warn!("failed to write data to socket");
    }
}

// A controller help screen.
fn display_help() -> String {
    format!(
        r"{} {} controller commands:
 help (?)           this help
 exit (quit)        exit controller
 start              start an idle load test
 stop               stop a running load test and return to idle state
 shutdown           shutdown running load test (and exit controller)
 host HOST          set host to load test, ie http://localhost/
 users INT          set number of simulated users
 hatchrate FLOAT    set per-second rate users hatch
 runtime TIME       set how long to run test, ie 1h30m5s
 config             display load test configuration
 config-json        display load test configuration in json format
 metrics            display metrics for current load test
 metrics-json       display metrics for current load test in json format",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    )
}

/// The parent process side of the Controller functionality.
impl GooseAttack {
    /// Use the provided oneshot channel to reply to a controller client request.
    pub(crate) fn reply_to_controller(
        &mut self,
        request: GooseControllerRequest,
        response: GooseControllerResponseMessage,
    ) {
        if let Some(oneshot_tx) = request.response_channel {
            if oneshot_tx
                .send(GooseControllerResponse {
                    client_id: request.client_id,
                    response,
                })
                .is_err()
            {
                warn!("failed to send response to controller via one-shot channel")
            }
        }
    }

    /// Handle Controller requests.
    pub(crate) async fn handle_controller_requests(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        // If the controller is enabled, check if we've received any
        // messages.
        if let Some(c) = goose_attack_run_state.controller_channel_rx.as_ref() {
            match c.try_recv() {
                Ok(message) => {
                    info!(
                        "request from controller client {}: {:?}",
                        message.client_id, message.request
                    );
                    match &message.request.command {
                        // Send back a copy of the running configuration.
                        GooseControllerCommand::Config | GooseControllerCommand::ConfigJson => {
                            self.reply_to_controller(
                                message,
                                GooseControllerResponseMessage::Config(Box::new(
                                    self.configuration.clone(),
                                )),
                            );
                        }
                        // Send back a copy of the running metrics.
                        GooseControllerCommand::Metrics | GooseControllerCommand::MetricsJson => {
                            self.reply_to_controller(
                                message,
                                GooseControllerResponseMessage::Metrics(Box::new(
                                    self.metrics.clone(),
                                )),
                            );
                        }
                        // Start the load test, and acknowledge command.
                        GooseControllerCommand::Start => {
                            // We can only start an idle load test.
                            if self.attack_phase == AttackPhase::Idle {
                                self.set_attack_phase(
                                    goose_attack_run_state,
                                    AttackPhase::Starting,
                                );
                                self.reply_to_controller(
                                    message,
                                    GooseControllerResponseMessage::Bool(true),
                                );
                                // Reset the run state when starting a new load test.
                                self.reset_run_state(goose_attack_run_state).await?;
                            } else {
                                self.reply_to_controller(
                                    message,
                                    GooseControllerResponseMessage::Bool(false),
                                );
                            }
                        }
                        // Stop the load test, and acknowledge command.
                        GooseControllerCommand::Stop => {
                            // We can only stop a starting or running load test.
                            if [AttackPhase::Starting, AttackPhase::Running]
                                .contains(&self.attack_phase)
                            {
                                self.set_attack_phase(
                                    goose_attack_run_state,
                                    AttackPhase::Stopping,
                                );
                                // Don't shutdown when load test is stopped by controller, remain idle instead.
                                goose_attack_run_state.shutdown_after_stop = false;
                                // Don't automatically restart the load test.
                                self.configuration.no_autostart = true;
                                self.reply_to_controller(
                                    message,
                                    GooseControllerResponseMessage::Bool(true),
                                );
                            } else {
                                self.reply_to_controller(
                                    message,
                                    GooseControllerResponseMessage::Bool(false),
                                );
                            }
                        }
                        // Stop the load test, and acknowledge request.
                        GooseControllerCommand::Shutdown => {
                            // If load test is Idle, there are no metrics to display.
                            if self.attack_phase == AttackPhase::Idle {
                                self.metrics.display_metrics = false;
                            }
                            // Shutdown after stopping.
                            goose_attack_run_state.shutdown_after_stop = true;
                            // Properly stop any running GooseAttack first.
                            self.set_attack_phase(goose_attack_run_state, AttackPhase::Stopping);
                            // Confirm shut down to Controller.
                            self.reply_to_controller(
                                message,
                                GooseControllerResponseMessage::Bool(true),
                            );
                        }
                        GooseControllerCommand::Host => {
                            if self.attack_phase == AttackPhase::Idle {
                                // The controller uses a regular expression to validate that
                                // this is a valid hostname, so simply use it with further
                                // validation.
                                if let Some(host) = &message.request.value {
                                    info!(
                                        "changing host from {:?} to {}",
                                        self.configuration.host, host
                                    );
                                    self.configuration.host = host.to_string();
                                    self.weighted_users = self.weight_task_set_users()?;
                                    info!("load test ready");
                                    self.reply_to_controller(
                                        message,
                                        GooseControllerResponseMessage::Bool(true),
                                    );
                                } else {
                                    warn!(
                                        "Controller didn't provide host: {:#?}",
                                        &message.request
                                    );
                                }
                            } else {
                                self.reply_to_controller(
                                    message,
                                    GooseControllerResponseMessage::Bool(false),
                                );
                            }
                        }
                        GooseControllerCommand::Users => {
                            if self.attack_phase == AttackPhase::Idle {
                                // The controller uses a regular expression to validate that
                                // this is a valid integer, so simply use it with further
                                // validation.
                                if let Some(users) = &message.request.value {
                                    info!(
                                        "changing users from {:?} to {}",
                                        self.configuration.users, users
                                    );
                                    // Use expect() as Controller uses regex to validate this is an integer.
                                    self.configuration.users = Some(
                                        usize::from_str(&users)
                                            .expect("failed to convert string to usize"),
                                    );
                                    self.weighted_users = self.weight_task_set_users()?;
                                    info!("load test ready");
                                    self.reply_to_controller(
                                        message,
                                        GooseControllerResponseMessage::Bool(true),
                                    );
                                } else {
                                    warn!(
                                        "Controller didn't provide users: {:#?}",
                                        &message.request
                                    );
                                }
                            } else {
                                self.reply_to_controller(
                                    message,
                                    GooseControllerResponseMessage::Bool(false),
                                );
                            }
                        }
                        GooseControllerCommand::HatchRate => {
                            // The controller uses a regular expression to validate that
                            // this is a valid float, so simply use it with further
                            // validation.
                            if let Some(hatch_rate) = &message.request.value {
                                info!(
                                    "changing hatch_rate from {:?} to {}",
                                    self.configuration.hatch_rate, hatch_rate
                                );
                                self.configuration.hatch_rate = Some(hatch_rate.clone());
                                self.reply_to_controller(
                                    message,
                                    GooseControllerResponseMessage::Bool(true),
                                );
                            } else {
                                warn!(
                                    "Controller didn't provide hatch_rate: {:#?}",
                                    &message.request
                                );
                            }
                        }
                        GooseControllerCommand::RunTime => {
                            // The controller uses a regular expression to validate that
                            // this is a valid run time, so simply use it with further
                            // validation.
                            if let Some(run_time) = &message.request.value {
                                info!(
                                    "changing run_time from {:?} to {}",
                                    self.configuration.run_time, run_time
                                );
                                self.configuration.run_time = run_time.clone();
                                self.set_run_time()?;
                                self.reply_to_controller(
                                    message,
                                    GooseControllerResponseMessage::Bool(true),
                                );
                            } else {
                                warn!(
                                    "Controller didn't provide run_time: {:#?}",
                                    &message.request
                                );
                            }
                        }
                        // These messages shouldn't be received here.
                        GooseControllerCommand::Help | GooseControllerCommand::Exit => {
                            warn!("Unexpected command: {:?}", &message.request);
                        }
                    }
                }
                Err(e) => {
                    // Errors can be ignored, they happen any time there are no messages.
                    debug!("error receiving message: {}", e);
                }
            }
        };
        Ok(())
    }
}
