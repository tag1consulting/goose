//! The raw hyper client, with the `raw-hyper` feature: each user's one HTTP/1.1
//! connection, over hyper's connection API with no pool, no cookies, no redirects,
//! no decompression, no proxies and no TLS.
//!
//! [`GooseUser::raw_get`](crate::goose::GooseUser::raw_get), or a
//! [`GooseRequest`](crate::goose::GooseRequest) built with `raw(true)`, sends its
//! request here instead of through the user's reqwest client. The Goose Book's
//! "Raw Hyper Client" page has the measurements behind it.

use http::header::{HeaderValue, HOST, USER_AGENT};
use http::{Method, Request, Response, Uri};
use hyper::body::Incoming;
use hyper::client::conn::http1::{self, SendRequest};
use hyper_util::rt::TokioIo;
use std::fmt;
use std::time::Duration;
use url::Url;

/// The user agent of every raw request: Goose's default.
const RAW_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

/// An open connection, and where it goes.
struct Connection {
    /// The host it was opened to, as the URL names it.
    host: String,
    /// The port it was opened to.
    port: u16,
    /// The `Host` header of each request on it.
    host_header: HeaderValue,
    /// Sends requests on it, one at a time.
    sender: SendRequest<reqwest::Body>,
}

/// A user's raw connection, opened by its first raw request and opened again when
/// it closes, fails, or a request goes to another host or port. A clone of a
/// [`GooseUser`](crate::goose::GooseUser) is another user, and starts with none.
#[derive(Default)]
pub(crate) struct RawConnection {
    connection: Option<Connection>,
}

impl Clone for RawConnection {
    fn clone(&self) -> Self {
        RawConnection::default()
    }
}

impl fmt::Debug for RawConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawConnection")
            .field("open", &self.connection.is_some())
            .finish()
    }
}

/// Why a raw request got no response.
#[derive(Debug)]
pub(crate) enum RawError {
    /// The URL is not `http://`: the raw client has no TLS.
    Scheme(String),
    /// The URL has no host, or a path that is not a valid request target.
    Url(String),
    /// Opening the connection failed.
    Connect(std::io::Error),
    /// The connection failed, or the server sent no valid response head.
    Http(hyper::Error),
    /// No response head arrived within the request timeout.
    Timeout(Duration),
}

impl fmt::Display for RawError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RawError::Scheme(scheme) => write!(
                f,
                "the raw hyper client sends http:// requests only, not {scheme}://"
            ),
            RawError::Url(url) => write!(f, "the raw hyper client cannot request {url}"),
            RawError::Connect(e) => write!(f, "error connecting: {e}"),
            RawError::Http(e) => write!(f, "error sending request: {e}"),
            RawError::Timeout(timeout) => {
                write!(f, "no response within {} ms", timeout.as_millis())
            }
        }
    }
}

/// What [`RawConnection::send`] returns: the response head, or why there is none,
/// and whether the request was sent a second time on a new connection.
pub(crate) struct RawSent {
    pub(crate) response: Result<Response<Incoming>, RawError>,
    pub(crate) resent: bool,
}

impl RawConnection {
    /// Sends a `method` request with no body for `url`, and returns once the
    /// response head arrives, or `timeout` after it was started. The body is read
    /// from the response, and must be read to the end, or dropped, before this
    /// connection sends again.
    ///
    /// Like reqwest, it sends an idempotent request once more, on a new connection,
    /// if a reused connection closes before the response head arrives.
    pub(crate) async fn send(&mut self, method: Method, url: &Url, timeout: Duration) -> RawSent {
        match tokio::time::timeout(timeout, self.send_now(method, url)).await {
            Ok(sent) => sent,
            Err(_) => {
                // Whatever the connection was doing, it can't be reused.
                self.connection = None;
                RawSent {
                    response: Err(RawError::Timeout(timeout)),
                    resent: false,
                }
            }
        }
    }

