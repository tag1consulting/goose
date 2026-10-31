//! What a Gaggle worker sends its manager once a second: everything its
//! metrics, graphs, dashboard snapshot and reports are built from, as a
//! change since its last delta or, at the end, as its whole metrics and the
//! graph rows since its last delta.
//!
//! Every part adds to another of its kind with one rule, [`GaggleDelta::add`],
//! which applying a delta, coalescing two deltas and merging workers all use.
//! Counts add, response time buckets add, sets join, and a users level is the
//! newest one sent; nothing is rounded on the way.
//!
//! Not to be confused with `metrics::delta`, the differences a report shows
//! against a baseline run.

use crate::goose::GooseMethod;
use crate::metrics::coordinated_omission::{CoEvent, CoSeverity};
use crate::metrics::{
    buckets, GooseCoordinatedOmissionMitigation, GooseRequestMetricTimingData,
    StatusCodeTimingSummary,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A second on the worker's own series clock: seconds since it started.
pub(crate) type Second = u64;

/// Most coordinated omission events one [`DeltaKind::Delta`] carries, oldest
/// first. Events past the cap are counted in [`CoPart::events_dropped`]
/// rather than sent: the request counts and the severity histogram stay
/// exact, but the report's event total, events per minute and per user and
/// per scenario event tables, which count the events held, leave those
/// events out. A [`DeltaKind::Final`] carries every event the worker holds.
pub(crate) const MAX_CO_EVENTS_PER_DELTA: usize = 1_000;

/// Whether a delta's metrics add to what the manager holds for the worker,
/// or replace it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum DeltaKind {
    /// What changed since the worker's previous delta.
    Delta,
    /// The worker's last delta: its whole metrics this epoch, which
    /// replace the metrics held for it, so a lost delta's metrics are made
    /// good; and, as any delta, the graph rows since its previous delta,
    /// which add to those held, so a lost delta's graph rows are not made
    /// good, only recorded in the manager's gaps.
    Final,
}

/// One worker's metrics, as a change, or its whole metrics in a final
/// delta (see [`DeltaKind::Final`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct GaggleDelta {
    /// The worker that sent it.
    pub(crate) worker: u32,
    /// 1 for the worker's first delta, one more for each after; the last
    /// one covered, when deltas were coalesced.
    pub(crate) seq: u64,
    /// The first sequence number covered: `seq`, unless deltas were
    /// coalesced.
    pub(crate) first_seq: u64,
    /// The metrics epoch: a metrics reset starts the next one, and the
    /// manager discards anything from an older one.
    pub(crate) epoch: u64,
    pub(crate) kind: DeltaKind,
    /// The worker's series second when it was taken: every second before
    /// the one before it is complete on the worker.
    pub(crate) now: Second,
    /// The series second of the metrics reset that started `epoch`, where
    /// the rate series and the heatmap's columns start again; `None` before
    /// a reset.
    pub(crate) series_origin: Option<Second>,
    pub(crate) metrics: MetricsPart,
    pub(crate) graph: GraphPart,
}

/// Why a delta was refused. A refused delta changes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DeltaError {
    /// It names a worker other than the one it came from.
    WrongWorker { from: u32, named: u32 },
    /// Its worker id is not below [`DeltaLimits::max_workers`].
    WorkerOutOfRange { worker: u32, limit: u32 },
    /// `first_seq` is after `seq`, or `seq` is `u64::MAX`.
    BadSequence { first_seq: u64, seq: u64 },
    /// It follows the worker's final delta.
    AfterFinal { seq: u64 },
    /// It covers some sequence numbers already applied and some not.
    Overlap { first_seq: u64, seq: u64, held: u64 },
    /// A series second after the delta's own clock, or a clock past
    /// [`DeltaLimits::max_second`].
    SecondOutOfRange { second: Second, limit: Second },
    /// A float that is not a finite number of at least 0.
    BadFloat,
    /// More names of one kind than [`DeltaLimits::max_names`].
    TooManyNames { names: usize, limit: usize },
    /// A user ordinal not below [`DeltaLimits::max_users`].
    UserOutOfRange { user: u64, limit: u64 },
    /// More coordinated omission events than
    /// [`MAX_CO_EVENTS_PER_DELTA`] in a delta that is not final.
    TooManyEvents { events: usize, limit: usize },
    /// A coordinated omission severity code no severity has.
    BadSeverity { code: u8 },
    /// A response time bucket or heatmap row no response time falls in, or
    /// buckets or rows out of order.
    BadBucket,
    /// A count or total past what one worker may hold ([`MAX_COUNT`], or
    /// [`MAX_SECOND_COUNT`] for one second).
    Overflow,
}

