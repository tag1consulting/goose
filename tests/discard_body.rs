//! A response dropped with its body unread would close its connection, so the user's
//! next request would open a new one. Goose reads such a body to the end itself, on the
//! user's own task, before the user's next request and before the transaction ends, so
//! the user keeps reusing its connections, as it does when the test reads the body with
//! `GooseResponse::discard_body`. `--drop-unread-bodies` and `into_inner()` opt out.
//!
//! A local HTTP/1.1 server counts the connections it accepts and answers every request
//! with the same body, large enough that it cannot arrive whole with the headers.

use serial_test::serial;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

mod common;

use goose::config::GooseConfiguration;
use goose::metrics::GooseMetrics;
use goose::prelude::*;

/// The body every response carries: 256 KiB.
const BODY_BYTES: usize = 256 * 1024;

/// How many requests one user makes.
const REQUESTS: usize = 10;

/// How many parts the slow server sends a body in, with [`PART_PAUSE`] before each
/// part but the first.
const PARTS: usize = 8;

/// The pause before each part of the body but the first, on the slow server.
const PART_PAUSE: Duration = Duration::from_millis(25);

/// Starts a keep-alive HTTP/1.1 server on a free local port. Returns its base URL and
/// the number of connections it has accepted. A `slow` server sends each body in
/// [`PARTS`] parts, pausing [`PART_PAUSE`] before each but the first, so the body ends
/// at least `(PARTS - 1) * PART_PAUSE` after the headers.
async fn start_server(slow: bool) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let accepted = connections.clone();
    tokio::spawn(async move {
        let body = vec![b'x'; BODY_BYTES];
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {BODY_BYTES}\r\n\r\n"
        );
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            accepted.fetch_add(1, Ordering::SeqCst);
            let body = body.clone();
            let head = head.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    // Every request is a GET with no body: answer each once its head ends.
                    let read = match socket.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => read,
                    };
                    request.extend_from_slice(&buffer[..read]);
                    while let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        request.drain(..end + 4);
                        if socket.write_all(head.as_bytes()).await.is_err() {
                            return;
                        }
                        let parts = if slow { PARTS } else { 1 };
                        for (part, bytes) in body.chunks(BODY_BYTES / parts).enumerate() {
                            if part > 0 {
                                tokio::time::sleep(PART_PAUSE).await;
                            }
                            if socket.write_all(bytes).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            });
        }
    });
    (format!("http://{address}"), connections)
}

/// Drops the response with its body unread: the idiom of Goose 0.19 and earlier.
async fn drop_body(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/body").await?;
    Ok(())
}

/// Moves the response out of the `GooseResponse`, then drops it with its body unread.
async fn move_out_and_drop(user: &mut GooseUser) -> TransactionResult {
    let response = user.get("/body").await?.response?;
    drop(response);
    Ok(())
}

/// Takes the `reqwest::Response` with `into_inner()`, the per response opt out, then
/// drops it with its body unread.
async fn into_inner_and_drop(user: &mut GooseUser) -> TransactionResult {
    let response = user.get("/body").await?.response?.into_inner();
    drop(response);
    Ok(())
}

/// Body bytes `discard_body` read, summed over every request, checked by the test
/// itself: a panic or assert inside a user's transaction would not fail the test.
static DISCARDED_BYTES: AtomicU64 = AtomicU64::new(0);

/// Reads the response body to the end with `discard_body`.
async fn discard_body(user: &mut GooseUser) -> TransactionResult {
    let read = user.get("/body").await?.discard_body().await?;
    DISCARDED_BYTES.fetch_add(read, Ordering::SeqCst);
    Ok(())
}

/// One user runs `transaction` REQUESTS times against a server, `slow` or not, with
/// `extra` options; returns the connections the server accepted, the requests Goose
/// counted, and the metrics.
async fn run(transaction: Transaction, slow: bool, extra: &[&str]) -> (usize, usize, GooseMetrics) {
    let (host, connections) = start_server(slow).await;
    let requests = REQUESTS.to_string();
    let mut args = vec![
        "--host",
        host.as_str(),
        "--users",
        "1",
        "--increase-rate",
        "1",
        "--iterations",
        requests.as_str(),
        "--co-mitigation",
        "disabled",
        "--quiet",
    ];
    args.extend_from_slice(extra);
    let configuration = GooseConfiguration::parse_args_default(&args).unwrap();
    let scenario = scenario!("Body").register_transaction(transaction);
    let metrics = common::build_load_test(configuration, vec![scenario], None, None)
        .execute()
        .await
        .unwrap();
    let counted = metrics
        .requests
        .values()
        .map(|request| request.success_count + request.fail_count)
        .sum();
    (connections.load(Ordering::SeqCst), counted, metrics)
}

