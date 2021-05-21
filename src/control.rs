use crate::GooseConfiguration;

use regex::{Regex, RegexSet};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use std::io;
use std::str;

#[derive(Debug)]
pub enum GooseControllerCommand {
    Stop,
    Users,
    HatchRate,
}

#[derive(Debug)]
pub struct GooseControllerCommandAndValue {
    pub command: GooseControllerCommand,
    pub value: String,
}

/// An enumeration of all messages that can be exchanged between the Goose parent process and
/// the controller thread.
#[derive(Debug)]
pub enum GooseControl {
    GooseControllerCommand(GooseControllerCommand),
    GooseControllerCommandAndValue(GooseControllerCommandAndValue),
}

/// The control loop listens for connection on the configured TCP port. Each connection
/// spawns a new thread so multiple clients can connect.
/// @TODO: set configurable limit of how many control connections are allowed
/// @TODO: authentication
/// @TODO: ssl
pub async fn controller_main(
    // Expose load test configuration to controller thread.
    // @TODO: use this to configure the listening ip and port.
    _configuration: GooseConfiguration,
    // A communication channel with the parent.
    // @TODO: pass a useful enum.
    communication_channel: flume::Sender<GooseControl>,
) -> io::Result<()> {
    // @TODO: make this configurable
    let addr = "127.0.0.1:5116";
    let listener = TcpListener::bind(&addr).await?;
    info!("controller listening on: {}", addr);

    loop {
        // Asynchronously wait for an inbound socket.
        let (mut socket, _) = listener.accept().await?;

        // Make a clone of the communication channel to hand to the next thread.
        let channel = communication_channel.clone();

        // Handle the client in a thread, allowing multiple clients to be processed
        // concurrently.
        tokio::spawn(async move {
            match socket.peer_addr() {
                Ok(p) => info!("client connected from {}", p),
                Err(e) => info!("client connected from UNKNOWN ADDRESS [{}]", e),
            };

            // @TODO: What happens if a larger command is entered?
            let mut buf = [0; 1024];

            // The following regular expressions get compiled a second time if matched by the
            // RegexSet in order to capture the matched value.
            let users_regex = r"(?i)^users (\d+)$";
            let hatchrate_regex = r"(?i)^hatchrate ([0-9]*(\.[0-9]*)?){1}$";

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
            ])
            .unwrap();

            // Also compile the following regular expressions once to use for when
            // the RegexSet matches these commands, to then capture the matched value.
            let re_users = Regex::new(users_regex).unwrap();
            let re_hatchrate = Regex::new(hatchrate_regex).unwrap();

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
                    write_to_socket(&mut socket, "goodbye!\n").await;
                    match socket.peer_addr() {
                        Ok(p) => info!("client disconnected from {}", p),
                        Err(e) => info!("client disconnected from UNKNOWN ADDRESS [{}]", e),
                    };
                    return;
                } else if matches.matched(1) {
                    write_to_socket(&mut socket, "echo\n").await;
                } else if matches.matched(2) {
                    write_to_socket(&mut socket, "stopping load test\n").await;
                    // @TODO: handle a possible error when sending.
                    let _ = channel.try_send(GooseControl::GooseControllerCommand(
                        GooseControllerCommand::Stop,
                    ));
                } else if matches.matched(3) {
                    // This requires a second lookup to capture the integer, as documented at:
                    // https://docs.rs/regex/1.5.4/regex/struct.RegexSet.html#limitations
                    let caps = re_users.captures(message).unwrap();
                    let users = caps.get(1).map_or("", |m| m.as_str());
                    let _ = channel.try_send(GooseControl::GooseControllerCommandAndValue(
                        GooseControllerCommandAndValue {
                            command: GooseControllerCommand::Users,
                            value: users.to_string(),
                        },
                    ));
                    write_to_socket(&mut socket, &format!("reconfigured users: {}\n", users)).await;
                } else if matches.matched(4) {
                    // This requires a second lookup to capture the integer, as documented at:
                    // https://docs.rs/regex/1.5.4/regex/struct.RegexSet.html#limitations
                    let caps = re_hatchrate.captures(message).unwrap();
                    let hatch_rate = caps.get(1).map_or("", |m| m.as_str());
                    let _ = channel.try_send(GooseControl::GooseControllerCommandAndValue(
                        GooseControllerCommandAndValue {
                            command: GooseControllerCommand::HatchRate,
                            value: hatch_rate.to_string(),
                        },
                    ));
                    write_to_socket(
                        &mut socket,
                        &format!("reconfigured hatch_rate: {}\n", hatch_rate),
                    )
                    .await;
                } else {
                    write_to_socket(&mut socket, "unrecognized command\n").await;
                }
            }
        });
    }
}

/// Simple helper to send a message to a client TcpStream.
async fn write_to_socket(socket: &mut tokio::net::TcpStream, message: &str) {
    socket
        .write_all(message.as_bytes())
        .await
        .expect("failed to write data to socket");
}
