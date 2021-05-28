//! Optionally launches telnet and WebSocket Controllers.
//!
//! By default, Goose launches both a telnet Controller and a WebSocket Controller, allowing
//! real-time control of the running load test.

use crate::metrics::GooseMetrics;
use crate::GooseConfiguration;

use futures::{SinkExt, StreamExt};
use regex::{Regex, RegexSet};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tungstenite::Message;

use std::io;
use std::str;

/// Goose currently supports two different Controller protocols: telnet and WebSocket.
#[derive(Debug)]
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
/// regex matched, if any.
#[derive(Debug)]
pub enum GooseControllerCommand {
    /// Change how quickly new [`GooseUser`](../goose/struct.GooseUser.html)s are launched.
    HatchRate,
    /// Display the current [`GooseConfiguration`](../struct.GooseConfiguration.html)s
    Config,
    /// Display the current [`GooseMetric`](../metrics/struct.GooseMetrics.html)s.
    Metrics,
    /// Displays a list of all supported commands.
    Help,
    /// Disconnect from the controller.
    Exit,
    /// Verify that the controller can talk to the parent process.
    Echo,
    /// Start an idle load test.
    Start,
    /// Stop a running test, putting it into an idle state.
    Stop,
    /// Tell the load test to shut down (which will disconnect the controller).
    Shutdown,
}

/// This structure is used to send commands and values to the parent process.
#[derive(Debug)]
pub struct GooseControllerCommandAndValue {
    /// The command that is being sent to the parent.
    pub command: GooseControllerCommand,
    /// The value that is being sent to the parent.
    pub value: String,
}

/// An enumeration of all messages that the controller can send to the parent thread.
#[derive(Debug)]
pub enum GooseControllerRequestMessage {
    /// A command alone.
    Command(GooseControllerCommand),
    /// A command and a value together.
    CommandAndValue(GooseControllerCommandAndValue),
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

/// The control loop listens for connection on the configured TCP port. Each connection
/// spawns a new thread so multiple clients can connect.
/// @TODO: set configurable limit of how many control connections are allowed
/// @TODO: authentication
/// @TODO: ssl
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
    let config_regex = r"(?i)^(config|config-json)$";
    let metrics_regex = r"(?i)^(metrics|stats|metrics-json|stats-json)$";
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
        config_regex,
        // Display running metrics for the currently active load test.
        metrics_regex,
        // Modify number of users simulated.
        // @TODO: enable when the parent process processes it properly.
        //users_regex,
        // Provide a list of possible commands.
        r"(?i)^(help|\?)$",
        // Exit/quit the controller connection, does not affect load test.
        r"(?i)^(exit|quit)$",
        // Confirm the server is still connected and alive.
        r"(?i)^echo$",
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
        Regex::new(hatchrate_regex).unwrap(),
        Regex::new(config_regex).unwrap(),
        Regex::new(metrics_regex).unwrap(),
    ];
    // @TODO: enable when the parent process processes it properly.
    //Regex::new(users_regex).unwrap();

    // Counter increments each time a controller client connects with this protocol.
    let mut thread_id: u32 = 0;

    // Wait for a connection.
    while let Ok((stream, _)) = listener.accept().await {
        thread_id += 1;
        // Spawn a new thread to communicate with client, igoring the returned JoinHandle.
        let _ = match &protocol {
            GooseControllerProtocol::Telnet => tokio::spawn(accept_telnet_connection(
                thread_id,
                channel_tx.clone(),
                stream,
                commands.clone(),
                captures.clone(),
            )),
            GooseControllerProtocol::WebSocket => tokio::spawn(accept_websocket_connection(
                thread_id,
                channel_tx.clone(),
                stream,
                commands.clone(),
                captures.clone(),
            )),
        };
    }

    Ok(())
}

