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
    scenario!("DashboardLoad").register_transaction(transaction!(get_index).set_name("index"))
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

/// A run folder in a new temporary folder, so the reports Save report writes
/// stay out of the checkout.
fn test_output_dir() -> String {
    std::env::temp_dir()
        .join(format!(
            "goose-dashboard-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
        .join("run")
        .to_string_lossy()
        .into_owned()
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
        "--output-dir".into(),
        test_output_dir(),
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
        "--output-dir".into(),
        test_output_dir(),
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
    assert!(configuration.dashboard_auth_token.is_none());
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
        !index_body.contains("/static/chart.min.js"),
        "Chart.js is bundled into app.js, not loaded on its own"
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
    assert!(
        js_body.contains("Chart.js v") && js_body.contains("Released under the MIT License"),
        "app.js must bundle Chart.js with its license comment"
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
    assert_eq!(
        chart_js.status(),
        404,
        "chart.min.js is no longer served; Chart.js is bundled into app.js"
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
    assert_eq!(
        configuration.dashboard_auth_token.as_deref(),
        Some(AUTH_TOKEN)
    );

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
        404,
        "chart.min.js is not served, with or without a token"
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
    // A tab still connected when the run ends keeps Goose up for the report's
    // download; this one closes first.
    drop(events_ok);

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
    assert_eq!(configuration.dashboard_host.as_deref(), Some("localhost"));

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
    assert!(configuration.dashboard_auth_token.is_none());
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
    let mut configuration = build_dashboard_config(&server, "127.0.0.1", port, Some(AUTH_TOKEN));
    assert!(!configuration.dashboard_control);
    // A listed report beside the run's folder, so Delete's 404 comes from the
    // missing route and not from a missing folder.
    let parent = reports_dir("control-off");
    configuration.output_dir = Some(parent.join("run").to_string_lossy().into_owned());
    std::fs::create_dir(parent.join("nightly")).unwrap();
    std::fs::write(
        parent.join("nightly").join("report.json"),
        r#"{"raw_metrics": {}}"#,
    )
    .unwrap();

    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));

    let (base, health) = wait_for_health(&[&base_v4], 80).await;
    let health_json: serde_json::Value = health.json().await.expect("health json");
    assert_eq!(health_json["control_enabled"], false);
    let listing: serde_json::Value = reqwest::Client::new()
        .get(format!("{base}/api/v1/reports"))
        .header("Authorization", bearer_header())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listing["reports"][0]["name"], "nightly");

    let client = reqwest::Client::new();
    let resp = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(
        resp.status(),
        404,
        "control off must 404 start, got {}",
        resp.status()
    );
    let resp = post_control(&client, &base, "/api/v1/control/quit", Some("{}"), true).await;
    assert_eq!(resp.status(), 404, "control off must 404 quit");
    for (path, body) in [
        ("/api/v1/control/report", r#"{"save": false}"#),
        ("/api/v1/control/report/now", "{}"),
    ] {
        let resp = post_control(&client, &base, path, Some(body), true).await;
        assert_eq!(resp.status(), 404, "control off must 404 {}", path);
    }
    let resp = client
        .delete(format!("{base}/api/v1/reports/nightly"))
        .header("Authorization", bearer_header())
        .send()
        .await
        .expect("DELETE");
    assert_eq!(resp.status(), 404, "control off must 404 delete");
    assert!(parent.join("nightly").join("report.json").exists());

    let _ = load.join().await.expect("execute");
    std::fs::remove_dir_all(&parent).ok();
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

/// Wait until `mock` has been called, or fail after 30 seconds.
async fn wait_for_calls(mock: &Mock<'_>, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while mock.calls() == 0 {
        assert!(Instant::now() < deadline, "no traffic reached {}", what);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A Start with a new host and an invalid rate is refused before anything
/// changes: the load test stays idle, and a plain Start runs against the
/// original host. The same Start with a valid rate is accepted, and the
/// snapshot shows the new host and a running phase.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_control_start_with_settings() {
    let server = MockServer::start();
    let original = setup_mock_endpoints(&server);
    let other = MockServer::start();
    let moved = setup_mock_endpoints(&other);
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");
    let configuration =
        build_control_config(&server, "127.0.0.1", port, ControlTestOpts::default());
    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));
    let (base, _) = wait_for_health(&[&base_v4], 80).await;
    let client = reqwest::Client::new();
    let token = Some(AUTH_TOKEN);
    wait_for_phase(&client, &base, token, &["idle"], Duration::from_secs(15)).await;

    // A new host with an invalid rate: refused, and nothing changes.
    let invalid = format!(r#"{{"host":"{}","increase_rate":0}}"#, other.base_url());
    let refused = post_control(
        &client,
        &base,
        "/api/v1/control/start",
        Some(&invalid),
        true,
    )
    .await;
    assert_eq!(refused.status(), 400);
    let refused: serde_json::Value = refused.json().await.expect("start json");
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["error"], "invalid_value");
    assert_eq!(refused["field"], "increase_rate");
    let idle = wait_for_phase(&client, &base, token, &["idle"], Duration::from_secs(15)).await;
    assert_eq!(idle["settings"]["host"], server.base_url());

    // A plain Start runs against the original host.
    let start = post_control(&client, &base, "/api/v1/control/start", None, true).await;
    let start: serde_json::Value = start.json().await.expect("start json");
    assert_eq!(start["ok"], true, "{start}");
    wait_for_phase(
        &client,
        &base,
        token,
        &["increase", "maintain"],
        Duration::from_secs(30),
    )
    .await;
    wait_for_calls(&original[0], "the original host").await;
    assert_eq!(moved[0].calls(), 0, "the refused host was load tested");
    let stop = post_control(&client, &base, "/api/v1/control/stop", None, true).await;
    assert_eq!(stop.status(), 200);
    wait_for_phase(&client, &base, token, &["idle"], Duration::from_secs(60)).await;

    // The same Start with a valid rate: accepted, against the new host.
    let valid = format!(r#"{{"host":"{}","increase_rate":50}}"#, other.base_url());
    let start = post_control(&client, &base, "/api/v1/control/start", Some(&valid), true).await;
    assert_eq!(start.status(), 200);
    let start: serde_json::Value = start.json().await.expect("start json");
    assert_eq!(start["ok"], true, "{start}");
    assert_eq!(start["command"], "start");
    assert_eq!(start["settings"]["host"], other.base_url());
    assert_eq!(start["settings"]["increase_rate"], 50.0);
    let running = wait_for_phase(
        &client,
        &base,
        token,
        &["increase", "maintain"],
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(running["settings"]["host"], other.base_url());
    wait_for_calls(&moved[0], "the new host").await;

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

/// Quit needs the control token, is refused while a load test runs, and
/// shuts Goose down from Idle.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_control_quit() {
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

    // No token, or the token in the URL: refused before the main loop.
    let unauth = post_control(&client, &base, "/api/v1/control/quit", Some("{}"), false).await;
    assert_eq!(unauth.status(), 401);
    let in_url = client
        .post(format!("{base}/api/v1/control/quit?token={AUTH_TOKEN}"))
        .header("Authorization", bearer_header())
        .body("{}")
        .send()
        .await
        .expect("quit POST");
    assert_eq!(in_url.status(), 401);
    let bad_body = post_control(
        &client,
        &base,
        "/api/v1/control/quit",
        Some("{\"x\":1}"),
        true,
    )
    .await;
    assert_eq!(bad_body.status(), 400);

    // Refused while the load test runs.
    let start = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(start.status(), 200);
    wait_for_phase(
        &client,
        &base,
        token,
        &["increase", "maintain"],
        Duration::from_secs(30),
    )
    .await;
    let refused = post_control(&client, &base, "/api/v1/control/quit", Some("{}"), true).await;
    assert_eq!(refused.status(), 200);
    let body: serde_json::Value = refused.json().await.expect("json");
    assert_eq!(body["ok"], false);
    assert_eq!(body["error"], "invalid_phase");
    assert_eq!(body["command"], "quit");

    // Accepted from Idle: Goose shuts down.
    let stop = post_control(&client, &base, "/api/v1/control/stop", Some("{}"), true).await;
    assert_eq!(stop.status(), 200);
    wait_for_phase(&client, &base, token, &["idle"], Duration::from_secs(30)).await;
    let quit = post_control(&client, &base, "/api/v1/control/quit", Some("{}"), true).await;
    assert_eq!(quit.status(), 200);
    let body: serde_json::Value = quit.json().await.expect("json");
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "quit");
    tokio::time::timeout(Duration::from_secs(15), load.join())
        .await
        .expect("Goose exits on Quit")
        .expect("load test execute");
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

/// Case 7: a second stop right behind a stop → ok:false invalid_phase.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_control_second_stop_refused() {
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
    assert_eq!(start_body["ok"], true);

    wait_for_active_users(&client, &base, token, 10, Duration::from_secs(30)).await;

    let stop = post_control(&client, &base, "/api/v1/control/stop", Some("{}"), true).await;
    assert_eq!(stop.status(), 200);
    let stop_body: serde_json::Value = stop.json().await.expect("json");
    assert_eq!(stop_body["ok"], true);
    assert_eq!(stop_body["phase"], "decrease");

    // A Stop replaces the plan with a ramp to zero that ignores the decrease
    // rate, so users are told to exit one per loop pass; the main loop then
    // waits in Decrease for them without reading commands. The second Stop
    // therefore usually lands in Idle. Both phases refuse it as invalid_phase;
    // `test_second_stop_refused_while_decreasing` in `lib.rs` pins the refusal
    // during Decrease.
    let stop2 = post_control(&client, &base, "/api/v1/control/stop", Some("{}"), true).await;
    assert_eq!(stop2.status(), 200);
    let stop2_body: serde_json::Value = stop2.json().await.expect("json");
    assert_eq!(stop2_body["ok"], false);
    assert_eq!(stop2_body["error"], "invalid_phase");
    assert_eq!(stop2_body["command"], "stop");
    assert!(
        stop2_body["phase"] == "decrease" || stop2_body["phase"] == "idle",
        "second stop must report decrease or idle, got {}",
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

/// Case 8b: a users ramp's own ramp down is not a cancel. The snapshot does
/// not report it as stopping, and Users is accepted: it abandons the ramp
/// down and holds the new count until stopped.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_control_users_during_ramp_down() {
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
            users: 10,
            increase_rate: "10",
            run_time: "1",
            decrease_rate: Some("0.34"),
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
        "a ramp down is not a cancel: {}",
        ramp_down
    );
    assert_eq!(ramp_down["settings"]["stage"], "ramping_down");

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
        "users during a ramp down: {}",
        users_body
    );
    assert_eq!(users_body["command"], "users");
    assert_eq!(users_body["phase"], "decrease");
    assert_eq!(users_body["target_users"], 4);

    // The new count replaces the ramp down: hold 4 users until stopped.
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
    assert_eq!(held["settings"]["stage"], "hold_until_stopped");

    load.abort().await;
}

/// Case 8c: Users is refused while a test plan runs, naming users: the plan
/// sets users. The plan keeps running.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_control_users_refused_during_test_plan() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");

    let configuration = build_control_config(
        &server,
        "127.0.0.1",
        port,
        ControlTestOpts {
            test_plan: Some("4,1s;4,30s;0,1s"),
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
    let running = wait_for_phase(
        &client,
        &base,
        token,
        &["maintain"],
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(running["settings"]["stage"], "test_plan");
    assert_eq!(running["settings"]["test_plan"], "4,1;4,30;0,1");
    assert!(running["settings"]["users"].is_null());

    let users = post_control(
        &client,
        &base,
        "/api/v1/control/users",
        Some(r#"{"users":6}"#),
        true,
    )
    .await;
    assert_eq!(users.status(), 200);
    let body: serde_json::Value = users.json().await.expect("json");
    assert_eq!(body["ok"], false, "users during a test plan: {}", body);
    assert_eq!(body["error"], "invalid_phase");
    assert_eq!(body["field"], "users");
    assert_eq!(
        body["message"],
        "failed to reconfigure users, the test plan sets users: edit the plan, or stop and switch mode"
    );
    let still = wait_for_phase(&client, &base, token, &["maintain"], Duration::from_secs(5)).await;
    assert_eq!(still["target_users"], 4);

    load.abort().await;
}

/// Users on an autostarted `-t` run keeps its run time: the run still ends
/// on time instead of holding until stopped, as it did before.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_control_users_keeps_run_time() {
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
            increase_rate: "50",
            run_time: "3",
            no_autostart: false,
            ..ControlTestOpts::default()
        },
    );
    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let started = Instant::now();
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));

    let (base, _) = wait_for_health(&[&base_v4], 80).await;
    let client = reqwest::Client::new();
    let held = wait_for_phase(
        &client,
        &base,
        Some(AUTH_TOKEN),
        &["maintain"],
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(held["settings"]["stage"], "hold");
    assert_eq!(held["settings"]["run_time_secs"], 3);
    let users = post_control(
        &client,
        &base,
        "/api/v1/control/users",
        Some(r#"{"users":4}"#),
        true,
    )
    .await;
    let body: serde_json::Value = users.json().await.expect("json");
    assert_eq!(body["ok"], true, "users on a timed run: {}", body);

    // The run ends by itself: no tab is open, so Goose shuts down.
    tokio::time::timeout(Duration::from_secs(20), load.join())
        .await
        .expect("the run must end on time, not hold until stopped")
        .expect("load test");
    assert!(started.elapsed() < Duration::from_secs(20));
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
    configuration.run_time = Some("5".to_string());
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

    // The transactions table fills from the metrics Goose already keeps.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut transaction = serde_json::Value::Null;
    let mut flags = serde_json::Value::Null;
    while Instant::now() < deadline {
        if let Ok(snap) = get_snapshot_json(&client, &base, None).await {
            if snap["transactions"][0]["run_count"].as_u64().unwrap_or(0) > 0 {
                transaction = snap["transactions"][0].clone();
                flags = snap["flags"].clone();
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        transaction["run_count"].as_u64().unwrap_or(0) > 0,
        "transactions[0] must record runs: {}",
        transaction
    );
    assert_eq!(transaction["scenario_name"], "DashboardLoad");
    assert_eq!(transaction["transaction_name"], "index");
    assert_eq!(flags["transaction_metrics_disabled"], false);
    assert_eq!(flags["scenario_metrics_disabled"], false);

    let _metrics = load.join().await.expect("load test execute");
}

/// `GET /api/v1/series` with these query parameters, encoded.
fn series_url(base: &str, params: &[(&str, &str)]) -> reqwest::Url {
    reqwest::Url::parse_with_params(&format!("{base}/api/v1/series"), params).expect("series url")
}

/// Sum of a JSON array of numbers.
fn json_sum(value: &serde_json::Value) -> f64 {
    value
        .as_array()
        .map(|values| values.iter().filter_map(|v| v.as_f64()).sum())
        .unwrap_or(0.0)
}

/// Measured requests counted in a heatmap column (flattened index, count
/// pairs).
fn column_count(column: &serde_json::Value) -> u64 {
    column["measured"]
        .as_array()
        .expect("measured")
        .chunks(2)
        .map(|pair| pair[1].as_u64().expect("count"))
        .sum()
}

/// A heatmap's columns by second.
fn heatmap_columns(heatmap: &serde_json::Value) -> std::collections::BTreeMap<u64, u64> {
    heatmap["columns"]
        .as_array()
        .expect("columns")
        .iter()
        .map(|column| {
            (
                column["second"].as_u64().expect("second"),
                column_count(column),
            )
        })
        .collect()
}

/// The response time heatmap: its columns count the same requests as the
/// requests per second series, a client holding a heatmap is sent only what
/// changed, and `GET /api/v1/series` returns one request's heatmap.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_dashboard_latency_by_second() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");

    let mut configuration = build_dashboard_config(&server, "127.0.0.1", port, None);
    configuration.run_time = Some("7".to_string());

    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));
    let started = Instant::now();

    let (base, _health) = wait_for_health(&[&base_v4], 80).await;
    let client = reqwest::Client::new();

    let wait = Duration::from_secs(4).saturating_sub(started.elapsed());
    tokio::time::sleep(wait).await;
    let snap = get_snapshot_json(&client, &base, None)
        .await
        .expect("snapshot");
    assert_eq!(snap["flags"]["percentiles_over_time"], true);
    let series = &snap["series"];
    let heatmap = &series["heatmap"];
    // Without a cursor: the whole window.
    assert_eq!(heatmap["since"], 0);
    assert_eq!(heatmap["start_second"], series["start_second"]);
    let rps_len = series["rps"].as_array().expect("rps").len() as u64;
    assert_eq!(
        heatmap["end_second"].as_u64().unwrap(),
        heatmap["start_second"].as_u64().unwrap() + rps_len - 1
    );
    assert!(heatmap.get("buckets").is_none());
    assert!(heatmap["buckets_us"].as_array().expect("buckets_us").len() >= 237);
    // The 100 to 110 ms row, past the rows under a millisecond.
    assert_eq!(
        heatmap["buckets_us"][91],
        serde_json::json!([105_000, 100_000, 109_999])
    );
    let columns = heatmap_columns(heatmap);
    assert!(json_sum(&series["rps"]) > 0.0);
    assert_eq!(
        columns.values().sum::<u64>() as f64,
        json_sum(&series["rps"])
    );
    // Mitigation is disabled: no synthetic times.
    for column in heatmap["columns"].as_array().unwrap() {
        assert_eq!(column["synthetic"], serde_json::json!([]));
    }

    // Holding that heatmap, a client is sent only later changes.
    let generation = heatmap["generation"].as_u64().expect("generation");
    let rev = heatmap["rev"].as_u64().expect("rev");
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let resp = client
        .get(format!(
            "{base}/api/v1/snapshot?heatmap_generation={generation}&heatmap_since={rev}"
        ))
        .send()
        .await
        .expect("snapshot GET");
    let later: serde_json::Value = resp.json().await.expect("snapshot json");
    let delta = &later["series"]["heatmap"];
    assert_eq!(delta["generation"], generation);
    assert_eq!(delta["since"], rev);
    let hold_from = heatmap["hold_from"].as_u64().unwrap();
    for (second, count) in heatmap_columns(delta) {
        // A changed second is sent whole, with every request so far.
        assert!(
            count >= columns.get(&second).copied().unwrap_or(0),
            "second {}",
            second
        );
        assert!(second + 3 >= hold_from, "old second {} resent", second);
    }

    let row = &snap["requests"][0];
    assert_eq!(row["has_series"], true);
    let method = row["method"].as_str().expect("method").to_string();
    let name = row["name"].as_str().expect("name").to_string();

    let resp = client
        .get(series_url(
            &base,
            &[("method", method.as_str()), ("name", name.as_str())],
        ))
        .send()
        .await
        .expect("series GET");
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"].to_str().unwrap(),
        "application/json"
    );
    let one: serde_json::Value = resp.json().await.expect("series json");
    assert_eq!(one["method"], method.as_str());
    assert_eq!(one["name"], name.as_str());
    assert_eq!(one["heatmap"]["since"], 0);
    // The only request: its complete seconds match the snapshot's.
    let one_columns = heatmap_columns(&one["heatmap"]);
    for (second, count) in &columns {
        if *second < hold_from {
            assert_eq!(one_columns.get(second), Some(count), "second {second}");
        }
    }
    // With its cursor, only what changed since.
    let one_generation = one["heatmap"]["generation"].as_u64().unwrap().to_string();
    let one_rev = one["heatmap"]["rev"].as_u64().unwrap().to_string();
    let resp = client
        .get(series_url(
            &base,
            &[
                ("method", method.as_str()),
                ("name", name.as_str()),
                ("heatmap_generation", one_generation.as_str()),
                ("heatmap_since", one_rev.as_str()),
            ],
        ))
        .send()
        .await
        .expect("series GET");
    let one_delta: serde_json::Value = resp.json().await.expect("series json");
    assert_eq!(one_delta["heatmap"]["since"].to_string(), one_rev);

    let status_and_error = |resp: reqwest::Response| async move {
        let status = resp.status().as_u16();
        let body: serde_json::Value = resp.json().await.expect("error json");
        (
            status,
            body["error"].as_str().unwrap_or_default().to_string(),
        )
    };
    let unknown = client
        .get(series_url(
            &base,
            &[("method", "GET"), ("name", "/nothing-here")],
        ))
        .send()
        .await
        .expect("series GET");
    assert_eq!(
        status_and_error(unknown).await,
        (404, "not_found".to_string())
    );
    let missing = client
        .get(series_url(&base, &[("method", "GET")]))
        .send()
        .await
        .expect("series GET");
    assert_eq!(
        status_and_error(missing).await,
        (400, "bad_request".to_string())
    );
    let long_name = "x".repeat(2_048);
    let too_long = client
        .get(series_url(
            &base,
            &[("method", "GET"), ("name", long_name.as_str())],
        ))
        .send()
        .await
        .expect("series GET");
    assert_eq!(
        status_and_error(too_long).await,
        (400, "bad_request".to_string())
    );

    let _metrics = load.join().await.expect("load test execute");
}

