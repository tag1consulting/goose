//! The raw hyper client, with the `raw-hyper` feature: each user's one HTTP/1.1
//! connection, over hyper's connection API with no pool, no redirects, no
//! decompression, no proxies and no HTTP/2, plain or over rustls.
//!
//! [`GooseUser::raw_get`](crate::goose::GooseUser::raw_get),
//! [`raw_post`](crate::goose::GooseUser::raw_post), or a
//! [`GooseRequest`](crate::goose::GooseRequest) built with `raw(true)`, sends its
//! request here instead of through the user's reqwest client. The Goose Book's
//! "Raw Hyper Client" page has the measurements behind it.

use bytes::Bytes;
use http::header::{HeaderMap, HeaderValue, ACCEPT, HOST, USER_AGENT};
#[cfg(feature = "cookies")]
use http::header::{COOKIE, SET_COOKIE};
use http::{Method, Request, Response, Uri};
use hyper::body::Incoming;
use hyper::body::{Body, Frame, SizeHint};
use hyper::client::conn::http1::{self, SendRequest};
use hyper_util::rt::TokioIo;
use std::convert::TryFrom;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::time::Sleep;
use url::Url;

/// What a user's raw requests take from the client Goose built for it: its user
/// agent, timeout and certificate checks, and its cookie jar.
#[derive(Clone, Debug)]
pub(crate) struct RawSettings {
    /// The `User-Agent` of a raw request that sets none.
    pub(crate) user_agent: HeaderValue,
    /// The time a raw request has from connecting to its body's last byte, unless
    /// the request sets its own. `None` waits for good.
    pub(crate) timeout: Option<Duration>,
    /// Over HTTPS, accept any certificate, as `--accept-invalid-certs` does.
    pub(crate) accept_invalid_certs: bool,
    /// The cookie jar the user's reqwest client stores cookies in. `None` when
    /// Goose does not hold the jar: no cookies, a shared client, or a client the
    /// load test built itself.
    #[cfg(feature = "cookies")]
    pub(crate) cookies: Option<Arc<reqwest::cookie::Jar>>,
}

impl RawSettings {
    /// The settings of a client built from `config`, with the jar `cookies`.
    pub(crate) fn new(
        config: &crate::client::GooseClientConfig,
        #[cfg(feature = "cookies")] cookies: Option<Arc<reqwest::cookie::Jar>>,
    ) -> Self {
        RawSettings {
            user_agent: HeaderValue::from_str(&config.user_agent)
                .unwrap_or_else(|_| HeaderValue::from_static(DEFAULT_USER_AGENT)),
            timeout: config.timeout,
            accept_invalid_certs: config.accept_invalid_certs,
            #[cfg(feature = "cookies")]
            cookies,
        }
    }
}

/// Goose's default user agent.
const DEFAULT_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

/// One request for [`RawConnection::send`].
pub(crate) struct RawRequest {
    pub(crate) method: Method,
    pub(crate) url: Url,
    /// The headers the test set. `Host`, `User-Agent`, `Accept` and `Cookie` are
    /// added unless the test set them.
    pub(crate) headers: HeaderMap,
    pub(crate) body: RawBody,
    /// The request's own timeout, which replaces the user's.
    pub(crate) timeout: Option<Duration>,
}

/// A raw request's body.
pub(crate) enum RawBody {
    /// No body.
    Empty,
    /// A body in memory, which can be sent again.
    Bytes(Bytes),
    /// A streamed body, sent once: a request with one is never sent again.
    Stream(Option<reqwest::Body>),
}

impl RawBody {
    /// The body a test gave reqwest.
    pub(crate) fn from_reqwest(body: Option<reqwest::Body>) -> Self {
        match body {
            None => RawBody::Empty,
            Some(body) => match body.as_bytes() {
                Some([]) => RawBody::Empty,
                Some(bytes) => RawBody::Bytes(Bytes::copy_from_slice(bytes)),
                None => RawBody::Stream(Some(body)),
            },
        }
    }

