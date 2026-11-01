//! The worker's side of the wire: it connects to its manager, says `Hello`,
//! and then keeps one stream open both ways. What the manager sends becomes
//! events for the worker's main loop; deltas, heartbeats, `Stopped` and log
//! chunks leave through an [`Outbox`].
//!
//! The dead man's switch lives here: when nothing comes from the manager for
//! the heartbeat timeout, or its stream closes without `Shutdown`, every
//! user task is aborted at once, without waiting for the main loop. A
//! `Shutdown` that finds users still running stops them the same way.

use super::delta::{DeltaKind, GaggleDelta};
use super::manager::{AUTHORIZATION, HEARTBEAT_INTERVAL, MAX_MESSAGE};
use super::merge::coalesce;
use super::proto::{
    gaggle_client::GaggleClient, manager_message::Message as FromManager, stopped::Reason,
    worker_message::Message as ToManager, Heartbeat, Hello, LogChunk, Metrics, PrepareAck, Stopped,
    WorkerMessage, PROTOCOL_VERSION,
};
use crate::GooseError;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

/// The most deltas a worker queues; past it the newest is coalesced into the
/// last queued, or dropped when it cannot be.
pub(crate) const MAX_QUEUED_DELTAS: usize = 8;

/// The most other messages queued before a log chunk waits for room.
const MAX_QUEUED_CHUNKS: usize = 4;

/// The longest a manager may ask a worker to wait before it starts.
pub(crate) const MAX_START_DELAY: Duration = Duration::from_secs(60);

/// The instant `StartAt` asks for, `delay_ms` from now: refused past
/// [`MAX_START_DELAY`].
fn start_instant(delay_ms: u64) -> Result<tokio::time::Instant, String> {
    let delay = Duration::from_millis(delay_ms);
    if delay > MAX_START_DELAY {
        return Err(format!(
            "the manager asked this worker to start in {delay_ms} ms, more than the {} seconds a worker waits",
            MAX_START_DELAY.as_secs()
        ));
    }
    tokio::time::Instant::now()
        .checked_add(delay)
        .ok_or_else(|| format!("the manager asked this worker to start in {delay_ms} ms, past what its clock holds"))
}

/// A log chunk's size.
pub(crate) const LOG_CHUNK: usize = 1 << 20;

/// The HTTP/2 stream and connection windows a worker offers its manager.
const WORKER_WINDOW: u32 = 1 << 20;

/// The backoff between connection attempts: 1, 2, 4 and 8 seconds, then
/// every 10 seconds.
fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(match attempt {
        0 => 1,
        1 => 2,
        2 => 4,
        3 => 8,
        _ => 10,
    })
}

/// The delta in `slot`, an empty one left in its place.
fn take(slot: &mut GaggleDelta) -> GaggleDelta {
    std::mem::replace(slot, GaggleDelta::new(0, 0, 0, DeltaKind::Delta))
}

/// What leaves the worker.
enum Outgoing {
    Delta(Box<GaggleDelta>),
    Message(ToManager),
}

#[derive(Default)]
struct OutboxState {
    queue: VecDeque<Outgoing>,
    deltas: usize,
    closed: bool,
    /// The newest delta was dropped: the last queued delta must not take
    /// the next, or the gap the manager sees in the sequence would close.
    hole: bool,
}

impl OutboxState {
    /// The last delta queued.
    fn last_delta(&mut self) -> Option<&mut Box<GaggleDelta>> {
        self.queue.iter_mut().rev().find_map(|item| match item {
            Outgoing::Delta(last) => Some(last),
            Outgoing::Message(_) => None,
        })
    }

    /// Free a place for a delta by joining two neighbouring queued deltas
    /// that follow each other, the newest such pair first: whether it
    /// could. Never two across an earlier drop, which would hide its gap.
    fn join_newest(&mut self) -> bool {
        let deltas: Vec<usize> = self
            .queue
            .iter()
            .enumerate()
            .filter(|(_, item)| matches!(item, Outgoing::Delta(_)))
            .map(|(at, _)| at)
            .collect();
        for pair in deltas.windows(2).rev() {
            let (before, newest) = (pair[0], pair[1]);
            let follows = match (&self.queue[before], &self.queue[newest]) {
                (Outgoing::Delta(a), Outgoing::Delta(b)) => b.first_seq == a.seq.saturating_add(1),
                _ => false,
            };
            if !follows {
                continue;
            }
            let Some(Outgoing::Delta(b)) = self.queue.remove(newest) else {
                continue;
            };
            let Outgoing::Delta(a) = &mut self.queue[before] else {
                continue;
            };
            match coalesce(take(a), &b) {
                Ok(joined) => {
                    **a = joined;
                    self.deltas -= 1;
                    return true;
                }
                Err((unchanged, _)) => {
                    **a = unchanged;
                    self.queue.insert(newest, Outgoing::Delta(b));
                }
            }
        }
        false
    }
}

/// What the worker sends its manager, in order, with at most
/// [`MAX_QUEUED_DELTAS`] deltas queued: past that, the newest is coalesced
/// into the last queued delta instead of growing the queue, or dropped,
/// leaving a gap in the sequence the manager records. The one exception is
/// a final delta that cannot be joined, which is never dropped and goes
/// past the cap. Heartbeats go to the front.
#[derive(Default)]
pub(crate) struct Outbox {
    state: Mutex<OutboxState>,
    /// Something was queued, or the outbox closed.
    queued: Notify,
    /// Something left.
    sent: Notify,
    /// Name another worker in the next delta, so the manager refuses it
    /// (unit tests).
    #[cfg(test)]
    pub(crate) corrupt_next: AtomicBool,
    /// Name another worker in every delta from this sequence number on, 0
    /// for none (unit tests).
    #[cfg(test)]
    pub(crate) corrupt_from: std::sync::atomic::AtomicU64,
    /// Fail to encode every delta (unit tests).
    #[cfg(test)]
    unencodable: AtomicBool,
}

impl Outbox {
    fn push(&self, item: Outgoing, front: bool) {
        if let Ok(mut state) = self.state.lock() {
            if state.closed {
                return;
            }
            if front {
                state.queue.push_front(item);
            } else {
                state.queue.push_back(item);
            }
        }
        self.queued.notify_one();
    }