/// With `--no-transaction-metrics --no-scenario-metrics` the snapshot says
/// both tables are off, while request metrics still flow.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_dashboard_scenario_and_transaction_metrics_disabled() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");

    let mut configuration = build_dashboard_config(&server, "127.0.0.1", port, None);
    configuration.run_time = Some("5".to_string());
    configuration.no_transaction_metrics = true;
    configuration.no_scenario_metrics = true;

    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));

    let (base, _health) = wait_for_health(&[&base_v4], 80).await;
    let client = reqwest::Client::new();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut snap = serde_json::Value::Null;
    while Instant::now() < deadline {
        if let Ok(s) = get_snapshot_json(&client, &base, None).await {
            if s["requests"].as_array().is_some_and(|r| !r.is_empty()) {
                snap = s;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        snap["requests"].as_array().is_some_and(|r| !r.is_empty()),
        "request rows must appear: {}",
        snap
    );
    assert_eq!(snap["scenarios"], serde_json::json!([]));
    assert_eq!(snap["transactions"], serde_json::json!([]));
    assert_eq!(snap["flags"]["transaction_metrics_disabled"], true);
    assert_eq!(snap["flags"]["scenario_metrics_disabled"], true);
    assert_eq!(snap["flags"]["metrics_disabled"], false);

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
    configuration.run_time = Some("1".to_string());
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

// ---------------------------------------------------------------------------
// Run folders: Save report, the reports routes, and the report switches.
// ---------------------------------------------------------------------------

/// A new empty folder for one test.
fn reports_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "goose-dashboard-reports-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run one short load test without the dashboard, its report written into
/// `folder` by a bare `--report-file`, with `notes`.
async fn write_one_report(server: &MockServer, folder: &std::path::Path, users: &str, notes: &str) {
    let folder = folder.to_string_lossy().into_owned();
    let configuration = common::build_configuration(
        server,
        vec![
            "--output-dir",
            &folder,
            "--report-file",
            "--notes",
            notes,
            "--users",
            users,
            "--increase-rate",
            "10",
        ],
    );
    common::run_load_test(
        common::build_load_test(configuration, vec![get_transactions()], None, None),
        None,
    )
    .await;
}

fn header<'a>(resp: &'a reqwest::Response, name: &str) -> &'a str {
    resp.headers()
        .get(name)
        .unwrap_or_else(|| panic!("missing header {}", name))
        .to_str()
        .unwrap()
}

