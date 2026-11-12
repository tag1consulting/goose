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

/// Runs one user against a server that offers HTTP/2 and HTTP/1.1, and returns
/// the protocols it negotiated. The user's client comes from a client builder
/// without cookies. `builder_http1_only`, when set, is the builder's
/// `http1_only`. `command_line` is added to the command line. `default`, when
/// set, is the `GooseDefault::Http1Only`.
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
/// connection. Each request is answered or fails, none hangs, and the client opens
/// a new connection after it, on which every user's last request is answered. How
/// many fail at the GOAWAY is printed, not checked.
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
    // One request first, so the users share one connection from the start.
    let warm_up = client.get(&url).send().await.unwrap();
    assert_eq!(warm_up.version(), reqwest::Version::HTTP_2);
    assert_eq!(warm_up.text().await.unwrap(), "ok");
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
                let (answered, outcome) = match goose.response {
                    Ok(response) => {
                        let version = response.version();
                        let body = response.text().await;
                        let answered = version == reqwest::Version::HTTP_2
                            && body.as_ref().ok().map(String::as_str) == Some("ok");
                        (answered, format!("{version:?} {body:?}"))
                    }
                    Err(error) => (false, error.to_string()),
                };
                // A request that was not answered is recorded as failed.
                assert!(answered || !recorded_success, "{}", outcome);
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

    let failed = outcomes
        .iter()
        .flatten()
        .filter(|&&answered| !answered)
        .count();
    println!(
        "GOAWAY after {GOAWAY_AFTER} requests: {failed} of {} requests failed",
        USERS * REQUESTS
    );
    // The server sent GOAWAY, and the client then opened a new HTTP/2 connection.
    let events: Vec<String> = protocols.try_iter().collect();
    let goaway = events.iter().position(|event| event == "GOAWAY");
    assert!(
        goaway.is_some_and(|goaway| events[goaway + 1..].iter().any(|event| event == "h2")),
        "{:?}",
        events
    );
    // Every user's last request was answered.
    for user in &outcomes {
        assert_eq!(user.last(), Some(&true), "{outcomes:?}");
    }
}
