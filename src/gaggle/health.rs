//! The manager's one model of its workers' health, and its log of what
//! happened to the Gaggle.
//!
//! For each worker: its state, its CPU use (from the CPU time it sends with
//! each delta and the cores it said it has), how long since its last delta
//! was merged, its figures over the whole run and over the last 10 seconds,
//! and what the merge did with its deltas. From these come its flags, each
//! in a live form, on the last 10 seconds, and a run form, on the whole run,
//! which the reports show. The manager's metrics processor writes what it
//! merges, and the manager's run what its workers say and do; both hold the
//! model behind one lock ([`MergeHealth`]).
//!
//! The model never holds a worker's address, host name, port or token: only
//! the name the manager gave it, which is safe as a folder name.

use super::delta::{GaggleDelta, RequestPart, Timing};
use crate::metrics::buckets;
use crate::runs::{
    GaggleEvent, GaggleEventKind, GaggleRecord, GaggleWorkerFlag, GaggleWorkerRecord,
    GaggleWorkerState,
};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The one state a worker is in.
pub(crate) type WorkerState = GaggleWorkerState;

/// A worker is lagging while running when its last delta was merged more
/// than this long ago: a worker sends one a second, so 3 seconds is three
/// missed in a row, past what a busy network or manager delays one by.
pub(crate) const LAGGING_AFTER: Duration = Duration::from_secs(3);

/// A worker's CPU use at or above this percent of its cores counts toward
/// saturation: past it a load generator queues its own work, which shows as
/// longer response times and fewer requests than it was asked for.
pub(crate) const SATURATED_PERCENT: f64 = 90.0;

/// A worker is saturated once its CPU use stays at or above
/// [`SATURATED_PERCENT`] this long, live, or adds up to this long over the
/// run: long enough that a garbage collection or a burst of logging alone
/// does not flag it.
pub(crate) const SATURATED_FOR: Duration = Duration::from_secs(5);

/// A worker is an outlier when its requests per user per second, or its
/// p95, is more than this percent from the median of the running workers:
/// workers running the same plan against the same target land well within
/// it, so a worker past it is shaped by something of its own.
pub(crate) const OUTLIER_PERCENT: f64 = 25.0;

/// Outliers are judged only among at least this many running workers: with
/// two, the median is their middle and each is as far from it as the other.
pub(crate) const OUTLIER_MIN_WORKERS: usize = 3;

/// Outliers are judged only once the run has run this long: until every
/// worker's users have started and warmed up, workers differ for no reason
/// of their own.
pub(crate) const OUTLIER_AFTER: Duration = Duration::from_secs(30);

/// How many seconds of deltas the live window holds: long enough to smooth
/// one slow second, short enough to show a change within seconds.
pub(crate) const WINDOW_SECS: u64 = 10;

/// The longest name the model keeps for a worker, in characters. The
/// manager's names are 64 already ([`super::manager`]); this keeps the
/// model's own memory bounded however a name was made.
const MAX_NAME: usize = 64;

/// A CPU sample is taken against the one before it only once at least this
/// long has passed: deltas that queued on the worker, or on the way, can
/// arrive together, and their CPU over a few milliseconds says nothing.
const MIN_CPU_SPAN: Duration = Duration::from_millis(900);

/// The model behind one lock: the manager's metrics processor writes what
/// it merges, the manager's run the rest.
pub(crate) type MergeHealth = Arc<Mutex<GaggleHealth>>;

/// What the merge did with one worker's deltas: anything here but
/// `last_seq` means the results undercount it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct WorkerMerge {
    /// The sequence number of the last delta merged.
    pub(crate) last_seq: u64,
    /// Deltas refused.
    pub(crate) refused: u64,
    /// Why the last refused delta was refused.
    pub(crate) refusal: Option<String>,
    /// Why the merge last built left the worker out, when it did.
    pub(crate) excluded: Option<String>,
    /// Deltas that never arrived: their graph rows are not in the results.
    pub(crate) missing: u64,
    /// Its final delta was merged.
    pub(crate) final_merged: bool,
}

/// A worker's requests, every request name together.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Fold {
    /// Successes and failures: either can be negative in a delta, where
    /// `set_success()` turned a failure into a success.
    pub(crate) success: i64,
    pub(crate) fail: i64,
    /// Every response time, in the buckets Goose keeps them in.
    pub(crate) times: Timing,
}