// Respond to an incoming telnet connection.
// @TODO: add support for ssl and optional authentication.
// @TODO: limit the number of active connections to prevent DoS.
async fn accept_telnet_connection(
    thread_id: u32,
    channel_tx: flume::Sender<GooseControllerRequest>,
    mut socket: tokio::net::TcpStream,
    commands: RegexSet,
    captures: Vec<Regex>,
) {
    let peer_addr = socket
        .peer_addr()
        .map_or("UNKNOWN ADDRESS".to_string(), |p| p.to_string());
    info!("telnet client [{}] connected from {}", thread_id, peer_addr);

    // Display initial goose> prompt.
    write_to_socket_raw(&mut socket, "goose> ").await;

    // @TODO: reset connection if larger command is entered (to unfreeze connection).
    let mut buf = [0; 1024];

    // Process data received from the client in a loop.
    loop {
        let n = socket
            .read(&mut buf)
            .await
            .expect("failed to read data from socket");

        if n == 0 {
            return;
        }

        let command = match str::from_utf8(&buf) {
            Ok(m) => {
                if let Some(c) = m.lines().next() {
                    c
                } else {
                    ""
                }
            }
            Err(e) => {
                warn!("ignoring unexpected input from telnet controller: {}", e);
                continue;
            }
        };

        let matches = commands.matches(command);
        // Help
        if matches.matched(GooseControllerCommand::Help as usize) {
            write_to_socket(&mut socket, &display_help()).await;
        // Exit
        } else if matches.matched(GooseControllerCommand::Exit as usize) {
            write_to_socket(&mut socket, "goodbye!").await;
            info!(
                "telnet client [{}] disconnected from {}",
                thread_id, peer_addr
            );
            return;
        // Echo
        } else if matches.matched(GooseControllerCommand::Echo as usize) {
            match send_to_parent_and_get_reply(
                thread_id,
                &channel_tx,
                GooseControllerCommand::Echo,
                None,
            )
            .await
            {
                Ok(_) => write_to_socket(&mut socket, "echo").await,
                Err(e) => write_to_socket(&mut socket, &format!("echo failed: [{}]", e)).await,
            }
        // Start
        } else if matches.matched(GooseControllerCommand::Start as usize) {
            write_to_socket_raw(&mut socket, "starting load test ...").await;
            match send_to_parent_and_get_reply(
                thread_id,
                &channel_tx,
                GooseControllerCommand::Start,
                None,
            )
            .await
            {
                Ok(_) => write_to_socket(&mut socket, "").await,
                Err(e) => {
                    write_to_socket(&mut socket, &format!("failed to start load test [{}]", e))
                        .await
                }
            }
        // Shutdown
        } else if matches.matched(GooseControllerCommand::Shutdown as usize) {
            write_to_socket_raw(&mut socket, "shutting down load test ...\n").await;
            if let Err(e) = send_to_parent_and_get_reply(
                thread_id,
                &channel_tx,
                GooseControllerCommand::Shutdown,
                None,
            )
            .await
            {
                write_to_socket(
                    &mut socket,
                    &format!("failed to shut down load test [{}]", e),
                )
                .await;
            }
        // Hatch rate
        } else if matches.matched(GooseControllerCommand::HatchRate as usize) {
            // Perform a second map to capture the hatch_rate value.
            let caps = captures[GooseControllerCommand::HatchRate as usize]
                .captures(command)
                .unwrap();
            let hatch_rate = caps.get(2).map_or("", |m| m.as_str());
            send_to_parent(
                thread_id,
                &channel_tx,
                None,
                GooseControllerCommand::HatchRate,
                Some(hatch_rate.to_string()),
            )
            .await;
            write_to_socket(
                &mut socket,
                &format!("reconfigured hatch_rate: {}", hatch_rate),
            )
            .await;
        // Config
        } else if matches.matched(GooseControllerCommand::Config as usize) {
            // Perform a second map to capture the actual command matched.
            let caps = captures[GooseControllerCommand::Config as usize]
                .captures(command)
                .unwrap();
            let config_format = caps.get(1).map_or("", |m| m.as_str());
            // Get an up-to-date copy of the configuration, as it may have changed since
            // the version that was initially passed in.
            if let Ok(value) = send_to_parent_and_get_reply(
                thread_id,
                &channel_tx,
                GooseControllerCommand::Config,
                None,
            )
            .await
            {
                match value {
                    GooseControllerResponseMessage::Config(config) => {
                        match config_format {
                            // Display human-readable configuration.
                            "config" => {
                                write_to_socket(&mut socket, &format!("{:#?}", config)).await;
                            }
                            // Display json-formatted configuration.
                            "config-json" => {
                                // Convert the configuration object to a JSON string.
                                let config_json: String =
                                    serde_json::to_string(&config).expect("unexpected failure");
                                write_to_socket(&mut socket, &config_json).await;
                            }
                            _ => (),
                        }
                    }
                    _ => warn!(
                        "parent process sent an unexpected reply, unable to update configuration"
                    ),
                }
            }
        // Metrics
        } else if matches.matched(GooseControllerCommand::Metrics as usize) {
            // Perform a second map to capture the actual command matched.
            let caps = captures[GooseControllerCommand::Metrics as usize]
                .captures(command)
                .unwrap();
            let metrics_format = caps.get(1).map_or("", |m| m.as_str());
            // Get a copy of the current running metrics.
            if let Ok(value) = send_to_parent_and_get_reply(
                thread_id,
                &channel_tx,
                GooseControllerCommand::Metrics,
                None,
            )
            .await
            {
                match value {
                    GooseControllerResponseMessage::Metrics(metrics) => {
                        match metrics_format {
                            // Display human-readable metrics.
                            "stats" | "metrics" => {
                                write_to_socket(&mut socket, &format!("{}", metrics)).await;
                            }
                            // Display raw json-formatted metrics.
                            "stats-json" | "metrics-json" => {
                                // Convert the configuration object to a JSON string.
                                let metrics_json: String =
                                    serde_json::to_string(&metrics).expect("unexpected failure");
                                write_to_socket(&mut socket, &metrics_json).await;
                            }
                            _ => (),
                        }
                    }
                    _ => {
                        warn!("parent process sent an unexpected reply, unable to display metrics")
                    }
                }
            }
        } else {
            write_to_socket(&mut socket, "unrecognized command").await;
        }
    }
}