    /// Queue a delta, coalescing it into the last queued delta when
    /// [`MAX_QUEUED_DELTAS`] are queued already. Coalescing fails only when
    /// a checked sum overflows, which the manager's own checked merge would
    /// refuse too: then the new delta, which carries nothing the manager
    /// could use, is dropped, and never a queued one. The deltas after it
    /// start a new place in the queue, made by joining the two newest
    /// before the drop, so the manager sees the dropped range as a gap.
    pub(crate) fn push_delta(&self, delta: GaggleDelta) {
        #[cfg(test)]
        let delta = {
            let mut delta = delta;
            let from = self.corrupt_from.load(Ordering::Relaxed);
            if self.corrupt_next.swap(false, Ordering::Relaxed) || (from != 0 && delta.seq >= from)
            {
                delta.worker = u32::MAX;
            }
            delta
        };
        if let Ok(mut state) = self.state.lock() {
            if state.closed {
                return;
            }
            if state.deltas >= MAX_QUEUED_DELTAS {
                let coalesced = if state.hole {
                    // After a drop: never across it.
                    if state.join_newest() {
                        None
                    } else {
                        Some(Err("no queued delta could make room".to_string()))
                    }
                } else {
                    // Joined in place: the queued delta is taken out, joined
                    // and put back, never copied.
                    state
                        .last_delta()
                        .map(|last| match coalesce(take(last), &delta) {
                            Ok(joined) => {
                                **last = joined;
                                Ok(())
                            }
                            Err((unchanged, e)) => {
                                **last = unchanged;
                                Err(e.to_string())
                            }
                        })
                };
                match coalesced {
                    Some(Ok(())) => return,
                    // A final delta is never dropped: no delta follows it
                    // to show a gap. It goes past the cap, the one delta
                    // that can.
                    Some(Err(why)) if delta.kind == DeltaKind::Final => {
                        error!(
                            "gaggle: the final metrics delta could not be joined to the {MAX_QUEUED_DELTAS} queued ({why}); queued past them"
                        );
                    }
                    Some(Err(why)) => {
                        error!(
                            "gaggle: dropped metrics deltas {} to {}: {MAX_QUEUED_DELTAS} are queued and it could not be joined to them ({why}); the manager records the gap",
                            delta.first_seq, delta.seq
                        );
                        state.hole = true;
                        return;
                    }
                    // Room was made, or no delta is queued to join.
                    None => {}
                }
            }
            state.hole = false;
            state.deltas += 1;
            state.queue.push_back(Outgoing::Delta(Box::new(delta)));
        }
        self.queued.notify_one();
    }

    /// Queue a message after everything queued.
    pub(crate) fn push_message(&self, message: ToManager) {
        self.push(Outgoing::Message(message), false);
    }

    /// Queue a log chunk, once fewer than [`MAX_QUEUED_CHUNKS`] messages
    /// wait, so a large log is never read into memory whole.
    async fn push_chunk(&self, chunk: LogChunk) {
        loop {
            let sent = self.sent.notified();
            let room = self.state.lock().map_or(true, |state| {
                state.closed || state.queue.len() < MAX_QUEUED_CHUNKS
            });
            if room {
                break;
            }
            sent.await;
        }
        self.push_message(ToManager::LogChunk(chunk));
    }

    /// The next message to send, waiting for one; `None` once closed.
    async fn next(&self) -> Option<WorkerMessage> {
        loop {
            let queued = self.queued.notified();
            let item = {
                let mut state = self.state.lock().ok()?;
                match state.queue.pop_front() {
                    Some(item) => {
                        if matches!(item, Outgoing::Delta(_)) {
                            state.deltas -= 1;
                        }
                        Some(item)
                    }
                    None if state.closed => return None,
                    None => None,
                }
            };
            if let Some(item) = item {
                self.sent.notify_waiters();
                let message = match item {
                    Outgoing::Message(message) => message,
                    Outgoing::Delta(delta) => {
                        let encoded = delta.encode();
                        #[cfg(test)]
                        let encoded = match self.unencodable.load(Ordering::Relaxed) {
                            true => Err(postcard::Error::SerializeBufferFull),
                            false => encoded,
                        };
                        match encoded {
                            Ok(bytes) => ToManager::Metrics(Metrics { delta: bytes }),
                            // No final metrics will reach the manager: say
                            // so, so it records them missing, rather than
                            // wait for them.
                            Err(e) if delta.kind == DeltaKind::Final => {
                                error!(
                                    "gaggle: could not encode the final metrics delta {}: {e}",
                                    delta.seq
                                );
                                ToManager::Stopped(Stopped {
                                    reason: Reason::Failed as i32,
                                    canceled: format!("it could not encode its final metrics: {e}"),
                                    run_folder: String::new(),
                                })
                            }
                            Err(e) => {
                                warn!("gaggle: could not encode delta {}: {e}", delta.seq);
                                continue;
                            }
                        }
                    }
                };
                return Some(WorkerMessage {
                    message: Some(message),
                });
            }
            queued.await;
        }
    }

    /// Send nothing more, once what is queued has left.
    fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
        }
        self.queued.notify_waiters();
        self.queued.notify_one();
        self.sent.notify_waiters();
    }

    /// Whether it is closed: nothing more is queued.
    fn is_closed(&self) -> bool {
        self.state.lock().map_or(true, |state| state.closed)
    }

    /// Whether nothing is queued.
    pub(crate) fn is_empty(&self) -> bool {
        self.state
            .lock()
            .map_or(true, |state| state.queue.is_empty())
    }

    /// The deltas queued (unit tests).
    #[cfg(test)]
    pub(crate) fn queued_deltas(&self) -> usize {
        self.state.lock().map_or(0, |state| state.deltas)
    }
}

/// What the manager tells the worker's main loop.
#[derive(Debug)]
pub(crate) enum LinkEvent {
    /// Its share of the load test, postcard encoded.
    Prepare(Vec<u8>),
    /// Start at this instant.
    StartAt(tokio::time::Instant),
    /// Reset the metrics, starting this epoch.
    ResetMetrics(u64),
    /// Stop every user.
    Stop(String),
    /// Send the logs.
    SendLogs,
    /// Exit.
    Shutdown,
    /// The manager is gone: every user was stopped already.
    Lost(String),
    /// The manager sent something this worker refuses: it exits.
    Refused(String),
}

/// A worker's connection to its manager.
pub(crate) struct Link {
    pub(crate) id: u32,
    pub(crate) outbox: Arc<Outbox>,
    events: flume::Receiver<LinkEvent>,
    /// Every user task the worker started, aborted at once by the dead man's
    /// switch.
    users: Arc<Mutex<Vec<tokio::task::AbortHandle>>>,
    /// Set once the manager said `Shutdown`.
    shutdown: Arc<AtomicBool>,
    /// Set when the worker's `execute()` returns, so the switch's last resort
    /// never ends a process that went on to do something else.
    pub(crate) done: Arc<AtomicBool>,
    /// Set once every user was stopped, by the dead man's switch, `Shutdown`
    /// or [`Link::stop_users`]: a user started after it is stopped at once.
    users_stopped: Arc<AtomicBool>,
    /// Set only when the dead man's switch fires: the heartbeats stop and
    /// it fires no more.
    fired: Arc<AtomicBool>,
    /// Set when the switch's last resort ends the process.
    forced: Arc<AtomicBool>,
}