/// The most connections one user may open while reusing them. hyper returns a
/// connection whose body was read to the end to the pool from a task of its own, so
/// the user's next request can find the pool empty and open a second connection;
/// after that the two are reused. A body dropped unread costs a connection on every
/// request instead.
const MOST_REUSED_CONNECTIONS: usize = 2;

/// Checks that REQUESTS requests reused the user's connections.
fn assert_reused(what: &str, connections: usize, counted: usize) {
    assert_eq!(counted, REQUESTS);
    assert!(
        connections <= MOST_REUSED_CONNECTIONS,
        "{} requests with {} opened {} connections",
        REQUESTS,
        what,
        connections
    );
}

/// Checks that REQUESTS requests did not reuse the user's connections.
fn assert_not_reused(what: &str, connections: usize, counted: usize) {
    assert_eq!(counted, REQUESTS);
    assert!(
        connections > MOST_REUSED_CONNECTIONS,
        "{} requests with {} opened {} connection(s)",
        REQUESTS,
        what,
        connections
    );
}

#[tokio::test]
#[serial]
/// The 0.19 idiom, `let _goose = user.get(...)`: Goose reads the dropped body, and the
/// user keeps reusing its connections.
async fn dropped_bodies_reuse_connections() {
    let (connections, counted, _) = run(transaction!(drop_body), false, &[]).await;
    assert_reused("dropped bodies", connections, counted);
}

#[tokio::test]
#[serial]
/// A response moved out of its `GooseResponse` and dropped unread is read by Goose too.
async fn moved_out_bodies_reuse_connections() {
    let (connections, counted, _) = run(transaction!(move_out_and_drop), false, &[]).await;
    assert_reused("moved out and dropped bodies", connections, counted);
}

#[tokio::test]
#[serial]
/// With `--drop-unread-bodies`, a response dropped with its body unread costs its
/// connection, as in Goose 0.19.
async fn dropped_bodies_open_new_connections_when_told() {
    let (connections, counted, _) =
        run(transaction!(drop_body), false, &["--drop-unread-bodies"]).await;
    assert_not_reused(
        "dropped bodies and --drop-unread-bodies",
        connections,
        counted,
    );
}

#[tokio::test]
#[serial]
/// A body taken with `into_inner()` is the caller's: dropped unread, it costs its
/// connection.
async fn into_inner_bodies_open_new_connections() {
    let (connections, counted, _) = run(transaction!(into_inner_and_drop), false, &[]).await;
    assert_not_reused(
        "bodies taken with into_inner() and dropped",
        connections,
        counted,
    );
}

#[tokio::test]
#[serial]
/// Goose reads a dropped body inside the transaction that dropped it: each transaction
/// lasts at least as long as the server takes to send the body, while each request's
/// recorded time, its time to first byte, ends before the body does.
async fn dropped_bodies_are_read_before_the_transaction_ends() {
    let (_, counted, metrics) = run(transaction!(drop_body), true, &[]).await;
    assert_eq!(counted, REQUESTS);
    // The server sends the last part (PARTS - 1) pauses after the headers.
    let body_us = (PART_PAUSE * (PARTS as u32 - 1)).as_micros() as u64;
    let transaction = &metrics.transactions[0][0];
    assert_eq!(transaction.success_count, REQUESTS);
    assert!(
        transaction.run_times.minimum_time_us >= body_us,
        "the shortest transaction took {} us, less than the {} us the body takes to send",
        transaction.run_times.minimum_time_us,
        body_us
    );
    let request = metrics.requests.values().next().unwrap();
    assert!(
        request.raw_data.maximum_time_us < body_us,
        "the longest request took {} us, not less than the {} us the body takes to send",
        request.raw_data.maximum_time_us,
        body_us
    );
}

#[tokio::test]
#[serial]
/// Bodies read to the end with `discard_body` keep reusing the user's connections.
async fn discarded_bodies_reuse_connections() {
    DISCARDED_BYTES.store(0, Ordering::SeqCst);
    let (connections, counted, _) = run(transaction!(discard_body), false, &[]).await;
    assert_eq!(
        DISCARDED_BYTES.load(Ordering::SeqCst),
        (REQUESTS * BODY_BYTES) as u64,
        "discard_body did not read every body to the end"
    );
    assert_reused("discarded bodies", connections, counted);
}