/// The names `GET /api/v1/reports` lists, in order.
async fn listed_names(client: &reqwest::Client, base: &str) -> Vec<String> {
    let listing: serde_json::Value = client
        .get(format!("{base}/api/v1/reports"))
        .header("Authorization", bearer_header())
        .send()
        .await
        .expect("list reports")
        .json()
        .await
        .expect("listing is JSON");
    listing["reports"]
        .as_array()
        .expect("reports")
        .iter()
        .map(|report| report["name"].as_str().unwrap().to_string())
        .collect()
}

/// Start a dashboard that runs nothing until told to, with run folders in
/// `parent`; returns the load test and its base URL.
async fn start_idle_dashboard(
    server: &MockServer,
    parent: &std::path::Path,
    report_file: Vec<String>,
    notes: Option<&str>,
) -> (LoadTestGuard, String) {
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");
    let mut configuration =
        build_control_config(server, "127.0.0.1", port, ControlTestOpts::default());
    configuration.output_dir = Some(parent.join("dash").to_string_lossy().into_owned());
    configuration.report_file = report_file;
    configuration.notes = notes.map(str::to_string);
    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));
    let (base, _) = wait_for_health(&[&base_v4], 80).await;
    (load, base)
}

/// Rules 12 and 13: the panel lists the folders that hold a report, also
/// one written without the dashboard and while Idle; it serves each
/// format, rendering one the folder lacks; and refuses anything else.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_reports_routes() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let parent = reports_dir("routes");
    write_one_report(&server, &parent.join("older"), "1", "first run").await;
    write_one_report(&server, &parent.join("newer"), "3", "second <run>").await;

    // A report.json alone, its HTML and Markdown rendered when asked for.
    std::fs::create_dir(parent.join("json-only")).unwrap();
    std::fs::copy(
        parent.join("older").join("report.json"),
        parent.join("json-only").join("report.json"),
    )
    .unwrap();
    // A report with no requests, started long ago, to compare against.
    let mut empty: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(parent.join("older").join("report.json")).unwrap(),
    )
    .unwrap();
    empty["raw_request_metrics"] = serde_json::json!([]);
    empty["raw_response_metrics"] = serde_json::json!([]);
    empty["started"] = serde_json::json!("2001-01-01T00:00:00Z");
    std::fs::create_dir(parent.join("empty")).unwrap();
    std::fs::write(
        parent.join("empty").join("report.json"),
        serde_json::to_string(&empty).unwrap(),
    )
    .unwrap();
    // Not listed and not served: a report still being written, a folder of
    // logs, a name the routes refuse, a symlink to a folder, and a folder
    // whose report.json is a symlink.
    std::fs::create_dir(parent.join("partial")).unwrap();
    std::fs::write(parent.join("partial").join(".report.json.tmp"), "{").unwrap();
    std::fs::create_dir(parent.join("logs-only")).unwrap();
    std::fs::write(parent.join("logs-only").join("goose.log"), "x").unwrap();
    std::fs::create_dir(parent.join(".hidden")).unwrap();
    std::fs::copy(
        parent.join("older").join("report.json"),
        parent.join(".hidden").join("report.json"),
    )
    .unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(parent.join("older"), parent.join("linked")).unwrap();
        std::fs::create_dir(parent.join("symjson")).unwrap();
        std::os::unix::fs::symlink(
            parent.join("older").join("report.json"),
            parent.join("symjson").join("report.json"),
        )
        .unwrap();
    }

    // Idle under --no-autostart: no run has a folder yet.
    let (load, base) = start_idle_dashboard(&server, &parent, Vec::new(), None).await;
    let client = reqwest::Client::new();
    let get = |path: String, auth: bool| {
        let mut req = client.get(format!("{base}{path}"));
        if auth {
            req = req.header("Authorization", bearer_header());
        }
        req.send()
    };

    // The listing, newest first by start time.
    assert_eq!(
        get("/api/v1/reports".into(), false).await.unwrap().status(),
        401
    );
    let listing = get("/api/v1/reports".into(), true).await.unwrap();
    assert_eq!(listing.status(), 200);
    let listing: serde_json::Value = listing.json().await.unwrap();
    assert_eq!(listing["dir"], parent.to_string_lossy().as_ref());
    let reports = listing["reports"].as_array().unwrap();
    let names: Vec<&str> = reports
        .iter()
        .map(|report| report["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["newer", "older", "json-only", "empty"]);
    assert_eq!(reports[0]["notes"], "second <run>");
    assert_eq!(reports[0]["in_progress"], false);
    assert_eq!(reports[0]["running"], false);
    assert_eq!(reports[0]["max_users"], 3);
    assert!(reports[0]["requests"].as_u64().unwrap() > 0);

    // A stored report, as an attachment.
    let download = get("/api/v1/reports/newer/report.html".into(), true)
        .await
        .unwrap();
    assert_eq!(download.status(), 200);
    let disposition = header(&download, "content-disposition").to_string();
    assert!(
        disposition.starts_with("attachment; filename=\"goose-")
            && disposition.ends_with("-newer.html\""),
        "{}",
        disposition
    );
    assert_eq!(header(&download, "x-content-type-options"), "nosniff");
    assert_eq!(header(&download, "cache-control"), "no-store");
    assert_eq!(header(&download, "content-security-policy"), "sandbox");
    let body = download.bytes().await.unwrap();
    assert_eq!(
        body.as_ref(),
        std::fs::read(parent.join("newer").join("report.html")).unwrap()
    );
    assert_eq!(
        get("/api/v1/reports/newer/report.html".into(), false)
            .await
            .unwrap()
            .status(),
        401
    );

    // Formats the folder lacks, rendered from its report.json.
    let markdown = get("/api/v1/reports/newer/report.md".into(), true)
        .await
        .unwrap();
    assert_eq!(markdown.status(), 200);
    assert!(header(&markdown, "content-type").starts_with("text/markdown"));
    let markdown = markdown.text().await.unwrap();
    assert!(markdown.contains("second <run>"), "{}", markdown);
    let html = get("/api/v1/reports/json-only/report.html".into(), true)
        .await
        .unwrap();
    assert_eq!(html.status(), 200);
    let html = html.text().await.unwrap();
    assert!(html.contains("first run"), "notes");
    assert!(html.contains("graphs were not recorded"), "rule 7 note");
    let json = get("/api/v1/reports/json-only/report.json".into(), true)
        .await
        .unwrap();
    assert_eq!(json.status(), 200);
    assert!(header(&json, "content-type").starts_with("application/json"));

    // A comparison, and one with a report that has no requests.
    let compare = get(
        "/api/v1/reports/newer/compare.md?baseline=older".into(),
        true,
    )
    .await
    .unwrap();
    assert_eq!(compare.status(), 200);
    assert!(header(&compare, "content-disposition").ends_with("-newer-vs-older.md\""));
    let markdown = compare.text().await.unwrap();
    let named = regex::Regex::new(
        r"^Run newer \(started \d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} UTC\) compared to baseline older \(started \d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} UTC\)\.\n\n",
    )
    .unwrap();
    assert!(named.is_match(&markdown), "{}", markdown);
    let resp = get(
        "/api/v1/reports/newer/compare.md?baseline=empty".into(),
        true,
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), 422);
    assert_eq!(
        resp.text().await.unwrap(),
        "Run empty has no requests to compare."
    );

    // Bad names, files and traversal attempts are plain 404s.
    for path in [
        "/api/v1/reports/missing/report.html",
        "/api/v1/reports/newer/goose.log",
        "/api/v1/reports/newer/report.pdf",
        "/api/v1/reports/newer/..%2Freport.json",
        "/api/v1/reports/..%2Fnewer/report.html",
        "/api/v1/reports/%2Fetc/report.html",
        "/api/v1/reports/../report.html",
        "/api/v1/reports/.hidden/report.json",
        "/api/v1/reports/partial/report.json",
        "/api/v1/reports/logs-only/report.html",
        "/api/v1/reports/linked/report.html",
        "/api/v1/reports/symjson/report.json",
        "/api/v1/reports/newer/compare.md",
        "/api/v1/reports/newer/compare.md?baseline=..%2Folder",
        "/api/v1/reports/newer/compare.md?baseline=partial",
    ] {
        let resp = get(path.to_string(), true).await.unwrap();
        assert_eq!(resp.status(), 404, "{}", path);
    }

    load.abort().await;
    std::fs::remove_dir_all(&parent).ok();
}