/// What [`Link::connect`] needs. No `Debug`: it holds the token.
pub(crate) struct LinkSettings {
    /// `host:port`.
    pub(crate) address: String,
    pub(crate) connect_timeout: Duration,
    pub(crate) heartbeat_timeout: Duration,
    pub(crate) hello: Hello,
    /// The token, sent in the request's metadata.
    pub(crate) token: Option<String>,
    /// The switch's last resort ends the process for real, rather than
    /// record that it would have (unit tests).
    #[cfg(test)]
    pub(crate) exit_for_real: bool,
}

impl LinkSettings {
    /// The `Hello` a worker of `plan_hash` named `name` says.
    pub(crate) fn hello(plan_hash: u64, name: String) -> Hello {
        Hello {
            goose_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: PROTOCOL_VERSION,
            plan_hash,
            build_id: None,
            worker_name: name,
        }
    }
}

/// The name a worker gives itself: the `HOSTNAME` environment variable, as a
/// pod's name in Kubernetes, or `worker-` and its process id.
pub(crate) fn worker_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| format!("worker-{}", std::process::id()))
}

/// The request stream: everything the outbox gives, in order.
fn outbound(
    outbox: Arc<Outbox>,
) -> impl tokio_stream::Stream<Item = WorkerMessage> + Send + 'static {
    futures::stream::unfold(outbox, |outbox| async move {
        let message = outbox.next().await?;
        Some((message, outbox))
    })
}

impl Link {
    /// Connect to the manager, retrying with backoff until
    /// [`LinkSettings::connect_timeout`] runs out, and say `Hello`. A refusal
    /// is an error at once, never retried. The deadline covers each attempt
    /// whole, the TCP connect, the HTTP/2 handshake, the stream's response
    /// and the `Welcome`, so a manager that accepts and then says nothing
    /// cannot hold a worker past it.
    pub(crate) async fn connect(settings: LinkSettings) -> Result<Link, GooseError> {
        let deadline = Instant::now() + settings.connect_timeout;
        let uri = format!("http://{}", settings.address);
        let endpoint = tonic::transport::Endpoint::from_shared(uri.clone())
            .map_err(|e| GooseError::Gaggle {
                detail: format!("--manager-host {}: {e}", settings.address),
            })?
            .connect_timeout(Duration::from_secs(5))
            // One connection, to its own manager: a window of 1 MiB lets the
            // manager's messages, custom data among them, move 1 MiB each
            // round trip. No HTTP/2 keepalive: the heartbeats find a dead
            // manager.
            .initial_stream_window_size(WORKER_WINDOW)
            .initial_connection_window_size(WORKER_WINDOW)
            .tcp_nodelay(true);
        let mut attempt = 0;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let tried =
                match tokio::time::timeout(left, Self::try_connect(&endpoint, &settings)).await {
                    Ok(tried) => tried,
                    Err(_) => Err(Refusal::Unreachable(
                        "it accepted the connection but did not answer".to_string(),
                    )),
                };
            let why = match tried {
                Ok(link) => return Ok(link),
                Err(Refusal::Refused(reason)) => {
                    return Err(GooseError::Gaggle {
                        detail: format!(
                            "the manager at {} refused this worker: {reason}",
                            settings.address
                        ),
                    })
                }
                Err(Refusal::Unreachable(why)) => why,
            };
            let wait = backoff(attempt);
            attempt += 1;
            if Instant::now() + wait >= deadline {
                return Err(GooseError::Gaggle {
                    detail: format!(
                        "could not reach the manager at {} within {} seconds: {why}",
                        settings.address,
                        settings.connect_timeout.as_secs()
                    ),
                });
            }
            info!(
                "gaggle: manager at {} not reached ({why}); trying again in {} s",
                settings.address,
                wait.as_secs()
            );
            tokio::time::sleep(wait).await;
        }
    }

    async fn try_connect(
        endpoint: &tonic::transport::Endpoint,
        settings: &LinkSettings,
    ) -> Result<Link, Refusal> {
        let channel = endpoint
            .connect()
            .await
            .map_err(|e| Refusal::Unreachable(error_chain(&e)))?;
        let mut client = GaggleClient::new(channel)
            .max_decoding_message_size(MAX_MESSAGE)
            .max_encoding_message_size(MAX_MESSAGE);
        let outbox = Arc::new(Outbox::default());
        outbox.push_message(ToManager::Hello(settings.hello.clone()));
        let mut request = tonic::Request::new(outbound(outbox.clone()));
        if let Some(token) = settings.token.as_deref() {
            // The options refuse a token that is not printable ASCII.
            let mut value: tonic::metadata::MetadataValue<tonic::metadata::Ascii> =
                format!("Bearer {token}").parse().map_err(|_| {
                    Refusal::Refused("the --gaggle-token is not printable ASCII".to_string())
                })?;
            // Kept out of any debug output of the request.
            value.set_sensitive(true);
            request.metadata_mut().insert(AUTHORIZATION, value);
        }
        let response = client.connect(request).await.map_err(|status| {
            // The manager checks the token before anything else.
            if status.code() == tonic::Code::Unauthenticated {
                Refusal::Refused(status.message().to_string())
            } else {
                Refusal::Unreachable(status.message().to_string())
            }
        })?;
        let mut inbound = response.into_inner();
        let first = tokio::time::timeout(settings.heartbeat_timeout, inbound.message())
            .await
            .map_err(|_| Refusal::Unreachable("no Welcome from the manager".to_string()))?;
        let id = match first {
            Ok(Some(message)) => match message.message {
                Some(FromManager::Welcome(welcome)) => welcome.worker_id,
                Some(FromManager::Reject(reject)) => return Err(Refusal::Refused(reject.reason)),
                other => {
                    return Err(Refusal::Unreachable(format!(
                        "the manager answered {other:?}, not Welcome"
                    )))
                }
            },
            Ok(None) => {
                return Err(Refusal::Unreachable(
                    "the manager closed the stream".to_string(),
                ))
            }
            Err(status) => return Err(Refusal::Unreachable(status.message().to_string())),
        };
        info!(
            "gaggle: joined the manager at {} as worker {id}",
            settings.address
        );
        let (events_tx, events) = flume::unbounded();
        let link = Link {
            id,
            outbox,
            events,
            users: Arc::new(Mutex::new(Vec::new())),
            shutdown: Arc::new(AtomicBool::new(false)),
            done: Arc::new(AtomicBool::new(false)),
            users_stopped: Arc::new(AtomicBool::new(false)),
            fired: Arc::new(AtomicBool::new(false)),
            forced: Arc::new(AtomicBool::new(false)),
        };
        let heard = Arc::new(Mutex::new(Instant::now()));
        let switch = Switch {
            fired: link.fired.clone(),
            users_stopped: link.users_stopped.clone(),
            users: link.users.clone(),
            events: events_tx.clone(),
            outbox: link.outbox.clone(),
            heartbeat_timeout: settings.heartbeat_timeout,
            done: link.done.clone(),
            forced: link.forced.clone(),
            last_resort_after: LAST_RESORT_AFTER,
            // A unit test's worker never ends the test binary, unless told.
            #[cfg(test)]
            record_exit: !settings.exit_for_real,
        };
        tokio::spawn(read_manager(
            inbound,
            events_tx,
            heard.clone(),
            link.shutdown.clone(),
            switch.clone(),
        ));
        tokio::spawn(heartbeat_manager(
            link.outbox.clone(),
            heard,
            link.shutdown.clone(),
            switch,
        ));
        Ok(link)
    }

    /// Watch a user task, so the dead man's switch can stop it.
    pub(crate) fn watch(&self, user: tokio::task::AbortHandle) {
        if let Ok(mut users) = self.users.lock() {
            // Every user is stopped by setting `users_stopped` before taking
            // this lock, so either that sees this user or this sees the flag.
            if self.users_stopped.load(Ordering::SeqCst) {
                user.abort();
                return;
            }
            // Kept to the end of the run, as the run state keeps every
            // user's JoinHandle: one per user started.
            users.push(user);
        }
    }

    /// Stop every user at once, and any started later, as the switch does.
    /// The switch stays armed, and the heartbeats go on.
    pub(crate) fn stop_users(&self) {
        abort_users(&self.users, &self.users_stopped, false);
    }

    /// The next event if one is waiting.
    pub(crate) fn try_event(&self) -> Option<LinkEvent> {
        self.events.try_recv().ok()
    }

    /// The next event, waiting at most `wait`.
    pub(crate) async fn next_event(&self, wait: Duration) -> Option<LinkEvent> {
        tokio::time::timeout(wait, self.events.recv_async())
            .await
            .ok()?
            .ok()
    }

    /// Answer `Prepare`.
    pub(crate) fn prepared(&self, error: Option<String>) {
        self.outbox
            .push_message(ToManager::PrepareAck(PrepareAck { error }));
    }

    /// Tell the manager the users stopped, after the final delta.
    pub(crate) fn stopped(&self, stopped: Stopped) {
        self.outbox.push_message(ToManager::Stopped(stopped));
    }

    /// Stream each of `logs`, `(name, path)`, in chunks of [`LOG_CHUNK`],
    /// in the background.
    pub(crate) fn send_logs(&self, logs: Vec<(String, std::path::PathBuf)>) {
        let outbox = self.outbox.clone();
        tokio::spawn(async move {
            for (file, path) in logs {
                if let Err(e) = send_log(&outbox, &file, &path).await {
                    warn!("gaggle: could not send log {}: {e}", path.display());
                    // End the file, so the manager does not wait for it.
                    outbox
                        .push_chunk(LogChunk {
                            file,
                            offset: u64::MAX,
                            data: Vec::new(),
                            last: true,
                        })
                        .await;
                }
            }
        });
    }

    /// Close the stream, once what is queued has left.
    pub(crate) fn close(&self) {
        self.outbox.close();
    }

    /// Close the stream, and give what is queued up to 2 seconds to leave.
    pub(crate) async fn flush(&self) {
        self.close();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && !self.outbox.is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

impl Drop for Link {
    /// The worker is done with its manager: its `execute()` is returning, so
    /// the switch's last resort must not end the process.
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
        self.outbox.close();
    }
}

