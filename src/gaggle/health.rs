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

/// The longest span, on a worker's own clock, a CPU sample is taken against
/// the one before: a worker that sent none for longer has lost its baseline,
/// as a percent averaged over more than the live window says nothing of it.
const MAX_CPU_SPAN: Duration = Duration::from_secs(WINDOW_SECS);

/// The longest event text kept, in characters: a sentence with a worker's
/// reason in it, which the worker could make as long as it likes.
const MAX_EVENT_TEXT: usize = 500;

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
    /// Every request name of `requests` together: each name's buckets are
    /// counted into one array indexed by bucket, so the work is the buckets
    /// given and the span of buckets they reach, never a sort.
    pub(crate) fn of<'a>(requests: impl IntoIterator<Item = &'a RequestPart>) -> Self {
        let mut fold = Fold::default();
        SCRATCH.with(|scratch| {
            let mut counts = scratch.borrow_mut();
            let (mut low, mut high) = (usize::MAX, 0);
            for request in requests {
                fold.success = fold.success.saturating_add(request.success);
                fold.fail = fold.fail.saturating_add(request.fail);
                add_counts(&mut fold.times, &request.raw);
                for &(bucket, count) in &request.raw.buckets {
                    let index = buckets::bucket_index(bucket);
                    counts[index] = counts[index].saturating_add(count);
                    low = low.min(index);
                    high = high.max(index);
                }
            }
            if low <= high {
                let held = counts[low..=high]
                    .iter()
                    .filter(|&&count| count > 0)
                    .count();
                let mut buckets = Vec::with_capacity(held);
                for (index, count) in counts[low..=high].iter_mut().enumerate() {
                    if *count > 0 {
                        buckets.push((buckets::bucket_at(low + index), *count));
                        *count = 0;
                    }
                }
                fold.times.buckets = buckets;
            }
        });
        fold
    }

    /// Add `other`'s requests: both bucket lists are sorted, so they merge
    /// in one pass.
    fn add(&mut self, other: &Fold) {
        self.success = self.success.saturating_add(other.success);
        self.fail = self.fail.saturating_add(other.fail);
        add_counts(&mut self.times, &other.times);
        self.times.buckets = merge_sorted(&self.times.buckets, &other.times.buckets);
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

/// The most buckets a time can fall in: [`buckets::bucket_index`] of the
/// longest time, `u64::MAX` microseconds, is below it.
const BUCKETS: usize = 3_840;

std::thread_local! {
    /// A count for every bucket, all 0 between folds: [`Fold::of`] counts
    /// a delta's buckets into it, 30 KB a thread, once.
    static SCRATCH: std::cell::RefCell<Vec<u64>> = std::cell::RefCell::new(vec![0; BUCKETS]);
}

/// Add `from`'s count, total and extremes to `to`; its buckets are the
/// caller's.
fn add_counts(to: &mut Timing, from: &Timing) {
    if from.count == 0 {
        return;
    }
    if to.count == 0 || from.minimum_us < to.minimum_us {
        to.minimum_us = from.minimum_us;
    }
    to.maximum_us = to.maximum_us.max(from.maximum_us);
    to.total_us = to.total_us.saturating_add(from.total_us);
    to.count = to.count.saturating_add(from.count);
}

/// Two sorted bucket lists as one, each bucket once, its counts added.
fn merge_sorted(a: &[(u64, u64)], b: &[(u64, u64)]) -> Vec<(u64, u64)> {
    let mut merged = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].0.cmp(&b[j].0) {
            std::cmp::Ordering::Less => {
                merged.push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                merged.push(b[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                merged.push((a[i].0, a[i].1.saturating_add(b[j].1)));
                i += 1;
                j += 1;
            }
        }
    }
    merged.extend_from_slice(&a[i..]);
    merged.extend_from_slice(&b[j..]);
    merged.shrink_to_fit();
    merged
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
            fail_percent: (requests > 0).then(|| fold.failures() as f64 * 100.0 / requests as f64),
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

/// One CPU time sample from a worker.
#[derive(Clone, Copy, Debug)]
struct CpuSample {
    /// When it arrived.
    at: Instant,
    /// The worker's clock as it read the CPU time, in microseconds, when it
    /// sent it.
    wall_micros: Option<u64>,
    /// The worker's CPU time, in microseconds.
    micros: u64,
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
    /// Its last CPU time sample.
    cpu_sample: Option<CpuSample>,
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
    /// How long the current saturation has lasted, over the spans of the
    /// samples it covers: zero while it is not saturated.
    saturated_run: Duration,
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
    /// When its totals' epoch began, when a metrics reset started them
    /// again during the run: its run rates are over the time since.
    totals_since: Option<Instant>,
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
            Some(since) => self
                .running_until
                .unwrap_or(now)
                .saturating_duration_since(since),
            None => Duration::ZERO,
        }
    }

    /// Its figures over the whole run, as of `now`.
    pub(crate) fn run_figures(&self, now: Instant) -> Figures {
        let users = self.users_started.max(self.users);
        let secs = match (self.running_since, self.totals_since) {
            (Some(since), Some(reset)) => self
                .running_until
                .unwrap_or(now)
                .saturating_duration_since(since.max(reset)),
            _ => self.running_for(now),
        };
        Figures::of(&self.totals, secs.as_secs_f64(), users)
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

    /// How long its CPU use has stayed at or above [`SATURATED_PERCENT`],
    /// over the spans of its samples since, on its own clock when they carry
    /// it: a worker that goes quiet is not taken as saturated for want of
    /// samples.
    pub(crate) fn saturated_for(&self) -> Duration {
        match self.above_since {
            Some(_) => self.saturated_run,
            None => Duration::ZERO,
        }
    }

    /// Nothing says it is still saturated.
    fn unsaturate(&mut self) {
        self.above_since = None;
        self.saturated_run = Duration::ZERO;
    }

    /// Whether it runs its share.
    pub(crate) fn is_running(&self) -> bool {
        self.state == WorkerState::Running
    }

    /// How long since its last delta was merged while running, as of `now`:
    /// since it started running, before its first.
    pub(crate) fn delta_gap(&self, now: Instant) -> Option<Duration> {
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
        matches!(self.state, WorkerState::Finished | WorkerState::UsersExited)
            && self.merge.last_seq > 0
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

    /// The merge's view of every worker (tests).
    #[cfg(test)]
    pub(crate) fn values(&self) -> impl Iterator<Item = &WorkerMerge> {
        self.workers.values().map(|worker| &worker.merge)
    }

    /// Worker `id` (tests).
    #[cfg(test)]
    pub(crate) fn worker(&self, id: u32) -> Option<&WorkerHealth> {
        self.workers.get(&id)
    }

    /// Every worker, in id order: what the dashboard shows of each.
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
        worker.lost = Some((at, 0, bounded(why)));
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
        // A worker's own words can reach `why`: kept bounded, as an event's
        // text is, as the dashboard sends it every second.
        worker.lost = Some((chrono::Utc::now(), second, bounded(why)));
        self.set_state(id, WorkerState::Lost, now);
        let name = self.entry(id).name.clone();
        self.event(
            GaggleEventKind::WorkerLost,
            Some(&name),
            lost_text(&name, why),
        );
        true
    }

    /// Worker `id`'s CPU time is `cpu_micros`, read at `wall_micros` on its
    /// own clock and arrived at `now`: its CPU use is worked out against the
    /// sample before, once [`MIN_CPU_SPAN`] apart, as a percent of its
    /// cores. The span between the two is the worker's own when both carry
    /// its clock, so a delta that arrives late or early reads the same CPU,
    /// and otherwise the time between their arrivals. Absent, or without its
    /// cores, nothing is worked out.
    pub(crate) fn cpu(
        &mut self,
        id: u32,
        cpu_micros: Option<u64>,
        wall_micros: Option<u64>,
        now: Instant,
    ) {
        let worker = self.entry(id);
        let (Some(micros), Some(cores)) = (cpu_micros, worker.cores) else {
            // No CPU time from it now: nothing says it is still saturated.
            worker.unsaturate();
            return;
        };
        let sample = CpuSample {
            at: now,
            wall_micros,
            micros,
        };
        let Some(before) = worker.cpu_sample else {
            worker.cpu_sample = Some(sample);
            return;
        };
        let span = match (before.wall_micros, wall_micros) {
            (Some(was), Some(is)) => {
                let span = Duration::from_micros(is.saturating_sub(was));
                // A clock that did not move on, as a restarted worker's,
                // or one that jumped past the longest span, gives no
                // baseline: start again from this sample, and as nothing
                // then says it is still saturated, a saturation ends.
                if is <= was || span > MAX_CPU_SPAN {
                    worker.cpu_sample = Some(sample);
                    worker.unsaturate();
                    return;
                }
                span
            }
            _ => now.saturating_duration_since(before.at),
        };
        if span < MIN_CPU_SPAN {
            return;
        }
        worker.cpu_sample = Some(sample);
        let used = micros.saturating_sub(before.micros) as f64;
        // A worker's CPU time and clock are untrusted, so the percent never
        // passes 100.
        let percent = (used * 100.0 / (span.as_micros() as f64 * f64::from(cores))).min(100.0);
        worker.cpu_percent = Some(percent);
        worker.cpu_peak = Some(worker.cpu_peak.map_or(percent, |peak| peak.max(percent)));
        if percent >= SATURATED_PERCENT {
            if worker.is_running() {
                worker.saturated_total += span;
            }
            worker.saturated_run += span;
            // Saturated since the start of the span this sample covers.
            worker.above_since.get_or_insert_with(|| {
                (
                    now.checked_sub(span).unwrap_or(now),
                    chrono::Utc::now() - chrono::Duration::from_std(span).unwrap_or_default(),
                )
            });
        } else {
            worker.unsaturate();
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
                worker.longest_gap = worker.longest_gap.max(now.saturating_duration_since(since));
            }
        }
        worker.last_delta = Some(now);
        match totals {
            Some((fold, epoch)) => {
                if worker.totals_epoch.is_some_and(|held| held != epoch) {
                    // A metrics reset: its totals, rates and live window
                    // start again, from the start of this delta's span.
                    let secs = sample.as_ref().map_or(0, |(_, secs)| *secs);
                    worker.totals_since =
                        Some(now.checked_sub(Duration::from_secs(secs)).unwrap_or(now));
                    worker.window.clear();
                }
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

    /// The epoch worker `id`'s totals are of (tests).
    #[cfg(test)]
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
        let before: std::collections::BTreeSet<u32> = self
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
                    excluded_text(&name, &why),
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
        if count == 1 || count.is_multiple_of(10) {
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
        // A worker's words reach the text, through the reasons it gives:
        // no control character, so none reaches a terminal or a report, and
        // at most MAX_EVENT_TEXT characters.
        let text = bounded(&text);
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
            live.saturated = worker.is_running() && worker.saturated_for() >= SATURATED_FOR;
            live.lagging = worker.delta_gap(now).is_some_and(|gap| gap > LAGGING_AFTER);
            if let Some((rate, p95)) = outliers.get(&worker.id) {
                live.rate_outlier = *rate;
                live.p95_outlier = *p95;
            }
            let name = &worker.name;
            if live.saturated && !worker.live.saturated {
                events.push((
                    GaggleEventKind::WorkerSaturated,
                    name.clone(),
                    saturated_text(name, worker.cpu_percent, worker.saturated_for()),
                ));
            }
            if live.lagging && !worker.live.lagging {
                events.push((
                    GaggleEventKind::WorkerLagging,
                    name.clone(),
                    lagging_text(name, worker.delta_gap(now).unwrap_or_default()),
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
            let (r, p) = (
                from(rate(figures), rate_median),
                from(p95(figures), p95_median),
            );
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
                    running_secs: ran
                        .then(|| (worker.running_for(now).as_secs_f64() * 10.0).round() / 10.0),
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

/// `text` with no control character, so none reaches a terminal or a
/// report, and at most [`MAX_EVENT_TEXT`] characters: a worker's words can
/// reach it, through the reasons it gives.
fn bounded(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_EVENT_TEXT)
        .collect()
}

/// What an event says of a worker that turned saturated, and what the
/// dashboard's attention banner says of one while it is: `load-17 saturated
/// (CPU 97% for 42 s)`, its CPU use now and how long it has stayed high.
pub(crate) fn saturated_text(name: &str, cpu_percent: Option<f64>, for_: Duration) -> String {
    let percent = cpu_percent.unwrap_or(0.0).round();
    let secs = for_.as_secs();
    format!("{name} saturated (CPU {percent}% for {secs} s)")
}

/// What an event says of a worker that turned lagging, and the banner of
/// one while it is: `load-42 lagging (last delta 5 s ago)`.
pub(crate) fn lagging_text(name: &str, gap: Duration) -> String {
    let secs = gap.as_secs();
    format!("{name} lagging (last delta {secs} s ago)")
}

/// What an event says of a worker lost: `load-31 lost: connection reset`.
pub(crate) fn lost_text(name: &str, why: &str) -> String {
    format!("{name} lost: {why}")
}

/// What the banner says of a worker lost, with the run's series second it
/// was lost in as `mm:ss`: `load-31 lost at 03:05 (connection reset)`.
pub(crate) fn lost_at_text(name: &str, second: u64, why: &str) -> String {
    format!("{name} lost at {} ({why})", minutes_seconds(second))
}

/// What an event says, and the banner, of a worker the merge left out:
/// `load-9's metrics left out: why`.
pub(crate) fn excluded_text(name: &str, why: &str) -> String {
    format!("{name}'s metrics left out: {why}")
}

/// `second` as `mm:ss`, minutes past 99 in full: `03:05`, `125:00`.
pub(crate) fn minutes_seconds(second: u64) -> String {
    format!("{:02}:{:02}", second / 60, second % 60)
}

/// The median of `values`, `None` with fewer than
/// [`OUTLIER_MIN_WORKERS`]: with two, each is as far from it as the other.
fn median(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut values: Vec<f64> = values.filter(|value| value.is_finite()).collect();
    if values.len() < OUTLIER_MIN_WORKERS {
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
            delta.seq.saturating_sub(delta.first_seq).saturating_add(1),
        )),
    }
}

/// This process's monotonic clock, in microseconds since a fixed point in
/// it: read on a worker with its CPU time, so the manager times the CPU over
/// the worker's own span.
pub(crate) fn wall_micros() -> u64 {
    wall_micros_at(Instant::now())
}

/// [`wall_micros`] for an instant this process read.
pub(crate) fn wall_micros_at(at: Instant) -> u64 {
    static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let origin = *ORIGIN.get_or_init(Instant::now);
    at.saturating_duration_since(origin).as_micros() as u64
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` successful requests, each taking `ms` milliseconds.
    fn fold(n: u64, ms: u64) -> Fold {
        let us = ms * 1_000;
        Fold {
            success: n as i64,
            fail: 0,
            times: Timing {
                buckets: if n > 0 {
                    vec![(buckets::bucket_floor(us), n)]
                } else {
                    Vec::new()
                },
                minimum_us: us,
                maximum_us: us,
                total_us: us * n,
                count: n,
            },
        }
    }

    /// A model of `workers` workers, each with `cores`, running since `at`.
    fn running(workers: u32, cores: Option<u32>, at: Instant) -> GaggleHealth {
        let mut health = GaggleHealth::default();
        for id in 0..workers {
            health.join(id, &format!("load-{id}"), cores);
            health.set_state(id, WorkerState::Ready, at);
            health.set_users(id, 10, 10);
        }
        health.start(at);
        health
    }

    fn secs(n: f64) -> Duration {
        Duration::from_secs_f64(n)
    }

    fn kinds(health: &GaggleHealth) -> Vec<GaggleEventKind> {
        health.events().0.map(|event| event.kind).collect()
    }

    fn record(health: &GaggleHealth, id: u32, now: Instant) -> GaggleWorkerRecord {
        health
            .records(now)
            .into_iter()
            .find(|record| record.id == id)
            .unwrap()
    }

    #[test]
    fn every_state_transition() {
        let at = Instant::now();
        let mut health = GaggleHealth::default();
        health.join(0, "a", Some(2));
        assert_eq!(health.worker(0).unwrap().state, WorkerState::Joining);
        // The run starts only workers that said they are ready.
        health.join(1, "b", Some(2));
        health.set_state(0, WorkerState::Ready, at);
        health.start(at);
        assert_eq!(health.worker(0).unwrap().state, WorkerState::Running);
        assert_eq!(health.worker(1).unwrap().state, WorkerState::Joining);
        for state in [WorkerState::Stopping, WorkerState::Finished] {
            health.set_state(0, state, at + secs(2.0));
            assert_eq!(health.worker(0).unwrap().state, state);
        }
        health.set_state(1, WorkerState::UsersExited, at);
        assert_eq!(health.worker(1).unwrap().state, WorkerState::UsersExited);
        // Lost stays lost, and is said once.
        assert!(health.lose(1, "connection reset", at));
        assert!(!health.lose(1, "again", at));
        health.set_state(1, WorkerState::Running, at);
        assert_eq!(health.worker(1).unwrap().state, WorkerState::Lost);
        assert_eq!(kinds(&health), vec![GaggleEventKind::WorkerLost]);
        assert_eq!(
            health.events().0.next().unwrap().text,
            "b lost: connection reset"
        );
        // Its running time stops when it stops.
        assert_eq!(record(&health, 0, at + secs(10.0)).running_secs, Some(2.0));
    }

    #[test]
    fn lagging_is_judged_past_three_seconds() {
        let at = Instant::now();
        let mut health = running(1, Some(1), at);
        health.applied(0, Some((fold(10, 5), 1)), None, at + secs(1.0));
        // Exactly 3 seconds is not lagging; just over is.
        health.tick(at + secs(4.0));
        assert!(!health.worker(0).unwrap().live.lagging);
        health.tick(at + secs(4.01));
        assert!(health.worker(0).unwrap().live.lagging);
        // Once, then recovered once.
        health.tick(at + secs(5.0));
        health.applied(0, Some((fold(10, 5), 1)), None, at + secs(5.5));
        health.tick(at + secs(5.6));
        health.tick(at + secs(6.0));
        assert_eq!(
            kinds(&health),
            vec![
                GaggleEventKind::WorkerLagging,
                GaggleEventKind::WorkerRecovered
            ]
        );
        assert!(health
            .events()
            .0
            .next()
            .unwrap()
            .text
            .starts_with("load-0 lagging (last delta 3 s ago)"));
        // The run form: the longest gap, 4.5 seconds, is past 3.
        let lagged = record(&health, 0, at + secs(6.0));
        assert!(lagged.flags.contains(&GaggleWorkerFlag::Lagging));
        assert_eq!(lagged.longest_delta_gap_secs, Some(4.5));
        // A longest gap of exactly 3 seconds is not lagging for the run.
        let mut health = running(1, Some(1), at);
        for second in 1..=4 {
            health.applied(0, None, None, at + secs(3.0 * second as f64));
            health.tick(at + secs(3.0 * second as f64));
        }
        let steady = record(&health, 0, at + secs(12.0));
        assert_eq!(steady.longest_delta_gap_secs, Some(3.0));
        assert!(!steady.flags.contains(&GaggleWorkerFlag::Lagging));
        // A stopped worker is never lagging.
        health.set_state(0, WorkerState::Finished, at + secs(12.0));
        health.tick(at + secs(60.0));
        assert!(!health.worker(0).unwrap().live.lagging);
    }

    /// Feed worker 0 of `health` CPU at `percent` of its one core every
    /// second from `from` to `to` seconds past `at`, from CPU time `micros`;
    /// the CPU time after.
    fn burn(
        health: &mut GaggleHealth,
        at: Instant,
        from: u64,
        to: u64,
        percent: u64,
        mut micros: u64,
    ) -> u64 {
        for second in from..=to {
            health.applied(0, None, None, at + secs(second as f64));
            health.cpu(0, Some(micros), None, at + secs(second as f64));
            health.tick(at + secs(second as f64));
            micros += percent * 10_000;
        }
        micros
    }

    #[test]
    fn saturation_is_judged_over_five_seconds() {
        let at = Instant::now();
        let mut health = running(1, Some(1), at);
        // 95% from second 0: saturated from the span starting at 0, so at 5.
        let micros = burn(&mut health, at, 0, 4, 95, 0);
        assert!(!health.worker(0).unwrap().live.saturated, "4 s is not 5");
        let since = health.worker(0).unwrap().above_since.unwrap().0;
        assert_eq!(since, at);
        let micros = burn(&mut health, at, 5, 6, 95, micros);
        assert!(health.worker(0).unwrap().live.saturated);
        // 90% exactly still counts; just under recovers.
        let micros = burn(&mut health, at, 7, 8, 90, micros);
        assert!(health.worker(0).unwrap().live.saturated);
        let _ = burn(&mut health, at, 9, 10, 89, micros);
        assert!(!health.worker(0).unwrap().live.saturated);
        assert_eq!(
            kinds(&health),
            vec![
                GaggleEventKind::WorkerSaturated,
                GaggleEventKind::WorkerRecovered
            ]
        );
        let saturated = health.events().0.next().unwrap().text.clone();
        assert_eq!(saturated, "load-0 saturated (CPU 95% for 5 s)");
        let done = record(&health, 0, at + secs(10.0));
        assert_eq!(done.peak_cpu_percent, Some(95));
        assert_eq!(done.saturated_secs, Some(9));
        assert!(done.flags.contains(&GaggleWorkerFlag::Saturated));
        // For the run: 5 seconds in total at or above, never 5 in a row, is
        // saturated; 4 is not.
        for (seconds, flagged) in [(4, false), (5, true)] {
            let mut health = running(1, Some(1), at);
            let mut micros = 0;
            health.cpu(0, Some(micros), None, at);
            for second in 1..=seconds * 2 {
                micros += if second % 2 == 1 { 950_000 } else { 100_000 };
                health.applied(0, None, None, at + secs(second as f64));
                health.cpu(0, Some(micros), None, at + secs(second as f64));
                health.tick(at + secs(second as f64));
            }
            assert!(!health.worker(0).unwrap().live.saturated);
            let ran = record(&health, 0, at + secs(20.0));
            assert_eq!(
                ran.flags.contains(&GaggleWorkerFlag::Saturated),
                flagged,
                "{seconds} s: {ran:?}"
            );
            assert!(kinds(&health).is_empty());
        }
    }

    #[test]
    fn a_worker_whose_cpu_samples_stop_is_not_taken_as_saturated() {
        let at = Instant::now();
        let mut health = running(1, Some(1), at);
        // 95% for two seconds, then its deltas go on without CPU samples.
        let _ = burn(&mut health, at, 0, 2, 95, 0);
        for second in 3..=10 {
            health.applied(0, None, None, at + secs(second as f64));
            health.tick(at + secs(second as f64));
        }
        assert!(!health.worker(0).unwrap().live.saturated);
        assert!(kinds(&health).is_empty(), "{:?}", kinds(&health));
        let ran = record(&health, 0, at + secs(10.0));
        assert!(
            !ran.flags.contains(&GaggleWorkerFlag::Saturated),
            "{:?}",
            ran
        );
    }

    #[test]
    fn cpu_is_a_percent_of_the_cores_and_absent_without_them() {
        let at = Instant::now();
        let mut health = running(1, Some(4), at);
        health.cpu(0, Some(0), None, at);
        // Two cores of four busy.
        health.cpu(0, Some(2_000_000), None, at + secs(1.0));
        assert_eq!(health.worker(0).unwrap().cpu_percent, Some(50.0));
        // Samples closer than the span are not taken against each other.
        health.cpu(0, Some(3_000_000), None, at + secs(1.1));
        assert_eq!(health.worker(0).unwrap().cpu_percent, Some(50.0));
        // A late sample, or an untrusted CPU time, is never past 100%.
        health.cpu(0, Some(u64::MAX), None, at + secs(2.0));
        assert_eq!(health.worker(0).unwrap().cpu_percent, Some(100.0));
        // Without CPU time, or without cores, nothing is worked out.
        for cores in [Some(4), None] {
            let mut health = running(1, cores, at);
            let cpu = |second: u64| (cores.is_none()).then_some(second * 1_000_000);
            for second in 0..10 {
                health.cpu(0, cpu(second), None, at + secs(second as f64));
                health.tick(at + secs(second as f64));
            }
            let worker = health.worker(0).unwrap();
            assert_eq!(worker.cpu_percent, None);
            let ran = record(&health, 0, at + secs(10.0));
            assert_eq!((ran.peak_cpu_percent, ran.saturated_secs), (None, None));
        }
    }

    /// How much later than the one before each of a worker's samples
    /// arrives, in milliseconds, in turn: a late one, one soon after it, and
    /// one a little early. The samples are a second apart on the worker.
    const LATE: [u64; 3] = [3_000, 100, 920];

    /// Feed worker 0 of `health` `samples` CPU samples a second apart on its
    /// clock at `percent` of its one core, the first at `at`, each arriving
    /// as [`LATE`] says, the clock sent when `clock`; after each the CPU
    /// percent, `None` before one is worked out.
    fn late(
        health: &mut GaggleHealth,
        at: Instant,
        samples: u64,
        percent: u64,
        clock: bool,
    ) -> Vec<Option<f64>> {
        let mut arrived = at;
        let mut read = Vec::new();
        for sample in 0..samples {
            if sample > 0 {
                arrived += Duration::from_millis(LATE[(sample as usize - 1) % LATE.len()]);
            }
            let wall = clock.then_some(sample * 1_000_000);
            health.applied(0, None, None, arrived);
            health.cpu(0, Some(sample * percent * 10_000), wall, arrived);
            health.tick(arrived);
            read.push(health.worker(0).unwrap().cpu_percent);
        }
        read
    }

    #[test]
    fn cpu_is_timed_by_the_workers_clock() {
        let at = Instant::now();
        let mut health = running(1, Some(1), at);
        let read = late(&mut health, at, 10, 95, true);
        // A percent from the second sample on, each 95 however late it came.
        assert_eq!(read[0], None);
        for percent in &read[1..] {
            let percent = percent.unwrap();
            assert!((percent - 95.0).abs() < 1.0, "{:?}", read);
        }
        assert_eq!(health.worker(0).unwrap().cpu_peak, Some(95.0));
    }

    #[test]
    fn cpu_without_the_workers_clock_is_timed_by_arrival() {
        let at = Instant::now();
        let mut health = running(1, Some(1), at);
        let read = late(&mut health, at, 4, 95, false);
        // A second's CPU over the 3 s it took to arrive reads low; one soon
        // after waits for a longer span; two seconds' CPU over the 1.02 s
        // since the late one is clamped.
        assert_eq!(read[0], None);
        assert!((read[1].unwrap() - 95.0 / 3.0).abs() < 0.01, "{:?}", read);
        assert_eq!(read[2], read[1]);
        assert_eq!(read[3], Some(100.0));
    }

    #[test]
    fn a_clock_that_goes_back_or_jumps_starts_again() {
        let at = Instant::now();
        let mut health = running(1, Some(1), at);
        let percent = |health: &GaggleHealth| health.worker(0).unwrap().cpu_percent;
        health.cpu(0, Some(0), Some(0), at);
        health.cpu(0, Some(500_000), Some(1_000_000), at + secs(1.0));
        assert_eq!(percent(&health), Some(50.0));
        // Back in time, then still: no percent, but each starts again.
        health.cpu(0, Some(1_400_000), Some(500_000), at + secs(2.0));
        assert_eq!(percent(&health), Some(50.0));
        health.cpu(0, Some(2_000_000), Some(500_000), at + secs(3.0));
        assert_eq!(percent(&health), Some(50.0));
        health.cpu(0, Some(2_900_000), Some(1_500_000), at + secs(4.0));
        assert_eq!(percent(&health), Some(90.0));
        // Past the longest span: no percent, and it starts again.
        let jumped = 1_500_000 + MAX_CPU_SPAN.as_micros() as u64 + 1;
        health.cpu(0, Some(4_000_000), Some(jumped), at + secs(5.0));
        assert_eq!(percent(&health), Some(90.0));
        health.cpu(0, Some(4_700_000), Some(jumped + 1_000_000), at + secs(6.0));
        assert_eq!(percent(&health), Some(70.0));
        assert_eq!(health.worker(0).unwrap().cpu_peak, Some(90.0));
        // A span of exactly the longest is still taken.
        let longest = MAX_CPU_SPAN.as_micros() as u64;
        let at_most = jumped + 1_000_000 + longest;
        health.cpu(
            0,
            Some(4_700_000 + longest / 2),
            Some(at_most),
            at + secs(7.0),
        );
        assert_eq!(percent(&health), Some(50.0));
    }

    #[test]
    fn starting_again_ends_a_saturation() {
        let at = Instant::now();
        let mut health = running(1, Some(1), at);
        let _ = late(&mut health, at, 6, 95, true);
        assert!(health.worker(0).unwrap().above_since.is_some());
        // The clock goes back: nothing says it is still saturated.
        health.cpu(0, Some(6_000_000), Some(1_000_000), at + secs(20.0));
        let worker = health.worker(0).unwrap();
        assert!(worker.above_since.is_none());
        assert_eq!(worker.saturated_for(), Duration::ZERO);
    }

    #[test]
    fn a_sample_without_the_clock_is_timed_by_arrival() {
        let at = Instant::now();
        let percent = |health: &GaggleHealth| health.worker(0).unwrap().cpu_percent;
        // The sample before carries the clock and this one does not, then
        // the other way round: each pair is timed by its arrival, 2 s apart,
        // not by the 1 s the clock would say.
        for (before, after) in [(Some(0), None), (None, Some(1_000_000))] {
            let mut health = running(1, Some(1), at);
            health.cpu(0, Some(0), before, at);
            health.cpu(0, Some(1_000_000), after, at + secs(2.0));
            assert_eq!(percent(&health), Some(50.0));
        }
    }

    #[test]
    fn saturation_fed_late_samples_is_the_workers_time() {
        let at = Instant::now();
        let mut health = running(1, Some(1), at);
        // Nine samples at 95%: eight seconds saturated on the worker's clock,
        // which took over eleven seconds to arrive.
        let _ = late(&mut health, at, 9, 95, true);
        let worker = health.worker(0).unwrap();
        assert_eq!(worker.saturated_total, secs(8.0));
        assert_eq!(worker.saturated_for(), secs(8.0));
        // Saturated from a second, on its clock, before the first late
        // sample arrived.
        let first = at + Duration::from_millis(LATE[0]);
        assert_eq!(worker.above_since.unwrap().0, first - secs(1.0));
        assert!(worker.live.saturated);
        let saturated = health.events().0.next().unwrap().text.clone();
        assert_eq!(saturated, "load-0 saturated (CPU 95% for 5 s)");
        let ran = record(&health, 0, at + secs(20.0));
        assert_eq!(ran.saturated_secs, Some(8));
        assert!(ran.flags.contains(&GaggleWorkerFlag::Saturated));
    }

    /// Workers 0 to `n - 1`, each with `rates[id]` requests a second at
    /// `p95s[id]` milliseconds, for `seconds` seconds.
    fn outlying(n: u32, rates: &[u64], ms: &[u64], seconds: u64, at: Instant) -> GaggleHealth {
        let mut health = running(n, Some(1), at);
        for second in 1..=seconds {
            let now = at + secs(second as f64);
            for id in 0..n {
                let sample = fold(rates[id as usize], ms[id as usize]);
                health.applied(id, Some((sample, 1)), None, now);
            }
            health.tick(now);
        }
        health
    }

    #[test]
    fn outliers_need_three_workers_and_thirty_seconds() {
        let at = Instant::now();
        // Worker 0 makes 35% fewer requests, at a 40% higher p95.
        let rates = [65, 100, 100, 100];
        let ms = [140, 100, 100, 100];
        // Two workers: never judged.
        let two = outlying(2, &rates, &ms, 40, at);
        assert!(two
            .records(at + secs(40.0))
            .iter()
            .all(|r| r.flags.is_empty()));
        // Three: not before 30 seconds, then flagged, live and for the run.
        let early = outlying(3, &rates, &ms, 29, at);
        assert_eq!(early.worker(0).unwrap().live.rate_outlier, None);
        assert!(early
            .records(at + secs(29.0))
            .iter()
            .all(|r| r.flags.is_empty()));
        for n in [3, 4] {
            let late = outlying(n, &rates, &ms, 31, at);
            let live = &late.worker(0).unwrap().live;
            assert_eq!((live.rate_outlier, live.p95_outlier), (Some(-35), Some(40)));
            let worker = record(&late, 0, at + secs(31.0));
            assert!(worker.flags.contains(&GaggleWorkerFlag::Outlier));
            assert_eq!(worker.rate_vs_median_percent, Some(-35));
            assert_eq!(worker.p95_vs_median_percent, Some(40));
            for id in 1..n {
                assert!(record(&late, id, at + secs(31.0)).flags.is_empty());
            }
        }
        // 25% from the median is not past it; 26% is.
        for (rate, flagged) in [(75, false), (74, true)] {
            let health = outlying(3, &[rate, 100, 100], &[100, 100, 100], 31, at);
            let worker = record(&health, 0, at + secs(31.0));
            assert_eq!(
                worker.flags.contains(&GaggleWorkerFlag::Outlier),
                flagged,
                "{rate}"
            );
        }
    }

    /// Three workers run, but only two have a rate, as one runs no users:
    /// two values are never judged against their own midpoint.
    #[test]
    fn outliers_need_three_values_not_only_three_workers() {
        let at = Instant::now();
        let mut health = running(3, Some(1), at);
        health.set_users(2, 0, 0);
        for second in 1..=31 {
            let now = at + secs(second as f64);
            health.applied(0, Some((fold(100, 100), 1)), None, now);
            health.applied(1, Some((fold(200, 100), 1)), None, now);
            health.tick(now);
        }
        for id in 0..3 {
            let ran = record(&health, id, at + secs(31.0));
            assert!(!ran.flags.contains(&GaggleWorkerFlag::Outlier), "{:?}", ran);
        }
    }

    #[test]
    fn the_window_holds_ten_seconds_across_gaps_and_coalesced_deltas() {
        let at = Instant::now();
        let mut health = running(1, Some(1), at);
        for second in 1..=8 {
            health.applied(0, Some((fold(10, 5), 1)), None, at + secs(second as f64));
        }
        // A coalesced delta of 3 seconds, after a gap: the window still
        // holds 10 seconds, the oldest dropped.
        health.applied(0, Some((fold(90, 50), 3)), None, at + secs(14.0));
        let window = health.worker(0).unwrap().window_figures();
        assert_eq!(window.requests, 7 * 10 + 90);
        assert_eq!(window.requests_per_second, Some(16.0));
        assert_eq!(window.requests_per_user_per_second, Some(1.6));
        assert_eq!(window.p95_ms.map(f64::round), Some(50.0));
        // The run keeps every request.
        let ran = record(&health, 0, at + secs(14.0));
        assert_eq!(ran.requests, Some(8 * 10 + 90));
        // 80 at 5 ms and 90 at 50 ms.
        assert_eq!(ran.average_ms.map(|ms| (ms * 100.0).round()), Some(2882.0));
        assert_eq!(ran.p50_ms.map(f64::round), Some(50.0));
        // Totals read back from the merge replace those held.
        health.applied(0, None, Some((fold(5, 1), 2)), at + secs(15.0));
        assert_eq!(record(&health, 0, at + secs(15.0)).requests, Some(5));
        assert_eq!(health.totals_epoch(0), Some(2));
    }

    #[test]
    fn a_metrics_reset_starts_the_run_rates_again() {
        let at = Instant::now();
        let mut health = running(1, Some(1), at);
        // Epoch 1, the ramp: 10 seconds of 50 requests a second.
        health.applied(
            0,
            Some((fold(50, 5), 1)),
            Some((fold(500, 5), 1)),
            at + secs(10.0),
        );
        // The reset, at 10 s: epoch 2, then 10 seconds of 100 a second.
        health.applied(
            0,
            Some((fold(100, 5), 1)),
            Some((fold(100, 5), 2)),
            at + secs(11.0),
        );
        for second in 12..=20 {
            health.applied(0, Some((fold(100, 5), 1)), None, at + secs(second as f64));
        }
        let ran = record(&health, 0, at + secs(20.0));
        assert_eq!(ran.requests, Some(1_000));
        // Over the 10 seconds since the reset, not the 20 of the run.
        assert_eq!(ran.requests_per_second, Some(100.0));
        // The live window holds only the new epoch.
        assert_eq!(health.worker(0).unwrap().window_figures().requests, 1_000);
    }

    #[test]
    fn the_event_log_keeps_the_newest_and_counts_the_rest() {
        let mut health = GaggleHealth::default();
        let extra = 25;
        for n in 0..GaggleRecord::MAX_EVENTS + extra {
            health.set_second(n as u64);
            health.event(GaggleEventKind::Stop, None, format!("event {n}"));
        }
        let (events, dropped) = health.events();
        let events: Vec<&GaggleEvent> = events.collect();
        assert_eq!(events.len(), GaggleRecord::MAX_EVENTS);
        assert_eq!(dropped, extra as u64);
        assert_eq!(events[0].text, format!("event {extra}"));
        assert_eq!(events[0].second, extra as u64);
        assert!(chrono::DateTime::parse_from_rfc3339(&events[0].time).is_ok());
    }

    #[test]
    fn refusals_and_exclusions_are_flagged_and_logged_sparingly() {
        let at = Instant::now();
        let mut health = running(1, Some(1), at);
        for n in 0..20 {
            health.refused(0, format!("delta {n}: too many names"));
        }
        health.excluded(vec![(0, "it reported an old epoch".to_string())]);
        health.excluded(vec![(0, "it reported an old epoch".to_string())]);
        assert_eq!(
            kinds(&health),
            vec![
                GaggleEventKind::DeltasRefused,
                GaggleEventKind::DeltasRefused,
                GaggleEventKind::DeltasRefused,
                GaggleEventKind::WorkerExcluded
            ]
        );
        let worker = record(&health, 0, at);
        assert_eq!(
            worker.flags,
            vec![GaggleWorkerFlag::Excluded, GaggleWorkerFlag::Refused]
        );
        health.excluded(Vec::new());
        assert_eq!(
            record(&health, 0, at).flags,
            vec![GaggleWorkerFlag::Refused]
        );
    }

    /// The budget at scale: 1,024 workers, each delta 50 request names of
    /// 40 response time buckets. The time to take one second of every
    /// worker's deltas, judge the flags, build the records and render the
    /// HTML Workers section, and the live window's memory a worker, are
    /// printed for the record and held to generous bounds.
    #[test]
    fn budget_at_1024_workers() {
        use super::super::delta::{DeltaKind, RequestPart};
        use crate::goose::GooseMethod;
        const WORKERS: u32 = 1_024;
        const NAMES: usize = 50;
        let at = Instant::now();
        let mut health = running(WORKERS, Some(4), at);
        let mut delta = GaggleDelta::new(0, 1, 1, DeltaKind::Delta);
        for name in 0..NAMES {
            let mut part = RequestPart::new(&format!("/page/{name}"), GooseMethod::Get, "");
            for bucket in 0..40u64 {
                let us = buckets::bucket_floor(1_000 + bucket * 997 + name as u64 * 13);
                part.raw.buckets.push((us, 3));
                part.raw.count += 3;
                part.raw.total_us += us * 3;
            }
            part.raw.buckets.sort_unstable();
            part.raw.buckets.dedup_by_key(|bucket| bucket.0);
            part.raw.minimum_us = part.raw.buckets[0].0;
            part.raw.maximum_us = part.raw.buckets.last().unwrap().0;
            part.success = 120;
            delta
                .metrics
                .requests
                .insert(format!("GET /page/{name}"), part);
        }
        let mut second = Duration::ZERO;
        // Past the first 30 seconds, so the outliers are judged too.
        for tick in 31..=42u64 {
            let now = at + secs(tick as f64);
            let started = Instant::now();
            for id in 0..WORKERS {
                health.cpu(id, Some(tick * 1_000_000), None, now);
                health.applied(id, sample_of(&delta), None, now);
            }
            health.tick(now);
            second = started.elapsed();
        }
        let started = Instant::now();
        let record = GaggleRecord {
            workers: health.records(at + secs(42.0)),
            ..GaggleRecord::default()
        };
        let records = started.elapsed();
        let started = Instant::now();
        let html = crate::report::workers::html(Some(&record), crate::report::escape_html);
        let rendered = started.elapsed();
        let worker = health.worker(0).unwrap();
        let window: usize = worker
            .window
            .iter()
            .map(|sample| {
                std::mem::size_of::<Sample>()
                    + sample.fold.times.buckets.capacity() * std::mem::size_of::<(u64, u64)>()
            })
            .sum();
        let totals = worker.totals.times.buckets.capacity() * std::mem::size_of::<(u64, u64)>();
        println!(
            "budget: 1,024 workers x 50 names: one second of deltas and the tick {second:?}, records {records:?}, HTML Workers section {rendered:?} ({} bytes); live window {window} bytes a worker ({} samples), run totals {totals} bytes a worker",
            html.len(),
            worker.window.len()
        );
        assert_eq!(worker.window.len(), WINDOW_SECS as usize);
        assert_eq!(record.workers.len(), WORKERS as usize);
        // Generous bounds for an unoptimized test build on a busy machine.
        assert!(second < Duration::from_secs(5), "{:?}", second);
        assert!(rendered < Duration::from_secs(5), "{:?}", rendered);
        assert!(window < 512 * 1024, "{}", window);
    }

    /// A worker's words in an event carry no control character, and are
    /// cut short.
    #[test]
    fn event_text_is_plain_and_bounded() {
        let mut health = GaggleHealth::default();
        health.event(
            GaggleEventKind::WorkerLost,
            None,
            format!("w lost: \u{1b}[31mred\r\n{}", "x".repeat(1_000)),
        );
        let text = &health.events().0.next().unwrap().text;
        assert!(text.starts_with("w lost:  [31mred  x"), "{}", text);
        assert!(!text.chars().any(char::is_control));
        assert_eq!(text.chars().count(), MAX_EVENT_TEXT);
    }

    #[test]
    fn names_are_capped() {
        let mut health = GaggleHealth::default();
        health.join(0, &"x".repeat(100), None);
        assert_eq!(health.worker(0).unwrap().name.chars().count(), MAX_NAME);
    }

    #[test]
    fn this_process_has_cpu_time() {
        let before = process_cpu_micros();
        if cfg!(unix) {
            let before = before.expect("getrusage works");
            // Burn a little CPU.
            let mut x = 0u64;
            for i in 0..5_000_000u64 {
                x = x.wrapping_add(i ^ (x >> 3));
            }
            assert!(x > 0);
            assert!(process_cpu_micros().unwrap() >= before);
        } else {
            assert_eq!(before, None);
        }
    }
}
