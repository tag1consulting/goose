//! Integration tests for the live web dashboard (observe + control).
//!
//! Requires the `dashboard` crate feature (`--features dashboard`). Does **not**
//! require `--report-file` — GraphData collection is gated on `--dashboard` alone.
//!
//! Control HTTP cases (design Testing § integration 1–9) live here and use
//! `#[serial]` + httpmock. Case 10 (oneshot disconnect / drop / timeout → 503
//! `unavailable`, and parent `internal` → 200) is covered by the unit test
//! `control_oneshot_error_paths` in `src/dashboard.rs`.

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
    /// `--test-plan`; replaces `users`, `increase_rate`, `decrease_rate` and
    /// `run_time`, which Goose refuses alongside a test plan.
    test_plan: Option<&'static str>,
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
            test_plan: None,
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
    let mut owned: Vec<String> = match opts.test_plan {
        Some(plan) => vec!["--test-plan".into(), plan.into()],
        None => vec![
            "--users".into(),
            opts.users.to_string(),
            "--increase-rate".into(),
            opts.increase_rate.into(),
            "--run-time".into(),
            opts.run_time.into(),
        ],
    };
    owned.extend([
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
    ]);
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

/// Abort a no-autostart GooseAttack that would otherwise idle forever.
///
/// Prefer [`LoadTestGuard`] in tests so panics still abort the task.
async fn abort_load_test(handle: tokio::task::JoinHandle<Result<GooseMetrics, GooseError>>) {
    handle.abort();
    let _ = handle.await;
}

/// RAII guard: aborts the load-test task on drop (including panic paths).
///
/// Tokio detaches `JoinHandle` on drop without aborting; with `run_time=0` /
/// `--no-autostart` that can leave a dashboard bound until process exit.
struct LoadTestGuard {
    handle: Option<tokio::task::JoinHandle<Result<GooseMetrics, GooseError>>>,
}

impl LoadTestGuard {
    fn new(handle: tokio::task::JoinHandle<Result<GooseMetrics, GooseError>>) -> Self {
        Self {
            handle: Some(handle),
        }
    }

    /// Abort and await the task (happy-path cleanup).
    async fn abort(mut self) {
        if let Some(handle) = self.handle.take() {
            abort_load_test(handle).await;
        }
    }

    /// Wait for natural completion (observe-only short run_time tests).
    async fn join(mut self) -> Result<GooseMetrics, GooseError> {
        let handle = self.handle.take().expect("load test handle");
        handle.await.expect("join load test")
    }
}

impl Drop for LoadTestGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
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

/// Send one `GET path` with the given `Host` on a new connection and return
/// the raw response.
async fn get_with_host(port: u16, path: &str, host: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let request = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut buf)).await;
    String::from_utf8_lossy(&buf).into_owned()
}

/// Regression for #697: on a loopback bind with no token, a page that rebinds
/// its own domain to 127.0.0.1 gets 403 instead of metrics, and every response
/// carries `no-store`, `nosniff` and `no-referrer`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_dashboard_refuses_rebound_host() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");

    let configuration = build_dashboard_config(&server, "127.0.0.1", port, None);
    assert!(configuration.dashboard_auth_token.is_empty());
    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load_handle = tokio::spawn(async move { goose_attack.execute().await });
    wait_for_health(&[&base_v4], 80).await;

    for path in ["/api/v1/snapshot", "/api/v1/events", "/"] {
        let reply = get_with_host(port, path, &format!("rebind.example:{port}")).await;
        assert!(
            reply.starts_with("HTTP/1.1 403"),
            "{} with a rebound Host must get 403, got {:?}",
            path,
            reply
        );
    }

    let reply = get_with_host(port, "/api/v1/snapshot", &format!("localhost:{port}")).await;
    assert!(
        reply.starts_with("HTTP/1.1 200"),
        "a loopback Host must be served, got {:?}",
        reply
    );
    for line in [
        "cache-control: no-store\r\n",
        "x-content-type-options: nosniff\r\n",
        "referrer-policy: no-referrer\r\n",
    ] {
        assert!(
            reply.contains(line),
            "snapshot must carry {:?}: {:?}",
            line,
            reply
        );
    }

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
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));

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

    let _ = load.join().await.expect("execute");
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
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));

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

    load.abort().await;
}