/// Stream one log file.
async fn send_log(outbox: &Outbox, file: &str, path: &std::path::Path) -> std::io::Result<()> {
    use tokio::io::AsyncReadExt;
    let mut reader = match tokio::fs::File::open(path).await {
        Ok(reader) => reader,
        // A log never written: the manager gets an empty file.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            outbox
                .push_chunk(LogChunk {
                    file: file.to_string(),
                    offset: 0,
                    data: Vec::new(),
                    last: true,
                })
                .await;
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    let mut offset = 0u64;
    let mut data = vec![0u8; LOG_CHUNK];
    let mut filled = 0;
    loop {
        // Nothing more leaves once the outbox is closed: stop reading.
        if outbox.is_closed() {
            return Ok(());
        }
        let read = reader.read(&mut data[filled..]).await?;
        filled += read;
        let end = read == 0;
        if filled == LOG_CHUNK || end {
            // The buffer moves into the chunk, and a fresh one takes its
            // place: no chunk is copied.
            let mut chunk = std::mem::replace(&mut data, vec![0u8; LOG_CHUNK]);
            chunk.truncate(filled);
            outbox
                .push_chunk(LogChunk {
                    file: file.to_string(),
                    offset,
                    data: chunk,
                    last: end,
                })
                .await;
            offset += filled as u64;
            filled = 0;
        }
        if end {
            return Ok(());
        }
    }
}

/// A failed attempt to join.
enum Refusal {
    /// The manager answered `Reject`.
    Refused(String),
    /// It could not be reached, or did not answer as a manager does.
    Unreachable(String),
}

/// An error and every source of it, as one line.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// How long after the switch fires, or `Shutdown` finds users running, a
/// worker whose `execute()` has not returned ends its process.
const LAST_RESORT_AFTER: Duration = Duration::from_secs(2);

/// Stop users at once, every one or, with `running_only`, those not yet
/// finished, and any started later: `stopped` is set before `users` is
/// locked, so [`Link::watch`] sees one or the other. How many it stopped.
fn abort_users(
    users: &Mutex<Vec<tokio::task::AbortHandle>>,
    stopped: &AtomicBool,
    running_only: bool,
) -> usize {
    stopped.store(true, Ordering::SeqCst);
    let Ok(users) = users.lock() else {
        return 0;
    };
    let mut stopped = 0;
    for user in users
        .iter()
        .filter(|user| !running_only || !user.is_finished())
    {
        user.abort();
        stopped += 1;
    }
    stopped
}

/// The dead man's switch.
#[derive(Clone)]
struct Switch {
    /// Set when it fires, and only then.
    fired: Arc<AtomicBool>,
    users_stopped: Arc<AtomicBool>,
    users: Arc<Mutex<Vec<tokio::task::AbortHandle>>>,
    events: flume::Sender<LinkEvent>,
    outbox: Arc<Outbox>,
    heartbeat_timeout: Duration,
    done: Arc<AtomicBool>,
    /// Set when the last resort ends the process.
    forced: Arc<AtomicBool>,
    /// How long the last resort waits for `execute()` to return.
    last_resort_after: Duration,
    /// The last resort records that it would have ended the process, in
    /// `forced`, instead of ending it (unit tests).
    #[cfg(test)]
    record_exit: bool,
}

impl Switch {
    /// Stop every user at once, tell the main loop why, and close the
    /// stream. A worker whose `execute()` has not returned 2 seconds later
    /// exits the process with an error status: load never outlives its
    /// manager.
    fn fire(&self, why: String) {
        if self.fired.swap(true, Ordering::SeqCst) {
            return;
        }
        error!("gaggle: lost the manager ({why}); stopping every user");
        abort_users(&self.users, &self.users_stopped, false);
        self.outbox.close();
        let _ = self.events.send(LinkEvent::Lost(why));
        self.last_resort("gaggle: lost the manager and the worker has not exited; exiting now");
    }

    /// The manager said `Shutdown`. Any user still running, as when the
    /// manager gave this worker up for lost, is stopped at once, and a
    /// worker whose `execute()` has not returned 2 seconds later exits with
    /// an error status, as when the switch fires: the main loop may be held.
    fn shut_down(&self) {
        let running = abort_users(&self.users, &self.users_stopped, true);
        if running > 0 {
            warn!("gaggle: the manager said shut down with {running} users running; stopping them");
            self.last_resort(
                "gaggle: the manager said shut down and the worker has not exited; exiting now",
            );
        }
    }

    /// End the process with an error status [`LAST_RESORT_AFTER`] from now,
    /// unless the worker's `execute()` has returned by then.
    fn last_resort(&self, message: &'static str) {
        let done = self.done.clone();
        let forced = self.forced.clone();
        let after = self.last_resort_after;
        #[cfg(test)]
        let record_exit = self.record_exit;
        std::thread::spawn(move || {
            std::thread::sleep(after);
            if !done.load(Ordering::Relaxed) {
                eprintln!("{message}");
                forced.store(true, Ordering::Relaxed);
                #[cfg(test)]
                if record_exit {
                    return;
                }
                std::process::exit(1);
            }
        });
    }
}

/// Read the manager's stream until it ends, as events.
async fn read_manager(
    mut inbound: tonic::Streaming<super::proto::ManagerMessage>,
    events: flume::Sender<LinkEvent>,
    heard: Arc<Mutex<Instant>>,
    shutdown: Arc<AtomicBool>,
    switch: Switch,
) {
    loop {
        let message = match inbound.message().await {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(status) => {
                debug!("gaggle: manager stream error: {status}");
                break;
            }
        };
        if let Ok(mut heard) = heard.lock() {
            *heard = Instant::now();
        }
        let event = match message.message {
            Some(FromManager::Heartbeat(_)) | None => continue,
            Some(FromManager::Welcome(_)) | Some(FromManager::Reject(_)) => continue,
            Some(FromManager::Prepare(prepare)) => LinkEvent::Prepare(prepare.spec),
            Some(FromManager::StartAt(start)) => match start_instant(start.delay_ms) {
                Ok(at) => LinkEvent::StartAt(at),
                Err(why) => LinkEvent::Refused(why),
            },
            Some(FromManager::ResetMetrics(reset)) => LinkEvent::ResetMetrics(reset.epoch),
            Some(FromManager::Stop(stop)) => LinkEvent::Stop(stop.reason),
            Some(FromManager::SendLogs(_)) => LinkEvent::SendLogs,
            Some(FromManager::Shutdown(_)) => {
                shutdown.store(true, Ordering::Relaxed);
                switch.shut_down();
                let _ = events.send(LinkEvent::Shutdown);
                return;
            }
        };
        if events.send(event).is_err() {
            return;
        }
    }
    if !shutdown.load(Ordering::Relaxed) {
        switch.fire("its stream closed without Shutdown".to_string());
    }
}

/// Send the manager a heartbeat every [`HEARTBEAT_INTERVAL`], on its own
/// task, whatever else the worker is doing; fire the switch when nothing has
/// come from the manager for the heartbeat timeout.
async fn heartbeat_manager(
    outbox: Arc<Outbox>,
    heard: Arc<Mutex<Instant>>,
    shutdown: Arc<AtomicBool>,
    switch: Switch,
) {
    let tick = HEARTBEAT_INTERVAL
        .min(switch.heartbeat_timeout / 4)
        .max(Duration::from_millis(100));
    let mut interval = tokio::time::interval(tick);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_sent = Instant::now();
    loop {
        interval.tick().await;
        if shutdown.load(Ordering::Relaxed) || switch.fired.load(Ordering::Relaxed) {
            return;
        }
        let silent = heard
            .lock()
            .map(|heard| heard.elapsed())
            .unwrap_or_default();
        if silent > switch.heartbeat_timeout {
            switch.fire(format!(
                "nothing from it for {} seconds",
                switch.heartbeat_timeout.as_secs()
            ));
            return;
        }
        if last_sent.elapsed() >= HEARTBEAT_INTERVAL {
            last_sent = Instant::now();
            outbox.push(Outgoing::Message(ToManager::Heartbeat(Heartbeat {})), true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gaggle::delta::DeltaKind;

    fn delta(seq: u64) -> GaggleDelta {
        let mut delta = GaggleDelta::new(0, seq, 1, DeltaKind::Delta);
        delta.now = seq;
        delta
            .graph
            .requests
            .insert("GET /".to_string(), [(seq, 1)].into());
        delta
    }

    /// The outbox's next message, within 30 seconds.
    async fn next(outbox: &Outbox) -> Option<WorkerMessage> {
        tokio::time::timeout(Duration::from_secs(30), outbox.next())
            .await
            .expect("within 30 seconds")
    }

    /// Past eight queued deltas the newest is coalesced into the last queued,
    /// which then covers both: nothing is lost and the queue does not grow.
    #[tokio::test]
    async fn outbox_coalesces_past_eight_deltas() {
        let outbox = Outbox::default();
        for seq in 1..=12 {
            outbox.push_delta(delta(seq));
        }
        assert_eq!(outbox.queued_deltas(), MAX_QUEUED_DELTAS);
        outbox.push_message(ToManager::Heartbeat(Heartbeat {}));
        let mut sent = Vec::new();
        outbox.close();
        while let Some(message) = next(&outbox).await {
            sent.push(message);
        }
        assert_eq!(sent.len(), MAX_QUEUED_DELTAS + 1);
        let last = match &sent[MAX_QUEUED_DELTAS - 1].message {
            Some(ToManager::Metrics(metrics)) => GaggleDelta::decode(&metrics.delta).unwrap(),
            other => panic!("{:?}", other),
        };
        assert_eq!((last.first_seq, last.seq), (8, 12));
        assert_eq!(last.graph.requests["GET /"].len(), 5);
    }

    /// The queued deltas, encoded (unit tests).
    fn queued(outbox: &Outbox) -> Vec<Vec<u8>> {
        let state = outbox.state.lock().unwrap();
        state
            .queue
            .iter()
            .filter_map(|item| match item {
                Outgoing::Delta(delta) => Some(delta.encode().unwrap()),
                Outgoing::Message(_) => None,
            })
            .collect()
    }

    /// A stalled link, and a delta that cannot be coalesced into the last
    /// queued, its sum overflowing: the queue stays at its cap, the queued
    /// deltas as they were, and the dropped delta shows as a gap the
    /// manager records. Nor does a later delta close the gap.
    #[tokio::test]
    async fn outbox_drops_a_delta_it_cannot_coalesce() {
        use crate::gaggle::merge::SeqGap;
        let outbox = Outbox::default();
        for seq in 1..=MAX_QUEUED_DELTAS as u64 {
            outbox.push_delta(delta(seq));
        }
        let before = queued(&outbox);
        let mut overflowing = delta(MAX_QUEUED_DELTAS as u64 + 1);
        overflowing.graph.requests.insert(
            "GET /".to_string(),
            [(MAX_QUEUED_DELTAS as u64, u64::MAX)].into(),
        );
        outbox.push_delta(overflowing);
        assert_eq!(outbox.queued_deltas(), MAX_QUEUED_DELTAS);
        assert_eq!(queued(&outbox), before, "a queued delta changed");
        // The next delta takes a place of its own, made by joining the two
        // newest before the gap.
        outbox.push_delta(delta(MAX_QUEUED_DELTAS as u64 + 2));
        assert_eq!(outbox.queued_deltas(), MAX_QUEUED_DELTAS);
        outbox.push_delta(delta(MAX_QUEUED_DELTAS as u64 + 3));
        assert_eq!(outbox.queued_deltas(), MAX_QUEUED_DELTAS);
        // A second drop, and the deltas after it: no join crosses either.
        let second = MAX_QUEUED_DELTAS as u64 + 4;
        let mut overflowing = delta(second);
        overflowing
            .graph
            .requests
            .insert("GET /".to_string(), [(second - 1, u64::MAX)].into());
        outbox.push_delta(overflowing);
        outbox.push_delta(delta(second + 1));
        outbox.push_delta(delta(second + 2));
        assert_eq!(outbox.queued_deltas(), MAX_QUEUED_DELTAS);
        outbox.close();
        let mut accumulator = crate::gaggle::tests::accumulator_for_test();
        while let Some(message) = next(&outbox).await {
            if let Some(ToManager::Metrics(metrics)) = message.message {
                accumulator
                    .apply(0, GaggleDelta::decode(&metrics.delta).unwrap())
                    .unwrap();
            }
        }
        let dropped = MAX_QUEUED_DELTAS as u64 + 1;
        assert_eq!(
            accumulator.gaps(),
            &[
                SeqGap {
                    worker: 0,
                    expected: dropped,
                    received: dropped + 1,
                },
                SeqGap {
                    worker: 0,
                    expected: second,
                    received: second + 1,
                }
            ]
        );
        assert_eq!(accumulator.missing(0), 2);
    }

    /// A final delta that cannot be joined to the last queued is never
    /// dropped: no delta follows it to show a gap. It goes past the cap.
    #[tokio::test]
    async fn outbox_never_drops_a_final_delta() {
        let outbox = Outbox::default();
        for seq in 1..=MAX_QUEUED_DELTAS as u64 {
            outbox.push_delta(delta(seq));
        }
        let last = MAX_QUEUED_DELTAS as u64 + 1;
        let mut last_final = delta(last);
        last_final.kind = DeltaKind::Final;
        last_final
            .graph
            .requests
            .insert("GET /".to_string(), [(last - 1, u64::MAX)].into());
        outbox.push_delta(last_final);
        assert_eq!(outbox.queued_deltas(), MAX_QUEUED_DELTAS + 1);
        outbox.close();
        let mut kinds = Vec::new();
        while let Some(message) = next(&outbox).await {
            if let Some(ToManager::Metrics(metrics)) = message.message {
                kinds.push(GaggleDelta::decode(&metrics.delta).unwrap().kind);
            }
        }
        assert_eq!(kinds.last(), Some(&DeltaKind::Final));
    }

    /// A final delta that cannot be encoded is never dropped unsaid: the
    /// manager is told the worker failed, so it records the final metrics
    /// missing. Any other delta that cannot be encoded is dropped, leaving
    /// a gap.
    #[tokio::test]
    async fn an_unencodable_final_delta_says_the_worker_failed() {
        let outbox = Outbox::default();
        outbox.unencodable.store(true, Ordering::Relaxed);
        outbox.push_delta(delta(1));
        let mut last_final = delta(2);
        last_final.kind = DeltaKind::Final;
        outbox.push_delta(last_final);
        outbox.close();
        let sent = next(&outbox).await.expect("a message for the final delta");
        match sent.message {
            Some(ToManager::Stopped(stopped)) => {
                assert_eq!(stopped.reason(), Reason::Failed);
                assert!(
                    stopped.canceled.contains("final metrics"),
                    "{}",
                    stopped.canceled
                );
            }
            other => panic!("{:?}", other),
        }
        assert!(next(&outbox).await.is_none());
    }

    /// A connection on which no worker is admitted is closed at its
    /// deadline even when the manager's writes are stalled, its peer never
    /// reading: the deadline is checked on writes too.
    #[tokio::test]
    async fn an_unadmitted_connection_closes_with_its_writes_stalled() {
        use super::super::manager::stalled_write_closes_for_test;
        tokio::time::timeout(Duration::from_secs(30), stalled_write_closes_for_test())
            .await
            .expect("the stalled connection closes within 30 seconds");
    }

    /// A heartbeat goes ahead of what is queued.
    #[tokio::test]
    async fn heartbeats_go_first() {
        let outbox = Outbox::default();
        outbox.push_delta(delta(1));
        outbox.push(Outgoing::Message(ToManager::Heartbeat(Heartbeat {})), true);
        let first = next(&outbox).await.unwrap();
        assert!(matches!(first.message, Some(ToManager::Heartbeat(_))));
    }

    /// A switch over `users`, its events and outbox; `done` set, and its
    /// last resort recording, so it never ends the test binary.
    fn switch(
        users: Vec<tokio::task::AbortHandle>,
        heartbeat_timeout: Duration,
    ) -> (Switch, flume::Receiver<LinkEvent>, Arc<Outbox>) {
        let (events, received) = flume::unbounded();
        let outbox = Arc::new(Outbox::default());
        let switch = Switch {
            fired: Arc::new(AtomicBool::new(false)),
            users_stopped: Arc::new(AtomicBool::new(false)),
            users: Arc::new(Mutex::new(users)),
            events,
            outbox: outbox.clone(),
            heartbeat_timeout,
            done: Arc::new(AtomicBool::new(true)),
            forced: Arc::new(AtomicBool::new(false)),
            last_resort_after: LAST_RESORT_AFTER,
            record_exit: true,
        };
        (switch, received, outbox)
    }

    /// A link over `switch`'s users, events and flags.
    fn link_of(switch: &Switch, outbox: Arc<Outbox>, events: flume::Receiver<LinkEvent>) -> Link {
        Link {
            id: 0,
            outbox,
            events,
            users: switch.users.clone(),
            shutdown: Arc::new(AtomicBool::new(false)),
            done: switch.done.clone(),
            users_stopped: switch.users_stopped.clone(),
            fired: switch.fired.clone(),
            forced: switch.forced.clone(),
        }
    }

    /// Stopping the users, as a worker that fails a phase does, leaves the
    /// switch armed: heartbeats still go, and a manager that then goes
    /// silent fires it.
    #[tokio::test]
    async fn stopping_users_leaves_the_switch_armed() {
        let user = tokio::spawn(std::future::pending::<()>());
        let (switch, events, outbox) = switch(
            vec![user.abort_handle()],
            HEARTBEAT_INTERVAL + Duration::from_secs(1),
        );
        let link = link_of(&switch, outbox.clone(), events);
        let heard = Arc::new(Mutex::new(Instant::now()));
        let task = tokio::spawn(heartbeat_manager(
            outbox.clone(),
            heard,
            link.shutdown.clone(),
            switch.clone(),
        ));
        link.stop_users();
        let ended = tokio::time::timeout(Duration::from_secs(30), user)
            .await
            .expect("within 30 seconds");
        assert!(ended.unwrap_err().is_cancelled());
        assert!(!switch.fired.load(Ordering::SeqCst));
        // A heartbeat still leaves, after the users stopped.
        let first = next(&outbox).await.expect("a heartbeat");
        assert!(matches!(first.message, Some(ToManager::Heartbeat(_))));
        // Nothing heard from the manager: the switch fires.
        tokio::time::timeout(Duration::from_secs(30), task)
            .await
            .expect("the heartbeat task ends")
            .unwrap();
        assert!(switch.fired.load(Ordering::SeqCst));
        assert!(
            matches!(link.try_event(), Some(LinkEvent::Lost(why)) if why.starts_with("nothing from it"))
        );
    }

    /// The dead man's switch aborts every user at once, tells the main loop,
    /// and closes the stream.
    #[tokio::test]
    async fn the_switch_aborts_every_user() {
        let user = tokio::spawn(std::future::pending::<()>());
        let (switch, events, outbox) = switch(vec![user.abort_handle()], Duration::from_secs(10));
        switch.fire("test".to_string());
        let ended = tokio::time::timeout(Duration::from_secs(30), user)
            .await
            .expect("within 30 seconds");
        assert!(ended.unwrap_err().is_cancelled());
        assert!(matches!(events.try_recv(), Ok(LinkEvent::Lost(why)) if why == "test"));
        assert!(next(&outbox).await.is_none());
    }

    /// With nothing heard from the manager for the heartbeat timeout, the
    /// heartbeat task fires the switch, which stops the users.
    #[tokio::test]
    async fn silence_fires_the_switch() {
        let user = tokio::spawn(std::future::pending::<()>());
        let (switch, events, outbox) =
            switch(vec![user.abort_handle()], Duration::from_millis(300));
        let heard = Arc::new(Mutex::new(Instant::now()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(heartbeat_manager(outbox, heard, shutdown, switch));
        let ended = tokio::time::timeout(Duration::from_secs(30), user)
            .await
            .expect("within 30 seconds");
        assert!(ended.unwrap_err().is_cancelled());
        assert!(
            matches!(events.try_recv(), Ok(LinkEvent::Lost(why)) if why.starts_with("nothing from it"))
        );
        tokio::time::timeout(Duration::from_secs(30), task)
            .await
            .expect("the heartbeat task ends")
            .unwrap();
    }

    /// A user the worker starts after the switch fired, in the same pass of
    /// its main loop, is stopped at once.
    #[tokio::test]
    async fn a_user_started_after_the_switch_is_stopped() {
        let (switch, events, outbox) = switch(Vec::new(), Duration::from_secs(10));
        let link = link_of(&switch, outbox, events);
        switch.fire("test".to_string());
        let late = tokio::spawn(std::future::pending::<()>());
        link.watch(late.abort_handle());
        let ended = tokio::time::timeout(Duration::from_secs(30), late)
            .await
            .expect("within 30 seconds");
        assert!(ended.unwrap_err().is_cancelled());
    }

    /// `Shutdown` that comes while users still run, the worker's main loop
    /// held: every user is stopped at once, and the last resort ends the
    /// worker 2 seconds later, as when the switch fires.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_stops_users_while_the_main_loop_is_held() {
        use super::super::manager::Manager;
        let test = async {
            let manager = Manager::start("127.0.0.1", 0, 1, None, 7, true, Duration::from_secs(10))
                .await
                .unwrap();
            let link = Link::connect(LinkSettings {
                address: manager.local_addr.to_string(),
                connect_timeout: Duration::from_secs(10),
                heartbeat_timeout: Duration::from_secs(10),
                hello: LinkSettings::hello(7, "held".to_string()),
                token: None,
                exit_for_real: false,
            })
            .await
            .unwrap();
            let user = tokio::spawn(std::future::pending::<()>());
            link.watch(user.abort_handle());
            // The worker's main loop never reads its events.
            let sent = Instant::now();
            manager.shut_down(link.id);
            let ended = tokio::time::timeout(Duration::from_secs(5), user)
                .await
                .expect("the user stops");
            assert!(ended.unwrap_err().is_cancelled());
            while !link.forced.load(Ordering::Relaxed) {
                assert!(
                    sent.elapsed() < Duration::from_secs(5),
                    "the last resort never fired"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            assert!(sent.elapsed() >= Duration::from_secs(2));
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// `Shutdown` after every user finished, as at the end of a run, stops
    /// nothing and arms no last resort.
    #[tokio::test]
    async fn shutdown_after_the_run_arms_nothing() {
        let finished = tokio::spawn(async {});
        let handle = finished.abort_handle();
        finished.await.unwrap();
        let (switch, _events, _outbox) = switch(vec![handle], Duration::from_secs(10));
        switch.done.store(false, Ordering::Relaxed);
        switch.shut_down();
        tokio::time::sleep(Duration::from_secs(4)).await;
        let forced = switch.forced.load(Ordering::Relaxed);
        switch.done.store(true, Ordering::Relaxed);
        assert!(!forced);
    }

    /// Relay every connection to `to`, each byte held `delay` each way:
    /// the address it listens on.
    async fn delaying_proxy(to: std::net::SocketAddr, delay: Duration) -> std::net::SocketAddr {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        async fn relay(
            mut from: tokio::net::tcp::OwnedReadHalf,
            mut to: tokio::net::tcp::OwnedWriteHalf,
            delay: Duration,
        ) {
            let (tx, mut rx) =
                tokio::sync::mpsc::unbounded_channel::<(tokio::time::Instant, Vec<u8>)>();
            let writer = tokio::spawn(async move {
                while let Some((due, bytes)) = rx.recv().await {
                    tokio::time::sleep_until(due).await;
                    if to.write_all(&bytes).await.is_err() {
                        return;
                    }
                }
            });
            let mut buffer = vec![0u8; 64 << 10];
            while let Ok(read) = from.read(&mut buffer).await {
                if read == 0
                    || tx
                        .send((tokio::time::Instant::now() + delay, buffer[..read].to_vec()))
                        .is_err()
                {
                    break;
                }
            }
            drop(tx);
            let _ = writer.await;
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let Ok(server) = tokio::net::TcpStream::connect(to).await else {
                    continue;
                };
                let (client_read, client_write) = client.into_split();
                let (server_read, server_write) = server.into_split();
                tokio::spawn(relay(client_read, server_write, delay));
                tokio::spawn(relay(server_read, client_write, delay));
            }
        });
        address
    }

    /// Across a link of 100 ms round trips, a manager of 2 workers takes a
    /// worker's 8 MiB log in under 4 seconds: its 1 MiB window moves about
    /// 10 MB/s, where a 64 KiB one moved 640 KB/s, 13 seconds for the log.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_log_crosses_a_slow_link_at_the_window_rate() {
        use super::super::manager::{Event, Manager};
        let test = async {
            let manager = Manager::start("127.0.0.1", 0, 2, None, 7, true, Duration::from_secs(30))
                .await
                .unwrap();
            let proxy = delaying_proxy(manager.local_addr, Duration::from_millis(50)).await;
            let link = Link::connect(LinkSettings {
                address: proxy.to_string(),
                connect_timeout: Duration::from_secs(10),
                heartbeat_timeout: Duration::from_secs(30),
                hello: LinkSettings::hello(7, "far".to_string()),
                token: None,
                exit_for_real: false,
            })
            .await
            .unwrap();
            let path =
                std::env::temp_dir().join(format!("goose-gaggle-far-log-{}", std::process::id()));
            const SIZE: usize = 8 << 20;
            std::fs::write(&path, vec![b'x'; SIZE]).unwrap();
            let sent = Instant::now();
            link.send_logs(vec![("requests.json".to_string(), path.clone())]);
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut received = 0;
            loop {
                match manager.next_event(deadline).await {
                    Some(Event::Log { chunk, .. }) => {
                        assert_eq!(chunk.offset, received as u64);
                        received += chunk.data.len();
                        if chunk.last {
                            break;
                        }
                    }
                    Some(Event::Joined) => {}
                    other => panic!("{:?}", other),
                }
            }
            let took = sent.elapsed();
            let _ = std::fs::remove_file(&path);
            assert_eq!(received, SIZE);
            println!("8 MiB across 100 ms round trips in {took:?}");
            assert!(took < Duration::from_secs(4), "8 MiB took {took:?}");
        };
        tokio::time::timeout(Duration::from_secs(60), test)
            .await
            .expect("within 60 seconds");
    }

    /// A start delay past a minute is refused, never a panic, up to the
    /// largest a message can carry.
    #[tokio::test]
    async fn start_at_past_a_minute_is_refused() {
        assert!(start_instant(2_000).is_ok());
        assert!(start_instant(60_000).is_ok());
        for delay_ms in [60_001, u64::MAX / 2, u64::MAX] {
            let why = start_instant(delay_ms).unwrap_err();
            assert!(why.contains(&delay_ms.to_string()), "{}", why);
        }
    }

    /// A listener that accepts the connection and never speaks HTTP/2: the
    /// worker gives up within the connect timeout, plus 2 seconds.
    #[tokio::test]
    async fn connect_timeout_covers_the_handshake() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let held = tokio::spawn(async move {
            let mut connections = Vec::new();
            while let Ok((connection, _)) = listener.accept().await {
                connections.push(connection);
            }
        });
        let timeout = Duration::from_secs(3);
        let started = Instant::now();
        let joined = tokio::time::timeout(
            Duration::from_secs(30),
            Link::connect(LinkSettings {
                address: address.to_string(),
                connect_timeout: timeout,
                heartbeat_timeout: Duration::from_secs(10),
                hello: LinkSettings::hello(7, "stalled".to_string()),
                token: None,
                exit_for_real: false,
            }),
        )
        .await
        .expect("the worker gives up within 30 seconds");
        let elapsed = started.elapsed();
        match joined {
            Err(GooseError::Gaggle { detail }) => {
                assert!(detail.contains("could not reach"), "{}", detail)
            }
            Err(other) => panic!("{}", other),
            Ok(_) => panic!("joined a listener that never answers"),
        }
        assert!(elapsed <= timeout + Duration::from_secs(2), "{:?}", elapsed);
        held.abort();
    }

    #[test]
    fn backoff_is_one_two_four_eight_then_ten() {
        crate::gaggle::within(30, || {
            let waits: Vec<u64> = (0..7).map(|attempt| backoff(attempt).as_secs()).collect();
            assert_eq!(waits, [1, 2, 4, 8, 10, 10, 10]);
        });
    }
}
