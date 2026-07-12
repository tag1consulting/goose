//! Integration tests for the live web dashboard (observe + control).
//!
//! Requires the `dashboard` crate feature (default-on). Does **not** require
//! `--report-file` — GraphData collection is gated on `--dashboard` alone.
//!
//! Control HTTP cases (design Testing § integration 1–10) live here and use
//! `#[serial]` + httpmock. Oneshot/error paths (case 10: 503 `unavailable` on
//! channel disconnect / oneshot drop / 5s timeout, and `internal` soft mapping)
//! are covered by unit tests and `dispatch_control` in `src/dashboard.rs`; no
//! extra integration case is required.

#![cfg(feature = "dashboard")]

use gumdrop::Options;
use httpmock::{Method::GET, Mock, MockServer};
use serial_test::serial;
use std::time::{Duration, Instant};

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

/// Options for control-enabled dashboard load tests.
#[derive(Clone, Debug)]
struct ControlTestOpts {
    users: usize,
    increase_rate: &'static str,
    decrease_rate: Option<&'static str>,
    /// `"0"` means unlimited maintain after ramp (preferred for control tests).
    run_time: &'static str,
    no_autostart: bool,
    control: bool,
    token: Option<&'static str>,
}

impl Default for ControlTestOpts {
    fn default() -> Self {
        Self {
            users: 2,
            increase_rate: "50",
            // No decrease_rate: with run_time "0" that would insert an immediate
            // ramp-to-zero step after increase. Omit it so the plan stays in
            // maintain until control Stop (cancel) or process abort.
            decrease_rate: None,
            run_time: "0",
            no_autostart: true,
            control: true,
            token: Some(AUTH_TOKEN),
        }
    }
}

/// Build a short-running observe-only load test with the dashboard enabled.
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

