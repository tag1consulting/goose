//! A server can close an idle keep-alive connection just as the next request goes out
//! on it. Goose sends an idempotent request that failed that way once more, on a new
//! connection, as a browser does; it never sends a POST again, as it can't see whether
//! any of the response arrived before the close.
//!
//! A local HTTP/1.1 server on raw TCP closes each connection exactly where a test needs.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

mod common;

use goose::config::GooseConfiguration;
use goose::metrics::GooseMetrics;
use goose::prelude::*;

/// What the server does with the requests on each connection.
#[derive(Clone, Copy)]
enum Close {
    /// Answers the first request on the first connection with keep-alive, then reads
    /// the next request's head and closes without a byte. Answers every request on
    /// every later connection.
    SecondRequestOnFirstConnection,
    /// Reads each request's head and closes without a byte.
    Always,
    /// Reads each request's head, sends a status line and one header, and closes.
    AfterPartialResponse,
}

/// What the server saw.
#[derive(Default)]
struct Seen {
    connections: AtomicUsize,
    /// The request line of every request head read, in order.
    requests: Mutex<Vec<String>>,
}

impl Seen {
    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

const RESPONSE: &[u8] =
    b"HTTP/1.1 200 OK\r\nConnection: keep-alive\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\nok";

/// Reads one request head, and the body its Content-Length gives. Returns its request
/// line, or None when the connection closes first.
async fn read_request(socket: &mut TcpStream, buffer: &mut Vec<u8>) -> Option<String> {
    let mut chunk = [0u8; 4096];
    let end = loop {
        if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break end + 4;
        }
        match socket.read(&mut chunk).await {
            Ok(0) | Err(_) => return None,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
    };
    let head = String::from_utf8_lossy(&buffer[..end]).to_string();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);
    while buffer.len() < end + length {
        match socket.read(&mut chunk).await {
            Ok(0) | Err(_) => return None,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
    }
    let body = String::from_utf8_lossy(&buffer[end..end + length]).to_string();
    buffer.drain(..end + length);
    let line = head.lines().next().unwrap_or_default().to_string();
    if body.is_empty() {
        Some(line)
    } else {
        Some(format!("{line} {body}"))
    }
}

/// Starts the server on a free local port. Returns its base URL and what it saw.
async fn start_server(close: Close) -> (String, Arc<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let seen = Arc::new(Seen::default());
    let server_seen = seen.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let connection = server_seen.connections.fetch_add(1, Ordering::SeqCst);
            let seen = server_seen.clone();
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let mut served = 0;
                while let Some(request) = read_request(&mut socket, &mut buffer).await {
                    seen.requests.lock().unwrap().push(request);
                    match close {
                        Close::SecondRequestOnFirstConnection if connection == 0 && served > 0 => {
                            return
                        }
                        Close::SecondRequestOnFirstConnection => {}
                        Close::Always => return,
                        Close::AfterPartialResponse => {
                            let _ = socket
                                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n")
                                .await;
                            return;
                        }
                    }
                    if socket.write_all(RESPONSE).await.is_err() {
                        return;
                    }
                    served += 1;
                }
            });
        }
    });
    (format!("http://{address}"), seen)
}

/// Runs one user through `transaction` once against the server.
async fn run(host: &str, transaction: Transaction) -> GooseMetrics {
    let configuration = GooseConfiguration::parse_args_default(&[
        "--host",
        host,
        "--users",
        "1",
        "--increase-rate",
        "1",
        "--iterations",
        "1",
        "--co-mitigation",
        "disabled",
        "--quiet",
    ])
    .unwrap();
    let scenario = scenario!("KeepAlive").register_transaction(transaction);
    common::build_load_test(configuration, vec![scenario], None, None)
        .execute()
        .await
        .unwrap()
}

/// The successes and failures Goose recorded for `key`, such as "GET /".
fn recorded(metrics: &GooseMetrics, key: &str) -> (usize, usize) {
    metrics
        .requests
        .get(key)
        .map(|request| (request.success_count, request.fail_count))
        .unwrap_or((0, 0))
}

/// Waits for hyper to return the first connection to the user's pool: it does so from
/// a task of its own, and the next request must find the connection there to reuse it.
async fn let_connection_return_to_pool() {
    tokio::time::sleep(Duration::from_millis(200)).await;
}

/// Two GETs, the second on the connection the first used.
async fn get_twice(user: &mut GooseUser) -> TransactionResult {
    user.get("/first").await?.discard_body().await?;
    let_connection_return_to_pool().await;
    user.get("/second").await?.discard_body().await?;
    Ok(())
}

/// One GET.
async fn get_once(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

/// A GET, then a POST with a body on the connection the GET used.
async fn get_then_post(user: &mut GooseUser) -> TransactionResult {
    user.get("/first").await?.discard_body().await?;
    let_connection_return_to_pool().await;
    let _goose = user.post("/second", "body=1").await?;
    Ok(())
}

#[tokio::test]
/// A GET whose reused connection the server closes before responding is sent again on
/// a new connection, and succeeds.
async fn get_is_sent_again_after_reused_connection_closes() {
    let (host, seen) = start_server(Close::SecondRequestOnFirstConnection).await;
    let metrics = run(&host, transaction!(get_twice)).await;

    assert_eq!(
        seen.requests(),
        vec![
            "GET /first HTTP/1.1",
            "GET /second HTTP/1.1",
            "GET /second HTTP/1.1"
        ]
    );
    assert_eq!(seen.connections(), 2);
    assert_eq!(recorded(&metrics, "GET /first"), (1, 0));
    assert_eq!(recorded(&metrics, "GET /second"), (1, 0));
}

#[tokio::test]
/// A GET is sent again only once: when the second attempt's connection closes too, the
/// request is recorded once, as a failure.
async fn get_is_sent_again_only_once() {
    let (host, seen) = start_server(Close::Always).await;
    let metrics = run(&host, transaction!(get_once)).await;

    assert_eq!(seen.requests(), vec!["GET / HTTP/1.1", "GET / HTTP/1.1"]);
    assert_eq!(seen.connections(), 2);
    assert_eq!(recorded(&metrics, "GET /"), (0, 1));
}

#[tokio::test]
/// Goose can't tell a close after part of the response head from a close before any
/// of it, so a GET whose response stops partway through its head is sent again once,
/// and recorded once, as a failure.
async fn get_is_sent_again_after_partial_response() {
    let (host, seen) = start_server(Close::AfterPartialResponse).await;
    let metrics = run(&host, transaction!(get_once)).await;

    assert_eq!(seen.requests(), vec!["GET / HTTP/1.1", "GET / HTTP/1.1"]);
    assert_eq!(seen.connections(), 2);
    assert_eq!(recorded(&metrics, "GET /"), (0, 1));
}

#[tokio::test]
/// A POST is not idempotent, so a POST whose reused connection the server closes
/// before responding is not sent again, and is recorded as a failure.
async fn post_is_not_sent_again() {
    let (host, seen) = start_server(Close::SecondRequestOnFirstConnection).await;
    let metrics = run(&host, transaction!(get_then_post)).await;

    assert_eq!(
        seen.requests(),
        vec!["GET /first HTTP/1.1", "POST /second HTTP/1.1 body=1"]
    );
    assert_eq!(seen.connections(), 1);
    assert_eq!(recorded(&metrics, "GET /first"), (1, 0));
    assert_eq!(recorded(&metrics, "POST /second"), (0, 1));
}
