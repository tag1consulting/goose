//! HTTP/2 in the clients Goose builds: which setting decides between HTTP/2 and
//! HTTP/1.1, and what a user sees when a server ends an HTTP/2 connection with
//! GOAWAY. Each test accepts the test server's certificate rather than trusting
//! its CA.

#[path = "common/tls_server.rs"]
mod tls_server;

use goose::client::{ClientStrategy, GooseClientBuilder};
use goose::config::{GooseConfiguration, GooseDefault};
use goose::goose::GooseUser;
use goose::prelude::*;
use std::sync::Arc;
use std::time::Duration;
use tls_server::{alpn_server, Offer};

async fn request_root(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

/// The protocols one user negotiated with a server that offers HTTP/2 and
/// HTTP/1.1, over a client builder without cookies, given `http1_only` when set,
/// with `command_line` and the `GooseDefault::Http1Only` `default` when set.
async fn negotiated(
    builder_http1_only: Option<bool>,
    command_line: &[&str],
    default: Option<bool>,
) -> Vec<String> {
    let (url, protocols) = alpn_server(Offer::Http2AndHttp1, None);
    let mut args = vec![
        "--host",
        url.as_str(),
        "--users",
        "1",
        "--increase-rate",
        "1",
        "--iterations",
        "1",
        "--co-mitigation",
        "disabled",
        "--accept-invalid-certs",
        "--quiet",
    ];
    args.extend_from_slice(command_line);
    let mut builder = GooseClientBuilder::new().without_cookies();
    if let Some(http1_only) = builder_http1_only {
        builder = builder.http1_only(http1_only);
    }
    let mut goose_attack =
        GooseAttack::initialize_with_config(GooseConfiguration::parse_args_default(&args).unwrap())
            .unwrap()
            .set_client_builder_without_cookies(builder)
            .unwrap()
            .register_scenario(
                scenario!("Precedence").register_transaction(transaction!(request_root)),
            );
    if let Some(value) = default {
        goose_attack = *goose_attack
            .set_default(GooseDefault::Http1Only, value)
            .unwrap();
    }
    let metrics = goose_attack.execute().await.unwrap();
    assert_eq!(metrics.requests.get("GET /").unwrap().fail_count, 0);
    protocols.try_iter().collect()
}

#[tokio::test]
async fn test_http1_only_precedence() {
    // Nothing set: HTTP/2.
    assert_eq!(negotiated(None, &[], None).await, ["h2"]);

    // A GooseDefault applies when nothing else is set.
    assert_eq!(negotiated(None, &[], Some(true)).await, ["http/1.1"]);

    // The builder wins over a GooseDefault, either way.
    assert_eq!(
        negotiated(Some(false), &[], Some(true)).await,
        ["h2"],
        "a GooseDefault replaced the builder's http1_only(false)"
    );
    assert_eq!(
        negotiated(Some(true), &[], Some(false)).await,
        ["http/1.1"],
        "a GooseDefault replaced the builder's http1_only(true)"
    );

    // --http1-only on the command line wins over the builder.
    assert_eq!(
        negotiated(Some(false), &["--http1-only"], None).await,
        ["http/1.1"],
        "--http1-only did not override the builder"
    );
}

/// Requests each user makes.
const REQUESTS: usize = 30;
/// Users sharing one client, so requests overlap on one HTTP/2 connection.
const USERS: usize = 4;
/// Requests the server answers on its first HTTP/2 connection before GOAWAY.
const GOAWAY_AFTER: usize = 10;

/// Users sharing one client make requests across a GOAWAY on the first HTTP/2
/// connection. Each request is answered or fails, none hangs, and the requests
/// after it are answered on a new connection. How many fail at the GOAWAY is
/// printed, not checked.
#[tokio::test]
async fn http2_goaway_neither_hangs_nor_breaks_later_requests() {
    let (url, protocols) = alpn_server(Offer::Http2AndHttp1, Some(GOAWAY_AFTER));
    let ClientStrategy::Shared(client) = GooseClientBuilder::new()
        .without_cookies()
        .accept_invalid_certs(true)
        .build_strategy()
        .unwrap()
    else {
        panic!("a builder without cookies builds a shared client");
    };
    let configuration = GooseConfiguration::default();
    let mut users = Vec::new();
    for _ in 0..USERS {
        let mut user = GooseUser::new(
            0,
            Arc::from("GOAWAY"),
            url::Url::parse(&url).unwrap(),
            &configuration,
            0,
            Some(reqwest::Client::clone(&client)),
        )
        .unwrap();
        users.push(tokio::spawn(async move {
            let mut outcomes = Vec::new();
            for _ in 0..REQUESTS {
                let goose = user.get("/").await.unwrap();
                let recorded_success = goose.request.success;
                let answered = match goose.response {
                    Ok(response) => {
                        response.version() == reqwest::Version::HTTP_2
                            && response.text().await.ok().as_deref() == Some("ok")
                    }
                    Err(_) => false,
                };
                // A request that was not answered is recorded as failed.
                assert!(answered || !recorded_success);
                outcomes.push(answered);
            }
            outcomes
        }));
    }
    let outcomes = tokio::time::timeout(Duration::from_secs(60), async {
        let mut outcomes = Vec::new();
        for user in users {
            outcomes.push(user.await.unwrap());
        }
        outcomes
    })
    .await
    .expect("every request ends within 60 seconds");

    let failed = outcomes.iter().flatten().filter(|&&answered| !answered).count();
    println!(
        "GOAWAY after {GOAWAY_AFTER} requests: {failed} of {} requests failed",
        USERS * REQUESTS
    );
    let handshakes: Vec<String> = protocols.try_iter().collect();
    assert!(
        handshakes.len() >= 2 && handshakes.iter().all(|protocol| protocol == "h2"),
        "{handshakes:?}"
    );
    // Every user's last requests were answered, on a later connection.
    for user in &outcomes {
        assert!(
            user[REQUESTS - GOAWAY_AFTER..].iter().all(|&answered| answered),
            "{outcomes:?}"
        );
    }
}
