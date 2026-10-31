//! The manager's side of the wire: a gRPC service that admits exactly the
//! workers it expects, then turns each worker's stream into events for the
//! manager's main loop, and watches each worker's heartbeat.

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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

/// The largest message either side sends or accepts: the largest custom data
/// a worker may be handed, a final delta, or a log chunk, with room to spare.
pub(crate) const MAX_MESSAGE: usize = 64 << 20;

/// How often the manager and a worker send each other a heartbeat.
pub(crate) const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);

/// How long a new connection has to say `Hello`.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

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
        if self.outbound.try_send(Ok(message)).is_err() {
            debug!("gaggle: worker {} send queue full or closed", self.name);
        }
    }

    /// Send `Shutdown`, then close the worker's stream.
    fn shut_down(&self) {
        self.send(ToWorker::Shutdown(Default::default()));
        self.closed.store(true, Ordering::Relaxed);
    }
}

/// What the service shares with the manager.
struct Shared {
    expected: usize,
    joined: Mutex<Vec<Joined>>,
    events: flume::Sender<Event>,
    token: Option<String>,
    plan_hash: u64,
    hash_check: bool,
    heartbeat_timeout: Duration,
}

/// Compare two tokens in time that does not depend on where they differ.
fn same_token(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
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
    /// Why `hello` is refused, naming the field at fault.
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
        let token = self.token.as_deref().unwrap_or("");
        if !same_token(&hello.token, token) {
            return Some("token: the worker's --gaggle-token is not the manager's".to_string());
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
        let mut unique = folder_name(name);
        if joined.iter().any(|worker| worker.name == unique) {
            unique = format!("{unique}-{id}");
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
struct Service(Arc<Shared>);

/// A refusal sent as the stream's only message.
fn reject(reason: String) -> ReceiverStream<Result<ManagerMessage, tonic::Status>> {
    let (tx, rx) = mpsc::channel(1);
    let _ = tx.try_send(Ok(ManagerMessage {
        message: Some(ToWorker::Reject(Reject { reason })),
    }));
    ReceiverStream::new(rx)
}

#[tonic::async_trait]
impl Gaggle for Service {
    type ConnectStream = ReceiverStream<Result<ManagerMessage, tonic::Status>>;

    async fn connect(
        &self,
        request: tonic::Request<tonic::Streaming<WorkerMessage>>,
    ) -> Result<tonic::Response<Self::ConnectStream>, tonic::Status> {
        let peer = request
            .remote_addr()
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
                hello.worker_name
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
                hello.worker_name
            );
            return Ok(tonic::Response::new(reject(reason)));
        };
        info!("gaggle: worker {id} ({name}) joined from {peer}");
        let _ = outbound.try_send(Ok(ManagerMessage {
            message: Some(ToWorker::Welcome(Welcome { worker_id: id })),
        }));
        let _ = self.0.events.send(Event::Joined);
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

/// Report worker `id` lost, once.
fn lose(shared: &Shared, id: u32, lost: &AtomicBool, why: String) {
    if !lost.swap(true, Ordering::Relaxed) {
        let _ = shared.events.send(Event::Lost { id, why });
    }
}

/// Read worker `id`'s stream until it ends, as events.
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
                lose(&shared, id, &lost, "it said Hello twice".to_string());
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
                    );
                    break;
                }
            },
            Some(FromWorker::Stopped(stopped)) => Event::Stopped { id, stopped },
            Some(FromWorker::LogChunk(chunk)) => Event::Log { id, chunk },
        };
        if shared.events.send(event).is_err() {
            break;
        }
    }
    if !got_final && !closed.load(Ordering::Relaxed) {
        lose(
            &shared,
            id,
            &lost,
            "its stream closed before its final metrics".to_string(),
        );
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
            );
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
        let (events_tx, events) = flume::unbounded();
        let shared = Arc::new(Shared {
            expected,
            joined: Mutex::new(Vec::new()),
            events: events_tx,
            token,
            plan_hash,
            hash_check,
            heartbeat_timeout,
        });
        let service = GaggleServer::new(Service(shared.clone()))
            .max_decoding_message_size(MAX_MESSAGE)
            .max_encoding_message_size(MAX_MESSAGE);
        let (stop_server, stopped) = tokio::sync::oneshot::channel::<()>();
        let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
        let server = tokio::spawn(async move {
            let served = tonic::transport::Server::builder()
                .http2_keepalive_interval(Some(HEARTBEAT_INTERVAL))
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

impl Drop for Manager {
    fn drop(&mut self) {
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
        assert_eq!(folder_name("gaggle2-worker-0"), "gaggle2-worker-0");
        assert_eq!(folder_name("../etc"), "..-etc");
        assert_eq!(folder_name(".."), "worker");
        assert_eq!(folder_name(""), "worker");
        assert_eq!(folder_name("a/b\\c d"), "a-b-c-d");
        assert_eq!(folder_name(&"x".repeat(100)).len(), 64);
    }

    #[test]
    fn tokens_compare_whole() {
        assert!(same_token("secret", "secret"));
        assert!(same_token("", ""));
        assert!(!same_token("secret", "secreT"));
        assert!(!same_token("secret", "secret1"));
        assert!(!same_token("", "x"));
    }
}
