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
use crate::runs::{
    log_file_name, EndedBy, GaggleLogLeft, GaggleRecord, GaggleWorkerRecord, WorkerState, LOG_NAMES,
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

/// How long a worker waits for its metrics processor to reset.
const RESET_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the manager waits for the next chunk of a worker's logs: a
/// transfer that goes this long with none fails. No deadline caps a
/// transfer that keeps moving, whatever its size.
const LOG_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// How far past its own series clock the manager takes a worker's second:
/// the workers' clocks start within milliseconds of its own, and a delta is
/// stamped up to a second late.
pub(crate) const CLOCK_SLACK: u64 = 10;

/// For Goose's own tests of a run across processes: a worker with this
/// environment variable set ignores its manager's metrics reset, so its
/// metrics stay on an older epoch than the run's, which the manager then
/// leaves out of the results and reports.
const TEST_SKIP_RESET_ENV: &str = "GOOSE_GAGGLE_TEST_SKIP_RESET";

/// What a worker does wrong on purpose, in unit tests.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TestFaults {
    /// Ignore the manager's metrics reset.
    pub(crate) skip_reset: bool,
    /// Send a first delta the manager refuses.
    pub(crate) refused_delta: bool,
    /// Send only deltas the manager refuses from this sequence number on,
    /// 0 for none.
    pub(crate) refuse_from: u64,
    /// The metrics processor never sends the final metrics.
    pub(crate) silent_final: bool,
    /// The metrics processor never answers a reset.
    pub(crate) silent_reset: bool,
    /// The dead man's switch's last resort ends the process for real.
    pub(crate) exit_for_real: bool,
}

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
    /// Ignore the manager's metrics reset ([`TEST_SKIP_RESET_ENV`]).
    skip_reset: bool,
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
    GooseError::Gaggle { detail: why }
}

