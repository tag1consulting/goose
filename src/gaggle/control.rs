//! Runtime control of a Gaggle: what a manager sends each worker when a
//! Controller or the dashboard changes the run, and the arithmetic that
//! splits a change into each worker's share, so the workers together run what
//! one process running the whole load test would.
//!
//! The manager checks a change against the rules one process applies, applies
//! it to its own run, the run's clock, and sends each worker its share in a
//! `Control`. A worker answers twice, each time with a `ControlAck`: that it
//! received the change, as soon as it reads it, then that it applied its share
//! through the same checks, or refused it. The front end hears once every
//! worker received the change; a worker that does not receive it within
//! [`ACK_TIMEOUT`] is lost, and one that received it stays in the run, however
//! long applying it takes, while its stream and heartbeats are alive.

use super::proto::{manager_message::Message as ToWorker, Control};
use super::run::{GaggleRole, ManagerRun};
use crate::controller::{
    ControlCommand, ControlReply, ControllerCommand, RunSettings, MIN_CONTROL_RATE,
};
use crate::metrics::MetricsCommand;
use crate::{AttackPhase, ControlOutcome, GooseAttack, GooseAttackRunState, GooseError};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

/// One user: its global ordinal, and the index in the load test's scenarios
/// of the scenario it runs.
pub(crate) type User = (usize, usize);

/// What a manager tells a worker to do while the load test runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum WorkerControl {
    /// Run settings, as this worker's share.
    Settings(WorkerSettings),
    /// Stop a running load test, as `Stop` does.
    Stop,
    /// Stop a running load test: the manager says `Shutdown` once the run
    /// ended and its logs are in.
    Shutdown,
}

impl WorkerControl {
    /// What the control changes, for the log: `stop`, `shutdown`, or the
    /// settings it sets, such as `users, throttle`.
    pub(crate) fn describe(&self) -> String {
        match self {
            WorkerControl::Stop => "stop".to_string(),
            WorkerControl::Shutdown => "shutdown".to_string(),
            WorkerControl::Settings(settings) => {
                let fields = [
                    ("host", settings.host.is_some()),
                    ("users", settings.users.is_some()),
                    ("increase_rate", settings.increase_rate.is_some()),
                    ("increase_time", settings.increase_time.is_some()),
                    ("decrease_rate", settings.decrease_rate.is_some()),
                    ("decrease_time", settings.decrease_time.is_some()),
                    ("run_time", settings.run_time.is_some()),
                    ("test_plan", settings.test_plan.is_some()),
                    ("throttle", settings.throttle.is_some()),
                ];
                let set: Vec<&str> = fields
                    .iter()
                    .filter(|(_, set)| *set)
                    .map(|(name, _)| *name)
                    .collect();
                match set.is_empty() {
                    true => "settings".to_string(),
                    false => set.join(", "),
                }
            }
        }
    }

    /// The control as it travels, postcard encoded.
    pub(crate) fn encode(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_stdvec(self)
    }

    /// A control read back from `bytes`.
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }
}

/// A worker's share of [`RunSettings`]: each field `None` to leave the
/// setting alone, `Some(None)` to clear it and `Some(Some(value))` to set it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct WorkerSettings {
    pub(crate) host: Option<Option<String>>,
    pub(crate) users: Option<WorkerUsers>,
    pub(crate) increase_rate: Option<Option<f32>>,
    pub(crate) increase_time: Option<Option<usize>>,
    pub(crate) decrease_rate: Option<Option<f32>>,
    pub(crate) decrease_time: Option<Option<usize>>,
    pub(crate) run_time: Option<Option<usize>>,
    pub(crate) test_plan: Option<Option<WorkerPlan>>,
    pub(crate) throttle: Option<Option<usize>>,
}

/// A worker's users from now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct WorkerUsers {
    /// Every user it runs, in the order it starts them: those it runs now
    /// first, then those still to start.
    pub(crate) users: Vec<User>,
    /// How long the change takes, in milliseconds: the whole load test's
    /// ramp, so every worker's ends with it.
    pub(crate) ramp_ms: usize,
}

/// A test plan as a worker runs it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct WorkerPlan {
    /// Each step's users and milliseconds.
    pub(crate) steps: Vec<(usize, usize)>,
    /// Every user the plan starts, in the order it starts them.
    pub(crate) users: Vec<User>,
    /// Whether the whole plan holds until stopped, its last step running
    /// users.
    pub(crate) holds: bool,
}

impl WorkerSettings {
    /// The settings without the users and the test plan, which a worker
    /// applies with their users, as run settings.
    pub(crate) fn run_settings(&self) -> RunSettings {
        RunSettings {
            host: self.host.clone(),
            users: None,
            increase_rate: self.increase_rate,
            increase_time: self.increase_time,
            decrease_rate: self.decrease_rate,
            decrease_time: self.decrease_time,
            run_time: self.run_time,
            test_plan: None,
            throttle: self.throttle,
        }
    }
}

/// The users a manager's workers run, as one process running them all
/// would: each worker's, in the order it starts them, and the ordinal the
/// next user started takes. A worker runs the first of its users now and
/// starts the rest as its ramp goes on; one process starts its users in
/// ordinal order and stops the last started first, so the users this book
/// holds are the ones one process would run at the end of every ramp.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct UserBook {
    workers: Vec<Vec<User>>,
    next: usize,
}

impl UserBook {
    /// A book of each worker's users, as the run started them, the next
    /// user taking ordinal `next`.
    pub(crate) fn new(workers: Vec<Vec<User>>, next: usize) -> Self {
        UserBook { workers, next }
    }

    /// The users of every worker together.
    pub(crate) fn total(&self) -> usize {
        self.workers.iter().map(Vec::len).sum()
    }

