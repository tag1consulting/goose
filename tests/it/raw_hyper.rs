//! The raw hyper client, with the `raw-hyper` feature: `GooseUser::raw_get`,
//! `raw_post` and `GooseRequestBuilder::raw` record the same request metrics as
//! the reqwest path, return the same `GooseResponse`, and send over one kept alive
//! connection per user, plain or over TLS, with the user's cookies, the test's
//! headers and body, and no redirects or gzip.

use httpmock::{
    Method::{DELETE, GET, HEAD, PATCH, POST, PUT},
    MockServer,
};
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
    run_with(host, transaction, &[]).await
}

/// [`run`], with the options `extra` as well.
async fn run_with(host: &str, transaction: Transaction, extra: &[&str]) -> GooseMetrics {
    let iterations = ITERATIONS.to_string();
    let mut args = vec![
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
        "--no-telnet",
    ];
    args.extend_from_slice(extra);
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
/// `GooseResponse`, and sends every request on one connection, with `Host`,
/// Goose's `User-Agent` and `Accept: */*` as reqwest sends them, no
/// `Accept-Encoding`, and from the second request on, with Goose's `cookies`
/// feature, the cookie the server set.
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
    for (i, head) in heads.iter().enumerate() {
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
        assert!(lower.contains("accept: */*\r\n"), "{}", head);
        // Without Goose's cookies feature there are no cookies to send.
        assert_eq!(
            lower.contains("cookie: session=raw\r\n"),
            i > 0 && cfg!(feature = "cookies"),
            "request {}: {}",
            i,
            head
        );
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
    let failed = FAILED.lock().unwrap().clone();
    assert_eq!(failed.len(), ITERATIONS);
    for error in failed.iter() {
        assert!(error.starts_with("error connecting: "), "{}", error);
        assert!(error.ends_with(": /"), "{}", error);
    }
}

#[tokio::test]
#[serial(raw_hyper)]
/// A scheme other than http or https fails without a connection.
async fn raw_other_schemes_are_refused() {
    FAILED.lock().unwrap().clear();
    let metrics = run(
        "http://127.0.0.1:9",
        transaction!(raw_get_ftp_recording_failure),
    )
    .await;

    let request = metrics.requests.get("GET ftp").expect("GET ftp recorded");
    assert_eq!(request.fail_count, ITERATIONS);
    let failed = FAILED.lock().unwrap().clone();
    assert_eq!(failed.len(), ITERATIONS);
    for error in failed.iter() {
        assert!(error.contains("not ftp://"), "{}", error);
    }
}

async fn raw_get_ftp_recording_failure(user: &mut GooseUser) -> TransactionResult {
    match user.raw_get_named("ftp://127.0.0.1:9/", "ftp").await {
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

#[cfg(feature = "cookies")]
/// Logs in over reqwest, then requests a page raw, then another over reqwest.
async fn login_then_raw(user: &mut GooseUser) -> TransactionResult {
    let _ = user.get("/login").await?.discard_body().await;
    let _ = user.raw_get("/page").await?.discard_body().await;
    let _ = user.get("/after").await?.discard_body().await;
    Ok(())
}

#[cfg(feature = "cookies")]
/// The mocks for [`login_then_raw`]: `/login` sets `session`; `/page` answers only
/// with that cookie, and sets `raw`; `/after` answers only with both.
fn cookie_mocks(server: &MockServer) -> [httpmock::Mock<'_>; 3] {
    let login = server.mock(|when, then| {
        when.method(GET).path("/login");
        then.status(200)
            .header("Set-Cookie", "session=abc; Path=/")
            .body("in");
    });
    let page = server.mock(|when, then| {
        when.method(GET).path("/page").cookie("session", "abc");
        then.status(200)
            .header("Set-Cookie", "raw=1; Path=/")
            .body("page");
    });
    let after = server.mock(|when, then| {
        when.method(GET)
            .path("/after")
            .cookie("session", "abc")
            .cookie("raw", "1");
        then.status(200).body("after");
    });
    [login, page, after]
}

#[cfg(feature = "cookies")]
#[tokio::test]
#[serial(raw_hyper)]
/// A user logs in over reqwest, and its raw request carries the session cookie;
/// the cookie the raw response sets goes back to the user's reqwest client.
async fn raw_requests_share_the_users_cookies() {
    let server = MockServer::start();
    let [login, page, after] = cookie_mocks(&server);

    let metrics = run(&server.base_url(), transaction!(login_then_raw)).await;
    login.assert_calls(ITERATIONS);
    page.assert_calls(ITERATIONS);
    after.assert_calls(ITERATIONS);
    assert_eq!(metrics.requests.get("GET /page").unwrap().fail_count, 0);
    assert_eq!(metrics.requests.get("GET /after").unwrap().fail_count, 0);
}

#[cfg(feature = "cookies")]
/// Replaces the user's reqwest client with one the test builds, with cookies.
async fn replace_client(user: &mut GooseUser) -> TransactionResult {
    let builder = reqwest::Client::builder().cookie_store(true);
    user.set_client_builder(builder).await?;
    Ok(())
}

#[cfg(feature = "cookies")]
#[tokio::test]
#[serial(raw_hyper)]
/// A client the load test built holds a jar Goose can't see: its raw requests
/// send no cookies, while its reqwest requests still do.
async fn raw_requests_send_no_cookies_after_set_client_builder() {
    let server = MockServer::start();
    let login = server.mock(|when, then| {
        when.method(GET).path("/login");
        then.status(200).header("Set-Cookie", "session=abc; Path=/");
    });
    let page = server.mock(|when, then| {
        when.method(GET).path("/page").cookie_missing("session");
        then.status(200);
    });

    let iterations = ITERATIONS.to_string();
    let host = server.base_url();
    let args = [
        "--host",
        &host,
        "--users",
        "1",
        "--increase-rate",
        "1",
        "--iterations",
        &iterations,
        "--co-mitigation",
        "disabled",
        "--quiet",
        "--no-telnet",
    ];
    let configuration = GooseConfiguration::parse_args_default(&args).unwrap();
    let scenario = scenario!("Raw")
        .register_transaction(transaction!(replace_client).set_on_start())
        .register_transaction(transaction!(login_then_page));
    common::build_load_test(configuration, vec![scenario], None, None)
        .execute()
        .await
        .unwrap();
    login.assert_calls(ITERATIONS);
    page.assert_calls(ITERATIONS);
}

#[cfg(feature = "cookies")]
async fn login_then_page(user: &mut GooseUser) -> TransactionResult {
    let _ = user.get("/login").await?.discard_body().await;
    let _ = user.raw_get("/page").await?.discard_body().await;
    Ok(())
}

/// Every method, with headers and a body set on a reqwest `RequestBuilder`, and
/// `raw_post`.
async fn raw_methods_with_headers_and_bodies(user: &mut GooseUser) -> TransactionResult {
    let _ = user.raw_post("/post", "posted").await?.discard_body().await;

    for (method, path) in [
        (GooseMethod::Put, "/put"),
        (GooseMethod::Patch, "/patch"),
        (GooseMethod::Delete, "/delete"),
        (GooseMethod::Post, "/json"),
    ] {
        let mut builder = user
            .get_request_builder(&method, path)?
            .header("x-one", "1")
            .header("x-two", "2");
        builder = if path == "/json" {
            builder.json(&serde_json::json!({ "a": 1 }))
        } else {
            builder.body(format!("{} body", &path[1..]))
        };
        let request = GooseRequest::builder()
            .set_request_builder(builder)
            .raw(true)
            .build();
        let _ = user.request(request).await?.discard_body().await;
    }

    let builder = user
        .get_request_builder(&GooseMethod::Head, "/head")?
        .header("user-agent", "custom");
    let request = GooseRequest::builder()
        .set_request_builder(builder)
        .name("head")
        .raw(true)
        .build();
    let _ = user.request(request).await?.discard_body().await;

    let _ = user.raw_get("/get?q=1").await?.discard_body().await;
    Ok(())
}

#[tokio::test]
#[serial(raw_hyper)]
/// A raw request sends the method, headers and body the test set, and a
/// `User-Agent` of the test's own in place of Goose's.
async fn raw_requests_send_headers_and_bodies() {
    let server = MockServer::start();
    let post = server.mock(|when, then| {
        when.method(POST)
            .path("/post")
            .header("content-length", "6")
            .body("posted");
        then.status(201);
    });
    let mut with_body = Vec::new();
    for (method, path, body) in [
        (PUT, "/put", "put body"),
        (PATCH, "/patch", "patch body"),
        (DELETE, "/delete", "delete body"),
    ] {
        with_body.push(server.mock(|when, then| {
            when.method(method)
                .path(path)
                .header("x-one", "1")
                .header("x-two", "2")
                .body(body);
            then.status(200);
        }));
    }
    let json = server.mock(|when, then| {
        when.method(POST)
            .path("/json")
            .header("x-one", "1")
            .header("content-type", "application/json")
            .body(r#"{"a":1}"#);
        then.status(200);
    });
    let head = server.mock(|when, then| {
        when.method(HEAD)
            .path("/head")
            .header("user-agent", "custom");
        then.status(200);
    });
    let get = server.mock(|when, then| {
        when.method(GET).path("/get").query_param("q", "1");
        then.status(200);
    });

    let metrics = run(
        &server.base_url(),
        transaction!(raw_methods_with_headers_and_bodies),
    )
    .await;
    post.assert_calls(ITERATIONS);
    for mock in &with_body {
        mock.assert_calls(ITERATIONS);
    }
    json.assert_calls(ITERATIONS);
    head.assert_calls(ITERATIONS);
    get.assert_calls(ITERATIONS);
    for (name, request) in &metrics.requests {
        assert_eq!(request.fail_count, 0, "{}", name);
    }
    assert_eq!(
        metrics.requests.get("POST /post").unwrap().success_count,
        ITERATIONS
    );
    assert_eq!(
        metrics.requests.get("HEAD head").unwrap().success_count,
        ITERATIONS
    );
}

/// A raw request set to HTTP/2, which a raw request can't honour.
async fn raw_http2_recording_error(user: &mut GooseUser) -> TransactionResult {
    let builder = user
        .get_request_builder(&GooseMethod::Get, "/")?
        .version(reqwest::Version::HTTP_2);
    let request = GooseRequest::builder()
        .set_request_builder(builder)
        .raw(true)
        .build();
    match user.request(request).await {
        Ok(_) => FAILED.lock().unwrap().push("no error".to_string()),
        Err(e) => FAILED.lock().unwrap().push(e.to_string()),
    }
    Ok(())
}

#[tokio::test]
#[serial(raw_hyper)]
/// A builder call a raw request can't honour is refused before anything is sent,
/// not ignored.
async fn raw_refuses_what_it_cannot_honour() {
    FAILED.lock().unwrap().clear();
    let server = MockServer::start();
    let any = server.mock(|when, then| {
        when.any_request();
        then.status(200);
    });
    let _ = run(&server.base_url(), transaction!(raw_http2_recording_error)).await;
    any.assert_calls(0);
    let failed = FAILED.lock().unwrap().clone();
    assert_eq!(failed.len(), ITERATIONS);
    for error in failed.iter() {
        assert!(
            error.contains("sent over HTTP/1.1, not HTTP/2"),
            "{}",
            error
        );
    }
}

/// Reads a raw HTTPS response's body.
async fn raw_get_https(user: &mut GooseUser) -> TransactionResult {
    match user.raw_get("/").await {
        Ok(goose) => {
            let text = goose.response?.text().await?;
            FAILED.lock().unwrap().push(format!("body: {text}"));
        }
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
/// Over HTTPS a raw request checks the certificate as the user's reqwest client
/// does: a certificate from an unknown CA fails the handshake, unless
/// `--accept-invalid-certs` is set, and then the request goes through, over
/// HTTP/1.1 by ALPN.
async fn raw_https_checks_certificates_as_reqwest_does() {
    use crate::tls_server::{alpn_server, Offer};

    let (url, protocols) = alpn_server(Offer::Http1, None);
    FAILED.lock().unwrap().clear();
    let _ = run(&url, transaction!(raw_get_https)).await;
    for error in FAILED.lock().unwrap().clone().iter() {
        assert!(
            error.starts_with("error in the TLS handshake: "),
            "{}",
            error
        );
    }
    assert!(protocols.try_recv().is_err(), "no handshake completed");

    FAILED.lock().unwrap().clear();
    let _ = run_with(
        &url,
        transaction!(raw_get_https),
        &["--accept-invalid-certs"],
    )
    .await;
    let failed = FAILED.lock().unwrap().clone();
    assert_eq!(failed.len(), ITERATIONS);
    for result in failed.iter() {
        assert_eq!(result, "body: ok");
    }
    let negotiated: Vec<String> = protocols.try_iter().collect();
    assert_eq!(negotiated.len(), ITERATIONS);
    assert!(
        negotiated.iter().all(|p| p == "http/1.1"),
        "{:?}",
        negotiated
    );
}

/// Starts a server that answers each request with a head promising 100 bytes, then
/// sends 7 and stalls. Returns its base URL and the connections it accepted.
async fn start_stalling_server() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let accepted = connections.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            accepted.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => request.extend_from_slice(&buffer[..read]),
                    }
                }
                let _ = socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial")
                    .await;
                // Holds the connection open until the client closes it.
                while let Ok(read) = socket.read(&mut buffer).await {
                    if read == 0 {
                        return;
                    }
                }
            });
        }
    });
    (format!("http://{address}"), connections)
}

