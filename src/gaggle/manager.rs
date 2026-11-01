//! The manager's side of the wire: a gRPC service that admits exactly the
//! workers it expects, then turns each worker's stream into events for the
//! manager's main loop, and watches each worker's heartbeat.
//!
//! What a client without the token can make the manager hold is bounded
//! before any message is read: at most `expected + 4` connections, each
//! closed unless a worker is admitted on it within [`HELLO_TIMEOUT`]; one
//! stream on each; HTTP/2 windows of [`WINDOW`], so a client can never have
//! more unread bytes buffered; and a token checked on the request's
//! metadata, before the stream's first message is decoded. The first
//! message, `Hello`, may be at most [`MAX_HELLO`] bytes.

use super::delta::{DeltaKind, GaggleDelta};
use super::proto::{
    gaggle_server::{Gaggle, GaggleServer},
    manager_message::Message as ToWorker,
    worker_message::Message as FromWorker,
    Heartbeat, Hello, LogChunk, ManagerMessage, Reject, Stopped, Welcome, WorkerMessage,
    PROTOCOL_VERSION,
};
use crate::GooseError;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::codegen::{Bytes, Service, StdError};

/// The largest message either side sends or accepts: the largest custom data
/// a worker may be handed, a final delta, or a log chunk, with room to spare.
pub(crate) const MAX_MESSAGE: usize = 64 << 20;

/// How often the manager and a worker send each other a heartbeat.
pub(crate) const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);

/// How long a new connection has to say `Hello`, and to have a worker
/// admitted on it before it is closed.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

/// The largest first message, `Hello`, the manager reads.
pub(crate) const MAX_HELLO: usize = 64 << 10;

/// The HTTP/2 stream and connection windows the manager offers: the most a
/// client can send that the manager has not read yet.
const WINDOW: u32 = 64 << 10;

/// The largest header block the manager accepts.
const MAX_HEADERS: u32 = 16 << 10;

/// The connections the manager holds beyond one for each expected worker,
/// for a worker that reconnects before its old connection is closed.
const SPARE_CONNECTIONS: usize = 4;

/// The events queued for the manager's main loop: past them, a worker's
/// reader waits, and HTTP/2 flow control slows the worker.
pub(crate) const EVENT_QUEUE: usize = 64;

/// The metadata key a worker's token travels under, as `Bearer <token>`.
pub(crate) const AUTHORIZATION: &str = "authorization";

/// Why a client without the right token is refused: the same whether it
/// sent none, or one of any other length or value.
const WRONG_TOKEN: &str = "token: the worker's --gaggle-token is not the manager's";

/// What a worker's stream tells the manager's main loop.
#[derive(Debug)]
pub(crate) enum Event {
    /// A worker was admitted.
    Joined,
    /// Its answer to `Prepare`: `None` when it is ready.
    PrepareAck { id: u32, error: Option<String> },
    /// A metrics delta.
    Delta { id: u32, delta: Box<GaggleDelta> },
    /// Its users stopped, and its final delta was sent first.
    Stopped { id: u32, stopped: Stopped },
    /// A piece of one of its logs.
    Log { id: u32, chunk: LogChunk },
    /// It is gone: its heartbeat timed out, its stream closed before its
    /// final delta, or it sent something it should not have.
    Lost { id: u32, why: String },
}

/// One admitted worker, as the manager's main loop sees it.
pub(crate) struct Joined {
    pub(crate) id: u32,
    /// Its name, unique in this Gaggle, safe as a folder name.
    pub(crate) name: String,
    outbound: mpsc::Sender<Result<ManagerMessage, tonic::Status>>,
    /// Set once the worker is lost or shut down: nothing more is sent.
    closed: Arc<AtomicBool>,
}

impl Joined {
    /// Send `message` to the worker, unless it is closed; never waits.
    fn send(&self, message: ToWorker) {
        if self.closed.load(Ordering::Relaxed) {
            return;
        }
        let message = ManagerMessage {
            message: Some(message),
        };
        let kind = message_kind(&message);
        if self.outbound.try_send(Ok(message)).is_err() {
            warn!(
                "gaggle: worker {} ({}): {kind} not sent, its send queue full or closed",
                self.id, self.name
            );
        }
    }

    /// Send `Shutdown`, then close the worker's stream.
    fn shut_down(&self) {
        self.send(ToWorker::Shutdown(Default::default()));
        self.closed.store(true, Ordering::Relaxed);
    }
}

/// The name of a message to a worker, for a log line.
fn message_kind(message: &ManagerMessage) -> &'static str {
    match &message.message {
        Some(ToWorker::Welcome(_)) => "Welcome",
        Some(ToWorker::Reject(_)) => "Reject",
        Some(ToWorker::Prepare(_)) => "Prepare",
        Some(ToWorker::StartAt(_)) => "StartAt",
        Some(ToWorker::Heartbeat(_)) => "Heartbeat",
        Some(ToWorker::ResetMetrics(_)) => "ResetMetrics",
        Some(ToWorker::Stop(_)) => "Stop",
        Some(ToWorker::SendLogs(_)) => "SendLogs",
        Some(ToWorker::Shutdown(_)) => "Shutdown",
        None => "an empty message",
    }
}

/// What the service shares with the manager.
struct Shared {
    expected: usize,
    joined: Mutex<Vec<Joined>>,
    events: flume::Sender<Event>,
    plan_hash: u64,
    hash_check: bool,
    heartbeat_timeout: Duration,
    /// Streams the handler ran for, past the token check (unit tests).
    #[cfg(test)]
    handled: std::sync::atomic::AtomicUsize,
}

