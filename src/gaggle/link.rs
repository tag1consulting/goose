//! The worker's side of the wire: it connects to its manager, says `Hello`,
//! and then keeps one stream open both ways. What the manager sends becomes
//! events for the worker's main loop; deltas, heartbeats, `Stopped` and log
//! chunks leave through an [`Outbox`].
//!
//! The dead man's switch lives here: when nothing comes from the manager for
//! the heartbeat timeout, or its stream closes without `Shutdown`, every
//! user task is aborted at once, without waiting for the main loop.

use super::delta::GaggleDelta;
use super::manager::{HEARTBEAT_INTERVAL, MAX_MESSAGE};
use super::merge::coalesce;
use super::proto::{
    gaggle_client::GaggleClient, manager_message::Message as FromManager,
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
/// last queued.
pub(crate) const MAX_QUEUED_DELTAS: usize = 8;

/// The most other messages queued before a log chunk waits for room.
const MAX_QUEUED_CHUNKS: usize = 4;

/// A log chunk's size.
pub(crate) const LOG_CHUNK: usize = 1 << 20;

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
}

/// What the worker sends its manager, in order, with at most
/// [`MAX_QUEUED_DELTAS`] deltas queued: past that, the newest is coalesced
/// into the last queued delta instead of growing the queue. Heartbeats go
/// to the front.
#[derive(Default)]
pub(crate) struct Outbox {
    state: Mutex<OutboxState>,
    /// Something was queued, or the outbox closed.
    queued: Notify,
    /// Something left.
    sent: Notify,
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
    /// [`MAX_QUEUED_DELTAS`] are queued already.
    pub(crate) fn push_delta(&self, delta: GaggleDelta) {
        if let Ok(mut state) = self.state.lock() {
            if state.closed {
                return;
            }
            if state.deltas >= MAX_QUEUED_DELTAS {
                let last = state.queue.iter_mut().rev().find_map(|item| match item {
                    Outgoing::Delta(last) => Some(last),
                    Outgoing::Message(_) => None,
                });
                if let Some(last) = last {
                    match coalesce(last, &delta) {
                        Ok(joined) => {
                            **last = joined;
                            return;
                        }
                        Err(e) => warn!("gaggle: could not coalesce a delta ({e}); queued it"),
                    }
                }
            }
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
                    Outgoing::Delta(delta) => match delta.encode() {
                        Ok(bytes) => ToManager::Metrics(Metrics { delta: bytes }),
                        Err(e) => {
                            warn!("gaggle: could not encode delta {}: {e}", delta.seq);
                            continue;
                        }
                    },
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
}

/// What [`Link::connect`] needs.
pub(crate) struct LinkSettings {
    /// `host:port`.
    pub(crate) address: String,
    pub(crate) connect_timeout: Duration,
    pub(crate) heartbeat_timeout: Duration,
    pub(crate) hello: Hello,
}

impl LinkSettings {
    /// The `Hello` a worker of `plan_hash` named `name` says.
    pub(crate) fn hello(plan_hash: u64, token: Option<&str>, name: String) -> Hello {
        Hello {
            goose_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: PROTOCOL_VERSION,
            plan_hash,
            build_id: None,
            token: token.unwrap_or_default().to_string(),
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
    /// is an error at once, never retried.
    pub(crate) async fn connect(settings: LinkSettings) -> Result<Link, GooseError> {
        let deadline = Instant::now() + settings.connect_timeout;
        let uri = format!("http://{}", settings.address);
        let endpoint = tonic::transport::Endpoint::from_shared(uri.clone())
            .map_err(|e| GooseError::Gaggle {
                detail: format!("--manager-host {}: {e}", settings.address),
            })?
            .connect_timeout(Duration::from_secs(5))
            .http2_keep_alive_interval(HEARTBEAT_INTERVAL)
            .tcp_nodelay(true);
        let mut attempt = 0;
        loop {
            let tried = Self::try_connect(&endpoint, &settings).await;
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
        let response = client
            .connect(outbound(outbox.clone()))
            .await
            .map_err(|status| Refusal::Unreachable(status.message().to_string()))?;
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
        };
        let heard = Arc::new(Mutex::new(Instant::now()));
        let switch = Switch {
            fired: Arc::new(AtomicBool::new(false)),
            users: link.users.clone(),
            events: events_tx.clone(),
            outbox: link.outbox.clone(),
            heartbeat_timeout: settings.heartbeat_timeout,
            done: link.done.clone(),
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
            users.retain(|user| !user.is_finished());
            users.push(user);
        }
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
                }
            }
        });
    }

    /// Close the stream, once what is queued has left.
    pub(crate) fn close(&self) {
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
        let read = reader.read(&mut data[filled..]).await?;
        filled += read;
        let end = read == 0;
        if filled == LOG_CHUNK || end {
            outbox
                .push_chunk(LogChunk {
                    file: file.to_string(),
                    offset,
                    data: data[..filled].to_vec(),
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

/// The dead man's switch.
#[derive(Clone)]
struct Switch {
    fired: Arc<AtomicBool>,
    users: Arc<Mutex<Vec<tokio::task::AbortHandle>>>,
    events: flume::Sender<LinkEvent>,
    outbox: Arc<Outbox>,
    heartbeat_timeout: Duration,
    done: Arc<AtomicBool>,
}

impl Switch {
    /// Stop every user at once, tell the main loop why, and close the
    /// stream. A worker whose `execute()` has not returned 4 seconds later
    /// exits the process with an error status: load never outlives its
    /// manager.
    fn fire(&self, why: String) {
        if self.fired.swap(true, Ordering::Relaxed) {
            return;
        }
        error!("gaggle: lost the manager ({why}); stopping every user");
        if let Ok(users) = self.users.lock() {
            for user in users.iter() {
                user.abort();
            }
        }
        self.outbox.close();
        let _ = self.events.send(LinkEvent::Lost(why));
        let done = self.done.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(4));
            if !done.load(Ordering::Relaxed) {
                eprintln!("gaggle: lost the manager and the worker has not exited; exiting now");
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
            Some(FromManager::StartAt(start)) => LinkEvent::StartAt(
                tokio::time::Instant::now() + Duration::from_millis(start.delay_ms),
            ),
            Some(FromManager::ResetMetrics(reset)) => LinkEvent::ResetMetrics(reset.epoch),
            Some(FromManager::Stop(stop)) => LinkEvent::Stop(stop.reason),
            Some(FromManager::SendLogs(_)) => LinkEvent::SendLogs,
            Some(FromManager::Shutdown(_)) => {
                shutdown.store(true, Ordering::Relaxed);
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

    /// A heartbeat goes ahead of what is queued.
    #[tokio::test]
    async fn heartbeats_go_first() {
        let outbox = Outbox::default();
        outbox.push_delta(delta(1));
        outbox.push(Outgoing::Message(ToManager::Heartbeat(Heartbeat {})), true);
        let first = next(&outbox).await.unwrap();
        assert!(matches!(first.message, Some(ToManager::Heartbeat(_))));
    }

    #[test]
    fn backoff_is_one_two_four_eight_then_ten() {
        crate::gaggle::within(30, || {
            let waits: Vec<u64> = (0..7).map(|attempt| backoff(attempt).as_secs()).collect();
            assert_eq!(waits, [1, 2, 4, 8, 10, 10, 10]);
        });
    }
}
