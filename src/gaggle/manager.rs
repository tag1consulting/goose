//! The manager's side of the wire: a gRPC service that admits exactly the
//! workers it expects, then turns each worker's stream into events for the
//! manager's main loop, and watches each worker's heartbeat.
//!
//! What a client without the token can make the manager hold is bounded
//! before any message is read: at most `expected + 4` connections, each
//! closed unless a worker is admitted on it within [`HELLO_TIMEOUT`], even
//! one that stalls the manager's writes; one stream at a time on each;
//! HTTP/2 windows that share [`UNREAD_BUDGET`] between the connections (see
//! [`window`]), so the manager holds at most about that much unread data,
//! from every client together; header blocks of at most [`MAX_HEADERS`]; and
//! a token checked on the request's metadata, before the stream's first
//! message is decoded. Once a worker is admitted, its stream may also hold
//! one message of at most [`MAX_MESSAGE`] as it is decoded. A client
//! without the token can still take every connection slot, again each time
//! one is closed, keeping workers out until their connect timeout: the
//! network must be trusted until TLS arrives. The first message, `Hello`,
//! may be at most [`MAX_HELLO`] bytes.

use super::delta::{DeltaKind, GaggleDelta};
use super::proto::{
    gaggle_server::{Gaggle, GaggleServer},
    manager_message::Message as ToWorker,
    worker_message::Message as FromWorker,
    Heartbeat, Hello, LogChunk, ManagerMessage, Reject, Stopped, Welcome, WorkerMessage,
    PROTOCOL_VERSION,
};
use crate::GooseError;
use prost::Message as _;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
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
/// admitted on it before it is closed: 10 seconds, and 2 in unit tests,
/// which wait it out; the process tests run the examples, built without
/// `cfg(test)`, so with the real deadline.
const HELLO_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_secs(2)
} else {
    Duration::from_secs(10)
};

/// The largest first message, `Hello`, the manager reads.
pub(crate) const MAX_HELLO: usize = 64 << 10;

/// The most data the manager holds sent but not yet read, across every
/// connection it allows: each connection's HTTP/2 stream and connection
/// window, and its [`MAX_HEADERS`], are a share of it (see [`window`]). The
/// 64 KiB floor takes it past this only with more than 1,634 workers, to
/// 160.3 MiB at 2,048.
const UNREAD_BUDGET: u64 = 128 << 20;

/// The smallest window a connection gets, HTTP/2's default.
const MIN_WINDOW: u64 = 64 << 10;

/// The largest window a connection gets.
const MAX_WINDOW: u64 = 1 << 20;

/// Each connection's HTTP/2 stream and connection window, when the manager
/// expects `expected` workers: [`UNREAD_BUDGET`] split between the
/// connections it allows, `expected + SPARE_CONNECTIONS`, less the
/// [`MAX_HEADERS`] each may also hold, from 64 KiB to 1 MiB. Windows and
/// headers together hold 6.1 MiB for 2 workers, the budget for 124, and
/// 160.3 MiB for 2,048, where the floor applies. A worker moves at most one
/// window each round trip: at 100 ms, 1 MiB is about 10 MB/s, for 2
/// workers, the 1,008 KiB 124 get a little less, and the 64 KiB that 2,048
/// get about 650 KB/s each, 1.3 GB/s for all of them. The windows are fixed
/// for the run, never grown with the link's bandwidth, so the budget holds
/// whatever a worker sends.
fn window(expected: usize) -> u32 {
    let connections = (expected as u64).saturating_add(SPARE_CONNECTIONS as u64);
    let share = (UNREAD_BUDGET / connections.max(1)).saturating_sub(u64::from(MAX_HEADERS));
    share.clamp(MIN_WINDOW, MAX_WINDOW) as u32
}

/// The largest header block the manager accepts.
const MAX_HEADERS: u32 = 16 << 10;

/// The connections the manager holds beyond one for each expected worker:
/// they absorb connections that never become workers, a client without the
/// token or a worker turned away, until their deadline. No worker rejoins:
/// once the Gaggle is full, `admit` refuses every other.
const SPARE_CONNECTIONS: usize = 4;

/// The events queued for the manager's main loop: past them, a worker's
/// reader waits, and HTTP/2 flow control slows the worker.
pub(crate) const EVENT_QUEUE: usize = 64;

/// The most bytes the events queued for the manager's main loop may hold,
/// counted as their messages' encoded sizes, taken before a delta is
/// decoded: past it, as past [`EVENT_QUEUE`] events, a worker's reader
/// waits. A decoded delta holds more than its encoded bytes, so this bounds
/// the queue's memory to a multiple of the budget, not the budget itself.
const EVENT_BYTES: u64 = 256 << 20;

/// An event as queued, holding its share of [`EVENT_BYTES`] until the main
/// loop takes it, or, for a delta, until the merge has applied it (see
/// [`taken`]).
type Queued = (Event, Option<tokio::sync::OwnedSemaphorePermit>);

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
    /// It holds its share of the event byte budget until the merge takes
    /// it, as its decoded delta waits for the merge until then.
    Delta {
        id: u32,
        delta: Box<GaggleDelta>,
        bytes: Option<tokio::sync::OwnedSemaphorePermit>,
    },
    /// Its users stopped: its final delta and `Stopped` follow.
    UsersStopped { id: u32 },
    /// Its users stopped, and its final delta was sent first.
    Stopped { id: u32, stopped: Stopped },
    /// A piece of one of its logs.
    Log { id: u32, chunk: LogChunk },
    /// It is gone: its heartbeat timed out, its stream closed before its
    /// final delta, or it sent something it should not have.
    Lost { id: u32, why: String },
}

/// What the manager sends one worker: its response stream ends once the
/// last sender is dropped and what was queued has left.
type Outbound = mpsc::Sender<Result<ManagerMessage, tonic::Status>>;

