//! The HTTP server loop and the token checks the dashboard and the MCP Controller
//! share: compiled with either feature.

use axum::http::{header, HeaderMap};
use axum::Router;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// How long a server [`serve`] runs lets open connections finish once told to stop
/// (an SSE stream sending `event: closed`, a response in flight) before closing
/// them. Bounds the wait in `execute()` when a dashboard client never finishes a
/// request.
pub(crate) const SERVER_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

/// How long a client has to send a request's headers, on a new connection and
/// between requests on a keep-alive one, before the connection is closed. A
/// header read timeout and not a whole request timeout, which would cut the
/// long-lived `/api/v1/events` streams.
pub(crate) const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// The accept loop of the HTTP server `name`, the prefix of its log lines. Owns
/// the listener and every connection task, so when it returns nothing of the
/// server is left running.
///
/// At most `max_connections` connections are open at once; a connection
/// accepted beyond that is closed at once, before any of it is read.
pub(crate) async fn serve(
    name: &'static str,
    listener: tokio::net::TcpListener,
    app: Router,
    max_connections: usize,
    mut stop_rx: watch::Receiver<bool>,
) {
    let mut builder = hyper::server::conn::http1::Builder::new();
    // Without a timer hyper applies no header read timeout at all.
    builder
        .timer(hyper_util::rt::TokioTimer::new())
        .header_read_timeout(HEADER_READ_TIMEOUT);
    // Each connection task holds one permit until it ends, aborted or not, so
    // the count does not wait for finished tasks to be reaped.
    let open_connections = Arc::new(tokio::sync::Semaphore::new(max_connections));
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            // A send or a dropped sender both mean stop.
            _ = stop_rx.changed() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => match Arc::clone(&open_connections).try_acquire_owned() {
                    Ok(permit) => {
                        connections.spawn(serve_connection(
                            name,
                            builder.clone(),
                            stream,
                            app.clone(),
                            stop_rx.clone(),
                            permit,
                        ));
                    }
                    Err(_) => {
                        debug!(
                            "[{name}]: closing connection from {peer}: {max_connections} connections already open"
                        );
                        drop(stream);
                    }
                },
                Err(e) => {
                    if !is_connection_error(&e) {
                        // Usually EMFILE: back off as axum::serve does, but
                        // keep listening for the stop signal meanwhile.
                        error!("[{name}]: accept error: {e}");
                        tokio::select! {
                            _ = stop_rx.changed() => break,
                            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                        }
                    }
                }
            },
        }
        // Reap finished connection tasks so the set does not grow with every
        // short lived connection.
        while connections.try_join_next().is_some() {}
    }

    // Stop accepting before the drain, so only open connections remain.
    drop(listener);

    let drained = tokio::time::timeout(SERVER_DRAIN_TIMEOUT, async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    if drained.is_err() {
        debug!(
            "[{name}]: closing {} connection(s) still open after the drain deadline",
            connections.len()
        );
        connections.shutdown().await;
    }
}

/// Serve one HTTP/1 connection until it ends or the server stops; on stop,
/// let an in flight response finish and then close. Holds its slot of the
/// connection cap until it returns.
async fn serve_connection(
    name: &'static str,
    builder: hyper::server::conn::http1::Builder,
    stream: tokio::net::TcpStream,
    app: Router,
    mut stop_rx: watch::Receiver<bool>,
    _permit: tokio::sync::OwnedSemaphorePermit,
) {
    let service = hyper_util::service::TowerToHyperService::new(app);
    let conn = builder.serve_connection(hyper_util::rt::TokioIo::new(stream), service);
    let mut conn = std::pin::pin!(conn);
    let result = tokio::select! {
        result = conn.as_mut() => result,
        _ = stop_rx.changed() => {
            conn.as_mut().graceful_shutdown();
            conn.await
        }
    };
    if let Err(e) = result {
        debug!("[{name}]: connection error: {e}");
    }
}

/// Accept errors that concern one connection only; the listener is fine.
fn is_connection_error(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
    )
}

/// Extract a Bearer token from `Authorization`. Scheme match is case-insensitive
/// (RFC 7235); the token itself is compared separately with constant-time eq.
///
/// Accepts any ASCII whitespace between scheme and token (`Bearer TOKEN`,
/// `Bearer\tTOKEN`, multiple spaces). Rejects extra tokens after the credential.
pub(crate) fn bearer_token_from_headers(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?;
    let s = value.to_str().ok()?;
    let mut parts = s.split_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;
    // Reject `Bearer tok extra`: only the scheme and a single token credential.
    if parts.next().is_some() {
        return None;
    }
    if scheme.eq_ignore_ascii_case("Bearer") && !token.is_empty() {
        Some(token)
    } else {
        None
    }
}

/// Best-effort constant-time equality for auth tokens.
///
/// Always walks a fixed number of bytes derived from both lengths so short-circuit
/// on length mismatch does not dominate the timing profile for typical token sizes.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    // Compare up to max(len_a, len_b), treating missing bytes as 0, and fold in
    // a length mismatch bit so equal prefixes of different lengths never match.
    let len_mismatch = (a.len() != b.len()) as u8;
    let max_len = a.len().max(b.len());
    let mut diff = len_mismatch;
    let mut i = 0;
    while i < max_len {
        let x = if i < a.len() { a[i] } else { 0 };
        let y = if i < b.len() { b[i] } else { 0 };
        diff |= x ^ y;
        i += 1;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"Secret"));
        assert!(!constant_time_eq(b"short", b"longer"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn constant_time_eq_length_mismatch() {
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"ab", b"abc"));
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreT"));
    }
}