// Respond to an incoming websocket connection.
// @TODO: add support for ssl and optional authentication.
// @TODO: limit the number of active connections to prevent DoS.
async fn accept_websocket_connection(
    thread_id: u32,
    channel_tx: flume::Sender<GooseControllerRequest>,
    stream: tokio::net::TcpStream,
    commands: RegexSet,
    captures: Vec<Regex>,
) {
    let peer_addr = stream
        .peer_addr()
        .map_or("UNKNOWN ADDRESS".to_string(), |p| p.to_string());
    info!(
        "websocket client [{}] connected from {}",
        thread_id, peer_addr
    );

    let ws_stream = tokio_tungstenite::accept_async(stream)
        .await
        .expect("Error during the websocket handshake occurred");

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // Process data received from the client in a loop.
    loop {
        // Wait until the client sends a command.
        let data = ws_receiver
            .next()
            .await
            .expect("failed to read data from socket");

        if let Ok(request) = data {
            if request.is_text() {
                if let Ok(request) = request.into_text() {
                    debug!("websocket request: {:?}", request.trim());

                    let command: GooseControllerWebSocketRequest =
                        match serde_json::from_str(&request) {
                            Ok(c) => c,
                            Err(_) => {
                                ws_sender
                                    .send(Message::Text(
                                        serde_json::to_string(&GooseControllerWebSocketResponse {
                                            response: "unrecognized json request".to_string(),
                                            success: false,
                                            error: Some("unrecognized json request".to_string()),
                                        })
                                        .unwrap(),
                                    ))
                                    .await
                                    .expect("failed to write data to stream");
                                continue;
                            }
                        };
                    debug!("websocket request command: {:?}", command);

                    let matches = commands.matches(&command.request);
                    // Exit
                    if matches.matched(GooseControllerCommand::Exit as usize) {
                        ws_sender
                            .send(Message::Close(Some(tungstenite::protocol::CloseFrame {
                                code: tungstenite::protocol::frame::coding::CloseCode::Normal,
                                reason: std::borrow::Cow::Borrowed("exit"),
                            })))
                            .await
                            .expect("failed to write data to stream");
                        info!(
                            "websocket client [{}] disconnected from {}",
                            thread_id, peer_addr
                        );
                        return;
                    // Echo
                    } else if matches.matched(GooseControllerCommand::Echo as usize) {
                        match send_to_parent_and_get_reply(
                            thread_id,
                            &channel_tx,
                            GooseControllerCommand::Echo,
                            None,
                        )
                        .await
                        {
                            Ok(_) => {
                                ws_sender
                                    .send(Message::Text(
                                        serde_json::to_string(&GooseControllerWebSocketResponse {
                                            response: "echo".to_string(),
                                            success: true,
                                            error: None,
                                        })
                                        .unwrap(),
                                    ))
                                    .await
                                    .expect("failed to write data to stream");
                            }
                            Err(e) => {
                                ws_sender
                                    .send(Message::Text(
                                        serde_json::to_string(&GooseControllerWebSocketResponse {
                                            response: "echo failed".to_string(),
                                            success: false,
                                            error: Some(e),
                                        })
                                        .unwrap(),
                                    ))
                                    .await
                                    .expect("failed to write data to stream");
                            }
                        }
                    // Start
                    } else if matches.matched(GooseControllerCommand::Start as usize) {
                        match send_to_parent_and_get_reply(
                            thread_id,
                            &channel_tx,
                            GooseControllerCommand::Start,
                            None,
                        )
                        .await
                        {
                            Ok(_) => {
                                ws_sender
                                    .send(Message::Text(
                                        serde_json::to_string(&GooseControllerWebSocketResponse {
                                            response: "load test started".to_string(),
                                            success: true,
                                            error: None,
                                        })
                                        .unwrap(),
                                    ))
                                    .await
                                    .expect("failed to write data to stream");
                            }
                            Err(e) => {
                                ws_sender
                                    .send(Message::Text(
                                        serde_json::to_string(&GooseControllerWebSocketResponse {
                                            response: "starting load test failed".to_string(),
                                            success: false,
                                            error: Some(e),
                                        })
                                        .unwrap(),
                                    ))
                                    .await
                                    .expect("failed to write data to stream");
                            }
                        }
                    // Shutdown
                    } else if matches.matched(GooseControllerCommand::Shutdown as usize) {
                        match send_to_parent_and_get_reply(
                            thread_id,
                            &channel_tx,
                            GooseControllerCommand::Shutdown,
                            None,
                        )
                        .await
                        {
                            Ok(_) => {
                                ws_sender
                                    .send(Message::Close(Some(tungstenite::protocol::CloseFrame {
                                        code: tungstenite::protocol::frame::coding::CloseCode::Away,
                                        reason: std::borrow::Cow::Borrowed("stopping"),
                                    })))
                                    .await
                                    .expect("failed to write data to stream");
                            }
                            Err(e) => {
                                ws_sender
                                    .send(Message::Text(
                                        serde_json::to_string(&GooseControllerWebSocketResponse {
                                            response: "failed to shut down load test".to_string(),
                                            success: false,
                                            error: Some(e),
                                        })
                                        .unwrap(),
                                    ))
                                    .await
                                    .expect("failed to write data to stream");
                            }
                        }
                    // Hatch rate
                    } else if matches.matched(GooseControllerCommand::HatchRate as usize) {
                        // Perform a second map to capture the hatch_rate value.
                        let caps = captures[GooseControllerCommand::HatchRate as usize]
                            .captures(&command.request)
                            .unwrap();
                        let hatch_rate = caps.get(2).map_or("", |m| m.as_str());
                        info!("matched hatch_rate: {}", hatch_rate);
                        send_to_parent(
                            thread_id,
                            &channel_tx,
                            None,
                            GooseControllerCommand::HatchRate,
                            Some(hatch_rate.to_string()),
                        )
                        .await;
                        ws_sender
                            .send(Message::Text(
                                serde_json::to_string(&GooseControllerWebSocketResponse {
                                    response: "set hatch_rate".to_string(),
                                    success: true,
                                    error: None,
                                })
                                .unwrap(),
                            ))
                            .await
                            .expect("failed to write data to stream");
                    // Config
                    } else if matches.matched(GooseControllerCommand::Config as usize) {
                        // Get an up-to-date copy of the configuration, as it may have changed since
                        // the version that was initially passed in.
                        if let Ok(GooseControllerResponseMessage::Config(config)) =
                            send_to_parent_and_get_reply(
                                thread_id,
                                &channel_tx,
                                GooseControllerCommand::Config,
                                None,
                            )
                            .await
                        {
                            // Convert the configuration object to a JSON string.
                            let config_json: String =
                                serde_json::to_string(&config).expect("unexpected failure");
                            ws_sender
                                .send(Message::Text(
                                    serde_json::to_string(&GooseControllerWebSocketResponse {
                                        response: config_json,
                                        success: true,
                                        error: None,
                                    })
                                    .unwrap(),
                                ))
                                .await
                                .expect("failed to write data to stream");
                        }
                    // Metrics
                    } else if matches.matched(GooseControllerCommand::Metrics as usize) {
                        // Get a copy of the current running metrics.
                        if let Ok(GooseControllerResponseMessage::Metrics(metrics)) =
                            send_to_parent_and_get_reply(
                                //if let GooseControllerResponseMessage::Metrics(metrics) = value {
                                thread_id,
                                &channel_tx,
                                GooseControllerCommand::Metrics,
                                None,
                            )
                            .await
                        {
                            // Convert the configuration object to a JSON string.
                            let metrics_json: String =
                                serde_json::to_string(&metrics).expect("unexpected failure");
                            ws_sender
                                .send(Message::Text(
                                    serde_json::to_string(&GooseControllerWebSocketResponse {
                                        response: metrics_json,
                                        success: true,
                                        error: None,
                                    })
                                    .unwrap(),
                                ))
                                .await
                                .expect("failed to write data to stream");
                        }
                    // Unknown command
                    } else {
                        ws_sender
                            .send(Message::Text(
                                serde_json::to_string(&GooseControllerWebSocketResponse {
                                    response: "unrecognized command".to_string(),
                                    success: false,
                                    error: Some("unrecognized command".to_string()),
                                })
                                .unwrap(),
                            ))
                            .await
                            .expect("failed to write data to stream");
                    }
                }
            } else if request.is_close() {
                info!(
                    "telnet client [{}] disconnected from {}",
                    thread_id, peer_addr
                );
                break;
            }
        }
    }
}