/// Case 3: start → running traffic → stop decrease → idle; second start while not idle fails.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_control_start_stop() {
    let server = MockServer::start();
    let mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");

    // Many users so the cancel ramp is wide enough to hard-assert Start rejection
    // while still decreasing (cancel plan is (active, elapsed)→(0, 0) but one user
    // exits per main-loop iteration).
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
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));

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

    // Running: increase or maintain, with mock traffic and enough users hatched.
    wait_for_phase(
        &client,
        &base,
        token,
        &["increase", "maintain"],
        Duration::from_secs(30),
    )
    .await;
    wait_for_active_users(&client, &base, token, 10, Duration::from_secs(30)).await;

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

    // Pipeline Stop then Start into the shared flume while Maintain sleeps
    // (~500 ms). One try_recv per main-loop iteration ⇒ Stop is handled first
    // (phase→decrease), then Start is dequeued on the next iteration still in
    // decrease. Cancel uses a 0 ms ramp, so awaiting Stop fully before Start
    // often races past Idle before the second request is processed.
    let stop_client = client.clone();
    let start_client = client.clone();
    let stop_base = base.clone();
    let start_base = base.clone();
    let (stop, during_stop) = tokio::join!(
        post_control(
            &stop_client,
            &stop_base,
            "/api/v1/control/stop",
            Some("{}"),
            true
        ),
        async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            post_control(
                &start_client,
                &start_base,
                "/api/v1/control/start",
                Some("{}"),
                true,
            )
            .await
        }
    );
    assert_eq!(stop.status(), 200);
    let stop_body: serde_json::Value = stop.json().await.expect("stop json");
    assert_eq!(stop_body["ok"], true);
    assert_eq!(stop_body["phase"], "decrease");
    assert_eq!(stop_body["target_users"], 0);

    assert_eq!(during_stop.status(), 200);
    let during_body: serde_json::Value = during_stop.json().await.expect("start json");
    assert_eq!(
        during_body["ok"], false,
        "Start must fail while decreasing, got {}",
        during_body
    );
    assert_eq!(during_body["error"], "invalid_phase");

    // After the cancel ramp completes the main loop returns to Idle and still
    // serves snapshots from local metrics (processor is recycled until Start).
    let idle_after =
        wait_for_phase(&client, &base, token, &["idle"], Duration::from_secs(60)).await;
    assert_eq!(idle_after["phase"], "idle");
    assert_eq!(
        idle_after["active_users"].as_u64().unwrap_or(u64::MAX),
        0,
        "idle after stop must report zero active users, got {idle_after}"
    );

    // Start must work again from idle (SPA re-enable path).
    let restart = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(restart.status(), 200);
    let restart_body: serde_json::Value = restart.json().await.expect("restart json");
    assert_eq!(restart_body["ok"], true);
    assert_eq!(restart_body["phase"], "increase");

    load.abort().await;
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
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));

    let (base, _) = wait_for_health(&[&base_v4], 80).await;
    let client = reqwest::Client::new();
    let token = Some(AUTH_TOKEN);

    let start = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(start.status(), 200);
    let start_body: serde_json::Value = start.json().await.expect("start json");
    assert_eq!(start_body["ok"], true);

    // Reach the initial 2 users and settle into maintain before reconfiguring
    // (avoids racing the first Increase→Maintain transition).
    wait_for_active_users(&client, &base, token, 2, Duration::from_secs(30)).await;
    wait_for_phase(
        &client,
        &base,
        token,
        &["maintain"],
        Duration::from_secs(30),
    )
    .await;

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
    assert_eq!(
        users_body["phase"], "increase",
        "set-users while running must enter increase, got {users_body}"
    );

    let snap = wait_for_active_users(&client, &base, token, 8, Duration::from_secs(45)).await;
    assert!(
        snap["active_users"].as_u64().unwrap_or(0) >= 8,
        "active_users must reach target 8, got {}",
        snap
    );

    load.abort().await;
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
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));

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

    load.abort().await;
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
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));

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

    load.abort().await;
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
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));

    let (base, _) = wait_for_health(&[&base_v4], 80).await;
    let client = reqwest::Client::new();
    let token = Some(AUTH_TOKEN);

    let start = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    let start_body: serde_json::Value = start.json().await.expect("json");
    assert_eq!(start_body["ok"], true);

    wait_for_active_users(&client, &base, token, 10, Duration::from_secs(30)).await;

    // Pipeline two Stops into the flume during Maintain sleep so the second is
    // dequeued while phase is still decrease (see test_control_start_stop).
    let stop1_client = client.clone();
    let stop2_client = client.clone();
    let stop1_base = base.clone();
    let stop2_base = base.clone();
    let (stop, stop2) = tokio::join!(
        post_control(
            &stop1_client,
            &stop1_base,
            "/api/v1/control/stop",
            Some("{}"),
            true
        ),
        async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            post_control(
                &stop2_client,
                &stop2_base,
                "/api/v1/control/stop",
                Some("{}"),
                true,
            )
            .await
        }
    );
    assert_eq!(stop.status(), 200);
    let stop_body: serde_json::Value = stop.json().await.expect("json");
    assert_eq!(stop_body["ok"], true);
    assert_eq!(stop_body["phase"], "decrease");

    assert_eq!(stop2.status(), 200);
    let stop2_body: serde_json::Value = stop2.json().await.expect("json");
    assert_eq!(stop2_body["ok"], false);
    assert_eq!(stop2_body["error"], "invalid_phase");
    assert_eq!(stop2_body["command"], "stop");
    assert_eq!(
        stop2_body["phase"], "decrease",
        "second stop must still report decrease phase, got {}",
        stop2_body
    );

    load.abort().await;
}

