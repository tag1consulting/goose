//! A response dropped with its body unread closes its connection, so the user's next
//! request opens a new one; `GooseResponse::discard_body` reads the body to the end and
//! the user keeps reusing its connections.
//!
//! A local HTTP/1.1 server counts the connections it accepts and answers every request
//! with the same body, large enough that it cannot arrive whole with the headers.

use serial_test::serial;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

mod common;

use goose::config::GooseConfiguration;
use goose::prelude::*;

/// The body every response carries: 256 KiB.
const BODY_BYTES: usize = 256 * 1024;

/// How many requests one user makes.
const REQUESTS: usize = 10;

/// Starts a keep-alive HTTP/1.1 server on a free local port. Returns its base URL and
/// the number of connections it has accepted.
async fn start_server() -> (String, Arc<AtomicUsize>) {
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
                        if socket.write_all(head.as_bytes()).await.is_err()
                            || socket.write_all(&body).await.is_err()
                        {
                            return;
                        }
                    }
                }
            });
        }
    });
    (format!("http://{address}"), connections)
}

/// Drops the response with its body unread.
async fn drop_body(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/body").await?;
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

/// One user runs `transaction` REQUESTS times against the server; returns the
/// connections the server accepted and the requests Goose counted.
async fn run(transaction: Transaction) -> (usize, usize) {
    let (host, connections) = start_server().await;
    let requests = REQUESTS.to_string();
    let configuration = GooseConfiguration::parse_args_default(&[
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
    ])
    .unwrap();
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
    (connections.load(Ordering::SeqCst), counted)
}

/// The most connections one user may open while reusing them. hyper returns a
/// connection whose body was read to the end to the pool from a task of its own, so
/// the user's next request can find the pool empty and open a second connection;
/// after that the two are reused. A body dropped unread costs a connection on every
/// request instead.
const MOST_REUSED_CONNECTIONS: usize = 2;

#[tokio::test]
#[serial]
/// A response dropped with its body unread costs its connection.
async fn dropped_bodies_open_new_connections() {
    let (connections, counted) = run(transaction!(drop_body)).await;
    assert_eq!(counted, REQUESTS);
    assert!(
        connections > MOST_REUSED_CONNECTIONS,
        "{} requests with dropped bodies opened {} connection(s)",
        REQUESTS,
        connections
    );
}

#[tokio::test]
#[serial]
/// Bodies read to the end with `discard_body` keep reusing the user's connections.
async fn discarded_bodies_reuse_connections() {
    DISCARDED_BYTES.store(0, Ordering::SeqCst);
    let (connections, counted) = run(transaction!(discard_body)).await;
    assert_eq!(counted, REQUESTS);
    assert_eq!(
        DISCARDED_BYTES.load(Ordering::SeqCst),
        (REQUESTS * BODY_BYTES) as u64,
        "discard_body did not read every body to the end"
    );
    assert!(
        connections <= MOST_REUSED_CONNECTIONS,
        "{} requests with discarded bodies opened {} connections",
        REQUESTS,
        connections
    );
}