/// One admitted worker, as the manager's main loop and the worker's
/// heartbeat task see it.
#[derive(Clone)]
pub(crate) struct Joined {
    pub(crate) id: u32,
    /// Its name, unique in this Gaggle, safe as a folder name.
    pub(crate) name: String,
    /// The worker's one sender, taken when it is shut down, so its stream
    /// ends.
    outbound: Arc<Mutex<Option<Outbound>>>,
    /// Set once the worker is lost or shut down: nothing more is sent.
    closed: Arc<AtomicBool>,
    /// Set once its response stream has ended, everything queued sent.
    ended: Arc<AtomicBool>,
    /// Woken when it is closed, so its reader stops at once.
    closing: Arc<tokio::sync::Notify>,
}

impl Joined {
    /// Send `message` to the worker, unless it is closed; never waits. A
    /// message that finds the queue full or closed is dropped, with a
    /// warning naming it and the worker, which is the error.
    fn send(&self, message: ToWorker) -> Result<(), String> {
        let message = ManagerMessage {
            message: Some(message),
        };
        let kind = message_kind(&message);
        let outbound = self.outbound.lock().ok();
        let outbound = outbound.as_ref().and_then(|outbound| outbound.as_ref());
        let Some(outbound) = outbound.filter(|_| !self.closed.load(Ordering::Relaxed)) else {
            return Err(format!(
                "gaggle: worker {} ({}): {kind} not sent, its stream closed",
                self.id, self.name
            ));
        };
        if outbound.try_send(Ok(message)).is_err() {
            let warning = dropped(self.id, &self.name, kind);
            warn!("{}", warning);
            #[cfg(test)]
            tests::warned(&warning);
            return Err(warning);
        }
        Ok(())
    }

    /// Send `Shutdown`, then close the worker's stream: its sender is
    /// dropped, so the stream ends once `Shutdown` has left, and the
    /// worker's reader stops with it.
    fn shut_down(&self) {
        let _ = self.send(ToWorker::Shutdown(Default::default()));
        self.close();
    }

    /// Send nothing more, and end the stream once what is queued has left.
    fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        if let Ok(mut outbound) = self.outbound.lock() {
            outbound.take();
        }
        self.closing.notify_waiters();
    }
}

/// A worker's response stream, which marks the worker's `ended` once every
/// message queued for it has been taken.
struct Responses {
    queued: ReceiverStream<Result<ManagerMessage, tonic::Status>>,
    ended: Arc<AtomicBool>,
}

impl tokio_stream::Stream for Responses {
    type Item = Result<ManagerMessage, tonic::Status>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let next = Pin::new(&mut this.queued).poll_next(cx);
        if let Poll::Ready(None) = next {
            this.ended.store(true, Ordering::Relaxed);
        }
        next
    }
}

/// The warning a message to a worker that could not be queued logs.
fn dropped(id: u32, name: &str, kind: &str) -> String {
    format!("gaggle: worker {id} ({name}): {kind} not sent, its send queue full or closed")
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
    /// Each worker admitted, at its id: `joined` never shrinks while the
    /// manager runs, so an id is its worker's index.
    joined: Mutex<Vec<Joined>>,
    events: flume::Sender<Queued>,
    /// What is left of [`EVENT_BYTES`].
    event_bytes: Arc<tokio::sync::Semaphore>,
    plan_hash: u64,
    hash_check: bool,
    heartbeat_timeout: Duration,
    /// What unit tests watch and change.
    #[cfg(test)]
    test: TestHooks,
}