/// Case 8: Users sent right behind a Stop never turns the Stop back into a
/// running load test. Either Users lands during the cancel ramp and is
/// refused, or it lands once the run is idle and only reconfigures it; the run
/// ends idle with no users both ways. The refusal itself is covered
/// deterministically by `test_set_users_refused_while_stopping` in `lib.rs`.
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
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));

    let (base, _) = wait_for_health(&[&base_v4], 80).await;
    let client = reqwest::Client::new();
    let token = Some(AUTH_TOKEN);

    let start = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    let start_body: serde_json::Value = start.json().await.expect("json");
    assert_eq!(start_body["ok"], true, "start: {}", start_body);

    wait_for_active_users(&client, &base, token, 10, Duration::from_secs(30)).await;

    // Send Users right behind Stop. Both usually reach the main loop in the
    // same batch (it sleeps up to 500ms in Maintain), which exercises the
    // refusal; when they do not, Users lands on an idle run. Both outcomes are
    // valid, so the test does not depend on which one happens.
    let stop_client = client.clone();
    let users_client = client.clone();
    let stop_base = base.clone();
    let users_base = base.clone();
    let (stop, users) = tokio::join!(
        post_control(
            &stop_client,
            &stop_base,
            "/api/v1/control/stop",
            Some("{}"),
            true
        ),
        async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            post_control(
                &users_client,
                &users_base,
                "/api/v1/control/users",
                Some(r#"{"users":15}"#),
                true,
            )
            .await
        }
    );
    let stop_body: serde_json::Value = stop.json().await.expect("json");
    assert_eq!(stop_body["ok"], true, "stop: {}", stop_body);
    assert_eq!(stop_body["phase"], "decrease");

    assert_eq!(users.status(), 200);
    let users_body: serde_json::Value = users.json().await.expect("json");
    assert_eq!(users_body["command"], "users");
    match users_body["phase"].as_str() {
        Some("decrease") => {
            assert_eq!(
                users_body["ok"], false,
                "users during a stop must be refused: {}",
                users_body
            );
            assert_eq!(users_body["error"], "invalid_phase");
            assert!(
                users_body["message"]
                    .as_str()
                    .is_some_and(|m| m.contains("stopping")),
                "message must say the load test is stopping: {}",
                users_body
            );
            assert!(users_body["target_users"].is_null());
        }
        Some("idle") => {
            assert_eq!(
                users_body["ok"], true,
                "users while idle reconfigures: {}",
                users_body
            );
        }
        _ => panic!("users must land in decrease or idle: {}", users_body),
    }

    // The cancel runs to completion either way: idle with no users. Before
    // the fix, a Users during the cancel ended in maintain and this timed out.
    let idle = wait_for_phase(&client, &base, token, &["idle"], Duration::from_secs(30)).await;
    assert_eq!(idle["active_users"], 0);
    assert_eq!(idle["stopping"], false, "idle ends the cancel: {}", idle);

    load.abort().await;
}