impl std::fmt::Display for DeltaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Most any count or total one worker's figures hold in an epoch. With at
/// most [`MAX_WORKERS`] workers, every sum of their figures fits an `i64`.
pub(crate) const MAX_COUNT: u64 = 1 << 51;

/// Most workers a manager merges: see [`MAX_COUNT`].
pub(crate) const MAX_WORKERS: u32 = 2_048;

/// Most any one second's count holds for one worker. A heatmap row of a
/// second counts in pieces of 2^24, so this bounds what one row costs.
pub(crate) const MAX_SECOND_COUNT: u64 = 1 << 32;

/// What a manager accepts from a worker, set when its accumulator is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DeltaLimits {
    /// The latest series second a delta may carry: no run is longer.
    pub(crate) max_second: Second,
    /// Most names of any one kind (request names, latency series,
    /// transactions, scenarios, request errors, transaction errors and
    /// hosts) one worker's figures hold.
    pub(crate) max_names: usize,
    /// User ordinals are below it: the users a scenario counts for a
    /// worker can be no more.
    pub(crate) max_users: u64,
    /// Worker ids are below it; at most [`MAX_WORKERS`].
    pub(crate) max_workers: u32,
}

impl Default for DeltaLimits {
    /// 30 days, 10,000 names, 1,000,000 users and 1,024 workers.
    fn default() -> Self {
        DeltaLimits {
            max_second: 30 * 24 * 60 * 60,
            max_names: 10_000,
            max_users: 1_000_000,
            max_workers: 1_024,
        }
    }
}

/// What a sum is bounded by.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bounds {
    /// Any count or total, and the size of a signed one.
    count: u64,
    /// Any one second's count.
    second: u64,
}

impl Bounds {
    /// One worker's figures.
    pub(crate) const WORKER: Bounds = Bounds {
        count: MAX_COUNT,
        second: MAX_SECOND_COUNT,
    };
    /// Every worker's figures added up, which [`Self::WORKER`] keeps in
    /// range.
    pub(crate) const MERGED: Bounds = Bounds {
        count: i64::MAX as u64,
        second: i64::MAX as u64,
    };
}

/// Whether adding only checks that every sum stays in bounds, changing
/// nothing, or adds. Every `add` here checks first, then adds the same
/// figures: so a sum out of bounds leaves what it was added to untouched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pass {
    Check,
    Apply,
}

/// `held + add`, at most `bound`, into `held` when applying.
fn sum(held: &mut u64, add: u64, bound: u64, pass: Pass) -> Result<(), DeltaError> {
    let total = held
        .checked_add(add)
        .filter(|&total| total <= bound)
        .ok_or(DeltaError::Overflow)?;
    if pass == Pass::Apply {
        *held = total;
    }
    Ok(())
}

/// [`sum`] for a signed count, at most `bound` either way.
fn sum_signed(held: &mut i64, add: i64, bound: u64, pass: Pass) -> Result<(), DeltaError> {
    let total = held
        .checked_add(add)
        .filter(|total| total.unsigned_abs() <= bound)
        .ok_or(DeltaError::Overflow)?;
    if pass == Pass::Apply {
        *held = total;
    }
    Ok(())
}

/// [`sum`] for a count held as `usize`.
fn sum_usize(held: &mut usize, add: usize, bound: u64, pass: Pass) -> Result<(), DeltaError> {
    let mut total = *held as u64;
    sum(&mut total, add as u64, bound, Pass::Apply)?;
    if pass == Pass::Apply {
        *held = total as usize;
    }
    Ok(())
}

/// Add each of `other`'s values into `held`'s of the same key with `add`,
/// a key `held` lacks starting from the default.
fn add_keyed<K: Ord + Clone, V: Default>(
    held: &mut BTreeMap<K, V>,
    other: &BTreeMap<K, V>,
    pass: Pass,
    mut add: impl FnMut(&mut V, &V, Pass) -> Result<(), DeltaError>,
) -> Result<(), DeltaError> {
    for (key, value) in other {
        match held.get_mut(key) {
            Some(held) => add(held, value, pass)?,
            None => {
                let mut fresh = V::default();
                add(&mut fresh, value, pass)?;
                if pass == Pass::Apply {
                    held.insert(key.clone(), fresh);
                }
            }
        }
    }
    Ok(())
}

