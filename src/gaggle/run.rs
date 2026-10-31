//! A Gaggle manager's and a worker's part in a [`GooseAttack`] run.
//!
//! The manager is a `GooseAttack` with no users of its own: its phase
//! machine, run time and test plan clock are the run's clock, and its
//! metrics processor merges what every worker sends. A worker is a
//! `GooseAttack` that waits idle, as with `--no-autostart`, until its manager
//! says when to start the share it was handed, and is then driven by what the
//! manager sends.

use super::link::{Link, LinkEvent, LinkSettings};
use super::manager::{Event, Manager};
use super::proto::{
    manager_message::Message as ToWorker, stopped::Reason, Prepare, ResetMetrics, SendLogs,
    StartAt, Stop, Stopped,
};
use super::spec::{self, LogSpec, WorkerRunSpec};
use crate::metrics::MetricsCommand;
use crate::runs::{
    log_file_name, EndedBy, GaggleLogLeft, GaggleRecord, GaggleWorkerRecord, LOG_NAMES,
};
use crate::{AttackMode, AttackPhase, GooseAttack, GooseAttackRunState, GooseError, WorkerShare};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long after `StartAt` every process starts.
const START_DELAY: Duration = Duration::from_secs(2);

/// How long the manager waits for every worker's answer to `Prepare`.
const PREPARE_TIMEOUT: Duration = Duration::from_secs(120);

/// How long the manager waits for every worker's final metrics after `Stop`.
const FINAL_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the manager waits for every worker's logs.
const LOGS_TIMEOUT: Duration = Duration::from_secs(600);

/// How far past its own series clock the manager takes a worker's second:
/// the workers' clocks start within milliseconds of its own, and a delta is
/// stamped up to a second late.
const CLOCK_SLACK: u64 = 10;

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
    last_seq: u64,
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
    /// The run already ended for every worker lost or every worker's users
    /// exited.
    ending: bool,
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

impl ManagerRun {
    fn worker(&mut self, id: u32) -> Option<&mut WorkerEntry> {
        self.workers.get_mut(id as usize)
    }

    /// Workers neither lost nor stopped.
    fn running(&self) -> impl Iterator<Item = &WorkerEntry> {
        self.workers
            .iter()
            .filter(|worker| worker.lost.is_none() && worker.stopped.is_none())
    }
}

/// The failure that ends a manager before its run: every worker is told to
/// shut down first.
async fn fail(manager: &mut Manager, detail: String) -> GooseError {
    error!("gaggle: {detail}");
    manager.shut_down_all().await;
    GooseError::Gaggle { detail }
}

impl GooseAttack {
    /// Whether this process is a Gaggle manager.
    pub(crate) fn gaggle_manager(&self) -> bool {
        self.attack_mode == AttackMode::Manager
    }

    /// Whether this process is a Gaggle worker.
    pub(crate) fn gaggle_worker(&self) -> bool {
        self.attack_mode == AttackMode::Worker
    }

    /// The link to a worker's manager.
    pub(crate) fn gaggle_link(&self) -> Option<&Link> {
        match self.gaggle.as_ref() {
            Some(GaggleRole::Worker(run)) => Some(&run.link),
            _ => None,
        }
    }

    /// Have the manager report the address it listens on (unit tests).
    #[cfg(test)]
    pub(crate) fn report_manager_address(
        mut self,
        bound: tokio::sync::oneshot::Sender<std::net::SocketAddr>,
    ) -> Self {
        self.gaggle_bound = Some(bound);
        self
    }