    async fn send_now(&mut self, method: Method, url: &Url) -> RawSent {
        let mut resent = false;
        let fail = |error, resent| RawSent {
            response: Err(error),
            resent,
        };
        if url.scheme() != "http" {
            return fail(RawError::Scheme(url.scheme().to_string()), resent);
        }
        let (host, port) = match (url.host_str(), url.port_or_known_default()) {
            (Some(host), Some(port)) => (host, port),
            _ => return fail(RawError::Url(url.to_string()), resent),
        };
        let target = &url[url::Position::BeforePath..url::Position::AfterQuery];
        let uri: Uri = match target.parse() {
            Ok(uri) => uri,
            Err(_) => return fail(RawError::Url(url.to_string()), resent),
        };
        let idempotent = matches!(
            method,
            Method::GET | Method::HEAD | Method::PUT | Method::DELETE | Method::OPTIONS
        );

        // Drop a connection to another host or port.
        if let Some(connection) = &self.connection {
            if connection.host != host || connection.port != port {
                self.connection = None;
            }
        }

        loop {
            let reused = self.connection.is_some();
            if !reused {
                match connect(host, port, url).await {
                    Ok(connection) => self.connection = Some(connection),
                    Err(e) => return fail(e, resent),
                }
            }
            let connection = self.connection.as_mut().expect("connected above");

            // Ready once the last response's body was read to the end; an error if
            // the connection closed meanwhile, which a new connection fixes, as no
            // request went out on it.
            if let Err(e) = connection.sender.ready().await {
                self.connection = None;
                if reused {
                    continue;
                }
                return fail(RawError::Http(e), resent);
            }

            let mut request = Request::new(reqwest::Body::from(""));
            *request.method_mut() = method.clone();
            *request.uri_mut() = uri.clone();
            let headers = request.headers_mut();
            headers.insert(HOST, connection.host_header.clone());
            headers.insert(USER_AGENT, HeaderValue::from_static(RAW_USER_AGENT));

            match connection.sender.send_request(request).await {
                Ok(response) => {
                    return RawSent {
                        response: Ok(response),
                        resent,
                    }
                }
                Err(e) => {
                    self.connection = None;
                    // A server can close an idle keep-alive connection just as the
                    // next request goes out on it: send once more, as reqwest does.
                    if reused && idempotent && !resent && closed_before_response(&e) {
                        resent = true;
                        continue;
                    }
                    return fail(RawError::Http(e), resent);
                }
            }
        }
    }
}

/// Opens a connection to `host` and `port`, for requests to `url`.
async fn connect(host: &str, port: u16, url: &Url) -> Result<Connection, RawError> {
    // An IPv6 address comes in brackets in a URL, and without them to connect.
    let address = host.trim_start_matches('[').trim_end_matches(']');
    let stream = tokio::net::TcpStream::connect((address, port))
        .await
        .map_err(RawError::Connect)?;
    // As reqwest's connections are.
    stream.set_nodelay(true).map_err(RawError::Connect)?;
    let (sender, connection) = http1::handshake(TokioIo::new(stream))
        .await
        .map_err(RawError::Http)?;
    // Drives the connection until it closes, or the user drops its sender.
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let host_header = match url.port() {
        Some(port) => HeaderValue::from_str(&format!("{host}:{port}")),
        None => HeaderValue::from_str(host),
    }
    .map_err(|_| RawError::Url(url.to_string()))?;
    Ok(Connection {
        host: host.to_string(),
        port,
        host_header,
        sender,
    })
}

/// Whether the connection closed after the request went out and before the
/// response head arrived.
fn closed_before_response(e: &hyper::Error) -> bool {
    if e.is_incomplete_message() || e.is_canceled() {
        return true;
    }
    let mut source = std::error::Error::source(e);
    while let Some(error) = source {
        if let Some(io) = error.downcast_ref::<std::io::Error>() {
            return matches!(
                io.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::NotConnected
            );
        }
        source = error.source();
    }
    false
}

/// `response` as the [`reqwest::Response`] a reqwest request for `url` returns, so
/// it reaches the test through the same
/// [`GooseHttpResponse`](crate::goose::GooseHttpResponse), its body streamed from
/// the connection as it is read.
pub(crate) fn into_reqwest_response(response: Response<Incoming>, url: &Url) -> reqwest::Response {
    use reqwest::ResponseBuilderExt;

    let mut response = response.map(reqwest::Body::wrap);
    // reqwest reads the response's URL from an extension only its builder sets.
    let (with_url, ()) = http::Response::builder()
        .url(url.clone())
        .body(())
        .expect("a response with no headers builds")
        .into_parts();
    response.extensions_mut().extend(with_url.extensions);
    reqwest::Response::from(response)
}
