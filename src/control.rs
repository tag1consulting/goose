use crate::GooseConfiguration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use std::io;
use std::str;

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
    communication_channel: flume::Sender<bool>,
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

                match message.to_lowercase().as_str() {
                    // Allow the client to exit/quit the connection.
                    "exit" | "quit" => {
                        write_to_socket(&mut socket, "goodbye!\n").await;
                        match socket.peer_addr() {
                            Ok(p) => info!("client disconnected from {}", p),
                            Err(e) => info!("client disconnected from UNKNOWN ADDRESS [{}]", e),
                        };
                        return;
                    }
                    // Allows client to confirm the server is still connected and alive.
                    "echo" => {
                        write_to_socket(&mut socket, "echo\n").await;
                    }
                    // Stop the load test.
                    "stop" => {
                        write_to_socket(&mut socket, "stopping load test\n").await;
                        // @TODO: handle a possible error sending.
                        let _ = channel.try_send(true);
                    }
                    // Unrecognized command.
                    _ => {
                        write_to_socket(&mut socket, "unrecognized command\n").await;
                    }
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