    /// Worker `worker`'s users.
    pub(crate) fn users(&self, worker: usize) -> &[User] {
        self.workers.get(worker).map_or(&[], Vec::as_slice)
    }

    /// The ordinal the next user started takes: every user started so far
    /// has a lower one.
    pub(crate) fn next_ordinal(&self) -> usize {
        self.next
    }

    /// Change to `users` in all over the workers `live` names: more users
    /// take the next ordinals and `scenarios`, which gives the scenario of
    /// each added user in the order they start, dealt one at a time to the
    /// live worker running fewest, the first among equals; fewer stop the
    /// users with the highest ordinals, last started first. Whether each
    /// worker's users changed.
    pub(crate) fn set_users(
        &mut self,
        users: usize,
        live: &[bool],
        scenarios: Vec<usize>,
    ) -> Vec<bool> {
        let mut changed = vec![false; self.workers.len()];
        let total = self.total();
        if users >= total {
            let added = scenarios
                .into_iter()
                .take(users - total)
                .enumerate()
                .map(|(at, scenario)| (self.next + at, scenario))
                .collect::<Vec<_>>();
            self.next += added.len();
            for worker in self.deal(added, live) {
                changed[worker] = true;
            }
        } else {
            // The worker whose last started user has the highest ordinal.
            let mut last: BinaryHeap<(usize, usize)> = self
                .workers
                .iter()
                .enumerate()
                .filter_map(|(worker, users)| users.last().map(|&(ordinal, _)| (ordinal, worker)))
                .collect();
            for _ in users..total {
                let Some((_, worker)) = last.pop() else {
                    break;
                };
                self.workers[worker].pop();
                changed[worker] = true;
                if let Some(&(ordinal, _)) = self.workers[worker].last() {
                    last.push((ordinal, worker));
                }
            }
        }
        changed
    }

    /// Deal `users` one at a time to the live worker running fewest, the
    /// first among equals: the workers that took any.
    pub(crate) fn deal(&mut self, users: Vec<User>, live: &[bool]) -> Vec<usize> {
        let mut fewest: BinaryHeap<Reverse<(usize, usize)>> = (0..self.workers.len())
            .filter(|&worker| live.get(worker).copied().unwrap_or(false))
            .map(|worker| Reverse((self.workers[worker].len(), worker)))
            .collect();
        let mut took = vec![false; self.workers.len()];
        for user in users {
            let Some(Reverse((count, worker))) = fewest.pop() else {
                break;
            };
            self.workers[worker].push(user);
            took[worker] = true;
            fewest.push(Reverse((count + 1, worker)));
        }
        (0..took.len()).filter(|&worker| took[worker]).collect()
    }

    /// Worker `worker` is lost: its users, which it no longer runs.
    pub(crate) fn lose(&mut self, worker: usize) -> Vec<User> {
        self.workers
            .get_mut(worker)
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// A test plan whose users start from ordinal [`Self::next_ordinal`]:
    /// each live worker's users are the next block of `order`, as many as
    /// its share of `steps` starts. The book then holds each worker's peak,
    /// what the throttle follows.
    pub(crate) fn set_plan(
        &mut self,
        plans: &[Vec<(usize, usize)>],
        order: &[usize],
    ) -> Vec<Vec<User>> {
        let mut offset = 0;
        let mut users = Vec::with_capacity(plans.len());
        for (worker, plan) in plans.iter().enumerate() {
            let count = super::spec::users_started(plan);
            let started: Vec<User> = (offset..offset + count)
                .map(|at| (self.next + at, order.get(at).copied().unwrap_or(0)))
                .collect();
            offset += count;
            if let Some(book) = self.workers.get_mut(worker) {
                *book = started[..super::spec::peak_users(plan).min(count)].to_vec();
            }
            users.push(started);
        }
        self.next += offset;
        users
    }
}

/// `throttle` split between workers running `users` each, by their share of
/// the users, the remainder spread: every worker gets at least 1, so the
/// shares add up to `throttle` whenever it is at least the number of
/// workers. A worker with no users keeps 1, as it may get users later; the
/// rest follow [`super::spec::throttle_shares`].
pub(crate) fn live_throttle_shares(throttle: usize, users: &[usize]) -> Vec<usize> {
    // With no users anywhere, the throttle is split evenly.
    if users.iter().all(|&users| users == 0) {
        return (0..users.len())
            .map(|worker| super::spec::split(throttle, users.len(), worker).max(1))
            .collect();
    }
    let idle = users.iter().filter(|&&users| users == 0).count();
    let busy: Vec<usize> = users.iter().copied().filter(|&users| users > 0).collect();
    let mut shares = super::spec::throttle_shares(throttle.saturating_sub(idle), &busy).into_iter();
    users
        .iter()
        .map(|&users| match users {
            0 => 1,
            _ => shares.next().unwrap_or(1).max(1),
        })
        .collect()
}

/// `rate` split between workers running `users` each, by their share of the
/// users: one with none gets the slowest rate, as a rate must be set.
pub(crate) fn rate_shares(rate: f32, users: &[usize]) -> Vec<f32> {
    let total: usize = users.iter().sum();
    users
        .iter()
        .map(|&users| match (users, total) {
            (0, _) | (_, 0) => MIN_CONTROL_RATE,
            (users, total) => (f64::from(rate) * users as f64 / total as f64) as f32,
        })
        .collect()
}

/// The manager's outcome for a change, and each worker's control when the
/// manager applied it.
type Change = (ControlOutcome, Option<Vec<(u32, WorkerControl)>>);

/// How long the manager waits for every worker to say it received a change:
/// 10 seconds, inside the 15 a front end waits.
pub(crate) const ACK_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_secs(5)
} else {
    Duration::from_secs(10)
};
const _: () = assert!(ACK_TIMEOUT.as_millis() < crate::controller::CONTROL_TIMEOUT.as_millis());

