//! A Gaggle manager's and a worker's part in a [`GooseAttack`] run.
//!
//! The manager is a `GooseAttack` with no users of its own: its phase
//! machine, run time and test plan clock are the run's clock, and its
//! metrics processor merges what every worker sends. A worker is a
//! `GooseAttack` that waits idle, as with `--no-autostart`, until its manager
//! says when to start the share it was handed, and is then driven by what the
//! manager sends.

use super::delta::DeltaLimits;
use super::link::{Link, LinkEvent, LinkSettings};
use super::manager::{Event, Manager};
use super::merge::MergeHealth;
use super::proto::{
    manager_message::Message as ToWorker, stopped::Reason, Prepare, ResetMetrics, SendLogs,
    StartAt, Stop, Stopped,
};
use super::spec::{self, LogSpec, WorkerRunSpec};
use crate::metrics::MetricsCommand;
use crate::runs::{EndedBy, GaggleLogLeft, GaggleRecord, GaggleWorkerRecord, GaggleWorkerState};
use crate::{AttackMode, AttackPhase, GooseAttack, GooseAttackRunState, GooseError, WorkerShare};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(test)]
use super::faults::{send_faulty_logs, UNASKED};
#[cfg(test)]
pub(crate) use super::faults::{
    LogsFault, PrepareFault, TestFaults, DISK_FULL, DISK_FULL_AFTER_LOGS, FAIL_LOG_WRITES,
    LOG_FAULT_AT,
};

// Each deadline below is shorter in unit tests, which wait it out; the
// process tests run the examples, built without `cfg(test)`, so with the
// real deadlines.

/// How long after `StartAt` every process starts: 2 seconds.
const START_DELAY: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(2)
};

/// How long the manager waits for every worker's answer to `Prepare`: 120
/// seconds.
const PREPARE_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_secs(3)
} else {
    Duration::from_secs(120)
};

/// How long the manager waits for a worker's final metrics once its users
/// stopped: 30 seconds.
pub(crate) const FINAL_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_secs(3)
} else {
    Duration::from_secs(30)
};

/// How long a worker waits for its metrics processor to hand over its final
/// metrics: 20 seconds, well inside the manager's [`FINAL_TIMEOUT`], so a
/// worker whose processor is late says so, and is lost for that, before the
/// manager gives up waiting.
const WORKER_FINAL_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_secs(2)
} else {
    Duration::from_secs(20)
};

// The worker's bound is at most two thirds of the manager's.
const _: () = assert!(WORKER_FINAL_TIMEOUT.as_millis() * 3 <= FINAL_TIMEOUT.as_millis() * 2);

/// How long a worker waits for its metrics processor to reset: 10 seconds.
const RESET_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_secs(2)
} else {
    Duration::from_secs(10)
};

/// How long the manager waits for the next full chunk, or the last, of a
/// worker's logs: a transfer that goes this long without one fails. No
/// deadline caps a transfer that keeps moving, whatever its size. 60
/// seconds, and 6 in unit tests, more than the 5 a canceled collection may
/// take there, so a cancel and an idle timeout are told apart.
pub(crate) const LOG_IDLE_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_secs(6)
} else {
    Duration::from_secs(60)
};

/// How often the manager's waits before and after its run look at the
/// killswitch.
const KILLSWITCH_POLL: Duration = Duration::from_millis(100);

/// How far past its own series clock the manager takes a worker's second:
/// the workers' clocks start within milliseconds of its own, and a delta is
/// stamped up to a second late.
pub(crate) const CLOCK_SLACK: u64 = 10;

/// What a [`GooseAttack`] keeps for a Gaggle.
#[derive(Default)]
pub(crate) struct GaggleState {
    /// The manager or worker side, with `--manager` or `--worker`.
    pub(crate) role: Option<GaggleRole>,
    /// How the `--gaggle-shadow-merge` self check ended, once it has.
    pub(crate) shadow: Option<Arc<std::sync::Mutex<Option<super::shadow::ShadowOutcome>>>>,
    /// The client options the command line gave: a manager hands its own to
    /// each worker, and a worker takes them in place of its own.
    pub(crate) client_command_line: crate::client::CommandLineClientOptions,
    /// Where a manager reports the address it listens on (unit tests).
    #[cfg(test)]
    pub(crate) bound: Option<tokio::sync::oneshot::Sender<std::net::SocketAddr>>,
    /// What a worker does wrong on purpose (unit tests).
    #[cfg(test)]
    pub(crate) test_faults: TestFaults,
    /// Where a manager reports each worker joining (unit tests).
    #[cfg(test)]
    pub(crate) joins: Option<tokio::sync::mpsc::UnboundedSender<usize>>,
    /// Where a worker puts its link's done flag, set once its `execute()`
    /// returns, which stops the dead man's switch's last resort (unit tests).
    #[cfg(test)]
    pub(crate) link_done: Arc<std::sync::Mutex<Option<Arc<std::sync::atomic::AtomicBool>>>>,
    /// Where the client settings every user gets are reported (unit tests).
    #[cfg(test)]
    pub(crate) client_report:
        Option<tokio::sync::oneshot::Sender<crate::client::GooseClientConfig>>,
}

/// This process's side of a Gaggle.
pub(crate) enum GaggleRole {
    Manager(Box<ManagerRun>),
    Worker(Box<WorkerRun>),
}

/// One of a manager's workers.
struct WorkerEntry {
    id: u32,
    name: String,
    users: usize,
    /// When it said its users stopped: its final metrics follow.
    users_stopped: Option<Instant>,
    stopped: Option<Reason>,
    lost: Option<(chrono::DateTime<chrono::Utc>, String)>,
    /// It stopped as told and then went away: not lost, but nothing more,
    /// its logs among them, will come from it.
    gone: bool,
    run_folder: Option<String>,
}

/// The manager's side of a run.
pub(crate) struct ManagerRun {
    manager: Manager,
    workers: Vec<WorkerEntry>,
    /// The logs every worker writes.
    logs: Vec<LogSpec>,
    logs_left: Vec<GaggleLogLeft>,
    /// The series second of the last clock sent to the merge.
    clock_second: Option<usize>,
    /// `Stop` was sent: a worker that stops now is not lost.
    stop_sent: bool,
    /// The killswitch had canceled the run when `Stop` was sent, and no
    /// Ctrl-C came while the workers sent their final metrics: the log
    /// collection sets that cancel aside, so it still collects.
    canceled_at_stop: bool,
    /// The run already ended for every worker lost or every worker's users
    /// exited.
    ending: bool,
    /// What the merge did with each worker's deltas.
    health: MergeHealth,
}

/// The worker's side of a run.
pub(crate) struct WorkerRun {
    link: Link,
    spec: WorkerRunSpec,
    /// When to start, once `StartAt` came.
    start_at: Option<tokio::time::Instant>,
    /// When this worker started, against `start_at`, for the log.
    started_late: Option<Duration>,
    /// The series second of the last delta queued.
    tick_second: Option<usize>,
    /// A metrics reset that came before the start, applied once started.
    pending_reset: Option<u64>,
    /// Each log, as the manager names it, and where it is here.
    log_files: Vec<(String, PathBuf)>,
    run_folder: Option<PathBuf>,
    /// Ignore the manager's metrics reset (unit tests).
    #[cfg(test)]
    skip_reset: bool,
    /// Its [`TestFaults::stop_reason`] was sent (unit tests).
    #[cfg(test)]
    stop_reason_sent: bool,
}

impl WorkerEntry {
    /// Mark it lost, unless it is already, or it stopped as told and then
    /// went away, which marks it gone instead. Whether it is newly lost.
    fn lose(&mut self, stop_sent: bool, why: String) -> bool {
        if self.lost.is_some() {
            return false;
        }
        if stop_sent && self.stopped.is_some() {
            self.gone = true;
            return false;
        }
        self.lost = Some((chrono::Utc::now(), why));
        true
    }

    /// Whether it can still send its logs: it is neither lost nor gone, and
    /// it ran, so has a run folder they are in. A worker that never started
    /// has none.
    fn sends_logs(&self) -> bool {
        self.lost.is_none() && !self.gone && self.run_folder.is_some()
    }
}

impl super::WorkerId for WorkerEntry {
    fn worker_id(&self) -> u32 {
        self.id
    }
}

impl ManagerRun {
    /// Worker `id`.
    fn worker(&mut self, id: u32) -> Option<&mut WorkerEntry> {
        super::worker_mut(&mut self.workers, id)
    }

    /// Workers neither lost nor stopped.
    fn running(&self) -> impl Iterator<Item = &WorkerEntry> {
        self.workers
            .iter()
            .filter(|worker| worker.lost.is_none() && worker.stopped.is_none())
    }
}

/// Whether the metrics processor answered on `reply` within `wait`. A reply
/// channel it dropped is no answer, as a timeout is not.
async fn answered<T>(reply: tokio::sync::oneshot::Receiver<T>, wait: Duration) -> bool {
    matches!(tokio::time::timeout(wait, reply).await, Ok(Ok(_)))
}

/// A worker that cannot finish a phase of its run: it stops every user, tells
/// the manager it failed (no final metrics follow, so the manager marks it
/// lost), gives that a moment to leave, and exits with an error.
async fn worker_failed(link: &Link, why: String) -> GooseError {
    error!("gaggle: {why}; this worker failed");
    link.stop_users();
    link.stopped(Stopped {
        reason: Reason::Failed as i32,
        canceled: why.clone(),
        run_folder: String::new(),
    });
    link.flush().await;
    gaggle_error(why)
}

/// The failure that ends a manager before its run, `error`: every worker is
/// told to shut down first.
async fn fail(manager: &mut Manager, error: GooseError) -> GooseError {
    error!("gaggle: {error}");
    manager.shut_down_all().await;
    error
}

/// A Gaggle error saying `detail`.
fn gaggle_error(detail: String) -> GooseError {
    GooseError::Gaggle { detail }
}

/// The error that ends a manager canceled `while`, once the killswitch is
/// triggered (Ctrl-C).
fn canceled(while_: &str) -> Option<GooseError> {
    crate::killswitch_reason()
        .map(|reason| gaggle_error(format!("canceled while {while_}: {reason}")))
}

/// Clear the killswitch for a wait, so a Ctrl-C during it is caught by the
/// wait rather than ending the process: the cancel it held, for
/// [`put_back_cancel`] to restore once the wait is over.
fn set_cancel_aside() -> Option<String> {
    let reason = crate::killswitch_reason();
    if reason.is_some() {
        crate::reset_killswitch();
    }
    reason
}

