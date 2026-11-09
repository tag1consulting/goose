//! A Goose user trusts the CA the system trusts, as a client reqwest builds itself
//! does. Its own test binary, as it points `SSL_CERT_FILE` at the test CA before
//! the first client in the process reads the system's root certificates, and the
//! only test in it, so no other thread reads the environment while it is set. Only
//! where the platform verifier reads the roots through `rustls-native-certs`, which
//! honors `SSL_CERT_FILE`: not on macOS, iOS, Windows or Android.
#![cfg(all(unix, not(target_vendor = "apple"), not(target_os = "android")))]

#[path = "common/tls_server.rs"]
mod tls_server;

use goose::config::GooseConfiguration;
use goose::goose::GooseUser;
use std::sync::Arc;

#[tokio::test]
async fn user_trusts_the_systems_roots_as_reqwest_does() {
    // Set before any client in this process is built.
    std::env::set_var("SSL_CERT_FILE", "tests/fixtures/tls/ca.pem");
    let (url, _) = tls_server::tls_server();

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