/// A token as a fixed length digest, so comparing two never depends on
/// either's length; no token is the empty one.
type TokenDigest = [u8; 20];

fn token_digest(token: &str) -> TokenDigest {
    use sha1::Digest;
    sha1::Sha1::digest(token.as_bytes()).into()
}

/// Compare two digests in time that does not depend on where they differ.
fn same_digest(a: &TokenDigest, b: &TokenDigest) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The token a request presents, `Bearer <token>` in its `authorization`
/// metadata: empty without one, and the whole value when it is not a
/// bearer token, which then matches no token.
fn presented_token(metadata: &tonic::metadata::MetadataMap) -> &str {
    match metadata.get(AUTHORIZATION).map(|value| value.to_str()) {
        None => "",
        Some(Ok(value)) => value.strip_prefix("Bearer ").unwrap_or(value),
        Some(Err(_)) => "\0not a token",
    }
}

/// The interceptor in front of the service: a request whose token is not
/// the manager's is refused before the stream handler runs, and so before
/// any message is decoded. Every refusal takes the same path: the presented
/// token, empty or not, is hashed and compared in full.
fn authorize(
    expected: &TokenDigest,
    request: tonic::Request<()>,
) -> Result<tonic::Request<()>, tonic::Status> {
    let presented = token_digest(presented_token(request.metadata()));
    if same_digest(&presented, expected) {
        Ok(request)
    } else {
        Err(tonic::Status::unauthenticated(WRONG_TOKEN))
    }
}

/// A worker name safe as a folder name: letters, digits, `-`, `_` and `.`,
/// never `.` or `..` alone, at most 64 characters.
fn folder_name(name: &str) -> String {
    let mut safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    if safe.trim_matches('.').is_empty() {
        safe = "worker".to_string();
    }
    safe
}

impl Shared {
    /// Why `hello` is refused, naming the field at fault. The token was
    /// checked first, before `hello` was read, so a client without it
    /// learns nothing about the manager.
    fn refusal(&self, hello: &Hello) -> Option<String> {
        if hello.protocol_version != PROTOCOL_VERSION {
            return Some(format!(
                "protocol_version: the worker speaks Gaggle protocol {}, this manager {}",
                hello.protocol_version, PROTOCOL_VERSION
            ));
        }
        if hello.goose_version != env!("CARGO_PKG_VERSION") {
            return Some(format!(
                "goose_version: the worker runs Goose {}, this manager {}",
                hello.goose_version,
                env!("CARGO_PKG_VERSION")
            ));
        }
        if self.hash_check && hello.plan_hash != self.plan_hash {
            return Some(format!(
                "plan_hash: the worker's load test ({:x}) is not the manager's ({:x}); build both from the same code, or pass --no-hash-check",
                hello.plan_hash, self.plan_hash
            ));
        }
        None
    }

    /// Admit a worker named `name`: its id, unique name and stream, or
    /// `None` when the Gaggle has all its workers.
    fn admit(
        &self,
        name: &str,
        outbound: mpsc::Sender<Result<ManagerMessage, tonic::Status>>,
    ) -> Option<(u32, String, Arc<AtomicBool>)> {
        let mut joined = self.joined.lock().ok()?;
        if joined.len() >= self.expected {
            return None;
        }
        let id = joined.len() as u32;
        let base = folder_name(name);
        let mut unique = base.clone();
        let mut n = id;
        while joined.iter().any(|worker| worker.name == unique) {
            unique = format!("{base}-{n}");
            n += 1;
        }
        let closed = Arc::new(AtomicBool::new(false));
        joined.push(Joined {
            id,
            name: unique.clone(),
            outbound,
            closed: closed.clone(),
        });
        Some((id, unique, closed))
    }
}

/// The gRPC service.
struct GaggleService(Arc<Shared>);

/// A refusal sent as the stream's only message.
fn reject(reason: String) -> ReceiverStream<Result<ManagerMessage, tonic::Status>> {
    let (tx, rx) = mpsc::channel(1);
    let _ = tx.try_send(Ok(ManagerMessage {
        message: Some(ToWorker::Reject(Reject { reason })),
    }));
    ReceiverStream::new(rx)
}

#[tonic::async_trait]
impl Gaggle for GaggleService {
    type ConnectStream = ReceiverStream<Result<ManagerMessage, tonic::Status>>;