/// Rule 12: a report another run left marked in progress lists as
/// incomplete, never as running.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_report_left_in_progress_lists_as_incomplete() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let parent = reports_dir("incomplete");
    write_one_report(&server, &parent.join("cut"), "1", "cut short").await;
    let path = parent.join("cut").join("report.json");
    let mut report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    report["in_progress"] = serde_json::json!(true);
    std::fs::write(&path, serde_json::to_string(&report).unwrap()).unwrap();

    let (load, base) = start_idle_dashboard(&server, &parent, Vec::new(), None).await;
    let listing: serde_json::Value = reqwest::Client::new()
        .get(format!("{base}/api/v1/reports"))
        .header("Authorization", bearer_header())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cut = &listing["reports"][0];
    assert_eq!(cut["name"], "cut");
    assert_eq!(cut["in_progress"], true);
    assert_eq!(cut["running"], false);
    load.abort().await;
    std::fs::remove_dir_all(&parent).ok();
}

/// Rules 9 to 11 and 13: Save report starts on with --dashboard and a
/// --report-file path, the switches need the control token, Save report now
/// writes a report marked in progress that the run's last report replaces,
/// Delete refuses the running run's folder, and with Save report off a run
/// writes only the paths.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_save_report_switches() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let parent = reports_dir("switches");
    let path = parent.join("always.html");
    let path_str = path.to_string_lossy().into_owned();
    let (load, base) =
        start_idle_dashboard(&server, &parent, vec![path_str.clone()], Some("switches")).await;
    let client = reqwest::Client::new();

    // The header's state: on, the path listed, the notes.
    let snap = wait_for_phase(
        &client,
        &base,
        Some(AUTH_TOKEN),
        &["idle"],
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(snap["report"]["save"], true);
    assert_eq!(snap["report"]["graph_data"], false);
    assert_eq!(snap["report"]["paths"], serde_json::json!([path_str]));
    assert_eq!(snap["report"]["notes"], "switches");
    assert_eq!(snap["report"]["parent"], parent.to_string_lossy().as_ref());

    // The switches need the control token and a body naming one.
    for (body, auth, status) in [
        (r#"{"save": false}"#, false, 401),
        ("{}", true, 400),
        (r#"{"save": "no"}"#, true, 400),
        (r#"{"save": false, "other": 1}"#, true, 400),
    ] {
        let resp = post_control(&client, &base, "/api/v1/control/report", Some(body), auth).await;
        assert_eq!(resp.status(), status, "{}", body);
    }
    let resp = post_control(
        &client,
        &base,
        "/api/v1/control/report/now",
        Some("{}"),
        false,
    )
    .await;
    assert_eq!(resp.status(), 401);
    // Save report now needs a run in progress.
    let resp = post_control(
        &client,
        &base,
        "/api/v1/control/report/now",
        Some("{}"),
        true,
    )
    .await;
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], false);
    assert_eq!(body["error"], "invalid_phase");
    let resp = post_control(
        &client,
        &base,
        "/api/v1/control/report",
        Some(r#"{"graph_data": true}"#),
        true,
    )
    .await;
    assert_eq!(resp.status(), 200);

    // Run 1, with Save report on: Save report now, then the final report.
    let start = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(start.status(), 200);
    wait_for_active_users(&client, &base, Some(AUTH_TOKEN), 2, Duration::from_secs(30)).await;
    let now = post_control(
        &client,
        &base,
        "/api/v1/control/report/now",
        Some("{}"),
        true,
    )
    .await;
    let body: serde_json::Value = now.json().await.unwrap();
    assert_eq!(body["ok"], true, "{}", body);
    let folder = parent.join("dash");
    let json = folder.join("report.json");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !json.exists() {
        assert!(
            Instant::now() < deadline,
            "Save report now wrote no report.json"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    assert_eq!(report["in_progress"], true);
    assert!(report["ended"].is_null());
    assert!(report["graphs"].is_object(), "Include graph data is on");
    let listing: serde_json::Value = client
        .get(format!("{base}/api/v1/reports"))
        .header("Authorization", bearer_header())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listing["reports"][0]["name"], "dash");
    assert_eq!(listing["reports"][0]["running"], true);
    // Delete refuses the folder of the run in progress.
    let resp = client
        .delete(format!("{base}/api/v1/reports/dash"))
        .header("Authorization", bearer_header())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 409);
    // Save report turned off after Save report now: the run's last report
    // still replaces the one marked in progress.
    let resp = post_control(
        &client,
        &base,
        "/api/v1/control/report",
        Some(r#"{"save": false}"#),
        true,
    )
    .await;
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], true);
    let stop = post_control(&client, &base, "/api/v1/control/stop", Some("{}"), true).await;
    assert_eq!(stop.status(), 200);
    let snap = wait_for_phase(
        &client,
        &base,
        Some(AUTH_TOKEN),
        &["idle"],
        Duration::from_secs(30),
    )
    .await;
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    assert_eq!(report["in_progress"], false, "the final report replaced it");
    assert_eq!(report["ended_by"], "stopped");
    assert_eq!(report["notes"], "switches");
    assert!(folder.join("report.html").exists());
    assert!(path.exists());
    assert_eq!(
        snap["report"]["last_report"],
        folder.to_string_lossy().as_ref()
    );
    // The panel lists the final report, not the one Save report now wrote.
    let listing: serde_json::Value = client
        .get(format!("{base}/api/v1/reports"))
        .header("Authorization", bearer_header())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listing["reports"][0]["name"], "dash");
    assert_eq!(listing["reports"][0]["in_progress"], false);
    assert_eq!(listing["reports"][0]["ended_by"], "stopped");

    // Run 2, with Save report still off: only the path is written.
    std::fs::remove_file(&path).unwrap();
    let start = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(start.status(), 200);
    wait_for_active_users(&client, &base, Some(AUTH_TOKEN), 2, Duration::from_secs(30)).await;
    let snap = get_snapshot_json(&client, &base, Some(AUTH_TOKEN))
        .await
        .unwrap();
    assert_eq!(snap["report"]["save"], false);
    // Run 1's folder holds files, so run 2 has the next.
    assert_eq!(
        snap["report"]["folder"],
        parent.join("dash-2").to_string_lossy().as_ref()
    );
    let stop = post_control(&client, &base, "/api/v1/control/stop", Some("{}"), true).await;
    assert_eq!(stop.status(), 200);
    wait_for_phase(
        &client,
        &base,
        Some(AUTH_TOKEN),
        &["idle"],
        Duration::from_secs(30),
    )
    .await;
    assert!(path.exists(), "a --report-file path is always written");
    assert!(!parent.join("dash-2").exists(), "nothing asked for dash-2");

    load.abort().await;
    std::fs::remove_dir_all(&parent).ok();
}