    /// The body for one attempt, `None` if it was already sent and can't be sent
    /// again.
    fn for_attempt(&mut self) -> Option<reqwest::Body> {
        match self {
            RawBody::Empty => Some(reqwest::Body::from(Bytes::new())),
            RawBody::Bytes(bytes) => Some(reqwest::Body::from(bytes.clone())),
            RawBody::Stream(body) => body.take(),
        }
    }
}

/// Where a connection goes.
#[derive(PartialEq, Eq)]
struct Origin {
    https: bool,
    host: String,
    port: u16,
}

/// An open connection, and where it goes.
struct Connection {
    origin: Origin,
    /// The `Host` header of each request on it.
    host_header: HeaderValue,
    /// Sends requests on it, one at a time.
    sender: SendRequest<reqwest::Body>,
}

/// A user's raw connection, opened by its first raw request and opened again when
/// it closes, fails, times out, or a request goes to another origin. A clone of a
/// [`GooseUser`](crate::goose::GooseUser) is another user: it starts with no
/// connection, and shares the cookie jar, as the clone's reqwest client does.
pub(crate) struct RawConnection {
    connection: Option<Connection>,
    settings: RawSettings,
    /// This user's TLS config, built with its first HTTPS request: a session cache
    /// of its own, so a new connection resumes this user's TLS session.
    tls: Option<Arc<rustls::ClientConfig>>,
}

impl RawConnection {
    pub(crate) fn new(settings: RawSettings) -> Self {
        RawConnection {
            connection: None,
            settings,
            tls: None,
        }
    }

    /// The load test replaced the user's reqwest client: Goose no longer holds its
    /// cookie jar, so raw requests send no cookies.
    pub(crate) fn client_replaced(&mut self) {
        #[cfg(feature = "cookies")]
        {
            self.settings.cookies = None;
        }
    }
}

impl Clone for RawConnection {
    fn clone(&self) -> Self {
        RawConnection::new(self.settings.clone())
    }
}

impl fmt::Debug for RawConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawConnection")
            .field("open", &self.connection.is_some())
            .finish()
    }
}

/// Why a raw request got no response, or its body could not be read.
#[derive(Debug)]
pub(crate) enum RawError {
    /// The URL is neither `http://` nor `https://`.
    Scheme(String),
    /// The URL has no host, or a path that is not a valid request target.
    Url(String),
    /// Opening the connection failed.
    Connect(std::io::Error),
    /// The TLS handshake failed, or no TLS config could be built.
    Tls(String),
    /// The connection failed, or the server sent no valid response head.
    Http(hyper::Error),
    /// The request had no response, or its body was not read to the end, within
    /// its timeout.
    Timeout(Duration),
    /// A request with a streamed body, which can't be sent again, met a closed
    /// connection.
    NotResent(hyper::Error),
}

impl fmt::Display for RawError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RawError::Scheme(scheme) => write!(
                f,
                "the raw hyper client sends http:// and https:// requests only, not {scheme}://"
            ),
            RawError::Url(url) => write!(f, "the raw hyper client cannot request {url}"),
            RawError::Connect(e) => write!(f, "error connecting: {e}"),
            RawError::Tls(e) => write!(f, "error in the TLS handshake: {e}"),
            RawError::Http(e) => write!(f, "error sending request: {e}"),
            RawError::Timeout(timeout) => {
                write!(f, "request timed out after {} ms", timeout.as_millis())
            }
            RawError::NotResent(e) => write!(
                f,
                "error sending request: {e}; a streamed body can't be sent again"
            ),
        }
    }
}

impl std::error::Error for RawError {}

/// What [`RawConnection::send`] returns: the response, or why there is none, and
/// whether the request was sent a second time on a new connection, which is true
/// whatever became of the second attempt.
pub(crate) struct RawSent {
    pub(crate) response: Result<Response<RawResponseBody>, RawError>,
    pub(crate) resent: bool,
}

