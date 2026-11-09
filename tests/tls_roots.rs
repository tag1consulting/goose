//! A Goose user trusts the CA the system trusts, as a client reqwest builds itself
//! does. Its own test binary, as it points `SSL_CERT_FILE` at the test CA before
//! the first client in the process reads the system's root certificates.

use goose::config::GooseConfiguration;
use goose::goose::GooseUser;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::io::{Read, Write};
use std::sync::Arc;

/// Answers each HTTPS request on 127.0.0.1 with `200 ok`, as `localhost`, with a
/// certificate signed by `tests/fixtures/tls/ca.pem`, and closes the connection.
fn tls_server() -> String {
    let certs = CertificateDer::pem_file_iter("tests/fixtures/tls/localhost.pem")
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let key = PrivateKeyDer::from_pem_file("tests/fixtures/tls/localhost.key").unwrap();
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
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let connection = rustls::ServerConnection::new(config.clone()).unwrap();
            let mut tls = rustls::StreamOwned::new(connection, stream.unwrap());
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match tls.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => request.extend_from_slice(&buffer[..read]),
                }
            }
            let _ = tls
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok");
            tls.conn.send_close_notify();
            let _ = tls.flush();
        }
    });
    url
}

#[tokio::test]
async fn user_trusts_the_systems_roots_as_reqwest_does() {
    // rustls-native-certs reads the system's roots from this file on Linux, where
    // Goose's CI runs; set before any client in this process is built.
    std::env::set_var("SSL_CERT_FILE", "tests/fixtures/tls/ca.pem");
    let url = tls_server();

    let configuration = GooseConfiguration::default();
    let mut user = GooseUser::new(
        0,
        Arc::from(""),
        url::Url::parse(&url).unwrap(),
        &configuration,
        0,
        None,
    )
    .unwrap();
    let goose = user.get("/").await.unwrap();
    assert_eq!(goose.response.unwrap().text().await.unwrap(), "ok");

    let reqwest = reqwest::Client::builder().build().unwrap();
    let response = reqwest.get(&url).send().await.unwrap();
    assert_eq!(response.text().await.unwrap(), "ok");
}