impl Fold {
    /// Every request name of `requests` together.
    pub(crate) fn of<'a>(requests: impl IntoIterator<Item = &'a RequestPart>) -> Self {
        let mut fold = Fold::default();
        let mut buckets: Vec<(u64, u64)> = Vec::new();
        for request in requests {
            fold.success = fold.success.saturating_add(request.success);
            fold.fail = fold.fail.saturating_add(request.fail);
            add_times(&mut fold.times, &request.raw, &mut buckets);
        }
        fold.times.buckets = sorted_sums(buckets);
        fold
    }

    /// Add `other`'s requests.
    fn add(&mut self, other: &Fold) {
        self.success = self.success.saturating_add(other.success);
        self.fail = self.fail.saturating_add(other.fail);
        let mut buckets = std::mem::take(&mut self.times.buckets);
        add_times(&mut self.times, &other.times, &mut buckets);
        self.times.buckets = sorted_sums(buckets);
    }

    /// Requests, never below 0.
    pub(crate) fn requests(&self) -> u64 {
        self.success.saturating_add(self.fail).max(0) as u64
    }

    /// Failures, never below 0.
    pub(crate) fn failures(&self) -> u64 {
        self.fail.max(0) as u64
    }

    /// The average response time in milliseconds, `None` without times.
    pub(crate) fn average_ms(&self) -> Option<f64> {
        (self.times.count > 0)
            .then(|| self.times.total_us as f64 / self.times.count as f64 / 1_000.0)
    }

    /// The time `percent` (0.0 to 1.0) of the requests finished within, in
    /// milliseconds, as Goose works out a request's percentiles: `None`
    /// without times.
    pub(crate) fn percentile_ms(&self, percent: f32) -> Option<f64> {
        (self.times.count > 0).then(|| {
            buckets::percentile(
                self.times
                    .buckets
                    .iter()
                    .map(|&(bucket, count)| (buckets::bucket_value(bucket), count as usize)),
                self.times.count as usize,
                self.times.minimum_us,
                self.times.maximum_us,
                percent,
            ) as f64
                / 1_000.0
        })
    }
}

/// Add `from`'s times to `to`, its buckets to `buckets`, which
/// [`sorted_sums`] then sorts and adds up.
fn add_times(to: &mut Timing, from: &Timing, buckets: &mut Vec<(u64, u64)>) {
    if from.count == 0 {
        return;
    }
    if to.count == 0 || from.minimum_us < to.minimum_us {
        to.minimum_us = from.minimum_us;
    }
    to.maximum_us = to.maximum_us.max(from.maximum_us);
    to.total_us = to.total_us.saturating_add(from.total_us);
    to.count = to.count.saturating_add(from.count);
    buckets.extend_from_slice(&from.buckets);
}

/// `buckets` sorted, each bucket once, its counts added up.
fn sorted_sums(mut buckets: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    buckets.sort_unstable_by_key(|&(bucket, _)| bucket);
    buckets.dedup_by(|later, kept| {
        if later.0 == kept.0 {
            kept.1 = kept.1.saturating_add(later.1);
            true
        } else {
            false
        }
    });
    buckets
}

/// One delta's requests in the live window, and the seconds it covers: one,
/// or more when deltas were coalesced.
#[derive(Clone, Debug)]
struct Sample {
    fold: Fold,
    secs: u64,
}

/// A worker's figures over a span: the live window's or the whole run's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Figures {
    pub(crate) requests: u64,
    pub(crate) failures: u64,
    pub(crate) fail_percent: Option<f64>,
    pub(crate) average_ms: Option<f64>,
    pub(crate) p50_ms: Option<f64>,
    pub(crate) p95_ms: Option<f64>,
    pub(crate) p99_ms: Option<f64>,
    pub(crate) requests_per_second: Option<f64>,
    pub(crate) requests_per_user_per_second: Option<f64>,
}