/// How many names one kind of `held`'s would hold with `other`'s added.
fn names_with<K: Ord, V, W>(held: &BTreeMap<K, V>, other: &BTreeMap<K, W>) -> usize {
    held.len() + other.keys().filter(|key| !held.contains_key(*key)).count()
}

impl GaggleDelta {
    /// An empty delta.
    pub(crate) fn new(worker: u32, seq: u64, epoch: u64, kind: DeltaKind) -> Self {
        GaggleDelta {
            worker,
            seq,
            first_seq: seq,
            epoch,
            kind,
            now: 0,
            series_origin: None,
            metrics: MetricsPart::default(),
            graph: GraphPart::default(),
        }
    }

    /// Add `other`, which follows this one: counts and buckets add, sets
    /// join, and `other`'s users levels, clock and sequence win. Both are of
    /// one epoch; the caller handles an epoch change. One worker's figures
    /// stay within [`Bounds::WORKER`]: past it nothing is added.
    pub(crate) fn add(&mut self, other: &GaggleDelta) -> Result<(), DeltaError> {
        self.add_pass(other, Pass::Check, Bounds::WORKER)?;
        self.add_pass(other, Pass::Apply, Bounds::WORKER)
    }

    /// [`Self::add`], one pass of it.
    pub(crate) fn add_pass(
        &mut self,
        other: &GaggleDelta,
        pass: Pass,
        bounds: Bounds,
    ) -> Result<(), DeltaError> {
        self.metrics.add_pass(&other.metrics, pass, bounds)?;
        self.graph.add_pass(&other.graph, pass, bounds)?;
        if pass == Pass::Apply {
            self.seq = self.seq.max(other.seq);
            self.now = self.now.max(other.now);
            self.series_origin = other.series_origin;
        }
        Ok(())
    }

    /// Whether a manager can take this delta at all, whatever it holds:
    /// its worker id, its sequence numbers, every second and float, how
    /// many names it has, its buckets and rows, and every count within
    /// [`Bounds::WORKER`].
    pub(crate) fn validate(&self, limits: &DeltaLimits) -> Result<(), DeltaError> {
        let limit = limits.max_workers.min(MAX_WORKERS);
        if self.worker >= limit {
            return Err(DeltaError::WorkerOutOfRange {
                worker: self.worker,
                limit,
            });
        }
        if self.first_seq > self.seq || self.seq == u64::MAX {
            return Err(DeltaError::BadSequence {
                first_seq: self.first_seq,
                seq: self.seq,
            });
        }
        if self.now > limits.max_second {
            return Err(DeltaError::SecondOutOfRange {
                second: self.now,
                limit: limits.max_second,
            });
        }
        if let Some(second) = self.graph.seconds().chain(self.series_origin).max() {
            if second > self.now {
                return Err(DeltaError::SecondOutOfRange {
                    second,
                    limit: self.now,
                });
            }
        }
        let mut floats = self.graph.averages.values().flat_map(|rows| rows.values());
        if floats.any(|average| !(average.sum_ms.is_finite() && average.sum_ms >= 0.0)) {
            return Err(DeltaError::BadFloat);
        }
        let ordinals = self.metrics.scenarios.values().flat_map(|s| s.users.iter());
        if let Some(&user) = ordinals.max() {
            if user >= limits.max_users {
                return Err(DeltaError::UserOutOfRange {
                    user,
                    limit: limits.max_users,
                });
            }
        }
        if let Some(co) = self.metrics.co.as_ref() {
            if self.kind == DeltaKind::Delta && co.events.len() > MAX_CO_EVENTS_PER_DELTA {
                return Err(DeltaError::TooManyEvents {
                    events: co.events.len(),
                    limit: MAX_CO_EVENTS_PER_DELTA,
                });
            }
            if let Some(&code) = co.severity.keys().find(|&&code| code > MAX_SEVERITY_CODE) {
                return Err(DeltaError::BadSeverity { code });
            }
        }
        if let Some(&users) = self.graph.users.values().max() {
            if users > MAX_COUNT {
                return Err(DeltaError::Overflow);
            }
        }
        let empty = GaggleDelta::new(self.worker, 0, self.epoch, self.kind);
        let names = empty.names_with(self, true, true);
        if names > limits.max_names {
            return Err(DeltaError::TooManyNames {
                names,
                limit: limits.max_names,
            });
        }
        let timings = self
            .metrics
            .requests
            .values()
            .flat_map(|request| std::iter::once(&request.raw).chain(request.adjusted.as_ref()));
        let timings = timings
            .chain(self.metrics.transactions.values().map(|t| &t.run_times))
            .chain(self.metrics.scenarios.values().map(|s| &s.run_times));
        for timing in timings {
            let ordered = timing.buckets.windows(2).all(|pair| pair[0].0 < pair[1].0);
            let floors = timing
                .buckets
                .iter()
                .all(|&(bucket, _)| buckets::bucket_floor(bucket) == bucket);
            if !(ordered && floors) {
                return Err(DeltaError::BadBucket);
            }
        }
        let max_row = crate::graph::gaggle_max_row();
        for histogram in [&self.graph.latency, &self.graph.synthetic]
            .iter()
            .flat_map(|by_name| by_name.values())
            .flat_map(|rows| rows.values())
        {
            let ordered = histogram.rows.windows(2).all(|pair| pair[0].0 < pair[1].0);
            if !ordered || histogram.rows.last().is_some_and(|&(row, _)| row > max_row) {
                return Err(DeltaError::BadBucket);
            }
        }
        // Every count added to nothing: each one within bounds.
        empty.clone().add_pass(self, Pass::Check, Bounds::WORKER)
    }