impl RawConnection {
    /// Sends `request` and returns once the response head arrives. The timeout
    /// covers the whole exchange: connecting, sending, the response head, and the
    /// body, which the returned body enforces as it is read, whoever reads it. The
    /// body must be read to the end, or dropped, before this connection sends
    /// again.
    ///
    /// Like reqwest, it sends an idempotent request once more, on a new connection,
    /// if a reused connection closes before the response head arrives.
    pub(crate) async fn send(&mut self, request: RawRequest) -> RawSent {
        let timeout = request.timeout.or(self.settings.timeout);
        let mut resent = false;
        let Some(timeout) = timeout else {
            let response = self.send_now(request, &mut resent).await;
            return RawSent {
                response: response.map(|r| r.map(|body| RawResponseBody::new(body, None))),
                resent,
            };
        };
        let mut deadline = Box::pin(tokio::time::sleep(timeout));
        let sent = {
            let exchange = self.send_now(request, &mut resent);
            tokio::pin!(exchange);
            tokio::select! {
                biased;
                sent = &mut exchange => Some(sent),
                _ = deadline.as_mut() => None,
            }
        };
        let response = match sent {
            Some(Ok(response)) => Ok(response
                .map(|body| RawResponseBody::new(body, Some(Deadline { deadline, timeout })))),
            Some(Err(e)) => Err(e),
            None => {
                // Whatever the connection was doing, it can't be reused.
                self.connection = None;
                Err(RawError::Timeout(timeout))
            }
        };
        RawSent { response, resent }
    }

    async fn send_now(
        &mut self,
        mut request: RawRequest,
        resent: &mut bool,
    ) -> Result<Response<Incoming>, RawError> {
        let url = &request.url;
        let https = match url.scheme() {
            "http" => false,
            "https" => true,
            other => return Err(RawError::Scheme(other.to_string())),
        };
        let (host, port) = match (url.host_str(), url.port_or_known_default()) {
            (Some(host), Some(port)) => (host, port),
            _ => return Err(RawError::Url(url.to_string())),
        };
        let uri: Uri = url[url::Position::BeforePath..url::Position::AfterQuery]
            .parse()
            .map_err(|_| RawError::Url(url.to_string()))?;
        let idempotent = matches!(
            request.method,
            Method::GET | Method::HEAD | Method::PUT | Method::DELETE | Method::OPTIONS
        );

        // The user's cookies for this URL, as reqwest adds them: unless the test
        // set its own Cookie header.
        #[cfg(feature = "cookies")]
        if let Some(jar) = &self.settings.cookies {
            if !request.headers.contains_key(COOKIE) {
                use reqwest::cookie::CookieStore;
                if let Some(cookies) = jar.cookies(url) {
                    request.headers.insert(COOKIE, cookies);
                }
            }
        }

        // Drop a connection to another origin.
        if let Some(connection) = &self.connection {
            let origin = &connection.origin;
            if origin.https != https || origin.host != host || origin.port != port {
                self.connection = None;
            }
        }

        loop {
            let reused = self.connection.is_some();
            if !reused {
                let origin = Origin {
                    https,
                    host: host.to_string(),
                    port,
                };
                self.connection = Some(self.connect(origin, url).await?);
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
                return Err(RawError::Http(e));
            }

            let body = match request.body.for_attempt() {
                Some(body) => body,
                // Only after a failed attempt with a streamed body, below.
                None => unreachable!("a streamed body is sent once"),
            };
            let mut outgoing = Request::new(body);
            *outgoing.method_mut() = request.method.clone();
            *outgoing.uri_mut() = uri.clone();
            let mut headers = request.headers.clone();
            if !headers.contains_key(HOST) {
                headers.insert(HOST, connection.host_header.clone());
            }
            if !headers.contains_key(USER_AGENT) {
                headers.insert(USER_AGENT, self.settings.user_agent.clone());
            }
            if !headers.contains_key(ACCEPT) {
                headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
            }
            *outgoing.headers_mut() = headers;

            match connection.sender.send_request(outgoing).await {
                Ok(response) => {
                    #[cfg(feature = "cookies")]
                    if let Some(jar) = &self.settings.cookies {
                        let mut set_cookies =
                            response.headers().get_all(SET_COOKIE).iter().peekable();
                        if set_cookies.peek().is_some() {
                            use reqwest::cookie::CookieStore;
                            jar.set_cookies(&mut set_cookies, url);
                        }
                    }
                    return Ok(response);
                }
                Err(e) => {
                    self.connection = None;
                    // A server can close an idle keep-alive connection just as the
                    // next request goes out on it: send once more, as reqwest does.
                    if reused && idempotent && !*resent && closed_before_response(&e) {
                        if let RawBody::Stream(_) = request.body {
                            return Err(RawError::NotResent(e));
                        }
                        // Counted from here, whatever the second attempt comes to.
                        *resent = true;
                        continue;
                    }
                    return Err(RawError::Http(e));
                }
            }
        }
    }