    /// The manager, before its run: listen, wait for every worker, run
    /// `test_start` once, hand each worker its share and custom data, wait
    /// for each to prepare, then tell all of them to start in
    /// [`START_DELAY`] and wait for that instant, when its own run starts.
    pub(crate) async fn gaggle_manager_start(&mut self) -> Result<(), GooseError> {
        self.attack_mode = AttackMode::Manager;
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
        if let Some(bound) = self.gaggle_bound.take() {
            let _ = bound.send(manager.local_addr);
        }

        let deadline = Instant::now() + connect_timeout;
        let mut joined = 0;
        while joined < expected {
            match manager.next_event(deadline).await {
                Some(Event::Joined) => joined += 1,
                Some(Event::Lost { id, why }) => {
                    return Err(fail(
                        &mut manager,
                        format!("worker {id} was lost before the run started: {why}"),
                    )
                    .await)
                }
                Some(_) => {}
                None => {
                    return Err(fail(
                        &mut manager,
                        format!(
                            "{joined} of {expected} workers joined within {} seconds (--gaggle-connect-timeout)",
                            connect_timeout.as_secs()
                        ),
                    )
                    .await)
                }
            }
        }
        info!("gaggle: all {expected} workers joined");

        // test_start runs once, here, before any worker starts.
        if let Err(e) = self.run_test_start().await {
            manager.shut_down_all().await;
            return Err(e);
        }

        let total = self.test_plan.total_users();
        let order = match self.user_scenarios(total) {
            Ok(order) => order,
            Err(e) => {
                manager.shut_down_all().await;
                return Err(e);
            }
        };
        let mut specs = spec::worker_specs(
            &self.configuration,
            &self.test_plan.steps,
            &order,
            expected,
            self.configuration.records_graph_data(),
        );
        for spec in specs.iter_mut() {
            let share = WorkerShare {
                index: spec.worker as usize,
                workers: expected,
                users: spec.users,
                user_offset: spec.user_offset,
            };
            match self.call_worker_data_hook(&share) {
                Ok(data) => spec.custom_data = data.unwrap_or_default(),
                Err(e) => {
                    manager.shut_down_all().await;
                    return Err(e);
                }
            }
        }
        let names = manager.workers();
        let mut workers = Vec::with_capacity(expected);
        for (spec, (id, name)) in specs.iter().zip(&names) {
            let bytes = match spec.encode() {
                Ok(bytes) => bytes,
                Err(e) => {
                    return Err(
                        fail(&mut manager, format!("could not encode a run spec: {e}")).await,
                    )
                }
            };
            manager.send(*id, ToWorker::Prepare(Prepare { spec: bytes }));
            workers.push(WorkerEntry {
                id: *id,
                name: name.clone(),
                users: spec.users,
                last_seq: 0,
                stopped: None,
                lost: None,
                gone: false,
                run_folder: None,
            });
        }

        let deadline = Instant::now() + PREPARE_TIMEOUT;
        let mut ready = BTreeSet::new();
        while ready.len() < expected {
            match manager.next_event(deadline).await {
                Some(Event::PrepareAck { id, error: None }) => {
                    ready.insert(id);
                }
                Some(Event::PrepareAck {
                    id,
                    error: Some(error),
                }) => {
                    return Err(fail(
                        &mut manager,
                        format!("worker {id} could not prepare its share: {error}"),
                    )
                    .await)
                }
                Some(Event::Lost { id, why }) => {
                    return Err(fail(
                        &mut manager,
                        format!("worker {id} was lost while preparing: {why}"),
                    )
                    .await)
                }
                Some(_) => {}
                None => {
                    return Err(fail(
                        &mut manager,
                        format!(
                            "{} of {expected} workers prepared within {} seconds",
                            ready.len(),
                            PREPARE_TIMEOUT.as_secs()
                        ),
                    )
                    .await)
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
        self.gaggle = Some(GaggleRole::Manager(Box::new(ManagerRun {
            manager,
            workers,
            logs: spec::log_specs(&self.configuration),
            logs_left: Vec::new(),
            clock_second: None,
            stop_sent: false,
            ending: false,
        })));
        tokio::time::sleep_until(start).await;
        info!(
            "gaggle: starting at {}",
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
        );
        Ok(())
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
            self.configuration.gaggle_token(),
            super::link::worker_name(),
        );
        let link = Link::connect(LinkSettings {
            address: self.configuration.manager_address(),
            connect_timeout: self.configuration.gaggle_connect_timeout(),
            heartbeat_timeout: self.configuration.gaggle_heartbeat_timeout(),
            hello,
        })
        .await?;
        // The manager waits for its other workers and runs test_start: the
        // heartbeats, and the dead man's switch, cover the wait.
        let bytes = loop {
            match link.next_event(Duration::from_secs(1)).await {
                Some(LinkEvent::Prepare(bytes)) => break bytes,
                Some(LinkEvent::Lost(why)) => {
                    return Err(GooseError::Gaggle {
                        detail: format!("lost the manager before the run started: {why}"),
                    })
                }
                Some(LinkEvent::Shutdown) => {
                    return Err(GooseError::Gaggle {
                        detail: "the manager shut down before the run started".to_string(),
                    })
                }
                _ => {}
            }
        };
        let mut spec = match WorkerRunSpec::decode(&bytes) {
            Ok(spec) => spec,
            Err(e) => {
                let detail = format!("could not read the run spec: {e}");
                link.prepared(Some(detail.clone()));
                link.flush().await;
                return Err(GooseError::Gaggle { detail });
            }
        };
        info!(
            "gaggle: worker {} of {}: {} users from ordinal {}",
            spec.worker, spec.workers, spec.users, spec.user_offset
        );
        spec.configure(&mut self.configuration);
        self.set_user_slice(spec.user_offset, spec.slice.clone());
        self.worker_data = Some(Arc::from(std::mem::take(&mut spec.custom_data)));
        self.gaggle = Some(GaggleRole::Worker(Box::new(WorkerRun {
            link,
            spec,
            start_at: None,
            started_late: None,
            pending_reset: None,
            tick_second: None,
            log_files: Vec::new(),
            run_folder: None,
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
        let start_at = match self.gaggle.as_mut() {
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
        if let Some(GaggleRole::Worker(run)) = self.gaggle.as_mut() {
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
        // A worker holding no users, with --iterations, has nothing to
        // run: its users have all exited, as the manager waits to hear.
        let pending = match self.gaggle.as_mut() {
            Some(GaggleRole::Worker(run)) => run.pending_reset.take(),
            _ => None,
        };
        if let Some(epoch) = pending {
            debug!("gaggle: metrics reset that came before the start, epoch {epoch}");
            self.gaggle_worker_reset(goose_attack_run_state).await;
        }
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
        let Some(mut role) = self.gaggle.take() else {
            return Ok(());
        };
        let result = match &mut role {
            GaggleRole::Manager(run) => self.manager_tick(goose_attack_run_state, run).await,
            GaggleRole::Worker(run) => self.worker_tick(goose_attack_run_state, run).await,
        };
        self.gaggle = Some(role);
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
            Event::Delta { id, delta } => {
                let Some(worker) = run.worker(id) else {
                    return;
                };
                // The results hold what a lost worker sent before it was lost.
                if worker.lost.is_some() {
                    return;
                }
                worker.last_seq = delta.seq;
                let _ = goose_attack_run_state
                    .metrics_cmd_tx
                    .send(MetricsCommand::GaggleApply { worker: id, delta });
            }
            Event::Stopped { id, stopped } => {
                let stop_sent = run.stop_sent;
                let reason = stopped.reason();
                let Some(worker) = run.worker(id) else {
                    return;
                };
                worker.stopped = Some(reason);
                worker.run_folder = Some(stopped.run_folder).filter(|folder| !folder.is_empty());
                if reason == Reason::Canceled && !stop_sent && worker.lost.is_none() {
                    let why = format!("it was stopped on the worker: {}", stopped.canceled);
                    self.lose_worker(goose_attack_run_state, run, id, why);
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
        let Some(worker) = run.worker(id) else {
            return;
        };
        let message = format!(
            "gaggle: worker {id} ({}) lost: {why}; last delta {}",
            worker.name, worker.last_seq
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
                    self.gaggle_worker_reset(goose_attack_run_state).await;
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
                    if running && !goose_attack_run_state.stopping {
                        info!("gaggle: the manager stopped the run: {reason}");
                        self.set_ended_by(EndedBy::Stopped, None);
                        self.cancel_attack(goose_attack_run_state).await?;
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
                    return Err(GooseError::Gaggle {
                        detail: format!("lost the manager: {why}"),
                    })
                }
            }
        }
        Ok(())
    }

    /// The manager's metrics reset: a worker resets its own metrics, the
    /// users still running counted again.
    async fn gaggle_worker_reset(&mut self, goose_attack_run_state: &mut GooseAttackRunState) {
        self.update_duration();
        let (ack, acked) = tokio::sync::oneshot::channel();
        let sent = goose_attack_run_state
            .metrics_cmd_tx
            .send(MetricsCommand::Reset {
                duration: self.metrics.duration,
                total_users: self.metrics.total_users,
                maximum_users: self.metrics.maximum_users,
                active_users: goose_attack_run_state.active_users,
                series_elapsed_secs: self.series_elapsed_secs(),
                running_users: goose_attack_run_state.running_users.clone(),
                ack,
            });
        if sent.is_ok() {
            let _ = acked.await;
        }
        goose_attack_run_state.running_metrics_timer = std::time::Instant::now();
        self.started = Some(std::time::Instant::now());
    }

    /// The manager's metrics reset, at the end of its ramp: every worker
    /// resets too.
    pub(crate) fn gaggle_manager_reset(&self, epoch: u64) {
        if let Some(GaggleRole::Manager(run)) = self.gaggle.as_ref() {
            info!("gaggle: resetting every worker's metrics");
            run.manager
                .broadcast(ToWorker::ResetMetrics(ResetMetrics { epoch }));
        }
    }

    /// The end of a run, once every local user stopped and before
    /// `test_stop`: a manager stops every worker and waits for its final
    /// metrics; a worker sends its final metrics, then `Stopped`.
    pub(crate) async fn gaggle_end_of_run(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) {
        let Some(mut role) = self.gaggle.take() else {
            return;
        };
        match &mut role {
            GaggleRole::Manager(run) => self.manager_stop(goose_attack_run_state, run).await,
            GaggleRole::Worker(run) => self.worker_stopped(goose_attack_run_state, run).await,
        }
        self.gaggle = Some(role);
    }

    async fn manager_stop(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        run: &mut ManagerRun,
    ) {
        run.stop_sent = true;
        let reason = match &self.ended_by {
            Some((EndedBy::Canceled, Some(reason))) => format!("canceled: {reason}"),
            Some((EndedBy::WorkersLost, _)) => "every other worker was lost".to_string(),
            Some((EndedBy::UsersExited, _)) => "every user exited".to_string(),
            Some((EndedBy::Stopped, _)) => "stopped".to_string(),
            _ => "the run ended".to_string(),
        };
        run.manager.broadcast(ToWorker::Stop(Stop { reason }));
        let deadline = Instant::now() + FINAL_TIMEOUT;
        while run.running().next().is_some() {
            match run.manager.next_event(deadline).await {
                Some(event) => self.manager_event(goose_attack_run_state, run, event),
                None => break,
            }
        }
        let late: Vec<u32> = run.running().map(|worker| worker.id).collect();
        for id in late {
            self.lose_worker(
                goose_attack_run_state,
                run,
                id,
                format!(
                    "no final metrics within {} seconds of Stop",
                    FINAL_TIMEOUT.as_secs()
                ),
            );
        }
    }

    async fn worker_stopped(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        run: &mut WorkerRun,
    ) {
        let now = self.series_elapsed_secs();
        let (respond, done) = tokio::sync::oneshot::channel();
        if goose_attack_run_state
            .metrics_cmd_tx
            .send(MetricsCommand::GaggleFinal {
                now,
                outbox: run.link.outbox.clone(),
                respond,
            })
            .is_ok()
        {
            let _ = done.await;
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
    }

    /// A worker about to exit: give what it queued a moment to leave.
    pub(crate) async fn gaggle_worker_exit(&self) {
        if let Some(link) = self.gaggle_link() {
            link.flush().await;
        }
    }

    /// The manager, once its workers stopped and before the reports: ask
    /// every worker for its logs and keep them in `workers/<name>/` in the
    /// run's folder. One deadline covers them all; a log not whole by then
    /// stays on its worker, and the run record says so.
    pub(crate) async fn gaggle_collect_logs(&mut self) {
        let Some(mut role) = self.gaggle.take() else {
            return;
        };
        if let GaggleRole::Manager(run) = &mut role {
            let folder = self
                .output
                .folder
                .as_ref()
                .map(|folder| folder.path.clone());
            let compress = self.configuration.compress.is_some();
            collect_logs(run, folder, compress).await;
        }
        self.gaggle = Some(role);
    }

    /// The manager, after its reports: tell every worker to shut down.
    pub(crate) async fn gaggle_shut_down(&mut self) {
        if let Some(GaggleRole::Manager(run)) = self.gaggle.as_mut() {
            run.manager.shut_down_all().await;
        }
    }

    /// The Gaggle section of the manager's run record.
    pub(crate) fn gaggle_run_record(&self) -> Option<GaggleRecord> {
        let Some(GaggleRole::Manager(run)) = self.gaggle.as_ref() else {
            return None;
        };
        let workers: Vec<GaggleWorkerRecord> = run
            .workers
            .iter()
            .map(|worker| GaggleWorkerRecord {
                id: worker.id,
                name: worker.name.clone(),
                users: worker.users,
                state: match (&worker.lost, worker.stopped) {
                    (Some(_), _) => "lost",
                    (None, Some(Reason::UsersExited)) => "users_exited",
                    (None, Some(_)) => "finished",
                    (None, None) => "running",
                }
                .to_string(),
                lost_at: worker
                    .lost
                    .as_ref()
                    .map(|(at, _)| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
                lost_reason: worker.lost.as_ref().map(|(_, why)| why.clone()),
                last_seq: worker.lost.as_ref().map(|_| worker.last_seq),
            })
            .collect();
        let degraded = workers.iter().any(|worker| worker.state == "lost");
        Some(GaggleRecord {
            workers,
            logs_left: run.logs_left.clone(),
            degraded,
        })
    }
}

/// The name a worker gives log `log` in its own run folder.
fn worker_file(log: &LogSpec, compress: bool) -> String {
    let mut file = log_file_name(
        LOG_NAMES[log.kind as usize % LOG_NAMES.len()],
        log.format.as_ref(),
    );
    if compress {
        file.push_str(".gz");
    }
    file
}

/// One log arriving from a worker.
struct Incoming {
    file: tokio::fs::File,
    written: u64,
}

async fn collect_logs(run: &mut ManagerRun, folder: Option<PathBuf>, compress: bool) {
    if run.logs.is_empty() {
        return;
    }
    let left = |worker: &WorkerEntry, log: &LogSpec| GaggleLogLeft {
        worker: worker.name.clone(),
        path: match &worker.run_folder {
            Some(folder) => format!("{folder}/{}", worker_file(log, compress)),
            None => worker_file(log, compress),
        },
    };
    // Each worker's logs still to come, by the name the manager keeps them
    // under.
    let mut pending: BTreeMap<u32, BTreeMap<String, LogSpec>> = BTreeMap::new();
    for worker in &run.workers {
        // A worker that never started wrote no logs: none is left on it.
        let never_ran = worker.lost.is_none() && worker.run_folder.is_none();
        if !worker.sends_logs() || folder.is_none() {
            if !never_ran {
                for log in &run.logs {
                    run.logs_left.push(left(worker, log));
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
    let deadline = Instant::now() + LOGS_TIMEOUT;
    let mut open: BTreeMap<(u32, String), Incoming> = BTreeMap::new();
    while pending.values().any(|files| !files.is_empty()) {
        let Some(event) = run.manager.next_event(deadline).await else {
            break;
        };
        match event {
            Event::Log { id, chunk } => {
                let Some(files) = pending.get_mut(&id) else {
                    continue;
                };
                if !files.contains_key(&chunk.file) {
                    warn!(
                        "gaggle: worker {id} sent a log the manager did not ask for: {:?}",
                        chunk.file
                    );
                    continue;
                }
                let key = (id, chunk.file.clone());
                if !open.contains_key(&key) {
                    let name = &run.workers[id as usize].name;
                    let dir = folder.join("workers").join(name);
                    let created = async {
                        tokio::fs::create_dir_all(&dir).await?;
                        tokio::fs::File::create(dir.join(&chunk.file)).await
                    }
                    .await;
                    match created {
                        Ok(file) => {
                            open.insert(key.clone(), Incoming { file, written: 0 });
                        }
                        Err(e) => {
                            warn!(
                                "gaggle: could not write worker {name}'s {}: {e}",
                                chunk.file
                            );
                            if let Some(log) = files.remove(&chunk.file) {
                                run.logs_left.push(left(&run.workers[id as usize], &log));
                            }
                            continue;
                        }
                    }
                }
                let Some(incoming) = open.get_mut(&key) else {
                    continue;
                };
                use tokio::io::AsyncWriteExt;
                let ok = chunk.offset == incoming.written
                    && incoming.file.write_all(&chunk.data).await.is_ok();
                if !ok {
                    warn!(
                        "gaggle: worker {id}'s {} arrived out of order or could not be written",
                        chunk.file
                    );
                    open.remove(&key);
                    if let Some(log) = files.remove(&chunk.file) {
                        run.logs_left.push(left(&run.workers[id as usize], &log));
                    }
                    continue;
                }
                incoming.written += chunk.data.len() as u64;
                if chunk.last {
                    let _ = incoming.file.flush().await;
                    open.remove(&key);
                    files.remove(&chunk.file);
                }
            }
            Event::Lost { id, why } => {
                warn!("gaggle: worker {id} lost while sending its logs: {why}");
                if let Some(files) = pending.get_mut(&id) {
                    for (_, log) in std::mem::take(files) {
                        run.logs_left.push(left(&run.workers[id as usize], &log));
                    }
                }
            }
            _ => {}
        }
    }
    for (id, files) in pending {
        for (_, log) in files {
            let entry = left(&run.workers[id as usize], &log);
            warn!(
                "gaggle: worker {}'s log {} was not complete within {} seconds; it stays on the worker",
                entry.worker,
                entry.path,
                LOGS_TIMEOUT.as_secs()
            );
            run.logs_left.push(entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> WorkerEntry {
        WorkerEntry {
            id: 0,
            name: "w".to_string(),
            users: 1,
            last_seq: 0,
            stopped: None,
            lost: None,
            gone: false,
            run_folder: Some("/runs/w".to_string()),
        }
    }

    /// A worker that goes away before it stopped is lost; one that stopped
    /// as told and then went away is gone, not lost, and either way sends no
    /// logs; nor does a worker that never ran, with no run folder.
    #[test]
    fn gaggle_lost_gone_and_logs() {
        crate::gaggle::within(30, || {
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
        });
    }
}