/// Reads the body, which never ends, and records how the read ended.
async fn raw_get_stalled_text(user: &mut GooseUser) -> TransactionResult {
    let goose = user.raw_get("/").await?;
    match goose.response?.text().await {
        Ok(text) => FAILED.lock().unwrap().push(format!("read: {text}")),
        Err(e) => FAILED.lock().unwrap().push(format!("{e:?}")),
    }
    Ok(())
}

#[tokio::test]
#[serial(raw_hyper)]
/// A server that sends a head and stalls: the request timeout covers the body, so
/// the test's read of it fails, and Goose's read of a body the test dropped unread
/// ends too, and the user goes on, with a new connection each time.
async fn raw_timeout_covers_the_body_goose_reads() {
    let run_both = async {
        let (host, connections) = start_stalling_server().await;
        let metrics = run_with(&host, transaction!(raw_get_dropped), &["--timeout", "0.3"]).await;
        let request = metrics.requests.get("GET dropped").expect("named request");
        // Each head came in time.
        assert_eq!(request.success_count, ITERATIONS);
        assert_eq!(connections.load(Ordering::SeqCst), ITERATIONS);

        FAILED.lock().unwrap().clear();
        let (host, _) = start_stalling_server().await;
        let _ = run_with(
            &host,
            transaction!(raw_get_stalled_text),
            &["--timeout", "0.3"],
        )
        .await;
        let failed = FAILED.lock().unwrap().clone();
        assert_eq!(failed.len(), ITERATIONS);
        for error in failed.iter() {
            // The error reqwest wraps the raw body's timeout in.
            assert!(error.contains("source: Timeout(300ms)"), "{}", error);
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(60), run_both)
        .await
        .expect("no user waits for good on a stalled body");
}