impl Figures {
    /// The figures of `fold` over `secs` seconds, with `users` users.
    fn of(fold: &Fold, secs: f64, users: usize) -> Self {
        let requests = fold.requests();
        let rate = (secs > 0.0).then(|| requests as f64 / secs);
        Figures {
            requests,
            failures: fold.failures(),
            fail_percent: (requests > 0)
                .then(|| fold.failures() as f64 * 100.0 / requests as f64),
            average_ms: fold.average_ms(),
            p50_ms: fold.percentile_ms(0.5),
            p95_ms: fold.percentile_ms(0.95),
            p99_ms: fold.percentile_ms(0.99),
            requests_per_second: rate,
            requests_per_user_per_second: rate
                .filter(|_| users > 0)
                .map(|rate| rate / users as f64),
        }
    }
}

/// A worker's flags at one time: live, on the last 10 seconds, or for the
/// run, on the whole run.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Flags {
    pub(crate) excluded: bool,
    pub(crate) refused: bool,
    pub(crate) missing: bool,
    pub(crate) saturated: bool,
    pub(crate) lagging: bool,
    /// How far its requests per user per second were from the median of
    /// the running workers, in percent, signed, when past
    /// [`OUTLIER_PERCENT`].
    pub(crate) rate_outlier: Option<i32>,
    /// How far its p95 was from the median, as `rate_outlier`.
    pub(crate) p95_outlier: Option<i32>,
}

/// What the manager knows of one worker.
#[derive(Clone, Debug, Default)]
pub(crate) struct WorkerHealth {
    pub(crate) id: u32,
    pub(crate) name: String,
    pub(crate) state: WorkerState,
    /// The cores it said its process may use.
    pub(crate) cores: Option<u32>,
    /// The users it runs now.
    pub(crate) users: usize,
    /// The users it started over the run.
    pub(crate) users_started: usize,
    /// When it was lost, and why.
    pub(crate) lost: Option<(chrono::DateTime<chrono::Utc>, u64, String)>,
    /// What the merge did with its deltas.
    pub(crate) merge: WorkerMerge,
    /// Its last CPU time sample: when it arrived, and the CPU time.
    cpu_sample: Option<(Instant, u64)>,
    /// Its CPU use over the last span between samples, in percent of its
    /// cores.
    pub(crate) cpu_percent: Option<f64>,
    pub(crate) cpu_peak: Option<f64>,
    /// How long its CPU use was at or above [`SATURATED_PERCENT`] over the
    /// run.
    pub(crate) saturated_total: Duration,
    /// Since when its CPU use has stayed at or above [`SATURATED_PERCENT`],
    /// as an instant and an RFC 3339 time: the current saturation's start.
    pub(crate) above_since: Option<(Instant, chrono::DateTime<chrono::Utc>)>,
    /// When it started running its share, and when it stopped.
    running_since: Option<Instant>,
    running_until: Option<Instant>,
    /// When a delta of it was last merged.
    last_delta: Option<Instant>,
    /// The longest it went without one while running.
    pub(crate) longest_gap: Duration,
    /// Its requests over the run, and the epoch they are of.
    totals: Fold,
    totals_epoch: Option<u64>,
    /// Its last deltas' requests, newest last, covering at most
    /// [`WINDOW_SECS`] seconds. Each sample holds one histogram of every
    /// request name's times, its buckets each 16 bytes: Goose's bucket
    /// scheme has 1,330 buckets from 1 microsecond to the default 60 second
    /// timeout, so a sample is at most about 21 KB and the window at most
    /// about 213 KB, and a worker whose times span a few milliseconds holds
    /// a few hundred bytes a sample.
    window: VecDeque<Sample>,
    /// Its live flags as last judged, for the events when one turns on or
    /// off.
    pub(crate) live: Flags,
}

impl super::WorkerId for WorkerHealth {
    fn worker_id(&self) -> u32 {
        self.id
    }
}

impl WorkerHealth {
    /// How long it has run its share, as of `now`.
    fn running_for(&self, now: Instant) -> Duration {
        match self.running_since {
            Some(since) => self.running_until.unwrap_or(now).saturating_duration_since(since),
            None => Duration::ZERO,
        }
    }

    /// Its figures over the whole run, as of `now`.
    pub(crate) fn run_figures(&self, now: Instant) -> Figures {
        let users = self.users_started.max(self.users);
        Figures::of(&self.totals, self.running_for(now).as_secs_f64(), users)
    }