/// Put back a cancel [`set_cancel_aside`] cleared, without logging it again:
/// the killswitch reads as it did before the wait, its first reason kept,
/// whether or not a Ctrl-C came during it.
fn put_back_cancel(reason: Option<String>) {
    if let Some(reason) = reason {
        match crate::CANCELED.write() {
            Ok(mut canceled) => *canceled = Some(reason),
            Err(poisoned) => *poisoned.into_inner() = Some(reason),
        }
    }
}

/// The manager's next event before `deadline`, `None` at the deadline, or
/// the canceled error once the killswitch is triggered: it is checked every
/// [`KILLSWITCH_POLL`].
async fn next_event_or_canceled(
    manager: &Manager,
    deadline: Instant,
    while_: &str,
) -> Result<Option<Event>, GooseError> {
    loop {
        if let Some(error) = canceled(while_) {
            return Err(error);
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        if let Some(event) = manager
            .next_event(deadline.min(now + KILLSWITCH_POLL))
            .await
        {
            return Ok(Some(event));
        }
    }
}

impl GooseAttack {
    /// A manager's step of a ramp up: its users run on its workers, so it
    /// counts them as its plan starts them, to keep the run's clock. Whether
    /// this process is a manager, which starts no user of its own.
    pub(crate) async fn gaggle_manager_increase(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        increase_rate: f32,
    ) -> bool {
        if !self.is_gaggle_manager() {
            return false;
        }
        if goose_attack_run_state.adjust_due() {
            goose_attack_run_state
                .schedule_next_adjust(Duration::from_secs_f64(1.0 / f64::from(increase_rate)));
            self.count_started_user(goose_attack_run_state);
        } else {
            goose_attack_run_state.sleep_until_adjust().await;
        }
        true
    }

    /// A manager's step of a ramp down, once a user is due to stop: it
    /// counts one fewer. Whether this process is a manager, which stops no
    /// user of its own.
    pub(crate) fn gaggle_manager_decrease(
        &self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> bool {
        if !self.is_gaggle_manager() {
            return false;
        }
        goose_attack_run_state.completed_users += 1;
        goose_attack_run_state.active_users -= 1;
        true
    }

    /// What a manager's merge refuses a delta past, and where it records
    /// what it did with each worker's deltas: user ordinals below the
    /// users this run starts, worker ids below the workers it expects.
    pub(crate) fn gaggle_merge_settings(&self) -> (DeltaLimits, MergeHealth) {
        let health = match self.gaggle.role.as_ref() {
            Some(GaggleRole::Manager(run)) => run.health.clone(),
            _ => MergeHealth::default(),
        };
        let workers = self.configuration.expect_workers.unwrap_or(1);
        let limits = DeltaLimits {
            max_users: self.test_plan.total_users() as u64,
            // At most MAX_WORKERS, which validate_gaggle enforces.
            max_workers: workers as u32,
            ..DeltaLimits::default()
        };
        (limits, health)
    }

    /// Have a worker's dead man's switch stop user task `user` at once, once
    /// its manager is gone.
    pub(crate) fn gaggle_watch_user<T>(&self, user: &tokio::task::JoinHandle<T>) {
        if let Some(link) = self.gaggle_link() {
            link.watch(user.abort_handle());
        }
    }

    /// The link to a worker's manager.
    pub(crate) fn gaggle_link(&self) -> Option<&Link> {
        match self.gaggle.role.as_ref() {
            Some(GaggleRole::Worker(run)) => Some(&run.link),
            _ => None,
        }
    }

    /// Have a worker do something wrong on purpose (unit tests).
    #[cfg(test)]
    pub(crate) fn gaggle_test_faults(mut self, faults: TestFaults) -> Self {
        self.gaggle.test_faults = faults;
        self
    }

    /// Where a worker puts its link's done flag, once it joined (unit
    /// tests).
    #[cfg(test)]
    pub(crate) fn link_done_record(
        &self,
    ) -> Arc<std::sync::Mutex<Option<Arc<std::sync::atomic::AtomicBool>>>> {
        self.gaggle.link_done.clone()
    }

    /// Have a worker report the client settings its users get (unit tests).
    #[cfg(test)]
    pub(crate) fn report_client_config(
        mut self,
        report: tokio::sync::oneshot::Sender<crate::client::GooseClientConfig>,
    ) -> Self {
        self.gaggle.client_report = Some(report);
        self
    }

    /// Have the manager report how many workers joined, as each does (unit
    /// tests).
    #[cfg(test)]
    pub(crate) fn report_joins(mut self, joins: tokio::sync::mpsc::UnboundedSender<usize>) -> Self {
        self.gaggle.joins = Some(joins);
        self
    }

    /// Have the manager report the address it listens on (unit tests).
    #[cfg(test)]
    pub(crate) fn report_manager_address(
        mut self,
        bound: tokio::sync::oneshot::Sender<std::net::SocketAddr>,
    ) -> Self {
        self.gaggle.bound = Some(bound);
        self
    }

    /// The manager, before its run: listen, wait for every worker, confirm
    /// the report files can be created, run `test_start` once, hand each
    /// worker its share and custom data, wait for each to prepare, then tell
    /// all of them to start in [`START_DELAY`] and wait for that instant,
    /// when its own run starts. A failure shuts every worker down, and once
    /// `test_start` ran, runs `test_stop`.
    pub(crate) async fn gaggle_manager_start(&mut self) -> Result<(), GooseError> {
        // Ctrl-C cancels the waits before the run too.
        crate::util::setup_ctrlc_handler();
        let (host, port) = self.configuration.manager_bind();
        let expected = self.configuration.expect_workers.unwrap_or(1);
        let connect_timeout = self.configuration.gaggle_connect_timeout();
        let mut manager = Manager::start(
            &host,
            port,
            expected,
            self.configuration.gaggle_token().map(str::to_string),
            self.metrics.hash,
            !self.configuration.no_hash_check,
            self.configuration.gaggle_heartbeat_timeout(),
        )
        .await?;
        #[cfg(test)]
        if let Some(bound) = self.gaggle.bound.take() {
            let _ = bound.send(manager.local_addr);
        }

        let deadline = Instant::now() + connect_timeout;
        let mut joined = 0;
        while joined < expected {
            let event = match next_event_or_canceled(&manager, deadline, "workers joined").await {
                Ok(event) => event,
                Err(e) => return Err(fail(&mut manager, e).await),
            };
            match event {
                Some(Event::Joined) => {
                    joined += 1;
                    #[cfg(test)]
                    if let Some(joins) = self.gaggle.joins.as_ref() {
                        let _ = joins.send(joined);
                    }
                }
                Some(Event::Lost { id, why }) => {
                    return Err(fail(&mut manager, gaggle_error(format!("worker {id} was lost before the run started: {why}")),
                    )
                    .await)
                }
                Some(_) => {}
                None => {
                    return Err(fail(&mut manager, gaggle_error(format!(
                            "{joined} of {expected} workers joined within {} seconds (--gaggle-connect-timeout)",
                            connect_timeout.as_secs()
                        )),
                    )
                    .await)
                }
            }
        }
        info!("gaggle: all {expected} workers joined");

        // The report files can be created: a path that cannot fails here,
        // before test_start runs or any worker starts.
        if let Err(e) = self.create_reports().await {
            return Err(fail(&mut manager, e).await);
        }

        // test_start runs once, here, before any worker starts.
        if let Err(e) = self.run_test_start().await {
            return Err(fail(&mut manager, e).await);
        }

        // test_start ran: any failure from here runs test_stop too, once, as
        // after any run.
        let workers = match self.gaggle_prepare_workers(&manager, expected).await {
            Ok(workers) => workers,
            Err(e) => {
                let e = fail(&mut manager, e).await;
                if let Err(stop) = self.run_test_stop().await {
                    warn!("test_stop after the manager failed: {stop}");
                }
                return Err(e);
            }
        };

        self.gaggle.role = Some(GaggleRole::Manager(Box::new(ManagerRun {
            manager,
            workers,
            logs: spec::log_specs(&self.configuration),
            logs_left: Vec::new(),
            clock_second: None,
            stop_sent: false,
            canceled_at_stop: false,
            ending: false,
            health: MergeHealth::default(),
        })));
        info!(
            "gaggle: starting at {}",
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
        );
        Ok(())
    }

    /// Hand each worker its share and custom data, wait for each to prepare,
    /// then tell all of them to start in [`START_DELAY`] and wait for that
    /// instant: the manager's workers, as it keeps them for the run.
    async fn gaggle_prepare_workers(
        &mut self,
        manager: &Manager,
        expected: usize,
    ) -> Result<Vec<WorkerEntry>, GooseError> {
        let total = self.test_plan.total_users();
        let order = self.user_scenarios(total)?;
        let specs = spec::worker_specs(
            &self.configuration,
            self.gaggle.client_command_line,
            &self.test_plan.steps,
            &order,
            expected,
            self.configuration.records_graph_data(),
        );
        let names = manager.workers();
        let mut workers = Vec::with_capacity(expected);
        // One worker at a time: its data is made, encoded into its Prepare
        // and dropped before the next worker's is made. Each encoded Prepare
        // then waits in its worker's send queue until the link takes it.
        for (mut spec, (id, name)) in specs.into_iter().zip(names) {
            let share = WorkerShare {
                index: spec.worker as usize,
                workers: expected,
                users: spec.users,
                user_offset: spec.user_offset,
            };
            spec.custom_data = self.call_worker_data_hook(&share)?.unwrap_or_default();
            let users = spec.users;
            let encoded = spec.encode();
            drop(spec);
            let bytes =
                encoded.map_err(|e| gaggle_error(format!("could not encode a run spec: {e}")))?;
            manager.send(id, ToWorker::Prepare(Prepare { spec: bytes }));
            workers.push(WorkerEntry {
                id,
                name,
                users,
                users_stopped: None,
                stopped: None,
                lost: None,
                gone: false,
                run_folder: None,
            });
        }

        let deadline = Instant::now() + PREPARE_TIMEOUT;
        let mut ready = BTreeSet::new();
        while ready.len() < expected {
            match next_event_or_canceled(manager, deadline, "workers prepared").await? {
                Some(Event::PrepareAck { id, error: None }) => {
                    ready.insert(id);
                }
                Some(Event::PrepareAck {
                    id,
                    error: Some(error),
                }) => {
                    return Err(gaggle_error(format!(
                        "worker {id} could not prepare its share: {error}"
                    )))
                }
                Some(Event::Lost { id, why }) => {
                    return Err(gaggle_error(format!(
                        "worker {id} was lost while preparing: {why}"
                    )))
                }
                Some(_) => {}
                None => {
                    return Err(gaggle_error(format!(
                        "{} of {expected} workers prepared within {} seconds",
                        ready.len(),
                        PREPARE_TIMEOUT.as_secs()
                    )))
                }
            }
        }

        let start = tokio::time::Instant::now() + START_DELAY;
        manager.broadcast(ToWorker::StartAt(StartAt {
            delay_ms: START_DELAY.as_millis() as u64,
        }));
        info!(
            "gaggle: every worker is ready; starting in {} seconds",
            START_DELAY.as_secs()
        );
        // The events that come meanwhile wait for the run.
        while tokio::time::Instant::now() < start {
            if let Some(error) = canceled("workers waited to start") {
                return Err(error);
            }
            tokio::time::sleep_until(start.min(tokio::time::Instant::now() + KILLSWITCH_POLL))
                .await;
        }
        Ok(workers)
    }

    /// The manager, failing once its workers were told to start but before
    /// its own run began, as when its dashboard cannot listen: every worker
    /// is shut down, and `test_stop` runs, as `test_start` did.
    pub(crate) async fn gaggle_manager_failed_to_start(&mut self) {
        let Some(GaggleRole::Manager(run)) = self.gaggle.role.as_mut() else {
            return;
        };
        run.manager.shut_down_all().await;
        if let Err(stop) = self.run_test_stop().await {
            warn!("test_stop after the manager failed: {stop}");
        }
    }

    /// The worker, before its run: connect to the manager, wait for its
    /// share, and configure this process to run it. The Controllers and the
    /// dashboard stay off.
    pub(crate) async fn gaggle_worker_join(&mut self) -> Result<(), GooseError> {
        self.attack_mode = AttackMode::Worker;
        self.configuration.no_telnet = true;
        self.configuration.websocket = false;
        self.configuration.dashboard = false;
        let hello = LinkSettings::hello(
            crate::plan_hash(&self.scenarios, &self.scheduler),
            super::link::worker_name(),
        );
        let link = Link::connect(LinkSettings {
            address: self.configuration.manager_address(),
            connect_timeout: self.configuration.gaggle_connect_timeout(),
            heartbeat_timeout: self.configuration.gaggle_heartbeat_timeout(),
            hello,
            token: self.configuration.gaggle_token().map(str::to_string),
        })
        .await?;
        #[cfg(test)]
        if let Ok(mut record) = self.gaggle.link_done.lock() {
            *record = Some(link.done.clone());
        }
        // The manager waits for its other workers and runs test_start: the
        // heartbeats, and the dead man's switch, cover the wait.
        let bytes = loop {
            match link.next_event(Duration::from_secs(1)).await {
                Some(LinkEvent::Prepare(bytes)) => break bytes,
                Some(LinkEvent::Lost(why)) => {
                    return Err(gaggle_error(format!(
                        "lost the manager before the run started: {why}"
                    )))
                }
                Some(LinkEvent::Shutdown) => {
                    return Err(gaggle_error(
                        "the manager shut down before the run started".to_string(),
                    ))
                }
                _ => {}
            }
        };
        let decoded = WorkerRunSpec::decode(&bytes)
            .map_err(|e| format!("could not read the run spec: {e}"))
            .and_then(|spec| spec.validate(self.scenarios.len()).map(|()| spec));
        #[cfg(test)]
        let decoded = self.gaggle.test_faults.on_prepare(&link, decoded).await?;
        let mut spec = match decoded {
            Ok(spec) => spec,
            Err(detail) => {
                link.prepared(Some(detail.clone()));
                link.flush().await;
                return Err(gaggle_error(detail));
            }
        };
        info!(
            "gaggle: worker {} of {}: {} users from ordinal {}",
            spec.worker, spec.workers, spec.users, spec.user_offset
        );
        spec.configure(&mut self.configuration);
        // The loggers take the load test's defaults again as each run starts:
        // the manager's spec decides every log, so none set in code returns.
        self.defaults.clear_logs();
        self.gaggle.client_command_line = spec.client_command_line;
        self.set_user_slice(spec.user_offset, spec.slice.clone());
        self.worker_data = Some(Arc::from(std::mem::take(&mut spec.custom_data)));
        #[cfg(test)]
        let skip_reset = self.gaggle.test_faults.skip_reset;
        #[cfg(test)]
        self.gaggle.test_faults.arm(&link.outbox);
        self.gaggle.role = Some(GaggleRole::Worker(Box::new(WorkerRun {
            link,
            spec,
            start_at: None,
            started_late: None,
            pending_reset: None,
            tick_second: None,
            log_files: Vec::new(),
            run_folder: None,
            #[cfg(test)]
            skip_reset,
            #[cfg(test)]
            stop_reason_sent: false,
        })));
        Ok(())
    }

    /// Tell the manager whether the worker's users are ready.
    pub(crate) fn gaggle_worker_prepared(&self, error: Option<&GooseError>) {
        if let Some(link) = self.gaggle_link() {
            link.prepared(error.map(ToString::to_string));
        }
    }

    /// The worker, idle: start at the instant the manager gave, or wait a
    /// little for it.
    pub(crate) async fn gaggle_worker_idle(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        let start_at = match self.gaggle.role.as_mut() {
            Some(GaggleRole::Worker(run)) => run.start_at,
            _ => None,
        };
        let Some(at) = start_at else {
            tokio::time::sleep(Duration::from_millis(100)).await;
            return Ok(());
        };
        let now = tokio::time::Instant::now();
        if at > now + Duration::from_millis(100) {
            tokio::time::sleep(Duration::from_millis(100)).await;
            return Ok(());
        }
        tokio::time::sleep_until(at).await;
        let late = tokio::time::Instant::now().saturating_duration_since(at);
        if let Some(GaggleRole::Worker(run)) = self.gaggle.role.as_mut() {
            run.start_at = None;
            run.started_late = Some(late);
        }
        info!(
            "gaggle: starting at {}, {} us after the start instant the manager gave",
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
            late.as_micros()
        );
        self.reset_run_state(goose_attack_run_state).await?;
        self.metrics
            .history
            .push(crate::test_plan::TestPlanHistory::step(
                crate::test_plan::TestPlanStepAction::Increasing,
                0,
            ));
        self.set_attack_phase(goose_attack_run_state, AttackPhase::Increase);
        let pending = match self.gaggle.role.as_mut() {
            Some(GaggleRole::Worker(run)) => run.pending_reset.take(),
            _ => None,
        };
        if let Some(epoch) = pending {
            debug!("gaggle: metrics reset that came before the start, epoch {epoch}");
            if let Err(why) = self.gaggle_worker_reset(goose_attack_run_state).await {
                return Err(match self.gaggle_link() {
                    Some(link) => worker_failed(link, why).await,
                    None => gaggle_error(why),
                });
            }
        }
        // A worker holding no users, with --iterations, has nothing to
        // run: its users have all exited, as the manager waits to hear.
        if self.test_plan.total_users() == 0 && self.configuration.iterations() > 0 {
            self.set_ended_by(EndedBy::UsersExited, None);
            self.cancel_attack(goose_attack_run_state).await?;
        }
        Ok(())
    }

    /// Once each pass of the main loop: a manager forwards what its workers
    /// sent to the merge and acts on lost workers; a worker queues each
    /// second's delta and acts on what its manager sent.
    pub(crate) async fn gaggle_tick(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        let Some(mut role) = self.gaggle.role.take() else {
            return Ok(());
        };
        let result = match &mut role {
            GaggleRole::Manager(run) => self.manager_tick(goose_attack_run_state, run).await,
            GaggleRole::Worker(run) => self.worker_tick(goose_attack_run_state, run).await,
        };
        self.gaggle.role = Some(role);
        result
    }

    async fn manager_tick(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        run: &mut ManagerRun,
    ) -> Result<(), GooseError> {
        if self.series_started.is_some() {
            let now = self.series_elapsed_secs();
            if run.clock_second != Some(now) {
                run.clock_second = Some(now);
                let _ = goose_attack_run_state
                    .metrics_cmd_tx
                    .send(MetricsCommand::GaggleClock {
                        second: now as u64 + CLOCK_SLACK,
                    });
            }
        }
        while let Some(event) = run.manager.try_event() {
            self.manager_event(goose_attack_run_state, run, event);
        }
        if run.ending || run.stop_sent || !self.run_in_progress {
            return Ok(());
        }
        let running = matches!(
            self.attack_phase,
            AttackPhase::Increase | AttackPhase::Maintain | AttackPhase::Decrease
        );
        if !running || goose_attack_run_state.stopping {
            return Ok(());
        }
        if run.workers.iter().all(|worker| worker.lost.is_some()) {
            run.ending = true;
            error!("gaggle: every worker was lost; ending the run");
            self.set_ended_by(
                EndedBy::WorkersLost,
                Some("every Gaggle worker was lost".to_string()),
            );
            self.cancel_attack(goose_attack_run_state).await?;
        } else if run.running().next().is_none()
            && run
                .workers
                .iter()
                .any(|worker| worker.stopped == Some(Reason::UsersExited))
        {
            run.ending = true;
            info!("gaggle: every worker's users exited");
            self.set_ended_by(EndedBy::UsersExited, None);
            self.cancel_attack(goose_attack_run_state).await?;
        }
        Ok(())
    }

    /// Act on one event from a worker.
    fn manager_event(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        run: &mut ManagerRun,
        event: Event,
    ) {
        match event {
            Event::Delta { id, delta, bytes } => {
                let Some(worker) = run.worker(id) else {
                    return;
                };
                // The results hold what a lost worker sent before it was lost.
                if worker.lost.is_some() {
                    return;
                }
                // Whether the merge takes a delta depends on all it holds of
                // the worker (its sequence, epoch, final delta and names),
                // so the metrics processor decides, and records the last
                // delta it merged in `run.health`: a refused delta never
                // counts as received.
                let _ = goose_attack_run_state
                    .metrics_cmd_tx
                    .send(MetricsCommand::GaggleApply {
                        worker: id,
                        delta,
                        bytes,
                    });
            }
            Event::Stopped { id, stopped } => {
                let stop_sent = run.stop_sent;
                // No final metrics came, nor will: lost, whenever it says so.
                // A reason this manager does not know, or none, is taken as
                // the same, never as a stop.
                let reason = match <Reason as std::convert::TryFrom<i32>>::try_from(stopped.reason)
                {
                    Ok(Reason::Failed) => {
                        let why = format!("it could not finish its run: {}", stopped.canceled);
                        self.lose_worker(goose_attack_run_state, run, id, why);
                        return;
                    }
                    Ok(Reason::Unspecified) | Err(_) => {
                        let why = format!(
                            "it stopped for an unknown reason, {}: {}",
                            stopped.reason, stopped.canceled
                        );
                        self.lose_worker(goose_attack_run_state, run, id, why);
                        return;
                    }
                    Ok(reason) => reason,
                };
                let Some(worker) = run.worker(id) else {
                    return;
                };
                worker.stopped = Some(reason);
                worker.run_folder = Some(stopped.run_folder).filter(|folder| !folder.is_empty());
                if reason == Reason::Canceled && !stop_sent && worker.lost.is_none() {
                    let why = format!("it was stopped on the worker: {}", stopped.canceled);
                    self.lose_worker(goose_attack_run_state, run, id, why);
                } else if worker.lost.is_none() {
                    // It sends no more deltas, so it no longer holds back
                    // which seconds are complete, whether or not its final
                    // merged.
                    let _ = goose_attack_run_state
                        .metrics_cmd_tx
                        .send(MetricsCommand::GaggleDone { worker: id });
                }
            }
            Event::UsersStopped { id } => {
                if let Some(worker) = run.worker(id) {
                    worker.users_stopped.get_or_insert_with(Instant::now);
                }
            }
            Event::Lost { id, why } => self.lose_worker(goose_attack_run_state, run, id, why),
            Event::Joined | Event::PrepareAck { .. } | Event::Log { .. } => {}
        }
    }

    /// Mark worker `id` lost: what it sent stays merged, the run goes on with
    /// the others, and its stream, if still open, gets `Shutdown`.
    fn lose_worker(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        run: &mut ManagerRun,
        id: u32,
        why: String,
    ) {
        let stop_sent = run.stop_sent;
        let merged = last_merged(&run.health, id);
        let Some(worker) = run.worker(id) else {
            return;
        };
        let message = format!(
            "gaggle: worker {id} ({}) lost: {why}; last delta merged {merged}",
            worker.name
        );
        if !worker.lose(stop_sent, why) {
            return;
        }
        warn!("{message}");
        run.manager.shut_down(id);
        let _ = goose_attack_run_state
            .metrics_cmd_tx
            .send(MetricsCommand::GaggleLost { worker: id });
    }

    async fn worker_tick(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        run: &mut WorkerRun,
    ) -> Result<(), GooseError> {
        let running = matches!(
            self.attack_phase,
            AttackPhase::Increase | AttackPhase::Maintain | AttackPhase::Decrease
        );
        #[cfg(test)]
        if running && self.gaggle.test_faults.exit_mid_run && self.series_elapsed_secs() >= 1 {
            return Err(gaggle_error("ended mid run on purpose".to_string()));
        }
        #[cfg(test)]
        if running && self.series_elapsed_secs() >= 1 {
            self.gaggle
                .test_faults
                .stop_once(&run.link, &mut run.stop_reason_sent);
        }
        if running && self.series_started.is_some() {
            let now = self.series_elapsed_secs();
            if run.tick_second != Some(now) {
                run.tick_second = Some(now);
                let _ = goose_attack_run_state
                    .metrics_cmd_tx
                    .send(MetricsCommand::GaggleTick {
                        now,
                        outbox: run.link.outbox.clone(),
                    });
            }
        }
        while let Some(event) = run.link.try_event() {
            match event {
                LinkEvent::Prepare(_) => warn!("gaggle: a second Prepare ignored"),
                LinkEvent::StartAt(at) => run.start_at = Some(at),
                #[cfg(test)]
                LinkEvent::ResetMetrics(epoch) if run.skip_reset => {
                    warn!("gaggle: metrics reset to epoch {epoch} ignored on purpose");
                }
                LinkEvent::ResetMetrics(epoch) if !running => {
                    // Before the start, as when the ramp ends in the
                    // milliseconds before this worker's start instant: reset
                    // once started, or the merge leaves this worker out.
                    if run.start_at.is_some() {
                        run.pending_reset = Some(epoch);
                    } else {
                        warn!("gaggle: a metrics reset while idle ignored, epoch {epoch}");
                    }
                }
                LinkEvent::ResetMetrics(epoch) => {
                    debug!("gaggle: metrics reset, epoch {epoch}");
                    if let Err(why) = self.gaggle_worker_reset(goose_attack_run_state).await {
                        return Err(worker_failed(&run.link, why).await);
                    }
                    let own = goose_attack_run_state
                        .metrics_epoch
                        .load(std::sync::atomic::Ordering::Relaxed);
                    if own != epoch {
                        warn!("gaggle: this worker's metrics epoch is {own}, the manager's {epoch}: its metrics may be left out of the merge");
                    }
                }
                LinkEvent::Stop(reason) => {
                    // A Stop before the start, as when the manager's run ends in
                    // the milliseconds before this worker's start instant:
                    // never start, and say so.
                    if !running && run.start_at.take().is_some() {
                        run.link.stopped(Stopped {
                            reason: Reason::Stopped as i32,
                            canceled: String::new(),
                            run_folder: String::new(),
                        });
                    }
                    #[cfg(test)]
                    if self.gaggle.test_faults.cancel_on_stop {
                        *LOG_FAULT_AT.lock().unwrap() = Some(std::time::Instant::now());
                        crate::trigger_killswitch("a test's Ctrl-C as the run stops");
                    }
                    if running && !goose_attack_run_state.stopping {
                        info!("gaggle: the manager stopped the run: {reason}");
                        self.set_ended_by(EndedBy::Stopped, None);
                        self.cancel_attack(goose_attack_run_state).await?;
                    }
                }
                #[cfg(test)]
                LinkEvent::SendLogs if self.gaggle.test_faults.logs != LogsFault::None => {
                    let fault = self.gaggle.test_faults.logs;
                    let files = run.log_files.iter().map(|(file, _)| file.clone()).collect();
                    send_faulty_logs(fault, run.link.outbox.clone(), files);
                    if fault == LogsFault::Slow {
                        tokio::time::sleep(Duration::from_secs(10)).await;
                    }
                }
                LinkEvent::SendLogs => {
                    info!(
                        "gaggle: sending {} logs to the manager",
                        run.log_files.len()
                    );
                    run.link.send_logs(run.log_files.clone());
                }
                LinkEvent::Shutdown => {
                    info!("gaggle: the manager said shut down");
                    goose_attack_run_state.shutdown_after_stop = true;
                    if self.attack_phase == AttackPhase::Idle {
                        self.set_attack_phase(goose_attack_run_state, AttackPhase::Shutdown);
                    } else if !goose_attack_run_state.stopping {
                        self.set_ended_by(EndedBy::Stopped, None);
                        self.cancel_attack(goose_attack_run_state).await?;
                    }
                }
                LinkEvent::Lost(why) => {
                    return Err(gaggle_error(format!("lost the manager: {why}")))
                }
                LinkEvent::Refused(why) => {
                    error!("gaggle: {why}");
                    return Err(gaggle_error(why));
                }
            }
        }
        Ok(())
    }

    /// The manager's metrics reset: a worker resets its own metrics, the
    /// users still running counted again. An error, why, when its metrics
    /// processor does not answer within [`RESET_TIMEOUT`].
    async fn gaggle_worker_reset(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), String> {
        self.update_duration();
        let (ack, acked) = tokio::sync::oneshot::channel();
        let command = self.reset_command(goose_attack_run_state, ack);
        let not_reset = || {
            format!(
                "the metrics processor did not reset within {} seconds",
                RESET_TIMEOUT.as_secs()
            )
        };
        // A unit test's fault: a processor that never answers. The command
        // is held unsent, its reply channel open, while the wait runs out.
        #[cfg(test)]
        if self.gaggle.test_faults.silent_reset {
            let _unsent = command;
            let _ = answered(acked, RESET_TIMEOUT).await;
            return Err(not_reset());
        }
        let sent = goose_attack_run_state.metrics_cmd_tx.send(command).is_ok();
        if sent && !answered(acked, RESET_TIMEOUT).await {
            return Err(not_reset());
        }
        goose_attack_run_state.running_metrics_timer = std::time::Instant::now();
        self.started = Some(std::time::Instant::now());
        Ok(())
    }

    /// The manager's metrics reset, at the end of its ramp: every worker
    /// resets too.
    pub(crate) fn gaggle_manager_reset(&self, epoch: u64) {
        if let Some(GaggleRole::Manager(run)) = self.gaggle.role.as_ref() {
            info!("gaggle: resetting every worker's metrics");
            run.manager
                .broadcast(ToWorker::ResetMetrics(ResetMetrics { epoch }));
        }
    }

    /// The end of a run, once every local user stopped and before
    /// `test_stop`: a manager stops every worker and waits for each to say
    /// its users stopped, then for its final metrics; a worker says its
    /// users stopped, sends its final metrics, then `Stopped`.
    pub(crate) async fn gaggle_end_of_run(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        let Some(mut role) = self.gaggle.role.take() else {
            return Ok(());
        };
        let result = match &mut role {
            GaggleRole::Manager(run) => {
                self.manager_stop(goose_attack_run_state, run).await;
                Ok(())
            }
            GaggleRole::Worker(run) => self.worker_stopped(goose_attack_run_state, run).await,
        };
        self.gaggle.role = Some(role);
        result
    }

    async fn manager_stop(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        run: &mut ManagerRun,
    ) {
        run.stop_sent = true;
        // A cancel that ended the run is set aside while the workers send
        // their final metrics, so a Ctrl-C now ends the wait rather than the
        // process; it is put back for test_stop and the log collection.
        let aside = set_cancel_aside();
        run.canceled_at_stop = aside.is_some();
        let reason = match &self.ended_by {
            Some((EndedBy::Canceled, Some(reason))) => format!("canceled: {reason}"),
            Some((EndedBy::WorkersLost, _)) => "every other worker was lost".to_string(),
            Some((EndedBy::UsersExited, _)) => "every user exited".to_string(),
            Some((EndedBy::Stopped, _)) => "stopped".to_string(),
            _ => "the run ended".to_string(),
        };
        run.manager.broadcast(ToWorker::Stop(Stop { reason }));
        // A user stops once the transaction it is in ends: each worker has
        // the request timeout and FINAL_TIMEOUT from Stop to say its users
        // stopped, and FINAL_TIMEOUT from then for its final metrics.
        let users_bound = self.configuration.request_timeout() + FINAL_TIMEOUT;
        let stop = Instant::now();
        // The merge refuses a delta past its clock, which the main loop, held
        // here, no longer moves: a final may carry seconds up to the end of
        // this wait.
        if self.series_started.is_some() {
            let last = self.series_elapsed_secs() as u64 + (users_bound + FINAL_TIMEOUT).as_secs();
            let _ = goose_attack_run_state
                .metrics_cmd_tx
                .send(MetricsCommand::GaggleClock {
                    second: last + CLOCK_SLACK,
                });
        }
        let deadline = |worker: &WorkerEntry| match worker.users_stopped {
            Some(at) => at.max(stop) + FINAL_TIMEOUT,
            None => stop + users_bound,
        };
        while let Some(next) = run.running().map(deadline).min() {
            // Ctrl-C: every worker still owed its final metrics is lost,
            // and test_stop, the reports and Shutdown run as after any run.
            if let Some(why) = crate::killswitch_reason() {
                let owed: Vec<u32> = run.running().map(|worker| worker.id).collect();
                for id in owed {
                    let why =
                        format!("canceled while the manager waited for its final metrics: {why}");
                    self.lose_worker(goose_attack_run_state, run, id, why);
                }
                // The logs are given up too.
                run.canceled_at_stop = false;
                break;
            }
            let wake = next.min(Instant::now() + KILLSWITCH_POLL);
            if let Some(event) = run.manager.next_event(wake).await {
                self.manager_event(goose_attack_run_state, run, event);
                continue;
            }
            let now = Instant::now();
            if now < next {
                continue;
            }
            let late: Vec<(u32, bool)> = run
                .running()
                .filter(|worker| deadline(worker) <= now)
                .map(|worker| (worker.id, worker.users_stopped.is_some()))
                .collect();
            for (id, users_stopped) in late {
                let why = if users_stopped {
                    format!(
                        "no final metrics within {} seconds of its users stopping",
                        FINAL_TIMEOUT.as_secs()
                    )
                } else {
                    format!(
                        "its users had not stopped within {} seconds of Stop (the request timeout and {} seconds)",
                        users_bound.as_secs(),
                        FINAL_TIMEOUT.as_secs()
                    )
                };
                self.lose_worker(goose_attack_run_state, run, id, why);
            }
        }
        put_back_cancel(aside);
    }

    /// A worker whose users stopped: say so, send its final metrics, then
    /// `Stopped`.
    /// One whose metrics processor does not hand over the final metrics
    /// within [`WORKER_FINAL_TIMEOUT`], inside the manager's wait for them,
    /// fails.
    async fn worker_stopped(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        run: &mut WorkerRun,
    ) -> Result<(), GooseError> {
        // A unit test's fault: users that never stop, as far as the manager
        // hears.
        #[cfg(test)]
        if let Some(error) = self.gaggle.test_faults.withhold_final(&run.link).await {
            return Err(error);
        }
        // Its final metrics follow at once: the manager waits for them from
        // now.
        run.link.users_stopped();
        let now = self.series_elapsed_secs();
        let (respond, done) = tokio::sync::oneshot::channel();
        let command = MetricsCommand::GaggleFinal {
            now,
            outbox: run.link.outbox.clone(),
            respond,
        };
        let no_final = || {
            format!(
                "the metrics processor did not hand over the final metrics within {} seconds",
                WORKER_FINAL_TIMEOUT.as_secs()
            )
        };
        // A unit test's fault: a processor that never answers. The command
        // is held unsent, its reply channel open, while the wait runs out.
        #[cfg(test)]
        if self.gaggle.test_faults.silent_final {
            let _unsent = command;
            let _ = answered(done, WORKER_FINAL_TIMEOUT).await;
            return Err(worker_failed(&run.link, no_final()).await);
        }
        // A unit test's fault: a processor that takes the command late.
        #[cfg(test)]
        let command = self
            .gaggle
            .test_faults
            .late_final(&goose_attack_run_state.metrics_cmd_tx, command);
        #[cfg(not(test))]
        let command = Some(command);
        let sent = command
            .is_none_or(|command| goose_attack_run_state.metrics_cmd_tx.send(command).is_ok());
        if sent && !answered(done, WORKER_FINAL_TIMEOUT).await {
            return Err(worker_failed(&run.link, no_final()).await);
        }
        let (reason, canceled) = match &self.ended_by {
            Some((EndedBy::UsersExited, _)) => (Reason::UsersExited, String::new()),
            Some((EndedBy::Canceled, reason)) => {
                (Reason::Canceled, reason.clone().unwrap_or_default())
            }
            _ => (Reason::Stopped, String::new()),
        };
        if let Some(folder) = self.output.folder.as_ref() {
            run.log_files = run.spec.log_files(folder);
            run.run_folder = Some(folder.path.clone());
        }
        if let Some(late) = run.started_late {
            info!(
                "gaggle: this worker started {} us after the manager's start instant",
                late.as_micros()
            );
        }
        run.link.stopped(Stopped {
            reason: reason as i32,
            canceled,
            run_folder: run
                .run_folder
                .as_ref()
                .map(|folder| folder.display().to_string())
                .unwrap_or_default(),
        });
        Ok(())
    }

    /// A worker about to exit: give what it queued a moment to leave.
    pub(crate) async fn gaggle_worker_exit(&self) {
        if let Some(link) = self.gaggle_link() {
            link.flush().await;
        }
    }

    /// The manager, once its workers stopped and its reports are written:
    /// ask every worker for its logs and keep them in `workers/<name>/` in the
    /// run's folder. A worker that sends no full chunk, nor a log's last,
    /// for [`LOG_IDLE_TIMEOUT`] gives them up, as Ctrl-C gives up every one: a
    /// log not whole then stays on its worker, and the run record says so.
    /// A log past the size its worker announced is refused the same way.
    /// Whether any log was left on a worker, which the reports then name.
    pub(crate) async fn gaggle_collect_logs(&mut self) -> bool {
        let Some(mut role) = self.gaggle.role.take() else {
            return false;
        };
        let mut left = false;
        if let GaggleRole::Manager(run) = &mut role {
            let folder = self
                .output
                .folder
                .as_ref()
                .map(|folder| folder.path.clone());
            collect_logs(run, folder).await;
            left = !run.logs_left.is_empty();
        }
        self.gaggle.role = Some(role);
        left
    }

    /// The manager, after its reports: tell every worker to shut down.
    pub(crate) async fn gaggle_shut_down(&mut self) {
        if let Some(GaggleRole::Manager(run)) = self.gaggle.role.as_mut() {
            run.manager.shut_down_all().await;
        }
    }

    /// The Gaggle section of the manager's run record.
    pub(crate) fn gaggle_run_record(&self) -> Option<GaggleRecord> {
        let Some(GaggleRole::Manager(run)) = self.gaggle.role.as_ref() else {
            return None;
        };
        let health = run
            .health
            .lock()
            .map(|health| health.clone())
            .unwrap_or_default();
        let workers: Vec<GaggleWorkerRecord> = run
            .workers
            .iter()
            .map(|worker| {
                let merge = health.get(&worker.id).cloned().unwrap_or_default();
                let state = match (&worker.lost, worker.stopped) {
                    (Some(_), _) => GaggleWorkerState::Lost,
                    (None, Some(Reason::UsersExited)) => GaggleWorkerState::UsersExited,
                    (None, Some(_)) => GaggleWorkerState::Finished,
                    (None, None) => GaggleWorkerState::Running,
                };
                // A worker that stopped after sending metrics sent its final
                // delta first: without it, its last running totals stand.
                let final_missing = matches!(
                    state,
                    GaggleWorkerState::Finished | GaggleWorkerState::UsersExited
                ) && merge.last_seq > 0
                    && !merge.final_merged;
                GaggleWorkerRecord {
                    id: worker.id,
                    name: worker.name.clone(),
                    users: worker.users,
                    state,
                    lost_at: worker
                        .lost
                        .as_ref()
                        .map(|(at, _)| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
                    lost_reason: worker.lost.as_ref().map(|(_, why)| why.clone()),
                    last_seq: worker.lost.as_ref().map(|_| merge.last_seq),
                    refused_deltas: merge.refused,
                    refused_reason: merge.refusal,
                    excluded_reason: merge.excluded,
                    missing_deltas: merge.missing,
                    final_missing,
                }
            })
            .collect();
        let degraded = workers.iter().any(GaggleWorkerRecord::degraded);
        Some(GaggleRecord {
            workers,
            logs_left: run.logs_left.clone(),
            degraded,
        })
    }
}

/// The sequence number of the last delta from worker `id` the merge took
/// so far.
fn last_merged(health: &MergeHealth, id: u32) -> u64 {
    health
        .lock()
        .ok()
        .and_then(|health| health.get(&id).map(|merge| merge.last_seq))
        .unwrap_or(0)
}

/// One log arriving from a worker, written to `part` until its last chunk,
/// when it is renamed to `path`: a log cut off part way never shows under
/// its real name.
struct Incoming {
    file: tokio::fs::File,
    /// The size its worker announced, which it may not pass.
    size: u64,
    written: u64,
    part: PathBuf,
    path: PathBuf,
}

impl Incoming {
    /// Give the log up: what arrived of it is deleted.
    async fn abandon(self) {
        drop(self.file);
        let _ = tokio::fs::remove_file(&self.part).await;
    }
}

/// Where worker `id` of `workers` keeps `log`, which stays on it.
fn log_left(workers: &[WorkerEntry], id: u32, log: &LogSpec) -> GaggleLogLeft {
    let worker = super::worker(workers, id);
    GaggleLogLeft {
        worker: worker.map_or_else(|| id.to_string(), |worker| worker.name.clone()),
        path: match worker.and_then(|worker| worker.run_folder.as_ref()) {
            Some(folder) => format!("{folder}/{}", log.worker_file()),
            None => log.worker_file(),
        },
    }
}

/// Give worker `id`'s `log` up: what arrived of it is deleted, and it is
/// recorded as left on its worker. The record, to warn with.
async fn give_up(
    run: &mut ManagerRun,
    open: &mut BTreeMap<(u32, String), Incoming>,
    id: u32,
    log: &LogSpec,
) -> GaggleLogLeft {
    if let Some(incoming) = open.remove(&(id, log.manager_file.clone())) {
        incoming.abandon().await;
    }
    let entry = log_left(&run.workers, id, log);
    run.logs_left.push(entry.clone());
    entry
}

/// Collect every worker's logs into `folder`, as [`GooseAttack::gaggle_collect_logs`]
/// says. A Ctrl-C from here stops the wait for logs, as does one since
/// `Stop`. One that canceled the run is set aside meanwhile, so the next is
/// caught rather than ending the process at once, and put back after: the
/// killswitch then reads as after any canceled run, logs or not.
async fn collect_logs(run: &mut ManagerRun, folder: Option<PathBuf>) {
    if run.logs.is_empty() {
        return;
    }
    let aside = if run.canceled_at_stop {
        set_cancel_aside()
    } else {
        None
    };
    collect_logs_into(run, folder).await;
    put_back_cancel(aside);
}

async fn collect_logs_into(run: &mut ManagerRun, folder: Option<PathBuf>) {
    // Each worker's logs still to come, by the name the manager keeps them
    // under.
    let mut pending: BTreeMap<u32, BTreeMap<String, LogSpec>> = BTreeMap::new();
    for worker in &run.workers {
        // A worker that never started wrote no logs: none is left on it.
        let never_ran = worker.lost.is_none() && worker.run_folder.is_none();
        if !worker.sends_logs() || folder.is_none() {
            if !never_ran {
                for log in &run.logs {
                    run.logs_left.push(log_left(&run.workers, worker.id, log));
                }
            }
            continue;
        }
        pending.insert(
            worker.id,
            run.logs
                .iter()
                .map(|log| (log.manager_file.clone(), log.clone()))
                .collect(),
        );
        run.manager.send(worker.id, ToWorker::SendLogs(SendLogs {}));
    }
    let Some(folder) = folder else {
        warn!("gaggle: this run has no folder, so its workers' logs stay on them");
        return;
    };
    // When each worker last made progress, a full chunk or a log's last,
    // from when it was asked.
    let asked = Instant::now();
    let mut heard: BTreeMap<u32, Instant> = pending.keys().map(|&id| (id, asked)).collect();
    let mut open: BTreeMap<(u32, String), Incoming> = BTreeMap::new();
    // The bytes every log so far was announced as, across workers.
    let mut announced = 0u64;
    while pending.values().any(|files| !files.is_empty()) {
        if crate::is_killswitch_triggered() {
            warn!("gaggle: canceled while the workers sent their logs; every log not yet whole stays on its worker");
            for (id, files) in std::mem::take(&mut pending) {
                for log in files.into_values() {
                    give_up(run, &mut open, id, &log).await;
                }
            }
            return;
        }
        let deadline = pending
            .iter()
            .filter(|(_, files)| !files.is_empty())
            .filter_map(|(id, _)| heard.get(id))
            .min()
            .map_or(asked, |&last| last + LOG_IDLE_TIMEOUT);
        // Every worker that made no progress for the idle timeout: its logs
        // still to come stay on it. Checked before each wait, so events that
        // make no progress, however often, never hold the check off.
        let now = Instant::now();
        if now >= deadline {
            for (&id, files) in pending.iter_mut() {
                if files.is_empty()
                    || heard
                        .get(&id)
                        .is_some_and(|&last| now < last + LOG_IDLE_TIMEOUT)
                {
                    continue;
                }
                for log in std::mem::take(files).into_values() {
                    let entry = give_up(run, &mut open, id, &log).await;
                    warn!(
                        "gaggle: worker {}'s log {} made no progress, a full chunk or its last, for {} seconds; it stays on the worker",
                        entry.worker,
                        entry.path,
                        LOG_IDLE_TIMEOUT.as_secs()
                    );
                }
            }
            continue;
        }
        let wake = deadline.min(now + KILLSWITCH_POLL);
        let Some(event) = run.manager.next_event(wake).await else {
            continue;
        };
        match event {
            Event::Log { id, chunk } => {
                let Some(files) = pending.get_mut(&id) else {
                    continue;
                };
                if !files.contains_key(&chunk.file) {
                    // The rest of a log given up is dropped: it was warned
                    // of once, as it was given up.
                    if !run.logs.iter().any(|log| log.manager_file == chunk.file) {
                        warn!(
                            "gaggle: worker {id} sent a log the manager did not ask for: {:?}",
                            chunk.file
                        );
                        #[cfg(test)]
                        UNASKED.lock().unwrap().push(chunk.file.clone());
                    }
                    continue;
                }
                let key = (id, chunk.file.clone());
                if let Some(why) = chunk.error.as_deref() {
                    warn!(
                        "gaggle: worker {id} could not send its {}: {why}",
                        chunk.file
                    );
                    if let Some(log) = files.remove(&chunk.file) {
                        give_up(run, &mut open, id, &log).await;
                    }
                    continue;
                }
                if !open.contains_key(&key) {
                    let Some(name) =
                        super::worker(&run.workers, id).map(|worker| worker.name.clone())
                    else {
                        continue;
                    };
                    let dir = folder.join("workers").join(&name);
                    let path = dir.join(&chunk.file);
                    let part = dir.join(format!("{}.part", chunk.file));
                    let created = async {
                        tokio::fs::create_dir_all(&dir).await?;
                        tokio::fs::File::create(&part).await
                    }
                    .await;
                    match created {
                        Ok(file) => {
                            let size = chunk.size.unwrap_or(0);
                            announced = announced.saturating_add(size);
                            info!(
                                "gaggle: worker {name}'s {} is {size} bytes; {announced} bytes of logs announced so far",
                                chunk.file
                            );
                            open.insert(
                                key.clone(),
                                Incoming {
                                    file,
                                    size,
                                    written: 0,
                                    part,
                                    path,
                                },
                            );
                        }
                        Err(e) => {
                            warn!(
                                "gaggle: could not write worker {name}'s {}: {e}",
                                chunk.file
                            );
                            if let Some(log) = files.remove(&chunk.file) {
                                give_up(run, &mut open, id, &log).await;
                            }
                            continue;
                        }
                    }
                }
                let Some(incoming) = open.get_mut(&key) else {
                    continue;
                };
                let past = incoming.written.saturating_add(chunk.data.len() as u64);
                if past > incoming.size {
                    warn!(
                        "gaggle: worker {id}'s {} passed the {} bytes it was announced as; it stays on the worker",
                        chunk.file, incoming.size
                    );
                    if let Some(log) = files.remove(&chunk.file) {
                        give_up(run, &mut open, id, &log).await;
                    }
                    continue;
                }
                use tokio::io::AsyncWriteExt;
                let written = chunk.offset == incoming.written
                    && incoming.file.write_all(&chunk.data).await.is_ok();
                #[cfg(test)]
                let written =
                    written && !FAIL_LOG_WRITES.load(std::sync::atomic::Ordering::Relaxed);
                #[cfg(test)]
                if !written && DISK_FULL_AFTER_LOGS.load(std::sync::atomic::Ordering::Relaxed) {
                    DISK_FULL.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                let mut ok = written;
                if ok {
                    incoming.written += chunk.data.len() as u64;
                    // Only progress keeps a transfer alive: a worker sends
                    // full chunks and then the last, so a shorter chunk that
                    // is not the last, as a trickle of small ones, does not.
                    if chunk.data.len() >= LOG_CHUNK || chunk.last {
                        heard.insert(id, Instant::now());
                    }
                }
                // A log is whole only at the size its worker announced.
                if ok && chunk.last {
                    ok = incoming.written == incoming.size
                        && incoming.file.flush().await.is_ok()
                        && tokio::fs::rename(&incoming.part, &incoming.path)
                            .await
                            .is_ok();
                    if ok {
                        open.remove(&key);
                        files.remove(&chunk.file);
                        continue;
                    }
                }
                if !ok {
                    warn!(
                        "gaggle: worker {id}'s {} arrived out of order or could not be written",
                        chunk.file
                    );
                    if let Some(log) = files.remove(&chunk.file) {
                        give_up(run, &mut open, id, &log).await;
                    }
                }
            }
            Event::Lost { id, why } => {
                warn!("gaggle: worker {id} lost while sending its logs: {why}");
                if let Some(files) = pending.get_mut(&id) {
                    for log in std::mem::take(files).into_values() {
                        give_up(run, &mut open, id, &log).await;
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gaggle::delta::{DeltaError, DeltaKind, GaggleDelta, ScenarioPart};
    use crate::gaggle::manager::handshake_tests::test_manager;

    /// A manager's merge takes the run's users and workers as its limits:
    /// a user ordinal equal to the run's user count, or a worker id equal to
    /// the workers it expects, is refused.
    #[test]
    fn gaggle_delta_limits_are_the_runs() {
        let configuration = crate::config::GooseConfiguration::parse_args_default(&[
            "--manager",
            "--expect-workers",
            "3",
            "--users",
            "6",
            "--host",
            "http://h",
        ])
        .unwrap();
        let mut attack = GooseAttack::initialize_with_config(configuration).unwrap();
        attack.test_plan = crate::test_plan::TestPlan::build(&attack.configuration).unwrap();
        let (limits, _) = attack.gaggle_merge_settings();
        assert_eq!((limits.max_users, limits.max_workers), (6, 3));
        let with_user = |worker: u32, user: u64| {
            let mut delta = GaggleDelta::new(worker, 1, 0, DeltaKind::Delta);
            let mut scenario = ScenarioPart::default();
            scenario.users.insert(user);
            delta.metrics.scenarios.insert(0, scenario);
            delta
        };
        assert_eq!(with_user(0, 5).validate(&limits), Ok(()));
        assert_eq!(
            with_user(0, 6).validate(&limits),
            Err(DeltaError::UserOutOfRange { user: 6, limit: 6 })
        );
        assert_eq!(
            with_user(3, 0).validate(&limits),
            Err(DeltaError::WorkerOutOfRange {
                worker: 3,
                limit: 3
            })
        );
    }

    /// Only an answer is an answer: a reply channel the metrics processor
    /// dropped fails the phase at once, as a timeout does later.
    #[tokio::test]
    async fn a_dropped_reply_is_no_answer() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        tx.send(()).unwrap();
        assert!(answered(rx, Duration::from_secs(1)).await);
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        drop(tx);
        let asked = Instant::now();
        assert!(!answered(rx, Duration::from_secs(10)).await);
        assert!(asked.elapsed() < Duration::from_secs(5));
        let (_tx, rx) = tokio::sync::oneshot::channel::<()>();
        assert!(!answered(rx, Duration::from_millis(100)).await);
    }

    fn entry() -> WorkerEntry {
        WorkerEntry {
            id: 0,
            name: "w".to_string(),
            users: 1,
            users_stopped: None,
            stopped: None,
            lost: None,
            gone: false,
            run_folder: Some("/runs/w".to_string()),
        }
    }

    /// A manager run of one finished worker, connected through a client of
    /// the test's own, collecting `logs`: the run, the client's sender, and
    /// its stream from the manager, past the Welcome.
    async fn collecting(
        logs: Vec<LogSpec>,
    ) -> (
        ManagerRun,
        tokio::sync::mpsc::Sender<crate::gaggle::proto::WorkerMessage>,
        tonic::Streaming<crate::gaggle::proto::ManagerMessage>,
    ) {
        use crate::gaggle::manager::handshake_tests::{channel, hello, open};
        let manager = test_manager(1, Duration::from_secs(10)).await;
        let (tx, inbound) = open(channel(&manager).await, hello(), None).await;
        let mut inbound = inbound.unwrap();
        let welcome = inbound.message().await.unwrap().unwrap();
        assert!(matches!(welcome.message, Some(ToWorker::Welcome(_))));
        let run = ManagerRun {
            manager,
            workers: vec![WorkerEntry {
                stopped: Some(Reason::Stopped),
                ..entry()
            }],
            logs,
            logs_left: Vec::new(),
            clock_second: None,
            stop_sent: true,
            canceled_at_stop: false,
            ending: false,
            health: MergeHealth::default(),
        };
        (run, tx, inbound)
    }

    /// The request log, as `--request-log` without a path asks for it.
    fn request_log() -> LogSpec {
        LogSpec {
            kind: 0,
            format: None,
            manager_file: "requests.json".to_string(),
            gzip: false,
        }
    }

    /// Wait on `inbound` for `SendLogs`, skipping heartbeats.
    async fn send_logs_asked(inbound: &mut tonic::Streaming<crate::gaggle::proto::ManagerMessage>) {
        loop {
            match inbound.message().await.unwrap().and_then(|m| m.message) {
                Some(ToWorker::SendLogs(_)) => return,
                Some(ToWorker::Heartbeat(_)) => continue,
                other => panic!("{:?}", other),
            }
        }
    }

    /// A worker is found by its own id, never by a position alone.
    #[test]
    fn workers_are_found_by_their_id() {
        let workers = vec![entry(), WorkerEntry { id: 5, ..entry() }];
        assert_eq!(
            crate::gaggle::worker(&workers, 0).map(|worker| worker.id),
            Some(0)
        );
        assert!(crate::gaggle::worker(&workers, 1).is_none());
        assert!(crate::gaggle::worker(&workers, 5).is_none());
    }

    /// Chunks of a log already given up are dropped without a warning each:
    /// its one warning came as it was given up.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_log_given_up_warns_once() {
        let test = async {
            let folder = std::env::temp_dir()
                .join(format!("goose-gaggle-given-up-log-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&folder);
            let log = |kind: u8, file: &str| LogSpec {
                kind,
                format: None,
                manager_file: file.to_string(),
                gzip: false,
            };
            let (mut run, tx, mut inbound) =
                collecting(vec![log(0, "given-up.json"), log(1, "kept.json")]).await;
            let worker = tokio::spawn(async move {
                send_logs_asked(&mut inbound).await;
                let mut failed = chunk("given-up.json", 0, b"", true);
                if let Some(crate::gaggle::proto::worker_message::Message::LogChunk(chunk)) =
                    failed.message.as_mut()
                {
                    chunk.error = Some("permission denied".to_string());
                }
                tx.send(failed).await.unwrap();
                for offset in 0..3 {
                    tx.send(chunk("given-up.json", offset, b"x", false))
                        .await
                        .unwrap();
                }
                tx.send(chunk("kept.json", 0, b"kept", true)).await.unwrap();
                (tx, inbound)
            });
            collect_logs(&mut run, Some(folder.clone())).await;
            let _ = worker.await.unwrap();
            let unasked = UNASKED
                .lock()
                .unwrap()
                .iter()
                .filter(|file| *file == "given-up.json")
                .count();
            assert_eq!(unasked, 0);
            assert_eq!(run.logs_left.len(), 1, "{:?}", run.logs_left);
            let _ = std::fs::remove_dir_all(&folder);
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// A log the worker says it could not read is recorded as left on it,
    /// and the manager creates no file for it.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_log_the_worker_could_not_read_is_left_on_it() {
        let test = async {
            let folder = std::env::temp_dir()
                .join(format!("goose-gaggle-unread-log-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&folder);
            let (mut run, tx, mut inbound) = collecting(vec![request_log()]).await;
            let worker = tokio::spawn(async move {
                send_logs_asked(&mut inbound).await;
                let mut failed = chunk("requests.json", 0, b"", true);
                if let Some(crate::gaggle::proto::worker_message::Message::LogChunk(chunk)) =
                    failed.message.as_mut()
                {
                    chunk.error = Some("permission denied".to_string());
                }
                tx.send(failed).await.unwrap();
                (tx, inbound)
            });
            collect_logs(&mut run, Some(folder.clone())).await;
            drop(worker.await.unwrap());
            assert_eq!(files_under(&folder), Vec::<String>::new());
            assert_eq!(run.logs_left.len(), 1, "{:?}", run.logs_left);
            let _ = std::fs::remove_dir_all(&folder);
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// Log collection's failure paths: a file the manager did not ask for
    /// is never written; a chunk out of order abandons its log; a log that
    /// cannot be created, and every log of a run with no folder, is left on
    /// its worker. None leaves a file under a log's name.
    #[tokio::test]
    #[serial_test::serial]
    async fn log_collection_failures_leave_logs_on_their_worker() {
        let test = async {
            let folder = std::env::temp_dir()
                .join(format!("goose-gaggle-log-failures-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&folder);
            // Not asked for, then the log asked for, whole.
            let (mut run, tx, mut inbound) = collecting(vec![request_log()]).await;
            let worker = tokio::spawn(async move {
                send_logs_asked(&mut inbound).await;
                tx.send(chunk("other.json", 0, b"unasked", true))
                    .await
                    .unwrap();
                tx.send(chunk("requests.json", 0, b"asked", true))
                    .await
                    .unwrap();
                (tx, inbound)
            });
            collect_logs(&mut run, Some(folder.clone())).await;
            drop(worker.await.unwrap());
            assert_eq!(files_under(&folder), ["workers/w/requests.json"]);
            assert!(run.logs_left.is_empty(), "{:?}", run.logs_left);
            let _ = std::fs::remove_dir_all(&folder);

            // Out of order.
            let (mut run, tx, mut inbound) = collecting(vec![request_log()]).await;
            let worker = tokio::spawn(async move {
                send_logs_asked(&mut inbound).await;
                tx.send(chunk("requests.json", 0, b"first", false))
                    .await
                    .unwrap();
                tx.send(chunk("requests.json", 99, b"later", true))
                    .await
                    .unwrap();
                (tx, inbound)
            });
            collect_logs(&mut run, Some(folder.clone())).await;
            drop(worker.await.unwrap());
            assert_eq!(files_under(&folder), Vec::<String>::new());
            assert_eq!(run.logs_left.len(), 1, "{:?}", run.logs_left);
            let _ = std::fs::remove_dir_all(&folder);

            // The run folder is a file: no log can be created in it.
            std::fs::write(&folder, b"not a folder").unwrap();
            let (mut run, tx, mut inbound) = collecting(vec![request_log()]).await;
            let worker = tokio::spawn(async move {
                send_logs_asked(&mut inbound).await;
                tx.send(chunk("requests.json", 0, b"log", true))
                    .await
                    .unwrap();
                (tx, inbound)
            });
            collect_logs(&mut run, Some(folder.clone())).await;
            drop(worker.await.unwrap());
            assert_eq!(run.logs_left.len(), 1, "{:?}", run.logs_left);
            assert_eq!(std::fs::read(&folder).unwrap(), b"not a folder");
            let _ = std::fs::remove_file(&folder);

            // No run folder: nothing is asked for, every log stays.
            let (mut run, _tx, _inbound) = collecting(vec![request_log()]).await;
            collect_logs(&mut run, None).await;
            assert_eq!(
                run.logs_left,
                [GaggleLogLeft {
                    worker: "w".to_string(),
                    path: "/runs/w/requests.json".to_string(),
                }]
            );
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// A worker, idle, its link joined to `manager`, holding its share of
    /// one user and a start instant a minute away; and its run state. It
    /// calls `initialize_attack`, which resets the process wide killswitch,
    /// so every test that uses it is serial.
    async fn idle_worker(manager: &Manager) -> (GooseAttack, GooseAttackRunState) {
        let link = Link::connect(LinkSettings {
            address: manager.local_addr.to_string(),
            connect_timeout: Duration::from_secs(10),
            heartbeat_timeout: Duration::from_secs(30),
            hello: LinkSettings::hello(7, "idle".to_string()),
            token: None,
        })
        .await
        .unwrap();
        let configuration = crate::config::GooseConfiguration::parse_args_default(&[
            "--host",
            "http://127.0.0.1:9",
            "--users",
            "1",
            "--no-telnet",
            "--quiet",
        ])
        .unwrap();
        let spec = spec::worker_specs(
            &configuration,
            Default::default(),
            &[(1, 1_000), (0, 0)],
            &[0],
            1,
            true,
        )
        .remove(0);
        let mut attack = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(crate::goose::Scenario::new("A").register_transaction(
                crate::goose::Transaction::new(std::sync::Arc::new(|_| Box::pin(async { Ok(()) }))),
            ));
        attack.test_plan = crate::test_plan::TestPlan::build(&attack.configuration).unwrap();
        let run_state = attack.initialize_attack().await.unwrap();
        attack.attack_mode = AttackMode::Worker;
        attack.gaggle.role = Some(GaggleRole::Worker(Box::new(WorkerRun {
            link,
            spec,
            start_at: Some(tokio::time::Instant::now() + Duration::from_secs(60)),
            started_late: None,
            tick_second: None,
            pending_reset: None,
            log_files: Vec::new(),
            run_folder: None,
            skip_reset: false,
            stop_reason_sent: false,
        })));
        (attack, run_state)
    }

    /// A `Stop` that comes before the worker's start instant, as when the
    /// manager's run ends in the milliseconds before it: the worker never
    /// starts, and says it stopped.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[serial_test::serial]
    async fn a_stop_before_the_start_never_starts() {
        let test = async {
            let manager = test_manager(1, Duration::from_secs(30)).await;
            let (mut attack, mut run_state) = idle_worker(&manager).await;
            manager.send(
                0,
                ToWorker::Stop(Stop {
                    reason: "the run ended".to_string(),
                }),
            );
            let deadline = Instant::now() + Duration::from_secs(10);
            let stopped = loop {
                attack.gaggle_tick(&mut run_state).await.unwrap();
                match manager.try_event() {
                    Some(Event::Stopped { stopped, .. }) => break stopped,
                    Some(_) => {}
                    None => {
                        assert!(Instant::now() < deadline, "no Stopped");
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                }
            };
            assert_eq!(stopped.reason(), Reason::Stopped);
            match attack.gaggle.role.as_ref() {
                Some(GaggleRole::Worker(run)) => assert!(run.start_at.is_none()),
                _ => panic!("not a worker"),
            }
            assert_eq!(attack.attack_phase, AttackPhase::Idle);
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// A metrics reset that comes before the worker's start instant, as
    /// when the ramp ends in the milliseconds before it, is held, and made
    /// once the worker starts: its metrics are then on the run's epoch.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[serial_test::serial]
    async fn a_reset_before_the_start_is_made_after_it() {
        let test = async {
            let manager = test_manager(1, Duration::from_secs(30)).await;
            let (mut attack, mut run_state) = idle_worker(&manager).await;
            let metrics_epoch = run_state.metrics_epoch.clone();
            let epoch = || metrics_epoch.load(std::sync::atomic::Ordering::Relaxed);
            let before = epoch();
            manager.send(0, ToWorker::ResetMetrics(ResetMetrics { epoch: 1 }));
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                attack.gaggle_tick(&mut run_state).await.unwrap();
                match attack.gaggle.role.as_mut() {
                    Some(GaggleRole::Worker(run)) if run.pending_reset == Some(1) => {
                        // Now due.
                        run.start_at = Some(tokio::time::Instant::now());
                        break;
                    }
                    _ => {}
                }
                assert!(Instant::now() < deadline, "the reset was not held");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            // Held, not made, before the start.
            assert_eq!(epoch(), before);
            attack.gaggle_worker_idle(&mut run_state).await.unwrap();
            // The start begins an epoch, as on any run, and the held reset
            // the next.
            assert_eq!(epoch(), before + 2, "the held reset was made at the start");
            match attack.gaggle.role.as_ref() {
                Some(GaggleRole::Worker(run)) => assert!(run.pending_reset.is_none()),
                _ => panic!("not a worker"),
            }
            assert_eq!(attack.attack_phase, AttackPhase::Increase);
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// A worker that sends chunks that move nothing, of a file never asked
    /// for or empty and not the last, is not kept alive by them, however
    /// often they come: its log is left on it once the idle deadline passes.
    #[tokio::test]
    #[serial_test::serial]
    async fn chunks_without_progress_do_not_keep_a_transfer_alive() {
        let test = async {
            let folder =
                std::env::temp_dir().join(format!("goose-gaggle-idle-log-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&folder);
            let (mut run, tx, mut inbound) = collecting(vec![request_log()]).await;
            let worker = tokio::spawn(async move {
                send_logs_asked(&mut inbound).await;
                // Back to back for 10 seconds: the manager's queue is never
                // empty, so its wait for the next event always has one.
                let until = Instant::now() + Duration::from_secs(10);
                while Instant::now() < until {
                    let unasked = chunk("other.json", 0, b"", false);
                    let empty = chunk("requests.json", 0, b"", false);
                    if tx.send(unasked).await.is_err() || tx.send(empty).await.is_err() {
                        break;
                    }
                }
                (tx, inbound)
            });
            let started = Instant::now();
            collect_logs(&mut run, Some(folder.clone())).await;
            assert!(
                started.elapsed() < LOG_IDLE_TIMEOUT * 3,
                "{:?}",
                started.elapsed()
            );
            assert_eq!(run.logs_left.len(), 1, "{:?}", run.logs_left);
            assert_eq!(files_under(&folder), Vec::<String>::new());
            worker.abort();
            let _ = std::fs::remove_dir_all(&folder);
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// A log chunk, as a worker sends it.
    fn chunk(
        file: &str,
        offset: u64,
        data: &[u8],
        last: bool,
    ) -> crate::gaggle::proto::WorkerMessage {
        crate::gaggle::proto::WorkerMessage {
            message: Some(crate::gaggle::proto::worker_message::Message::LogChunk(
                crate::gaggle::proto::LogChunk {
                    file: file.to_string(),
                    offset,
                    data: data.to_vec(),
                    last,
                    error: None,
                    // A log in one chunk is its size; one in more is
                    // announced larger than any a test sends.
                    size: Some(if last && offset == 0 {
                        data.len() as u64
                    } else {
                        1 << 30
                    }),
                },
            )),
        }
    }

    /// Every file under `folder`, relative to it.
    fn files_under(folder: &std::path::Path) -> Vec<String> {
        let mut found = Vec::new();
        let mut dirs = vec![folder.to_path_buf()];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else {
                    found.push(path.strip_prefix(folder).unwrap().display().to_string());
                }
            }
        }
        found.sort();
        found
    }

    /// A worker lost part way through sending a log leaves no file under
    /// the log's name, nor any part of it, and the log is recorded as left
    /// on the worker; a log that arrives whole is renamed into place.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_log_cut_off_part_way_leaves_no_file() {
        let test = async {
            let folder =
                std::env::temp_dir().join(format!("goose-gaggle-cut-log-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&folder);
            let (mut run, tx, mut inbound) = collecting(vec![request_log()]).await;
            let worker = tokio::spawn(async move {
                send_logs_asked(&mut inbound).await;
                tx.send(chunk("requests.json", 0, b"half", false))
                    .await
                    .unwrap();
                // The worker goes before the rest of the log.
                drop(tx);
                drop(inbound);
            });
            collect_logs(&mut run, Some(folder.clone())).await;
            worker.await.unwrap();
            assert_eq!(files_under(&folder), Vec::<String>::new());
            assert_eq!(
                run.logs_left,
                [GaggleLogLeft {
                    worker: "w".to_string(),
                    path: "/runs/w/requests.json".to_string(),
                }]
            );

            let (mut run, tx, mut inbound) = collecting(vec![request_log()]).await;
            let worker = tokio::spawn(async move {
                send_logs_asked(&mut inbound).await;
                let mut first = chunk("requests.json", 0, b"whole ", false);
                if let Some(crate::gaggle::proto::worker_message::Message::LogChunk(chunk)) =
                    first.message.as_mut()
                {
                    chunk.size = Some(9);
                }
                tx.send(first).await.unwrap();
                tx.send(chunk("requests.json", 6, b"log", true))
                    .await
                    .unwrap();
                (tx, inbound)
            });
            collect_logs(&mut run, Some(folder.clone())).await;
            drop(worker.await.unwrap());
            assert_eq!(files_under(&folder), ["workers/w/requests.json"]);
            assert_eq!(
                std::fs::read(folder.join("workers/w/requests.json")).unwrap(),
                b"whole log"
            );
            assert!(run.logs_left.is_empty(), "{:?}", run.logs_left);
            let _ = std::fs::remove_dir_all(&folder);
            // Its last chunk short of the size announced: not whole.
            let (mut run, tx, mut inbound) = collecting(vec![request_log()]).await;
            let worker = tokio::spawn(async move {
                send_logs_asked(&mut inbound).await;
                let mut first = chunk("requests.json", 0, b"whole ", false);
                if let Some(crate::gaggle::proto::worker_message::Message::LogChunk(chunk)) =
                    first.message.as_mut()
                {
                    chunk.size = Some(9);
                }
                tx.send(first).await.unwrap();
                tx.send(chunk("requests.json", 6, b"lo", true))
                    .await
                    .unwrap();
                (tx, inbound)
            });
            collect_logs(&mut run, Some(folder.clone())).await;
            drop(worker.await.unwrap());
            assert_eq!(files_under(&folder), Vec::<String>::new());
            assert_eq!(run.logs_left.len(), 1, "{:?}", run.logs_left);
            let _ = std::fs::remove_dir_all(&folder);
        };
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("within 30 seconds");
    }

    /// A worker that finished after sending metrics, but whose final delta
    /// the merge never took, degrades the run: its last running totals
    /// stand in for its final. One whose final was merged does not.
    #[tokio::test]
    async fn gaggle_finished_worker_without_its_final_degrades_the_run() {
        async fn record(final_merged: bool) -> GaggleRecord {
            let manager = test_manager(1, Duration::from_secs(10)).await;
            let health = MergeHealth::default();
            health.lock().unwrap().insert(
                0,
                crate::gaggle::merge::WorkerMerge {
                    last_seq: 4,
                    final_merged,
                    ..Default::default()
                },
            );
            let configuration =
                crate::config::GooseConfiguration::parse_args_default(&["--host", "http://h"])
                    .unwrap();
            let mut attack = GooseAttack::initialize_with_config(configuration).unwrap();
            attack.gaggle.role = Some(GaggleRole::Manager(Box::new(ManagerRun {
                manager,
                workers: vec![WorkerEntry {
                    stopped: Some(Reason::Stopped),
                    ..entry()
                }],
                logs: Vec::new(),
                logs_left: Vec::new(),
                clock_second: None,
                stop_sent: true,
                canceled_at_stop: false,
                ending: false,
                health,
            })));
            attack.gaggle_run_record().unwrap()
        }
        let missing = record(false).await;
        assert!(missing.degraded, "{:?}", missing);
        assert!(missing.workers[0].final_missing);
        assert!(missing
            .intro()
            .iter()
            .any(|line| line.contains("final metrics never arrived")));
        let merged = record(true).await;
        assert!(!merged.degraded, "{:?}", merged);
    }

    /// A worker that goes away before it stopped is lost; one that stopped
    /// as told and then went away is gone, not lost, and either way sends no
    /// logs; nor does a worker that never ran, with no run folder.
    #[test]
    fn gaggle_lost_gone_and_logs() {
        let mut running = entry();
        assert!(running.sends_logs());
        assert!(running.lose(false, "silent".to_string()));
        assert!(!running.lose(false, "again".to_string()));
        assert!(running.lost.is_some() && !running.sends_logs());

        let mut stopped = entry();
        stopped.stopped = Some(Reason::Stopped);
        assert!(stopped.sends_logs());
        assert!(!stopped.lose(true, "closed".to_string()));
        assert!(stopped.lost.is_none() && stopped.gone);
        assert!(
            !stopped.sends_logs(),
            "the manager stops waiting for its logs"
        );

        // Stopped before Stop was sent: lost.
        let mut early = entry();
        early.stopped = Some(Reason::Canceled);
        assert!(early.lose(false, "canceled".to_string()));

        let never = WorkerEntry {
            run_folder: None,
            stopped: Some(Reason::Stopped),
            ..entry()
        };
        assert!(
            !never.sends_logs(),
            "a worker that never started has no logs"
        );
    }
}
