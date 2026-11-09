//! The test TLS server for the unit tests in `src/client.rs` and for
//! `tests/tls_roots.rs`. Cargo does not build it as a test target of its own.

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::io::{Read, Write};
use std::sync::Arc;

/// Answers each HTTPS request on 127.0.0.1 with `200 ok`, as `localhost`, with a
/// certificate signed by `tests/fixtures/tls/ca.pem`. It closes every connection, so
/// each request is a new TLS handshake, and sends whether each handshake that got as
/// far as a request resumed a session. A dropped receiver is ignored.
pub(crate) fn tls_server() -> (String, std::sync::mpsc::Receiver<rustls::HandshakeKind>) {
    let certs = CertificateDer::pem_slice_iter(include_bytes!("../fixtures/tls/localhost.pem"))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let key =
        PrivateKeyDer::from_pem_slice(include_bytes!("../fixtures/tls/localhost.key")).unwrap();
    let config = Arc::new(
        rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap(),
    );
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