/// Case 8b: a test plan's own ramp down is not a cancel. The snapshot does not
/// report it as stopping, and Users is accepted and takes effect.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_control_users_during_plan_ramp_down() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");

    // 10 users in 1s, hold 1s, then one user off every 3s.
    let configuration = build_control_config(
        &server,
        "127.0.0.1",
        port,
        ControlTestOpts {
            test_plan: Some("10,1s;10,1s;0,30s"),
            ..ControlTestOpts::default()
        },
    );

    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));

    let (base, _) = wait_for_health(&[&base_v4], 80).await;
    let client = reqwest::Client::new();
    let token = Some(AUTH_TOKEN);

    let start = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    let start_body: serde_json::Value = start.json().await.expect("json");
    assert_eq!(start_body["ok"], true, "start: {}", start_body);

    let ramp_down = wait_for_phase(
        &client,
        &base,
        token,
        &["decrease"],
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(
        ramp_down["stopping"], false,
        "a plan's ramp down is not a cancel: {}",
        ramp_down
    );

    let users = post_control(
        &client,
        &base,
        "/api/v1/control/users",
        Some(r#"{"users":4}"#),
        true,
    )
    .await;
    assert_eq!(users.status(), 200);
    let users_body: serde_json::Value = users.json().await.expect("json");
    assert_eq!(
        users_body["ok"], true,
        "users during a plan ramp down: {}",
        users_body
    );
    assert_eq!(users_body["command"], "users");
    assert_eq!(users_body["phase"], "decrease");
    assert_eq!(users_body["target_users"], 4);

    // The new count replaces the rest of the plan: hold 4 users.
    let held = wait_for_phase(
        &client,
        &base,
        token,
        &["maintain"],
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(held["target_users"], 4, "maintain after users: {}", held);
    assert_eq!(held["stopping"], false);

    load.abort().await;
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
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));

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
    load.abort().await;
}

/// Batched request metrics must reach the dashboard charts without
/// `--report-file`: the requests per second series has nonzero samples while
/// the load test runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_dashboard_series_without_report_file() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");

    let mut configuration = build_dashboard_config(&server, "127.0.0.1", port, None);
    configuration.run_time = "5".to_string();
    assert!(configuration.report_file.is_empty());

    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));

    let (base, _health) = wait_for_health(&[&base_v4], 80).await;
    let client = reqwest::Client::new();

    // Poll until the requests per second series shows load or the run ends.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut rps_sum = 0.0;
    while Instant::now() < deadline {
        if let Ok(snap) = get_snapshot_json(&client, &base, None).await {
            rps_sum = snap["series"]["rps"]
                .as_array()
                .map(|rps| rps.iter().filter_map(|v| v.as_f64()).sum())
                .unwrap_or(0.0);
            if rps_sum > 0.0 {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        rps_sum > 0.0,
        "requests per second series must have nonzero samples without --report-file"
    );

    let _metrics = load.join().await.expect("load test execute");
}

/// Send one keep-alive `GET /api/v1/health` on `stream` and return the
/// response, or `None` if the connection is closed or gives no complete
/// response within two seconds.
async fn health_on(stream: &mut tokio::net::TcpStream) -> Option<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let request = "GET /api/v1/health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
    stream.write_all(request.as_bytes()).await.ok()?;
    let mut buf = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let text = String::from_utf8_lossy(&buf).into_owned();
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            let length = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length: "))
                .and_then(|v| v.trim().parse::<usize>().ok())?;
            if body.len() >= length {
                return Some(text);
            }
        }
        let mut chunk = [0u8; 1024];
        let left = deadline.checked_duration_since(Instant::now())?;
        match tokio::time::timeout(left, stream.read(&mut chunk)).await {
            Ok(Ok(n)) if n > 0 => buf.extend_from_slice(&chunk[..n]),
            _ => return None,
        }
    }
}