    /// The most names of any one kind this delta would hold with `other`'s
    /// added: its metrics' names only when `metrics`, its graph rows' only
    /// when `graph`, others counting from none.
    pub(crate) fn names_with(&self, other: &GaggleDelta, metrics: bool, graph: bool) -> usize {
        let none = GaggleDelta::new(0, 0, 0, DeltaKind::Delta);
        let held_metrics = if metrics {
            &self.metrics
        } else {
            &none.metrics
        };
        let held_graph = if graph { &self.graph } else { &none.graph };
        let (m, o) = (held_metrics, &other.metrics);
        let (g, h) = (held_graph, &other.graph);
        [
            names_with(&m.requests, &o.requests),
            names_with(&m.transactions, &o.transactions),
            names_with(&m.scenarios, &o.scenarios),
            names_with(&m.errors, &o.errors),
            names_with(&m.transaction_errors, &o.transaction_errors),
            m.hosts.len() + o.hosts.difference(&m.hosts).count(),
            names_with(&g.requests, &h.requests),
            names_with(&g.errors, &h.errors),
            names_with(&g.averages, &h.averages),
            names_with(&g.latency, &h.latency),
            names_with(&g.synthetic, &h.synthetic),
        ]
        .iter()
        .copied()
        .max()
        .unwrap_or(0)
    }

    /// Encode for the wire.
    pub(crate) fn encode(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_stdvec(self)
    }

    /// Decode from the wire.
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }
}

/// Response times in microseconds, as [`GooseRequestMetricTimingData`] holds
/// them: each bucket's fastest time and count, sorted, and the unrounded
/// fastest, slowest and total.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Timing {
    pub(crate) buckets: Vec<(u64, u64)>,
    pub(crate) minimum_us: u64,
    pub(crate) maximum_us: u64,
    pub(crate) total_us: u64,
    pub(crate) count: u64,
}

impl Timing {
    pub(crate) fn from_data(data: &GooseRequestMetricTimingData) -> Self {
        let GooseRequestMetricTimingData {
            times_us,
            minimum_time_us,
            maximum_time_us,
            total_time_us,
            counter,
        } = data;
        let mut buckets: Vec<(u64, u64)> = times_us
            .iter()
            .map(|(&bucket, &count)| (bucket, count as u64))
            .collect();
        // Only a user's batch keeps its buckets unsorted; sort and join
        // repeats so equal times compare equal.
        buckets.sort_unstable_by_key(|&(bucket, _)| bucket);
        buckets.dedup_by(|later, kept| {
            if later.0 == kept.0 {
                kept.1 += later.1;
                true
            } else {
                false
            }
        });
        Timing {
            buckets,
            minimum_us: *minimum_time_us,
            maximum_us: *maximum_time_us,
            total_us: *total_time_us,
            count: *counter as u64,
        }
    }

    pub(crate) fn to_data(&self) -> GooseRequestMetricTimingData {
        let Timing {
            buckets: held,
            minimum_us,
            maximum_us,
            total_us,
            count,
        } = self;
        let mut times_us = crate::metrics::TimeBuckets::default();
        for &(bucket, count) in held {
            debug_assert_eq!(buckets::bucket_floor(bucket), bucket);
            times_us.add(bucket, count as usize);
        }
        GooseRequestMetricTimingData {
            times_us,
            minimum_time_us: *minimum_us,
            maximum_time_us: *maximum_us,
            total_time_us: *total_us,
            counter: *count as usize,
        }
    }

