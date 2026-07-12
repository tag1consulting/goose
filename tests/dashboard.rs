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

/// Reserve an ephemeral loopback port for the dashboard.
///
/// Goose's configure path treats `dashboard_port == 0` as "unset" (fills 5118),
/// so tests cannot pass `--dashboard-port 0` through `execute()`. Instead we
/// sample a free port here. The listener is dropped before Goose binds, which
/// is a small TOCTOU window; tests are `#[serial]` and retry reservation to
/// keep CI stable. Unit tests in `src/dashboard.rs` cover true ephemeral bind
/// (port 0) via direct `setup_dashboard` without configure().
fn reserve_port() -> u16 {
    for _ in 0..32 {
        if let Ok(listener) = std::net::TcpListener::bind("127.0.0.1:0") {
            let port = listener.local_addr().expect("local_addr").port();
            drop(listener);
            if port > 1024 {
                return port;
            }
        }
    }
    panic!("could not reserve an ephemeral TCP port for dashboard tests");
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

/// Build a short-running load test with the dashboard enabled.
/// Does not set `--report-file` — dashboard alone must be sufficient.
fn build_dashboard_config(
    server: &MockServer,
    host: &str,
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
        host.to_string(),
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

async fn wait_for_health(bases: &[&str], attempts: u32) -> (String, reqwest::Response) {
    let client = reqwest::Client::new();
    for i in 0..attempts {
        for base in bases {
            let url = format!("{base}/api/v1/health");
            if let Ok(resp) = client.get(&url).send().await {
                if resp.status().is_success() {
                    return ((*base).to_string(), resp);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100 + i as u64 * 25)).await;
    }
    panic!("dashboard health endpoint not ready for bases {:?}", bases);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_dashboard_loopback_no_token() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");

    let configuration = build_dashboard_config(&server, "127.0.0.1", port, None);
    assert!(configuration.dashboard);
    assert!(configuration.dashboard_auth_token.is_empty());
    assert!(configuration.report_file.is_empty());

    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);

    let load_handle = tokio::spawn(async move { goose_attack.execute().await });

    // Wait for the dashboard to accept connections.
    let (base, health) = wait_for_health(&[&base_v4], 80).await;
    let health_json: serde_json::Value = health.json().await.expect("health json");
    assert_eq!(health_json["ok"], true);
    assert!(!health_json["version"].as_str().unwrap().is_empty());

    // Static shell is public.
    let client = reqwest::Client::new();
    let index = client.get(format!("{base}/")).send().await.expect("GET /");
    assert_eq!(index.status(), 200);
    let index_body = index.text().await.unwrap();
    assert!(index_body.contains("Goose"));
    assert!(index_body.contains("/static/app.js"));
    assert!(
        index_body.contains("/static/chart.min.js"),
        "UI must load vendored Chart.js"
    );

    let app_js = client
        .get(format!("{base}/static/app.js"))
        .send()
        .await
        .expect("GET /static/app.js");
    assert_eq!(app_js.status(), 200);
    let js_body = app_js.text().await.unwrap();
    assert!(js_body.contains("snapshot"));
    assert!(
        js_body.contains("EventSource"),
        "UI must use EventSource for live SSE updates"
    );
    assert!(
        js_body.contains("/api/v1/events"),
        "UI must wire EventSource to /api/v1/events"
    );
    assert!(
        js_body.contains("series"),
        "UI must render SeriesWindow charts"
    );

    let app_css = client
        .get(format!("{base}/static/app.css"))
        .send()
        .await
        .expect("GET /static/app.css");
    assert_eq!(app_css.status(), 200);

    let chart_js = client
        .get(format!("{base}/static/chart.min.js"))
        .send()
        .await
        .expect("GET /static/chart.min.js");
    assert_eq!(chart_js.status(), 200);
    let chart_body = chart_js.text().await.unwrap();
    assert!(
        chart_body.contains("Chart"),
        "vendored chart.min.js must expose Chart"
    );

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
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");

    let configuration = build_dashboard_config(&server, "127.0.0.1", port, Some(AUTH_TOKEN));
    assert_eq!(configuration.dashboard_auth_token, AUTH_TOKEN);

    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);

    let load_handle = tokio::spawn(async move { goose_attack.execute().await });

    let (base, _) = wait_for_health(&[&base_v4], 80).await;
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

    // Events SSE without token → 401.
    let events_unauth = client
        .get(format!("{base}/api/v1/events"))
        .send()
        .await
        .expect("unauth events");
    assert_eq!(events_unauth.status(), 401);

    // Events SSE with query token → 200 + event-stream.
    let events_ok = client
        .get(format!("{base}/api/v1/events?token={AUTH_TOKEN}"))
        .send()
        .await
        .expect("events token");
    assert_eq!(events_ok.status(), 200);
    let events_ct = events_ok
        .headers()
        .get("content-type")
        .expect("content-type")
        .to_str()
        .unwrap();
    assert!(
        events_ct.contains("text/event-stream"),
        "SSE content-type was {}",
        events_ct
    );

    let _ = load_handle.await.expect("join").expect("execute");
}

/// Regression: `--dashboard-host localhost` must bind via ToSocketAddrs
/// (SocketAddr::parse rejects hostnames).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_dashboard_binds_localhost() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    // `localhost` may resolve to 127.0.0.1 and/or ::1; try both families.
    let base_v4 = format!("http://127.0.0.1:{port}");
    let base_name = format!("http://localhost:{port}");
    let base_v6 = format!("http://[::1]:{port}");

    let configuration = build_dashboard_config(&server, "localhost", port, None);
    assert_eq!(configuration.dashboard_host, "localhost");

    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);

    let load_handle = tokio::spawn(async move { goose_attack.execute().await });

    let (base, _) = wait_for_health(&[&base_name, &base_v4, &base_v6], 80).await;

    let client = reqwest::Client::new();
    let snapshot = client
        .get(format!("{base}/api/v1/snapshot"))
        .send()
        .await
        .expect("snapshot after localhost bind");
    assert_eq!(
        snapshot.status(),
        200,
        "dashboard must be listening when host is localhost"
    );

    let _ = load_handle.await.expect("join").expect("execute");
}