/// What unit tests watch and change in a manager.
#[cfg(test)]
#[derive(Default)]
struct TestHooks {
    /// Streams the handler ran for, past the token check.
    handled: std::sync::atomic::AtomicUsize,
    /// How long admitting a worker takes, in milliseconds.
    admit_delay_ms: std::sync::atomic::AtomicU64,
    /// Worker streams being read.
    reading: std::sync::atomic::AtomicUsize,
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

/// A value no token can be: a token is printable ASCII.
const NOT_A_TOKEN: &str = "\0not a token";

/// The token a request presents, `Bearer <token>` in its `authorization`
/// metadata: empty without one, and a value no token can be when it is not
/// a bearer token, so it matches no token, the empty one included.
fn presented_token(metadata: &tonic::metadata::MetadataMap) -> &str {
    match metadata.get(AUTHORIZATION).map(|value| value.to_str()) {
        None => "",
        Some(Ok(value)) => value.strip_prefix("Bearer ").unwrap_or(NOT_A_TOKEN),
        Some(Err(_)) => NOT_A_TOKEN,
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

/// The longest folder name a worker gets, any suffix included.
const MAX_FOLDER_NAME: usize = 64;

/// A worker name safe as a folder name on any system: letters, digits, `-`,
/// `_` and `.`, never `.` or `..` alone, never a Windows device name (see
/// [`is_device_name`]), which gets `worker-` before it, and at most
/// [`MAX_FOLDER_NAME`] characters.
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
        .take(MAX_FOLDER_NAME)
        .collect();
    // Windows drops a name's trailing dots: `pod.` would be `pod`.
    safe.truncate(safe.trim_end_matches('.').len());
    if safe.is_empty() {
        safe = "worker".to_string();
    }
    if is_device_name(&safe) {
        safe = format!("worker-{safe}");
        safe.truncate(MAX_FOLDER_NAME);
        // Cut short, it may end in a dot again.
        safe.truncate(safe.trim_end_matches('.').len());
    }
    safe
}

/// Whether Windows reserves `name` for a device, with or without an
/// extension, ignoring case: `CON`, `PRN`, `AUX`, `NUL`, `COM1` to `COM9` and
/// `LPT1` to `LPT9`.
fn is_device_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

/// `base`, a [`folder_name`], with `-n` after it, cut so the whole is at
/// most [`MAX_FOLDER_NAME`] characters.
fn suffixed(base: &str, n: u32) -> String {
    let suffix = format!("-{n}");
    // A folder name is ASCII, so any byte is a character boundary.
    let keep = base.len().min(MAX_FOLDER_NAME - suffix.len());
    format!("{}{suffix}", &base[..keep])
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

    /// Admit a worker named `name`: its id, unique name and stream, or why
    /// not: the Gaggle has all its workers, or the list of them can no
    /// longer be trusted, a thread having panicked while it held it. Names
    /// are unique ignoring case, as folders on a case insensitive file
    /// system are: one taken already gets the worker's id appended.
    fn admit(&self, name: &str, outbound: Outbound) -> Result<Joined, String> {
        let Ok(mut joined) = self.joined.lock() else {
            let why = "the manager's list of workers is unusable, a thread having panicked while it held it".to_string();
            error!("gaggle: could not admit a worker: {why}");
            return Err(why);
        };
        if joined.len() >= self.expected {
            return Err(format!(
                "the Gaggle has all {} of its workers already",
                self.expected
            ));
        }
        let id = joined.len() as u32;
        let base = folder_name(name);
        let mut unique = base.clone();
        let mut n = id;
        while joined
            .iter()
            .any(|worker| worker.name.eq_ignore_ascii_case(&unique))
        {
            unique = suffixed(&base, n);
            n += 1;
        }
        let worker = Joined {
            id,
            name: unique,
            outbound: Arc::new(Mutex::new(Some(outbound))),
            closed: Arc::new(AtomicBool::new(false)),
            ended: Arc::new(AtomicBool::new(false)),
            closing: Default::default(),
        };
        joined.push(worker.clone());
        Ok(worker)
    }
}

/// The gRPC service.
struct GaggleService(Arc<Shared>);

/// A refusal sent as the stream's only message.
fn reject(reason: String) -> Responses {
    let (tx, rx) = mpsc::channel(1);
    let _ = tx.try_send(Ok(ManagerMessage {
        message: Some(ToWorker::Reject(Reject { reason })),
    }));
    Responses {
        queued: ReceiverStream::new(rx),
        ended: Arc::new(AtomicBool::new(false)),
    }
}

#[tonic::async_trait]
impl Gaggle for GaggleService {
    type ConnectStream = Responses;

    async fn connect(
        &self,
        request: tonic::Request<tonic::Streaming<WorkerMessage>>,
    ) -> Result<tonic::Response<Self::ConnectStream>, tonic::Status> {
        #[cfg(test)]
        self.0
            .test
            .handled
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let connection = request.extensions().get::<Peer>().cloned();
        let peer = peer_name(connection.as_ref().and_then(|connection| connection.addr));
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
        // Held open from now on, so the deadline cannot close the
        // connection of a worker as it is admitted.
        if let Some(connection) = connection.as_ref() {
            connection.admitted.store(true, Ordering::Relaxed);
        }
        #[cfg(test)]
        {
            let delay = self.0.test.admit_delay_ms.load(Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        let (outbound, rx) = mpsc::channel(64);
        let worker = match self.0.admit(&hello.worker_name, outbound) {
            Ok(worker) => worker,
            Err(reason) => {
                if let Some(connection) = connection.as_ref() {
                    connection.admitted.store(false, Ordering::Relaxed);
                }
                warn!(
                    "gaggle: refused worker {:?} from {peer}: {reason}",
                    folder_name(&hello.worker_name)
                );
                return Ok(tonic::Response::new(reject(reason)));
            }
        };
        let id = worker.id;
        info!("gaggle: worker {id} ({}) joined from {peer}", worker.name);
        let _ = worker.send(ToWorker::Welcome(Welcome { worker_id: id }));
        let _ = self.0.events.send_async((Event::Joined, None)).await;
        let heard = Arc::new(Mutex::new(Instant::now()));
        let lost = Arc::new(AtomicBool::new(false));
        let responses = Responses {
            queued: ReceiverStream::new(rx),
            ended: worker.ended.clone(),
        };
        tokio::spawn(read_worker(
            worker.clone(),
            inbound,
            self.0.clone(),
            heard.clone(),
            lost.clone(),
        ));
        tokio::spawn(heartbeat_worker(worker, self.0.clone(), heard, lost));
        Ok(tonic::Response::new(responses))
    }
}

/// A peer's address for a log line, `unknown` without one.
fn peer_name(addr: Option<SocketAddr>) -> String {
    addr.map_or_else(|| "unknown".to_string(), |addr| addr.to_string())
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
        if this.expired(cx) {
            return Poll::Ready(Err(Counted::too_late()));
        }
        Pin::new(&mut this.stream).poll_read(cx, buf)
    }
}

impl Counted {
    /// Whether no worker was admitted on the connection in time. Checked
    /// on every read and write, so a client that stalls the manager's
    /// writes, by never reading, cannot keep the connection past it.
    fn expired(&mut self, cx: &mut Context<'_>) -> bool {
        !self.peer.admitted.load(Ordering::Relaxed) && self.admit_by.as_mut().poll(cx).is_ready()
    }

    fn too_late() -> std::io::Error {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "no worker was admitted on this connection in time",
        )
    }
}

impl tokio::io::AsyncWrite for Counted {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        if this.expired(cx) {
            return Poll::Ready(Err(Counted::too_late()));
        }
        Pin::new(&mut this.stream).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        if this.expired(cx) {
            return Poll::Ready(Err(Counted::too_late()));
        }
        Pin::new(&mut this.stream).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.expired(cx) {
            return Poll::Ready(Err(Counted::too_late()));
        }
        Pin::new(&mut this.stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        // Past the deadline the connection is dropped without waiting for
        // its peer to read what is left.
        if this.expired(cx) {
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.stream).poll_shutdown(cx)
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
                        peer_name(addr)
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
        let _ = shared
            .events
            .send_async((Event::Lost { id, why }, None))
            .await;
    }
}

/// Read worker `id`'s stream until it ends, as events. The event queue is
/// bounded: while it is full this waits, reading nothing more, so HTTP/2
/// flow control holds the worker back. Nothing the main loop waits on is
/// held here while it waits: the main loop only ever takes events and
/// sends with `try_send`, and the lock on `heard` is released before.
/// `heard` is refreshed as each message arrives, before it is queued, and
/// again while a full queue holds it, so a worker held back by a slow main
/// loop is never taken for a silent one.
async fn read_worker(
    worker: Joined,
    mut inbound: tonic::Streaming<WorkerMessage>,
    shared: Arc<Shared>,
    heard: Arc<Mutex<Instant>>,
    lost: Arc<AtomicBool>,
) {
    #[cfg(test)]
    let _reading = Reading::new(&shared);
    let id = worker.id;
    let mut got_final = false;
    loop {
        // Created before the check, so a close between them still wakes it.
        let closing = worker.closing.notified();
        if worker.closed.load(Ordering::Relaxed) {
            break;
        }
        let message = tokio::select! {
            message = inbound.message() => message,
            () = closing => break,
        };
        let message = match message {
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
        if matches!(message.message, Some(FromWorker::Heartbeat(_)) | None) {
            continue;
        }
        // Its share of the byte budget first, before a delta is decoded, so
        // no reader holds a decoded delta outside the budget.
        let bytes = (message.encoded_len() as u64).min(EVENT_BYTES) as u32;
        let permit = held(
            shared.event_bytes.clone().acquire_many_owned(bytes),
            &worker,
            &shared,
            &heard,
        )
        .await;
        let Some(permit) = permit else {
            break;
        };
        let event = match message.message {
            // Skipped before the budget was taken.
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
                        bytes: None,
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
            Some(FromWorker::UsersStopped(_)) => Event::UsersStopped { id },
            Some(FromWorker::Stopped(stopped)) => Event::Stopped { id, stopped },
            Some(FromWorker::LogChunk(chunk)) => Event::Log { id, chunk },
        };
        let queued = held(
            shared.events.send_async((event, permit.ok())),
            &worker,
            &shared,
            &heard,
        )
        .await;
        if !matches!(queued, Some(Ok(()))) {
            break;
        }
    }
    // A worker that stopped as told, then went away, is not lost: the
    // manager's main loop knows it stopped and keeps it so. Either way the
    // event ends any wait for it, its logs among them.
    if !worker.closed.load(Ordering::Relaxed) {
        let why = if got_final {
            "its stream closed after its final metrics, before Shutdown"
        } else {
            "its stream closed before its final metrics"
        };
        lose(&shared, id, &lost, why.to_string()).await;
    }
}

/// Wait for `future`, which the manager holds back, as when its event queue
/// is full: `heard` is kept fresh meanwhile, as the worker has not gone
/// silent. `None` once the worker is closed.
async fn held<T>(
    future: impl Future<Output = T>,
    worker: &Joined,
    shared: &Shared,
    heard: &Mutex<Instant>,
) -> Option<T> {
    // Created before the check, so a close between them still wakes it.
    let closing = worker.closing.notified();
    if worker.closed.load(Ordering::Relaxed) {
        return None;
    }
    let (mut future, mut closing) = (std::pin::pin!(future), std::pin::pin!(closing));
    let mut holding = tokio::time::interval(HEARTBEAT_INTERVAL.min(shared.heartbeat_timeout / 2));
    loop {
        tokio::select! {
            out = &mut future => return Some(out),
            () = &mut closing => return None,
            _ = holding.tick() => {
                if let Ok(mut heard) = heard.lock() {
                    *heard = Instant::now();
                }
            }
        }
    }
}

/// Counts a worker's reader while it runs (unit tests).
#[cfg(test)]
struct Reading(Arc<Shared>);

#[cfg(test)]
impl Reading {
    fn new(shared: &Arc<Shared>) -> Self {
        shared.test.reading.fetch_add(1, Ordering::Relaxed);
        Reading(shared.clone())
    }
}

#[cfg(test)]
impl Drop for Reading {
    fn drop(&mut self) {
        self.0.test.reading.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Send `worker` a heartbeat every [`HEARTBEAT_INTERVAL`], and report it
/// lost, send it `Shutdown` and close its stream when nothing has come from
/// it for the heartbeat timeout.
async fn heartbeat_worker(
    worker: Joined,
    shared: Arc<Shared>,
    heard: Arc<Mutex<Instant>>,
    lost: Arc<AtomicBool>,
) {
    let id = worker.id;
    let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL.min(shared.heartbeat_timeout / 2));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        if worker.closed.load(Ordering::Relaxed) || worker.ended.load(Ordering::Relaxed) {
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
                format!("nothing from it for {:?}", shared.heartbeat_timeout),
            )
            .await;
            worker.shut_down();
            return;
        }
        let _ = worker.send(ToWorker::Heartbeat(Heartbeat {}));
    }
}

/// An event as the main loop takes it: a delta keeps its share of the byte
/// budget, for the merge to free once it has taken the delta; any other
/// event frees its share now.
fn taken((mut event, permit): Queued) -> Event {
    if let Event::Delta { bytes, .. } = &mut event {
        *bytes = permit;
    }
    event
}

/// Worker `id` of `joined`: the one at its index, which ids follow, whose
/// own id it is.
fn find(joined: &[Joined], id: u32) -> Option<&Joined> {
    joined.get(id as usize).filter(|worker| worker.id == id)
}

/// A running manager: its listening service and its workers.
pub(crate) struct Manager {
    shared: Arc<Shared>,
    events: flume::Receiver<Queued>,
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
            event_bytes: Arc::new(tokio::sync::Semaphore::new(EVENT_BYTES as usize)),
            plan_hash,
            hash_check,
            heartbeat_timeout,
            #[cfg(test)]
            test: Default::default(),
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
        let window = window(expected);
        let server = tokio::spawn(async move {
            // No HTTP/2 keepalive: the heartbeats find a dead worker.
            let served = tonic::transport::Server::builder()
                .max_concurrent_streams(1)
                .concurrency_limit_per_connection(1)
                .initial_stream_window_size(window)
                .initial_connection_window_size(window)
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
        let queued = tokio::time::timeout(wait, self.events.recv_async())
            .await
            .ok()?
            .ok()?;
        Some(taken(queued))
    }

    /// The next event if one is waiting.
    pub(crate) fn try_event(&self) -> Option<Event> {
        self.events.try_recv().ok().map(taken)
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
            if let Some(worker) = find(&joined, id) {
                let _ = worker.send(message);
            }
        }
    }

    /// Send every worker still connected `message`.
    pub(crate) fn broadcast(&self, message: ToWorker) {
        if let Ok(joined) = self.shared.joined.lock() {
            for worker in joined.iter() {
                let _ = worker.send(message.clone());
            }
        }
    }

    /// Send worker `id` `Shutdown` and close its stream: a lost worker. One
    /// that gets it stops any user still running at once and exits within 2
    /// seconds, even with its main loop held; one it never reaches fires
    /// its own dead man's switch when its stream closes or goes silent.
    pub(crate) fn shut_down(&self, id: u32) {
        if let Ok(joined) = self.shared.joined.lock() {
            if let Some(worker) = find(&joined, id) {
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
        // Each stream ends once its `Shutdown` has left: wait for that, at
        // most a second, before the service stops.
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            let drained = self.shared.joined.lock().map_or(true, |joined| {
                joined.iter().all(|w| w.ended.load(Ordering::Relaxed))
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
                worker.close();
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
    use super::handshake_tests::test_manager;
    use super::*;

    /// Every warning a dropped message logged.
    static WARNED: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// Keep `warning`, logged for a dropped message.
    pub(super) fn warned(warning: &str) {
        if let Ok(mut warned) = WARNED.lock() {
            warned.push(warning.to_string());
        }
    }

    /// A connection on which no worker is admitted is closed at its
    /// deadline even when the manager's writes are stalled, its peer never
    /// reading: the deadline is checked on writes too.
    #[tokio::test]
    async fn an_unadmitted_connection_closes_with_its_writes_stalled() {
        let test = async {
            use tokio::io::AsyncWriteExt;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
                .await
                .unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            let mut counted = Counted {
                stream,
                peer: Peer {
                    addr: None,
                    admitted: Arc::new(AtomicBool::new(false)),
                },
                admit_by: Box::pin(tokio::time::sleep(Duration::from_millis(500))),
                _permit: Arc::new(tokio::sync::Semaphore::new(1))
                    .try_acquire_owned()
                    .unwrap(),
            };
            let chunk = vec![0u8; 64 << 10];
            let error = loop {
                if let Err(e) = counted.write_all(&chunk).await {
                    break e;
                }
            };
            assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
            drop(client);
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("the stalled connection closes within 30 seconds");
    }

    #[test]
    fn worker_names_are_safe_folder_names() {
        assert_eq!(folder_name("gaggle2-worker-0"), "gaggle2-worker-0");
        assert_eq!(folder_name("../etc"), "..-etc");
        assert_eq!(folder_name(".."), "worker");
        assert_eq!(folder_name(""), "worker");
        // Windows would drop the trailing dots, making it `pod`.
        assert_eq!(folder_name("pod.."), "pod");
        // A device name made longer and cut short ends in no dot either.
        let device = folder_name(&format!("CON.{}.{}", "a".repeat(52), "b".repeat(7)));
        assert!(!device.ends_with('.') && device.len() <= 63, "{}", device);
        assert_eq!(folder_name("a/b\\c d"), "a-b-c-d");
        assert_eq!(folder_name(&"x".repeat(100)).len(), 64);
        // With a suffix, still 64 at most.
        let long = folder_name(&"x".repeat(100));
        assert_eq!(suffixed(&long, 12), format!("{}-12", "x".repeat(61)));
        assert_eq!(suffixed("pod", 2), "pod-2");
        // Windows device names, with an extension or in any case.
        for device in ["CON", "con", "Nul.txt", "aux", "PRN", "COM1", "lpt9.log"] {
            assert_eq!(folder_name(device), format!("worker-{device}"));
        }
        for name in ["COM10", "COM0", "console", "nullify", "LPT", "auxiliary.x"] {
            assert_eq!(folder_name(name), name);
        }
    }

    /// Worker names that differ only in case get folders of their own.
    #[tokio::test]
    async fn worker_names_are_unique_ignoring_case() {
        let manager = test_manager(5, Duration::from_secs(10)).await;
        let (tx, _) = mpsc::channel(1);
        for name in ["Pod-A", "pod-a", "POD-A"] {
            assert!(manager.shared.admit(name, tx.clone()).is_ok());
        }
        // Two of the longest names: the second's suffix keeps it at 64.
        let long = "x".repeat(100);
        assert!(manager.shared.admit(&long, tx.clone()).is_ok());
        let suffixed = manager.shared.admit(&long, tx.clone()).ok().unwrap();
        assert_eq!(suffixed.name, format!("{}-4", "x".repeat(62)));
        assert_eq!(suffixed.name.len(), 64);
        let names: Vec<String> = manager
            .workers()
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        assert_eq!(names[..3], ["Pod-A", "pod-a-1", "POD-A-2"]);
    }

    /// The window budget, less each connection's header allowance, is split
    /// between the connections the manager allows, 1 MiB at most and 64 KiB
    /// at least: windows and headers together hold the budget, but for the
    /// floor with more than 1,634 workers.
    #[test]
    fn windows_share_the_unread_budget() {
        let headers = u64::from(MAX_HEADERS);
        assert_eq!(window(2), 1 << 20);
        assert_eq!(u64::from(window(124)), (1 << 20) - headers);
        assert_eq!(u64::from(window(125)), UNREAD_BUDGET / 129 - headers);
        assert_eq!(window(1634), (UNREAD_BUDGET / 1638 - headers) as u32);
        assert_eq!(window(1635), 64 << 10);
        assert_eq!(window(2048), 64 << 10);
        // Every connection's window and headers, together: 6 MiB and 96 KiB
        // for 2 workers, the budget for 124, and 160.3 MiB for 2,048.
        assert_eq!((u64::from(window(2)) + headers) * 6, (6 << 20) + (96 << 10));
        assert_eq!((u64::from(window(124)) + headers) * 128, UNREAD_BUDGET);
        assert_eq!(
            (u64::from(window(2048)) + headers) * 2052,
            2052 * (80 << 10)
        );
    }

    /// A Gaggle that is full and one whose worker list is unusable refuse a
    /// worker each for its own reason.
    #[tokio::test]
    async fn admit_names_why_it_refuses() {
        let manager = test_manager(1, Duration::from_secs(10)).await;
        let (tx, _) = mpsc::channel(1);
        assert!(manager.shared.admit("a", tx.clone()).is_ok());
        let full = manager.shared.admit("b", tx.clone()).err().unwrap();
        assert!(full.contains("all 1 of its workers"), "{}", full);
        let shared = manager.shared.clone();
        let _ = std::thread::spawn(move || {
            let _held = shared.joined.lock().unwrap();
            panic!("poison the worker list");
        })
        .join();
        let poisoned = manager.shared.admit("c", tx).err().unwrap();
        assert!(poisoned.contains("panicked"), "{}", poisoned);
    }

    /// A message that finds a worker's queue full is dropped, and the
    /// warning names the message and the worker.
    #[tokio::test]
    async fn a_dropped_message_is_named() {
        let (outbound, _rx) = mpsc::channel(1);
        let worker = Joined {
            id: 3,
            name: "w".to_string(),
            outbound: Arc::new(Mutex::new(Some(outbound))),
            closed: Arc::new(AtomicBool::new(false)),
            ended: Arc::new(AtomicBool::new(false)),
            closing: Default::default(),
        };
        let warning = "gaggle: worker 3 (w): Stop not sent, its send queue full or closed";
        assert_eq!(worker.send(ToWorker::Heartbeat(Heartbeat {})), Ok(()));
        assert_eq!(
            worker.send(ToWorker::Stop(Default::default())),
            Err(warning.to_string())
        );
        // It was logged, not only returned.
        assert!(WARNED
            .lock()
            .unwrap()
            .iter()
            .any(|logged| logged == warning));
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
        // Only a bearer token: the right token on its own is refused.
        let bare = |value: &str| {
            let mut request = tonic::Request::new(());
            request
                .metadata_mut()
                .insert(AUTHORIZATION, value.parse().unwrap());
            authorize(&token_digest("secret"), request)
                .err()
                .map(|status| (status.code(), status.message().to_string()))
        };
        for value in ["secret", "bearer secret", "Basic secret", "Bearer"] {
            assert_eq!(bare(value), wrong, "{:?}", value);
        }
        assert_eq!(bare("Bearer secret"), None);
    }
}

#[cfg(test)]
pub(crate) mod handshake_tests {
    use super::super::proto::gaggle_client::GaggleClient;
    use super::*;

    /// A manager on a port the system picks, for `expected` workers of plan
    /// hash 7, with no token, checking the plan hash, and a heartbeat
    /// timeout of `heartbeat`.
    pub(crate) async fn test_manager(expected: usize, heartbeat: Duration) -> Manager {
        Manager::start("127.0.0.1", 0, expected, None, 7, true, heartbeat)
            .await
            .unwrap()
    }

    /// A channel to `manager`.
    pub(crate) async fn channel(manager: &Manager) -> tonic::transport::Channel {
        tonic::transport::Endpoint::from_shared(format!("http://{}", manager.local_addr))
            .unwrap()
            .connect()
            .await
            .unwrap()
    }

    /// Open a stream on `channel` that says `hello`, presenting `token`:
    /// what is left to send on it, and its answer.
    pub(crate) async fn open(
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

    /// A manager with a token refuses a stream presenting an empty `Bearer `
    /// token, or none, before its handler runs.
    #[tokio::test]
    async fn an_empty_bearer_token_is_refused() {
        let test = async {
            let manager = Manager::start(
                "127.0.0.1",
                0,
                1,
                Some("tok-empty-check".to_string()),
                7,
                true,
                Duration::from_secs(30),
            )
            .await
            .unwrap();
            for token in [Some(""), None] {
                let answer = answer(&manager, hello(), token).await;
                assert_eq!(answer, Err(WRONG_TOKEN.to_string()), "{:?}", token);
            }
            assert_eq!(handled(&manager), 0);
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// The streams the handler ran for.
    fn handled(manager: &Manager) -> usize {
        manager
            .shared
            .test
            .handled
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// A worker that says Hello and then nothing: the manager loses it once
    /// its heartbeat timeout passes, and tells it to shut down.
    #[tokio::test]
    async fn gaggle_loses_a_silent_worker() {
        let test = async {
            let manager = test_manager(1, Duration::from_secs(1)).await;
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

    /// `shut_down` closes the worker's stream: `Shutdown` arrives, the
    /// stream from the manager ends promptly, and the manager stops
    /// reading the worker, though the worker's own stream stays open.
    #[tokio::test]
    async fn shut_down_closes_the_stream() {
        let test = async {
            let manager = test_manager(1, Duration::from_secs(10)).await;
            let (tx, inbound) = open(channel(&manager).await, hello(), None).await;
            let mut inbound = inbound.unwrap();
            let reading = || manager.shared.test.reading.load(Ordering::Relaxed);
            while reading() == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            manager.shut_down(0);
            let shut_down = Instant::now();
            let ended = tokio::time::timeout(Duration::from_secs(5), async {
                let mut got_shutdown = false;
                loop {
                    match inbound.message().await {
                        Ok(Some(message)) => {
                            got_shutdown |= matches!(message.message, Some(ToWorker::Shutdown(_)))
                        }
                        Ok(None) | Err(_) => return got_shutdown,
                    }
                }
            })
            .await
            .expect("the stream ends");
            assert!(ended, "the stream ended without Shutdown");
            while reading() > 0 {
                assert!(
                    shut_down.elapsed() < Duration::from_secs(5),
                    "the manager still reads the worker"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
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
            let manager = test_manager(1, Duration::from_secs(10)).await;
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

    pub(crate) fn hello() -> Hello {
        Hello {
            goose_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: PROTOCOL_VERSION,
            plan_hash: 7,
            worker_name: "w".to_string(),
        }
    }

    /// Through a client of the test's own: a worker of another protocol
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

    /// A delta the main loop takes keeps its share of the byte budget, as
    /// its decoded form goes on to wait for the merge; it frees it only when
    /// the event, and so the delta, is dropped. Any other event frees it as
    /// it is taken.
    #[tokio::test]
    async fn a_taken_delta_holds_its_bytes_until_dropped() {
        use super::super::delta::{DeltaKind, GaggleDelta};
        use super::super::proto::Metrics;
        let test = async {
            let manager = test_manager(1, Duration::from_secs(30)).await;
            let (tx, inbound) = open(channel(&manager).await, hello(), None).await;
            let _inbound = inbound.unwrap();
            let free = || manager.shared.event_bytes.available_permits();
            let all = free();
            let delta = GaggleDelta::new(0, 1, 0, DeltaKind::Delta);
            tx.send(WorkerMessage {
                message: Some(FromWorker::Metrics(Metrics {
                    delta: delta.encode().unwrap(),
                })),
            })
            .await
            .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            let taken = loop {
                match manager.next_event(deadline).await {
                    Some(Event::Joined) => continue,
                    other => break other,
                }
            };
            assert!(matches!(taken, Some(Event::Delta { .. })), "{:?}", taken);
            assert!(free() < all, "the delta's bytes were freed as it was taken");
            drop(taken);
            assert_eq!(free(), all);
            drop(tx);
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// A worker held back by a full event queue for longer than the
    /// heartbeat timeout is not lost: the manager, not the worker, is what
    /// holds it.
    #[tokio::test]
    async fn a_worker_held_by_a_full_queue_is_not_lost() {
        use super::super::proto::LogChunk;
        let test = async {
            let manager = test_manager(1, Duration::from_secs(1)).await;
            let (tx, inbound) = open(channel(&manager).await, hello(), None).await;
            let _inbound = inbound.unwrap();
            const CHUNKS: u64 = 2 * EVENT_QUEUE as u64;
            let sender = tokio::spawn(async move {
                for offset in 0..CHUNKS {
                    let chunk = LogChunk {
                        file: "requests.json".to_string(),
                        offset,
                        data: vec![b'x'; 1 << 10],
                        last: offset + 1 == CHUNKS,
                        error: None,
                        size: None,
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
            // Three heartbeat timeouts with the queue full and nothing taken.
            tokio::time::sleep(Duration::from_secs(3)).await;
            assert_eq!(manager.events.len(), EVENT_QUEUE);
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut chunks = 0;
            while chunks < CHUNKS {
                match manager.next_event(deadline).await {
                    Some(Event::Log { .. }) => chunks += 1,
                    Some(Event::Joined) => {}
                    other => panic!("{:?}", other),
                }
            }
            drop(sender.await);
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
            let manager = test_manager(1, Duration::from_secs(30)).await;
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
                        error: None,
                        size: None,
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

    /// A worker's reader waits once the events queued hold the byte budget,
    /// though fewer than EVENT_QUEUE are queued, and goes on as the main
    /// loop takes them: every chunk arrives.
    #[tokio::test]
    async fn gaggle_event_queue_is_bounded_by_bytes() {
        use super::super::proto::LogChunk;
        let test = async {
            let manager = test_manager(1, Duration::from_secs(30)).await;
            // A budget of 1 MiB, for the test: 4 chunks of 256 KiB fill it.
            manager
                .shared
                .event_bytes
                .forget_permits((EVENT_BYTES - (1 << 20)) as usize);
            let (tx, inbound) = open(channel(&manager).await, hello(), None).await;
            let _inbound = inbound.unwrap();
            const CHUNKS: u64 = 12;
            let sender = tokio::spawn(async move {
                for offset in 0..CHUNKS {
                    let chunk = LogChunk {
                        file: "requests.json".to_string(),
                        offset,
                        data: vec![b'x'; (256 << 10) - 64],
                        last: offset + 1 == CHUNKS,
                        error: None,
                        size: None,
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
            // Nothing taken: the reader fills the budget, then stops there.
            let filled = Instant::now() + Duration::from_secs(10);
            while manager.events.len() < 4 {
                assert!(
                    Instant::now() < filled,
                    "{} events queued",
                    manager.events.len()
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
            let queued = manager.events.len();
            // The Joined event, and what fits in 1 MiB.
            assert!(queued <= 5, "{} events queued past the byte budget", queued);
            assert!(queued >= 4, "{} events queued", queued);
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut chunks = 0;
            while chunks < CHUNKS {
                match manager.next_event(deadline).await {
                    Some(Event::Log { .. }) => chunks += 1,
                    Some(Event::Joined) => {}
                    other => panic!("{:?}", other),
                }
            }
            drop(sender.await);
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
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
            let manager = test_manager(1, Duration::from_secs(10)).await;
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
            let manager = test_manager(2, Duration::from_secs(10)).await;
            let channel = channel(&manager).await;
            let (first_tx, first) = open(channel.clone(), hello(), None).await;
            let mut first = first.unwrap();
            assert!(matches!(
                first.message().await.unwrap().unwrap().message,
                Some(ToWorker::Welcome(_))
            ));
            let second =
                tokio::time::timeout(Duration::from_secs(3), open(channel.clone(), hello(), None))
                    .await;
            assert!(second.is_err(), "a second stream was served");
            assert_eq!(handled(&manager), 1);
            drop(first_tx);
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// A worker whose Hello comes just before its connection's deadline,
    /// and whose admission takes past it, is admitted and kept: its
    /// connection counts as admitted from before the deadline.
    #[tokio::test]
    async fn a_worker_admitted_at_its_deadline_is_kept() {
        let test = async {
            let manager = test_manager(1, Duration::from_secs(60)).await;
            manager
                .shared
                .test
                .admit_delay_ms
                .store(2_000, Ordering::Relaxed);
            let connected = Instant::now();
            let channel = channel(&manager).await;
            let (tx, rx) = mpsc::channel(4);
            let joining = tokio::spawn(async move {
                GaggleClient::new(channel)
                    .connect(tonic::Request::new(ReceiverStream::new(rx)))
                    .await
                    .map(tonic::Response::into_inner)
            });
            tokio::time::sleep_until((connected + HELLO_TIMEOUT - Duration::from_secs(1)).into())
                .await;
            tx.send(WorkerMessage {
                message: Some(FromWorker::Hello(hello())),
            })
            .await
            .unwrap();
            let mut inbound = joining.await.unwrap().expect("admitted");
            assert!(matches!(
                inbound.message().await.unwrap().unwrap().message,
                Some(ToWorker::Welcome(_))
            ));
            // Still open, past the deadline: heartbeats keep coming.
            let until = Instant::now() + Duration::from_secs(2);
            while Instant::now() < until {
                let message = tokio::time::timeout(Duration::from_secs(5), inbound.message())
                    .await
                    .expect("a heartbeat");
                assert!(matches!(message, Ok(Some(_))), "{:?}", message);
            }
            drop(tx);
        };
        tokio::time::timeout(Duration::from_secs(60), test)
            .await
            .expect("within 60 seconds");
    }

    /// A connection whose worker the manager refuses, the Gaggle full,
    /// is not held as admitted: it is closed at its deadline, and its
    /// permit freed, though its client keeps it open.
    #[tokio::test]
    async fn a_connection_refused_when_full_still_closes_at_its_deadline() {
        use tokio::io::AsyncReadExt;
        let test = async {
            let manager = test_manager(1, Duration::from_secs(60)).await;
            assert_eq!(answer(&manager, hello(), None).await, Ok(0));
            // The admitted worker's connection closes with its client.
            tokio::time::sleep(Duration::from_millis(500)).await;
            // Every permit, each held by a refused worker's connection.
            let mut refused = Vec::new();
            for _ in 0..=SPARE_CONNECTIONS {
                let (tx, inbound) = open(
                    channel(&manager).await,
                    Hello {
                        worker_name: "b".to_string(),
                        ..hello()
                    },
                    None,
                )
                .await;
                let mut inbound = inbound.unwrap();
                match inbound.message().await.unwrap().and_then(|m| m.message) {
                    Some(ToWorker::Reject(reject)) => {
                        assert!(
                            reject.reason.contains("all 1 of its workers"),
                            "{}",
                            reject.reason
                        )
                    }
                    other => panic!("{:?}", other),
                }
                refused.push((tx, inbound));
            }
            tokio::time::sleep(HELLO_TIMEOUT + Duration::from_secs(1)).await;
            // Their permits are free again: a new connection is held open.
            let mut fresh = tokio::net::TcpStream::connect(manager.local_addr)
                .await
                .unwrap();
            let mut buffer = [0u8; 1024];
            let held = tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    match fresh.read(&mut buffer).await {
                        Ok(0) | Err(_) => break,
                        // The server's HTTP/2 preface.
                        Ok(_) => continue,
                    }
                }
            })
            .await;
            assert!(held.is_err(), "a refused connection kept its permit");
            drop(refused);
        };
        tokio::time::timeout(Duration::from_secs(60), test)
            .await
            .expect("within 60 seconds");
    }

    /// The manager holds at most `expected + 4` connections: one more is
    /// closed at once, and one on which no worker is admitted is closed
    /// after HELLO_TIMEOUT, so a worker can join again.
    #[tokio::test]
    async fn gaggle_holds_a_bounded_number_of_connections() {
        use tokio::io::AsyncReadExt;
        let test = async {
            let manager = test_manager(1, Duration::from_secs(30)).await;
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
                let closed = tokio::time::timeout(HELLO_TIMEOUT + Duration::from_secs(5), async {
                    let mut buffer = [0u8; 1024];
                    loop {
                        match connection.read(&mut buffer).await {
                            Ok(0) | Err(_) => break,
                            // The server's HTTP/2 preface.
                            Ok(_) => continue,
                        }
                    }
                })
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