    /// Add `other`'s times, as [`GooseRequestMetricTimingData::merge`] does.
    fn add(&mut self, other: &Timing, pass: Pass, bounds: Bounds) -> Result<(), DeltaError> {
        let empty = self.count == 0;
        sum(&mut self.total_us, other.total_us, bounds.count, pass)?;
        sum(&mut self.count, other.count, bounds.count, pass)?;
        add_sorted(&mut self.buckets, &other.buckets, bounds.count, pass)?;
        if pass == Pass::Apply {
            if other.count > 0 && (empty || other.minimum_us < self.minimum_us) {
                self.minimum_us = other.minimum_us;
            }
            self.maximum_us = self.maximum_us.max(other.maximum_us);
        }
        Ok(())
    }
}

/// Add `other`, a sorted list of `(key, count)`, to `held`, another, each
/// sum at most `bound`; still sorted.
fn add_sorted<K: Ord + Copy>(
    held: &mut Vec<(K, u64)>,
    other: &[(K, u64)],
    bound: u64,
    pass: Pass,
) -> Result<(), DeltaError> {
    let a = &held[..];
    let mut out = Vec::new();
    if pass == Pass::Apply {
        out.reserve(a.len().max(other.len()));
    }
    let (mut i, mut j) = (0, 0);
    let mut push = |item: (K, u64)| {
        if pass == Pass::Apply {
            out.push(item);
        }
    };
    while i < a.len() && j < other.len() {
        match a[i].0.cmp(&other[j].0) {
            std::cmp::Ordering::Less => {
                push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                let mut count = 0;
                sum(&mut count, other[j].1, bound, Pass::Apply)?;
                push(other[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                let mut count = a[i].1;
                sum(&mut count, other[j].1, bound, Pass::Apply)?;
                push((a[i].0, count));
                i += 1;
                j += 1;
            }
        }
    }
    for &(_, count) in &other[j..] {
        sum(&mut 0, count, bound, Pass::Apply)?;
    }
    if pass == Pass::Apply {
        out.extend_from_slice(&a[i..]);
        out.extend_from_slice(&other[j..]);
        *held = out;
    }
    Ok(())
}

/// The coordinated omission rule. A request's adjusted times, once it has
/// any, are its measured times plus the synthetic ones; a request without
/// them has only measured times. So two parts combine to adjusted times
/// when either has them, as the adjusted (or else measured) times of each
/// added up, and to none otherwise: worker A with adjusted times and worker
/// B without merge to A's adjusted times plus B's measured times.
fn add_adjusted(
    raw: &Timing,
    adjusted: &mut Option<Timing>,
    other_raw: &Timing,
    other_adjusted: &Option<Timing>,
    pass: Pass,
    bounds: Bounds,
) -> Result<(), DeltaError> {
    let other = other_adjusted.as_ref().unwrap_or(other_raw);
    match adjusted.as_mut() {
        Some(held) => held.add(other, pass, bounds),
        None if other_adjusted.is_none() => Ok(()),
        None => {
            let mut combined = raw.clone();
            combined.add(other, pass, bounds)?;
            if pass == Pass::Apply {
                *adjusted = Some(combined);
            }
            Ok(())
        }
    }
}

/// One request's figures, keyed in [`MetricsPart::requests`] as
/// `"METHOD name"`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct RequestPart {
    pub(crate) path: String,
    pub(crate) method: GooseMethod,
    pub(crate) custom_method: String,
    pub(crate) raw: Timing,
    /// Coordinated omission adjusted times, when there are any.
    pub(crate) adjusted: Option<Timing>,
    pub(crate) status_codes: BTreeMap<u16, u64>,
    pub(crate) status_timings: BTreeMap<u16, StatusCodeTimingSummary>,
    /// Signed: `set_success()` turns a failure into a success.
    pub(crate) success: i64,
    pub(crate) fail: i64,
}

impl RequestPart {
    pub(crate) fn new(path: &str, method: GooseMethod, custom_method: &str) -> Self {
        RequestPart {
            path: path.to_string(),
            method,
            custom_method: custom_method.to_string(),
            raw: Timing::default(),
            adjusted: None,
            status_codes: BTreeMap::new(),
            status_timings: BTreeMap::new(),
            success: 0,
            fail: 0,
        }
    }

    fn add(&mut self, other: &RequestPart, pass: Pass, bounds: Bounds) -> Result<(), DeltaError> {
        add_adjusted(
            &self.raw,
            &mut self.adjusted,
            &other.raw,
            &other.adjusted,
            pass,
            bounds,
        )?;
        self.raw.add(&other.raw, pass, bounds)?;
        add_keyed(
            &mut self.status_codes,
            &other.status_codes,
            pass,
            |a, b, pass| sum(a, *b, bounds.count, pass),
        )?;
        add_keyed(
            &mut self.status_timings,
            &other.status_timings,
            pass,
            |a, b, pass| add_summary(a, b, pass, bounds),
        )?;
        sum_signed(&mut self.success, other.success, bounds.count, pass)?;
        sum_signed(&mut self.fail, other.fail, bounds.count, pass)
    }
}

/// [`StatusCodeTimingSummary::merge`], checked.
fn add_summary(
    held: &mut StatusCodeTimingSummary,
    other: &StatusCodeTimingSummary,
    pass: Pass,
    bounds: Bounds,
) -> Result<(), DeltaError> {
    let mut checked = held.clone();
    sum_usize(&mut checked.count, other.count, bounds.count, Pass::Apply)?;
    sum(
        &mut checked.total_time_us,
        other.total_time_us,
        bounds.count,
        Pass::Apply,
    )?;
    if pass == Pass::Apply {
        held.merge(other);
    }
    Ok(())
}

/// One transaction's figures.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct TransactionPart {
    pub(crate) run_times: Timing,
    pub(crate) success: u64,
    pub(crate) fail: u64,
}