/// Wait until `folder` holds its report.json.
async fn wait_for_report(folder: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !folder.join("report.json").exists() {
        assert!(Instant::now() < deadline, "no report in {:?}", folder);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Start an autostarted dashboard run held at test_start, its report saved
/// into `folder`, with dashboard control when `control`; returns the load
/// test, the base URL, and the gate that lets it run.
async fn start_saved_dashboard_run(
    server: &MockServer,
    folder: &std::path::Path,
    stop: Option<&Transaction>,
    control: bool,
) -> (LoadTestGuard, String, tokio::sync::oneshot::Sender<()>) {
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");
    let mut configuration = build_dashboard_config(server, "127.0.0.1", port, Some(AUTH_TOKEN));
    configuration.dashboard_control = control;
    configuration.output_dir = Some(folder.to_string_lossy().into_owned());
    configuration.run_time = Some("2".to_string());
    let ready = arm_test_ready();
    let start = transaction!(wait_for_test_ready);
    let goose_attack =
        common::build_load_test(configuration, vec![get_transactions()], Some(&start), stop);
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));
    let (base, _) = wait_for_health(&[&base_v4], 80).await;
    (load, base, ready)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_dashboard_waits_after_autostarted_run() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let client = reqwest::Client::new();

    // A tab connected when the run ends: Goose goes idle and the report can
    // be downloaded. It stays up after the tab closes, until Quit.
    let parent = reports_dir("wait");
    let folder = parent.join("run");
    let (load, base, ready) = start_saved_dashboard_run(&server, &folder, None, true).await;
    let sse = client
        .get(format!("{base}/api/v1/events"))
        .header("Authorization", bearer_header())
        .send()
        .await
        .expect("SSE connect");
    assert_eq!(sse.status(), 200);
    ready.send(()).expect("attack waits in test_start");
    wait_for_report(&folder).await;
    let snap = wait_for_phase(
        &client,
        &base,
        Some(AUTH_TOKEN),
        &["idle"],
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(
        snap["report"]["last_report"],
        folder.to_string_lossy().as_ref()
    );
    let download = client
        .get(format!("{base}/api/v1/reports/run/report.html"))
        .header("Authorization", bearer_header())
        .send()
        .await
        .expect("download while waiting");
    assert_eq!(download.status(), 200);
    assert!(!download.bytes().await.unwrap().is_empty());

    // The run has ended, so its folder is no longer the running one.
    let resp = client
        .delete(format!("{base}/api/v1/reports/run"))
        .header("Authorization", bearer_header())
        .send()
        .await
        .expect("delete after the run");
    assert_eq!(resp.status(), 204);
    assert!(!folder.exists());

    // No exit timer: still up with no tab connected.
    drop(sse);
    tokio::time::sleep(Duration::from_secs(3)).await;
    let health = client
        .get(format!("{base}/api/v1/health"))
        .send()
        .await
        .expect("still up after the tab closed");
    assert_eq!(health.status(), 200);

    let quit = post_control(&client, &base, "/api/v1/control/quit", Some("{}"), true).await;
    assert_eq!(quit.status(), 200);
    let body: serde_json::Value = quit.json().await.expect("quit json");
    assert_eq!(body["ok"], true);
    let metrics = tokio::time::timeout(Duration::from_secs(15), load.join())
        .await
        .expect("Goose exits on Quit")
        .expect("load test execute");
    assert!(metrics.duration > 0);
    // Nothing more was written on the way out.
    assert!(!parent.join("run-2").exists());
    std::fs::remove_dir_all(&parent).ok();

    // No tab connected when the run ends: Goose exits at once, as before.
    let parent = reports_dir("nowait");
    let folder = parent.join("run");
    let (load, _base, ready) = start_saved_dashboard_run(&server, &folder, None, true).await;
    let started = Instant::now();
    ready.send(()).expect("attack waits in test_start");
    tokio::time::timeout(Duration::from_secs(20), load.join())
        .await
        .expect("Goose exits without waiting")
        .expect("load test execute");
    assert!(started.elapsed() < Duration::from_secs(15));
    assert!(folder.join("report.json").exists());
    std::fs::remove_dir_all(&parent).ok();
}

/// How many times test_stop ran since the counting test reset it.
static TEST_STOP_RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

async fn count_test_stop(_user: &mut GooseUser) -> TransactionResult {
    TEST_STOP_RUNS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

fn test_stop_runs() -> usize {
    TEST_STOP_RUNS.load(std::sync::atomic::Ordering::SeqCst)
}

/// Run an autostarted dashboard run that counts test_stop, with an SSE
/// client connected, until Goose waits Idle after it. Returns the load
/// test, the base URL and the SSE response that keeps the wait going.
async fn wait_after_counted_run(
    server: &MockServer,
    folder: &std::path::Path,
) -> (LoadTestGuard, String, reqwest::Response) {
    TEST_STOP_RUNS.store(0, std::sync::atomic::Ordering::SeqCst);
    let stop = transaction!(count_test_stop);
    let (load, base, ready) = start_saved_dashboard_run(server, folder, Some(&stop), true).await;
    let client = reqwest::Client::new();
    let sse = client
        .get(format!("{base}/api/v1/events"))
        .header("Authorization", bearer_header())
        .send()
        .await
        .expect("SSE connect");
    assert_eq!(sse.status(), 200);
    ready.send(()).expect("attack waits in test_start");
    wait_for_report(folder).await;
    wait_for_phase(
        &client,
        &base,
        Some(AUTH_TOKEN),
        &["idle"],
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(test_stop_runs(), 1, "test_stop ran once when the run ended");
    (load, base, sse)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_test_stop_runs_once_when_exiting_from_idle() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);

    // Ctrl-C while waiting Idle after the run.
    let parent = reports_dir("test-stop-ctrl-c");
    let (load, _base, sse) = wait_after_counted_run(&server, &parent.join("run")).await;
    goose::trigger_killswitch("test: exit from Idle");
    let result = tokio::time::timeout(Duration::from_secs(15), load.join())
        .await
        .expect("Goose exits on Ctrl-C");
    goose::reset_killswitch();
    drop(sse);
    result.expect("load test execute");
    assert_eq!(test_stop_runs(), 1, "test_stop ran again on exit from Idle");
    std::fs::remove_dir_all(&parent).ok();

    // Quit while waiting Idle after the run.
    let parent = reports_dir("test-stop-quit");
    let (load, base, sse) = wait_after_counted_run(&server, &parent.join("run")).await;
    let client = reqwest::Client::new();
    let quit = post_control(&client, &base, "/api/v1/control/quit", Some("{}"), true).await;
    assert_eq!(quit.status(), 200);
    tokio::time::timeout(Duration::from_secs(15), load.join())
        .await
        .expect("Goose exits on Quit")
        .expect("load test execute");
    drop(sse);
    assert_eq!(test_stop_runs(), 1, "test_stop ran again on Quit");
    std::fs::remove_dir_all(&parent).ok();
}

/// Delete needs the control token in the Authorization header, removes a
/// folder's reports and Goose's logs and nothing else, and refuses a name the
/// listing would not list.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_delete_report() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let parent = reports_dir("delete");
    write_one_report(&server, &parent.join("old"), "1", "to delete").await;
    for file in ["goose.log", "requests.csv", "keep.txt"] {
        std::fs::write(parent.join("old").join(file), "x").unwrap();
    }
    write_one_report(&server, &parent.join("gone"), "1", "all of it").await;
    std::fs::create_dir(parent.join("no-report")).unwrap();
    std::fs::write(parent.join("no-report").join("report.html"), "x").unwrap();
    // A report.json that is not a Goose report: the panel does not list it.
    std::fs::create_dir(parent.join("other")).unwrap();
    std::fs::write(parent.join("other").join("report.json"), "{}").unwrap();
    std::fs::write(parent.join("other").join("debug.log"), "x").unwrap();

    let (load, base) = start_idle_dashboard(&server, &parent, Vec::new(), None).await;
    let client = reqwest::Client::new();
    let delete = |name: &str, auth: bool, token_in_url: bool| {
        let mut url = format!("{base}/api/v1/reports/{name}");
        if token_in_url {
            url.push_str(&format!("?token={AUTH_TOKEN}"));
        }
        let mut req = client.delete(url);
        if auth {
            req = req.header("Authorization", bearer_header());
        }
        req.send()
    };
    assert_eq!(delete("old", false, false).await.unwrap().status(), 401);
    assert_eq!(delete("old", false, true).await.unwrap().status(), 401);
    assert_eq!(delete("old", true, true).await.unwrap().status(), 401);
    for name in ["missing", "..%2F..%2Fetc", ".hidden", "no-report", "other"] {
        assert_eq!(
            delete(name, true, false).await.unwrap().status(),
            404,
            "{}",
            name
        );
    }
    assert!(parent.join("no-report").join("report.html").exists());
    assert!(parent.join("other").join("report.json").exists());
    assert!(parent.join("other").join("debug.log").exists());

    // Only the reports and Goose's logs go: a file Goose did not write stays,
    // and so does its folder.
    assert_eq!(delete("old", true, false).await.unwrap().status(), 204);
    let mut left: Vec<String> = std::fs::read_dir(parent.join("old"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, vec!["keep.txt"]);
    assert_eq!(delete("old", true, false).await.unwrap().status(), 404);
    // A folder left empty goes too.
    assert_eq!(delete("gone", true, false).await.unwrap().status(), 204);
    assert!(!parent.join("gone").exists());
    assert_eq!(listed_names(&client, &base).await, Vec::<String>::new());

    load.abort().await;
    std::fs::remove_dir_all(&parent).ok();
}

/// Rules 12 and 14: a folder holding only gzipped reports is listed; a
/// gzipped report is sent as it is to a client that accepts gzip, and
/// decompressed for one that does not; a format the folder lacks is rendered
/// from `report.json.gz`; and Delete removes the gzipped files.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_gzipped_reports_in_the_panel() {
    use std::io::Read;
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let parent = reports_dir("gzipped");
    let folder = parent.join("small");
    let folder_str = folder.to_string_lossy().into_owned();
    let configuration = common::build_configuration(
        &server,
        vec![
            "--output-dir",
            &folder_str,
            "--report-file",
            "--notes",
            "gzipped",
            "--compress",
        ],
    );
    common::run_load_test(
        common::build_load_test(configuration, vec![get_transactions()], None, None),
        None,
    )
    .await;
    let gunzip = |bytes: &[u8]| {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(bytes)
            .read_to_end(&mut out)
            .unwrap();
        out
    };
    let stored = std::fs::read(folder.join("report.html.gz")).unwrap();

    let (load, base) = start_idle_dashboard(&server, &parent, Vec::new(), None).await;
    let client = reqwest::Client::builder().no_gzip().build().unwrap();
    assert_eq!(listed_names(&client, &base).await, vec!["small"]);

    // Accepts gzip: the stored file, as it is.
    let resp = client
        .get(format!("{base}/api/v1/reports/small/report.html"))
        .header("Authorization", bearer_header())
        .header("Accept-Encoding", "gzip, deflate")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(header(&resp, "content-encoding"), "gzip");
    assert!(header(&resp, "content-type").starts_with("text/html"));
    assert!(header(&resp, "content-disposition").ends_with("-small.html\""));
    assert_eq!(resp.bytes().await.unwrap().as_ref(), stored.as_slice());

    // Does not accept gzip: decompressed.
    for accept in [None, Some("identity"), Some("gzip;q=0")] {
        let mut req = client
            .get(format!("{base}/api/v1/reports/small/report.html"))
            .header("Authorization", bearer_header());
        if let Some(accept) = accept {
            req = req.header("Accept-Encoding", accept);
        }
        let resp = req.send().await.unwrap();
        assert_eq!(resp.status(), 200, "{:?}", accept);
        assert!(
            resp.headers().get("content-encoding").is_none(),
            "{:?}",
            accept
        );
        assert_eq!(
            resp.bytes().await.unwrap().as_ref(),
            gunzip(&stored).as_slice()
        );
    }

    // Markdown, which the folder lacks, rendered from report.json.gz.
    let resp = client
        .get(format!("{base}/api/v1/reports/small/report.md"))
        .header("Authorization", bearer_header())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.text().await.unwrap().contains("gzipped"));

    // Delete removes the gzipped reports, and the folder they leave empty.
    let resp = client
        .delete(format!("{base}/api/v1/reports/small"))
        .header("Authorization", bearer_header())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
    assert!(!folder.exists());

    load.abort().await;
    std::fs::remove_dir_all(&parent).ok();
}