/// Send a message to the client TcpStream, no prompt or line feed.
async fn write_to_socket_raw(socket: &mut tokio::net::TcpStream, message: &str) {
    socket
        // Add a linefeed to the end of the message.
        .write_all(message.as_bytes())
        .await
        .expect("failed to write data to socket");
}

/// Send a message to the client TcpStream.
async fn write_to_socket(socket: &mut tokio::net::TcpStream, message: &str) {
    socket
        // Add a linefeed to the end of the message.
        .write_all([message, "\ngoose> "].concat().as_bytes())
        .await
        .expect("failed to write data to socket");
}

/// Send a message to parent thread, with or without an optional value.
async fn send_to_parent(
    client_id: u32,
    channel: &flume::Sender<GooseControllerRequest>,
    response_channel: Option<tokio::sync::oneshot::Sender<GooseControllerResponse>>,
    command: GooseControllerCommand,
    optional_value: Option<String>,
) {
    if let Some(value) = optional_value {
        // @TODO: handle a possible error when sending.
        let _ = channel.try_send(GooseControllerRequest {
            response_channel,
            client_id,
            request: GooseControllerRequestMessage::CommandAndValue(
                GooseControllerCommandAndValue { command, value },
            ),
        });
    } else {
        // @TODO: handle a possible error when sending.
        let _ = channel.try_send(GooseControllerRequest {
            response_channel,
            client_id,
            request: GooseControllerRequestMessage::Command(command),
        });
    }
}