    /// Its figures over the live window.
    pub(crate) fn window_figures(&self) -> Figures {
        let mut fold = Fold::default();
        let mut secs = 0;
        for sample in &self.window {
            fold.add(&sample.fold);
            secs += sample.secs;
        }
        Figures::of(&fold, secs as f64, self.users)
    }

    /// Whether it runs its share.
    fn is_running(&self) -> bool {
        self.state == WorkerState::Running
    }

    /// How long since its last delta was merged while running, as of `now`:
    /// since it started running, before its first.
    fn delta_gap(&self, now: Instant) -> Option<Duration> {
        if !self.is_running() {
            return None;
        }
        let since = self.last_delta.or(self.running_since)?;
        Some(now.saturating_duration_since(since))
    }

    /// The flags its merge conditions give, live and for the run alike.
    fn merge_flags(&self) -> Flags {
        Flags {
            excluded: self.merge.excluded.is_some(),
            refused: self.merge.refused > 0,
            missing: self.merge.missing > 0 || self.final_missing(),
            ..Flags::default()
        }
    }

    /// It stopped after sending metrics, and its final metrics never
    /// arrived: the results hold its last running totals.
    pub(crate) fn final_missing(&self) -> bool {
        matches!(
            self.state,
            WorkerState::Finished | WorkerState::UsersExited
        ) && self.merge.last_seq > 0
            && !self.merge.final_merged
    }
}

/// What the manager knows of its workers, and what happened to the Gaggle.
#[derive(Debug, Default)]
pub(crate) struct GaggleHealth {
    workers: BTreeMap<u32, WorkerHealth>,
    /// The newest [`GaggleRecord::MAX_EVENTS`] events, oldest first.
    events: VecDeque<GaggleEvent>,
    events_dropped: u64,
    /// When the run started running, for the outlier rule.
    running_since: Option<Instant>,
    /// The run's series second, as last told.
    second: u64,
}

impl GaggleHealth {
    /// The merge's view of worker `id`, as the merge last wrote it.
    pub(crate) fn get(&self, id: &u32) -> Option<&WorkerMerge> {
        self.workers.get(id).map(|worker| &worker.merge)
    }

    /// Worker `id`.
    pub(crate) fn worker(&self, id: u32) -> Option<&WorkerHealth> {
        self.workers.get(&id)
    }

    /// Every worker, by id.
    pub(crate) fn workers(&self) -> impl Iterator<Item = &WorkerHealth> {
        self.workers.values()
    }

    /// Worker `id`, kept from now on, named `name` and with `cores`.
    pub(crate) fn join(&mut self, id: u32, name: &str, cores: Option<u32>) {
        let worker = self.entry(id);
        worker.name = name.chars().take(MAX_NAME).collect();
        worker.cores = cores;
        worker.state = WorkerState::Joining;
    }

    fn entry(&mut self, id: u32) -> &mut WorkerHealth {
        self.workers.entry(id).or_insert_with(|| WorkerHealth {
            id,
            state: WorkerState::Joining,
            ..WorkerHealth::default()
        })
    }

    /// The run's series second is `second`: events from now on are stamped
    /// with it.
    pub(crate) fn set_second(&mut self, second: u64) {
        self.second = second;
    }

    /// Worker `id` is now `state`, at `now`. A lost worker stays lost.
    pub(crate) fn set_state(&mut self, id: u32, state: WorkerState, now: Instant) {
        let worker = self.entry(id);
        if worker.state == WorkerState::Lost {
            return;
        }
        match state {
            WorkerState::Running => {
                worker.running_since.get_or_insert(now);
                worker.running_until = None;
            }
            WorkerState::Stopping
            | WorkerState::Finished
            | WorkerState::UsersExited
            | WorkerState::Lost => {
                if worker.running_since.is_some() {
                    worker.running_until.get_or_insert(now);
                }
            }
            WorkerState::Joining | WorkerState::Ready => {}
        }
        worker.state = state;
    }

    /// The run started at `now`: every worker ready runs its share.
    pub(crate) fn start(&mut self, now: Instant) {
        self.running_since = Some(now);
        let ready: Vec<u32> = self
            .workers
            .values()
            .filter(|worker| worker.state == WorkerState::Ready)
            .map(|worker| worker.id)
            .collect();
        for id in ready {
            self.set_state(id, WorkerState::Running, now);
        }
    }