/// How often the manager warns that a worker received a change and has yet
/// to apply it: every 30 seconds. Applying a change has no deadline.
const APPLY_WARNING: Duration = Duration::from_secs(30);

/// How long a lost worker's users take to start on the workers left, with
/// `--gaggle-rebalance`.
const REBALANCE_RAMP_MS: usize = 1_000;

/// A change a front end waits on: answered once every worker it was sent to
/// received it, or was lost.
pub(crate) struct PendingControl {
    /// The [`SentControl`] it waits on.
    pub(super) id: u64,
    pub(super) respond: oneshot::Sender<ControlReply>,
    /// The manager's own outcome, which the reply carries.
    pub(super) outcome: ControlOutcome,
    /// Each worker lost before it received the change, and why.
    pub(super) lost: Vec<String>,
}

/// A change sent to the workers, until each has applied it or was lost.
pub(crate) struct SentControl {
    pub(super) id: u64,
    /// What it changes, for the log.
    pub(super) what: String,
    pub(super) sent: Instant,
    /// How many workers it was sent to.
    pub(super) workers: usize,
    /// The workers yet to say they received it, each lost at
    /// [`ACK_TIMEOUT`].
    pub(super) unreceived: BTreeSet<u32>,
    /// The workers yet to apply it.
    pub(super) unapplied: BTreeSet<u32>,
    /// How many applied it.
    pub(super) applied: usize,
    /// When the manager last warned it is still being applied.
    pub(super) warned: Instant,
}

impl SentControl {
    /// A worker's answer: it received the change, or, `received` false, it
    /// applied it, which says it received it too.
    pub(super) fn answered(&mut self, worker: u32, received: bool) {
        self.unreceived.remove(&worker);
        if !received && self.unapplied.remove(&worker) {
            self.applied += 1;
        }
    }

    /// A worker lost: never waited on again. Whether it had yet to receive
    /// the change.
    pub(super) fn forget(&mut self, worker: u32) -> bool {
        self.unapplied.remove(&worker);
        self.unreceived.remove(&worker)
    }
}

impl GooseAttack {
    /// The manager's run, if this process is a manager with workers.
    fn manager_run(&self) -> Option<&ManagerRun> {
        match self.gaggle.role.as_ref() {
            Some(GaggleRole::Manager(run)) => Some(run),
            _ => None,
        }
    }

    /// [`Self::manager_run`], to change.
    fn manager_run_mut(&mut self) -> Option<&mut ManagerRun> {
        match self.gaggle.role.as_mut() {
            Some(GaggleRole::Manager(run)) => Some(run),
            _ => None,
        }
    }

    /// Whether a change waits for its workers to say they received it: until
    /// then, every other control request stays queued, in the order it came.
    /// None waits for a change to be applied.
    pub(crate) fn gaggle_control_pending(&self) -> bool {
        self.manager_run().is_some_and(|run| run.pending.is_some())
    }

    /// A Gaggle manager's command that changes the run, other than a Start:
    /// checked as one process checks it, and against what a Gaggle needs,
    /// applied to the manager's own run, then sent to every worker as its
    /// share; the front end hears once every worker received it. A Start is
    /// answered as one process answers it, and its workers are handed their
    /// shares as the run begins.
    pub(crate) async fn gaggle_control(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        command: ControlCommand,
        respond: oneshot::Sender<ControlReply>,
    ) -> Result<(), GooseError> {
        #[cfg(feature = "dashboard")]
        let with_settings = matches!(command, ControlCommand::Settings { .. });
        let (outcome, controls) = match self.gaggle_change(goose_attack_run_state, command).await {
            Ok(change) => change,
            Err(e) => {
                let internal = self.control_outcome(
                    goose_attack_run_state,
                    false,
                    Some("internal"),
                    "internal control error",
                    None,
                );
                let _ = respond.send(ControlReply::Outcome(internal));
                return Err(e);
            }
        };
        #[cfg(feature = "dashboard")]
        let outcome = match with_settings {
            true => ControlOutcome {
                settings: Some(Box::new(self.snapshot_settings(goose_attack_run_state))),
                ..outcome
            },
            false => outcome,
        };
        match controls {
            Some(controls) => self.fan_out(outcome, controls, respond),
            None => {
                let _ = respond.send(ControlReply::Outcome(outcome));
            }
        }
        Ok(())
    }

    /// The manager's outcome for `command`, and each worker's control when it
    /// was applied.
    async fn gaggle_change(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        command: ControlCommand,
    ) -> Result<Change, GooseError> {
        let state = &mut *goose_attack_run_state;
        let running = matches!(
            self.attack_phase,
            AttackPhase::Increase | AttackPhase::Maintain | AttackPhase::Decrease
        );
        Ok(match command {
            ControlCommand::Command(ControllerCommand::Stop) => {
                let outcome = self.control_stop(state).await?;
                let stop = outcome.ok.then(|| self.every_worker(WorkerControl::Stop));
                (outcome, stop)
            }
            ControlCommand::Command(ControllerCommand::Shutdown) => {
                let outcome = self.control_shutdown(state).await?;
                let stop = self.every_worker(WorkerControl::Shutdown);
                if running {
                    (outcome, Some(stop))
                } else {
                    // From Idle the manager exits at once: each worker is
                    // told it was asked to, ahead of the `Shutdown` that
                    // follows, so it exits cleanly.
                    self.tell_workers(stop);
                    (outcome, None)
                }
            }
            ControlCommand::Command(ControllerCommand::Quit) => {
                let outcome = self.control_quit(state);
                if outcome.ok {
                    let stop = self.every_worker(WorkerControl::Shutdown);
                    self.tell_workers(stop);
                }
                (outcome, None)
            }
            ControlCommand::Command(ControllerCommand::Users(users)) => {
                let settings = RunSettings {
                    users: Some(users),
                    ..RunSettings::default()
                };
                self.gaggle_settings(state, settings, true)?
            }
            ControlCommand::Command(ControllerCommand::TestPlan(plan)) => {
                let settings = RunSettings {
                    test_plan: Some(Some(plan)),
                    ..RunSettings::default()
                };
                self.gaggle_settings(state, settings, false)?
            }
            ControlCommand::Command(command) => {
                self.gaggle_settings(state, RunSettings::from_command(command), false)?
            }
            #[cfg(feature = "dashboard")]
            ControlCommand::Settings { settings, .. } => {
                self.gaggle_settings(state, *settings, false)?
            }
            #[cfg(feature = "dashboard")]
            // `fans_out` keeps these on the path one process takes.
            ControlCommand::Report { .. } | ControlCommand::SaveReportNow => {
                unreachable!("the report switches never fan out")
            }
        })
    }

