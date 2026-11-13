//! The test TLS servers for the unit tests in `src/client.rs` and for the tests
//! in `tests/` that include this file. Cargo does not build it as a test target of
//! its own.

// Each file that includes this one uses only some of it.
#![allow(dead_code)]

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A server config for `localhost`, with a certificate signed by
/// `tests/fixtures/tls/ca.pem`, offering nothing by ALPN.
fn server_config() -> rustls::ServerConfig {
    let certs = CertificateDer::pem_slice_iter(include_bytes!("../fixtures/tls/localhost.pem"))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let key =
        PrivateKeyDer::from_pem_slice(include_bytes!("../fixtures/tls/localhost.key")).unwrap();
    rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .unwrap()
}

/// Answers each HTTPS request on 127.0.0.1 with `200 ok`, as `localhost`, with a
/// certificate signed by `tests/fixtures/tls/ca.pem`. It closes every connection, so
/// each request is a new TLS handshake, and sends whether each handshake that got as
/// far as a request resumed a session. A dropped receiver is ignored.
pub(crate) fn tls_server() -> (String, std::sync::mpsc::Receiver<rustls::HandshakeKind>) {
    let config = Arc::new(server_config());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "https://localhost:{}/",
        listener.local_addr().unwrap().port()
    );
    let (handshakes, received) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let stream = stream.unwrap();
            let connection = rustls::ServerConnection::new(config.clone()).unwrap();
            let mut tls = rustls::StreamOwned::new(connection, stream);
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match tls.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => request.extend_from_slice(&buffer[..read]),
                }
            }
            if !request.windows(4).any(|w| w == b"\r\n\r\n") {
                // The client refused the certificate.
                continue;
            }
            let _ = handshakes.send(tls.conn.handshake_kind().unwrap());
            tls.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
                .unwrap();
            tls.conn.send_close_notify();
            let _ = tls.flush();
        }
    });
    (url, received)
}

/// The protocols an [`alpn_server`] offers by ALPN.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Offer {
    /// `h2`, then `http/1.1`.
    Http2AndHttp1,
    /// `http/1.1` only.
    Http1,
}

/// Like [`tls_server`], but offering `offer` by ALPN and answering over the
/// protocol each handshake negotiated: over HTTP/1.1 it closes the connection after
/// one request; over HTTP/2 it answers every request on the connection. It sends the
/// protocol each completed handshake negotiated, `h2` or `http/1.1`, or an empty
/// string when the client offered neither. With `goaway_after` set to `Some(n)`, it
/// shuts the first HTTP/2 connection down gracefully, with GOAWAY, after `n`
/// requests, reports `GOAWAY` on the channel when it does, and keeps accepting new
/// connections. A dropped receiver is ignored.
pub(crate) fn alpn_server(
    offer: Offer,
    goaway_after: Option<usize>,
) -> (String, std::sync::mpsc::Receiver<String>) {
    let mut config = server_config();
    config.alpn_protocols = match offer {
        Offer::Http2AndHttp1 => vec![b"h2".to_vec(), b"http/1.1".to_vec()],
        Offer::Http1 => vec![b"http/1.1".to_vec()],
    };
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "https://localhost:{}/",
        listener.local_addr().unwrap().port()
    );
    let (protocols, received) = std::sync::mpsc::channel();
    // Its own runtime, so a test can block on the receiver.
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let first_http2 = Arc::new(AtomicBool::new(true));
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let acceptor = acceptor.clone();
                let protocols = protocols.clone();
                let first_http2 = first_http2.clone();
                tokio::spawn(async move {
                    // A client that refuses the certificate ends the handshake here.
                    let Ok(tls) = acceptor.accept(stream).await else {
                        return;
                    };
                    let protocol = tls
                        .get_ref()
                        .1
                        .alpn_protocol()
                        .map(|protocol| String::from_utf8_lossy(protocol).into_owned())
                        .unwrap_or_default();
                    let _ = protocols.send(protocol.clone());
                    if protocol == "h2" {
                        // Only the first HTTP/2 connection gets GOAWAY.
                        let goaway_after =
                            goaway_after.filter(|_| first_http2.swap(false, Ordering::SeqCst));
                        serve_http2(tls, goaway_after, &protocols).await;
                    } else {
                        serve_http1(tls).await;
                    }
                });
            }
        });
    });
    (url, received)
}

/// Answers one HTTP/1.1 request with `200 ok` and closes the connection.
async fn serve_http1(mut tls: tokio_rustls::server::TlsStream<tokio::net::TcpStream>) {
    let mut request = Vec::new();
    let mut buffer = [0; 4096];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        match tls.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(read) => request.extend_from_slice(&buffer[..read]),
        }
    }
    let _ = tls
        .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
        .await;
    let _ = tls.shutdown().await;
}

/// Answers every HTTP/2 request on the connection with `200 ok`, and after
/// `goaway_after` requests, when set, shuts the connection down gracefully and
/// sends `GOAWAY` on `events`.
async fn serve_http2(
    tls: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    goaway_after: Option<usize>,
    events: &std::sync::mpsc::Sender<String>,
) {
    let Ok(mut connection) = h2::server::handshake(tls).await else {
        return;
    };
    let mut answered = 0;
    while let Some(Ok((_request, mut respond))) = connection.accept().await {
        if let Ok(mut body) = respond.send_response(http::Response::new(()), false) {
            let _ = body.send_data(bytes::Bytes::from_static(b"ok"), true);
        }
        answered += 1;
        if goaway_after == Some(answered) {
            connection.graceful_shutdown();
            let _ = events.send("GOAWAY".to_string());
        }
    }
}