/// Assert that nothing on `port` answers: a new connection is refused, the
/// keep-alive connection gets no response, and the port can be bound again.
async fn assert_dashboard_stopped(port: u16, keep_alive: &mut tokio::net::TcpStream) {
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_err(),
        "dashboard must not accept connections after execute() returns"
    );
    assert_eq!(
        health_on(keep_alive).await,
        None,
        "keep-alive connection must not be served after execute() returns"
    );
    drop(
        tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .expect("dashboard port must be free after execute() returns"),
    );
}

/// The receiving end of the gate `wait_for_test_ready` waits on; installed
/// fresh for each attack by `arm_test_ready`, so no state leaks between tests.
static TEST_READY: std::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>> =
    std::sync::Mutex::new(None);

/// Arm the gate for the next attack and return the sender that opens it.
/// Pair with `transaction!(wait_for_test_ready)` as the attack's test_start.
fn arm_test_ready() -> tokio::sync::oneshot::Sender<()> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    *TEST_READY.lock().expect("test ready gate") = Some(rx);
    tx
}

/// test_start transaction that holds the attack until the test opens the
/// gate, so the attack cannot finish (or fail) before the test has connected
/// to the dashboard, however slow the runner. A dropped sender (the test
/// panicked) also opens it.
async fn wait_for_test_ready(_user: &mut GooseUser) -> TransactionResult {
    let ready = TEST_READY
        .lock()
        .expect("test ready gate")
        .take()
        .expect("arm_test_ready() before the attack starts");
    let _ = ready.await;
    Ok(())
}

/// Run one short attack on `port`, open a keep-alive connection to the
/// dashboard while it runs, and return that connection after `execute()`.
async fn attack_with_keep_alive(server: &MockServer, port: u16) -> tokio::net::TcpStream {
    let mut configuration = build_dashboard_config(server, "127.0.0.1", port, None);
    configuration.run_time = "1".to_string();
    let ready = arm_test_ready();
    let start = transaction!(wait_for_test_ready);
    let goose_attack =
        common::build_load_test(configuration, vec![get_transactions()], Some(&start), None);
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));

    let base = format!("http://127.0.0.1:{port}");
    let _ = wait_for_health(&[&base], 80).await;
    let mut keep_alive = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect to dashboard");
    let first = health_on(&mut keep_alive)
        .await
        .expect("health answers during the attack");
    assert!(first.starts_with("HTTP/1.1 200"), "{}", first);

    ready.send(()).expect("attack waits in test_start");
    let _metrics = load.join().await.expect("load test execute");
    keep_alive
}

/// Regression for #695: two attacks back to back on one port in one
/// runtime. The second binds, and the dashboard stops answering when each
/// `execute()` returns, on a new connection and on a keep-alive one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_dashboard_stops_when_execute_returns() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();

    let mut keep_alive = attack_with_keep_alive(&server, port).await;
    assert_dashboard_stopped(port, &mut keep_alive).await;

    let mut keep_alive = attack_with_keep_alive(&server, port).await;
    assert_dashboard_stopped(port, &mut keep_alive).await;
}