/// Rule 10: Save report turned on during a run keeps the graph data from
/// then on, so the report has a heatmap; with it off at the start, Goose kept
/// only the dashboard's window and no heatmap.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_save_report_turned_on_during_a_run() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let parent = reports_dir("on-mid-run");
    let (load, base) = start_idle_dashboard(&server, &parent, Vec::new(), None).await;
    let client = reqwest::Client::new();
    let resp = post_control(
        &client,
        &base,
        "/api/v1/control/report",
        Some(r#"{"save": false, "graph_data": true}"#),
        true,
    )
    .await;
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], true, "{}", body);
    let start = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(start.status(), 200);
    wait_for_active_users(&client, &base, Some(AUTH_TOKEN), 2, Duration::from_secs(30)).await;
    let resp = post_control(
        &client,
        &base,
        "/api/v1/control/report",
        Some(r#"{"save": true}"#),
        true,
    )
    .await;
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], true, "{}", body);
    // Requests after the switch, for the heatmap to hold.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let stop = post_control(&client, &base, "/api/v1/control/stop", Some("{}"), true).await;
    assert_eq!(stop.status(), 200);
    wait_for_phase(
        &client,
        &base,
        Some(AUTH_TOKEN),
        &["idle"],
        Duration::from_secs(30),
    )
    .await;
    let report: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(parent.join("dash").join("report.json")).unwrap(),
    )
    .unwrap();
    assert!(
        report["graphs"]["heatmap"].is_object(),
        "Save report turned on during the run keeps the heatmap: {}",
        report["graphs"]
    );
    load.abort().await;
    std::fs::remove_dir_all(&parent).ok();
}