    /// Worker `id` was lost in an earlier run, at `at`, for `why`.
    pub(crate) fn lost_before(&mut self, id: u32, at: chrono::DateTime<chrono::Utc>, why: &str) {
        let worker = self.entry(id);
        worker.lost = Some((at, 0, why.to_string()));
        worker.state = WorkerState::Lost;
    }

    /// Worker `id` runs `users` users now, and started `started` over the
    /// run.
    pub(crate) fn set_users(&mut self, id: u32, users: usize, started: usize) {
        let worker = self.entry(id);
        worker.users = users;
        worker.users_started = started;
    }

    /// Worker `id` was lost at `now` for `why`: an event says so. Whether it
    /// was newly lost.
    pub(crate) fn lose(&mut self, id: u32, why: &str, now: Instant) -> bool {
        let second = self.second;
        let worker = self.entry(id);
        if worker.state == WorkerState::Lost {
            return false;
        }
        worker.lost = Some((chrono::Utc::now(), second, why.to_string()));
        self.set_state(id, WorkerState::Lost, now);
        let name = self.entry(id).name.clone();
        self.event(
            GaggleEventKind::WorkerLost,
            Some(&name),
            format!("{name} lost: {why}"),
        );
        true
    }

    /// Worker `id`'s CPU time is `cpu_micros`, arrived at `now`: its CPU
    /// use is worked out against the sample before, once
    /// [`MIN_CPU_SPAN`] apart, as a percent of its cores. Absent, or without
    /// its cores, nothing is worked out.
    pub(crate) fn cpu(&mut self, id: u32, cpu_micros: Option<u64>, now: Instant) {
        let worker = self.entry(id);
        let (Some(micros), Some(cores)) = (cpu_micros, worker.cores) else {
            return;
        };
        let Some((at, before)) = worker.cpu_sample else {
            worker.cpu_sample = Some((now, micros));
            return;
        };
        let span = now.saturating_duration_since(at);
        if span < MIN_CPU_SPAN {
            return;
        }
        worker.cpu_sample = Some((now, micros));
        let used = micros.saturating_sub(before) as f64;
        let percent = used * 100.0 / (span.as_micros() as f64 * f64::from(cores));
        worker.cpu_percent = Some(percent);
        worker.cpu_peak = Some(worker.cpu_peak.map_or(percent, |peak| peak.max(percent)));
        if percent >= SATURATED_PERCENT {
            if worker.is_running() {
                worker.saturated_total += span;
            }
            // Saturated since the start of the span this sample covers.
            worker
                .above_since
                .get_or_insert_with(|| (at, chrono::Utc::now() - chrono::Duration::from_std(span).unwrap_or_default()));
        } else {
            worker.above_since = None;
        }
    }

    /// The merge took a delta from worker `id` at `now`. `sample` is the
    /// delta's requests, for the live window, `None` for a final delta,
    /// whose metrics are the whole epoch's, or a delta the merge ignored;
    /// `totals` its requests over the whole epoch, when the delta started
    /// them again (its first, a final, or a new epoch's), otherwise the
    /// sample adds to those held.
    pub(crate) fn applied(
        &mut self,
        id: u32,
        sample: Option<(Fold, u64)>,
        totals: Option<(Fold, u64)>,
        now: Instant,
    ) {
        let worker = self.entry(id);
        if worker.is_running() {
            if let Some(since) = worker.last_delta.or(worker.running_since) {
                worker.longest_gap = worker
                    .longest_gap
                    .max(now.saturating_duration_since(since));
            }
        }
        worker.last_delta = Some(now);
        match totals {
            Some((fold, epoch)) => {
                worker.totals = fold;
                worker.totals_epoch = Some(epoch);
            }
            None => {
                if let Some((fold, _)) = &sample {
                    worker.totals.add(fold);
                }
            }
        }
        if let Some((fold, secs)) = sample {
            worker.window.push_back(Sample {
                fold,
                secs: secs.max(1),
            });
            let mut held: u64 = worker.window.iter().map(|sample| sample.secs).sum();
            while held > WINDOW_SECS && worker.window.len() > 1 {
                if let Some(old) = worker.window.pop_front() {
                    held -= old.secs;
                }
            }
        }
    }

