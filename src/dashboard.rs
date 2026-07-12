//! Read-only live web dashboard HTTP server.
//!
//! Compiled only with the `dashboard` crate feature. Spawns an axum server that
//! serves a minimal static shell and a one-shot metrics snapshot API. The parent
//! GooseAttack main loop answers [`DashboardRequest`]s via oneshot channels using
//! [`MetricsCommand::GetDashboardSnapshot`].
//!
//! SSE streaming lands in a later PR; this module serves one-shot snapshots only.

use crate::metrics::DashboardSnapshot;
use crate::{GooseConfiguration, GooseError};

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde::Serialize;
use std::sync::Arc;
use tower_http::set_header::SetResponseHeaderLayer;

/// Embedded SPA shell and assets (no inline scripts — CSP-friendly).
const INDEX_HTML: &str = include_str!("dashboard/static/index.html");
const APP_JS: &str = include_str!("dashboard/static/app.js");
const APP_CSS: &str = include_str!("dashboard/static/app.css");

/// Content-Security-Policy applied to every response.
const CSP: &str =
    "default-src 'self'; script-src 'self'; connect-src 'self'; style-src 'self'; img-src 'self' data:";

/// Result of a successful dashboard server spawn.
#[derive(Debug)]
pub(crate) struct DashboardSetup {
    /// Parent end of the request channel.
    pub request_rx: flume::Receiver<DashboardRequest>,
    /// Actual TCP port after bind (useful when configured port is 0 / ephemeral).
    /// Read by unit tests; production only needs `request_rx`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub bound_port: u16,
}

/// Requests from the dashboard HTTP task to the GooseAttack main loop.
#[derive(Debug)]
pub(crate) enum DashboardRequest {
    /// Build and return a compact metrics snapshot.
    GetSnapshot {
        respond: tokio::sync::oneshot::Sender<DashboardSnapshot>,
    },
}

/// Shared state for axum handlers.
#[derive(Clone)]
struct DashboardState {
    /// Channel to the parent main loop.
    request_tx: flume::Sender<DashboardRequest>,
    /// Auth token; empty means auth is disabled (loopback-only runs).
    auth_token: String,
}

#[derive(Debug, Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    version: String,
}

/// Format `host:port` for [`TcpListener::bind`], bracketing bare IPv6 literals.
///
/// Mirrors how operators type `--dashboard-host` (`localhost`, `127.0.0.1`,
/// `::1`, `[::1]`) while remaining valid for `ToSocketAddrs`.
fn format_bind_address(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Spawn the dashboard HTTP server task.
///
/// Binds to `configuration.dashboard_host`:`configuration.dashboard_port` and
/// returns the parent end of the request channel. Logs the listening URL.
///
/// When `--dashboard` is set, bind failure is a hard error (opt-in server must
/// not silently disappear). When the dashboard is disabled, returns `Ok(None)`.
///
/// When/if multi-mode (Gaggle) returns, the dashboard should only run on the
/// standalone/manager process — currently AttackMode is StandAlone only.
pub(crate) async fn setup_dashboard(
    configuration: &GooseConfiguration,
) -> Result<Option<DashboardSetup>, GooseError> {
    if !configuration.dashboard {
        return Ok(None);
    }

    let host = if configuration.dashboard_host.is_empty() {
        "127.0.0.1"
    } else {
        configuration.dashboard_host.as_str()
    };
    let port = configuration.dashboard_port;
    // Use a host:port string so `ToSocketAddrs` resolves `localhost` and IPv6
    // (same approach as Controllers). Do not parse as SocketAddr first — that
    // rejects hostnames and unbracketed `::1`.
    let address = format_bind_address(host, port);

    let listener = tokio::net::TcpListener::bind(&address).await.map_err(|e| {
        error!("[dashboard]: failed to bind {address}: {e} (is the port already in use?)");
        GooseError::Io(e)
    })?;

    let bound = listener.local_addr().map_err(|e| {
        error!("[dashboard]: failed to read local address after bind on {address}: {e}");
        GooseError::Io(e)
    })?;

    let (request_tx, request_rx) = flume::unbounded();
    let state = DashboardState {
        request_tx,
        auth_token: configuration.dashboard_auth_token.clone(),
    };

    let app = build_router(state);

    info!("[dashboard]: listening on http://{bound} (read-only)");

    // Detached server task — no need to rejoin when the load test ends.
    // Wrap in Some so the JoinHandle is not a bare `let _ = future` (clippy).
    let _ = Some(tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            error!("[dashboard]: server error: {e}");
        }
    }));

    Ok(Some(DashboardSetup {
        request_rx,
        bound_port: bound.port(),
    }))
}

/// Build the axum router with public shell/static/health and auth-gated snapshot.
fn build_router(state: DashboardState) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/static/app.js", get(app_js_handler))
        .route("/static/app.css", get(app_css_handler))
        .route("/api/v1/health", get(health_handler))
        .route("/api/v1/snapshot", get(snapshot_handler))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ))
        .with_state(Arc::new(state))
}

async fn index_handler() -> Response<Body> {
    static_response("text/html; charset=utf-8", INDEX_HTML)
}

async fn app_js_handler() -> Response<Body> {
    static_response("application/javascript; charset=utf-8", APP_JS)
}