/// Save report now answers a folder it can't create with a refusal and the
/// run goes on; with `--no-metrics`, Save report can't be turned on.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_report_switches_refuse_what_they_cannot_do() {
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let parent = reports_dir("refusals");
    let (load, base) = start_idle_dashboard(&server, &parent, Vec::new(), None).await;
    let client = reqwest::Client::new();
    let start = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(start.status(), 200);
    wait_for_active_users(&client, &base, Some(AUTH_TOKEN), 2, Duration::from_secs(30)).await;
    // A file where the run's folder would go.
    std::fs::write(parent.join("dash"), "in the way").unwrap();
    let resp = post_control(
        &client,
        &base,
        "/api/v1/control/report/now",
        Some("{}"),
        true,
    )
    .await;
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], false, "{}", body);
    assert_eq!(body["error"], "unavailable");
    // The run goes on, and the dashboard says why the report wasn't saved.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let snap = get_snapshot_json(&client, &base, Some(AUTH_TOKEN))
            .await
            .unwrap();
        assert!(
            ["increase", "maintain"].contains(&snap["phase"].as_str().unwrap()),
            "{}",
            snap["phase"]
        );
        if snap["report"]["error"].is_string() {
            break;
        }
        assert!(Instant::now() < deadline, "{}", snap["report"]);
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    std::fs::remove_file(parent.join("dash")).unwrap();
    let stop = post_control(&client, &base, "/api/v1/control/stop", Some("{}"), true).await;
    assert_eq!(stop.status(), 200);
    wait_for_phase(
        &client,
        &base,
        Some(AUTH_TOKEN),
        &["idle"],
        Duration::from_secs(30),
    )
    .await;
    load.abort().await;

    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");
    let mut configuration =
        build_control_config(&server, "127.0.0.1", port, ControlTestOpts::default());
    configuration.no_metrics = true;
    configuration.co_mitigation = None;
    // A bare --debug-log, which --no-metrics allows, gives the run a folder.
    configuration.output_dir = Some(parent.join("none").to_string_lossy().into_owned());
    configuration.debug_log = Some("\0".to_string());
    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));
    let (base, _) = wait_for_health(&[&base_v4], 80).await;
    let snap = wait_for_phase(
        &client,
        &base,
        Some(AUTH_TOKEN),
        &["idle"],
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(snap["report"]["save"], false);
    let resp = post_control(
        &client,
        &base,
        "/api/v1/control/report",
        Some(r#"{"save": true}"#),
        true,
    )
    .await;
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], false, "{}", body);
    assert_eq!(body["error"], "invalid_report");
    // Nor can Save report now write one, even with a folder for a bare log.
    let start = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(start.status(), 200);
    wait_for_active_users(&client, &base, Some(AUTH_TOKEN), 2, Duration::from_secs(30)).await;
    let resp = post_control(
        &client,
        &base,
        "/api/v1/control/report/now",
        Some("{}"),
        true,
    )
    .await;
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], false, "{}", body);
    assert_eq!(body["error"], "invalid_report");
    let stop = post_control(&client, &base, "/api/v1/control/stop", Some("{}"), true).await;
    assert_eq!(stop.status(), 200);
    wait_for_phase(
        &client,
        &base,
        Some(AUTH_TOKEN),
        &["idle"],
        Duration::from_secs(30),
    )
    .await;
    assert!(!parent.join("none").join("report.json").exists());
    load.abort().await;
    std::fs::remove_dir_all(&parent).ok();
}