/// Write a request whose body never arrives, so the dashboard connection is
/// busy (the handler waits for the body) rather than idle.
async fn stalled_request(port: u16) -> tokio::net::TcpStream {
    use tokio::io::AsyncWriteExt;
    let mut stalled = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect to dashboard");
    stalled
        .write_all(
            format!(
                "POST /api/v1/control/users HTTP/1.1\r\nHost: 127.0.0.1\r\n\
                 Authorization: {}\r\nContent-Length: 100\r\n\r\n{{",
                bearer_header()
            )
            .as_bytes(),
        )
        .await
        .expect("write partial request");
    stalled
}

/// Mirrors `SERVER_DRAIN_TIMEOUT` in src/dashboard.rs (crate private): how
/// long the server gives a busy connection before closing it.
const DASHBOARD_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

/// Assert `stalled` was closed before `execute()` returned, i.e. `execute()`
/// waited for the drain.
///
/// If `execute()` returned without waiting, the server would close a busy
/// connection only at the drain deadline, a full `DASHBOARD_DRAIN_TIMEOUT`
/// after `execute()` returned, and a timer never fires early. If it waited,
/// the close has already happened and the read only has to wake. So any
/// bound below the deadline tells the two apart; half of it leaves a wide
/// margin for a slow runner on both sides.
async fn assert_closed_now(stalled: &mut tokio::net::TcpStream) {
    use tokio::io::AsyncReadExt;
    let bound = DASHBOARD_DRAIN_TIMEOUT / 2;
    let mut chunk = [0u8; 64];
    let read = tokio::time::timeout(bound, stalled.read(&mut chunk)).await;
    assert!(
        matches!(read, Ok(Ok(0)) | Ok(Err(_))),
        "busy connection must be closed before execute() returns (checked for {:?}), got {:?}",
        bound,
        read
    );
}

/// `execute()` waits for the dashboard drain: a connection stuck in a request
/// is already closed when `execute()` returns.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_execute_waits_for_dashboard_drain() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    let configuration = build_control_config(
        &server,
        "127.0.0.1",
        port,
        ControlTestOpts {
            users: 1,
            increase_rate: "1",
            run_time: "1",
            no_autostart: false,
            ..ControlTestOpts::default()
        },
    );
    let ready = arm_test_ready();
    let start = transaction!(wait_for_test_ready);
    let goose_attack =
        common::build_load_test(configuration, vec![get_transactions()], Some(&start), None);
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));

    let base = format!("http://127.0.0.1:{port}");
    let _ = wait_for_health(&[&base], 80).await;
    let mut stalled = stalled_request(port).await;

    ready.send(()).expect("attack waits in test_start");
    let _metrics = load.join().await.expect("load test execute");
    assert_closed_now(&mut stalled).await;
}

/// The dashboard also stops when `execute()` returns an error from the phase
/// loop: here the report file cannot be created, which fails after
/// test_start, and test_start waits until the test has connected.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_dashboard_stops_when_execute_fails() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();

    let mut configuration = build_control_config(
        &server,
        "127.0.0.1",
        port,
        ControlTestOpts {
            no_autostart: false,
            ..ControlTestOpts::default()
        },
    );
    configuration.report_file = vec!["/nonexistent-goose-dir/report.html".to_string()];
    let ready = arm_test_ready();
    let start = transaction!(wait_for_test_ready);
    let goose_attack =
        common::build_load_test(configuration, vec![get_transactions()], Some(&start), None);
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));

    let base = format!("http://127.0.0.1:{port}");
    let _ = wait_for_health(&[&base], 80).await;
    let mut keep_alive = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect to dashboard");
    assert!(health_on(&mut keep_alive).await.is_some());
    let mut stalled = stalled_request(port).await;

    ready.send(()).expect("attack waits in test_start");
    let result = load.join().await;
    assert!(
        matches!(result, Err(GooseError::InvalidOption { ref option, .. }) if option == "--report-file"),
        "execute() must fail on the report file: {:?}",
        result.map(|_| ())
    );
    assert_closed_now(&mut stalled).await;
    assert_dashboard_stopped(port, &mut keep_alive).await;
}