/// Send a message to parent thread, with or without an optional value, and wait for
/// a reply.
async fn send_to_parent_and_get_reply(
    client_id: u32,
    channel_tx: &flume::Sender<GooseControllerRequest>,
    command: GooseControllerCommand,
    value: Option<String>,
) -> Result<GooseControllerResponseMessage, String> {
    // Create a one-shot channel to allow the parent to reply to our request. As flume
    // doesn't implement a one-shot channel, we use tokio for this temporary channel.
    let (response_tx, response_rx): (
        tokio::sync::oneshot::Sender<GooseControllerResponse>,
        tokio::sync::oneshot::Receiver<GooseControllerResponse>,
    ) = tokio::sync::oneshot::channel();

    // Send request to parent.
    send_to_parent(client_id, channel_tx, Some(response_tx), command, value).await;

    // Await response from parent.
    match response_rx.await {
        Ok(value) => Ok(value.response),
        Err(e) => Err(format!("one-shot channel dropped without reply: {}", e)),
    }
}

// A controller help screen.
// @TODO: document `users` when enabled:
// users INT          set number of simulated users
fn display_help() -> String {
    format!(
        r"{} {} controller commands:
 help (?)           this help
 exit (quit)        exit controller
 echo               confirm controller is working
 start              start an idle loat test
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
