//! The raw hyper client, with the `raw-hyper` feature: `GooseUser::raw_get` and
//! `GooseRequestBuilder::raw` record the same request metrics as the reqwest
//! path, return the same `GooseResponse`, and send over one kept alive connection
//! per user, with no cookies, redirects or gzip.

use httpmock::{Method::GET, MockServer};
use serial_test::serial;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::common;

use goose::config::GooseConfiguration;
use goose::metrics::GooseMetrics;
use goose::prelude::*;

/// How many times each user runs its transaction.
const ITERATIONS: usize = 6;

/// What the local server answers every request with.
const BODY: &str = "Hello, raw world";

/// Body bytes the transactions read, summed, checked by the test itself: a panic
/// inside a user's transaction would not fail the test.
static READ_BYTES: AtomicU64 = AtomicU64::new(0);

/// Each request's head, as the local server received it.
type Heads = Arc<Mutex<Vec<String>>>;

/// Starts an HTTP/1.1 server on a free local port that answers every GET with
/// [`BODY`] and a cookie. With `close_after_one`, it closes each connection
/// after its first response, with no `Connection: close` to say so: the server
/// that closes an idle keep-alive connection as the next request goes out.
/// Returns its base URL, the connections it accepted, and the request heads.
async fn start_server(close_after_one: bool) -> (String, Arc<AtomicUsize>, Heads) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let heads: Heads = Arc::new(Mutex::new(Vec::new()));
    let (accepted, seen) = (connections.clone(), heads.clone());
    tokio::spawn(async move {
        let response = format!(
            "HTTP/1.1 200 OK\r\nSet-Cookie: session=raw\r\nContent-Length: {}\r\n\r\n{BODY}",
            BODY.len()
        );
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            accepted.fetch_add(1, Ordering::SeqCst);
            let (response, seen) = (response.clone(), seen.clone());
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let read = match socket.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => read,
                    };
                    request.extend_from_slice(&buffer[..read]);
                    while let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head: Vec<u8> = request.drain(..end + 4).collect();
                        seen.lock()
                            .unwrap()
                            .push(String::from_utf8_lossy(&head).to_string());
                        if socket.write_all(response.as_bytes()).await.is_err() {
                            return;
                        }
                        if close_after_one {
                            return;
                        }
                    }
                }
            });
        }
    });
    (format!("http://{address}"), connections, heads)
}

/// Runs one user, `ITERATIONS` times, through `transaction` against `host`.
async fn run(host: &str, transaction: Transaction) -> GooseMetrics {
    let iterations = ITERATIONS.to_string();
    let args = [
        "--host",
        host,
        "--users",
        "1",
        "--increase-rate",
        "1",
        "--iterations",
        &iterations,
        "--co-mitigation",
        "disabled",
        "--quiet",
    ];
    let configuration = GooseConfiguration::parse_args_default(&args).unwrap();
    let scenario = scenario!("Raw").register_transaction(transaction);
    common::build_load_test(configuration, vec![scenario], None, None)
        .execute()
        .await
        .unwrap()
}

/// Reads the body with `text()`, as a test reads any response.
async fn raw_get_text(user: &mut GooseUser) -> TransactionResult {
    let goose = user.raw_get("/").await?;
    let text = goose.response?.text().await?;
    READ_BYTES.fetch_add(text.len() as u64, Ordering::SeqCst);
    Ok(())
}

/// Drops the response with its body unread, for Goose to read.
async fn raw_get_dropped(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.raw_get_named("/", "dropped").await?;
    Ok(())
}

#[tokio::test]
#[serial(raw_hyper)]
/// `raw_get` records each request as `get` does, returns its body through the same
/// `GooseResponse`, and sends every request on one connection, with `Host` and
/// Goose's `User-Agent` and no cookie, though the server sets one.
async fn raw_get_records_metrics_on_one_connection() {
    READ_BYTES.store(0, Ordering::SeqCst);
    let (host, connections, heads) = start_server(false).await;
    let metrics = run(&host, transaction!(raw_get_text)).await;

    let request = metrics.requests.get("GET /").expect("GET / recorded");
    assert_eq!(request.success_count, ITERATIONS);
    assert_eq!(request.fail_count, 0);
    assert_eq!(request.status_code_counts.get(&200), Some(&ITERATIONS));
    assert_eq!(
        READ_BYTES.load(Ordering::SeqCst),
        (ITERATIONS * BODY.len()) as u64
    );
    assert_eq!(connections.load(Ordering::SeqCst), 1);

    let heads = heads.lock().unwrap();
    assert_eq!(heads.len(), ITERATIONS);
    let port = host.rsplit(':').next().unwrap();
    for head in heads.iter() {
        let lower = head.to_ascii_lowercase();
        assert!(lower.starts_with("get / http/1.1\r\n"), "{}", head);
        assert!(
            lower.contains(&format!("host: 127.0.0.1:{port}\r\n")),
            "{}",
            head
        );
        assert!(
            lower.contains(&format!(
                "user-agent: goose/{}\r\n",
                env!("CARGO_PKG_VERSION")
            )),
            "{}",
            head
        );
        assert!(!lower.contains("cookie"), "{}", head);
        assert!(!lower.contains("accept-encoding"), "{}", head);
    }
}