/// The failure that ends a manager before its run: every worker is told to
/// shut down first.
async fn fail(manager: &mut Manager, detail: String) -> GooseError {
    error!("gaggle: {detail}");
    manager.shut_down_all().await;
    GooseError::Gaggle { detail }
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
            goose_attack_run_state.active_users += 1;
            self.metrics.total_users += 1;
            if goose_attack_run_state.active_users > self.metrics.maximum_users {
                self.metrics.maximum_users = goose_attack_run_state.active_users;
            }
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
            max_workers: workers.min(u32::MAX as usize) as u32,
            ..DeltaLimits::default()
        };
        (limits, health)
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
        if let Some(bound) = self.gaggle.bound.take() {
            let _ = bound.send(manager.local_addr);
        }

        let deadline = Instant::now() + connect_timeout;
        let mut joined = 0;
        while joined < expected {
            match manager.next_event(deadline).await {
                Some(Event::Joined) => {
                    joined += 1;
                    #[cfg(test)]
                    if let Some(joins) = self.gaggle.joins.as_ref() {
                        let _ = joins.send(joined);
                    }
                }
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
        // and dropped before the next worker's is made, so the manager never
        // holds every worker's data, nor two copies of one.
        for (mut spec, (id, name)) in specs.into_iter().zip(names) {
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
            let users = spec.users;
            let encoded = spec.encode();
            drop(spec);
            let bytes = match encoded {
                Ok(bytes) => bytes,
                Err(e) => {
                    return Err(
                        fail(&mut manager, format!("could not encode a run spec: {e}")).await,
                    )
                }
            };
            manager.send(id, ToWorker::Prepare(Prepare { spec: bytes }));
            workers.push(WorkerEntry {
                id,
                name,
                users,
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
        self.gaggle.role = Some(GaggleRole::Manager(Box::new(ManagerRun {
            manager,
            workers,
            logs: spec::log_specs(&self.configuration),
            logs_left: Vec::new(),
            clock_second: None,
            stop_sent: false,
            ending: false,
            health: MergeHealth::default(),
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
            super::link::worker_name(),
        );
        let link = Link::connect(LinkSettings {
            address: self.configuration.manager_address(),
            connect_timeout: self.configuration.gaggle_connect_timeout(),
            heartbeat_timeout: self.configuration.gaggle_heartbeat_timeout(),
            hello,
            token: self.configuration.gaggle_token().map(str::to_string),
            #[cfg(test)]
            exit_for_real: self.gaggle.test_faults.exit_for_real,
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
        let decoded = WorkerRunSpec::decode(&bytes)
            .map_err(|e| format!("could not read the run spec: {e}"))
            .and_then(|spec| spec.validate(self.scenarios.len()).map(|()| spec));
        let mut spec = match decoded {
            Ok(spec) => spec,
            Err(detail) => {
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
        self.gaggle.client_command_line = spec.client_command_line;
        self.set_user_slice(spec.user_offset, spec.slice.clone());
        self.worker_data = Some(Arc::from(std::mem::take(&mut spec.custom_data)));
        let skip_reset = std::env::var_os(TEST_SKIP_RESET_ENV).is_some();
        #[cfg(test)]
        let skip_reset = skip_reset || self.gaggle.test_faults.skip_reset;
        #[cfg(test)]
        if self.gaggle.test_faults.refused_delta {
            link.outbox
                .corrupt_next
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        #[cfg(test)]
        link.outbox.corrupt_from.store(
            self.gaggle.test_faults.refuse_from,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.gaggle.role = Some(GaggleRole::Worker(Box::new(WorkerRun {
            link,
            spec,
            start_at: None,
            started_late: None,
            pending_reset: None,
            tick_second: None,
            log_files: Vec::new(),
            run_folder: None,
            skip_reset,
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
        // A worker holding no users, with --iterations, has nothing to
        // run: its users have all exited, as the manager waits to hear.
        let pending = match self.gaggle.role.as_mut() {
            Some(GaggleRole::Worker(run)) => run.pending_reset.take(),
            _ => None,
        };
        if let Some(epoch) = pending {
            debug!("gaggle: metrics reset that came before the start, epoch {epoch}");
            if let Err(why) = self.gaggle_worker_reset(goose_attack_run_state).await {
                return Err(match self.gaggle_link() {
                    Some(link) => worker_failed(link, why).await,
                    None => GooseError::Gaggle { detail: why },
                });
            }
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
            Event::Delta { id, delta } => {
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
                    .send(MetricsCommand::GaggleApply { worker: id, delta });
            }
            Event::Stopped { id, stopped } => {
                let stop_sent = run.stop_sent;
                let reason = stopped.reason();
                // No final metrics came, nor will: lost, whenever it says so.
                if reason == Reason::Failed {
                    let why = format!("it could not finish its run: {}", stopped.canceled);
                    self.lose_worker(goose_attack_run_state, run, id, why);
                    return;
                }
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
                LinkEvent::ResetMetrics(epoch) if run.skip_reset => {
                    warn!("gaggle: metrics reset to epoch {epoch} ignored ({TEST_SKIP_RESET_ENV})");
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
                LinkEvent::Refused(why) => {
                    error!("gaggle: {why}");
                    return Err(GooseError::Gaggle { detail: why });
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
        let command = MetricsCommand::Reset {
            duration: self.metrics.duration,
            total_users: self.metrics.total_users,
            maximum_users: self.metrics.maximum_users,
            active_users: goose_attack_run_state.active_users,
            series_elapsed_secs: self.series_elapsed_secs(),
            running_users: goose_attack_run_state.running_users.clone(),
            ack,
        };
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
    /// `test_stop`: a manager stops every worker and waits for its final
    /// metrics; a worker sends its final metrics, then `Stopped`.
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

    /// A worker whose users stopped: send its final metrics, then `Stopped`.
    /// One whose metrics processor does not hand over the final metrics
    /// within [`FINAL_TIMEOUT`], as long as the manager waits for them,
    /// fails.
    async fn worker_stopped(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        run: &mut WorkerRun,
    ) -> Result<(), GooseError> {
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
                FINAL_TIMEOUT.as_secs()
            )
        };
        // A unit test's fault: a processor that never answers. The command
        // is held unsent, its reply channel open, while the wait runs out.
        #[cfg(test)]
        if self.gaggle.test_faults.silent_final {
            let _unsent = command;
            let _ = answered(done, FINAL_TIMEOUT).await;
            return Err(worker_failed(&run.link, no_final()).await);
        }
        let sent = goose_attack_run_state.metrics_cmd_tx.send(command).is_ok();
        if sent && !answered(done, FINAL_TIMEOUT).await {
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

    /// The manager, once its workers stopped and before the reports: ask
    /// every worker for its logs and keep them in `workers/<name>/` in the
    /// run's folder. A worker that sends no chunk for [`LOG_IDLE_TIMEOUT`]
    /// gives up its transfers; a log not whole then
    /// stays on its worker, and the run record says so.
    pub(crate) async fn gaggle_collect_logs(&mut self) {
        let Some(mut role) = self.gaggle.role.take() else {
            return;
        };
        if let GaggleRole::Manager(run) = &mut role {
            let folder = self
                .output
                .folder
                .as_ref()
                .map(|folder| folder.path.clone());
            collect_logs(run, folder).await;
        }
        self.gaggle.role = Some(role);
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
                    (Some(_), _) => WorkerState::Lost,
                    (None, Some(Reason::UsersExited)) => WorkerState::UsersExited,
                    (None, Some(_)) => WorkerState::Finished,
                    (None, None) => WorkerState::Running,
                };
                // A worker that stopped after sending metrics sent its final
                // delta first: without it, its last running totals stand.
                let final_missing =
                    matches!(state, WorkerState::Finished | WorkerState::UsersExited)
                        && merge.last_seq > 0
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

/// The name a worker gives log `log` in its own run folder.
fn worker_file(log: &LogSpec) -> String {
    let mut file = log_file_name(LOG_NAMES[log.kind as usize], log.format.as_ref());
    if log.gzip {
        file.push_str(".gz");
    }
    file
}

/// One log arriving from a worker, written to `part` until its last chunk,
/// when it is renamed to `path`: a log cut off part way never shows under
/// its real name.
struct Incoming {
    file: tokio::fs::File,
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

async fn collect_logs(run: &mut ManagerRun, folder: Option<PathBuf>) {
    if run.logs.is_empty() {
        return;
    }
    let left = |worker: &WorkerEntry, log: &LogSpec| GaggleLogLeft {
        worker: worker.name.clone(),
        path: match &worker.run_folder {
            Some(folder) => format!("{folder}/{}", worker_file(log)),
            None => worker_file(log),
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
    // When each worker last sent a chunk, from when it was asked.
    let asked = Instant::now();
    let mut heard: BTreeMap<u32, Instant> = pending.keys().map(|&id| (id, asked)).collect();
    let mut open: BTreeMap<(u32, String), Incoming> = BTreeMap::new();
    while pending.values().any(|files| !files.is_empty()) {
        let deadline = pending
            .iter()
            .filter(|(_, files)| !files.is_empty())
            .filter_map(|(id, _)| heard.get(id))
            .min()
            .map_or(asked, |&last| last + LOG_IDLE_TIMEOUT);
        let Some(event) = run.manager.next_event(deadline).await else {
            // Every worker that sent nothing for the idle timeout: its logs
            // still to come stay on it.
            let now = Instant::now();
            if now < deadline {
                // No event, and no worker idle yet: the event queue closed.
                tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                continue;
            }
            for (&id, files) in pending.iter_mut() {
                if files.is_empty()
                    || heard
                        .get(&id)
                        .is_some_and(|&last| now < last + LOG_IDLE_TIMEOUT)
                {
                    continue;
                }
                for (file, log) in std::mem::take(files) {
                    if let Some(incoming) = open.remove(&(id, file)) {
                        incoming.abandon().await;
                    }
                    let entry = left(&run.workers[id as usize], &log);
                    warn!(
                        "gaggle: no part of worker {}'s log {} came for {} seconds; it stays on the worker",
                        entry.worker,
                        entry.path,
                        LOG_IDLE_TIMEOUT.as_secs()
                    );
                    run.logs_left.push(entry);
                }
            }
            continue;
        };
        match event {
            Event::Log { id, chunk } => {
                let Some(files) = pending.get_mut(&id) else {
                    continue;
                };
                heard.insert(id, Instant::now());
                if !files.contains_key(&chunk.file) {
                    warn!(
                        "gaggle: worker {id} sent a log the manager did not ask for: {:?}",
                        chunk.file
                    );
                    continue;
                }
                let key = (id, chunk.file.clone());
                if let Some(why) = chunk.error.as_deref() {
                    warn!(
                        "gaggle: worker {id} could not send its {}: {why}",
                        chunk.file
                    );
                    if let Some(incoming) = open.remove(&key) {
                        incoming.abandon().await;
                    }
                    if let Some(log) = files.remove(&chunk.file) {
                        run.logs_left.push(left(&run.workers[id as usize], &log));
                    }
                    continue;
                }
                if !open.contains_key(&key) {
                    let name = &run.workers[id as usize].name;
                    let dir = folder.join("workers").join(name);
                    let path = dir.join(&chunk.file);
                    let part = dir.join(format!("{}.part", chunk.file));
                    let created = async {
                        tokio::fs::create_dir_all(&dir).await?;
                        tokio::fs::File::create(&part).await
                    }
                    .await;
                    match created {
                        Ok(file) => {
                            open.insert(
                                key.clone(),
                                Incoming {
                                    file,
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
                let mut ok = chunk.offset == incoming.written
                    && incoming.file.write_all(&chunk.data).await.is_ok();
                if ok {
                    incoming.written += chunk.data.len() as u64;
                }
                if ok && chunk.last {
                    ok = incoming.file.flush().await.is_ok()
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
                    if let Some(incoming) = open.remove(&key) {
                        incoming.abandon().await;
                    }
                    if let Some(log) = files.remove(&chunk.file) {
                        run.logs_left.push(left(&run.workers[id as usize], &log));
                    }
                }
            }
            Event::Lost { id, why } => {
                warn!("gaggle: worker {id} lost while sending its logs: {why}");
                if let Some(files) = pending.get_mut(&id) {
                    for (file, log) in std::mem::take(files) {
                        if let Some(incoming) = open.remove(&(id, file)) {
                            incoming.abandon().await;
                        }
                        run.logs_left.push(left(&run.workers[id as usize], &log));
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

    /// A manager's merge takes the run's users and workers as its limits:
    /// a user ordinal equal to the run's user count, or a worker id equal to
    /// the workers it expects, is refused.
    #[test]
    fn gaggle_delta_limits_are_the_runs() {
        crate::gaggle::within(30, || {
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
        });
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
        let manager = Manager::start("127.0.0.1", 0, 1, None, 7, true, Duration::from_secs(10))
            .await
            .unwrap();
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

    /// A log the worker says it could not read is recorded as left on it,
    /// and the manager creates no file for it.
    #[tokio::test]
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
                tx.send(chunk("requests.json", 0, b"whole ", false))
                    .await
                    .unwrap();
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
            let manager = Manager::start("127.0.0.1", 0, 1, None, 7, true, Duration::from_secs(10))
                .await
                .unwrap();
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