/// One scenario's figures.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct ScenarioPart {
    /// The global ordinals of the users that ran it.
    pub(crate) users: BTreeSet<u64>,
    pub(crate) run_times: Timing,
}

/// One request error's identity; its occurrences are the map's value.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct ErrorKey {
    /// The errors table key, `"error.METHOD.name"`.
    pub(crate) key: String,
    pub(crate) method: GooseMethod,
    pub(crate) name: String,
    pub(crate) error: String,
    pub(crate) custom_method: String,
}

/// One transaction error's identity; its occurrences are the map's value.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct TransactionErrorKey {
    /// The transaction errors table key.
    pub(crate) key: String,
    pub(crate) scenario_index: u64,
    pub(crate) scenario_name: String,
    pub(crate) transaction_index: u64,
    pub(crate) transaction_name: String,
    pub(crate) error: String,
}

/// Coordinated omission counts and events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct CoPart {
    pub(crate) strategy: GooseCoordinatedOmissionMitigation,
    pub(crate) started_secs: u64,
    pub(crate) actual: u64,
    pub(crate) synthetic: u64,
    pub(crate) severity: BTreeMap<u8, u64>,
    /// Oldest first, user ids as global ordinals.
    pub(crate) events: Vec<CoEvent>,
    /// Events left out at [`MAX_CO_EVENTS_PER_DELTA`].
    pub(crate) events_dropped: u64,
}

impl CoPart {
    fn add(&mut self, other: &CoPart, pass: Pass, bounds: Bounds) -> Result<(), DeltaError> {
        sum(&mut self.actual, other.actual, bounds.count, pass)?;
        sum(&mut self.synthetic, other.synthetic, bounds.count, pass)?;
        add_keyed(&mut self.severity, &other.severity, pass, |a, b, pass| {
            sum(a, *b, bounds.count, pass)
        })?;
        sum(
            &mut self.events_dropped,
            other.events_dropped,
            bounds.count,
            pass,
        )?;
        if pass == Pass::Apply {
            self.started_secs = self.started_secs.min(other.started_secs);
            self.events.extend(other.events.iter().cloned());
        }
        Ok(())
    }

    /// Keep at most [`MAX_CO_EVENTS_PER_DELTA`] events, the oldest.
    pub(crate) fn cap_events(&mut self) {
        if self.events.len() > MAX_CO_EVENTS_PER_DELTA {
            self.events_dropped += (self.events.len() - MAX_CO_EVENTS_PER_DELTA) as u64;
            self.events.truncate(MAX_CO_EVENTS_PER_DELTA);
        }
    }
}

/// A severity as the wire carries it.
pub(crate) fn severity_code(severity: CoSeverity) -> u8 {
    match severity {
        CoSeverity::Minor => 0,
        CoSeverity::Moderate => 1,
        CoSeverity::Severe => 2,
        CoSeverity::Critical => 3,
    }
}

/// The highest code [`severity_code`] gives.
const MAX_SEVERITY_CODE: u8 = 3;

