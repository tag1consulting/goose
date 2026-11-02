//! Runtime control of a Gaggle: what a manager sends each worker when a
//! Controller or the dashboard changes the run, and the arithmetic that
//! splits a change into each worker's share, so the workers together run what
//! one process running the whole load test would.
//!
//! The manager checks a change against the rules one process applies, applies
//! it to its own run, the run's clock, and sends each worker its share in a
//! `Control`. A worker applies its share through the same checks and answers
//! with a `ControlAck`; the front end hears once every worker answered.

use super::proto::{manager_message::Message as ToWorker, Control};
use super::run::{GaggleRole, ManagerRun};
use crate::controller::{
    ControlCommand, ControlReply, ControllerCommand, RunSettings, MIN_CONTROL_RATE,
};
use crate::metrics::MetricsCommand;
use crate::throttle::MAX_THROTTLE_RISE;
use crate::{AttackPhase, ControlOutcome, GooseAttack, GooseAttackRunState, GooseError};
use serde::{Deserialize, Serialize};
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
        let mut took = Vec::new();
        for user in users {
            let fewest = (0..self.workers.len())
                .filter(|&worker| live.get(worker).copied().unwrap_or(false))
                .min_by_key(|&worker| (self.workers[worker].len(), worker));
            let Some(worker) = fewest else {
                break;
            };
            self.workers[worker].push(user);
            if !took.contains(&worker) {
                took.push(worker);
            }
        }
        took
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

/// How long the manager waits for every worker to answer a change: 10
/// seconds, inside the 15 a front end waits.
pub(crate) const ACK_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_secs(2)
} else {
    Duration::from_secs(10)
};
const _: () = assert!(ACK_TIMEOUT.as_millis() < crate::controller::CONTROL_TIMEOUT.as_millis());

/// How long a lost worker's users take to start on the workers left, with
/// `--gaggle-rebalance`.
const REBALANCE_RAMP_MS: usize = 1_000;

/// A change sent to the workers, which its front end hears about once every
/// worker answered, or was lost.
pub(crate) struct PendingControl {
    pub(super) id: u64,
    /// The workers yet to answer.
    pub(super) awaiting: BTreeSet<u32>,
    pub(super) deadline: Instant,
    pub(super) respond: oneshot::Sender<ControlReply>,
    /// The manager's own outcome, which the reply carries.
    pub(super) outcome: ControlOutcome,
    /// Each worker lost meanwhile, and why.
    pub(super) lost: Vec<String>,
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

    /// Whether a change waits for its workers' answers: until it is
    /// answered, every other control request stays queued, in the order it
    /// came.
    pub(crate) fn gaggle_control_pending(&self) -> bool {
        self.manager_run().is_some_and(|run| run.pending.is_some())
    }