#[tokio::test]
#[serial(raw_hyper)]
/// A body dropped unread is read by Goose, so the user keeps its one connection,
/// and the request carries the name it was given.
async fn raw_dropped_bodies_keep_the_connection() {
    let (host, connections, _) = start_server(false).await;
    let metrics = run(&host, transaction!(raw_get_dropped)).await;

    let request = metrics.requests.get("GET dropped").expect("named request");
    assert_eq!(request.success_count, ITERATIONS);
    assert_eq!(connections.load(Ordering::SeqCst), 1);
}

#[tokio::test]
#[serial(raw_hyper)]
/// A server that closes each connection after one response: each next request
/// finds its connection closed, opens another, and succeeds.
async fn raw_requests_survive_closed_connections() {
    READ_BYTES.store(0, Ordering::SeqCst);
    let (host, connections, _) = start_server(true).await;
    let metrics = run(&host, transaction!(raw_get_text)).await;

    let request = metrics.requests.get("GET /").expect("GET / recorded");
    assert_eq!(request.success_count, ITERATIONS);
    assert_eq!(request.fail_count, 0);
    assert_eq!(connections.load(Ordering::SeqCst), ITERATIONS);
}

/// A raw request built with `GooseRequest`, expecting a 404.
async fn raw_expect_404(user: &mut GooseUser) -> TransactionResult {
    let request = GooseRequest::builder()
        .path("/missing")
        .expect_status_code(404)
        .raw(true)
        .build();
    let _ = user.request(request).await?.discard_body().await;
    Ok(())
}

/// A raw request answered with a redirect, which it does not follow.
async fn raw_get_redirect(user: &mut GooseUser) -> TransactionResult {
    let goose = user.raw_get("/redirect").await?;
    let status = goose.response?.status();
    READ_BYTES.store(status.as_u16() as u64, Ordering::SeqCst);
    Ok(())
}

#[tokio::test]
#[serial(raw_hyper)]
/// Status codes count as on the reqwest path: an expected 404 succeeds, and a
/// redirect is returned as it came, and fails as any status but 2xx does.
async fn raw_statuses_and_redirects() {
    READ_BYTES.store(0, Ordering::SeqCst);
    let server = MockServer::start();
    let missing = server.mock(|when, then| {
        when.method(GET).path("/missing");
        then.status(404);
    });
    let redirect = server.mock(|when, then| {
        when.method(GET).path("/redirect");
        then.status(302).header("Location", "/elsewhere");
    });
    let elsewhere = server.mock(|when, then| {
        when.method(GET).path("/elsewhere");
        then.status(200);
    });

    let metrics = run(&server.base_url(), transaction!(raw_expect_404)).await;
    let request = metrics.requests.get("GET /missing").unwrap();
    assert_eq!(request.success_count, ITERATIONS);
    assert_eq!(request.fail_count, 0);
    missing.assert_calls(ITERATIONS);

    let metrics = run(&server.base_url(), transaction!(raw_get_redirect)).await;
    let request = metrics.requests.get("GET /redirect").unwrap();
    assert_eq!(request.fail_count, ITERATIONS);
    assert_eq!(request.status_code_counts.get(&302), Some(&ITERATIONS));
    assert_eq!(READ_BYTES.load(Ordering::SeqCst), 302);
    redirect.assert_calls(ITERATIONS);
    elsewhere.assert_calls(0);
}

/// Whether a raw request failed the transaction with `RequestFailed`, and with
/// what error.
static FAILED: Mutex<Vec<String>> = Mutex::new(Vec::new());

async fn raw_get_recording_failure(user: &mut GooseUser) -> TransactionResult {
    match user.raw_get("/").await {
        Ok(_) => FAILED.lock().unwrap().push("no error".to_string()),
        Err(e) => match *e {
            TransactionError::RequestFailed { ref raw_request } => {
                FAILED.lock().unwrap().push(raw_request.error.clone())
            }
            ref other => FAILED.lock().unwrap().push(format!("other: {other}")),
        },
    }
    Ok(())
}

#[tokio::test]
#[serial(raw_hyper)]
/// A request that gets no response, here as nothing listens on the port, is
/// recorded as failed, with its error, and returns `RequestFailed`.
async fn raw_connect_failure_is_recorded() {
    FAILED.lock().unwrap().clear();
    // A port nothing listens on: bound, then closed.
    let port = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let metrics = run(
        &format!("http://127.0.0.1:{port}"),
        transaction!(raw_get_recording_failure),
    )
    .await;

    let request = metrics.requests.get("GET /").expect("GET / recorded");
    assert_eq!(request.success_count, 0);
    assert_eq!(request.fail_count, ITERATIONS);
    let failed = FAILED.lock().unwrap();
    assert_eq!(failed.len(), ITERATIONS);
    for error in failed.iter() {
        assert!(error.starts_with("error connecting: "), "{}", error);
        assert!(error.ends_with(": /"), "{}", error);
    }
}

#[tokio::test]
#[serial(raw_hyper)]
/// The raw client has no TLS: an https URL fails without a connection.
async fn raw_https_is_refused() {
    FAILED.lock().unwrap().clear();
    let metrics = run(
        "https://127.0.0.1:9",
        transaction!(raw_get_recording_failure),
    )
    .await;

    let request = metrics.requests.get("GET /").expect("GET / recorded");
    assert_eq!(request.fail_count, ITERATIONS);
    let failed = FAILED.lock().unwrap();
    assert_eq!(failed.len(), ITERATIONS);
    for error in failed.iter() {
        assert!(
            error.contains("http:// requests only, not https://"),
            "{}",
            error
        );
    }
}
