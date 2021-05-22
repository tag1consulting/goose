use crate::metrics::GooseMetrics;
use crate::GooseConfiguration;

use regex::{Regex, RegexSet};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use std::io;
use std::str;

#[derive(Debug)]
pub enum GooseControllerCommand {
    Config,
    HatchRate,
    Metrics,
    Stop,
    Users,
}

#[derive(Debug)]
pub struct GooseControllerCommandAndValue {
    pub command: GooseControllerCommand,
    pub value: String,
}

/// An enumeration of all messages that the controller can send to the parent thread.
#[derive(Debug)]
pub enum GooseControllerRequestMessage {
    Command(GooseControllerCommand),
    CommandAndValue(GooseControllerCommandAndValue),
}

/// An enumeration of all messages the parent can reply back to the controller thread.
#[derive(Debug)]
pub enum GooseControllerResponseMessage {
    Config(GooseConfiguration),
    Metrics(GooseMetrics),
}

/// The actual request that's passed from the controller to the parent thread.
#[derive(Debug)]
pub struct GooseControllerRequest {
    /// Optional one-shot channel if a reply is required.
    pub response_channel: Option<tokio::sync::oneshot::Sender<GooseControllerResponse>>,
    /// An integer identifying which controller client is making the request.
    pub client_id: u32,
    /// The actual reqeuest message.
    pub request: GooseControllerRequestMessage,
}

/// The actual request that's passed from the controller to the parent thread.
#[derive(Debug)]
pub struct GooseControllerResponse {
    pub client_id: u32,
    pub response: GooseControllerResponseMessage,
}