    /// The epoch worker `id`'s totals are of.
    pub(crate) fn totals_epoch(&self, id: u32) -> Option<u64> {
        self.workers.get(&id).and_then(|worker| worker.totals_epoch)
    }

    /// The merge's view of worker `id`, to write.
    pub(crate) fn merge_mut(&mut self, id: u32) -> &mut WorkerMerge {
        &mut self.entry(id).merge
    }

    /// The merge left out `excluded` workers, and no others: an event for
    /// each newly left out.
    pub(crate) fn excluded(&mut self, excluded: Vec<(u32, String)>) {
        let before: Vec<u32> = self
            .workers
            .values()
            .filter(|worker| worker.merge.excluded.is_some())
            .map(|worker| worker.id)
            .collect();
        for worker in self.workers.values_mut() {
            worker.merge.excluded = None;
        }
        for (id, why) in excluded {
            let name = self.entry(id).name.clone();
            if !before.contains(&id) {
                self.event(
                    GaggleEventKind::WorkerExcluded,
                    Some(&name),
                    format!("{name}'s metrics left out: {why}"),
                );
            }
            self.entry(id).merge.excluded = Some(why);
        }
    }

    /// A delta from worker `id` was refused for `why`: an event for the
    /// first, and then for every tenth, so a worker refused every second
    /// cannot fill the log.
    pub(crate) fn refused(&mut self, id: u32, why: String) {
        let worker = self.entry(id);
        worker.merge.refused += 1;
        let count = worker.merge.refused;
        worker.merge.refusal = Some(why.clone());
        let name = worker.name.clone();
        if count == 1 || count % 10 == 0 {
            let deltas = if count == 1 { "delta" } else { "deltas" };
            self.event(
                GaggleEventKind::DeltasRefused,
                Some(&name),
                format!("{name}: {count} {deltas} refused, the last for {why}"),
            );
        }
    }

    /// Log an event of `kind`, about `worker` when it is about one, saying
    /// `text`, stamped with the run's series second and the time now.
    pub(crate) fn event(&mut self, kind: GaggleEventKind, worker: Option<&str>, text: String) {
        if self.events.len() >= GaggleRecord::MAX_EVENTS {
            self.events.pop_front();
            self.events_dropped += 1;
        }
        self.events.push_back(GaggleEvent {
            second: self.second,
            time: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            kind,
            worker: worker.map(str::to_string),
            text,
        });
    }

    /// The events, oldest first, and how many older ones were dropped.
    pub(crate) fn events(&self) -> (impl Iterator<Item = &GaggleEvent>, u64) {
        (self.events.iter(), self.events_dropped)
    }

    /// Judge every worker's live flags as of `now`: an event when a worker
    /// turns saturated or lagging, and one when it recovers, once each way.
    pub(crate) fn tick(&mut self, now: Instant) {
        let outliers = self.outliers(now, false);
        let mut events = Vec::new();
        for worker in self.workers.values_mut() {
            if let Some(gap) = worker.delta_gap(now) {
                worker.longest_gap = worker.longest_gap.max(gap);
            }
            let mut live = worker.merge_flags();
            live.saturated = worker.is_running()
                && worker
                    .above_since
                    .is_some_and(|(since, _)| now.saturating_duration_since(since) >= SATURATED_FOR);
            live.lagging = worker.delta_gap(now).is_some_and(|gap| gap > LAGGING_AFTER);
            if let Some((rate, p95)) = outliers.get(&worker.id) {
                live.rate_outlier = *rate;
                live.p95_outlier = *p95;
            }
            let name = &worker.name;
            if live.saturated && !worker.live.saturated {
                let percent = worker.cpu_percent.unwrap_or(0.0).round();
                let secs = worker
                    .above_since
                    .map_or(0, |(since, _)| now.saturating_duration_since(since).as_secs());
                events.push((
                    GaggleEventKind::WorkerSaturated,
                    name.clone(),
                    format!("{name} saturated (CPU {percent}% for {secs} s)"),
                ));
            }
            if live.lagging && !worker.live.lagging {
                let secs = worker.delta_gap(now).unwrap_or_default().as_secs();
                events.push((
                    GaggleEventKind::WorkerLagging,
                    name.clone(),
                    format!("{name} lagging (last delta {secs} s ago)"),
                ));
            }
            // A worker that stopped or was lost is not said to recover.
            let recovered = |was: bool, is: bool| was && !is && worker.is_running();
            if recovered(worker.live.saturated, live.saturated) {
                events.push((
                    GaggleEventKind::WorkerRecovered,
                    name.clone(),
                    format!("{name} recovered: no longer saturated"),
                ));
            }
            if recovered(worker.live.lagging, live.lagging) {
                events.push((
                    GaggleEventKind::WorkerRecovered,
                    name.clone(),
                    format!("{name} recovered: no longer lagging"),
                ));
            }
            worker.live = live;
        }
        for (kind, name, text) in events {
            self.event(kind, Some(&name), text);
        }
    }