    /// Opens a connection to `origin`, for requests to `url`.
    async fn connect(&mut self, origin: Origin, url: &Url) -> Result<Connection, RawError> {
        // An IPv6 address comes in brackets in a URL, and without them to connect.
        let address = origin.host.trim_start_matches('[').trim_end_matches(']');
        let stream = tokio::net::TcpStream::connect((address, origin.port))
            .await
            .map_err(RawError::Connect)?;
        // As reqwest's connections are.
        stream.set_nodelay(true).map_err(RawError::Connect)?;
        let sender = if origin.https {
            let config = match &self.tls {
                Some(config) => config.clone(),
                None => {
                    let config = Arc::new(
                        crate::client::raw_tls_config(self.settings.accept_invalid_certs)
                            .ok_or_else(|| {
                                RawError::Tls("could not build the certificate verifier".into())
                            })?,
                    );
                    self.tls = Some(config.clone());
                    config
                }
            };
            let server_name = rustls::pki_types::ServerName::try_from(address.to_string())
                .map_err(|e| RawError::Tls(e.to_string()))?;
            let stream = tokio_rustls::TlsConnector::from(config)
                .connect(server_name, stream)
                .await
                .map_err(|e| RawError::Tls(e.to_string()))?;
            handshake(stream).await?
        } else {
            handshake(stream).await?
        };
        let host_header = match url.port() {
            Some(port) => HeaderValue::from_str(&format!("{}:{port}", origin.host)),
            None => HeaderValue::from_str(&origin.host),
        }
        .map_err(|_| RawError::Url(url.to_string()))?;
        Ok(Connection {
            origin,
            host_header,
            sender,
        })
    }
}

/// An HTTP/1.1 handshake over `stream`, its connection driven by a task of its own
/// until it closes, or the user drops its sender.
async fn handshake<S>(stream: S) -> Result<SendRequest<reqwest::Body>, RawError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    let (sender, connection) = http1::handshake(TokioIo::new(stream))
        .await
        .map_err(RawError::Http)?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok(sender)
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

/// When a request's time is up.
struct Deadline {
    deadline: Pin<Box<Sleep>>,
    timeout: Duration,
}

/// A raw response's body: hyper's, failing once the request's timeout passes, so
/// a server that sends a head and then stalls can't hold the user, whether the
/// test reads the body or Goose reads it unread.
pub(crate) struct RawResponseBody {
    body: Incoming,
    deadline: Option<Deadline>,
}

impl RawResponseBody {
    fn new(body: Incoming, deadline: Option<Deadline>) -> Self {
        RawResponseBody { body, deadline }
    }
}