    /// Send each worker its control, no front end waiting.
    fn tell_workers(&mut self, controls: Vec<(u32, WorkerControl)>) {
        if let Some(run) = self.manager_run_mut() {
            let _ = send_controls(run, controls);
        }
    }

    /// `control` for every worker still running.
    fn every_worker(&self, control: WorkerControl) -> Vec<(u32, WorkerControl)> {
        self.manager_run()
            .map(|run| {
                live(run)
                    .into_iter()
                    .map(|id| (id, control.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Run settings on a manager: refused as a Gaggle needs, then as one
    /// process refuses them; applied to the manager's run, and each worker's
    /// share made. `users` is the `users` command, which changes a running
    /// load test's users as the settings can not.
    fn gaggle_settings(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        settings: RunSettings,
        users: bool,
    ) -> Result<Change, GooseError> {
        // Settings go through `refuse_settings`, which checks what a Gaggle
        // needs first; the users command does not.
        if users {
            if let Some(refused) = self.gaggle_refuse(goose_attack_run_state, &settings) {
                return Ok((refused, None));
            }
        }
        let running = matches!(
            self.attack_phase,
            AttackPhase::Increase | AttackPhase::Maintain | AttackPhase::Decrease
        );
        let active = goose_attack_run_state.active_users;
        let ramp_ms = settings
            .users
            .map(|target| self.users_ramp_ms(active, target));
        let outcome = match (users, settings.users) {
            (true, Some(target)) => self.control_set_users(goose_attack_run_state, target)?,
            _ => self.control_settings(goose_attack_run_state, settings.clone())?,
        };
        if !outcome.ok {
            return Ok((outcome, None));
        }
        // An idle worker keeps nothing: its next share carries every change.
        if !running {
            let idle = WorkerControl::Settings(WorkerSettings::default());
            return Ok((outcome, Some(self.every_worker(idle))));
        }
        let shares = self.worker_shares(goose_attack_run_state, &settings, ramp_ms.unwrap_or(0))?;
        Ok((outcome, Some(shares)))
    }

    /// Why `settings` can not be applied to this Gaggle, before the rules
    /// one process applies: a running throttle below one request a second
    /// for each worker; and a running users ramp whose
    /// workers run test plans of their own, as one had no users at the
    /// start, can not change its users, rates or run time.
    pub(crate) fn gaggle_refuse(
        &self,
        goose_attack_run_state: &GooseAttackRunState,
        settings: &RunSettings,
    ) -> Option<ControlOutcome> {
        let run = self.manager_run()?;
        let refuse = |error, field, message: String| {
            Some(
                self.control_outcome(goose_attack_run_state, false, Some(error), message, None)
                    .with_field(field),
            )
        };
        let running = matches!(
            self.attack_phase,
            AttackPhase::Increase | AttackPhase::Maintain | AttackPhase::Decrease
        );
        let live = live(run);
        if running && self.run_shape.is_some() && !run.ramps {
            let field = settings.fields().into_iter().find(|field| {
                matches!(
                    *field,
                    "users" | "increase_rate" | "decrease_rate" | "run_time"
                )
            });
            if let Some(field) = field {
                return refuse(
                    "invalid_phase",
                    field,
                    format!("failed to configure {field}, a worker of this Gaggle had no users when the run started, so each runs a test plan of its own: stop, and start with at least as many users as workers"),
                );
            }
        }
        if let Some(Some(throttle)) = settings.throttle {
            // The shares follow the users the change would leave: checked
            // alone, against the users each worker runs now.
            if running && (settings.users.is_some() || settings.test_plan.is_some()) {
                return refuse(
                    "conflicting_settings",
                    "throttle",
                    "failed to configure throttle, a running Gaggle changes its throttle alone, not with users or a test plan".to_string(),
                );
            }
            if throttle < live.len() {
                return refuse(
                    "conflicting_settings",
                    "throttle",
                    format!(
                        "failed to configure throttle, {throttle} is fewer than the {} workers: each needs at least 1 request per second",
                        live.len()
                    ),
                );
            }
        }
        None
    }

    /// Each running worker's share of `settings`, which the manager's run
    /// just took: the users and a test plan as [`UserBook`] deals them, each
    /// rate by the worker's share of the users, the same host and times, and
    /// the throttle by its share of the users, at least 1. A change to the
    /// users or test plan moves the decrease rate and throttle shares with
    /// them. `ramp_ms` is how long a users change takes on the manager.
    fn worker_shares(
        &mut self,
        goose_attack_run_state: &GooseAttackRunState,
        settings: &RunSettings,
        ramp_ms: usize,
    ) -> Result<Vec<(u32, WorkerControl)>, GooseError> {
        let Some(run) = self.manager_run() else {
            return Ok(Vec::new());
        };
        let ids = live(run);
        let mask = mask(run);
        let workers = run.workers.len();
        let total = run.book.total();
        // Draw the scenarios one process would give the users added, before
        // the book is borrowed.
        let split = match &settings.test_plan {
            Some(Some(plan)) => super::spec::plans(&plan.steps, ids.len()),
            _ => Vec::new(),
        };
        let added = match settings.users {
            Some(users) => users.saturating_sub(total),
            None => split
                .iter()
                .map(|plan| super::spec::users_started(plan))
                .sum(),
        };
        let order = match added {
            0 => Vec::new(),
            added => self.user_scenarios(added)?,
        };
        let throttle = self.configuration.throttle_requests();
        let decrease_rate = self
            .configuration
            .decrease_rate
            .as_deref()
            .and_then(|rate| rate.parse::<f32>().ok());
        let Some(run) = self.manager_run_mut() else {
            return Ok(Vec::new());
        };
        let mut shares = vec![WorkerSettings::default(); workers];
        let mut moved = false;
        if let Some(users) = settings.users {
            run.book.set_users(users, &mask, order.clone());
            for &id in &ids {
                shares[id as usize].users = Some(WorkerUsers {
                    users: run.book.users(id as usize).to_vec(),
                    ramp_ms,
                });
            }
            moved = true;
        }
        if let Some(plan) = &settings.test_plan {
            match plan {
                Some(plan) => {
                    let holds = plan.steps.last().is_some_and(|&(users, _)| users > 0);
                    let mut plans = vec![Vec::new(); workers];
                    for (&id, steps) in ids.iter().zip(split) {
                        plans[id as usize] = steps;
                    }
                    let users = run.book.set_plan(&plans, &order);
                    for (&id, users) in ids.iter().map(|id| (id, &users[*id as usize])) {
                        shares[id as usize].test_plan = Some(Some(WorkerPlan {
                            steps: plans[id as usize].clone(),
                            users: users.clone(),
                            holds,
                        }));
                    }
                    moved = true;
                }
                None => {
                    for &id in &ids {
                        shares[id as usize].test_plan = Some(None);
                    }
                }
            }
        }
        let counts: Vec<usize> = ids
            .iter()
            .map(|&id| run.book.users(id as usize).len())
            .collect();
        let rates = |rate: Option<Option<f32>>| -> Vec<Option<Option<f32>>> {
            match rate {
                Some(Some(rate)) => rate_shares(rate, &counts)
                    .into_iter()
                    .map(|r| Some(Some(r)))
                    .collect(),
                other => vec![other; ids.len()],
            }
        };
        let increase = rates(settings.increase_rate);
        let decrease = match (settings.decrease_rate, decrease_rate) {
            // The decrease rate follows the users it ramps down.
            (None, Some(rate)) if moved && settings.decrease_time.is_none() => {
                rates(Some(Some(rate)))
            }
            (rate, _) => rates(rate),
        };
        let throttles: Vec<Option<Option<usize>>> = match settings.throttle {
            Some(Some(throttle)) => live_throttle_shares(throttle, &counts)
                .into_iter()
                .map(|share| Some(Some(share)))
                .collect(),
            None if moved && throttle > 0 => follow_throttle(&ids, &counts, throttle)
                .into_iter()
                .map(|share| Some(Some(share)))
                .collect(),
            other => vec![other; ids.len()],
        };
        let next = run.book.next_ordinal();
        let controls = ids
            .iter()
            .enumerate()
            .map(|(at, &id)| {
                let mut share = std::mem::take(&mut shares[id as usize]);
                share.host.clone_from(&settings.host);
                share.increase_rate = increase[at];
                share.increase_time = settings.increase_time;
                share.decrease_rate = decrease[at];
                share.decrease_time = settings.decrease_time;
                share.run_time = settings.run_time;
                share.throttle = throttles[at];
                (id, WorkerControl::Settings(share))
            })
            .collect();
        // The merge takes the ordinals the added users hold.
        let _ = goose_attack_run_state
            .metrics_cmd_tx
            .send(MetricsCommand::GaggleMaxUsers {
                max_users: next as u64,
            });
        Ok(controls)
    }

    /// Send each worker its control, as one change: the front end hears
    /// `outcome` once every worker received it or was lost.
    fn fan_out(
        &mut self,
        outcome: ControlOutcome,
        controls: Vec<(u32, WorkerControl)>,
        respond: oneshot::Sender<ControlReply>,
    ) {
        let Some(run) = self.manager_run_mut() else {
            let _ = respond.send(ControlReply::Outcome(outcome));
            return;
        };
        let Some(id) = send_controls(run, controls) else {
            let _ = respond.send(ControlReply::Outcome(outcome));
            return;
        };
        run.pending = Some(PendingControl {
            id,
            respond,
            outcome,
            lost: Vec::new(),
        });
    }

    /// A worker lost while a run goes on: the throttle's shares are made
    /// again over the workers left, so the whole load test's limit holds,
    /// and with `--gaggle-rebalance` its users are dealt to them, each
    /// keeping its ordinal and scenario.
    pub(super) fn gaggle_after_loss(
        &mut self,
        goose_attack_run_state: &GooseAttackRunState,
        run: &mut ManagerRun,
        id: u32,
    ) {
        let running = matches!(
            self.attack_phase,
            AttackPhase::Increase | AttackPhase::Maintain | AttackPhase::Decrease
        );
        let users = run.book.lose(id as usize);
        if !running || goose_attack_run_state.stopping || run.stop_sent || run.ending {
            return;
        }
        let ids = live(run);
        if ids.is_empty() {
            return;
        }
        let mut took = Vec::new();
        if self.configuration.gaggle_rebalance && !users.is_empty() {
            let ramping_down = self
                .run_shape
                .as_ref()
                .is_none_or(|shape| shape.ramp_down_started);
            if run.ramps && !ramping_down {
                info!(
                    "gaggle: moving worker {id}'s {} users to the {} workers left",
                    users.len(),
                    ids.len()
                );
                took = run.book.deal(users, &mask(run));
            } else {
                warn!("gaggle: worker {id}'s users are not moved: --gaggle-rebalance moves the users of a users ramp before its ramp down, not of a test plan");
            }
        }
        let throttle = self.configuration.throttle_requests();
        if took.is_empty() && throttle == 0 {
            return;
        }
        // During the ramp up, the moved users start with the survivors' own
        // still to start, over what is left of the ramp, not all at once.
        let ramp_ms = match self.attack_phase {
            AttackPhase::Increase => {
                let elapsed = self.step_elapsed() as usize;
                self.test_plan
                    .steps
                    .get(self.test_plan.current)
                    .map_or(0, |step| step.1.saturating_sub(elapsed))
                    .max(REBALANCE_RAMP_MS)
            }
            _ => REBALANCE_RAMP_MS,
        };
        let counts: Vec<usize> = ids
            .iter()
            .map(|&id| run.book.users(id as usize).len())
            .collect();
        let shares = (throttle > 0).then(|| follow_throttle(&ids, &counts, throttle));
        let controls = ids
            .iter()
            .enumerate()
            .map(|(at, &id)| {
                let users = took.contains(&(id as usize)).then(|| WorkerUsers {
                    users: run.book.users(id as usize).to_vec(),
                    ramp_ms,
                });
                let settings = WorkerSettings {
                    users,
                    throttle: shares.as_ref().map(|shares| Some(shares[at])),
                    ..WorkerSettings::default()
                };
                (id, WorkerControl::Settings(settings))
            })
            .collect();
        if let Some(shares) = shares.as_ref() {
            info!("gaggle: throttle shares over the workers left: {shares:?}");
        }
        // Answered as any change is, but no front end waits on it.
        let _ = send_controls(run, controls);
    }

    /// The changes sent to the workers: a worker that has not said it
    /// received one within [`ACK_TIMEOUT`] is lost; the change a front end
    /// waits on is answered once every worker received it; a change some
    /// worker received and has yet to apply is warned about every
    /// [`APPLY_WARNING`]; and one every worker applied is logged.
    pub(super) fn gaggle_settle(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        run: &mut ManagerRun,
    ) {
        let now = Instant::now();
        let late: BTreeSet<u32> = run
            .sent
            .iter()
            .filter(|sent| now.duration_since(sent.sent) >= ACK_TIMEOUT)
            .flat_map(|sent| sent.unreceived.iter().copied())
            .collect();
        for id in late {
            let why = format!(
                "it did not receive a change within {} seconds",
                ACK_TIMEOUT.as_secs()
            );
            self.lose_worker(goose_attack_run_state, run, id, why);
            // Never waited on again, however it was marked.
            for sent in run.sent.iter_mut() {
                sent.forget(id);
            }
        }
        let received = run.pending.as_ref().map(|pending| {
            run.sent
                .iter()
                .find(|sent| sent.id == pending.id)
                .is_none_or(|sent| sent.unreceived.is_empty())
        });
        if received == Some(true) {
            if let Some(pending) = run.pending.take() {
                let workers = run
                    .sent
                    .iter()
                    .find(|sent| sent.id == pending.id)
                    .map_or(0, |sent| sent.workers);
                let mut outcome = pending.outcome;
                outcome.message = format!(
                    "{}; accepted by {} of {} workers",
                    outcome.message,
                    workers.saturating_sub(pending.lost.len()),
                    workers
                );
                if !pending.lost.is_empty() {
                    outcome.message = format!("{}; {}", outcome.message, pending.lost.join("; "));
                }
                let _ = pending.respond.send(ControlReply::Outcome(outcome));
            }
        }
        for sent in run.sent.iter_mut() {
            if now.duration_since(sent.warned) < APPLY_WARNING {
                continue;
            }
            let applying: Vec<u32> = sent
                .unapplied
                .difference(&sent.unreceived)
                .copied()
                .collect();
            if applying.is_empty() {
                continue;
            }
            sent.warned = now;
            for id in applying {
                let name = run
                    .workers
                    .get(id as usize)
                    .map_or("", |worker| worker.name.as_str());
                warn!(
                    "gaggle: worker {id} ({name}) received change {} ({}) and has yet to apply it, {} seconds after it was sent",
                    sent.id,
                    sent.what,
                    now.duration_since(sent.sent).as_secs()
                );
                #[cfg(test)]
                super::faults::DONE_WARNINGS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        run.sent.retain(|sent| {
            if !sent.unapplied.is_empty() {
                return true;
            }
            info!(
                "gaggle: change {} ({}) applied by {} of {} workers, {:.1} seconds after it was sent",
                sent.id,
                sent.what,
                sent.applied,
                sent.workers,
                now.duration_since(sent.sent).as_secs_f64()
            );
            #[cfg(test)]
            super::faults::CHANGES_DONE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            false
        });
    }
}

/// The workers neither lost nor gone, by id.
fn live(run: &ManagerRun) -> Vec<u32> {
    run.workers
        .iter()
        .filter(|worker| worker.is_live())
        .map(|worker| worker.id)
        .collect()
}

/// Whether each worker, at its id, can take users: neither lost nor gone,
/// and still running, as one whose users all exited (`--iterations`) is
/// idle until the next run.
fn mask(run: &ManagerRun) -> Vec<bool> {
    run.workers
        .iter()
        .map(|worker| worker.is_live() && worker.stopped.is_none())
        .collect()
}

/// The throttle's shares over workers `ids` running `counts` users, kept to
/// 1 a worker when `throttle` is fewer than the workers, which is logged.
fn follow_throttle(ids: &[u32], counts: &[usize], throttle: usize) -> Vec<usize> {
    if throttle < ids.len() {
        warn!(
            "gaggle: the throttle of {throttle} is fewer than the {} workers left: each keeps 1 request per second",
            ids.len()
        );
    }
    live_throttle_shares(throttle, counts)
        .into_iter()
        .map(|share| share.max(1))
        .collect()
}

/// Send each worker its control under one new id, kept in
/// [`ManagerRun::sent`] until every worker applied it: the id, `None` when
/// it went to no worker.
fn send_controls(run: &mut ManagerRun, controls: Vec<(u32, WorkerControl)>) -> Option<u64> {
    let id = run.next_control;
    run.next_control += 1;
    let what = controls
        .first()
        .map_or_else(String::new, |(_, control)| control.describe());
    let mut to = BTreeSet::new();
    for (worker, control) in controls {
        match control.encode() {
            Ok(command) => {
                run.manager
                    .send(worker, ToWorker::Control(Control { id, command }));
                to.insert(worker);
            }
            Err(e) => warn!("gaggle: could not encode a change for worker {worker}: {e}"),
        }
    }
    if to.is_empty() {
        return None;
    }
    let now = Instant::now();
    run.sent.push(SentControl {
        id,
        what,
        sent: now,
        workers: to.len(),
        unreceived: to.clone(),
        unapplied: to,
        applied: 0,
        warned: now,
    });
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gaggle::spec::{plans, split, users_started};
    use rand::{Rng, SeedableRng};

    /// The scenario one process gives each of `added` users, in the order
    /// they start: a stand in for its weighted allocation.
    fn allocation(added: usize, weights: &[usize]) -> Vec<usize> {
        let cycle: Vec<usize> = weights
            .iter()
            .enumerate()
            .flat_map(|(scenario, &weight)| std::iter::repeat_n(scenario, weight))
            .collect();
        let mut order: Vec<usize> = cycle.iter().copied().cycle().take(added).collect();
        order.reverse();
        order
    }

    /// One process running `users` changes: the users it runs, by ordinal,
    /// stopping the last started first.
    #[derive(Default)]
    struct OneProcess {
        running: Vec<User>,
        next: usize,
    }

    impl OneProcess {
        fn set_users(&mut self, users: usize, scenarios: &[usize]) {
            while self.running.len() > users {
                self.running.pop();
            }
            for &scenario in scenarios
                .iter()
                .take(users.saturating_sub(self.running.len()))
            {
                self.running.push((self.next, scenario));
                self.next += 1;
            }
        }
    }

    /// Every user of `book`'s live workers, by ordinal.
    fn every_user(book: &UserBook, workers: usize) -> Vec<User> {
        let mut users: Vec<User> = (0..workers).flat_map(|w| book.users(w).to_vec()).collect();
        users.sort_unstable();
        users
    }

    #[test]
    fn shares_add_up_to_the_whole() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(645);
        for _ in 0..2_000 {
            let workers = rng.random_range(1..12);
            let users: Vec<usize> = (0..workers).map(|_| rng.random_range(0..400)).collect();
            let total: usize = users.iter().sum();
            // Users.
            let global = rng.random_range(0..100_000);
            let parts: usize = (0..workers).map(|w| split(global, workers, w)).sum();
            assert_eq!(parts, global);
            // Rates, over the workers that run users.
            let rate = rng.random_range(0.01_f32..100_000.0);
            let rates = rate_shares(rate, &users);
            let busy: f64 = rates
                .iter()
                .zip(&users)
                .filter(|(_, &users)| users > 0)
                .map(|(&rate, _)| f64::from(rate))
                .sum();
            if total > 0 {
                assert!(
                    (busy - f64::from(rate)).abs() <= f64::from(rate) * 1e-5,
                    "{} {:?}",
                    rate,
                    rates
                );
            }
            assert!(rates.iter().all(|&rate| rate > 0.0));
            // Test plan steps.
            let steps: Vec<(usize, usize)> = (0..rng.random_range(1..6))
                .map(|_| (rng.random_range(0..5_000), rng.random_range(0..60_000)))
                .collect();
            let split_plans = plans(&steps, workers);
            for (step, &(users, ms)) in steps.iter().enumerate() {
                let sum: usize = split_plans.iter().map(|plan| plan[step].0).sum();
                assert_eq!(sum, users);
                assert!(split_plans.iter().all(|plan| plan[step].1 == ms));
            }
            // The throttle: never 0, and the whole once it covers a request a
            // second for each worker.
            let throttle = rng.random_range(1..1_000_000);
            let shares = live_throttle_shares(throttle, &users);
            assert!(
                shares.iter().all(|&share| share > 0),
                "{} {:?}",
                throttle,
                users
            );
            if throttle >= workers {
                assert_eq!(
                    shares.iter().sum::<usize>(),
                    throttle,
                    "{users:?} {shares:?}"
                );
            }
        }
    }

    #[test]
    fn users_changes_run_what_one_process_runs() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(797);
        for _ in 0..300 {
            let workers = rng.random_range(1..8);
            let weights: Vec<usize> = (0..rng.random_range(1..4))
                .map(|_| rng.random_range(1..7))
                .collect();
            let live = vec![true; workers];
            // The run starts as a manager hands out shares: blocks of
            // ordinals in worker order.
            let start = rng.random_range(workers..300);
            let order = allocation(start, &weights);
            let mut offset = 0;
            let blocks = (0..workers)
                .map(|w| {
                    let count = split(start, workers, w);
                    let block = (offset..offset + count).map(|at| (at, order[at])).collect();
                    offset += count;
                    block
                })
                .collect();
            let mut book = UserBook::new(blocks, start);
            let mut one = OneProcess::default();
            one.set_users(start, &order);
            for _ in 0..rng.random_range(1..20) {
                let users: usize = rng.random_range(1..600);
                let added = users.saturating_sub(book.total());
                let scenarios = allocation(added, &weights);
                book.set_users(users, &live, scenarios.clone());
                one.set_users(users, &scenarios);
                assert_eq!(book.total(), users);
                assert_eq!(book.next_ordinal(), one.next);
                // The same users, each with the same scenario: no ordinal
                // twice, none skipped, as one process numbers them.
                assert_eq!(every_user(&book, workers), one.running);
                // Each worker starts its users in ordinal order.
                for w in 0..workers {
                    assert!(book.users(w).windows(2).all(|pair| pair[0].0 < pair[1].0));
                }
            }
        }
    }

    #[test]
    fn added_users_go_to_the_workers_running_fewest() {
        let first = vec![(0, 0), (1, 0), (2, 0)];
        let mut book = UserBook::new(vec![first, vec![(3, 0)], Vec::new()], 4);
        let changed = book.set_users(9, &[true, true, false], vec![1; 5]);
        assert_eq!(changed, [true, true, false]);
        assert_eq!(
            [
                book.users(0).len(),
                book.users(1).len(),
                book.users(2).len()
            ],
            [5, 4, 0]
        );
        // The highest ordinals stop first, wherever they run.
        book.set_users(4, &[true, true, false], Vec::new());
        assert_eq!(book.total(), 4);
        assert_eq!(every_user(&book, 3), [(0, 0), (1, 0), (2, 0), (3, 0)]);
    }

    #[test]
    fn a_lost_workers_users_keep_their_ordinals_and_scenarios() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(66);
        for _ in 0..200 {
            let workers = rng.random_range(2..8);
            let blocks: Vec<Vec<User>> = (0..workers)
                .map(|w| {
                    (0..rng.random_range(0..50))
                        .map(|at| (w * 100 + at, at % 3))
                        .collect()
                })
                .collect();
            let mut book = UserBook::new(blocks, workers * 100);
            let before = every_user(&book, workers);
            let lost = rng.random_range(0..workers);
            let mut live = vec![true; workers];
            live[lost] = false;
            let users = book.lose(lost);
            book.deal(users, &live);
            assert!(book.users(lost).is_empty());
            assert_eq!(every_user(&book, workers), before);
        }
    }

    #[test]
    fn a_test_plan_hands_out_new_ordinals() {
        let steps = [(10, 1_000), (4, 1_000), (7, 1_000)];
        let split_plans = plans(&steps, 3);
        let total: usize = split_plans.iter().map(|plan| users_started(plan)).sum();
        let order: Vec<usize> = (0..total).map(|at| at % 2).collect();
        let mut book = UserBook::new(vec![vec![(0, 0)], vec![(1, 1)], vec![(2, 0)]], 3);
        let users = book.set_plan(&split_plans, &order);
        let ordinals: Vec<usize> = users
            .iter()
            .flatten()
            .map(|&(ordinal, _)| ordinal)
            .collect();
        assert_eq!(ordinals, (3..3 + total).collect::<Vec<_>>());
        for (plan, users) in split_plans.iter().zip(&users) {
            assert_eq!(users.len(), users_started(plan));
        }
        assert_eq!(book.next_ordinal(), 3 + total);
    }

    #[test]
    fn worker_controls_survive_the_wire() {
        let control = WorkerControl::Settings(WorkerSettings {
            host: Some(Some("http://h/".to_string())),
            users: Some(WorkerUsers {
                users: vec![(7, 1), (9, 0)],
                ramp_ms: 1_500,
            }),
            decrease_rate: Some(Some(0.5)),
            run_time: Some(None),
            throttle: Some(Some(40)),
            ..WorkerSettings::default()
        });
        assert_eq!(
            WorkerControl::decode(&control.encode().unwrap()).unwrap(),
            control
        );
        assert!(WorkerControl::decode(&[0xff, 0xff]).is_err());
    }

    /// The scenarios the manager gives the users a `users` change adds are
    /// those one process starts: it builds a change's users with
    /// `user_scenario_order` and starts the last built first.
    #[test]
    fn a_managers_users_take_one_process_scenarios() {
        use crate::goose::{Scenario, Transaction};
        let noop = || Transaction::new(std::sync::Arc::new(|_| Box::pin(async { Ok(()) })));
        let mut attack = crate::GooseAttack::initialize_with_config(Default::default())
            .unwrap()
            .register_scenario(
                Scenario::new("A")
                    .set_weight(3)
                    .unwrap()
                    .register_transaction(noop()),
            )
            .register_scenario(Scenario::new("B").register_transaction(noop()));
        let live = vec![true; 3];
        let mut book = UserBook::new(vec![Vec::new(); 3], 0);
        let mut one: Vec<User> = Vec::new();
        let mut next = 0;
        for users in [5, 12, 7, 15, 1, 9] {
            if users > one.len() {
                let added = users - one.len();
                // One process: built in order, popped from the end.
                let mut built = attack.user_scenario_order(added).unwrap();
                while let Some(scenario) = built.pop() {
                    one.push((next, scenario));
                    next += 1;
                }
            } else {
                one.truncate(users);
            }
            let added = users.saturating_sub(book.total());
            let scenarios = match added {
                0 => Vec::new(),
                added => attack.user_scenarios(added).unwrap(),
            };
            book.set_users(users, &live, scenarios);
            assert_eq!(every_user(&book, 3), one, "after users {}", users);
        }
    }
}