    async fn connect(
        &self,
        request: tonic::Request<tonic::Streaming<WorkerMessage>>,
    ) -> Result<tonic::Response<Self::ConnectStream>, tonic::Status> {
        #[cfg(test)]
        self.0
            .handled
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let connection = request.extensions().get::<Peer>().cloned();
        let peer = connection
            .as_ref()
            .and_then(|connection| connection.addr)
            .map_or_else(|| "unknown".to_string(), |addr| addr.to_string());
        let mut inbound = request.into_inner();
        let hello = match tokio::time::timeout(HELLO_TIMEOUT, inbound.message()).await {
            Ok(Ok(Some(WorkerMessage {
                message: Some(FromWorker::Hello(hello)),
            }))) => hello,
            _ => {
                warn!("gaggle: a connection from {peer} did not say Hello; closed");
                return Err(tonic::Status::invalid_argument("expected Hello"));
            }
        };
        if let Some(reason) = self.0.refusal(&hello) {
            warn!(
                "gaggle: refused worker {:?} from {peer}: {reason}",
                folder_name(&hello.worker_name)
            );
            return Ok(tonic::Response::new(reject(reason)));
        }
        let (outbound, rx) = mpsc::channel(64);
        let Some((id, name, closed)) = self.0.admit(&hello.worker_name, outbound.clone()) else {
            let reason = format!(
                "the Gaggle has all {} of its workers already",
                self.0.expected
            );
            warn!(
                "gaggle: refused worker {:?} from {peer}: {reason}",
                folder_name(&hello.worker_name)
            );
            return Ok(tonic::Response::new(reject(reason)));
        };
        if let Some(connection) = connection {
            connection.admitted.store(true, Ordering::Relaxed);
        }
        info!("gaggle: worker {id} ({name}) joined from {peer}");
        let _ = outbound.try_send(Ok(ManagerMessage {
            message: Some(ToWorker::Welcome(Welcome { worker_id: id })),
        }));
        let _ = self.0.events.send_async(Event::Joined).await;
        let heard = Arc::new(Mutex::new(Instant::now()));
        let lost = Arc::new(AtomicBool::new(false));
        tokio::spawn(read_worker(
            id,
            inbound,
            self.0.clone(),
            heard.clone(),
            lost.clone(),
            closed.clone(),
        ));
        tokio::spawn(heartbeat_worker(
            id,
            outbound,
            self.0.clone(),
            heard,
            lost,
            closed,
        ));
        Ok(tonic::Response::new(ReceiverStream::new(rx)))
    }
}

/// What the service knows of the connection a request came on.
#[derive(Clone)]
struct Peer {
    addr: Option<SocketAddr>,
    /// A worker was admitted on it: it stays open.
    admitted: Arc<AtomicBool>,
}

/// An accepted connection, holding one of the manager's connection permits
/// until it closes, and closed when no worker is admitted on it within
/// [`HELLO_TIMEOUT`].
struct Counted {
    stream: tokio::net::TcpStream,
    peer: Peer,
    admit_by: Pin<Box<tokio::time::Sleep>>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl tonic::transport::server::Connected for Counted {
    type ConnectInfo = Peer;

    fn connect_info(&self) -> Peer {
        self.peer.clone()
    }
}

impl tokio::io::AsyncRead for Counted {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if !this.peer.admitted.load(Ordering::Relaxed)
            && this.admit_by.as_mut().poll(cx).is_ready()
        {
            return Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "no worker was admitted on this connection in time",
            )));
        }
        Pin::new(&mut this.stream).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for Counted {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

/// Every connection `listener` accepts while a permit is free; one past
/// them is closed at once.
fn counted(
    listener: tokio::net::TcpListener,
    permits: usize,
) -> impl tokio_stream::Stream<Item = std::io::Result<Counted>> + Send + 'static {
    use futures::StreamExt;
    let permits = Arc::new(tokio::sync::Semaphore::new(permits));
    tokio_stream::wrappers::TcpListenerStream::new(listener).filter_map(move |accepted| {
        let permits = permits.clone();
        async move {
            let stream = match accepted {
                Ok(stream) => stream,
                Err(e) => return Some(Err(e)),
            };
            let addr = stream.peer_addr().ok();
            match permits.try_acquire_owned() {
                Ok(permit) => Some(Ok(Counted {
                    stream,
                    peer: Peer {
                        addr,
                        admitted: Arc::new(AtomicBool::new(false)),
                    },
                    admit_by: Box::pin(tokio::time::sleep(HELLO_TIMEOUT)),
                    _permit: permit,
                })),
                Err(_) => {
                    warn!(
                        "gaggle: a connection from {} closed: the manager holds all the connections it allows",
                        addr.map_or_else(|| "unknown".to_string(), |addr| addr.to_string())
                    );
                    None
                }
            }
        }
    })
}

/// The service, with each request's body checked as it arrives: a first
/// message longer than [`MAX_HELLO`] ends the stream before it is read.
#[derive(Clone)]
struct HelloLimit<S>(S);

impl<S: tonic::server::NamedService> tonic::server::NamedService for HelloLimit<S> {
    const NAME: &'static str = S::NAME;
}

impl<S> Service<tonic::codegen::http::Request<tonic::body::Body>> for HelloLimit<S>
where
    S: Service<tonic::codegen::http::Request<tonic::body::Body>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, request: tonic::codegen::http::Request<tonic::body::Body>) -> S::Future {
        self.0.call(request.map(|body| {
            tonic::body::Body::new(FirstMessage {
                body: Box::pin(body),
                header: [0; 5],
                seen: 0,
            })
        }))
    }
}

/// A request body whose first gRPC message may be at most [`MAX_HELLO`]
/// bytes, checked from its 5 byte prefix as soon as that arrives.
struct FirstMessage {
    body: Pin<Box<tonic::body::Body>>,
    header: [u8; 5],
    /// How much of the prefix has arrived.
    seen: usize,
}