    /// Each running worker's distance from the median of the running
    /// workers, `(rate, p95)` in percent, each only when past
    /// [`OUTLIER_PERCENT`]: on the whole run when `run`, else on the live
    /// window. Judged among at least [`OUTLIER_MIN_WORKERS`] workers, once
    /// the run has run [`OUTLIER_AFTER`]; otherwise none.
    fn outliers(&self, now: Instant, run: bool) -> BTreeMap<u32, (Option<i32>, Option<i32>)> {
        let mut far = BTreeMap::new();
        let ran = self
            .running_since
            .is_some_and(|since| now.saturating_duration_since(since) >= OUTLIER_AFTER);
        if !ran {
            return far;
        }
        // For the run: every worker that ran its share and was not lost.
        let judged: Vec<(u32, Figures)> = self
            .workers
            .values()
            .filter(|worker| match run {
                true => worker.running_since.is_some() && worker.state != WorkerState::Lost,
                false => worker.is_running(),
            })
            .map(|worker| {
                let figures = match run {
                    true => worker.run_figures(now),
                    false => worker.window_figures(),
                };
                (worker.id, figures)
            })
            .collect();
        if judged.len() < OUTLIER_MIN_WORKERS {
            return far;
        }
        let rate = |figures: &Figures| figures.requests_per_user_per_second;
        let p95 = |figures: &Figures| figures.p95_ms;
        let rate_median = median(judged.iter().filter_map(|(_, figures)| rate(figures)));
        let p95_median = median(judged.iter().filter_map(|(_, figures)| p95(figures)));
        for (id, figures) in &judged {
            let from = |value: Option<f64>, median: Option<f64>| -> Option<i32> {
                let (value, median) = (value?, median?);
                if median <= 0.0 {
                    return None;
                }
                let percent = (value - median) * 100.0 / median;
                (percent.abs() > OUTLIER_PERCENT).then(|| percent.round() as i32)
            };
            let (r, p) = (from(rate(figures), rate_median), from(p95(figures), p95_median));
            if r.is_some() || p.is_some() {
                far.insert(*id, (r, p));
            }
        }
        far
    }

