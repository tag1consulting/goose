//! Example showing how to load test a raw TCP server with Goose.
//!
//! Goose is primarily an HTTP load testing framework, but its metrics system can
//! track any protocol. This example demonstrates using [`GooseUser::record_custom_request`]
//! to record timing and success/failure information for TCP operations.
//!
//! # How it works
//!
//! 1. The target host and port are read from `--host` (e.g. `http://localhost:9000`).
//! 2. Each simulated user opens a TCP connection, sends a payload, and reads the echo.
//! 3. Timing is measured manually with [`std::time::Instant`].
//! 4. [`GooseUser::record_custom_request`] records the result — it will appear
//!    in the Goose metrics output under the method label you provide (e.g. `TCP`).
//!
//! # Running the example
//!
//! Start a local TCP echo server (e.g. `ncat -l 9000 -k -e /bin/cat`), then:
//!
//! ```text
//! cargo run --example tcp_loadtest -- --host http://localhost:9000 --users 10 --run-time 30s --no-reset-metrics
//! ```
//!
//! The `--host` flag controls which server is targeted. The scheme (`http://`) is
//! required by Goose for URL validation; the host and port are extracted and used
//! directly for the TCP connection.
//!
//! ## License
//!
//! Copyright 2020-2026 Jeremy Andrews
//!
//! Licensed under the Apache License, Version 2.0 (the "License");
//! you may not use this file except in compliance with the License.
//! You may obtain a copy of the License at
//!
//! <http://www.apache.org/licenses/LICENSE-2.0>
//!
//! Unless required by applicable law or agreed to in writing, software
//! distributed under the License is distributed on an "AS IS" BASIS,
//! WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//! See the License for the specific language governing permissions and
//! limitations under the License.

use goose::prelude::*;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_scenario(
            scenario!("TcpUser")
                .set_wait_time(Duration::from_millis(100), Duration::from_millis(500))?
                .register_transaction(transaction!(tcp_echo).set_name("tcp_echo")),
        )
        .execute()
        .await?;

    Ok(())
}

/// Connects to the TCP server specified by `--host`, sends a payload, reads the
/// echo response, and records the round-trip as a Goose `TCP` metric.
///
/// # Persistent connections
///
/// This example opens a new TCP connection for each transaction. For production
/// load tests you may want to keep connections persistent across transactions
/// using Goose session data. Because `TcpStream` is not `Clone`, wrap it in
/// `Arc<tokio::sync::Mutex<TcpStream>>`:
///
/// ```ignore
/// // In an on_start transaction:
/// user.set_session_data(Arc::new(tokio::sync::Mutex::new(stream)));
///
/// // In subsequent transactions:
/// let stream = user.get_session_data_unchecked::<Arc<tokio::sync::Mutex<TcpStream>>>();
/// let mut guard = stream.lock().await;
/// ```
///
/// See `examples/session.rs` for the full session data pattern.
async fn tcp_echo(user: &mut GooseUser) -> TransactionResult {
    let host = user.base_url.host_str().unwrap_or("127.0.0.1");
    let port = user.base_url.port().ok_or_else(|| {
        Box::new(TransactionError::Custom(
            "--host must include a port number (e.g. --host http://127.0.0.1:9000)".to_string(),
        ))
    })?;
    let addr = format!("{}:{}", host, port);

    let payload = b"Hello, TCP!\n";

    let started = Instant::now();

    let result = async {
        let mut stream = TcpStream::connect(&addr).await?;
        stream.write_all(payload).await?;

        // Read echoed response (same length as what we sent).
        let mut buf = vec![0u8; payload.len()];
        stream.read_exact(&mut buf).await?;

        Ok::<_, std::io::Error>(buf)
    }
    .await;

    let response_time = started.elapsed().as_millis() as u64;

    match result {
        Ok(_response) => {
            user.record_custom_request("TCP", "tcp_echo", response_time, true, 0, None)
                .await?;
        }
        Err(e) => {
            let error_msg = e.to_string();
            user.record_custom_request(
                "TCP",
                "tcp_echo",
                response_time,
                false,
                0,
                Some(&error_msg),
            )
            .await?;
        }
    }

    Ok(())
}
