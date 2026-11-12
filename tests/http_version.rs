//! Every path that builds a Goose client negotiates HTTP/2 over HTTPS when the
//! server offers it, and HTTP/1.1 with `--http1-only`, whether it checks
//! certificates or not; plain HTTP is HTTP/1.1. Its own test binary, as it points
//! `SSL_CERT_FILE` at the test CA before the first client in the process reads the
//! system's root certificates, and the only test in it, so no other thread reads
//! the environment while it is set. Only where the platform verifier reads the
//! roots through `rustls-native-certs`, which honors `SSL_CERT_FILE`: not on macOS,
//! iOS, Windows or Android.
#![cfg(all(unix, not(target_vendor = "apple"), not(target_os = "android")))]

#[path = "common/tls_server.rs"]
mod tls_server;

use goose::client::GooseClientBuilder;
use goose::config::GooseConfiguration;
use goose::prelude::*;
use httpmock::MockServer;
use std::sync::Mutex;
use std::time::Duration;
use tls_server::{alpn_server, Offer};

/// The HTTP version of each response the users got, as the client saw it.
static VERSIONS: Mutex<Vec<reqwest::Version>> = Mutex::new(Vec::new());

/// How a load test gives its users their clients.
#[derive(Clone, Copy, Debug)]
enum ClientPath {
    /// No client builder: each user builds its own from the configuration.
    NoBuilder,
    /// A client builder with cookies: a client for each user.
    #[cfg(feature = "cookies")]
    WithCookies,
    /// A client builder without cookies: one client every user shares.
    WithoutCookies,
}

async fn request_root(user: &mut GooseUser) -> TransactionResult {
    let goose = user.get("/").await?;
    if let Ok(response) = &goose.response {
        VERSIONS.lock().unwrap().push(response.version());
    }
    Ok(())
}

/// Runs one user for one iteration against `host` over `path` with `options`, and
/// returns the HTTP version of each response it got.
async fn run(path: ClientPath, host: &str, options: &[&str]) -> Vec<reqwest::Version> {
    let mut args = vec![
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
    ];
    args.extend_from_slice(options);
    let configuration = GooseConfiguration::parse_args_default(&args).unwrap();
    let mut goose_attack = GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("Version").register_transaction(transaction!(request_root)));
    match path {
        ClientPath::NoBuilder => {}
        #[cfg(feature = "cookies")]
        ClientPath::WithCookies => {
            goose_attack = goose_attack
                .set_client_builder_with_cookies(GooseClientBuilder::new())
                .unwrap()
        }
        ClientPath::WithoutCookies => {
            goose_attack = goose_attack
                .set_client_builder_without_cookies(GooseClientBuilder::new().without_cookies())
                .unwrap()
        }
    }
    let metrics = goose_attack.execute().await.unwrap();
    let requests = metrics.requests.get("GET /").expect("the user made a request");
    assert_eq!(requests.fail_count, 0, "{path:?} {options:?}");
    std::mem::take(&mut *VERSIONS.lock().unwrap())
}

#[tokio::test]
async fn every_client_path_negotiates_http2_unless_http1_only() {
    // Set before any client in this process is built.
    std::env::set_var("SSL_CERT_FILE", "tests/fixtures/tls/ca.pem");
    let (url, protocols) = alpn_server(Offer::Http2AndHttp1, None);
    let plain = MockServer::start();
    plain.mock(|when, then| {
        when.path("/");
        then.status(200);
    });

    let paths = [
        ClientPath::NoBuilder,
        #[cfg(feature = "cookies")]
        ClientPath::WithCookies,
        ClientPath::WithoutCookies,
    ];
    for path in paths {
        for (options, expected) in [
            (&[][..], "h2"),
            (&["--http1-only"][..], "http/1.1"),
            (&["--accept-invalid-certs"][..], "h2"),
            (&["--accept-invalid-certs", "--http1-only"][..], "http/1.1"),
        ] {
            let versions = run(path, &url, options).await;
            let expected_version = if expected == "h2" {
                reqwest::Version::HTTP_2
            } else {
                reqwest::Version::HTTP_11
            };
            assert_eq!(versions, [expected_version], "{path:?} {options:?}");
            let negotiated: Vec<String> = protocols.try_iter().collect();
            assert!(!negotiated.is_empty(), "{path:?} {options:?}: no handshake");
            assert!(
                negotiated.iter().all(|protocol| protocol == expected),
                "{path:?} {options:?}: {negotiated:?}, expected {expected}"
            );

            // The same options over plain HTTP: always HTTP/1.1.
            let versions = run(path, &plain.base_url(), options).await;
            assert_eq!(
                versions,
                [reqwest::Version::HTTP_11],
                "{path:?} {options:?} over plain HTTP"
            );
        }
    }
    // Nothing else connected to the TLS server.
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(protocols.try_iter().count(), 0);
}