/// [`severity_code`] read back.
pub(crate) fn severity_from_code(code: u8) -> CoSeverity {
    match code {
        0 => CoSeverity::Minor,
        1 => CoSeverity::Moderate,
        2 => CoSeverity::Severe,
        _ => CoSeverity::Critical,
    }
}

/// Everything `GooseMetrics` is built from.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct MetricsPart {
    pub(crate) requests: BTreeMap<String, RequestPart>,
    /// Keyed by scenario index and transaction index.
    pub(crate) transactions: BTreeMap<(u64, u64), TransactionPart>,
    /// Keyed by scenario index.
    pub(crate) scenarios: BTreeMap<u64, ScenarioPart>,
    pub(crate) errors: BTreeMap<ErrorKey, u64>,
    pub(crate) transaction_errors: BTreeMap<TransactionErrorKey, u64>,
    pub(crate) hosts: BTreeSet<String>,
    pub(crate) co: Option<CoPart>,
}

impl MetricsPart {
    /// Add `other`'s figures, within `bounds`: one pass (see [`Pass`]).
    pub(crate) fn add_pass(
        &mut self,
        other: &MetricsPart,
        pass: Pass,
        bounds: Bounds,
    ) -> Result<(), DeltaError> {
        for (key, request) in &other.requests {
            match self.requests.get_mut(key) {
                Some(held) => held.add(request, pass, bounds)?,
                None => {
                    let mut fresh =
                        RequestPart::new(&request.path, request.method, &request.custom_method);
                    fresh.add(request, pass, bounds)?;
                    if pass == Pass::Apply {
                        self.requests.insert(key.clone(), fresh);
                    }
                }
            }
        }
        add_keyed(
            &mut self.transactions,
            &other.transactions,
            pass,
            |held, transaction, pass| {
                held.run_times.add(&transaction.run_times, pass, bounds)?;
                sum(&mut held.success, transaction.success, bounds.count, pass)?;
                sum(&mut held.fail, transaction.fail, bounds.count, pass)
            },
        )?;
        add_keyed(
            &mut self.scenarios,
            &other.scenarios,
            pass,
            |held, scenario, pass| {
                held.run_times.add(&scenario.run_times, pass, bounds)?;
                if pass == Pass::Apply {
                    held.users.extend(scenario.users.iter().copied());
                }
                Ok(())
            },
        )?;
        add_keyed(&mut self.errors, &other.errors, pass, |a, b, pass| {
            sum(a, *b, bounds.count, pass)
        })?;
        add_keyed(
            &mut self.transaction_errors,
            &other.transaction_errors,
            pass,
            |a, b, pass| sum(a, *b, bounds.count, pass),
        )?;
        if let Some(co) = other.co.as_ref() {
            match self.co.as_mut() {
                Some(held) => held.add(co, pass, bounds)?,
                None => {
                    let mut fresh = CoPart {
                        strategy: co.strategy.clone(),
                        started_secs: co.started_secs,
                        actual: 0,
                        synthetic: 0,
                        severity: BTreeMap::new(),
                        events: Vec::new(),
                        events_dropped: 0,
                    };
                    fresh.add(co, pass, bounds)?;
                    if pass == Pass::Apply {
                        self.co = Some(fresh);
                    }
                }
            }
        }
        if pass == Pass::Apply {
            self.hosts.extend(other.hosts.iter().cloned());
        }
        Ok(())
    }
}

/// The average response time of one request in one second, in
/// milliseconds: how many responses and their sum, so averages merge
/// weighted by count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Average {
    pub(crate) count: u64,
    pub(crate) sum_ms: f64,
}

/// One second's response times as the heatmap counts them: each non empty
/// row's index and count, sorted, and the unrounded fastest, slowest, total
/// and count.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Histogram {
    pub(crate) rows: Vec<(u32, u64)>,
    pub(crate) minimum_us: u64,
    pub(crate) maximum_us: u64,
    pub(crate) total_us: u64,
    pub(crate) count: u64,
}

impl Histogram {
    fn add(&mut self, other: &Histogram, pass: Pass, bounds: Bounds) -> Result<(), DeltaError> {
        let empty = self.count == 0;
        sum(&mut self.total_us, other.total_us, bounds.count, pass)?;
        sum(&mut self.count, other.count, bounds.second, pass)?;
        add_sorted(&mut self.rows, &other.rows, bounds.second, pass)?;
        if pass == Pass::Apply {
            if other.count > 0 && (empty || other.minimum_us < self.minimum_us) {
                self.minimum_us = other.minimum_us;
            }
            self.maximum_us = self.maximum_us.max(other.maximum_us);
        }
        Ok(())
    }
}