/// Rule 15: under --compress, Save report now writes the run's folder report
/// gzipped: a report.json.gz marked in progress, and no plain report.json.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_save_report_now_gzips_under_compress() {
    use std::io::Read;
    let server = MockServer::start();
    let _mocks = setup_mock_endpoints(&server);
    let parent = reports_dir("now-gzipped");
    let port = reserve_port();
    let base_v4 = format!("http://127.0.0.1:{port}");
    let mut configuration =
        build_control_config(&server, "127.0.0.1", port, ControlTestOpts::default());
    configuration.output_dir = Some(parent.join("dash").to_string_lossy().into_owned());
    configuration.compress = Some(6);
    let goose_attack = common::build_load_test(configuration, vec![get_transactions()], None, None);
    let load = LoadTestGuard::new(tokio::spawn(async move { goose_attack.execute().await }));
    let (base, _) = wait_for_health(&[&base_v4], 80).await;
    let client = reqwest::Client::new();
    let start = post_control(&client, &base, "/api/v1/control/start", Some("{}"), true).await;
    assert_eq!(start.status(), 200);
    wait_for_active_users(&client, &base, Some(AUTH_TOKEN), 2, Duration::from_secs(30)).await;
    let now = post_control(
        &client,
        &base,
        "/api/v1/control/report/now",
        Some("{}"),
        true,
    )
    .await;
    let body: serde_json::Value = now.json().await.unwrap();
    assert_eq!(body["ok"], true, "{}", body);
    let folder = parent.join("dash");
    let gz = folder.join("report.json.gz");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !gz.exists() {
        assert!(
            Instant::now() < deadline,
            "Save report now wrote no report.json.gz"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!folder.join("report.json").exists());
    assert!(!folder.join("report.html").exists());
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(&gz).unwrap())
        .read_to_string(&mut text)
        .unwrap();
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["in_progress"], true);
    let stop = post_control(&client, &base, "/api/v1/control/stop", Some("{}"), true).await;
    assert_eq!(stop.status(), 200);
    wait_for_phase(
        &client,
        &base,
        Some(AUTH_TOKEN),
        &["idle"],
        Duration::from_secs(30),
    )
    .await;
    load.abort().await;
    std::fs::remove_dir_all(&parent).ok();
}