/// The control loop listens for connection on the configured TCP port. Each connection
/// spawns a new thread so multiple clients can connect.
/// @TODO: set configurable limit of how many control connections are allowed
/// @TODO: authentication
/// @TODO: ssl
pub async fn controller_main(
    // Expose load test configuration to controller thread.
    // @TODO: use this to configure the listening ip and port.
    configuration: GooseConfiguration,
    // For sending requests to the parent process.
    communication_channel_tx: flume::Sender<GooseControllerRequest>,
) -> io::Result<()> {
    // @TODO: make this configurable
    let addr = "127.0.0.1:5116";
    let listener = TcpListener::bind(&addr).await?;
    info!("controller listening on: {}", addr);

    // Simple incrementing counter each time a controller thread launches.
    let mut threads: u32 = 0;

    loop {
        // Asynchronously wait for an inbound socket.
        let (mut socket, _) = listener.accept().await?;

        // Clone the communication channel to hand to the next thread.
        let channel_tx = communication_channel_tx.clone();

        // Give each controller an initial copy of the Goose onfiguration.
        let _controller_thread_config = configuration.clone();

        // Increment counter each time a new thread launches, and pass id into thread.
        threads += 1;
        let controller_thread_id = threads;

        // Handle the client in a thread, allowing multiple clients to be processed
        // concurrently.
        tokio::spawn(async move {
            match socket.peer_addr() {
                Ok(p) => info!("client [{}] connected from {}", controller_thread_id, p),
                Err(e) => info!(
                    "client [{}] conected from UNKNOWN ADDRESS [{}]",
                    controller_thread_id, e
                ),
            };

            // Display initial goose> prompt.
            socket
                .write_all("goose> ".as_bytes())
                .await
                .expect("failed to write data to socket");

            // @TODO: What happens if a larger command is entered?
            let mut buf = [0; 1024];

            // The following regular expressions get compiled a second time if matched by the
            // RegexSet in order to capture the matched value.
            let users_regex = r"(?i)^users (\d+)$";
            let hatchrate_regex = r"(?i)^(hatchrate|hatch_rate) ([0-9]*(\.[0-9]*)?){1}$";
            let config_regex = r"(?i)^(config|config-json)$";
            let metrics_regex = r"(?i)^(metrics|stats|metrics-json|stats-json)$";

            // Compile regular expression set once to use for for matching all commands
            // received through the controller port.
            // @TODO: Figure out a clean way to map the location in the RegexSet here when
            // performing the matches.matched() tests below. The current implementation is
            // fragile to programmer mistakes if a command is inserted or moved.
            let commands = RegexSet::new(&[
                // Exit/quit the controller connection, does not affect load test.
                r"(?i)^exit|quit$",
                // Confirm the server is still connected and alive.
                r"(?i)^echo$",
                // Stop the load test (which will cause the controller connection to quit).
                r"(?i)^stop$",
                // Modify number of users simulated.
                users_regex,
                // Modify how quickly users hatch (or exit if users are reduced).
                hatchrate_regex,
                // Display the current load test configuration.
                config_regex,
                // Display running metrics for the currently active load test.
                metrics_regex,
            ])
            .unwrap();

            // Also compile the following regular expressions once to use for when
            // the RegexSet matches these commands, to then capture the matched value.
            let re_users = Regex::new(users_regex).unwrap();
            let re_hatchrate = Regex::new(hatchrate_regex).unwrap();
            let re_config = Regex::new(config_regex).unwrap();
            let re_metrics = Regex::new(metrics_regex).unwrap();

            // Process data received from the client in a loop.
            loop {
                let n = socket
                    .read(&mut buf)
                    .await
                    .expect("failed to read data from socket");

                if n == 0 {
                    return;
                }

                // @TODO: why doesn't trim() work?
                //let message = str::from_utf8(&buf).unwrap().trim();
                let message = match str::from_utf8(&buf) {
                    Ok(m) => {
                        let mut messages = m.lines();
                        // @TODO: don't crash when we fail to exctract a line
                        messages.next().expect("failed to extract a line")
                    }
                    Err(_) => continue,
                };

                let matches = commands.matches(message);
                if matches.matched(0) {
                    write_to_socket(&mut socket, "goodbye!").await;
                    match socket.peer_addr() {
                        Ok(p) => info!("client [{}] disconnected from {}", controller_thread_id, p),
                        Err(e) => info!(
                            "client [{}] disconnected from UNKNOWN ADDRESS [{}]",
                            controller_thread_id, e
                        ),
                    };
                    return;
                } else if matches.matched(1) {
                    write_to_socket(&mut socket, "echo").await;
                } else if matches.matched(2) {
                    send_to_parent(
                        controller_thread_id,
                        &channel_tx,
                        None,
                        GooseControllerCommand::Stop,
                        None,
                    )
                    .await;
                    write_to_socket(&mut socket, "stopping load test...").await;
                } else if matches.matched(3) {
                    // This requires a second lookup to capture the integer, as documented at:
                    // https://docs.rs/regex/1.5.4/regex/struct.RegexSet.html#limitations
                    let caps = re_users.captures(message).unwrap();
                    let users = caps.get(1).map_or("", |m| m.as_str());
                    send_to_parent(
                        controller_thread_id,
                        &channel_tx,
                        None,
                        GooseControllerCommand::Users,
                        Some(users.to_string()),
                    )
                    .await;
                    write_to_socket(&mut socket, &format!("reconfigured users: {}", users)).await;
                } else if matches.matched(4) {
                    // This requires a second lookup to capture the integer, as documented at:
                    // https://docs.rs/regex/1.5.4/regex/struct.RegexSet.html#limitations
                    let caps = re_hatchrate.captures(message).unwrap();
                    let hatch_rate = caps.get(2).map_or("", |m| m.as_str());
                    send_to_parent(
                        controller_thread_id,
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
                } else if matches.matched(5) {
                    let caps = re_config.captures(message).unwrap();
                    let config_format = caps.get(1).map_or("", |m| m.as_str());
                    // Get an up-to-date copy of the configuration, as it may have changed since
                    // the version that was initially passed in.
                    if let Ok(value) = send_to_parent_and_get_reply(
                        controller_thread_id,
                        &channel_tx,
                        GooseControllerCommand::Config,
                        None,
                    )
                    .await
                    {
                        match value {
                            GooseControllerResponseMessage::Config(config) => {
                                match config_format {
                                    "config" => {
                                        write_to_socket(&mut socket, &format!("{:#?}", config)).await;
                                    },
                                    "config-json" => {
                                        // Convert the configuration object to a JSON string.
                                        let config_json: String = serde_json::to_string(&config)
                                            .expect("unexpected failure");
                                        write_to_socket(&mut socket, &config_json).await;
                                    }
                                    _ => (),
                                }
                            },
                            _ => warn!("parent process sent an unexpected reply, unable to update configuration"),
                        }
                    }
                } else if matches.matched(6) {
                    let caps = re_metrics.captures(message).unwrap();
                    let metrics_format = caps.get(1).map_or("", |m| m.as_str());
                    // Get a copy of the current running metrics.
                    if let Ok(value) = send_to_parent_and_get_reply(
                        controller_thread_id,
                        &channel_tx,
                        GooseControllerCommand::Metrics,
                        None,
                    )
                    .await
                    {
                        match value {
                            GooseControllerResponseMessage::Metrics(metrics) => {
                                match metrics_format {
                                    "stats" | "metrics" => {
                                        write_to_socket(&mut socket, &format!("{}", metrics)).await;
                                    },
                                    "stats-json" | "metrics-json" => {
                                        // Convert the configuration object to a JSON string.
                                        let metrics_json: String = serde_json::to_string(&metrics)
                                            .expect("unexpected failure");
                                        write_to_socket(&mut socket, &metrics_json).await;
                                    },
                                    _ => (),
                                }
                            },
                            _ => warn!("parent process sent an unexpected reply, unable to display metrics"),
                        }
                    }
                } else {
                    write_to_socket(&mut socket, "unrecognized command").await;
                }
            }
        });
    }
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

    // Await resposne from parent.
    match response_rx.await {
        Ok(value) => Ok(value.response),
        Err(e) => Err(format!("one-shot channel dropped without reply: {}", e)),
    }
}
