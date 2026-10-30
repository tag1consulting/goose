//! What a Gaggle worker sends its manager once a second: everything its
//! metrics, graphs, dashboard snapshot and reports are built from, as a
//! change since its last delta or, at the end, as its whole state.
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
/// rather than sent: the counts and the severity histogram stay exact, but
/// the report's per user event table leaves those events out. A
/// [`DeltaKind::Final`] carries every event the worker holds.
pub(crate) const MAX_CO_EVENTS_PER_DELTA: usize = 1_000;

/// Whether a delta adds to what the manager holds for the worker, or
/// replaces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum DeltaKind {
    /// What changed since the worker's previous delta.
    Delta,
    /// The worker's whole state, which replaces everything held for it.
    Final,
}

/// One worker's metrics, as a change or as its whole state.
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
    /// one epoch; the caller handles an epoch change.
    pub(crate) fn add(&mut self, other: &GaggleDelta) {
        self.seq = self.seq.max(other.seq);
        self.now = self.now.max(other.now);
        self.series_origin = other.series_origin;
        self.metrics.add(&other.metrics);
        self.graph.add(&other.graph);
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
        let mut buckets: Vec<(u64, u64)> = data
            .times_us
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
            minimum_us: data.minimum_time_us,
            maximum_us: data.maximum_time_us,
            total_us: data.total_time_us,
            count: data.counter as u64,
        }
    }

    pub(crate) fn to_data(&self) -> GooseRequestMetricTimingData {
        let mut data = GooseRequestMetricTimingData::default();
        for &(bucket, count) in &self.buckets {
            debug_assert_eq!(buckets::bucket_floor(bucket), bucket);
            data.times_us.add(bucket, count as usize);
        }
        data.minimum_time_us = self.minimum_us;
        data.maximum_time_us = self.maximum_us;
        data.total_time_us = self.total_us;
        data.counter = self.count as usize;
        data
    }

    /// Add `other`'s times, as [`GooseRequestMetricTimingData::merge`] does.
    pub(crate) fn add(&mut self, other: &Timing) {
        if other.count > 0 && (self.count == 0 || other.minimum_us < self.minimum_us) {
            self.minimum_us = other.minimum_us;
        }
        self.maximum_us = self.maximum_us.max(other.maximum_us);
        self.total_us += other.total_us;
        self.count += other.count;
        self.buckets = add_sorted(&self.buckets, &other.buckets);
    }
}