    /// Each worker's record as of `now`, with its figures and flags for the
    /// whole run, in id order.
    pub(crate) fn records(&self, now: Instant) -> Vec<GaggleWorkerRecord> {
        let outliers = self.outliers(now, true);
        self.workers
            .values()
            .map(|worker| {
                let ran = worker.running_since.is_some();
                let figures = worker.run_figures(now);
                let (rate_outlier, p95_outlier) =
                    outliers.get(&worker.id).copied().unwrap_or((None, None));
                let mut flags = Vec::new();
                let merge = worker.merge_flags();
                let lost = worker.state == WorkerState::Lost;
                let saturated = worker.saturated_total >= SATURATED_FOR;
                let lagging = worker.longest_gap > LAGGING_AFTER;
                for (flag, on) in [
                    (GaggleWorkerFlag::Lost, lost),
                    (GaggleWorkerFlag::Excluded, merge.excluded),
                    (GaggleWorkerFlag::Saturated, saturated),
                    (GaggleWorkerFlag::Lagging, lagging),
                    (
                        GaggleWorkerFlag::Outlier,
                        rate_outlier.is_some() || p95_outlier.is_some(),
                    ),
                    (GaggleWorkerFlag::Refused, merge.refused),
                    (GaggleWorkerFlag::Missing, merge.missing),
                ] {
                    if on {
                        flags.push(flag);
                    }
                }
                let times = (figures.requests > 0 || worker.totals.times.count > 0) && ran;
                GaggleWorkerRecord {
                    id: worker.id,
                    name: worker.name.clone(),
                    users: worker.users_started,
                    state: worker.state,
                    lost_at: worker
                        .lost
                        .as_ref()
                        .map(|(at, _, _)| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
                    lost_reason: worker.lost.as_ref().map(|(_, _, why)| why.clone()),
                    last_seq: worker.lost.as_ref().map(|_| worker.merge.last_seq),
                    refused_deltas: worker.merge.refused,
                    refused_reason: worker.merge.refusal.clone(),
                    excluded_reason: worker.merge.excluded.clone(),
                    missing_deltas: worker.merge.missing,
                    final_missing: worker.final_missing(),
                    lost_second: worker.lost.as_ref().map(|(_, second, _)| *second),
                    cores: worker.cores,
                    peak_cpu_percent: worker.cpu_peak.map(|peak| peak.round().max(0.0) as u32),
                    saturated_secs: worker
                        .cpu_peak
                        .map(|_| worker.saturated_total.as_secs_f64().round() as u64),
                    longest_delta_gap_secs: ran
                        .then(|| (worker.longest_gap.as_secs_f64() * 10.0).round() / 10.0),
                    running_secs: ran.then(|| {
                        (worker.running_for(now).as_secs_f64() * 10.0).round() / 10.0
                    }),
                    requests: ran.then_some(figures.requests),
                    failures: ran.then_some(figures.failures),
                    average_ms: figures.average_ms.filter(|_| times),
                    p50_ms: figures.p50_ms.filter(|_| times),
                    p95_ms: figures.p95_ms.filter(|_| times),
                    p99_ms: figures.p99_ms.filter(|_| times),
                    requests_per_second: figures.requests_per_second.filter(|_| ran),
                    requests_per_user_per_second: figures
                        .requests_per_user_per_second
                        .filter(|_| ran),
                    rate_vs_median_percent: rate_outlier,
                    p95_vs_median_percent: p95_outlier,
                    flags,
                }
            })
            .collect()
    }
}

/// The median of `values`, `None` without any.
fn median(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut values: Vec<f64> = values.filter(|value| value.is_finite()).collect();
    if values.is_empty() {
        return None;
    }
    values.sort_unstable_by(f64::total_cmp);
    let middle = values.len() / 2;
    Some(match values.len() % 2 {
        0 => (values[middle - 1] + values[middle]) / 2.0,
        _ => values[middle],
    })
}

/// A delta's requests, for [`GaggleHealth::applied`]: its requests and the
/// seconds it covers, `None` for a final delta, whose requests are the
/// whole epoch's.
pub(crate) fn sample_of(delta: &GaggleDelta) -> Option<(Fold, u64)> {
    match delta.kind {
        super::delta::DeltaKind::Final => None,
        super::delta::DeltaKind::Delta => Some((
            Fold::of(delta.metrics.requests.values()),
            delta.seq.saturating_sub(delta.first_seq) + 1,
        )),
    }
}

/// This process's CPU time so far, user and system, in microseconds: `None`
/// where the platform has no cheap source of it. Read on a worker as each
/// delta is sent, off the user threads.
pub(crate) fn process_cpu_micros() -> Option<u64> {
    #[cfg(unix)]
    {
        use std::convert::TryFrom;
        // SAFETY: getrusage writes the struct it is given and reads
        // nothing else; a zeroed rusage is a valid one to hand it.
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
            return None;
        }
        let micros = |time: libc::timeval| -> Option<u64> {
            let secs = u64::try_from(time.tv_sec).ok()?;
            let micros = u64::try_from(time.tv_usec).ok()?;
            secs.checked_mul(1_000_000)?.checked_add(micros)
        };
        micros(usage.ru_utime)?.checked_add(micros(usage.ru_stime)?)
    }
    #[cfg(not(unix))]
    {
        None
    }
}
