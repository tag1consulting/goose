//! Integration tests for the read-only live web dashboard.
//!
//! Requires the `dashboard` crate feature (default-on). Does **not** require
//! `--report-file` — GraphData collection is gated on `--dashboard` alone.

#![cfg(feature = "dashboard")]

use gumdrop::Options;
use httpmock::{Method::GET, Mock, MockServer};
use serial_test::serial;
use std::time::Duration;

use goose::config::GooseConfiguration;
use goose::prelude::*;

mod common;

const INDEX_PATH: &str = "/";
const AUTH_TOKEN: &str = "test-dashboard-token";

/// Pick an ephemeral free TCP port on loopback.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral")
        .local_addr()
        .expect("local_addr")
        .port()
}

fn setup_mock_endpoints(server: &MockServer) -> Vec<Mock<'_>> {
    vec![server.mock(|when, then| {
        when.method(GET).path(INDEX_PATH);
        then.status(200);
    })]
}

async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

fn get_transactions() -> Scenario {
    scenario!("DashboardLoad").register_transaction(transaction!(get_index))
}

/// Build a short-running load test with the dashboard enabled on a free port.
/// Does not set `--report-file` — dashboard alone must be sufficient.
fn build_dashboard_config(
    server: &MockServer,
    port: u16,
    token: Option<&str>,
) -> GooseConfiguration {
    let mut owned: Vec<String> = vec![
        "--users".into(),
        "1".into(),
        "--increase-rate".into(),
        "1".into(),
        "--run-time".into(),
        "8".into(),
        "--no-telnet".into(),
        "--no-websocket".into(),
        "--dashboard".into(),
        "--dashboard-host".into(),
        "127.0.0.1".into(),
        "--dashboard-port".into(),
        port.to_string(),
        "--co-mitigation".into(),
        "disabled".into(),
        "--quiet".into(),
        "--host".into(),
        server.base_url(),
    ];
    if let Some(t) = token {
        owned.push("--dashboard-auth-token".into());
        owned.push(t.to_string());
    }
    let refs: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
    GooseConfiguration::parse_args_default(&refs)
        .expect("failed to parse dashboard test configuration")
}

async fn wait_for_health(base: &str, attempts: u32) -> reqwest::Response {
    let client = reqwest::Client::new();
    let url = format!("{base}/api/v1/health");
    for i in 0..attempts {
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => return resp,
            _ => {
                tokio::time::sleep(Duration::from_millis(100 + i as u64 * 25)).await;
            }
        }
    }
    panic!("dashboard health endpoint not ready at {}", url);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_dashboard_loopback_no_token() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = free_port();
    let base = format!("http://127.0.0.1:{port}");

    let configuration = build_dashboard_config(&server, port, None);
    assert!(configuration.dashboard);
    assert!(configuration.dashboard_auth_token.is_empty());
    assert!(configuration.report_file.is_empty());

    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);

    let load_handle = tokio::spawn(async move { goose_attack.execute().await });

    // Wait for the dashboard to accept connections.
    let health = wait_for_health(&base, 80).await;
    let health_json: serde_json::Value = health.json().await.expect("health json");
    assert_eq!(health_json["ok"], true);
    assert!(!health_json["version"].as_str().unwrap().is_empty());

    // Static shell is public.
    let client = reqwest::Client::new();
    let index = client.get(format!("{base}/")).send().await.expect("GET /");
    assert_eq!(index.status(), 200);
    let index_body = index.text().await.unwrap();
    assert!(index_body.contains("Goose Dashboard"));
    assert!(index_body.contains("/static/app.js"));

    let app_js = client
        .get(format!("{base}/static/app.js"))
        .send()
        .await
        .expect("GET /static/app.js");
    assert_eq!(app_js.status(), 200);
    let js_body = app_js.text().await.unwrap();
    assert!(js_body.contains("snapshot"));

    let app_css = client
        .get(format!("{base}/static/app.css"))
        .send()
        .await
        .expect("GET /static/app.css");
    assert_eq!(app_css.status(), 200);

    // Snapshot without token on loopback → 200.
    let snapshot = client
        .get(format!("{base}/api/v1/snapshot"))
        .send()
        .await
        .expect("GET /api/v1/snapshot");
    assert_eq!(
        snapshot.status(),
        200,
        "loopback without token must allow snapshot"
    );
    let snap: serde_json::Value = snapshot.json().await.expect("snapshot json");
    assert_eq!(snap["version"], 1);
    assert!(snap["phase"].as_str().is_some());
    assert!(snap.get("aggregate").is_some());
    assert!(snap.get("requests").is_some());
    assert!(snap.get("errors").is_some());
    assert!(snap.get("series").is_some());
    assert!(snap.get("flags").is_some());

    // CSP header present.
    let head = client
        .get(format!("{base}/"))
        .send()
        .await
        .expect("GET / for CSP");
    let csp = head
        .headers()
        .get("content-security-policy")
        .expect("CSP header")
        .to_str()
        .unwrap();
    assert!(csp.contains("default-src 'self'"));
    assert!(csp.contains("script-src 'self'"));

    // Let the load test finish.
    let _metrics = load_handle.await.expect("join").expect("load test execute");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_dashboard_token_auth() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = free_port();
    let base = format!("http://127.0.0.1:{port}");

    let configuration = build_dashboard_config(&server, port, Some(AUTH_TOKEN));
    assert_eq!(configuration.dashboard_auth_token, AUTH_TOKEN);

    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);

    let load_handle = tokio::spawn(async move { goose_attack.execute().await });

    let _ = wait_for_health(&base, 80).await;
    let client = reqwest::Client::new();

    // Health remains public with token configured.
    let health = client
        .get(format!("{base}/api/v1/health"))
        .send()
        .await
        .expect("health");
    assert_eq!(health.status(), 200);

    // Shell / static remain public.
    assert_eq!(
        client
            .get(format!("{base}/"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .get(format!("{base}/static/app.js"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );

    // Snapshot without token → 401, no metrics body.
    let unauth = client
        .get(format!("{base}/api/v1/snapshot"))
        .send()
        .await
        .expect("unauth snapshot");
    assert_eq!(unauth.status(), 401);
    let unauth_body = unauth.text().await.unwrap();
    assert!(
        !unauth_body.contains("aggregate"),
        "401 must not leak metrics body"
    );

    // Snapshot with wrong token → 401.
    let wrong = client
        .get(format!("{base}/api/v1/snapshot?token=wrong"))
        .send()
        .await
        .expect("wrong token");
    assert_eq!(wrong.status(), 401);

    // Snapshot with query token → 200.
    let with_query = client
        .get(format!("{base}/api/v1/snapshot?token={AUTH_TOKEN}"))
        .send()
        .await
        .expect("query token");
    assert_eq!(with_query.status(), 200);
    let snap: serde_json::Value = with_query.json().await.unwrap();
    assert_eq!(snap["version"], 1);

    // Snapshot with Bearer header → 200.
    let with_header = client
        .get(format!("{base}/api/v1/snapshot"))
        .header("Authorization", format!("Bearer {AUTH_TOKEN}"))
        .send()
        .await
        .expect("bearer token");
    assert_eq!(with_header.status(), 200);

    let _ = load_handle.await.expect("join").expect("execute");
}