impl FirstMessage {
    /// Take what `data` holds of the prefix; an error once it says the
    /// first message is too long.
    fn check(&mut self, data: &Bytes) -> Result<(), tonic::Status> {
        if self.seen == self.header.len() {
            return Ok(());
        }
        for &byte in data.iter().take(self.header.len() - self.seen) {
            self.header[self.seen] = byte;
            self.seen += 1;
        }
        if self.seen < self.header.len() {
            return Ok(());
        }
        let length = u32::from_be_bytes([
            self.header[1],
            self.header[2],
            self.header[3],
            self.header[4],
        ]) as usize;
        if length > MAX_HELLO {
            return Err(tonic::Status::out_of_range(format!(
                "the first message is {length} bytes, more than the {MAX_HELLO} a Hello may be"
            )));
        }
        Ok(())
    }
}

impl http_body::Body for FirstMessage {
    type Data = Bytes;
    type Error = StdError;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, StdError>>> {
        let this = self.get_mut();
        let frame = match this.body.as_mut().poll_frame(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Some(Ok(frame))) => frame,
            Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e.into()))),
            Poll::Ready(None) => return Poll::Ready(None),
        };
        if let Some(data) = frame.data_ref() {
            if let Err(status) = this.check(data) {
                return Poll::Ready(Some(Err(Box::new(status))));
            }
        }
        Poll::Ready(Some(Ok(frame)))
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.body.size_hint()
    }
}

/// Report worker `id` lost, once.
async fn lose(shared: &Shared, id: u32, lost: &AtomicBool, why: String) {
    if !lost.swap(true, Ordering::Relaxed) {
        let _ = shared.events.send_async(Event::Lost { id, why }).await;
    }
}

/// Read worker `id`'s stream until it ends, as events. The event queue is
/// bounded: while it is full this waits, reading nothing more, so HTTP/2
/// flow control holds the worker back. Nothing the main loop waits on is
/// held here while it waits: the main loop only ever takes events and
/// sends with `try_send`, and the lock on `heard` is released before.
/// `heard` is refreshed when an event is queued too, so a worker held back
/// by a slow main loop is not taken for a silent one.
async fn read_worker(
    id: u32,
    mut inbound: tonic::Streaming<WorkerMessage>,
    shared: Arc<Shared>,
    heard: Arc<Mutex<Instant>>,
    lost: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
) {
    let mut got_final = false;
    loop {
        let message = match inbound.message().await {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(status) => {
                debug!("gaggle: worker {id} stream error: {status}");
                break;
            }
        };
        if let Ok(mut heard) = heard.lock() {
            *heard = Instant::now();
        }
        let event = match message.message {
            Some(FromWorker::Heartbeat(_)) | None => continue,
            Some(FromWorker::Hello(_)) => {
                lose(&shared, id, &lost, "it said Hello twice".to_string()).await;
                break;
            }
            Some(FromWorker::PrepareAck(ack)) => Event::PrepareAck {
                id,
                error: ack.error,
            },
            Some(FromWorker::Metrics(metrics)) => match GaggleDelta::decode(&metrics.delta) {
                Ok(delta) => {
                    got_final |= delta.kind == DeltaKind::Final;
                    Event::Delta {
                        id,
                        delta: Box::new(delta),
                    }
                }
                Err(e) => {
                    lose(
                        &shared,
                        id,
                        &lost,
                        format!("it sent a delta that does not decode: {e}"),
                    )
                    .await;
                    break;
                }
            },
            Some(FromWorker::Stopped(stopped)) => Event::Stopped { id, stopped },
            Some(FromWorker::LogChunk(chunk)) => Event::Log { id, chunk },
        };
        if shared.events.send_async(event).await.is_err() {
            break;
        }
        if let Ok(mut heard) = heard.lock() {
            *heard = Instant::now();
        }
    }
    // A worker that stopped as told, then went away, is not lost: the
    // manager's main loop knows it stopped and keeps it so. Either way the
    // event ends any wait for it, its logs among them.
    if !closed.load(Ordering::Relaxed) {
        let why = if got_final {
            "its stream closed after its final metrics, before Shutdown"
        } else {
            "its stream closed before its final metrics"
        };
        lose(&shared, id, &lost, why.to_string()).await;
    }
}

/// Send worker `id` a heartbeat every [`HEARTBEAT_INTERVAL`], and report it
/// lost, send it `Shutdown` and close its stream when nothing has come from
/// it for the heartbeat timeout.
async fn heartbeat_worker(
    id: u32,
    outbound: mpsc::Sender<Result<ManagerMessage, tonic::Status>>,
    shared: Arc<Shared>,
    heard: Arc<Mutex<Instant>>,
    lost: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
) {
    let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL.min(shared.heartbeat_timeout / 2));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        if closed.load(Ordering::Relaxed) || outbound.is_closed() {
            return;
        }
        let silent = heard
            .lock()
            .map(|heard| heard.elapsed())
            .unwrap_or_default();
        if silent > shared.heartbeat_timeout {
            lose(
                &shared,
                id,
                &lost,
                format!(
                    "nothing from it for {} seconds",
                    shared.heartbeat_timeout.as_secs()
                ),
            )
            .await;
            let _ = outbound.try_send(Ok(ManagerMessage {
                message: Some(ToWorker::Shutdown(Default::default())),
            }));
            closed.store(true, Ordering::Relaxed);
            return;
        }
        let _ = outbound.try_send(Ok(ManagerMessage {
            message: Some(ToWorker::Heartbeat(Heartbeat {})),
        }));
    }
}

/// A running manager: its listening service and its workers.
pub(crate) struct Manager {
    shared: Arc<Shared>,
    events: flume::Receiver<Event>,
    /// The address it listens on.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) local_addr: SocketAddr,
    stop_server: Option<tokio::sync::oneshot::Sender<()>>,
    server: Option<tokio::task::JoinHandle<()>>,
}