/// Two sorted lists of `(key, count)` added up, still sorted.
fn add_sorted<K: Ord + Copy>(a: &[(K, u64)], b: &[(K, u64)]) -> Vec<(K, u64)> {
    let mut out = Vec::with_capacity(a.len().max(b.len()));
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].0.cmp(&b[j].0) {
            std::cmp::Ordering::Less => {
                out.push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                out.push(b[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                out.push((a[i].0, a[i].1 + b[j].1));
                i += 1;
                j += 1;
            }
        }
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
    out
}

/// The coordinated omission rule. A request's adjusted times, once it has
/// any, are its measured times plus the synthetic ones; a request without
/// them has only measured times. So two parts combine to adjusted times
/// when either has them, as the adjusted (or else measured) times of each
/// added up, and to none otherwise: worker A with adjusted times and worker
/// B without merge to A's adjusted times plus B's measured times.
pub(crate) fn add_adjusted(
    raw: &Timing,
    adjusted: &mut Option<Timing>,
    other_raw: &Timing,
    other_adjusted: &Option<Timing>,
) {
    if adjusted.is_none() && other_adjusted.is_none() {
        return;
    }
    let mut combined = adjusted.take().unwrap_or_else(|| raw.clone());
    combined.add(other_adjusted.as_ref().unwrap_or(other_raw));
    *adjusted = Some(combined);
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

    fn add(&mut self, other: &RequestPart) {
        add_adjusted(&self.raw, &mut self.adjusted, &other.raw, &other.adjusted);
        self.raw.add(&other.raw);
        for (&code, &count) in &other.status_codes {
            *self.status_codes.entry(code).or_default() += count;
        }
        for (&code, summary) in &other.status_timings {
            self.status_timings.entry(code).or_default().merge(summary);
        }
        self.success += other.success;
        self.fail += other.fail;
    }
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
    fn add(&mut self, other: &CoPart) {
        self.started_secs = self.started_secs.min(other.started_secs);
        self.actual += other.actual;
        self.synthetic += other.synthetic;
        for (&severity, &count) in &other.severity {
            *self.severity.entry(severity).or_default() += count;
        }
        self.events.extend(other.events.iter().cloned());
        self.events_dropped += other.events_dropped;
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
    pub(crate) fn add(&mut self, other: &MetricsPart) {
        for (key, request) in &other.requests {
            match self.requests.get_mut(key) {
                Some(held) => held.add(request),
                None => {
                    self.requests.insert(key.clone(), request.clone());
                }
            }
        }
        for (&key, transaction) in &other.transactions {
            let held = self.transactions.entry(key).or_default();
            held.run_times.add(&transaction.run_times);
            held.success += transaction.success;
            held.fail += transaction.fail;
        }
        for (&key, scenario) in &other.scenarios {
            let held = self.scenarios.entry(key).or_default();
            held.users.extend(scenario.users.iter().copied());
            held.run_times.add(&scenario.run_times);
        }
        for (key, &count) in &other.errors {
            *self.errors.entry(key.clone()).or_default() += count;
        }
        for (key, &count) in &other.transaction_errors {
            *self.transaction_errors.entry(key.clone()).or_default() += count;
        }
        self.hosts.extend(other.hosts.iter().cloned());
        match (self.co.as_mut(), other.co.as_ref()) {
            (Some(held), Some(co)) => held.add(co),
            (None, Some(co)) => self.co = Some(co.clone()),
            _ => {}
        }
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
    pub(crate) fn add(&mut self, other: &Histogram) {
        if other.count > 0 && (self.count == 0 || other.minimum_us < self.minimum_us) {
            self.minimum_us = other.minimum_us;
        }
        self.maximum_us = self.maximum_us.max(other.maximum_us);
        self.total_us += other.total_us;
        self.count += other.count;
        self.rows = add_sorted(&self.rows, &other.rows);
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

/// Add every `(second, value)` of `other` into `held` with `add`.
fn add_rows<V: Clone + Default>(
    held: &mut BTreeMap<String, BTreeMap<Second, V>>,
    other: &BTreeMap<String, BTreeMap<Second, V>>,
    add: impl Fn(&mut V, &V),
) {
    for (key, rows) in other {
        let held = held.entry(key.clone()).or_default();
        for (&second, value) in rows {
            add(held.entry(second).or_default(), value);
        }
    }
}

impl GraphPart {
    pub(crate) fn add(&mut self, other: &GraphPart) {
        self.add_rates(other);
        for (&second, &users) in &other.users {
            self.users.insert(second, users);
        }
    }

    /// [`Self::add`] without the users levels.
    pub(crate) fn add_rates(&mut self, other: &GraphPart) {
        add_rows(&mut self.requests, &other.requests, |a, b| *a += b);
        add_rows(&mut self.errors, &other.errors, |a, b| *a += b);
        add_rows(&mut self.averages, &other.averages, |a, b| {
            a.count += b.count;
            a.sum_ms += b.sum_ms;
        });
        add_rows(&mut self.latency, &other.latency, Histogram::add);
        add_rows(&mut self.synthetic, &other.synthetic, Histogram::add);
        for (&second, &count) in &other.transactions {
            *self.transactions.entry(second).or_default() += count;
        }
        for (&second, &count) in &other.scenarios {
            *self.scenarios.entry(second).or_default() += count;
        }
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