/// Everything `GraphData` is built from, by series second.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct GraphPart {
    /// Requests started in each second, by `"METHOD name"`.
    pub(crate) requests: BTreeMap<String, BTreeMap<Second, u64>>,
    /// Failed requests in each second, less those `set_success()` took
    /// back, so signed.
    pub(crate) errors: BTreeMap<String, BTreeMap<Second, i64>>,
    pub(crate) averages: BTreeMap<String, BTreeMap<Second, Average>>,
    /// Measured response times by second, of every name the worker saw.
    pub(crate) latency: BTreeMap<String, BTreeMap<Second, Histogram>>,
    /// Coordinated omission's synthetic times by second.
    pub(crate) synthetic: BTreeMap<String, BTreeMap<Second, Histogram>>,
    pub(crate) transactions: BTreeMap<Second, u64>,
    pub(crate) scenarios: BTreeMap<Second, u64>,
    /// The users running at each second: a level, so a later value for a
    /// second replaces an earlier one. Kept over metrics resets.
    pub(crate) users: BTreeMap<Second, u64>,
}

impl GraphPart {
    /// [`GaggleDelta::add_pass`] for the graph rows.
    fn add_pass(
        &mut self,
        other: &GraphPart,
        pass: Pass,
        bounds: Bounds,
    ) -> Result<(), DeltaError> {
        self.add_rates(other, pass, bounds)?;
        if pass == Pass::Apply {
            for (&second, &users) in &other.users {
                self.users.insert(second, users);
            }
        }
        Ok(())
    }

    /// Add `other`'s rows apart from the users levels, within `bounds`:
    /// past them nothing is added.
    pub(crate) fn add_rates(
        &mut self,
        other: &GraphPart,
        pass: Pass,
        bounds: Bounds,
    ) -> Result<(), DeltaError> {
        let count = |a: &mut u64, b: &u64, pass| sum(a, *b, bounds.second, pass);
        let rows = |held: &mut BTreeMap<Second, u64>, other: &BTreeMap<Second, u64>, pass| {
            add_keyed(held, other, pass, count)
        };
        add_keyed(&mut self.requests, &other.requests, pass, rows)?;
        add_keyed(
            &mut self.errors,
            &other.errors,
            pass,
            |held, other, pass| {
                add_keyed(held, other, pass, |a, b, pass| {
                    sum_signed(a, *b, bounds.second, pass)
                })
            },
        )?;
        add_keyed(
            &mut self.averages,
            &other.averages,
            pass,
            |held, other, pass| {
                add_keyed(held, other, pass, |a: &mut Average, b, pass| {
                    // Bounded as a count is, so every worker's added up
                    // stays finite.
                    let total = a.sum_ms + b.sum_ms;
                    if !(total.is_finite() && total <= bounds.count as f64) {
                        return Err(DeltaError::Overflow);
                    }
                    sum(&mut a.count, b.count, bounds.second, pass)?;
                    if pass == Pass::Apply {
                        a.sum_ms = total;
                    }
                    Ok(())
                })
            },
        )?;
        for (held, other) in [
            (&mut self.latency, &other.latency),
            (&mut self.synthetic, &other.synthetic),
        ] {
            add_keyed(held, other, pass, |held, other, pass| {
                add_keyed(held, other, pass, |a, b, pass| a.add(b, pass, bounds))
            })?;
        }
        add_keyed(&mut self.transactions, &other.transactions, pass, count)?;
        add_keyed(&mut self.scenarios, &other.scenarios, pass, count)
    }

    /// Every series second a row or users level is held for.
    pub(crate) fn seconds(&self) -> impl Iterator<Item = Second> + Clone + '_ {
        fn keys<V>(
            by_name: &BTreeMap<String, BTreeMap<Second, V>>,
        ) -> impl Iterator<Item = Second> + Clone + '_ {
            by_name.values().flat_map(|rows| rows.keys().copied())
        }
        keys(&self.requests)
            .chain(keys(&self.errors))
            .chain(keys(&self.averages))
            .chain(keys(&self.latency))
            .chain(keys(&self.synthetic))
            .chain(self.transactions.keys().copied())
            .chain(self.scenarios.keys().copied())
            .chain(self.users.keys().copied())
    }

    /// Forget the rate series, keeping the users levels, at a metrics reset.
    pub(crate) fn clear_rates(&mut self) {
        let users = std::mem::take(&mut self.users);
        *self = GraphPart {
            users,
            ..Default::default()
        };
    }
}