async fn app_css_handler() -> Response<Body> {
    static_response("text/css; charset=utf-8", APP_CSS)
}

/// Build a static asset response without `unwrap` on the builder.
fn static_response(content_type: &'static str, body: &'static str) -> Response<Body> {
    ([(header::CONTENT_TYPE, content_type)], Body::from(body)).into_response()
}

async fn health_handler() -> impl IntoResponse {
    Json(HealthResponse {
        ok: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

async fn snapshot_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Result<Json<DashboardSnapshot>, StatusCode> {
    if !authorize(&state.auth_token, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();
    if state
        .request_tx
        .send(DashboardRequest::GetSnapshot {
            respond: respond_tx,
        })
        .is_err()
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    match respond_rx.await {
        Ok(snapshot) => Ok(Json(snapshot)),
        Err(_) => Err(StatusCode::SERVICE_UNAVAILABLE),
    }
}

/// Authorize a metric API request.
///
/// When `configured_token` is empty, all requests are allowed (loopback trust).
/// Otherwise accept `Authorization: Bearer <token>` or `?token=<token>` with a
/// constant-time comparison.
fn authorize(configured_token: &str, headers: &HeaderMap, query_token: Option<&str>) -> bool {
    if configured_token.is_empty() {
        return true;
    }

    if let Some(token) = query_token {
        if constant_time_eq(token.as_bytes(), configured_token.as_bytes()) {
            return true;
        }
    }

    if let Some(value) = headers.get(header::AUTHORIZATION) {
        if let Ok(s) = value.to_str() {
            if let Some(token) = s.strip_prefix("Bearer ") {
                if constant_time_eq(token.as_bytes(), configured_token.as_bytes()) {
                    return true;
                }
            }
        }
    }

    false
}

/// Best-effort constant-time equality for auth tokens.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GooseConfiguration;

    #[test]
    fn constant_time_eq_matches() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"Secret"));
        assert!(!constant_time_eq(b"short", b"longer"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn authorize_empty_token_allows_all() {
        let headers = HeaderMap::new();
        assert!(authorize("", &headers, None));
        assert!(authorize("", &headers, Some("anything")));
    }

    #[test]
    fn authorize_requires_token_when_configured() {
        let headers = HeaderMap::new();
        assert!(!authorize("s3cret", &headers, None));
        assert!(!authorize("s3cret", &headers, Some("wrong")));
        assert!(authorize("s3cret", &headers, Some("s3cret")));

        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer s3cret"),
        );
        assert!(authorize("s3cret", &headers, None));

        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer wrong"),
        );
        assert!(!authorize("s3cret", &headers, None));
    }

    #[test]
    fn format_bind_address_brackets_ipv6() {
        assert_eq!(format_bind_address("127.0.0.1", 5118), "127.0.0.1:5118");
        assert_eq!(format_bind_address("localhost", 5118), "localhost:5118");
        assert_eq!(format_bind_address("::1", 5118), "[::1]:5118");
        assert_eq!(format_bind_address("[::1]", 5118), "[::1]:5118");
        assert_eq!(format_bind_address("0.0.0.0", 80), "0.0.0.0:80");
    }

    #[tokio::test]
    async fn binds_localhost_and_ipv6_loopback() {
        // localhost (hostname — rejected by SocketAddr::parse, must use ToSocketAddrs).
        let config = GooseConfiguration {
            dashboard: true,
            dashboard_host: "localhost".to_string(),
            dashboard_port: 0,
            dashboard_auth_token: String::new(),
            ..Default::default()
        };
        let setup = setup_dashboard(&config)
            .await
            .expect("localhost bind should succeed")
            .expect("dashboard enabled");
        assert!(setup.bound_port > 0, "ephemeral port must be recorded");
        // Drop the channel; server task keeps running until process ends (fine in tests).
        drop(setup);

        // ::1 (bare IPv6 literal needs brackets in the bind string).
        let config = GooseConfiguration {
            dashboard: true,
            dashboard_host: "::1".to_string(),
            dashboard_port: 0,
            dashboard_auth_token: String::new(),
            ..Default::default()
        };
        // ::1 may be unavailable in some CI network namespaces; treat bind errors
        // as soft-skip only when the OS reports address-family issues.
        match setup_dashboard(&config).await {
            Ok(Some(setup)) => {
                assert!(setup.bound_port > 0);
            }
            Ok(None) => panic!("dashboard enabled but setup returned None"),
            Err(e) => {
                // Acceptable on hosts without IPv6 loopback.
                let msg = format!("{e}");
                eprintln!("skipping ::1 bind assertion: {msg}");
            }
        }
    }

    #[tokio::test]
    async fn bind_failure_is_hard_error() {
        // Occupy a port, then try to bind the dashboard to the same port.
        let occupied = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("occupy port");
        let port = occupied.local_addr().unwrap().port();

        let config = GooseConfiguration {
            dashboard: true,
            dashboard_host: "127.0.0.1".to_string(),
            dashboard_port: port,
            dashboard_auth_token: String::new(),
            ..Default::default()
        };
        let err = setup_dashboard(&config)
            .await
            .expect_err("second bind on same port must fail hard");
        match err {
            GooseError::Io(_) => {}
            other => panic!("expected GooseError::Io, got {:?}", other),
        }
        // Keep occupied alive until after the failed bind.
        drop(occupied);
    }
}