    /// A Gaggle manager's command that changes the run, other than a Start:
    /// checked as one process checks it, and against what a Gaggle needs,
    /// applied to the manager's own run, then sent to every worker as its
    /// share; the front end hears once every worker answered. A Start is
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
                // From Idle, every worker is told to shut down as the
                // manager does.
                let stop = running.then(|| self.every_worker(WorkerControl::Shutdown));
                (outcome, stop)
            }
            ControlCommand::Command(ControllerCommand::Quit) => (self.control_quit(state), None),
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
            ControlCommand::Report { .. } | ControlCommand::SaveReportNow => {
                let refused = self.control_outcome(
                    state,
                    false,
                    Some("internal"),
                    "internal control error",
                    None,
                );
                (refused, None)
            }
        })
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
        if let Some(refused) = self.gaggle_refuse(goose_attack_run_state, &settings) {
            return Ok((refused, None));
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
    /// for each worker, or one that would raise a worker's share past
    /// [`MAX_THROTTLE_RISE`] times its start; and a running users ramp whose
    /// workers run test plans of their own, as one had no users at the
    /// start, can not change its users, rates or run time.
    fn gaggle_refuse(
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
            if running && goose_attack_run_state.throttle_started > 0 {
                let users: Vec<usize> = live
                    .iter()
                    .map(|&id| run.book.users(id as usize).len())
                    .collect();
                let shares = live_throttle_shares(throttle, &users);
                for (&id, share) in live.iter().zip(shares) {
                    let started = run.throttle_started[id as usize];
                    if share > started.saturating_mul(MAX_THROTTLE_RISE) {
                        return refuse(
                            "conflicting_settings",
                            "throttle",
                            format!("failed to configure throttle, worker {id}'s share would be {share}, more than {MAX_THROTTLE_RISE} times the {started} it started with"),
                        );
                    }
                }
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
                Some(_) => {
                    let mut plans = vec![Vec::new(); workers];
                    for (&id, steps) in ids.iter().zip(split) {
                        plans[id as usize] = steps;
                    }
                    let users = run.book.set_plan(&plans, &order);
                    for (&id, users) in ids.iter().map(|id| (id, &users[*id as usize])) {
                        shares[id as usize].test_plan = Some(Some(WorkerPlan {
                            steps: plans[id as usize].clone(),
                            users: users.clone(),
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
            None if moved && throttle > 0 => follow_throttle(run, &ids, &counts, throttle)
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
    /// `outcome` once every worker answered or was lost.
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
        let id = send_controls(run, controls);
        let awaiting = id.1;
        if awaiting.is_empty() {
            let _ = respond.send(ControlReply::Outcome(outcome));
            return;
        }
        run.pending = Some(PendingControl {
            id: id.0,
            awaiting,
            deadline: Instant::now() + ACK_TIMEOUT,
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
            if run.ramps && self.run_shape.is_some() {
                info!(
                    "gaggle: moving worker {id}'s {} users to the {} workers left",
                    users.len(),
                    ids.len()
                );
                took = run.book.deal(users, &mask(run));
            } else {
                warn!("gaggle: worker {id}'s users are not moved: --gaggle-rebalance moves the users of a users ramp, not of a test plan");
            }
        }
        let throttle = self.configuration.throttle_requests();
        if took.is_empty() && throttle == 0 {
            return;
        }
        let counts: Vec<usize> = ids
            .iter()
            .map(|&id| run.book.users(id as usize).len())
            .collect();
        let shares = (throttle > 0).then(|| follow_throttle(run, &ids, &counts, throttle));
        let controls = ids
            .iter()
            .enumerate()
            .map(|(at, &id)| {
                let users = took.contains(&(id as usize)).then(|| WorkerUsers {
                    users: run.book.users(id as usize).to_vec(),
                    ramp_ms: REBALANCE_RAMP_MS,
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

    /// The manager's change waiting on its workers: answered once none is
    /// left to answer, and each still silent at its deadline is lost.
    pub(super) fn gaggle_settle(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        run: &mut ManagerRun,
    ) {
        let late: Vec<u32> = match run.pending.as_ref() {
            Some(pending) if Instant::now() >= pending.deadline => {
                pending.awaiting.iter().copied().collect()
            }
            _ => Vec::new(),
        };
        for id in late {
            let why = format!(
                "it did not answer a change within {} seconds",
                ACK_TIMEOUT.as_secs()
            );
            self.lose_worker(goose_attack_run_state, run, id, why);
        }
        if run
            .pending
            .as_ref()
            .is_some_and(|pending| pending.awaiting.is_empty())
        {
            if let Some(pending) = run.pending.take() {
                let mut outcome = pending.outcome;
                if !pending.lost.is_empty() {
                    outcome.message = format!("{}; {}", outcome.message, pending.lost.join("; "));
                }
                let _ = pending.respond.send(ControlReply::Outcome(outcome));
            }
        }
    }
}

/// The workers neither lost nor gone, by id.
fn live(run: &ManagerRun) -> Vec<u32> {
    run.workers
        .iter()
        .filter(|worker| worker.lost.is_none() && !worker.gone)
        .map(|worker| worker.id)
        .collect()
}

/// Whether each worker, at its id, is neither lost nor gone.
fn mask(run: &ManagerRun) -> Vec<bool> {
    run.workers
        .iter()
        .map(|worker| worker.lost.is_none() && !worker.gone)
        .collect()
}

/// The throttle's shares over workers `ids` running `counts` users, each
/// kept to [`MAX_THROTTLE_RISE`] times its start, and to 1 a worker when
/// `throttle` is fewer than the workers: either is logged.
fn follow_throttle(run: &ManagerRun, ids: &[u32], counts: &[usize], throttle: usize) -> Vec<usize> {
    if throttle < ids.len() {
        warn!(
            "gaggle: the throttle of {throttle} is fewer than the {} workers left: each keeps 1 request per second",
            ids.len()
        );
    }
    live_throttle_shares(throttle, counts)
        .into_iter()
        .zip(ids)
        .map(|(share, &id)| {
            let most = run.throttle_started[id as usize].saturating_mul(MAX_THROTTLE_RISE);
            if share > most {
                warn!("gaggle: worker {id}'s throttle share {share} is kept to {most}, {MAX_THROTTLE_RISE} times its start");
            }
            share.min(most).max(1)
        })
        .collect()
}

/// Send each worker its control under one new id: the id, and the workers
/// it was sent to.
fn send_controls(
    run: &mut ManagerRun,
    controls: Vec<(u32, WorkerControl)>,
) -> (u64, BTreeSet<u32>) {
    let id = run.next_control;
    run.next_control += 1;
    let mut sent = BTreeSet::new();
    for (worker, control) in controls {
        match control.encode() {
            Ok(command) => {
                run.manager
                    .send(worker, ToWorker::Control(Control { id, command }));
                sent.insert(worker);
            }
            Err(e) => warn!("gaggle: could not encode a change for worker {worker}: {e}"),
        }
    }
    (id, sent)
}