impl Manager {
    /// Listen on `host:port` for `expected` workers.
    pub(crate) async fn start(
        host: &str,
        port: u16,
        expected: usize,
        token: Option<String>,
        plan_hash: u64,
        hash_check: bool,
        heartbeat_timeout: Duration,
    ) -> Result<Self, GooseError> {
        let bind = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        let listener =
            tokio::net::TcpListener::bind(&bind)
                .await
                .map_err(|e| GooseError::Gaggle {
                    detail: format!("the manager could not listen on {bind}: {e}"),
                })?;
        let local_addr = listener.local_addr()?;
        let (events_tx, events) = flume::bounded(EVENT_QUEUE);
        let shared = Arc::new(Shared {
            expected,
            joined: Mutex::new(Vec::new()),
            events: events_tx,
            plan_hash,
            hash_check,
            heartbeat_timeout,
            #[cfg(test)]
            handled: Default::default(),
        });
        let expected_token = token_digest(token.as_deref().unwrap_or(""));
        // tonic sets one decoding limit for every message on a service:
        // `HelloLimit` holds the first to MAX_HELLO as it arrives.
        let service = HelloLimit(tonic::codegen::InterceptedService::new(
            GaggleServer::new(GaggleService(shared.clone()))
                .max_decoding_message_size(MAX_MESSAGE)
                .max_encoding_message_size(MAX_MESSAGE),
            move |request| authorize(&expected_token, request),
        ));
        let (stop_server, stopped) = tokio::sync::oneshot::channel::<()>();
        let incoming = Box::pin(counted(listener, expected + SPARE_CONNECTIONS));
        let server = tokio::spawn(async move {
            let served = tonic::transport::Server::builder()
                .http2_keepalive_interval(Some(HEARTBEAT_INTERVAL))
                .max_concurrent_streams(1)
                .concurrency_limit_per_connection(1)
                .initial_stream_window_size(WINDOW)
                .initial_connection_window_size(WINDOW)
                .http2_max_header_list_size(MAX_HEADERS)
                .add_service(service)
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = stopped.await;
                })
                .await;
            if let Err(e) = served {
                warn!("gaggle: manager service ended: {e}");
            }
        });
        info!("gaggle: manager listening on {local_addr} for {expected} workers");
        Ok(Manager {
            shared,
            events,
            local_addr,
            stop_server: Some(stop_server),
            server: Some(server),
        })
    }

    /// The next event, waiting until `deadline`.
    pub(crate) async fn next_event(&self, deadline: Instant) -> Option<Event> {
        let wait = deadline.saturating_duration_since(Instant::now());
        tokio::time::timeout(wait, self.events.recv_async())
            .await
            .ok()?
            .ok()
    }

    /// The next event if one is waiting.
    pub(crate) fn try_event(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// Each worker's id and name.
    pub(crate) fn workers(&self) -> Vec<(u32, String)> {
        self.shared
            .joined
            .lock()
            .map(|joined| joined.iter().map(|w| (w.id, w.name.clone())).collect())
            .unwrap_or_default()
    }

    /// Send worker `id` `message`.
    pub(crate) fn send(&self, id: u32, message: ToWorker) {
        if let Ok(joined) = self.shared.joined.lock() {
            if let Some(worker) = joined.get(id as usize) {
                worker.send(message);
            }
        }
    }

    /// Send every worker still connected `message`.
    pub(crate) fn broadcast(&self, message: ToWorker) {
        if let Ok(joined) = self.shared.joined.lock() {
            for worker in joined.iter() {
                worker.send(message.clone());
            }
        }
    }

    /// Send worker `id` `Shutdown` and close its stream: a lost worker, so its
    /// own switch stops any load it still runs.
    pub(crate) fn shut_down(&self, id: u32) {
        if let Ok(joined) = self.shared.joined.lock() {
            if let Some(worker) = joined.get(id as usize) {
                worker.shut_down();
            }
        }
    }

    /// Send every worker `Shutdown`, give the messages a moment to leave,
    /// and stop listening.
    pub(crate) async fn shut_down_all(&mut self) {
        if let Ok(joined) = self.shared.joined.lock() {
            for worker in joined.iter() {
                worker.shut_down();
            }
        }
        // The streams end when the service stops: wait until each worker's
        // queue is drained, at most a second, so `Shutdown` reaches it.
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            let drained = self.shared.joined.lock().map_or(true, |joined| {
                joined
                    .iter()
                    .all(|w| w.outbound.capacity() == w.outbound.max_capacity())
            });
            if drained {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if let Some(stop) = self.stop_server.take() {
            let _ = stop.send(());
        }
        if let Some(server) = self.server.take() {
            let _ = tokio::time::timeout(Duration::from_secs(2), server).await;
        }
    }
}

/// A manager dropped without shutting its workers down, as when its run
/// fails or its task is aborted: every worker's stream closes without
/// `Shutdown`, so each worker's dead man's switch stops its load.
impl Drop for Manager {
    fn drop(&mut self) {
        if let Ok(mut joined) = self.shared.joined.lock() {
            for worker in joined.iter() {
                worker.closed.store(true, Ordering::Relaxed);
            }
            joined.clear();
        }
        if let Some(stop) = self.stop_server.take() {
            let _ = stop.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_names_are_safe_folder_names() {
        crate::gaggle::within(30, || {
            assert_eq!(folder_name("gaggle2-worker-0"), "gaggle2-worker-0");
            assert_eq!(folder_name("../etc"), "..-etc");
            assert_eq!(folder_name(".."), "worker");
            assert_eq!(folder_name(""), "worker");
            assert_eq!(folder_name("a/b\\c d"), "a-b-c-d");
            assert_eq!(folder_name(&"x".repeat(100)).len(), 64);
        });
    }

    /// The token a request presents.
    fn presenting(token: Option<&str>) -> tonic::Request<()> {
        let mut request = tonic::Request::new(());
        if let Some(token) = token {
            request
                .metadata_mut()
                .insert(AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
        }
        request
    }

    /// The interceptor's answer to a request presenting `token`.
    fn refused(expected: &str, token: Option<&str>) -> Option<(tonic::Code, String)> {
        authorize(&token_digest(expected), presenting(token))
            .err()
            .map(|status| (status.code(), status.message().to_string()))
    }

    /// Tokens compare whole, as fixed length digests: a token of another
    /// length is refused exactly as a wrong one of the same length is, and
    /// no token is the empty one.
    #[test]
    fn tokens_compare_whole() {
        crate::gaggle::within(30, || {
            assert_eq!(refused("secret", Some("secret")), None);
            assert_eq!(refused("", None), None);
            let wrong = refused("secret", Some("secreT"));
            assert_eq!(
                wrong,
                Some((tonic::Code::Unauthenticated, WRONG_TOKEN.to_string()))
            );
            for other in [Some("secret1"), Some("s"), Some(""), None] {
                assert_eq!(refused("secret", other), wrong, "{:?}", other);
            }
            // A token to a manager without one is refused too.
            assert_eq!(refused("", Some("x")), wrong);
            assert_eq!(token_digest("secret").len(), token_digest("s").len());
        });
    }
}

#[cfg(test)]
mod handshake_tests {
    use super::super::proto::gaggle_client::GaggleClient;
    use super::*;

    /// A channel to `manager`.
    async fn channel(manager: &Manager) -> tonic::transport::Channel {
        tonic::transport::Endpoint::from_shared(format!("http://{}", manager.local_addr))
            .unwrap()
            .connect()
            .await
            .unwrap()
    }

    /// Open a stream on `channel` that says `hello`, presenting `token`:
    /// what is left to send on it, and its answer.
    async fn open(
        channel: tonic::transport::Channel,
        hello: Hello,
        token: Option<&str>,
    ) -> (
        mpsc::Sender<WorkerMessage>,
        Result<tonic::Streaming<ManagerMessage>, tonic::Status>,
    ) {
        let (tx, rx) = mpsc::channel(4);
        tx.send(WorkerMessage {
            message: Some(FromWorker::Hello(hello)),
        })
        .await
        .unwrap();
        let mut request = tonic::Request::new(ReceiverStream::new(rx));
        if let Some(token) = token {
            request
                .metadata_mut()
                .insert(AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
        }
        let answer = GaggleClient::new(channel)
            .max_encoding_message_size(MAX_MESSAGE)
            .connect(request)
            .await
            .map(tonic::Response::into_inner);
        (tx, answer)
    }

    /// What a manager answers `hello` presenting `token`: its `Welcome`
    /// worker id, or its `Reject` reason or error status message.
    async fn answer(manager: &Manager, hello: Hello, token: Option<&str>) -> Result<u32, String> {
        let (tx, inbound) = open(channel(manager).await, hello, token).await;
        let first = match inbound {
            Ok(mut inbound) => inbound.message().await,
            Err(status) => Err(status),
        };
        // Keep the stream open until the answer is read.
        drop(tx);
        match first {
            Ok(Some(ManagerMessage {
                message: Some(ToWorker::Welcome(welcome)),
            })) => Ok(welcome.worker_id),
            Ok(Some(ManagerMessage {
                message: Some(ToWorker::Reject(reject)),
            })) => Err(reject.reason),
            Err(status) => Err(status.message().to_string()),
            other => panic!("{:?}", other),
        }
    }

    /// The streams the handler ran for.
    fn handled(manager: &Manager) -> usize {
        manager
            .shared
            .handled
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// A worker that says Hello and then nothing: the manager loses it once
    /// its heartbeat timeout passes, and tells it to shut down.
    #[tokio::test]
    async fn gaggle_loses_a_silent_worker() {
        let test = async {
            let manager = Manager::start("127.0.0.1", 0, 1, None, 7, true, Duration::from_secs(1))
                .await
                .unwrap();
            let (tx, inbound) = open(channel(&manager).await, hello(), None).await;
            let mut inbound = inbound.unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            let lost = loop {
                match manager.next_event(deadline).await {
                    Some(Event::Joined) => continue,
                    other => break other,
                }
            };
            assert!(
                matches!(&lost, Some(Event::Lost { id: 0, why }) if why.starts_with("nothing from it")),
                "{:?}",
                lost
            );
            // It was told to shut down, after its Welcome and heartbeats.
            loop {
                let message = inbound.message().await.unwrap().expect("Shutdown");
                match message.message {
                    Some(ToWorker::Shutdown(_)) => break,
                    Some(ToWorker::Welcome(_)) | Some(ToWorker::Heartbeat(_)) => continue,
                    other => panic!("{:?}", other),
                }
            }
            drop(tx);
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// A worker whose stream closes after its final metrics, before
    /// Shutdown, is reported: whatever waits for it, its logs among them,
    /// stops waiting.
    #[tokio::test]
    async fn gaggle_reports_a_worker_gone_after_its_final_metrics() {
        use super::super::delta::{DeltaKind, GaggleDelta};
        use super::super::proto::Metrics;
        let test = async {
            let manager = Manager::start("127.0.0.1", 0, 1, None, 7, true, Duration::from_secs(10))
                .await
                .unwrap();
            let (tx, inbound) = open(channel(&manager).await, hello(), None).await;
            let inbound = inbound.unwrap();
            let last = GaggleDelta::new(0, 1, 0, DeltaKind::Final);
            tx.send(WorkerMessage {
                message: Some(FromWorker::Metrics(Metrics {
                    delta: last.encode().unwrap(),
                })),
            })
            .await
            .unwrap();
            // Once the final delta has arrived, the worker goes.
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match manager.next_event(deadline).await {
                    Some(Event::Joined) => continue,
                    Some(Event::Delta { .. }) => break,
                    other => panic!("{:?}", other),
                }
            }
            drop(tx);
            drop(inbound);
            // Well before the 10 second heartbeat timeout.
            let deadline = Instant::now() + Duration::from_secs(5);
            let gone = loop {
                match manager.next_event(deadline).await {
                    Some(Event::Joined) | Some(Event::Delta { .. }) => continue,
                    other => break other,
                }
            };
            assert!(
                matches!(&gone, Some(Event::Lost { id: 0, why }) if why.contains("after its final metrics")),
                "{:?}",
                gone
            );
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    fn hello() -> Hello {
        Hello {
            goose_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: PROTOCOL_VERSION,
            plan_hash: 7,
            build_id: None,
            worker_name: "w".to_string(),
        }
    }

    /// B2.3, through a client of the test's own: a worker of another protocol
    /// version, Goose version, plan or token is refused, naming the field;
    /// then a worker that matches is welcome, and one more than the Gaggle
    /// expects is refused.
    #[tokio::test]
    async fn gaggle_refuses_a_mismatched_hello() {
        let test = async {
            let manager = Manager::start(
                "127.0.0.1",
                0,
                1,
                Some("secret".to_string()),
                7,
                true,
                Duration::from_secs(10),
            )
            .await
            .unwrap();
            let token = Some("secret");
            let refused = [
                (
                    Hello {
                        protocol_version: PROTOCOL_VERSION + 1,
                        ..hello()
                    },
                    token,
                    "protocol_version",
                ),
                (
                    Hello {
                        goose_version: "0.0.1".to_string(),
                        ..hello()
                    },
                    token,
                    "goose_version",
                ),
                (
                    Hello {
                        plan_hash: 8,
                        ..hello()
                    },
                    token,
                    "plan_hash",
                ),
                (hello(), Some("guess"), "token"),
                // A worker with no token, to a manager with one.
                (hello(), None, "token"),
            ];
            for (hello, token, field) in refused {
                let reason = answer(&manager, hello, token).await.unwrap_err();
                assert!(reason.starts_with(field), "{}: {}", field, reason);
                assert!(!reason.contains("secret") && !reason.contains("guess"));
            }
            // A wrong token is the only thing a client without one hears,
            // however much else is wrong.
            let everything = answer(
                &manager,
                Hello {
                    protocol_version: PROTOCOL_VERSION + 1,
                    goose_version: "0.0.1".to_string(),
                    plan_hash: 8,
                    ..hello()
                },
                Some("guess"),
            )
            .await
            .unwrap_err();
            assert!(everything.starts_with("token"), "{}", everything);
            assert!(
                !everything.contains("protocol") && !everything.contains(env!("CARGO_PKG_VERSION")),
                "{}",
                everything
            );
            assert_eq!(answer(&manager, hello(), token).await, Ok(0));
            let full = answer(&manager, hello(), token).await.unwrap_err();
            assert!(full.contains("all 1 of its workers"), "{}", full);
            // --no-hash-check lets another plan in; a token still binds.
            let unchecked =
                Manager::start("127.0.0.1", 0, 1, None, 7, false, Duration::from_secs(10))
                    .await
                    .unwrap();
            let token = answer(&unchecked, hello(), Some("secret"))
                .await
                .unwrap_err();
            assert!(token.starts_with("token"), "{}", token);
            let other_plan = Hello {
                plan_hash: 8,
                ..hello()
            };
            assert_eq!(answer(&unchecked, other_plan, None).await, Ok(0));
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// A worker streaming log chunks faster than the main loop takes them
    /// waits for room: the event queue never holds more than its bound, and
    /// every chunk arrives.
    #[tokio::test]
    async fn gaggle_event_queue_is_bounded() {
        use super::super::proto::LogChunk;
        let test = async {
            let manager = Manager::start("127.0.0.1", 0, 1, None, 7, true, Duration::from_secs(30))
                .await
                .unwrap();
            let (tx, inbound) = open(channel(&manager).await, hello(), None).await;
            let _inbound = inbound.unwrap();
            const CHUNKS: u64 = 4 * EVENT_QUEUE as u64;
            let sender = tokio::spawn(async move {
                for offset in 0..CHUNKS {
                    let chunk = LogChunk {
                        file: "requests.json".to_string(),
                        offset,
                        data: vec![b'x'; 16 << 10],
                        last: offset + 1 == CHUNKS,
                    };
                    if tx
                        .send(WorkerMessage {
                            message: Some(FromWorker::LogChunk(chunk)),
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                tx
            });
            let deadline = Instant::now() + Duration::from_secs(40);
            let (mut chunks, mut most) = (0, 0);
            while chunks < CHUNKS {
                most = most.max(manager.events.len());
                match manager.next_event(deadline).await {
                    Some(Event::Log { .. }) => chunks += 1,
                    Some(Event::Joined) => {}
                    other => panic!("{:?}", other),
                }
                // A slow writer.
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(most <= EVENT_QUEUE, "{} events queued", most);
            drop(sender.await);
        };
        tokio::time::timeout(Duration::from_secs(60), test)
            .await
            .expect("within 60 seconds");
    }

    /// A client with no token, or a wrong one of any length, is refused by
    /// the interceptor: the stream handler, which decodes the first
    /// message, never runs for it.
    #[tokio::test]
    async fn gaggle_refuses_a_wrong_token_before_reading_a_message() {
        let test = async {
            let manager = Manager::start(
                "127.0.0.1",
                0,
                1,
                Some("secret".to_string()),
                7,
                true,
                Duration::from_secs(10),
            )
            .await
            .unwrap();
            for token in [None, Some("secreT"), Some("a much longer guess"), Some("")] {
                let (_tx, answer) = open(channel(&manager).await, hello(), token).await;
                let status = answer.expect_err("refused");
                assert_eq!(status.code(), tonic::Code::Unauthenticated, "{:?}", token);
                assert_eq!(status.message(), WRONG_TOKEN, "{:?}", token);
            }
            assert_eq!(handled(&manager), 0, "the handler ran for a refused client");
            assert_eq!(answer(&manager, hello(), Some("secret")).await, Ok(0));
            assert_eq!(handled(&manager), 1);
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// A first message past MAX_HELLO is refused as its length arrives,
    /// before it is read; one within it is read.
    #[tokio::test]
    async fn gaggle_refuses_a_hello_past_its_limit() {
        let test = async {
            let manager = Manager::start("127.0.0.1", 0, 1, None, 7, true, Duration::from_secs(10))
                .await
                .unwrap();
            let large = Hello {
                worker_name: "w".repeat(1 << 20),
                ..hello()
            };
            let refused = answer(&manager, large, None).await.unwrap_err();
            assert!(refused.contains("Hello"), "{}", refused);
            assert!(manager.try_event().is_none(), "nobody joined");
            let within = Hello {
                worker_name: "w".repeat(MAX_HELLO - 1024),
                ..hello()
            };
            assert_eq!(answer(&manager, within, None).await, Ok(0));
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// One stream on a connection at a time: a second stream on the
    /// connection of a worker's open stream is not served.
    #[tokio::test]
    async fn gaggle_serves_one_stream_per_connection() {
        let test = async {
            let manager = Manager::start("127.0.0.1", 0, 2, None, 7, true, Duration::from_secs(10))
                .await
                .unwrap();
            let channel = channel(&manager).await;
            let (first_tx, first) = open(channel.clone(), hello(), None).await;
            let mut first = first.unwrap();
            assert!(matches!(
                first.message().await.unwrap().unwrap().message,
                Some(ToWorker::Welcome(_))
            ));
            let second = tokio::time::timeout(
                Duration::from_secs(3),
                open(channel.clone(), hello(), None),
            )
            .await;
            assert!(second.is_err(), "a second stream was served");
            assert_eq!(handled(&manager), 1);
            drop(first_tx);
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// The manager holds at most `expected + 4` connections: one more is
    /// closed at once, and one on which no worker is admitted is closed
    /// after HELLO_TIMEOUT, so a worker can join again.
    #[tokio::test]
    async fn gaggle_holds_a_bounded_number_of_connections() {
        use tokio::io::AsyncReadExt;
        let test = async {
            let manager = Manager::start("127.0.0.1", 0, 1, None, 7, true, Duration::from_secs(30))
                .await
                .unwrap();
            let mut held = Vec::new();
            for _ in 0..1 + SPARE_CONNECTIONS {
                held.push(
                    tokio::net::TcpStream::connect(manager.local_addr)
                        .await
                        .unwrap(),
                );
            }
            // Give the manager time to accept them all.
            tokio::time::sleep(Duration::from_millis(500)).await;
            let mut extra = tokio::net::TcpStream::connect(manager.local_addr)
                .await
                .unwrap();
            let mut byte = [0u8; 1];
            let closed = tokio::time::timeout(Duration::from_secs(5), extra.read(&mut byte))
                .await
                .expect("the extra connection is closed at once");
            assert!(matches!(closed, Ok(0) | Err(_)), "{:?}", closed);
            // Nothing admitted on the held connections: each is closed.
            for mut connection in held {
                let closed = tokio::time::timeout(
                    HELLO_TIMEOUT + Duration::from_secs(5),
                    async {
                        let mut buffer = [0u8; 1024];
                        loop {
                            match connection.read(&mut buffer).await {
                                Ok(0) | Err(_) => break,
                                // The server's HTTP/2 preface.
                                Ok(_) => continue,
                            }
                        }
                    },
                )
                .await;
                assert!(closed.is_ok(), "an idle connection was held");
            }
            assert_eq!(answer(&manager, hello(), None).await, Ok(0));
        };
        tokio::time::timeout(Duration::from_secs(60), test)
            .await
            .expect("within 60 seconds");
    }
}