impl Body for RawResponseBody {
    type Data = Bytes;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let this = &mut *self;
        if let Some(deadline) = &this.deadline {
            if deadline.deadline.is_elapsed() {
                return Poll::Ready(Some(Err(Box::new(RawError::Timeout(deadline.timeout)))));
            }
        }
        match Pin::new(&mut this.body).poll_frame(cx) {
            Poll::Ready(frame) => Poll::Ready(frame.map(|f| f.map_err(Into::into))),
            Poll::Pending => {
                if let Some(deadline) = &mut this.deadline {
                    if deadline.deadline.as_mut().poll(cx).is_ready() {
                        return Poll::Ready(Some(Err(Box::new(RawError::Timeout(
                            deadline.timeout,
                        )))));
                    }
                }
                Poll::Pending
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

/// `response` as the [`reqwest::Response`] a reqwest request for `url` returns, so
/// it reaches the test through the same
/// [`GooseHttpResponse`](crate::goose::GooseHttpResponse), its body streamed from
/// the connection as it is read.
pub(crate) fn into_reqwest_response(
    response: Response<RawResponseBody>,
    url: &Url,
) -> reqwest::Response {
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

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    /// A connection with `timeout`, no cookies, checking certificates.
    fn connection(timeout: Duration) -> RawConnection {
        RawConnection::new(RawSettings {
            user_agent: HeaderValue::from_static(DEFAULT_USER_AGENT),
            timeout: Some(timeout),
            accept_invalid_certs: false,
            #[cfg(feature = "cookies")]
            cookies: None,
        })
    }

    /// A GET of `url`.
    fn get(url: &str) -> RawRequest {
        RawRequest {
            method: Method::GET,
            url: Url::parse(url).unwrap(),
            headers: HeaderMap::new(),
            body: RawBody::Empty,
            timeout: None,
        }
    }

    /// Reads from `socket` until the end of one request head; false if it closed.
    async fn read_head(socket: &mut TcpStream) -> bool {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match socket.read(&mut byte).await {
                Ok(1) => head.push(byte[0]),
                _ => return false,
            }
        }
        true
    }

    /// Holds `socket` open, sending nothing, until the client closes it.
    async fn stall(mut socket: TcpStream) {
        let mut buffer = [0u8; 1024];
        while let Ok(read) = socket.read(&mut buffer).await {
            if read == 0 {
                return;
            }
        }
    }

    /// A server that sends a response head and part of its body, then stalls: the
    /// body read fails once the request's timeout passes, though the head came in
    /// time. A timeout that ended at the head would leave the read waiting for good.
    #[tokio::test]
    async fn the_timeout_covers_the_body() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            assert!(read_head(&mut socket).await);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial")
                .await
                .unwrap();
            stall(socket).await;
        });

        let timeout = Duration::from_millis(300);
        let mut connection = connection(timeout);
        let started = std::time::Instant::now();
        let sent = connection.send(get(&url)).await;
        let response = sent.response.expect("the head arrives in time");
        let read = tokio::time::timeout(Duration::from_secs(10), response.into_body().collect())
            .await
            .expect("the body read ends with the request's timeout");
        let error = read.expect_err("the body is never finished");
        assert!(
            error.to_string().contains("timed out after 300 ms"),
            "{}",
            error
        );
        assert!(started.elapsed() >= timeout);
    }

    /// A server that closes the kept alive connection once the next request is out,
    /// then stalls on the request sent again: the resend is counted, though it
    /// timed out.
    #[tokio::test]
    async fn a_resend_that_times_out_is_counted() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            assert!(read_head(&mut first).await);
            first
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await
                .unwrap();
            // The second request arrives, and the connection closes unanswered.
            assert!(read_head(&mut first).await);
            drop(first);
            let (mut second, _) = listener.accept().await.unwrap();
            assert!(read_head(&mut second).await);
            stall(second).await;
        });

        let mut connection = connection(Duration::from_millis(500));
        let sent = connection.send(get(&url)).await;
        assert!(!sent.resent);
        let body = sent.response.unwrap().into_body().collect().await.unwrap();
        assert_eq!(body.to_bytes(), "ok");

        let sent = connection.send(get(&url)).await;
        assert!(
            matches!(sent.response, Err(RawError::Timeout(_))),
            "{:?}",
            sent.response.as_ref().err()
        );
        assert!(sent.resent, "the resend that timed out is counted");
    }
}