/// Build a dashboard config with control / no-autostart / custom rates.
fn build_control_config(
    server: &MockServer,
    host: &str,
    port: u16,
    opts: ControlTestOpts,
) -> GooseConfiguration {
    let mut owned: Vec<String> = vec![
        "--users".into(),
        opts.users.to_string(),
        "--increase-rate".into(),
        opts.increase_rate.into(),
        "--run-time".into(),
        opts.run_time.into(),
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
    if let Some(rate) = opts.decrease_rate {
        owned.push("--decrease-rate".into());
        owned.push(rate.into());
    }
    if opts.no_autostart {
        owned.push("--no-autostart".into());
    }
    if opts.control {
        owned.push("--dashboard-control".into());
    }
    if let Some(t) = opts.token {
        owned.push("--dashboard-auth-token".into());
        owned.push(t.to_string());
    }
    let refs: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
    GooseConfiguration::parse_args_default(&refs)
        .expect("failed to parse control dashboard test configuration")
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

fn bearer_header() -> String {
    format!("Bearer {AUTH_TOKEN}")
}

async fn post_control(
    client: &reqwest::Client,
    base: &str,
    path: &str,
    body: Option<&str>,
    with_auth: bool,
) -> reqwest::Response {
    let mut req = client
        .post(format!("{base}{path}"))
        .header("Content-Type", "application/json");
    if with_auth {
        req = req.header("Authorization", bearer_header());
    }
    if let Some(b) = body {
        req = req.body(b.to_string());
    } else {
        req = req.body("{}");
    }
    req.send().await.expect("control POST")
}

/// Fetch a snapshot, retrying transient 503s (metrics processor recycle blips).
async fn get_snapshot_json(
    client: &reqwest::Client,
    base: &str,
    token: Option<&str>,
) -> Result<serde_json::Value, String> {
    let url = match token {
        Some(t) => format!("{base}/api/v1/snapshot?token={t}"),
        None => format!("{base}/api/v1/snapshot"),
    };
    let mut req = client.get(&url);
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    let resp = req.send().await.map_err(|e| format!("snapshot GET: {e}"))?;
    let status = resp.status();
    if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        return Err("snapshot 503".into());
    }
    if status != reqwest::StatusCode::OK {
        return Err(format!("snapshot HTTP {status}"));
    }
    resp.json().await.map_err(|e| format!("snapshot json: {e}"))
}

/// Poll snapshot until `phase` is one of `phases` or `timeout` elapses.
async fn wait_for_phase(
    client: &reqwest::Client,
    base: &str,
    token: Option<&str>,
    phases: &[&str],
    timeout: Duration,
) -> serde_json::Value {
    let deadline = Instant::now() + timeout;
    let mut last = serde_json::Value::Null;
    let mut last_err = String::new();
    while Instant::now() < deadline {
        match get_snapshot_json(client, base, token).await {
            Ok(snap) => {
                last = snap;
                if let Some(phase) = last["phase"].as_str() {
                    if phases.contains(&phase) {
                        return last;
                    }
                }
            }
            Err(e) => last_err = e,
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!(
        "timed out waiting for phase in {:?}; last snapshot: {}; last_err: {}",
        phases, last, last_err
    );
}

/// Poll snapshot until `active_users` reaches `target` (or higher).
async fn wait_for_active_users(
    client: &reqwest::Client,
    base: &str,
    token: Option<&str>,
    target: u64,
    timeout: Duration,
) -> serde_json::Value {
    let deadline = Instant::now() + timeout;
    let mut last = serde_json::Value::Null;
    let mut last_err = String::new();
    while Instant::now() < deadline {
        match get_snapshot_json(client, base, token).await {
            Ok(snap) => {
                last = snap;
                if last["active_users"].as_u64().unwrap_or(0) >= target {
                    return last;
                }
            }
            Err(e) => last_err = e,
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!(
        "timed out waiting for active_users >= {}; last snapshot: {}; last_err: {}",
        target, last, last_err
    );
}

/// After Stop, decrease shuts down the metrics processor before Idle is visible
/// on `/snapshot`. Detect idle by polling Start until it is accepted.
async fn wait_until_startable(
    client: &reqwest::Client,
    base: &str,
    timeout: Duration,
) -> serde_json::Value {
    let deadline = Instant::now() + timeout;
    let mut last = serde_json::Value::Null;
    while Instant::now() < deadline {
        let resp = post_control(client, base, "/api/v1/control/start", Some("{}"), true).await;
        if resp.status() == reqwest::StatusCode::OK {
            last = resp.json().await.expect("start json");
            if last["ok"] == true {
                return last;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!(
        "timed out waiting until Start succeeds (idle); last: {}",
        last
    );
}

/// Abort a no-autostart GooseAttack that would otherwise idle forever.
async fn abort_load_test(handle: tokio::task::JoinHandle<Result<GooseMetrics, GooseError>>) {
    handle.abort();
    let _ = handle.await;
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
    assert_eq!(
        client
            .get(format!("{base}/static/chart.min.js"))
            .send()
            .await
            .unwrap()
            .status(),
        200,
        "chart.min.js must stay public when token is configured"
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

// ---------------------------------------------------------------------------
// Dashboard control HTTP integration tests (design cases 1–9; case 10 = units)
// ---------------------------------------------------------------------------

/// Case 1: control off → POST start 404; health.control_enabled false.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_control_disabled_returns_404() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");

    // Observe-only (no --dashboard-control).
    let configuration = build_dashboard_config(&server, "127.0.0.1", port, Some(AUTH_TOKEN));
    assert!(!configuration.dashboard_control);

    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load_handle = tokio::spawn(async move { goose_attack.execute().await });

    let (base, health) = wait_for_health(&[&base_v4], 80).await;
    let health_json: serde_json::Value = health.json().await.expect("health json");
    assert_eq!(health_json["control_enabled"], false);

    let client = reqwest::Client::new();
    let resp = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(
        resp.status(),
        404,
        "control off must 404 start, got {}",
        resp.status()
    );

    let _ = load_handle.await.expect("join").expect("execute");
}

/// Case 2: control always requires token on loopback.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_control_requires_token_on_loopback() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");

    let configuration =
        build_control_config(&server, "127.0.0.1", port, ControlTestOpts::default());
    assert!(configuration.dashboard_control);
    assert!(configuration.no_autostart);

    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load_handle = tokio::spawn(async move { goose_attack.execute().await });

    let (base, health) = wait_for_health(&[&base_v4], 80).await;
    let health_json: serde_json::Value = health.json().await.expect("health json");
    assert_eq!(health_json["control_enabled"], true);

    let client = reqwest::Client::new();

    // Without auth → 401.
    let unauth = post_control(&client, &base, "/api/v1/control/start", Some("{}"), false).await;
    assert_eq!(unauth.status(), 401);

    // Bearer on idle → 200 ok start.
    let started = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(started.status(), 200);
    let body: serde_json::Value = started.json().await.expect("start json");
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "start");
    assert_eq!(body["phase"], "increase");

    abort_load_test(load_handle).await;
}

/// Case 3: start → running traffic → stop decrease → idle; second start while not idle fails.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_control_start_stop() {
    let server = MockServer::start();
    let mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");

    let configuration = build_control_config(
        &server,
        "127.0.0.1",
        port,
        ControlTestOpts {
            users: 3,
            increase_rate: "50",
            ..ControlTestOpts::default()
        },
    );

    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load_handle = tokio::spawn(async move { goose_attack.execute().await });

    let (base, _) = wait_for_health(&[&base_v4], 80).await;
    let client = reqwest::Client::new();
    let token = Some(AUTH_TOKEN);

    // Idle before start.
    let idle = wait_for_phase(&client, &base, token, &["idle"], Duration::from_secs(15)).await;
    assert_eq!(idle["phase"], "idle");

    let start = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(start.status(), 200);
    let start_body: serde_json::Value = start.json().await.expect("start json");
    assert_eq!(start_body["ok"], true);
    assert_eq!(start_body["phase"], "increase");

    // Running: increase or maintain, with mock traffic.
    wait_for_phase(
        &client,
        &base,
        token,
        &["increase", "maintain"],
        Duration::from_secs(30),
    )
    .await;

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if mocks[0].calls() > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(mocks[0].calls() > 0, "expected mock traffic after start");

    // Second start while running → invalid_phase.
    let second_start =
        post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(second_start.status(), 200);
    let second_body: serde_json::Value = second_start.json().await.expect("start json");
    assert_eq!(second_body["ok"], false);
    assert_eq!(second_body["error"], "invalid_phase");

    let stop = post_control(&client, &base, "/api/v1/control/stop", Some("{}"), true).await;
    assert_eq!(stop.status(), 200);
    let stop_body: serde_json::Value = stop.json().await.expect("stop json");
    assert_eq!(stop_body["ok"], true);
    assert_eq!(stop_body["phase"], "decrease");
    assert_eq!(stop_body["target_users"], 0);

    // Stop begins cancel (decrease), not idle: Start must still be rejected.
    let during_stop = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(during_stop.status(), 200);
    let during_body: serde_json::Value = during_stop.json().await.expect("start json");
    // Either still decreasing, or already idle if cancel was instant — if idle,
    // ok would be true and wait_until_startable below is a no-op start attempt.
    if during_body["ok"] == false {
        assert_eq!(during_body["error"], "invalid_phase");
    }

    // Eventual idle after cancel ramp: metrics processor is recycled on the way
    // to idle, so snapshot may 503; detect idle by Start acceptance instead.
    let restart_body = if during_body["ok"] == true {
        during_body
    } else {
        wait_until_startable(&client, &base, Duration::from_secs(60)).await
    };
    assert_eq!(restart_body["ok"], true);
    assert_eq!(restart_body["phase"], "increase");

    abort_load_test(load_handle).await;
}

/// Case 4: while running, POST higher users with pinned high increase-rate.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_control_set_users() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");

    let configuration = build_control_config(
        &server,
        "127.0.0.1",
        port,
        ControlTestOpts {
            users: 2,
            increase_rate: "100",
            ..ControlTestOpts::default()
        },
    );

    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load_handle = tokio::spawn(async move { goose_attack.execute().await });

    let (base, _) = wait_for_health(&[&base_v4], 80).await;
    let client = reqwest::Client::new();
    let token = Some(AUTH_TOKEN);

    let start = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(start.status(), 200);
    let start_body: serde_json::Value = start.json().await.expect("start json");
    assert_eq!(start_body["ok"], true);

    // Reach at least the initial 2 users.
    wait_for_active_users(&client, &base, token, 2, Duration::from_secs(30)).await;

    let users_resp = post_control(
        &client,
        &base,
        "/api/v1/control/users",
        Some(r#"{"users":8}"#),
        true,
    )
    .await;
    assert_eq!(users_resp.status(), 200);
    let users_body: serde_json::Value = users_resp.json().await.expect("users json");
    assert_eq!(users_body["ok"], true);
    assert_eq!(users_body["command"], "users");
    assert_eq!(users_body["target_users"], 8);

    let snap = wait_for_active_users(&client, &base, token, 8, Duration::from_secs(45)).await;
    assert!(
        snap["active_users"].as_u64().unwrap_or(0) >= 8,
        "active_users must reach target 8, got {}",
        snap
    );

    abort_load_test(load_handle).await;
}

/// Case 5: second start while running → ok:false invalid_phase.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_control_start_when_running_fails() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");

    let configuration =
        build_control_config(&server, "127.0.0.1", port, ControlTestOpts::default());
    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load_handle = tokio::spawn(async move { goose_attack.execute().await });

    let (base, _) = wait_for_health(&[&base_v4], 80).await;
    let client = reqwest::Client::new();
    let token = Some(AUTH_TOKEN);

    let start = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(start.status(), 200);
    let start_body: serde_json::Value = start.json().await.expect("json");
    assert_eq!(start_body["ok"], true);

    wait_for_phase(
        &client,
        &base,
        token,
        &["increase", "maintain"],
        Duration::from_secs(30),
    )
    .await;

    let second = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(second.status(), 200);
    let body: serde_json::Value = second.json().await.expect("json");
    assert_eq!(body["ok"], false);
    assert_eq!(body["error"], "invalid_phase");
    assert_eq!(body["command"], "start");

    abort_load_test(load_handle).await;
}

/// Case 6: users validation — 0 / missing / non-integer → 400.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_control_users_validation() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");

    let configuration =
        build_control_config(&server, "127.0.0.1", port, ControlTestOpts::default());
    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load_handle = tokio::spawn(async move { goose_attack.execute().await });

    let (base, _) = wait_for_health(&[&base_v4], 80).await;
    let client = reqwest::Client::new();

    // users: 0 → 400 invalid_users.
    let zero = post_control(
        &client,
        &base,
        "/api/v1/control/users",
        Some(r#"{"users":0}"#),
        true,
    )
    .await;
    assert_eq!(zero.status(), 400);
    let zero_body: serde_json::Value = zero.json().await.expect("json");
    assert_eq!(zero_body["ok"], false);
    assert_eq!(zero_body["error"], "invalid_users");
    assert_eq!(zero_body["command"], "users");

    // missing users field → 400 invalid_users.
    let missing = post_control(&client, &base, "/api/v1/control/users", Some(r#"{}"#), true).await;
    assert_eq!(missing.status(), 400);
    let missing_body: serde_json::Value = missing.json().await.expect("json");
    assert_eq!(missing_body["error"], "invalid_users");

    // non-integer (float) → 400 invalid_users.
    let non_int = post_control(
        &client,
        &base,
        "/api/v1/control/users",
        Some(r#"{"users":1.5}"#),
        true,
    )
    .await;
    assert_eq!(non_int.status(), 400);
    let non_int_body: serde_json::Value = non_int.json().await.expect("json");
    assert_eq!(non_int_body["error"], "invalid_users");

    // non-object body → 400 bad_request.
    let bad = post_control(
        &client,
        &base,
        "/api/v1/control/users",
        Some(r#"[1]"#),
        true,
    )
    .await;
    assert_eq!(bad.status(), 400);
    let bad_body: serde_json::Value = bad.json().await.expect("json");
    assert_eq!(bad_body["error"], "bad_request");

    abort_load_test(load_handle).await;
}

/// Case 7: second stop while decreasing → ok:false invalid_phase.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_control_stop_while_decreasing() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");

    // Many users so cancel ramp is not instantaneous.
    let configuration = build_control_config(
        &server,
        "127.0.0.1",
        port,
        ControlTestOpts {
            users: 20,
            increase_rate: "100",
            ..ControlTestOpts::default()
        },
    );

    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load_handle = tokio::spawn(async move { goose_attack.execute().await });

    let (base, _) = wait_for_health(&[&base_v4], 80).await;
    let client = reqwest::Client::new();
    let token = Some(AUTH_TOKEN);

    let start = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    let start_body: serde_json::Value = start.json().await.expect("json");
    assert_eq!(start_body["ok"], true);

    wait_for_active_users(&client, &base, token, 10, Duration::from_secs(30)).await;

    let stop = post_control(&client, &base, "/api/v1/control/stop", Some("{}"), true).await;
    assert_eq!(stop.status(), 200);
    let stop_body: serde_json::Value = stop.json().await.expect("json");
    assert_eq!(stop_body["ok"], true);
    assert_eq!(stop_body["phase"], "decrease");

    // Immediate second stop — still not Increase/Maintain.
    let stop2 = post_control(&client, &base, "/api/v1/control/stop", Some("{}"), true).await;
    assert_eq!(stop2.status(), 200);
    let stop2_body: serde_json::Value = stop2.json().await.expect("json");
    assert_eq!(stop2_body["ok"], false);
    assert_eq!(stop2_body["error"], "invalid_phase");
    assert_eq!(stop2_body["command"], "stop");

    abort_load_test(load_handle).await;
}

/// Case 8: Users during decrease succeeds (Controller parity).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_control_users_while_decreasing() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");

    let configuration = build_control_config(
        &server,
        "127.0.0.1",
        port,
        ControlTestOpts {
            users: 20,
            increase_rate: "100",
            ..ControlTestOpts::default()
        },
    );

    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load_handle = tokio::spawn(async move { goose_attack.execute().await });

    let (base, _) = wait_for_health(&[&base_v4], 80).await;
    let client = reqwest::Client::new();
    let token = Some(AUTH_TOKEN);

    let start = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    let start_body: serde_json::Value = start.json().await.expect("json");
    assert_eq!(start_body["ok"], true);

    wait_for_active_users(&client, &base, token, 10, Duration::from_secs(30)).await;

    let stop = post_control(&client, &base, "/api/v1/control/stop", Some("{}"), true).await;
    let stop_body: serde_json::Value = stop.json().await.expect("json");
    assert_eq!(stop_body["ok"], true);
    assert_eq!(stop_body["phase"], "decrease");

    // Users is allowed during Decrease.
    let users = post_control(
        &client,
        &base,
        "/api/v1/control/users",
        Some(r#"{"users":5}"#),
        true,
    )
    .await;
    assert_eq!(users.status(), 200);
    let users_body: serde_json::Value = users.json().await.expect("json");
    assert_eq!(
        users_body["ok"], true,
        "users during decrease: {}",
        users_body
    );
    assert_eq!(users_body["command"], "users");
    assert_eq!(users_body["target_users"], 5);

    abort_load_test(load_handle).await;
}

/// Case 9: open SSE, then Start; control still completes (shared flume FIFO).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_control_with_active_sse() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");

    let configuration =
        build_control_config(&server, "127.0.0.1", port, ControlTestOpts::default());
    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load_handle = tokio::spawn(async move { goose_attack.execute().await });

    let (base, _) = wait_for_health(&[&base_v4], 80).await;
    let client = reqwest::Client::new();

    // Hold an open SSE stream (shares the dashboard request channel with control).
    let mut sse = client
        .get(format!("{base}/api/v1/events?token={AUTH_TOKEN}"))
        .send()
        .await
        .expect("SSE connect");
    assert_eq!(sse.status(), 200);
    let ct = sse
        .headers()
        .get("content-type")
        .expect("content-type")
        .to_str()
        .unwrap();
    assert!(
        ct.contains("text/event-stream"),
        "SSE content-type was {}",
        ct
    );

    // Drain at least one snapshot event so the hub is active.
    let sse_deadline = Instant::now() + Duration::from_secs(5);
    let mut saw_snapshot = false;
    while Instant::now() < sse_deadline {
        match tokio::time::timeout(Duration::from_millis(500), sse.chunk()).await {
            Ok(Ok(Some(chunk))) => {
                let text = String::from_utf8_lossy(&chunk);
                if text.contains("snapshot") {
                    saw_snapshot = true;
                    break;
                }
            }
            Ok(Ok(None)) => break,
            Ok(Err(_)) => break,
            Err(_) => continue,
        }
    }
    assert!(saw_snapshot, "expected at least one SSE snapshot event");

    // Start must still complete while SSE is open.
    let start = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(start.status(), 200);
    let body: serde_json::Value = start.json().await.expect("json");
    assert_eq!(body["ok"], true);
    assert_eq!(body["phase"], "increase");

    // Drop SSE and shut down.
    drop(sse);
    abort_load_test(load_handle).await;
}
