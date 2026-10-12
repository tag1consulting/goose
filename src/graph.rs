//! Optional graph data collected during the load tests.
//!
//! If the HTML report is enabled the graph data will be collected and stored in
//! the [`GraphData`] structure during the load test. When the report is written
//! this data is converted into [`Graph`] structures and HTML markup is generated
//! based on them.

use crate::metrics::dashboard_snapshot::{
    Heatmap, HeatmapColumn, HeatmapCursor, RequestSeries, SeriesWindow, SERIES_WINDOW_SECS,
};
use crate::metrics::{round_metric_time, GooseRequestMetricTimingData};
use crate::test_plan::{TestPlanHistory, TestPlanStepAction};
use chrono::prelude::*;
use itertools::Itertools;
use serde::Serialize;
use serde_json::json;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::TryFrom;
use std::fmt::Write;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::OnceLock;

/// One key's counts by second, and for requests, which latency ring holds the
/// request's own response times. Laid out in order, so a request's pending
/// response times follow the fields a batch merge reads first.
#[derive(Clone)]
#[repr(C)]
struct KeySeries {
    counts: TimeSeries<u32, u32>,
    /// `None` until the request's latency is first recorded through this
    /// entry, then the index of its ring in [`LatencyHistograms`], or
    /// `Some(None)` when it has no ring of its own. Caching it here lets a
    /// batch merge find the ring with the same lookup that counts the
    /// request. Unused for errors. Reset with the latency histograms, by
    /// [`GraphData::reset_rate_series`].
    latency_ring: Option<Option<usize>>,
    /// The request's response times from the newest batches, kept here,
    /// next to the count each batch already updates, until they move to its
    /// ring (see [`PendingLatency`]). Unused for errors.
    pending: PendingLatency,
}

/// A request's batched response times for two consecutive seconds, counted
/// by fixed bucket index, waiting to move to its latency ring. It lives in
/// the request's [`KeySeries`], so merging a batch writes next to the count
/// the batch already updates instead of reaching into the ring's slot for
/// the second; the counts move to the ring once both seconds are past, or
/// when a heatmap is built. Laid out in order: what every merge writes
/// first, and both seconds' counts of a bucket side by side.
#[derive(Clone)]
#[repr(C)]
struct PendingLatency {
    /// The seconds held: `second` and `second + 1`, once `held`.
    second: usize,
    held: bool,
    /// Fastest, slowest, total and count, for each second held.
    totals: [[usize; 4]; 2],
    /// Rounded times with no fixed index (past 60 seconds): which second
    /// held (0 or 1), the rounded time and its count.
    overflow: Vec<(usize, usize, usize)>,
    /// Counts by fixed bucket index, for each second held.
    counts: [[u32; 2]; FIXED_BUCKETS],
}

impl PendingLatency {
    fn new() -> Self {
        PendingLatency {
            second: 0,
            held: false,
            totals: [[0; 4]; 2],
            overflow: Vec::new(),
            counts: [[0; 2]; FIXED_BUCKETS],
        }
    }

    /// Add a batch's timing data for held second `at` (0 or 1).
    fn add(&mut self, at: usize, data: &GooseRequestMetricTimingData) {
        let totals = &mut self.totals[at];
        if data.counter > 0 && (totals[3] == 0 || data.minimum_time < totals[0]) {
            totals[0] = data.minimum_time;
        }
        totals[1] = totals[1].max(data.maximum_time);
        totals[2] += data.total_time;
        totals[3] += data.counter;
        for (&rounded, &count) in &data.times {
            match dense_index(rounded) {
                Some(index) => {
                    let held = &mut self.counts[index][at];
                    *held = held.saturating_add(u32::try_from(count).unwrap_or(u32::MAX));
                }
                None => self.overflow.push((at, rounded, count)),
            }
        }
    }

    /// Move everything held to `ring`, each second marked with `rev` of its
    /// own, and empty the counts.
    fn move_to(&mut self, ring: &mut LatencyRing, mut next_rev: impl FnMut() -> u64) {
        for at in 0..2 {
            let [minimum, maximum, total, counter] = self.totals[at];
            if counter == 0 {
                continue;
            }
            if let Some(histogram) = ring.slot_mut(self.second + at, next_rev()) {
                for (index, counts) in self.counts.iter().enumerate() {
                    if counts[at] > 0 {
                        histogram.add_dense(index, counts[at] as usize);
                    }
                }
                for &(held, rounded, count) in &self.overflow {
                    if held == at {
                        histogram.add_bucket(rounded, count);
                    }
                }
                histogram.add_totals(minimum, maximum, total, counter);
            }
            for counts in self.counts.iter_mut() {
                counts[at] = 0;
            }
            self.totals[at] = [0; 4];
        }
        self.overflow.clear();
    }
}

impl std::ops::Deref for KeySeries {
    type Target = TimeSeries<u32, u32>;
    fn deref(&self) -> &Self::Target {
        &self.counts
    }
}

impl std::ops::DerefMut for KeySeries {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.counts
    }
}

#[derive(Clone)]
struct ItemsPerSecond {
    series: HashMap<String, KeySeries>,
    /// Absolute second used as `origin` for newly inserted keys.
    ///
    /// After dashboard pruning / rate-series reset this tracks the retained window
    /// so a late-seen request name does not allocate from absolute second 0.
    origin: usize,
}

impl ItemsPerSecond {
    fn new() -> ItemsPerSecond {
        ItemsPerSecond {
            series: Default::default(),
            origin: 0,
        }
    }

    fn with_origin(origin: usize) -> ItemsPerSecond {
        ItemsPerSecond {
            series: Default::default(),
            origin,
        }
    }

    #[inline(always)]
    fn initialize_or_increment(&mut self, key: &str, second: usize, value: u32) -> u32 {
        let data = self.increment(key, second, value);
        data.get(second)
    }

    /// Add `value` to `key`'s count for `second`, returning the key's entry.
    #[inline(always)]
    fn increment(&mut self, key: &str, second: usize, value: u32) -> &mut KeySeries {
        let origin = self.origin;
        let data = self
            .series
            .entry(key.to_string())
            .or_insert_with(|| KeySeries {
                counts: TimeSeries::with_origin(origin),
                latency_ring: None,
                pending: PendingLatency::new(),
            });
        data.increase_value(second, value);
        data
    }

    /// A report graph of every key's counts, in the map's iteration order.
    fn graph<'a>(
        &'a self,
        html_id: &'a str,
        y_axis_label: &'a str,
        granular_data: bool,
    ) -> Graph<'a, u32, u32> {
        Graph {
            html_id,
            y_axis_label,
            granular_data,
            data: self
                .series
                .iter()
                .map(|(label, series)| (label.as_str(), &series.counts))
                .collect(),
        }
    }

    #[cfg(test)]
    #[inline(always)]
    fn insert(&mut self, key: &str, time_series: TimeSeries<u32, u32>) {
        self.series.insert(
            key.to_string(),
            KeySeries {
                counts: time_series,
                latency_ring: None,
                pending: PendingLatency::new(),
            },
        );
    }

    #[cfg(test)]
    #[inline(always)]
    fn len(&self) -> usize {
        self.series.len()
    }

    #[cfg(test)]
    #[inline(always)]
    fn get(&self, key: &str) -> Option<TimeSeries<u32, u32>> {
        self.series.get(key).map(|data| data.counts.clone())
    }
}

/// Most request names that get their own latency series. Later names still
/// count toward the all requests series.
pub(crate) const MAX_REQUEST_SERIES: usize = 100;

/// Rounded response times up to the default 60 second timeout each have a
/// fixed index: every millisecond to 100 ms, then 110 to 500 ms by 10, 600
/// to 1000 ms by 100 and 2 to 60 seconds by 1000.
const FIXED_BUCKETS: usize = 205;

/// The fixed index of a rounded response time, if it has one.
fn dense_index(rounded: usize) -> Option<usize> {
    match rounded {
        0..=100 => Some(rounded),
        101..=500 => Some(100 + (rounded - 100) / 10),
        501..=1000 => Some(140 + (rounded - 500) / 100),
        1001..=60_000 => Some(145 + (rounded - 1000) / 1000),
        _ => None,
    }
}

/// The rounded response time a fixed index stands for.
fn dense_value(index: usize) -> usize {
    match index {
        0..=100 => index,
        101..=140 => 100 + (index - 100) * 10,
        141..=145 => 500 + (index - 140) * 100,
        _ => 1000 + (index - 145) * 1000,
    }
}

/// Bits of a [`SecondHistogram`] entry that hold the count; the bucket index
/// is in the top 8 bits.
const COUNT_BITS: u32 = 24;
/// Most one entry counts. A bucket that fills an entry continues in the next.
const COUNT_MAX: u32 = (1 << COUNT_BITS) - 1;

/// One second's response times, in the same buckets as
/// [`GooseRequestMetricTimingData`](crate::metrics::GooseRequestMetricTimingData) but held compactly: a sorted list with 4
/// bytes for each bucket the second has seen (its index and its count), so a
/// quiet name costs little and a busy one at most about 1 KB a second.
/// Rounded times above 60 seconds, possible with a longer `--timeout`, go to
/// a short sorted overflow list. The metrics processor merges each batch's
/// response times for a request name (the batch's
/// [`GooseRequestMetricTimingData`], all from one second) into the rings. A
/// ring counts its newest seconds in a flat array instead (see
/// [`LatencyRing`]), so merging a batch is an addition per bucket rather
/// than a search, and packs each second once it stops filling.
#[derive(Clone, Debug, Default)]
pub(crate) struct SecondHistogram {
    entries: Vec<u32>,
    overflow: Vec<(usize, u32)>,
    /// While a ring's second is still filling: its count for each fixed
    /// index, with `entries` empty.
    open: Option<Box<[usize; FIXED_BUCKETS]>>,
    /// The fastest response, not rounded.
    pub(crate) minimum_time: usize,
    /// The slowest response, not rounded.
    pub(crate) maximum_time: usize,
    /// Sum of all response times.
    pub(crate) total_time: usize,
    /// Number of responses.
    pub(crate) counter: usize,
}

/// Two histograms are equal when they hold the same response times, however
/// each holds them.
impl PartialEq for SecondHistogram {
    fn eq(&self, other: &Self) -> bool {
        self.minimum_time == other.minimum_time
            && self.maximum_time == other.maximum_time
            && self.total_time == other.total_time
            && self.counter == other.counter
            && self
                .buckets()
                .coalesce(merge_bucket)
                .eq(other.buckets().coalesce(merge_bucket))
    }
}

/// A bucket as `(rounded time, count)`.
type Bucket = (usize, usize);

/// Join consecutive pieces of one bucket (a bucket past [`COUNT_MAX`]).
fn merge_bucket(a: Bucket, b: Bucket) -> Result<Bucket, (Bucket, Bucket)> {
    if a.0 == b.0 {
        Ok((a.0, a.1 + b.1))
    } else {
        Err((a, b))
    }
}

impl SecondHistogram {
    /// An empty histogram for a ring's newest second, counted by fixed index
    /// until [`pack`](Self::pack).
    fn open() -> Self {
        SecondHistogram {
            open: Some(Box::new([0; FIXED_BUCKETS])),
            ..Default::default()
        }
    }

    /// Add `count` to the bucket with fixed index `index`, searching from
    /// entry `from` on. Returns where the search for a higher index can
    /// start.
    fn add_indexed(&mut self, from: usize, index: usize, mut count: usize) -> usize {
        debug_assert!(index < FIXED_BUCKETS && FIXED_BUCKETS <= 1 << (32 - COUNT_BITS));
        let bucket = index as u32;
        let key = bucket << COUNT_BITS;
        let mut at = from + self.entries[from..].partition_point(|&entry| entry < key);
        while count > 0 {
            if at < self.entries.len() && self.entries[at] >> COUNT_BITS == bucket {
                let room = (COUNT_MAX - (self.entries[at] & COUNT_MAX)) as usize;
                let add = room.min(count);
                self.entries[at] += add as u32;
                count -= add;
            } else {
                let add = count.min(COUNT_MAX as usize);
                self.entries.insert(at, key | add as u32);
                count -= add;
            }
            at += 1;
        }
        at
    }

    fn add_bucket(&mut self, rounded: usize, count: usize) {
        match dense_index(rounded) {
            Some(index) => self.add_dense(index, count),
            None => self.add_overflow(rounded, count),
        }
    }

    /// Add `count` to the bucket with fixed index `index`.
    fn add_dense(&mut self, index: usize, count: usize) {
        match self.open.as_mut() {
            Some(open) => open[index] += count,
            None => {
                self.add_indexed(0, index, count);
            }
        }
    }

    /// Add `count` to the bucket of a rounded time with no fixed index.
    fn add_overflow(&mut self, rounded: usize, count: usize) {
        let count = u32::try_from(count).unwrap_or(u32::MAX);
        match self
            .overflow
            .binary_search_by_key(&rounded, |&(bucket, _)| bucket)
        {
            Ok(at) => self.overflow[at].1 = self.overflow[at].1.saturating_add(count),
            Err(at) => self.overflow.insert(at, (rounded, count)),
        }
    }

    fn add_totals(&mut self, minimum: usize, maximum: usize, total: usize, counter: usize) {
        // The counters, not a minimum of 0, say whether there are times yet:
        // a response under a millisecond is a minimum of 0.
        if counter > 0 && (self.counter == 0 || minimum < self.minimum_time) {
            self.minimum_time = minimum;
        }
        if maximum > self.maximum_time {
            self.maximum_time = maximum;
        }
        self.total_time += total;
        self.counter += counter;
    }

    /// Record one response time, as
    /// [`GooseRequestMetricTimingData::record_time`](crate::metrics::GooseRequestMetricTimingData::record_time)
    /// does.
    pub(crate) fn record(&mut self, time: u64) {
        let time_ms = time as usize;
        self.add_totals(time_ms, time_ms, time_ms, 1);
        self.add_bucket(round_metric_time(time), 1);
    }

    /// Add another second's response times.
    pub(crate) fn merge(&mut self, other: &SecondHistogram) {
        if let Some(open) = self.open.as_mut() {
            for (index, count) in other.indexed() {
                open[index] += count;
            }
        } else {
            // Both are sorted, so each search starts where the last one ended.
            let mut from = 0;
            for (index, count) in other.indexed() {
                from = self.add_indexed(from, index, count);
            }
        }
        for &(bucket, count) in &other.overflow {
            self.add_bucket(bucket, count as usize);
        }
        self.add_totals(
            other.minimum_time,
            other.maximum_time,
            other.total_time,
            other.counter,
        );
    }

    /// Add a batch's response times for one request name, all from this
    /// second: buckets of rounded times, and the unrounded minimum, maximum,
    /// total and count, as
    /// [`GooseRequestMetricTimingData::merge`] adds them.
    pub(crate) fn merge_timing(&mut self, other: &GooseRequestMetricTimingData) {
        for (&rounded, &count) in &other.times {
            self.add_bucket(rounded, count);
        }
        self.add_totals(
            other.minimum_time,
            other.maximum_time,
            other.total_time,
            other.counter,
        );
    }

    /// Once the second is unlikely to grow: move the counts of an open
    /// second into the compact list, and release spare capacity.
    fn pack(&mut self) {
        if let Some(open) = self.open.take() {
            let mut from = 0;
            for (index, &count) in open.iter().enumerate() {
                if count > 0 {
                    from = self.add_indexed(from, index, count);
                }
            }
        }
        self.entries.shrink_to_fit();
        self.overflow.shrink_to_fit();
    }

    /// The heatmap's buckets of this second: each non empty bucket as
    /// `index, count`, flattened, by ascending heatmap index.
    fn heat_cells(&self) -> Vec<u64> {
        let mut cells: Vec<u64> = Vec::new();
        let overflow = self
            .overflow
            .iter()
            .map(|&(rounded, count)| (heat_index(rounded), count as usize));
        for (index, count) in self.indexed().chain(overflow) {
            let (index, count) = (index as u64, count as u64);
            // A bucket past COUNT_MAX continues in the next entry.
            match cells.len().checked_sub(2).map(|at| cells[at]) {
                Some(last) if last == index => *cells.last_mut().expect("a count") += count,
                _ => cells.extend([index, count]),
            }
        }
        cells
    }

    /// Non empty buckets with a fixed index as `(index, count)`, fastest
    /// first. A bucket past [`COUNT_MAX`] appears more than once,
    /// consecutively, unless the second is open.
    fn indexed(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        let open = self.open.iter().flat_map(|open| {
            open.iter()
                .enumerate()
                .filter(|&(_, &count)| count > 0)
                .map(|(index, &count)| (index, count))
        });
        let packed = self
            .entries
            .iter()
            .map(|&entry| ((entry >> COUNT_BITS) as usize, (entry & COUNT_MAX) as usize));
        open.chain(packed)
    }

    /// Non empty buckets as `(rounded time, count)`, fastest first. A bucket
    /// past [`COUNT_MAX`] may appear more than once, consecutively.
    fn buckets(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.indexed()
            .map(|(index, count)| (dense_value(index), count))
            .chain(
                self.overflow
                    .iter()
                    .map(|&(bucket, count)| (bucket, count as usize)),
            )
    }

    /// The response time `percent` of these requests finished within,
    /// computed as for the whole run's percentiles (unit tests).
    #[cfg(test)]
    pub(crate) fn percentile(&self, percent: f32) -> usize {
        crate::metrics::response_time_percentile(
            self.buckets(),
            self.counter,
            self.minimum_time,
            self.maximum_time,
            percent,
        )
    }
}

/// The heatmap's index of a rounded response time: its fixed index, or past
/// 60 seconds one more for each second.
fn heat_index(rounded: usize) -> usize {
    dense_index(rounded)
        .unwrap_or_else(|| FIXED_BUCKETS - 1 + rounded.saturating_sub(60_000) / 1000)
}

/// A bucket's rounded time and the lowest and highest measured times that
/// round to it, in milliseconds.
fn bucket_entry(value: u64) -> [u64; 3] {
    // round_metric_time never decreases, so the times that round to `value`
    // are one range: from the first that rounds to it up to the first that
    // rounds past it.
    let first_rounding_to = |target: u64| {
        let (mut low, mut high) = (0u64, value * 2 + 1000);
        while low < high {
            let middle = (low + high) / 2;
            if (round_metric_time(middle) as u64) < target {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        low
    };
    [
        value,
        first_rounding_to(value),
        first_rounding_to(value + 1) - 1,
    ]
}

/// The heatmap's bucket table, `[time, lowest, highest]` for each index from
/// 0 to `last`, and always every fixed index.
fn bucket_table(last: usize) -> Vec<[u64; 3]> {
    static FIXED: OnceLock<Vec<[u64; 3]>> = OnceLock::new();
    let mut table = FIXED
        .get_or_init(|| {
            (0..FIXED_BUCKETS)
                .map(|index| bucket_entry(dense_value(index) as u64))
                .collect()
        })
        .clone();
    for index in FIXED_BUCKETS..=last {
        table.push(bucket_entry(
            60_000 + (index + 1 - FIXED_BUCKETS) as u64 * 1000,
        ));
    }
    table
}

/// A new generation for a set of latency histograms: unique in the process
/// and, starting from the clock, unlikely to repeat one a client kept from an
/// earlier Goose.
fn next_heatmap_generation() -> u64 {
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    NEXT.get_or_init(|| {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |since| since.as_millis() as u64);
        AtomicU64::new(now_ms * 1000)
    })
    .fetch_add(1, AtomicOrdering::Relaxed)
}

/// One second of a [`LatencyRing`].
#[derive(Clone)]
struct RingSlot {
    second: usize,
    /// The histograms' revision when this second last changed.
    rev: u64,
    data: SecondHistogram,
    /// The heatmap's cells of this second as of a revision (0 before the
    /// first build), so a column is built once for each change, whichever
    /// heatmap asks for it.
    cells: (u64, Vec<u64>),
}

impl RingSlot {
    /// This second's heatmap cells, built only if it changed since the last
    /// build.
    fn heat_cells(&mut self) -> Vec<u64> {
        if self.cells.0 != self.rev {
            self.cells = (self.rev, self.data.heat_cells());
        }
        self.cells.1.clone()
    }
}

/// One response time histogram for each of the last [`SERIES_WINDOW_SECS`]
/// seconds. Second `s` lives in slot `s % len`, tagged with `s` so a slot that
/// still holds an older second is recognized and reused, and with the
/// revision of its last change so a heatmap can send only what changed. The
/// newest seconds stay open, counted by fixed index, until a second two later
/// begins.
#[derive(Clone)]
pub(crate) struct LatencyRing {
    slots: Vec<Option<RingSlot>>,
    /// Seconds whose histograms are still open.
    open: Vec<usize>,
}

impl LatencyRing {
    pub(crate) fn new() -> Self {
        LatencyRing {
            slots: (0..SERIES_WINDOW_SECS).map(|_| None).collect(),
            open: Vec::new(),
        }
    }

    /// The histogram for `second`, replacing an older second's, marked as
    /// changed at `rev`. `None` when the slot already holds a newer second:
    /// the sample is too old to show.
    fn slot_mut(&mut self, second: usize, rev: u64) -> Option<&mut SecondHistogram> {
        // The window is a constant, so the remainder needs no division.
        let len = SERIES_WINDOW_SECS as usize;
        debug_assert_eq!(self.slots.len(), len);
        let at = second % len;
        match &self.slots[at] {
            Some(slot) if slot.second > second => return None,
            Some(slot) if slot.second == second => {}
            _ => {
                self.slots[at] = Some(RingSlot {
                    second,
                    rev,
                    data: SecondHistogram::open(),
                    cells: (0, Vec::new()),
                });
                // A new second has begun: those two or more seconds back
                // have all but their slowest requests, so pack them.
                let slots = &mut self.slots;
                self.open.retain(|&older| {
                    if older + 2 > second {
                        return true;
                    }
                    if let Some(slot) = &mut slots[older % len] {
                        if slot.second == older {
                            slot.data.pack();
                        }
                    }
                    false
                });
                self.open.push(second);
            }
        }
        self.slots[at].as_mut().map(|slot| {
            slot.rev = rev;
            &mut slot.data
        })
    }

    /// Record one response time in `second`, at `rev`.
    pub(crate) fn record(&mut self, second: usize, time: u64, rev: u64) {
        if let Some(data) = self.slot_mut(second, rev) {
            data.record(time);
        }
    }

    /// Add a batch's response times for one request name to `second`, at
    /// `rev` (unit tests; the metrics processor merges into two rings at once).
    #[cfg(test)]
    pub(crate) fn merge_timing(
        &mut self,
        second: usize,
        other: &GooseRequestMetricTimingData,
        rev: u64,
    ) {
        if let Some(data) = self.slot_mut(second, rev) {
            data.merge_timing(other);
        }
    }

    /// Forget every second.
    pub(crate) fn clear(&mut self) {
        for slot in self.slots.iter_mut() {
            *slot = None;
        }
        self.open.clear();
    }

    /// [`Self::slot`], to build and keep its heatmap cells.
    fn slot_at_mut(&mut self, second: usize) -> Option<&mut RingSlot> {
        self.slots[second % SERIES_WINDOW_SECS as usize]
            .as_mut()
            .filter(|slot| slot.second == second)
    }

    fn slot(&self, second: usize) -> Option<&RingSlot> {
        self.slots[second % SERIES_WINDOW_SECS as usize]
            .as_ref()
            .filter(|slot| slot.second == second)
    }

    /// The histogram for `second`, if the ring still holds it (unit tests).
    #[cfg(test)]
    pub(crate) fn get(&self, second: usize) -> Option<&SecondHistogram> {
        self.slot(second).map(|slot| &slot.data)
    }
}

/// Where a heatmap's measured columns come from.
trait MeasuredColumns {
    /// The revision `second` last changed at, if it has response times.
    fn rev(&self, second: usize) -> Option<u64>;
    /// `second`'s heatmap cells.
    fn cells(&mut self, second: usize) -> Vec<u64>;
}

impl MeasuredColumns for LatencyRing {
    fn rev(&self, second: usize) -> Option<u64> {
        self.slot(second).map(|slot| slot.rev)
    }

    fn cells(&mut self, second: usize) -> Vec<u64> {
        self.slot_at_mut(second)
            .map_or_else(Vec::new, RingSlot::heat_cells)
    }
}

/// All requests: every name's ring and the ring of names without one,
/// added up for a column only when a heatmap asks for it.
struct AllRequests<'a> {
    rings: &'a [LatencyRing],
    others: &'a LatencyRing,
    /// Each second's cells as of the revision they were built at.
    cells: &'a mut [Option<(usize, u64, Vec<u64>)>],
}

impl AllRequests<'_> {
    fn slots(&self, second: usize) -> impl Iterator<Item = &RingSlot> + '_ {
        self.rings
            .iter()
            .chain([self.others])
            .filter_map(move |ring| ring.slot(second))
    }
}

impl MeasuredColumns for AllRequests<'_> {
    fn rev(&self, second: usize) -> Option<u64> {
        self.slots(second).map(|slot| slot.rev).max()
    }

    fn cells(&mut self, second: usize) -> Vec<u64> {
        let Some(rev) = self.rev(second) else {
            return Vec::new();
        };
        let at = second % self.cells.len();
        match &self.cells[at] {
            Some((kept, built, cells)) if *kept == second && *built == rev => cells.clone(),
            _ => {
                let mut all = SecondHistogram::open();
                for slot in self.slots(second) {
                    all.merge(&slot.data);
                }
                let cells = all.heat_cells();
                self.cells[at] = Some((second, rev, cells.clone()));
                cells
            }
        }
    }
}

/// Per second response time histograms for the dashboard's heatmap: each of
/// the first [`MAX_REQUEST_SERIES`] request names, the names after them
/// together, all requests as their sum, and the same for synthetic times
/// while coordinated omission mitigation is on. A batch's response times for
/// a name go to one ring; the all requests column of a second is only added
/// up when a heatmap needs it.
pub(crate) struct LatencyHistograms {
    /// Requests whose names have no ring of their own.
    others: LatencyRing,
    /// The all requests heatmap's cells, kept by second (see [`AllRequests`]).
    all_cells: Vec<Option<(usize, u64, Vec<u64>)>>,
    /// Each admitted request name's ring, in the order names were admitted.
    rings: Vec<LatencyRing>,
    /// Admitted request names and the index of their ring in `rings`.
    by_request: HashMap<String, usize>,
    synthetic: Option<(LatencyRing, HashMap<String, LatencyRing>)>,
    /// Changes when the rings are cleared, so a client starts over.
    generation: u64,
    /// Counts changes: each record or merge marks the seconds it changes
    /// with the next value.
    rev: u64,
}

impl LatencyHistograms {
    /// `synthetic` keeps rings for coordinated omission mitigation's
    /// synthetic times too; pass it when mitigation is not `disabled`.
    pub(crate) fn new(synthetic: bool) -> Self {
        LatencyHistograms {
            others: LatencyRing::new(),
            all_cells: (0..SERIES_WINDOW_SECS).map(|_| None).collect(),
            rings: Vec::new(),
            by_request: HashMap::new(),
            synthetic: synthetic.then(|| (LatencyRing::new(), HashMap::new())),
            generation: next_heatmap_generation(),
            rev: 0,
        }
    }

    /// The revision of a change about to be made.
    fn next_rev(&mut self) -> u64 {
        self.rev += 1;
        self.rev
    }

    /// The index of `key`'s ring, admitting the name while there is room.
    fn ring_index(&mut self, key: &str) -> Option<usize> {
        if let Some(&index) = self.by_request.get(key) {
            return Some(index);
        }
        if self.rings.len() >= MAX_REQUEST_SERIES {
            return None;
        }
        self.rings.push(LatencyRing::new());
        self.by_request
            .insert(key.to_string(), self.rings.len() - 1);
        Some(self.rings.len() - 1)
    }

    /// The index of `key`'s ring, looked up through `cached` (a
    /// [`KeySeries`]'s `latency_ring`) and only by name the first time.
    fn cached_index(&mut self, key: &str, cached: &mut Option<Option<usize>>) -> Option<usize> {
        match *cached {
            Some(index) => index,
            None => {
                let index = self.ring_index(key);
                *cached = Some(index);
                index
            }
        }
    }

    /// The ring of the name with ring `index`, or of the names without one.
    fn ring(&mut self, index: Option<usize>) -> &mut LatencyRing {
        match index {
            Some(index) => &mut self.rings[index],
            None => &mut self.others,
        }
    }

    #[cfg(test)]
    fn record(&mut self, key: &str, second: usize, time: u64) {
        let rev = self.next_rev();
        let index = self.ring_index(key);
        self.ring(index).record(second, time, rev);
    }

    fn record_cached(
        &mut self,
        key: &str,
        cached: &mut Option<Option<usize>>,
        second: usize,
        time: u64,
    ) {
        let rev = self.next_rev();
        let index = self.cached_index(key, cached);
        self.ring(index).record(second, time, rev);
    }

    /// Merge a batch's timing data for `key` in `second` into the key's
    /// pending counts, moving what they held to its ring first when `second`
    /// is past both seconds they hold. Times for a second before them go to
    /// the ring directly.
    fn merge_pending(
        &mut self,
        key: &str,
        entry: &mut KeySeries,
        second: usize,
        data: &GooseRequestMetricTimingData,
    ) {
        let index = self.cached_index(key, &mut entry.latency_ring);
        let pending = &mut entry.pending;
        if !pending.held {
            pending.held = true;
            pending.second = second;
        }
        if second < pending.second {
            let rev = self.next_rev();
            if let Some(histogram) = self.ring(index).slot_mut(second, rev) {
                histogram.merge_timing(data);
            }
            return;
        }
        if second > pending.second + 1 {
            self.move_pending(index, pending);
            pending.second = second;
        }
        pending.add(second - pending.second, data);
    }

    /// Move `pending` to the ring of the name with ring `index`.
    fn move_pending(&mut self, index: Option<usize>, pending: &mut PendingLatency) {
        let rev = &mut self.rev;
        let ring = match index {
            Some(index) => &mut self.rings[index],
            None => &mut self.others,
        };
        pending.move_to(ring, || {
            *rev += 1;
            *rev
        });
    }

    /// All requests in `second`: every ring's histogram for it added up, if
    /// any ring holds it.
    #[cfg(test)]
    pub(crate) fn all_second(&self, second: usize) -> Option<SecondHistogram> {
        let mut slots = self
            .rings
            .iter()
            .chain([&self.others])
            .filter_map(|ring| ring.slot(second))
            .peekable();
        slots.peek()?;
        let mut all = SecondHistogram::default();
        for slot in slots {
            all.merge(&slot.data);
        }
        Some(all)
    }

    /// Record a synthetic time. A name gets a synthetic ring only if it has a
    /// measured one.
    fn record_synthetic(&mut self, key: &str, second: usize, time: u64) {
        let rev = self.next_rev();
        let Some((all, by_request)) = self.synthetic.as_mut() else {
            return;
        };
        all.record(second, time, rev);
        if !self.by_request.contains_key(key) {
            return;
        }
        if let Some(ring) = by_request.get_mut(key) {
            ring.record(second, time, rev);
        } else {
            let mut ring = LatencyRing::new();
            ring.record(second, time, rev);
            by_request.insert(key.to_string(), ring);
        }
    }

    /// Clear every ring and forget every name, so names are admitted again,
    /// and start a new generation.
    fn clear(&mut self) {
        self.others.clear();
        self.all_cells.iter_mut().for_each(|cells| *cells = None);
        self.rings.clear();
        self.by_request.clear();
        if let Some((all, by_request)) = self.synthetic.as_mut() {
            all.clear();
            by_request.clear();
        }
        self.generation = next_heatmap_generation();
    }

    /// The heatmap of one measured ring and, when `co_active` and synthetic
    /// rings are kept, its synthetic counterpart, over `start..=end`: only
    /// the seconds changed after `since` when it is a cursor of these
    /// histograms, otherwise every second with data.
    #[allow(clippy::too_many_arguments)]
    ///
    /// `generation` and `rev` are the histograms' own, passed in so the
    /// rings can be borrowed to build and keep each column's cells.
    fn heatmap(
        (generation, rev): (u64, u64),
        measured: &mut impl MeasuredColumns,
        mut synthetic: Option<&mut LatencyRing>,
        start: usize,
        end: usize,
        hold_from: usize,
        since: Option<HeatmapCursor>,
    ) -> Heatmap {
        let since = match since {
            Some(cursor) if cursor.generation == generation && cursor.rev <= rev => cursor.rev,
            _ => 0,
        };
        let mut columns = Vec::new();
        let mut last_index = FIXED_BUCKETS - 1;
        for second in start..=end {
            let real = measured.rev(second);
            let added = synthetic
                .as_deref_mut()
                .and_then(|ring| ring.slot_at_mut(second));
            if !real.is_some_and(|real| real > since) && !added.iter().any(|slot| slot.rev > since)
            {
                continue;
            }
            let real = match real {
                Some(_) => measured.cells(second),
                None => Vec::new(),
            };
            let added = added.map_or_else(Vec::new, RingSlot::heat_cells);
            for cells in [&real, &added] {
                if let Some(&index) = cells.iter().rev().nth(1) {
                    last_index = last_index.max(index as usize);
                }
            }
            columns.push(HeatmapColumn {
                second: second as u64,
                measured: real,
                synthetic: added,
            });
        }
        Heatmap {
            generation,
            rev,
            since,
            start_second: start as u64,
            end_second: end as u64,
            hold_from: hold_from as u64,
            buckets: bucket_table(last_index),
            columns,
        }
    }

    fn heatmap_all(
        &mut self,
        start: usize,
        end: usize,
        hold_from: usize,
        since: Option<HeatmapCursor>,
        co_active: bool,
    ) -> Heatmap {
        let synthetic = self
            .synthetic
            .as_mut()
            .filter(|_| co_active)
            .map(|(all, _)| all);
        let mut all = AllRequests {
            rings: &self.rings,
            others: &self.others,
            cells: &mut self.all_cells,
        };
        Self::heatmap(
            (self.generation, self.rev),
            &mut all,
            synthetic,
            start,
            end,
            hold_from,
            since,
        )
    }

    fn heatmap_request(
        &mut self,
        key: &str,
        start: usize,
        end: usize,
        hold_from: usize,
        since: Option<HeatmapCursor>,
        co_active: bool,
    ) -> Option<Heatmap> {
        let measured = &mut self.rings[*self.by_request.get(key)?];
        let synthetic = self
            .synthetic
            .as_mut()
            .filter(|_| co_active)
            .and_then(|(_, by_request)| by_request.get_mut(key));
        Some(Self::heatmap(
            (self.generation, self.rev),
            measured,
            synthetic,
            start,
            end,
            hold_from,
            since,
        ))
    }
}

/// Used to collect graph data during a load test.
pub(crate) struct GraphData {
    /// Counts requests per second for each request type.
    requests_per_second: ItemsPerSecond,
    /// Counts errors per second.
    errors_per_second: ItemsPerSecond,
    /// Maintains average response time per second.
    average_response_time_per_second: HashMap<String, TimeSeries<MovingAverage, f32>>,
    /// Number of transactions at the end of each second of the test.
    transactions_per_second: TimeSeries<usize, usize>,
    /// Number of scenarios at the end of each second of the test.
    scenarios_per_second: TimeSeries<usize, usize>,
    /// Number of users at the end of each second of the test.
    users_per_second: TimeSeries<usize, usize>,
    /// Response time histograms by second for the dashboard's heatmap.
    /// Only the metrics processor's copy has them, and only while the
    /// dashboard tracks percentiles over time.
    latency: Option<LatencyHistograms>,
}

impl GraphData {
    /// Create a new GraphData object.
    pub(crate) fn new() -> Self {
        trace!("new graph");
        GraphData {
            requests_per_second: ItemsPerSecond::new(),
            errors_per_second: ItemsPerSecond::new(),
            average_response_time_per_second: HashMap::new(),
            transactions_per_second: TimeSeries::new(),
            scenarios_per_second: TimeSeries::new(),
            users_per_second: TimeSeries::new(),
            latency: None,
        }
    }

    /// Create GraphData for the metrics processor: with latency histograms
    /// when `latency_series` is set, and synthetic ones too when
    /// `synthetic_latency` (coordinated omission mitigation is on).
    pub(crate) fn for_metrics_processor(latency_series: bool, synthetic_latency: bool) -> Self {
        let mut graph_data = GraphData::new();
        if latency_series {
            graph_data.latency = Some(LatencyHistograms::new(synthetic_latency));
        }
        graph_data
    }

    /// Reset per-second *rate* series after the increase→maintain metrics reset.
    ///
    /// Request/error/latency/transaction/scenario buckets restart so running
    /// metrics and HTML report rates reflect steady-state load only.
    ///
    /// **Active users are a level over the whole run**, not a rate: the users
    /// series is preserved so the live dashboard (and HTML active-users graph)
    /// still show the initial ramp and any later increases/decreases. Wiping
    /// users and reseeding second 0 with the final count made the chart look
    /// like the test had always been at full load.
    ///
    /// When dashboard pruning has already advanced the users series `origin`,
    /// rate series are recreated with that same origin so the next sample at
    /// absolute second *S* does not expand vectors from 0..S.
    pub(crate) fn reset_rate_series(&mut self) {
        let origin = self.users_per_second.origin;
        self.requests_per_second = ItemsPerSecond::with_origin(origin);
        self.errors_per_second = ItemsPerSecond::with_origin(origin);
        self.average_response_time_per_second = HashMap::new();
        self.transactions_per_second = TimeSeries::with_origin(origin);
        self.scenarios_per_second = TimeSeries::with_origin(origin);
        if let Some(latency) = self.latency.as_mut() {
            latency.clear();
        }
    }

    /// Record one measured response time for the heatmap, without
    /// counting a request (unit tests).
    #[cfg(test)]
    pub(crate) fn record_latency(&mut self, key: &str, second: usize, time: u64) {
        if let Some(latency) = self.latency.as_mut() {
            latency.record(key, second, time);
        }
    }

    /// Count one request in `second` and, when `latency` is given and the
    /// histograms are kept, record its response time for the heatmap.
    pub(crate) fn record_request_per_second_and_latency(
        &mut self,
        key: &str,
        second: usize,
        latency: Option<u64>,
    ) {
        let data = self.requests_per_second.increment(key, second, 1);
        debug!(
            "incremented second {second} for requests per second counter: {}",
            data.get(second)
        );
        if let (Some(time), Some(histograms)) = (latency, self.latency.as_mut()) {
            histograms.record_cached(key, &mut data.latency_ring, second, time);
        }
    }

    /// Merge a batch's count of requests in `second` and, when the
    /// histograms are kept and `times` is given, their response times (the
    /// batch's timing data for this request, every time from `second`),
    /// finding the request's ring through the same lookup that counts them.
    /// `next`, the batch's timing data for this request from the second
    /// after, is counted and kept the same way in `second + 1`.
    pub(crate) fn record_requests_per_second_and_latency_batch(
        &mut self,
        key: &str,
        second: usize,
        count: u32,
        times: Option<&GooseRequestMetricTimingData>,
        next: Option<&GooseRequestMetricTimingData>,
    ) {
        let data = self.requests_per_second.increment(key, second, count);
        if let (Some(times), Some(histograms)) = (times, self.latency.as_mut()) {
            if times.counter > 0 {
                histograms.merge_pending(key, data, second, times);
            }
        }
        if let Some(next) = next {
            data.increase_value(second + 1, next.counter as u32);
            if let Some(histograms) = self.latency.as_mut() {
                if next.counter > 0 {
                    histograms.merge_pending(key, data, second + 1, next);
                }
            }
        }
    }

    /// Move every request's pending response times to its ring, so the
    /// rings hold every time merged so far.
    fn move_pending_latency(&mut self) {
        let Some(histograms) = self.latency.as_mut() else {
            return;
        };
        for (key, entry) in self.requests_per_second.series.iter_mut() {
            if entry.pending.held {
                let index = histograms.cached_index(key, &mut entry.latency_ring);
                histograms.move_pending(index, &mut entry.pending);
            }
        }
    }

    /// Whether response times are kept by second for the dashboard.
    pub(crate) fn records_latency(&self) -> bool {
        self.latency.is_some()
    }

    /// Record one synthetic response time from coordinated omission
    /// mitigation, when synthetic histograms are kept.
    pub(crate) fn record_synthetic_latency(&mut self, key: &str, second: usize, time: u64) {
        if let Some(latency) = self.latency.as_mut() {
            latency.record_synthetic(key, second, time);
        }
    }

    /// Whether synthetic response times are kept for the heatmap.
    pub(crate) fn records_synthetic_latency(&self) -> bool {
        self.latency
            .as_ref()
            .is_some_and(|latency| latency.synthetic.is_some())
    }

    /// Keys (`"METHOD name"`) of the requests with their own latency series.
    pub(crate) fn latency_series_keys(&self) -> HashSet<&str> {
        self.latency
            .as_ref()
            .map(|latency| latency.by_request.keys().map(String::as_str).collect())
            .unwrap_or_default()
    }

    /// The latency histograms, if kept (unit tests).
    #[cfg(test)]
    pub(crate) fn latency_histograms(&mut self) -> Option<&LatencyHistograms> {
        self.latency.as_ref()
    }

    /// The measured histogram for `second`: all requests when `key` is
    /// `None`, otherwise `key`'s own ring (unit tests).
    #[cfg(test)]
    pub(crate) fn latency_second_for_test(
        &mut self,
        key: Option<&str>,
        second: usize,
    ) -> Option<SecondHistogram> {
        self.move_pending_latency();
        let latency = self.latency.as_ref()?;
        match key {
            None => latency.all_second(second),
            Some(key) => latency.rings[*latency.by_request.get(key)?]
                .get(second)
                .cloned(),
        }
    }

    /// The average response time for `key` in `second`, if recorded (unit
    /// tests).
    #[cfg(test)]
    pub(crate) fn average_in_second_for_test(&self, key: &str, second: usize) -> Option<f32> {
        self.average_response_time_per_second
            .get(key)
            .map(|series| series.get(second))
            .filter(|average| average.count > 0)
            .map(|average| average.average)
    }

    /// The requests per second count for `key` in `second` (unit tests).
    #[cfg(test)]
    pub(crate) fn requests_in_second_for_test(&self, key: &str, second: usize) -> u32 {
        self.requests_per_second
            .series
            .get(key)
            .map_or(0, |series| series.get(second))
    }

    /// Last absolute second with a users sample, if any.
    pub(crate) fn users_last_absolute_second(&self) -> Option<usize> {
        self.users_per_second.last_absolute_second()
    }

    /// Record requests per second metric, without latency (unit tests).
    #[cfg(test)]
    pub(crate) fn record_requests_per_second(&mut self, key: &str, second: usize) {
        let value = self
            .requests_per_second
            .initialize_or_increment(key, second, 1);
        debug!("incremented second {second} for requests per second counter: {value}");
    }

    /// Record errors per second metric.
    pub(crate) fn record_errors_per_second(&mut self, key: &str, second: usize) {
        let value = self
            .errors_per_second
            .initialize_or_increment(key, second, 1);

        debug!("incremented second {second} for errors per second counter: {value}");
    }

    /// Record average response time per second metric.
    pub(crate) fn record_average_response_time_per_second(
        &mut self,
        key: String,
        second: usize,
        response_time: u64,
    ) {
        let origin = self.users_per_second.origin;
        let data = self
            .average_response_time_per_second
            .entry(key)
            .or_insert_with(|| TimeSeries::with_origin(origin));
        data.increase_value(second, response_time as f32);

        debug!(
            "updated second {} for average response time per second: {}",
            second,
            data.get(second).average
        );
    }

    /// Record transactions per second metric.
    pub(crate) fn record_transactions_per_second(&mut self, second: usize) {
        self.transactions_per_second.increase_value(second, 1);

        debug!(
            "incremented second {} for transactions per second counter: {}",
            second,
            self.transactions_per_second.get(second)
        );
    }

    /// Record scenarios per second metric.
    pub(crate) fn record_scenarios_per_second(&mut self, second: usize) {
        self.scenarios_per_second.increase_value(second, 1);

        debug!(
            "incremented second {} for scenarios per second counter: {}",
            second,
            self.scenarios_per_second.get(second)
        );
    }

    /// Record pre-aggregated average response time for batch merging.
    ///
    /// Performs a proper weighted merge of the batch's total response time and
    /// count into the per-second moving average, avoiding the precision loss
    /// that would result from truncating the average to an integer. `next`
    /// is the batch's total response time and count for `second + 1`, merged
    /// through the same lookup.
    pub(crate) fn record_average_response_time_per_second_batch(
        &mut self,
        key: &str,
        second: usize,
        total_time: f64,
        count: u32,
        next: Option<(f64, u32)>,
    ) {
        let next = next.filter(|&(_, count)| count > 0);
        if count == 0 && next.is_none() {
            return;
        }
        let origin = self.users_per_second.origin;
        let data = self
            .average_response_time_per_second
            .entry(key.to_string())
            .or_insert_with(|| TimeSeries::with_origin(origin));
        for (second, total_time, count) in std::iter::once((second, total_time, count))
            .chain(next.map(|(total_time, count)| (second + 1, total_time, count)))
        {
            if count > 0 {
                let batch_avg = MovingAverage {
                    count,
                    average: total_time as f32 / count as f32,
                };
                data.merge_value(second, &batch_avg);
            }
        }
    }

    /// Record multiple transactions per second for batch merging.
    pub(crate) fn record_transactions_per_second_batch(&mut self, second: usize, count: usize) {
        self.transactions_per_second.increase_value(second, count);
    }

    /// Record multiple scenarios per second for batch merging.
    pub(crate) fn record_scenarios_per_second_batch(&mut self, second: usize, count: usize) {
        self.scenarios_per_second.increase_value(second, count);
    }

    /// Records number of users for a current second.
    pub(crate) fn record_users_per_second(&mut self, users: usize, second: usize) {
        self.users_per_second.set_and_maintain_last(second, users);
    }

    /// Record active users at `second` only if time is not going backwards.
    ///
    /// Used by the dashboard snapshot path to pin the current level at "now"
    /// without rewinding past samples (which would recreate mid-ramp V-glitches
    /// when combined with gap-fill).
    pub(crate) fn record_users_monotonic(&mut self, users: usize, second: usize) {
        if let Some(last) = self.users_per_second.last_absolute_second() {
            if second < last {
                return;
            }
        }
        self.users_per_second.set_and_maintain_last(second, users);
    }

    /// Convenience wrapper for unit tests (no end-second hint).
    ///
    /// Production paths always pass a series-clock hint via
    /// [`Self::export_series_window_until`].
    #[cfg(test)]
    pub(crate) fn export_series_window(&mut self, window_secs: u32) -> SeriesWindow {
        self.export_series_window_until(window_secs, None)
    }

    /// [`Self::export_series_window_with_heatmap`] for a client with no
    /// heatmap yet and coordinated omission mitigation without events (unit
    /// tests).
    #[cfg(test)]
    pub(crate) fn export_series_window_until(
        &mut self,
        window_secs: u32,
        end_second_hint: Option<usize>,
    ) -> SeriesWindow {
        self.export_series_window_with_heatmap(window_secs, end_second_hint, false, None)
    }

    /// The trailing window's first and last second, as
    /// [`Self::export_series_window_with_heatmap`] defines them, or `None`
    /// before anything is recorded.
    fn series_window_bounds(
        &self,
        window_secs: u32,
        end_second_hint: Option<usize>,
    ) -> Option<(usize, usize)> {
        if window_secs == 0 {
            return None;
        }
        let mut end_second: Option<usize> = end_second_hint;
        for series in self.requests_per_second.series.values() {
            if let Some(last) = series.last_absolute_second() {
                end_second = Some(end_second.map_or(last, |e| e.max(last)));
            }
        }
        if let Some(last) = self.users_per_second.last_absolute_second() {
            end_second = Some(end_second.map_or(last, |e| e.max(last)));
        }
        let end_second = end_second?;
        Some((
            end_second.saturating_sub(window_secs as usize - 1),
            end_second,
        ))
    }

    /// First second the heatmap does not draw yet: the newest seconds may
    /// still be missing their slowest requests.
    fn latency_hold_from(end_second: usize, end_second_hint: Option<usize>) -> usize {
        end_second_hint.unwrap_or(end_second).saturating_sub(1)
    }

    /// Export a trailing aggregate series window for the live dashboard.
    ///
    /// Normative algorithm:
    /// 1. `end_second` = max second index across RPS keys, users series, and
    ///    optional `end_second_hint` (series clock "now" from the main loop)
    /// 2. `start_second` = `end_second.saturating_sub(window_secs - 1)`
    /// 3. Allocate fixed-length RPS/FPS/latency buffers; for each per-key series
    ///    add its overlapping range into those buffers (and accumulate
    ///    request-count-weighted latency the same way)
    /// 4. Missing RPS/error/latency seconds contribute 0
    /// 5. **Users are a level, not a rate**: missing samples (including trailing
    ///    seconds where request metrics advanced past the last `RecordUsers`)
    ///    hold the last known active-user count — never drop to 0 solely because
    ///    the users series lags RPS by a few seconds.
    /// 6. With latency histograms kept, `heatmap` covers the same seconds:
    ///    the seconds that changed after `heatmap_since` when it is a cursor
    ///    of these histograms, otherwise every second, with synthetic times
    ///    only when `co_active` (coordinated omission mitigation has recorded
    ///    an event) and synthetic histograms are kept. Without latency
    ///    histograms it is `None`.
    ///
    /// `end_second_hint` extends the window through series-clock "now" when the
    /// last stored sample is a few seconds older (main-loop cadence / no recent
    /// requests).
    pub(crate) fn export_series_window_with_heatmap(
        &mut self,
        window_secs: u32,
        end_second_hint: Option<usize>,
        co_active: bool,
        heatmap_since: Option<HeatmapCursor>,
    ) -> SeriesWindow {
        self.move_pending_latency();
        let Some((start_second, end_second)) =
            self.series_window_bounds(window_secs, end_second_hint)
        else {
            let mut empty = SeriesWindow::empty();
            empty.heatmap = self
                .latency
                .as_mut()
                .map(|latency| latency.heatmap_all(0, 0, 0, heatmap_since, co_active));
            return empty;
        };
        let len = end_second - start_second + 1;

        // Allocate fixed-length outputs once, then accumulate per key into the
        // overlapping range. This is O(stored_samples_in_window) rather than
        // O(window_secs × unique_keys), which matters at ~1 Hz with many request
        // names.
        let mut rps = vec![0.0f64; len];
        let mut fps = vec![0.0f64; len];
        let mut avg_latency_ms = vec![0.0f64; len];
        let mut latency_weight = vec![0u64; len];

        for series in self.requests_per_second.series.values() {
            accumulate_rate_series_into_window(series, start_second, end_second, &mut rps);
        }
        for series in self.errors_per_second.series.values() {
            accumulate_rate_series_into_window(series, start_second, end_second, &mut fps);
        }

        // Weighted latency: for each key, walk the overlap of its RPS and latency
        // series with the export window and add into fixed output vectors.
        for (key, latency_series) in &self.average_response_time_per_second {
            let Some(rps_series) = self.requests_per_second.series.get(key) else {
                continue;
            };
            if latency_series.data.is_empty() || rps_series.data.is_empty() {
                continue;
            }
            let series_start = latency_series
                .origin
                .max(rps_series.origin)
                .max(start_second);
            let series_end = (latency_series.origin + latency_series.data.len() - 1)
                .min(rps_series.origin + rps_series.data.len() - 1)
                .min(end_second);
            if series_start > series_end {
                continue;
            }
            for s in series_start..=series_end {
                let count = rps_series.data[s - rps_series.origin] as u64;
                if count == 0 {
                    continue;
                }
                let avg = latency_series.data[s - latency_series.origin].average as f64;
                let idx = s - start_second;
                avg_latency_ms[idx] += avg * count as f64;
                latency_weight[idx] += count;
            }
        }
        for i in 0..len {
            if latency_weight[i] > 0 {
                avg_latency_ms[i] /= latency_weight[i] as f64;
            }
        }

        // Level hold: do not paint "0 users" for trailing seconds where only
        // RPS has advanced (main-loop RecordUsers lags drain_pending). Users are
        // a single series so O(window) is fine.
        let mut users = Vec::with_capacity(len);
        for s in start_second..=end_second {
            users.push(self.users_per_second.get_held(s) as u64);
        }

        let hold_from = Self::latency_hold_from(end_second, end_second_hint);
        let heatmap = self.latency.as_mut().map(|latency| {
            latency.heatmap_all(
                start_second,
                end_second,
                hold_from,
                heatmap_since,
                co_active,
            )
        });

        SeriesWindow {
            start_second: start_second as u64,
            rps,
            fps,
            users,
            avg_latency_ms,
            heatmap,
        }
    }

    /// One request's heatmap for `GET /api/v1/series`, over the same seconds
    /// as [`Self::export_series_window_with_heatmap`] and sent the same way.
    /// `None` when the request has no histograms of its own.
    pub(crate) fn export_request_series(
        &mut self,
        method: String,
        name: String,
        window_secs: u32,
        end_second_hint: Option<usize>,
        co_active: bool,
        since: Option<HeatmapCursor>,
    ) -> Option<RequestSeries> {
        self.latency.as_ref()?;
        self.move_pending_latency();
        let key = format!("{method} {name}");
        let (start_second, end_second) = self
            .series_window_bounds(window_secs, end_second_hint)
            .unwrap_or((0, 0));
        let latency = self.latency.as_mut()?;
        let heatmap = latency.heatmap_request(
            &key,
            start_second,
            end_second,
            Self::latency_hold_from(end_second, end_second_hint),
            since,
            co_active,
        )?;
        Some(RequestSeries {
            method,
            name,
            heatmap,
        })
    }

    /// Drop series samples older than the trailing `keep_secs` window.
    ///
    /// Used when the live dashboard is enabled without `--report-file`, so
    /// multi-hour soaks do not retain per-second history that the dashboard
    /// never exports (export window is [`SERIES_WINDOW_SECS`](crate::metrics::dashboard_snapshot::SERIES_WINDOW_SECS)).
    /// No-op when `keep_secs == 0`.
    ///
    /// After length-based pruning, all per-key series are **aligned** to
    /// `users_per_second.origin`. Length-only prune leaves idle keys (short
    /// `data`, origin still 0) alone; if such a key becomes active again at a
    /// large absolute second, `increase_value` would expand from the stale
    /// origin and defeat the memory bound. Alignment drops pre-window samples
    /// and advances each series origin to the retained window.
    pub(crate) fn prune_to_window(&mut self, keep_secs: u32) {
        let keep = keep_secs as usize;
        if keep == 0 {
            return;
        }
        for series in self.requests_per_second.series.values_mut() {
            series.prune_to_window(keep);
        }
        for series in self.errors_per_second.series.values_mut() {
            series.prune_to_window(keep);
        }
        for series in self.average_response_time_per_second.values_mut() {
            series.prune_to_window(keep);
        }
        self.transactions_per_second.prune_to_window(keep);
        self.scenarios_per_second.prune_to_window(keep);
        self.users_per_second.prune_to_window(keep);

        // Align every series (and the ItemsPerSecond new-key origin) to the
        // retained users window so idle keys cannot re-expand from second 0.
        let origin = self.users_per_second.origin;
        for series in self.requests_per_second.series.values_mut() {
            series.align_origin_to(origin);
        }
        for series in self.errors_per_second.series.values_mut() {
            series.align_origin_to(origin);
        }
        for series in self.average_response_time_per_second.values_mut() {
            series.align_origin_to(origin);
        }
        self.transactions_per_second.align_origin_to(origin);
        self.scenarios_per_second.align_origin_to(origin);
        self.requests_per_second.origin = origin;
        self.errors_per_second.origin = origin;
    }

    /// Max stored samples across RPS/users series (unit tests / diagnostics).
    #[cfg(test)]
    pub(crate) fn max_series_len_for_test(&self) -> usize {
        let mut max_len = self.users_per_second.data.len();
        for series in self.requests_per_second.series.values() {
            max_len = max_len.max(series.data.len());
        }
        for series in self.errors_per_second.series.values() {
            max_len = max_len.max(series.data.len());
        }
        max_len
            .max(self.transactions_per_second.data.len())
            .max(self.scenarios_per_second.data.len())
    }

    /// Generate active users graph.
    pub(crate) fn get_active_users_graph(&self, granular_data: bool) -> Graph<'_, usize, usize> {
        self.create_graph_from_single_data(
            "graph-active-users",
            "Active users #",
            granular_data,
            &self.users_per_second,
        )
    }

    /// Generate requests per second graph.
    pub(crate) fn get_requests_per_second_graph(&self, granular_data: bool) -> Graph<'_, u32, u32> {
        self.requests_per_second
            .graph("graph-rps", "Requests #", granular_data)
    }

    /// Generate average response time graph.
    pub(crate) fn get_average_response_time_graph(
        &self,
        granular_data: bool,
    ) -> Graph<'_, MovingAverage, f32> {
        self.create_graph_from_data(
            "graph-avg-response-time",
            "Response time [ms]",
            granular_data,
            &self.average_response_time_per_second,
        )
    }

    /// Generate active transactions graph.
    pub(crate) fn get_transactions_per_second_graph(
        &self,
        granular_data: bool,
    ) -> Graph<'_, usize, usize> {
        self.create_graph_from_single_data(
            "graph-tps",
            "Transactions #",
            granular_data,
            &self.transactions_per_second,
        )
    }

    /// Generate active scenarios graph.
    pub(crate) fn get_scenarios_per_second_graph(
        &self,
        granular_data: bool,
    ) -> Graph<'_, usize, usize> {
        self.create_graph_from_single_data(
            "graph-sps",
            "Scenarios #",
            granular_data,
            &self.scenarios_per_second,
        )
    }

    /// Generate errors per second graph.
    pub(crate) fn get_errors_per_second_graph(&self, granular_data: bool) -> Graph<'_, u32, u32> {
        self.errors_per_second
            .graph("graph-eps", "Errors #", granular_data)
    }

    /// Creates a Graph from granular data.
    fn create_graph_from_data<
        'a,
        T: Clone + TimeSeriesValue<T, U>,
        U: Serialize + Copy + PartialEq + PartialOrd,
    >(
        &self,
        html_id: &'a str,
        y_axis_label: &'a str,
        granular_data: bool,
        data: &'a HashMap<String, TimeSeries<T, U>>,
    ) -> Graph<'a, T, U> {
        Graph::new(html_id, y_axis_label, granular_data, data)
    }

    /// Creates a Graph from single (just total numbers, not granular) data.
    fn create_graph_from_single_data<
        'a,
        T: Clone + TimeSeriesValue<T, U>,
        U: Serialize + Copy + PartialEq + PartialOrd,
    >(
        &self,
        html_id: &'a str,
        y_axis_label: &'a str,
        granular_data: bool,
        data: &'a TimeSeries<T, U>,
    ) -> Graph<'a, T, U> {
        Graph {
            html_id,
            y_axis_label,
            granular_data,
            data: vec![("Total", data)],
        }
    }
}

/// Defines the HTML graph data.
#[derive(Debug)]
pub(crate) struct Graph<'a, T: Clone + TimeSeriesValue<T, U>, U: Serialize + Copy> {
    /// HTML ID of the graph's main wrapper.
    html_id: &'a str,
    /// Label of the y axis.
    y_axis_label: &'a str,
    /// Indicates whether the granular data should be displayed on graphs.
    granular_data: bool,
    /// Graph data, borrowed from [`GraphData`] in the iteration order of its
    /// map, so rendering a report does not copy every series.
    data: Vec<(&'a str, &'a TimeSeries<T, U>)>,
}

impl<'a, T: Clone + TimeSeriesValue<T, U>, U: Serialize + Copy + PartialEq + PartialOrd>
    Graph<'a, T, U>
{
    /// Creates a new Graph object.
    #[allow(clippy::too_many_arguments)]
    fn new(
        html_id: &'a str,
        y_axis_label: &'a str,
        granular_data: bool,
        data: &'a HashMap<String, TimeSeries<T, U>>,
    ) -> Graph<'a, T, U> {
        Graph {
            html_id,
            y_axis_label,
            granular_data,
            data: data
                .iter()
                .map(|(label, series)| (label.as_str(), series))
                .collect(),
        }
    }

    /// The series with this label, for tests.
    #[cfg(test)]
    fn series(&self, label: &str) -> Option<&TimeSeries<T, U>> {
        self.data
            .iter()
            .find(|(l, _)| *l == label)
            .map(|(_, series)| *series)
    }

    /// Helper function to build HTML charts powered by the
    /// [ECharts](https://echarts.apache.org) library.
    pub(crate) fn get_markup(
        self,
        history: &[TestPlanHistory],
        test_started_time: DateTime<Utc>,
    ) -> String {
        let mut steps = String::new();
        for step in history.windows(2) {
            let started = Local
                .timestamp_opt(step[0].timestamp.timestamp(), 0)
                // @TODO: Error handling
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            let stopped = Local
                .timestamp_opt(step[1].timestamp.timestamp(), 0)
                // @TODO: Error handling
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            match &step[0].action {
                // For increasing show the current number of users to the new number of users.
                TestPlanStepAction::Increasing => {
                    let _ = write!(
                        steps,
                        r#"[
                            {{
                                xAxis: '{started}',
                                itemStyle: {{ borderColor: 'rgba(44, 102, 79, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{started}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{started}',
                                itemStyle: {{ color: 'rgba(44, 102, 79, 0.05)' }},
                            }},
                            {{
                                xAxis: '{stopped}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{stopped}',
                                itemStyle: {{ borderColor: 'rgba(44, 102, 79, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{stopped}'
                            }}
                        ],"#,
                    );
                }
                // For decreasing show the new number of users from the current number of users.
                TestPlanStepAction::Decreasing | TestPlanStepAction::Canceling => {
                    let _ = write!(
                        steps,
                        r#"[
                            {{
                                xAxis: '{started}',
                                itemStyle: {{ borderColor: 'rgba(179, 65, 65, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{started}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{started}',
                                itemStyle: {{ color: 'rgba(179, 65, 65, 0.05)' }},
                            }},
                            {{
                                xAxis: '{stopped}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{stopped}',
                                itemStyle: {{ borderColor: 'rgba(179, 65, 65, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{stopped}'
                            }}
                        ],"#,
                    );
                }
                _ => {}
            }
        }

        let mut total_values: TimeSeries<T, U> = TimeSeries::new();
        if self.data.is_empty() {
            "<!-- no data, no legend -->".to_string()
        } else {
            let (legend, main_label, main_values, other_values) = if self.data.len() > 1 {
                // If we are dealing with a metric with granular data we need to calculate totals.
                for (_, single_data) in &self.data {
                    total_values.add_time_series(single_data);
                }

                // We will have multiple lines. We need to prepare the legend section on the graph
                // and create data series for all of them.
                let mut legend = vec!["Total"];

                let mut other_values = String::new();
                if self.granular_data {
                    // In order for this to sort correctly we need to flip label and time series since tuples
                    // are sorted lexicographically so we want time series to be the first element of the tuple.
                    for (sub_data, label) in self
                        .data
                        .iter()
                        .map(|&(label, sub_data)| (sub_data, label))
                        .sorted()
                        .rev()
                    {
                        legend.push(label);

                        let formatted_line = format!(
                            r#"{{
                                name: '{label}',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                data: {values},
                            }},
                        "#,
                            label = label,
                            values = json!(self.add_timestamp_to_html_graph_data(
                                &sub_data.get_graph_data(),
                                test_started_time
                            ))
                        );
                        other_values += formatted_line.as_str();
                    }
                    (
                        format!(
                            r#"legend: {{
                            type: '{legend_type}',
                            width: '75%',
                            data: {data},
                        }},"#,
                            legend_type = if self.data.len() > 4 {
                                "scroll"
                            } else {
                                "plain"
                            },
                            data = json!(legend)
                        ),
                        "Total",
                        &total_values,
                        other_values,
                    )
                } else {
                    ("".to_string(), "Total", &total_values, "".to_string())
                }
            } else {
                // If there is only one data series in the metric we simply display it.
                (
                    "".to_string(),
                    self.data[0].0,
                    self.data[0].1,
                    "".to_string(),
                )
            };

            format!(
                r#"<div class="graph">
                <div id="{html_id}" style="width: 1000px; height:500px; background: white;"></div>

                <script type="text/javascript">
                    var chartDom = document.getElementById('{html_id}');
                    var myChart = echarts.init(chartDom);

                    myChart.setOption({{
                        color: ['#2c664f', '#5470c6', '#91cc75', '#fac858', '#ee6666', '#73c0de', '#3ba272', '#fc8452', '#9a60b4', '#ea7ccc'],
                        tooltip: {{ trigger: 'axis' }},
                        toolbox: {{
                            feature: {{
                                dataZoom: {{ yAxisIndex: 'none' }},
                                restore: {{}},
                                saveAsImage: {{}}
                            }}
                        }},
                        dataZoom: [
                            {{
                                type: 'inside',
                                start: 0,
                                end: 100,
                                fillerColor: 'rgba(34, 80, 61, 0.25)',
                                selectedDataBackground: {{
                                    lineStyle: {{ color: '#2c664f' }},
                                    areaStyle: {{ color: '#378063' }}
                                }}
                            }},
                            {{
                                start: 0,
                                end: 100,
                                fillerColor: 'rgba(34, 80, 61, 0.25)',
                                selectedDataBackground: {{
                                    lineStyle: {{ color: '#2c664f' }},
                                    areaStyle: {{ color: '#378063' }}
                                }}
                            }},
                        ],
                        xAxis: {{ type: 'time' }},
                        yAxis: {{
                            name: '{y_axis_label}',
                            nameLocation: 'center',
                            nameRotate: 90,
                            nameGap: 45,
                            type: 'value'
                        }},
                        {legend}
                        series: [
                            {{
                                name: '{main_label}',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                lineStyle: {{ color: '#2c664f' }},
                                areaStyle: {{ color: '#378063' }},
                                markArea: {{
                                    data: [
                                        {steps}
                                    ]
                                }},
                                data: {main_values},
                            }},
                            {other_values}
                        ]
                    }});
                </script>
            </div>"#,
                html_id = self.html_id,
                main_values = json!(self.add_timestamp_to_html_graph_data(
                    &main_values.get_graph_data(),
                    test_started_time
                )),
                y_axis_label = self.y_axis_label,
                main_label = main_label,
                legend = legend,
                other_values = other_values,
                steps = steps,
            )
        }
    }

    /// Adds timestamps to the graph data series to ensure correct time display on x axis.
    ///
    /// Will take a vector of (generally numerical) values and convert them into tuples where
    /// the second element will be the data point and the first element will be formatted time
    /// it belongs to.
    fn add_timestamp_to_html_graph_data(
        &self,
        data: &[Option<U>],
        started: DateTime<Utc>,
    ) -> Vec<(String, U)> {
        data.iter()
            .enumerate()
            .map(|(second, value)| {
                (
                    Local
                        .timestamp_opt(second as i64 + started.timestamp(), 0)
                        // @TODO: Error handling
                        .unwrap()
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    *value,
                )
            })
            .filter_map(|(time, data_option)| data_option.map(|data| (time, data)))
            .collect::<Vec<_>>()
    }
}

/// Add one rate series' samples into a fixed-length export window buffer.
///
/// Only visits the absolute-second overlap between the series storage and
/// `[start_second, end_second]`. Missing buckets stay at their pre-zeroed value.
fn accumulate_rate_series_into_window(
    series: &TimeSeries<u32, u32>,
    start_second: usize,
    end_second: usize,
    out: &mut [f64],
) {
    if series.data.is_empty() {
        return;
    }
    let series_end = series.origin + series.data.len() - 1;
    let overlap_start = start_second.max(series.origin);
    let overlap_end = end_second.min(series_end);
    if overlap_start > overlap_end {
        return;
    }
    for s in overlap_start..=overlap_end {
        out[s - start_second] += series.data[s - series.origin] as f64;
    }
}

/// Data structure to represent time series data.
#[derive(Debug, Clone)]
struct TimeSeries<T: TimeSeriesValue<T, U>, U> {
    /// Time series data.
    ///
    /// Each element represents the value for one second in the time series.
    /// `data[i]` holds the sample for absolute second `origin + i`.
    ///
    /// Stored as a [`VecDeque`] so dashboard pruning can drop a prefix with
    /// amortized O(dropped) cost (`pop_front`) instead of O(retained) shifts
    /// from `Vec::drain(0..n)` on every 1 Hz prune across many request keys.
    data: VecDeque<T>,
    /// Absolute second corresponding to `data[0]`. Non-zero only after pruning
    /// (dashboard-only long soaks). Report-file collection never prunes, so this
    /// stays 0 and indices remain absolute seconds from test start.
    origin: usize,
    /// Total value of the time series (sum of all elements).
    total: T,
    /// Phantom data indicates to the compiler that the "U" generic data type has zero size.
    phantom: PhantomData<U>,
}

impl<T: Clone + TimeSeriesValue<T, U>, U: PartialEq + PartialOrd> Ord for TimeSeries<T, U> {
    fn cmp(&self, other: &Self) -> Ordering {
        let self_total = self.total();
        let other_total = other.total();

        if self_total > other_total {
            Ordering::Greater
        } else if self_total < other_total {
            Ordering::Less
        } else {
            Ordering::Equal
        }
    }
}

impl<T: Clone + TimeSeriesValue<T, U>, U: PartialEq + PartialOrd> PartialOrd for TimeSeries<T, U> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: Clone + TimeSeriesValue<T, U>, U: PartialEq> Eq for TimeSeries<T, U> {}

impl<T: Clone + TimeSeriesValue<T, U>, U: PartialEq> PartialEq for TimeSeries<T, U> {
    fn eq(&self, other: &Self) -> bool {
        self.total() == other.total()
    }
}

impl<T: Clone + TimeSeriesValue<T, U>, U> TimeSeries<T, U> {
    /// Creates a new TimeSeries object.
    fn new() -> TimeSeries<T, U> {
        Self::with_origin(0)
    }

    /// Creates an empty series whose first bucket is absolute second `origin`.
    ///
    /// Used after dashboard pruning / rate reset so inserts at absolute second *S*
    /// store at index `S - origin` instead of expanding from 0.
    fn with_origin(origin: usize) -> TimeSeries<T, U> {
        TimeSeries {
            data: VecDeque::new(),
            origin,
            phantom: PhantomData,
            total: T::initial_value(),
        }
    }

    /// Last absolute second that has a stored sample, if any.
    fn last_absolute_second(&self) -> Option<usize> {
        if self.data.is_empty() {
            None
        } else {
            Some(self.origin + self.data.len() - 1)
        }
    }

    /// Drop samples older than the trailing `keep_secs` window (by absolute second).
    ///
    /// After pruning, `origin` advances and `data` holds only the retained window so
    /// multi-hour dashboard-only soaks do not grow unbounded. No-op when the series
    /// is already within the window.
    ///
    /// Uses repeated [`VecDeque::pop_front`] so cost is O(dropped) rather than
    /// O(retained) from a front `Vec::drain` — important when many per-key series
    /// are pruned at ~1 Hz after already sitting at the keep window.
    ///
    /// **Limitation:** when `data.len() <= keep_secs` this is a no-op even if the
    /// series still has a stale low `origin` (idle keys). Callers that need a
    /// shared window must follow up with [`Self::align_origin_to`].
    fn prune_to_window(&mut self, keep_secs: usize) {
        if keep_secs == 0 || self.data.len() <= keep_secs {
            return;
        }
        let drop = self.data.len() - keep_secs;
        for _ in 0..drop {
            self.data.pop_front();
        }
        self.origin = self.origin.saturating_add(drop);
        // `total` still reflects historical increments (including dropped samples).
        // Dashboard export uses per-second buckets only; HTML reports never prune.
    }

    /// Advance `origin` to `target_origin`, dropping any samples strictly before it.
    ///
    /// Complements length-based [`Self::prune_to_window`]: idle per-key series keep
    /// a short `data` buffer and never length-prune, so without this step a late
    /// reappearance at absolute second *S* would expand from the stale origin.
    /// No-op when `target_origin <= self.origin`.
    fn align_origin_to(&mut self, target_origin: usize) {
        if target_origin <= self.origin {
            return;
        }
        if self.data.is_empty() {
            self.origin = target_origin;
            return;
        }
        let last = self.origin + self.data.len() - 1;
        if last < target_origin {
            // Entire series is before the retained window.
            self.data.clear();
            self.origin = target_origin;
            return;
        }
        let drop = target_origin - self.origin;
        for _ in 0..drop {
            self.data.pop_front();
        }
        self.origin = target_origin;
    }

    /// Increases the the value for a given second.
    fn increase_value(&mut self, second: usize, value: U) {
        // Samples that fall before the pruned window are dropped (dashboard-only).
        if second < self.origin {
            return;
        }
        self.expand(second, T::initial_value());
        let idx = second - self.origin;
        self.data[idx].increase_value(&value);
        self.total.increase_value(&value);
    }

    /// Merges a pre-aggregated value into a given second.
    fn merge_value(&mut self, second: usize, value: &T) {
        // Samples that fall before the pruned window are dropped (dashboard-only).
        if second < self.origin {
            return;
        }
        self.expand(second, T::initial_value());
        let idx = second - self.origin;
        self.data[idx].merge(value);
        self.total.merge(value);
    }

    /// Adds another time series.
    fn add_time_series(&mut self, other: &TimeSeries<T, U>) {
        for (offset, other_item) in other.data.iter().enumerate() {
            let second = other.origin + offset;
            if second < self.origin {
                continue;
            }
            self.expand(second, T::initial_value());
            let idx = second - self.origin;
            self.data.get_mut(idx).unwrap().merge(other_item);
            self.total.merge(other_item);
        }
    }

    /// Sets a value for a given second and maintains last recorded value if
    /// there is a gap in the time series.
    fn set_and_maintain_last(&mut self, second: usize, value: U) {
        if second < self.origin {
            return;
        }
        self.expand(second, self.last());
        self.total.increase_value(&value);
        let idx = second - self.origin;
        self.data[idx].set_value(value);
    }

    /// Returns a value for a given second.
    fn get(&self, second: usize) -> T {
        if second < self.origin {
            return T::initial_value();
        }
        match self.data.get(second - self.origin) {
            Some(value) => value.clone(),
            None => T::initial_value(),
        }
    }

    /// Level-style read: hold the nearest known sample when `second` is outside
    /// the stored range (before origin or past the last sample).
    ///
    /// Used for active-user counts, which are continuous levels. Rate series
    /// (RPS/errors) correctly use [`Self::get`] so missing buckets stay 0.
    fn get_held(&self, second: usize) -> T {
        if self.data.is_empty() {
            return T::initial_value();
        }
        if second < self.origin {
            return self.data[0].clone();
        }
        let last_abs = self.origin + self.data.len() - 1;
        if second > last_abs {
            return self.last();
        }
        self.data[second - self.origin].clone()
    }

    /// Returns the last value in the time series or initial value if the time
    /// series is empty.
    fn last(&self) -> T {
        match self.data.back() {
            Some(last) => last.clone(),
            None => T::initial_value(),
        }
    }

    /// Gets time series suitable for usage in HTML graphs (generally each value
    /// becomes some kind of a scalar).
    ///
    /// Report generation never prunes (`origin == 0`), so the returned vector is
    /// indexed by absolute second from test start. If pruning were used with
    /// reports, callers would need to pad `origin` leading empties; that path is
    /// intentionally unsupported.
    fn get_graph_data(&self) -> Vec<Option<U>> {
        self.data
            .iter()
            .map(|value| value.get_graph_value())
            .collect::<Vec<_>>()
    }

    /// Expands vectors that collect per-second data for HTML report graphs with a
    /// default value.
    ///
    /// We need to do that since we don't know for how long the load test will run
    /// and we can't initialize these vectors at the beginning. It is also
    /// better to do it as we go to save memory.
    ///
    /// `second` is absolute (from test start). Storage index is `second - origin`.
    fn expand(&mut self, second: usize, initial: T) {
        debug_assert!(second >= self.origin);
        let idx = second - self.origin;
        if self.data.len() <= idx {
            for _ in 0..(idx - self.data.len() + 1) {
                self.data.push_back(initial.clone());
                self.total.merge(&initial);
            }
        };
    }

    /// Gets time series total value (sum of all values).
    fn total(&self) -> U {
        self.total.get_total_value()
    }
}

/// Defines a single value in a TimeSeries.
pub trait TimeSeriesValue<T, U> {
    /// Initial ("zero") value.
    fn initial_value() -> T;
    /// Adds the given value to the current value.
    fn increase_value(&mut self, value: &U);
    /// Sets the value (and drops existing one if present).
    fn set_value(&mut self, value: U);
    /// Merges (adds) another TimeSeriesValue.
    fn merge(&mut self, other: &T);
    /// Gets representation of the value suitable for HTML graphs (generally a scalar).
    fn get_graph_value(&self) -> Option<U>;
    fn get_total_value(&self) -> U;
}

impl TimeSeriesValue<usize, usize> for usize {
    fn initial_value() -> usize {
        0
    }
    fn increase_value(&mut self, value: &usize) {
        *self += *value;
    }
    fn set_value(&mut self, value: usize) {
        *self = value;
    }
    fn merge(&mut self, other: &usize) {
        *self += *other;
    }
    fn get_graph_value(&self) -> Option<usize> {
        match *self == 0 {
            true => None,
            false => Some(*self),
        }
    }
    fn get_total_value(&self) -> usize {
        *self
    }
}

impl TimeSeriesValue<u32, u32> for u32 {
    fn initial_value() -> u32 {
        0
    }
    fn increase_value(&mut self, value: &u32) {
        *self += *value;
    }
    fn set_value(&mut self, value: u32) {
        *self = value;
    }
    fn merge(&mut self, other: &u32) {
        *self += *other;
    }
    fn get_graph_value(&self) -> Option<u32> {
        match *self == 0 {
            true => None,
            false => Some(*self),
        }
    }
    fn get_total_value(&self) -> u32 {
        *self
    }
}

impl TimeSeriesValue<MovingAverage, f32> for MovingAverage {
    fn initial_value() -> MovingAverage {
        MovingAverage::new()
    }
    fn increase_value(&mut self, value: &f32) {
        self.add_item(value);
    }
    fn set_value(&mut self, _: f32) {
        panic!("TimeSeriesValue::set() is not supported for MovingAverage");
    }
    fn merge(&mut self, other: &MovingAverage) {
        let total_count = self.count + other.count;
        if total_count == 0 {
            self.average = 0.;
        } else {
            self.average = self.average * (self.count as f32 / total_count as f32)
                + other.average * (other.count as f32 / total_count as f32);
        };
        self.count = total_count;
    }
    fn get_graph_value(&self) -> Option<f32> {
        match self.average == 0f32 {
            true => None,
            false => Some(self.average),
        }
    }
    fn get_total_value(&self) -> f32 {
        self.average
    }
}

/// Data structure to maintain moving averages.
///
/// It will maintain the current average and the number of data items that
/// were used to compute it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MovingAverage {
    /// Number of data items that were used to compute the current average.
    count: u32,
    /// Current average.
    average: f32,
}

impl MovingAverage {
    /// Create a new MovingAverage object.
    fn new() -> Self {
        MovingAverage {
            count: 0,
            average: 0.,
        }
    }

    /// Adds a new data item and calculates the new average.
    fn add_item(&mut self, item: &f32) {
        self.count += 1;
        self.average += (*item - self.average) / self.count as f32;
    }
}

impl Default for MovingAverage {
    /// Creates an empty moving average.
    fn default() -> MovingAverage {
        MovingAverage::new()
    }
}

impl Ord for MovingAverage {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.average > other.average {
            Ordering::Greater
        } else if self.average < other.average {
            Ordering::Less
        } else {
            Ordering::Equal
        }
    }
}

impl PartialOrd for MovingAverage {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for MovingAverage {
    fn eq(&self, other: &Self) -> bool {
        self.average == other.average
    }
}

impl Eq for MovingAverage {}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_graph_setters() {
        let mut graph = GraphData::new();
        graph.requests_per_second.insert(
            "GET /",
            TimeSeries {
                origin: 0,
                data: VecDeque::from(vec![123, 234, 345, 456, 567]),
                phantom: PhantomData,
                total: 0,
            },
        );
        graph.users_per_second = TimeSeries {
            origin: 0,
            data: VecDeque::from(vec![345, 456, 567, 123, 234]),
            phantom: PhantomData,
            total: 0,
        };
        graph.average_response_time_per_second.insert(
            "GET /".to_string(),
            TimeSeries {
                origin: 0,
                data: VecDeque::from(vec![
                    MovingAverage {
                        count: 123,
                        average: 1.23,
                    },
                    MovingAverage {
                        count: 234,
                        average: 2.34,
                    },
                    MovingAverage {
                        count: 345,
                        average: 3.45,
                    },
                    MovingAverage {
                        count: 456,
                        average: 4.56,
                    },
                    MovingAverage {
                        count: 567,
                        average: 5.67,
                    },
                ]),
                phantom: PhantomData,
                total: MovingAverage {
                    count: 0,
                    average: 0.,
                },
            },
        );
        graph.transactions_per_second = TimeSeries {
            origin: 0,
            data: VecDeque::from(vec![345, 123, 234, 456, 567]),
            phantom: PhantomData,
            total: 0,
        };

        graph.scenarios_per_second = TimeSeries {
            origin: 0,
            data: VecDeque::from(vec![345, 123, 234, 456, 567]),
            phantom: PhantomData,
            total: 0,
        };

        graph.errors_per_second.insert(
            "GET /",
            TimeSeries {
                origin: 0,
                data: VecDeque::from(vec![567, 123, 234, 345, 456]),
                phantom: PhantomData,
                total: 0,
            },
        );

        let rps_graph = graph.get_requests_per_second_graph(true);
        let expected_time_series: TimeSeries<u32, u32> = TimeSeries {
            origin: 0,
            data: VecDeque::from(vec![123, 234, 345, 456, 567]),
            phantom: PhantomData,
            total: 0,
        };
        assert_eq!(
            rps_graph.series("GET /").unwrap().clone(),
            expected_time_series
        );
        assert_eq!(rps_graph.html_id, "graph-rps");
        assert_eq!(rps_graph.y_axis_label, "Requests #");

        let users_graph = graph.get_active_users_graph(true);
        let expected_time_series: TimeSeries<usize, usize> = TimeSeries {
            origin: 0,
            data: VecDeque::from(vec![345, 456, 567, 123, 234]),
            phantom: PhantomData,
            total: 0,
        };
        assert_eq!(
            users_graph.series("Total").unwrap().clone(),
            expected_time_series
        );
        assert_eq!(users_graph.html_id, "graph-active-users");
        assert_eq!(users_graph.y_axis_label, "Active users #");

        let avg_rt_graph = graph.get_average_response_time_graph(true);
        let expected_time_series: TimeSeries<MovingAverage, f32> = TimeSeries {
            origin: 0,
            data: VecDeque::from(vec![
                MovingAverage {
                    count: 123,
                    average: 1.23,
                },
                MovingAverage {
                    count: 234,
                    average: 2.34,
                },
                MovingAverage {
                    count: 345,
                    average: 3.45,
                },
                MovingAverage {
                    count: 456,
                    average: 4.56,
                },
                MovingAverage {
                    count: 567,
                    average: 5.67,
                },
            ]),
            phantom: PhantomData,
            total: MovingAverage {
                count: 0,
                average: 0.,
            },
        };
        assert_eq!(
            avg_rt_graph.series("GET /").unwrap().clone(),
            expected_time_series
        );
        assert_eq!(avg_rt_graph.html_id, "graph-avg-response-time");
        assert_eq!(avg_rt_graph.y_axis_label, "Response time [ms]");

        let transactions_graph = graph.get_transactions_per_second_graph(true);
        let expected_time_series: TimeSeries<usize, usize> = TimeSeries {
            origin: 0,
            data: VecDeque::from(vec![345, 123, 234, 456, 567]),
            phantom: PhantomData,
            total: 0,
        };
        assert_eq!(
            transactions_graph.series("Total").unwrap().clone(),
            expected_time_series
        );
        assert_eq!(transactions_graph.html_id, "graph-tps");
        assert_eq!(transactions_graph.y_axis_label, "Transactions #");

        let scenarios_graph = graph.get_scenarios_per_second_graph(true);
        let expected_time_series: TimeSeries<usize, usize> = TimeSeries {
            origin: 0,
            data: VecDeque::from(vec![345, 123, 234, 456, 567]),
            phantom: PhantomData,
            total: 0,
        };
        assert_eq!(
            scenarios_graph.series("Total").unwrap().clone(),
            expected_time_series
        );
        assert_eq!(scenarios_graph.html_id, "graph-sps");
        assert_eq!(scenarios_graph.y_axis_label, "Scenarios #");

        let errors_graph = graph.get_errors_per_second_graph(true);
        let expected_time_series: TimeSeries<u32, u32> = TimeSeries {
            origin: 0,
            data: VecDeque::from(vec![567, 123, 234, 345, 456]),
            phantom: PhantomData,
            total: 0,
        };

        assert_eq!(
            errors_graph.series("GET /").unwrap().clone(),
            expected_time_series
        );
        assert_eq!(errors_graph.html_id, "graph-eps");
        assert_eq!(errors_graph.y_axis_label, "Errors #");
    }

    #[test]
    fn test_record_requests_per_second() {
        // Should be initialized with empty requests per second vector.
        let mut graph = GraphData::new();
        assert_eq!(graph.requests_per_second.len(), 0);

        graph.record_requests_per_second("GET /", 0);
        graph.record_requests_per_second("GET /", 0);
        graph.record_requests_per_second("GET /", 0);
        graph.record_requests_per_second("GET /", 1);
        graph.record_requests_per_second("GET /", 2);
        graph.record_requests_per_second("GET /", 2);
        graph.record_requests_per_second("GET /", 2);
        graph.record_requests_per_second("GET /", 2);
        graph.record_requests_per_second("GET /", 2);
        assert_eq!(
            graph.requests_per_second.get("GET /").unwrap().data.len(),
            3
        );
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[0], 3);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[1], 1);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[2], 5);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().total(), 9);

        graph.record_requests_per_second("GET /", 100);
        graph.record_requests_per_second("GET /", 100);
        graph.record_requests_per_second("GET /", 100);
        graph.record_requests_per_second("GET /", 0);
        graph.record_requests_per_second("GET /", 1);
        graph.record_requests_per_second("GET /", 2);
        graph.record_requests_per_second("GET /", 5);
        assert_eq!(
            graph.requests_per_second.get("GET /").unwrap().data.len(),
            101
        );
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[0], 4);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[1], 2);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[2], 6);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[3], 0);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[4], 0);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[5], 1);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[100], 3);
        for second in 6..100 {
            assert_eq!(
                graph.requests_per_second.get("GET /").unwrap().data[second],
                0
            );
        }
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().total(), 16);

        graph.record_requests_per_second("GET /user", 0);
        graph.record_requests_per_second("GET /user", 1);
        graph.record_requests_per_second("GET /user", 1);
        graph.record_requests_per_second("GET /", 2);
        graph.record_requests_per_second("GET /", 5);
        assert_eq!(
            graph
                .requests_per_second
                .get("GET /user")
                .unwrap()
                .data
                .len(),
            2
        );
        assert_eq!(
            graph.requests_per_second.get("GET /user").unwrap().data[0],
            1
        );
        assert_eq!(
            graph.requests_per_second.get("GET /user").unwrap().data[1],
            2
        );

        assert_eq!(
            graph.requests_per_second.get("GET /").unwrap().data.len(),
            101
        );
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[0], 4);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[1], 2);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[2], 7);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[3], 0);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[4], 0);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[5], 2);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[100], 3);
        for second in 6..100 {
            assert_eq!(
                graph.requests_per_second.get("GET /").unwrap().data[second],
                0
            );
        }
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().total(), 18);
        assert_eq!(
            graph.requests_per_second.get("GET /user").unwrap().total(),
            3
        );

        graph.record_requests_per_second("GET /user", 100);
        graph.record_requests_per_second("GET /user", 0);
        graph.record_requests_per_second("GET /user", 1);
        graph.record_requests_per_second("GET /", 0);
        graph.record_requests_per_second("GET /", 1);
        assert_eq!(
            graph
                .requests_per_second
                .get("GET /user")
                .unwrap()
                .data
                .len(),
            101
        );
        assert_eq!(
            graph.requests_per_second.get("GET /user").unwrap().data[0],
            2
        );
        assert_eq!(
            graph.requests_per_second.get("GET /user").unwrap().data[1],
            3
        );
        assert_eq!(
            graph.requests_per_second.get("GET /user").unwrap().data[100],
            1
        );
        for second in 6..100 {
            assert_eq!(
                graph.requests_per_second.get("GET /user").unwrap().data[second],
                0
            );
        }

        assert_eq!(
            graph.requests_per_second.get("GET /").unwrap().data.len(),
            101
        );
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[0], 5);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[1], 3);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[2], 7);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[3], 0);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[4], 0);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[5], 2);
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().data[100], 3);
        for second in 6..100 {
            assert_eq!(
                graph.requests_per_second.get("GET /").unwrap().data[second],
                0
            );
        }
        assert_eq!(graph.requests_per_second.get("GET /").unwrap().total(), 20);
        assert_eq!(
            graph.requests_per_second.get("GET /user").unwrap().total(),
            6
        );
    }

    #[test]
    fn test_record_errors_per_second() {
        // Should be initialized with empty errors per second vector.
        let mut graph = GraphData::new();
        assert_eq!(graph.errors_per_second.len(), 0);

        graph.record_errors_per_second("GET /", 0);
        graph.record_errors_per_second("GET /", 0);
        graph.record_errors_per_second("GET /", 0);
        graph.record_errors_per_second("GET /", 1);
        graph.record_errors_per_second("GET /", 2);
        graph.record_errors_per_second("GET /", 2);
        graph.record_errors_per_second("GET /", 2);
        graph.record_errors_per_second("GET /", 2);
        graph.record_errors_per_second("GET /", 2);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data.len(), 3);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[0], 3);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[1], 1);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[2], 5);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().total(), 9);

        graph.record_errors_per_second("GET /", 100);
        graph.record_errors_per_second("GET /", 100);
        graph.record_errors_per_second("GET /", 100);
        graph.record_errors_per_second("GET /", 0);
        graph.record_errors_per_second("GET /", 1);
        graph.record_errors_per_second("GET /", 2);
        graph.record_errors_per_second("GET /", 5);
        assert_eq!(
            graph.errors_per_second.get("GET /").unwrap().data.len(),
            101
        );
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[0], 4);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[1], 2);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[2], 6);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[3], 0);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[4], 0);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[5], 1);
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().data[100], 3);
        for second in 6..100 {
            assert_eq!(
                graph.errors_per_second.get("GET /").unwrap().data[second],
                0
            );
        }
        assert_eq!(graph.errors_per_second.get("GET /").unwrap().total(), 16);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn test_record_average_response_time_per_second() {
        // Should be initialized with empty average response time per second vector.
        let mut graph = GraphData::new();
        assert_eq!(graph.average_response_time_per_second.len(), 0);

        graph.record_average_response_time_per_second("GET /".to_string(), 0, 5);
        graph.record_average_response_time_per_second("GET /".to_string(), 0, 4);
        graph.record_average_response_time_per_second("GET /".to_string(), 0, 3);
        graph.record_average_response_time_per_second("GET /".to_string(), 1, 1);
        graph.record_average_response_time_per_second("GET /".to_string(), 2, 4);
        graph.record_average_response_time_per_second("GET /".to_string(), 2, 8);
        graph.record_average_response_time_per_second("GET /".to_string(), 2, 12);
        graph.record_average_response_time_per_second("GET /".to_string(), 2, 4);
        graph.record_average_response_time_per_second("GET /".to_string(), 2, 4);
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data
                .len(),
            3
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[0]
                .average,
            4.
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[1]
                .average,
            1.
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[2]
                .average,
            6.4
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .total(),
            5.0000005
        );

        graph.record_average_response_time_per_second("GET /".to_string(), 100, 5);
        graph.record_average_response_time_per_second("GET /".to_string(), 100, 9);
        graph.record_average_response_time_per_second("GET /".to_string(), 100, 7);
        graph.record_average_response_time_per_second("GET /".to_string(), 0, 2);
        graph.record_average_response_time_per_second("GET /".to_string(), 1, 2);
        graph.record_average_response_time_per_second("GET /".to_string(), 2, 5);
        graph.record_average_response_time_per_second("GET /".to_string(), 5, 2);
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data
                .len(),
            101
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[0]
                .average,
            3.5
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[1]
                .average,
            1.5
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[2]
                .average,
            6.166667
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[3]
                .average,
            0.
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[4]
                .average,
            0.
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[5]
                .average,
            2.
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .data[100]
                .average,
            7.
        );
        for second in 6..100 {
            assert_eq!(
                graph
                    .average_response_time_per_second
                    .get("GET /")
                    .unwrap()
                    .data[second]
                    .average,
                0.
            );
        }
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /")
                .unwrap()
                .total(),
            4.8125005
        );
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn test_record_average_response_time_per_second_batch() {
        let mut graph = GraphData::new();

        // Batch merge: 3 requests with total_time 60.0 → avg 20.0
        graph.record_average_response_time_per_second_batch("GET /", 0, 60.0, 3, None);
        let avg = graph
            .average_response_time_per_second
            .get("GET /")
            .unwrap()
            .get(0);
        assert_eq!(avg.average, 20.0);
        assert_eq!(avg.count, 3);

        // Add a single individual request with response time 40.
        graph.record_average_response_time_per_second("GET /".to_string(), 0, 40);
        let avg = graph
            .average_response_time_per_second
            .get("GET /")
            .unwrap()
            .get(0);
        // Weighted merge: (3 * 20 + 1 * 40) / 4 = 25.0
        assert_eq!(avg.average, 25.0);
        assert_eq!(avg.count, 4);

        // Another batch at a different second.
        graph.record_average_response_time_per_second_batch("GET /", 5, 100.0, 2, None);
        let avg = graph
            .average_response_time_per_second
            .get("GET /")
            .unwrap()
            .get(5);
        assert_eq!(avg.average, 50.0);
        assert_eq!(avg.count, 2);

        // Merge a second batch into the same second.
        graph.record_average_response_time_per_second_batch("GET /", 5, 200.0, 2, None);
        let avg = graph
            .average_response_time_per_second
            .get("GET /")
            .unwrap()
            .get(5);
        // Weighted: (2 * 50 + 2 * 100) / 4 = 75.0
        assert_eq!(avg.average, 75.0);
        assert_eq!(avg.count, 4);
    }

    #[test]
    fn test_record_transactions_per_second() {
        // Should be initialized with empty transactions per second vector.
        let mut graph = GraphData::new();
        assert_eq!(graph.transactions_per_second.data.len(), 0);

        graph.record_transactions_per_second(0);
        graph.record_transactions_per_second(0);
        graph.record_transactions_per_second(0);
        graph.record_transactions_per_second(1);
        graph.record_transactions_per_second(2);
        graph.record_transactions_per_second(2);
        graph.record_transactions_per_second(2);
        graph.record_transactions_per_second(2);
        graph.record_transactions_per_second(2);
        assert_eq!(graph.transactions_per_second.data.len(), 3);
        assert_eq!(graph.transactions_per_second.data[0], 3);
        assert_eq!(graph.transactions_per_second.data[1], 1);
        assert_eq!(graph.transactions_per_second.data[2], 5);
        assert_eq!(graph.transactions_per_second.total(), 9);

        graph.record_transactions_per_second(100);
        graph.record_transactions_per_second(100);
        graph.record_transactions_per_second(100);
        graph.record_transactions_per_second(0);
        graph.record_transactions_per_second(1);
        graph.record_transactions_per_second(2);
        graph.record_transactions_per_second(5);
        assert_eq!(graph.transactions_per_second.data.len(), 101);
        assert_eq!(graph.transactions_per_second.data[0], 4);
        assert_eq!(graph.transactions_per_second.data[1], 2);
        assert_eq!(graph.transactions_per_second.data[2], 6);
        assert_eq!(graph.transactions_per_second.data[3], 0);
        assert_eq!(graph.transactions_per_second.data[4], 0);
        assert_eq!(graph.transactions_per_second.data[5], 1);
        assert_eq!(graph.transactions_per_second.data[100], 3);
        for second in 6..100 {
            assert_eq!(graph.transactions_per_second.data[second], 0);
        }
        assert_eq!(graph.transactions_per_second.total(), 16);
    }

    #[test]
    fn test_record_users_per_second() {
        // Should be initialized with empty transactions per second vector.
        let mut graph = GraphData::new();
        assert_eq!(graph.users_per_second.data.len(), 0);

        graph.record_users_per_second(1, 0);
        graph.record_users_per_second(1, 1);
        graph.record_users_per_second(2, 2);
        assert_eq!(graph.users_per_second.data.len(), 3);
        assert_eq!(graph.users_per_second.data[0], 1);
        assert_eq!(graph.users_per_second.data[1], 1);
        assert_eq!(graph.users_per_second.data[2], 2);
        assert_eq!(graph.users_per_second.total(), 6);

        graph.record_users_per_second(5, 5);
        graph.record_users_per_second(10, 37);
        assert_eq!(graph.users_per_second.data.len(), 38);
        assert_eq!(graph.users_per_second.data[0], 1);
        assert_eq!(graph.users_per_second.data[1], 1);
        assert_eq!(graph.users_per_second.data[2], 2);
        assert_eq!(graph.users_per_second.data[3], 2);
        assert_eq!(graph.users_per_second.data[4], 2);
        assert_eq!(graph.users_per_second.data[5], 5);
        assert_eq!(graph.users_per_second.data[37], 10);
        for second in 6..36 {
            assert_eq!(graph.users_per_second.data[second], 5);
        }
        assert_eq!(graph.users_per_second.total(), 187);
    }

    #[test]
    fn test_export_series_window_empty() {
        let mut graph = GraphData::new();
        let window = graph.export_series_window(300);
        assert_eq!(window, SeriesWindow::empty());
        assert_eq!(window.start_second, 0);
        assert!(window.rps.is_empty());
        assert!(window.fps.is_empty());
        assert!(window.users.is_empty());
        assert!(window.avg_latency_ms.is_empty());
    }

    #[test]
    fn test_export_series_window_multi_key_reduction() {
        let mut graph = GraphData::new();

        // Key A: 2 requests at second 0 (latency 10ms each), 1 request at second 2 (latency 30ms).
        graph.record_requests_per_second("GET /a", 0);
        graph.record_requests_per_second("GET /a", 0);
        graph.record_average_response_time_per_second("GET /a".to_string(), 0, 10);
        graph.record_average_response_time_per_second("GET /a".to_string(), 0, 10);
        graph.record_requests_per_second("GET /a", 2);
        graph.record_average_response_time_per_second("GET /a".to_string(), 2, 30);

        // Key B: 3 requests at second 2 only (latency 50ms), disjoint from A's second-0 traffic.
        graph.record_requests_per_second("GET /b", 2);
        graph.record_requests_per_second("GET /b", 2);
        graph.record_requests_per_second("GET /b", 2);
        graph.record_average_response_time_per_second("GET /b".to_string(), 2, 50);
        graph.record_average_response_time_per_second("GET /b".to_string(), 2, 50);
        graph.record_average_response_time_per_second("GET /b".to_string(), 2, 50);

        // One error on key B at second 2.
        graph.record_errors_per_second("GET /b", 2);

        graph.record_users_per_second(1, 0);
        graph.record_users_per_second(4, 2);

        let window = graph.export_series_window(300);
        assert_eq!(window.start_second, 0);
        assert_eq!(window.rps.len(), 3);
        assert_eq!(window.fps.len(), 3);
        assert_eq!(window.users.len(), 3);
        assert_eq!(window.avg_latency_ms.len(), 3);

        // Second 0: 2 RPS from A only; latency 10ms.
        assert!((window.rps[0] - 2.0).abs() < f64::EPSILON);
        assert!((window.fps[0] - 0.0).abs() < f64::EPSILON);
        assert_eq!(window.users[0], 1);
        assert!((window.avg_latency_ms[0] - 10.0).abs() < 0.001);

        // Second 1: empty → 0 (gap filled by users maintain-last, but RPS/latency 0).
        assert!((window.rps[1] - 0.0).abs() < f64::EPSILON);
        assert!((window.fps[1] - 0.0).abs() < f64::EPSILON);
        // users_per_second.set_and_maintain_last fills gap with last value (1).
        assert_eq!(window.users[1], 1);
        assert!((window.avg_latency_ms[1] - 0.0).abs() < f64::EPSILON);

        // Second 2: 1 (A) + 3 (B) = 4 RPS; 1 error; weighted latency = (30*1 + 50*3)/4 = 45.
        assert!((window.rps[2] - 4.0).abs() < f64::EPSILON);
        assert!((window.fps[2] - 1.0).abs() < f64::EPSILON);
        assert_eq!(window.users[2], 4);
        assert!((window.avg_latency_ms[2] - 45.0).abs() < 0.001);
    }

    #[test]
    fn test_export_series_window_trailing_window_and_empty_seconds() {
        let mut graph = GraphData::new();
        // Activity only at seconds 0 and 5; window of 3 ending at 5 → seconds 3,4,5.
        graph.record_requests_per_second("GET /", 0);
        graph.record_requests_per_second("GET /", 5);
        graph.record_requests_per_second("GET /", 5);
        graph.record_average_response_time_per_second("GET /".to_string(), 0, 100);
        graph.record_average_response_time_per_second("GET /".to_string(), 5, 20);
        graph.record_average_response_time_per_second("GET /".to_string(), 5, 40);
        graph.record_users_per_second(2, 0);
        graph.record_users_per_second(8, 5);

        let window = graph.export_series_window(3);
        assert_eq!(window.start_second, 3);
        assert_eq!(window.rps.len(), 3);
        // Seconds 3 and 4 empty for RPS → 0.
        assert!((window.rps[0] - 0.0).abs() < f64::EPSILON);
        assert!((window.rps[1] - 0.0).abs() < f64::EPSILON);
        assert!((window.rps[2] - 2.0).abs() < f64::EPSILON);
        assert!((window.avg_latency_ms[0] - 0.0).abs() < f64::EPSILON);
        assert!((window.avg_latency_ms[1] - 0.0).abs() < f64::EPSILON);
        // Average of 20 and 40 = 30.
        assert!((window.avg_latency_ms[2] - 30.0).abs() < 0.001);
        // Users: 2 at second 0, then set_and_maintain fills 1..4 with 2 when
        // recording 8 at second 5. Window seconds 3,4,5 → [2, 2, 8].
        assert_eq!(window.users, vec![2, 2, 8]);
    }

    #[test]
    fn test_export_series_users_hold_when_rps_leads() {
        // Regression: Active users chart dropped to 0 near the right edge because
        // end_second came from RPS while users series lagged (RecordUsers is
        // main-loop only; drain_pending can advance RPS further).
        let mut graph = GraphData::new();
        for s in 0..=50 {
            graph.record_users_per_second(10, s);
        }
        // RPS continues through second 56 with no further RecordUsers.
        for s in 0..=56 {
            graph.record_requests_per_second("GET /", s);
        }

        let window = graph.export_series_window(300);
        assert_eq!(window.start_second, 0);
        assert_eq!(window.rps.len(), 57);
        assert_eq!(window.users.len(), 57);
        // Trailing seconds past last RecordUsers must hold 10, not drop to 0.
        for (i, &u) in window.users.iter().enumerate() {
            assert_eq!(u, 10, "users[{i}] should hold last known count 10, got {u}");
        }
        // RPS still present at the end (not all zeros).
        assert!(window.rps[56] > 0.0);
    }

    #[test]
    fn test_export_series_users_monotonic_ramp_without_forward_stamp() {
        // Regression: stamping active_users forward to the RPS/duration second
        // (set_and_maintain_last gap-fill with a stale count, then partial
        // overwrites from real RecordUsers) produced V-shaped glitches mid-ramp.
        // Only sequential RecordUsers samples should define the series; export
        // holds the last known level past the end.
        let mut graph = GraphData::new();
        // Steady 10 users.
        for s in 0..=60 {
            graph.record_users_per_second(10, s);
        }
        // Clean increase 10 → 20 over seconds 61..70.
        for (s, users) in (61..=70).zip(11..=20) {
            graph.record_users_per_second(users, s);
        }
        // RPS leads past the last users sample (as drain_pending does).
        for s in 0..=78 {
            graph.record_requests_per_second("GET /", s);
        }

        let window = graph.export_series_window(300);
        assert_eq!(window.users.len(), 79);
        // Steady region.
        for s in 0..=60 {
            assert_eq!(window.users[s], 10, "steady users[{s}]");
        }
        // Monotonic ramp — no dips.
        for s in 61..=70 {
            let expected = (s - 60 + 10) as u64; // 11..20
            assert_eq!(window.users[s], expected, "ramp users[{s}]");
            if s > 61 {
                assert!(
                    window.users[s] >= window.users[s - 1],
                    "ramp must be non-decreasing at {s}: {} < {}",
                    window.users[s],
                    window.users[s - 1]
                );
            }
        }
        // Trailing hold at 20 (not 0, not a dip).
        for s in 71..=78 {
            assert_eq!(window.users[s], 20, "hold users[{s}]");
        }
    }

    #[test]
    fn test_out_of_order_users_write_creates_v_glitch() {
        // Documents why user stamps must be monotonic in time: gap-fill with a
        // jump forward, then overwriting intermediate seconds, leaves stale dips.
        let mut graph = GraphData::new();
        graph.record_users_per_second(10, 60);
        // Bad stamp: jump to second 70 with live count 20 (fills 61..69 with 10).
        graph.record_users_per_second(20, 70);
        // Real hatch samples only overwrite some intermediate seconds.
        graph.record_users_per_second(15, 65);
        graph.record_users_per_second(18, 68);

        let window = graph.export_series_window(300);
        // 60=10, 61-64=10 (stale fill), 65=15, 66-67=10 (stale!), 68=18, 69=10, 70=20
        assert_eq!(window.users[60], 10);
        assert_eq!(window.users[65], 15);
        assert_eq!(
            window.users[66], 10,
            "stale gap-fill creates a dip after 15"
        );
        assert_eq!(window.users[68], 18);
        assert_eq!(
            window.users[69], 10,
            "stale gap-fill creates a dip before 20"
        );
        assert_eq!(window.users[70], 20);
        // Confirm the V: 15 → 10 → 18 is non-monotonic mid-ramp.
        assert!(window.users[66] < window.users[65] && window.users[66] < window.users[68]);
    }

    #[test]
    fn test_record_users_monotonic_rejects_backwards_writes() {
        let mut graph = GraphData::new();
        graph.record_users_per_second(10, 5);
        graph.record_users_per_second(20, 10);
        // Would create a V-glitch if applied — must be ignored.
        graph.record_users_monotonic(15, 7);
        let window = graph.export_series_window(300);
        assert_eq!(window.users[7], 10, "backwards write must not rewrite past");
        assert_eq!(window.users[10], 20);
        // Forward monotonic stamp is allowed and pins the edge.
        graph.record_users_monotonic(25, 12);
        let window = graph.export_series_window(300);
        assert_eq!(window.users[10], 20);
        assert_eq!(window.users[11], 20);
        assert_eq!(window.users[12], 25);
    }

    #[test]
    fn test_reset_rate_series_preserves_users_history() {
        let mut graph = GraphData::new();
        for (s, users) in (0..=5).zip(1..=6) {
            graph.record_users_per_second(users, s);
        }
        graph.record_requests_per_second("GET /", 3);
        graph.record_errors_per_second("GET /", 3);
        graph.record_average_response_time_per_second("GET /".into(), 3, 42);
        graph.record_transactions_per_second(3);
        graph.record_scenarios_per_second(3);

        graph.reset_rate_series();

        let window = graph.export_series_window(300);
        // Users ramp intact.
        assert_eq!(window.users, vec![1, 2, 3, 4, 5, 6]);
        // Rate series wiped.
        assert!(window.rps.iter().all(|&v| v == 0.0));
        assert!(window.fps.iter().all(|&v| v == 0.0));
        assert!(window.avg_latency_ms.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn test_reset_rate_series_preserves_pruned_origin() {
        // After a long dashboard-only soak, prune advances origin. A subsequent
        // increase→maintain reset must not recreate rate series at origin=0 or the
        // next absolute-second sample allocates a multi-hour-sized vector.
        let mut graph = GraphData::new();
        for s in 0..=900 {
            graph.record_users_per_second(10, s);
            graph.record_requests_per_second("GET /", s);
        }
        graph.prune_to_window(600);
        assert!(graph.users_per_second.origin > 0);
        let origin_before = graph.users_per_second.origin;

        graph.reset_rate_series();
        assert_eq!(graph.requests_per_second.origin, origin_before);
        assert_eq!(graph.transactions_per_second.origin, origin_before);

        // Sample at a late absolute second: storage length stays near the window,
        // not ~absolute second from 0.
        let late = origin_before + 50;
        graph.record_requests_per_second("GET /new", late);
        let stored = graph
            .requests_per_second
            .series
            .get("GET /new")
            .expect("series")
            .data
            .len();
        assert!(
            stored <= 51,
            "expected compact storage after origin-preserving reset, got len={stored}",
            stored = stored
        );
    }

    #[test]
    fn test_prune_aligns_idle_key_origin_on_reappear() {
        // Idle request names only receive early samples, so length-based prune
        // leaves their origin at 0. After a long soak, re-activating that key
        // must not expand from absolute second 0.
        let mut graph = GraphData::new();
        graph.record_requests_per_second("GET /idle", 0);
        graph.record_requests_per_second("GET /idle", 1);
        graph.record_errors_per_second("GET /idle", 0);
        graph.record_average_response_time_per_second("GET /idle".into(), 0, 10);

        for s in 0..=900 {
            graph.record_users_per_second(10, s);
            graph.record_requests_per_second("GET /active", s);
            graph.record_transactions_per_second(s);
            graph.record_scenarios_per_second(s);
        }
        graph.prune_to_window(600);

        let origin = graph.users_per_second.origin;
        assert!(origin > 0, "users origin should advance after prune");
        assert_eq!(
            graph
                .requests_per_second
                .series
                .get("GET /idle")
                .expect("idle rps")
                .origin,
            origin
        );
        assert_eq!(
            graph
                .errors_per_second
                .series
                .get("GET /idle")
                .expect("idle errors")
                .origin,
            origin
        );
        assert_eq!(
            graph
                .average_response_time_per_second
                .get("GET /idle")
                .expect("idle latency")
                .origin,
            origin
        );

        let late = origin + 50;
        graph.record_requests_per_second("GET /idle", late);
        graph.record_errors_per_second("GET /idle", late);
        graph.record_average_response_time_per_second("GET /idle".into(), late, 12);

        for (label, len) in [
            (
                "rps",
                graph
                    .requests_per_second
                    .series
                    .get("GET /idle")
                    .unwrap()
                    .data
                    .len(),
            ),
            (
                "errors",
                graph
                    .errors_per_second
                    .series
                    .get("GET /idle")
                    .unwrap()
                    .data
                    .len(),
            ),
            (
                "latency",
                graph
                    .average_response_time_per_second
                    .get("GET /idle")
                    .unwrap()
                    .data
                    .len(),
            ),
        ] {
            assert!(
                len <= 51,
                "idle {label} series must stay compact after reappear, got len={len}",
                label = label,
                len = len
            );
        }
    }

    #[test]
    fn test_export_series_window_until_extends_to_hint() {
        let mut graph = GraphData::new();
        graph.record_users_per_second(7, 2);
        // No RPS; without hint end would be second 2.
        let window = graph.export_series_window_until(300, Some(8));
        assert_eq!(window.users.len(), 9); // seconds 0..8
        assert_eq!(window.users[2], 7);
        // Hold last known level through the hint ("now").
        for s in 3..=8 {
            assert_eq!(window.users[s], 7, "hold users[{s}]");
        }
    }

    /// GraphData with latency histograms, synthetic ones too when `synthetic`.
    fn latency_graph(synthetic: bool) -> GraphData {
        GraphData::for_metrics_processor(true, synthetic)
    }

    fn histograms(graph: &mut GraphData) -> &LatencyHistograms {
        graph.move_pending_latency();
        graph.latency_histograms().expect("latency histograms")
    }

    #[test]
    fn latency_batch_merge_equals_direct_recording() {
        let times: [(usize, u64); 6] = [(3, 12), (3, 140), (3, 1400), (4, 7), (4, 90), (4, 2600)];
        // Two batches, so the second finds the request's ring through the
        // index cached on its requests per second entry.
        let mut batches: [HashMap<usize, GooseRequestMetricTimingData>; 2] = Default::default();
        for (i, &(second, time)) in times.iter().enumerate() {
            batches[i % 2].entry(second).or_default().record_time(time);
        }
        let mut merged = latency_graph(false);
        for batch in &batches {
            for (second, data) in batch {
                merged.record_requests_per_second_and_latency_batch(
                    "GET /a",
                    *second,
                    data.counter as u32,
                    Some(data),
                    None,
                );
            }
        }
        let mut direct = latency_graph(false);
        for &(second, time) in &times {
            direct.record_request_per_second_and_latency("GET /a", second, Some(time));
        }
        assert_eq!(
            merged.requests_per_second.get("GET /a").unwrap().data,
            direct.requests_per_second.get("GET /a").unwrap().data
        );
        for second in [3, 4] {
            let merged = histograms(&mut merged);
            let direct = histograms(&mut direct);
            assert_eq!(merged.all_second(second), direct.all_second(second));
            assert_eq!(
                merged.rings[merged.by_request["GET /a"]].get(second),
                direct.rings[direct.by_request["GET /a"]].get(second)
            );
            let mut expected = SecondHistogram::default();
            for &(_, time) in times.iter().filter(|(s, _)| *s == second) {
                expected.record(time);
            }
            assert_eq!(merged.all_second(second), Some(expected.clone()));
        }
    }

    /// Batches merged through a request's pending counts, in and out of
    /// order, with a jump of several seconds, an earlier second and a time
    /// past 60 seconds, give the rings and the heatmap the same response
    /// times as recording each request directly, whenever they are read.
    #[test]
    fn pending_latency_equals_direct_recording() {
        let batches: [(usize, &[u64]); 8] = [
            (5, &[3, 40]),
            (6, &[7]),
            (5, &[61_500, 2]),
            (6, &[0, 900]),
            (9, &[12]),
            (4, &[33]),
            (10, &[5, 5]),
            (9, &[250]),
        ];
        let mut merged = latency_graph(false);
        let mut direct = latency_graph(false);
        for (i, &(second, times)) in batches.iter().enumerate() {
            let mut data = GooseRequestMetricTimingData::new(None);
            for &time in times {
                data.record_time(time);
                direct.record_request_per_second_and_latency("GET /a", second, Some(time));
            }
            merged.record_requests_per_second_and_latency_batch(
                "GET /a",
                second,
                data.counter as u32,
                Some(&data),
                None,
            );
            // Read part way through as well as at the end.
            if i == 3 {
                let merged = histograms(&mut merged);
                let direct = histograms(&mut direct);
                for second in 4..=6 {
                    assert_eq!(merged.all_second(second), direct.all_second(second));
                }
            }
        }
        let window = |graph: &mut GraphData| {
            graph
                .export_series_window_until(300, Some(12))
                .heatmap
                .unwrap()
                .columns
        };
        assert_eq!(window(&mut merged), window(&mut direct));
        for second in 3..=11 {
            let merged = histograms(&mut merged);
            let direct = histograms(&mut direct);
            assert_eq!(
                merged.all_second(second),
                direct.all_second(second),
                "{second}"
            );
            assert_eq!(
                merged.rings[merged.by_request["GET /a"]].get(second),
                direct.rings[direct.by_request["GET /a"]].get(second),
                "{second}"
            );
        }
    }

    #[test]
    fn second_histogram_matches_timing_data() {
        // Every rounded time maps to its own counter and back.
        let mut rounded: Vec<usize> = (0..=100).collect();
        rounded.extend((110..=500).step_by(10));
        rounded.extend((600..=1000).step_by(100));
        rounded.extend((2000..=60_000).step_by(1000));
        assert_eq!(rounded.len(), FIXED_BUCKETS);
        for (index, &value) in rounded.iter().enumerate() {
            assert_eq!(dense_index(value), Some(index));
            assert_eq!(dense_value(index), value);
        }
        assert_eq!(dense_index(61_000), None);

        // Same buckets, totals and percentiles as the whole run histogram,
        // including times past 60 seconds (a longer --timeout).
        let mut times: Vec<u64> = Vec::new();
        let mut seed: u64 = 7;
        for _ in 0..5000 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            times.push((seed >> 33) % 90_000 / (1 + (seed >> 60)));
        }
        let mut reference = crate::metrics::GooseRequestMetricTimingData::default();
        let mut histogram = SecondHistogram::default();
        for &time in &times {
            reference.record_time(time);
            histogram.record(time);
        }
        assert!(!histogram.overflow.is_empty());
        let buckets: Vec<(usize, usize)> = histogram.buckets().collect();
        let expected: Vec<(usize, usize)> = reference.times.iter().map(|(&t, &c)| (t, c)).collect();
        assert_eq!(buckets, expected);
        assert_eq!(histogram.counter, reference.counter);
        assert_eq!(histogram.total_time, reference.total_time);
        assert_eq!(histogram.minimum_time, reference.minimum_time);
        assert_eq!(histogram.maximum_time, reference.maximum_time);
        for percent in [0.0, 0.5, 0.95, 0.99, 1.0] {
            assert_eq!(
                histogram.percentile(percent),
                crate::metrics::calculate_response_time_percentile(
                    &reference.times,
                    reference.counter,
                    reference.minimum_time,
                    reference.maximum_time,
                    percent,
                )
            );
        }

        // Batches merged into an empty histogram give the same histogram.
        let (first, second) = times.split_at(times.len() / 3);
        let mut via_batch = SecondHistogram::default();
        for part in [first, second] {
            let mut batch = SecondHistogram::default();
            for &time in part {
                batch.record(time);
            }
            via_batch.merge(&batch);
        }
        assert_eq!(via_batch, histogram);

        // A bucket past the most one entry holds continues in the next.
        let mut full = SecondHistogram::default();
        full.add_indexed(0, 5, COUNT_MAX as usize + 10);
        full.add_indexed(0, 5, 3);
        full.add_indexed(0, 4, 1);
        assert_eq!(
            full.buckets().collect::<Vec<_>>(),
            vec![(4, 1), (5, COUNT_MAX as usize), (5, 13)]
        );

        // Merging two halves equals recording everything.
        let (first, second) = times.split_at(times.len() / 2);
        let mut a = SecondHistogram::default();
        first.iter().for_each(|&time| a.record(time));
        let mut b = SecondHistogram::default();
        second.iter().for_each(|&time| b.record(time));
        a.merge(&b);
        assert_eq!(a, histogram);
    }

    #[test]
    fn open_second_equals_packed_second() {
        let mut times: Vec<u64> = Vec::new();
        let mut seed: u64 = 11;
        for _ in 0..3000 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            times.push((seed >> 33) % 90_000 / (1 + (seed >> 60)));
        }
        let mut packed = SecondHistogram::default();
        times.iter().for_each(|&time| packed.record(time));

        // Recorded and merged into an open second, read open and packed.
        let mut open = SecondHistogram::open();
        let (first, second) = times.split_at(times.len() / 2);
        first.iter().for_each(|&time| open.record(time));
        let mut batch = SecondHistogram::default();
        second.iter().for_each(|&time| batch.record(time));
        open.merge(&batch);
        assert!(open.entries.is_empty());
        assert_eq!(open, packed);
        assert_eq!(
            open.buckets().collect::<Vec<_>>(),
            packed.buckets().collect::<Vec<_>>()
        );
        for percent in [0.0, 0.5, 0.95, 0.99, 1.0] {
            assert_eq!(open.percentile(percent), packed.percentile(percent));
        }
        open.pack();
        assert!(open.open.is_none());
        assert_eq!(open.entries, packed.entries);
        assert_eq!(open.overflow, packed.overflow);

        // A packed second keeps taking late responses.
        open.record(7);
        packed.record(7);
        assert_eq!(open.entries, packed.entries);

        // An open bucket past the most one entry holds packs into two.
        let mut full = SecondHistogram::open();
        full.add_bucket(5, COUNT_MAX as usize + 10);
        full.add_bucket(4, 1);
        full.pack();
        assert_eq!(
            full.buckets().collect::<Vec<_>>(),
            vec![(4, 1), (5, COUNT_MAX as usize), (5, 10)]
        );
    }

    #[test]
    fn latency_ring_packs_seconds_two_back() {
        let mut ring = LatencyRing::new();
        ring.record(10, 20, 1);
        ring.record(11, 20, 2);
        let is_open = |ring: &LatencyRing, second| ring.get(second).unwrap().open.is_some();
        assert!(is_open(&ring, 10) && is_open(&ring, 11));
        ring.record(12, 20, 3);
        assert!(!is_open(&ring, 10));
        assert!(is_open(&ring, 11) && is_open(&ring, 12));
        // A late response to a packed second still counts, and marks it
        // changed.
        ring.record(10, 30, 4);
        assert_eq!(ring.get(10).unwrap().counter, 2);
        assert_eq!(ring.slot(10).unwrap().rev, 4);
        assert_eq!(ring.slot(11).unwrap().rev, 2);
        // After a gap every older open second is packed.
        ring.record(40, 20, 5);
        assert!(!is_open(&ring, 11) && !is_open(&ring, 12));
        assert_eq!(ring.open, vec![40]);
        ring.clear();
        assert!(ring.open.is_empty());
    }

    #[test]
    fn latency_ring_drops_samples_older_than_its_slot() {
        let mut ring = LatencyRing::new();
        ring.record(400, 25, 1);
        // Second 100 maps to the same slot as 400 (300 slots).
        ring.record(100, 10, 2);
        ring.merge_timing(
            100,
            &{
                let mut data = GooseRequestMetricTimingData::default();
                data.record_time(10);
                data
            },
            3,
        );
        assert!(ring.get(100).is_none());
        let held = ring.get(400).expect("slot keeps second 400");
        assert_eq!(held.counter, 1);
        assert_eq!(held.maximum_time, 25);
        // A sample too old to keep does not mark the slot changed.
        assert_eq!(ring.slot(400).unwrap().rev, 1);
        // A newer second replaces the older one.
        ring.record(700, 5, 4);
        assert!(ring.get(400).is_none());
        assert_eq!(ring.get(700).map(|d| d.counter), Some(1));
    }

    /// The `(index, count)` pairs of `second`'s measured buckets.
    fn measured_pairs(heatmap: &Heatmap, second: u64) -> Vec<(u64, u64)> {
        heatmap
            .column(second)
            .map(|column| HeatmapColumn::pairs(&column.measured))
            .unwrap_or_default()
    }

    #[test]
    fn heatmap_bucket_table_follows_round_metric_time() {
        let table = bucket_table(FIXED_BUCKETS);
        assert_eq!(table.len(), FIXED_BUCKETS + 1);
        // Every measured time from 0 to past 61 seconds falls in exactly the
        // bucket whose range holds it, and the ranges meet end to end.
        for (index, entry) in table.iter().enumerate() {
            let [value, lowest, highest] = *entry;
            assert!(lowest <= value && value <= highest, "{:?}", entry);
            for time in [lowest, highest] {
                assert_eq!(round_metric_time(time) as u64, value, "{entry:?}");
                assert_eq!(heat_index(round_metric_time(time)), index, "{entry:?}");
            }
            if index > 0 {
                assert_eq!(table[index - 1][2] + 1, lowest, "{entry:?}");
            }
        }
        for (index, entry) in [
            (0, [0, 0, 0]),
            (1, [1, 1, 1]),
            (99, [99, 99, 99]),
            (100, [100, 100, 104]),
            (101, [110, 105, 114]),
            (140, [500, 495, 549]),
            (141, [600, 550, 649]),
            (145, [1000, 950, 1499]),
            (146, [2000, 1500, 2499]),
            (204, [60_000, 59_500, 60_499]),
            (205, [61_000, 60_500, 61_499]),
        ] {
            assert_eq!(table[index], entry, "index {index}");
        }
        assert_eq!(bucket_table(0).len(), FIXED_BUCKETS);
    }

    /// A column's cells are kept from one heatmap to the next, and rebuilt
    /// when its second changes.
    #[test]
    fn heatmap_columns_rebuild_only_when_changed() {
        let mut graph = latency_graph(false);
        graph.record_requests_per_second("GET /", 1);
        graph.record_latency("GET /", 1, 5);
        let first = graph
            .export_series_window_until(300, Some(4))
            .heatmap
            .unwrap();
        assert_eq!(measured_pairs(&first, 1), [(5, 1)]);
        let again = graph
            .export_series_window_until(300, Some(4))
            .heatmap
            .unwrap();
        assert_eq!(again.columns, first.columns);
        graph.record_latency("GET /", 1, 6);
        let changed = graph
            .export_series_window_until(300, Some(4))
            .heatmap
            .unwrap();
        assert_eq!(measured_pairs(&changed, 1), [(5, 1), (6, 1)]);
    }

    #[test]
    fn heatmap_sends_only_changed_seconds() {
        let mut graph = latency_graph(false);
        for (second, time) in [(1, 5), (2, 7), (2, 7), (2, 120)] {
            graph.record_requests_per_second("GET /", second);
            graph.record_latency("GET /", second, time);
        }
        let first = graph
            .export_series_window_until(300, Some(4))
            .heatmap
            .unwrap();
        assert_eq!(first.since, 0);
        assert_eq!(
            (first.start_second, first.end_second, first.hold_from),
            (0, 4, 3)
        );
        assert_eq!(first.columns.len(), 2);
        assert_eq!(measured_pairs(&first, 1), [(5, 1)]);
        // 120 ms is in the 120 ms bucket, index 102.
        assert_eq!(measured_pairs(&first, 2), [(7, 2), (102, 1)]);
        assert!(first
            .columns
            .iter()
            .all(|column| column.synthetic.is_empty()));
        assert_eq!(first.buckets.len(), FIXED_BUCKETS);

        // Nothing changed: no columns.
        let cursor = HeatmapCursor {
            generation: first.generation,
            rev: first.rev,
        };
        let same = graph
            .export_series_window_with_heatmap(300, Some(4), false, Some(cursor))
            .heatmap
            .unwrap();
        assert_eq!((same.since, same.rev), (first.rev, first.rev));
        assert!(same.columns.is_empty());

        // A late response to second 1 and a new second 3: only those, whole.
        graph.record_latency("GET /", 1, 6);
        graph.record_latency("GET /", 3, 75_000);
        let delta = graph
            .export_series_window_with_heatmap(300, Some(4), false, Some(cursor))
            .heatmap
            .unwrap();
        assert_eq!(delta.since, first.rev);
        assert_eq!(
            delta
                .columns
                .iter()
                .map(|column| column.second)
                .collect::<Vec<_>>(),
            [1, 3]
        );
        assert_eq!(measured_pairs(&delta, 1), [(5, 1), (6, 1)]);
        // Past 60 seconds one row per second: 75 s is index 219, and the
        // table grows to cover it.
        assert_eq!(measured_pairs(&delta, 3), [(219, 1)]);
        assert_eq!(delta.buckets.len(), 220);
        assert_eq!(delta.buckets[219], [75_000, 74_500, 75_499]);

        // A cursor of another generation, or from the future, gets the whole
        // window.
        for stale in [
            HeatmapCursor {
                generation: cursor.generation + 1,
                rev: cursor.rev,
            },
            HeatmapCursor {
                generation: cursor.generation,
                rev: delta.rev + 1,
            },
        ] {
            let whole = graph
                .export_series_window_with_heatmap(300, Some(4), false, Some(stale))
                .heatmap
                .unwrap();
            assert_eq!(whole.since, 0);
            assert_eq!(whole.columns.len(), 3);
        }

        // A metrics reset starts a new generation with no columns.
        graph.reset_rate_series();
        let reset = graph
            .export_series_window_with_heatmap(300, Some(4), false, Some(cursor))
            .heatmap
            .unwrap();
        assert_ne!(reset.generation, cursor.generation);
        assert_eq!(reset.since, 0);
        assert!(reset.columns.is_empty());
    }

    #[test]
    fn heatmap_slow_second_and_split_buckets() {
        let mut graph = latency_graph(false);
        for _ in 0..10 {
            graph.record_requests_per_second("GET /slow", 5);
            graph.record_latency("GET /slow", 5, 1400);
        }
        let heatmap = graph
            .export_series_window_until(300, Some(7))
            .heatmap
            .unwrap();
        assert_eq!(measured_pairs(&heatmap, 5), [(145, 10)]);

        // A bucket past the most one packed entry holds is still one cell.
        let mut full = SecondHistogram::default();
        full.add_indexed(0, 5, COUNT_MAX as usize + 10);
        full.add_indexed(0, 4, 1);
        assert_eq!(full.heat_cells(), [4, 1, 5, COUNT_MAX as u64 + 10]);
    }

    #[test]
    fn heatmap_window_gaps_and_hold() {
        let mut graph = latency_graph(false);
        for second in [2, 4, 9, 10] {
            graph.record_requests_per_second("GET /", second);
            graph.record_latency("GET /", second, 20);
        }
        let heatmap = graph
            .export_series_window_until(300, Some(10))
            .heatmap
            .unwrap();
        // Seconds without requests have no column.
        assert_eq!(
            heatmap.columns.iter().map(|c| c.second).collect::<Vec<_>>(),
            [2, 4, 9, 10]
        );
        // The two newest seconds, from hint - 1, are held back.
        assert_eq!(heatmap.hold_from, 9);
        // A request filed one second past the hint extends the window; the
        // hold still starts at hint - 1.
        graph.record_requests_per_second("GET /", 11);
        graph.record_latency("GET /", 11, 20);
        let heatmap = graph
            .export_series_window_until(300, Some(10))
            .heatmap
            .unwrap();
        assert_eq!((heatmap.end_second, heatmap.hold_from), (11, 9));
        // Nothing recorded yet: an empty heatmap, not none.
        let empty = latency_graph(false)
            .export_series_window_until(300, None)
            .heatmap
            .unwrap();
        assert!(empty.columns.is_empty());
    }

    #[test]
    fn latency_keeps_first_hundred_names() {
        let mut graph = latency_graph(true);
        for i in 0..=MAX_REQUEST_SERIES {
            graph.record_latency(&format!("GET /{i}"), 1, 10);
        }
        let latency = histograms(&mut graph);
        assert_eq!(latency.by_request.len(), MAX_REQUEST_SERIES);
        assert!(!latency.by_request.contains_key("GET /100"));
        // The 101st name still counts toward all requests.
        assert_eq!(
            latency.all_second(1).map(|d| d.counter),
            Some(MAX_REQUEST_SERIES + 1)
        );
        // No synthetic ring for a name without a measured one.
        graph.record_synthetic_latency("GET /100", 1, 500);
        graph.record_synthetic_latency("GET /0", 1, 500);
        let (all, by_request) = histograms(&mut graph).synthetic.as_ref().unwrap();
        assert_eq!(all.get(1).map(|d| d.counter), Some(2));
        assert!(by_request.contains_key("GET /0"));
        assert!(!by_request.contains_key("GET /100"));

        let keys = graph.latency_series_keys();
        assert_eq!(keys.len(), MAX_REQUEST_SERIES);
        assert!(keys.contains("GET /99"));
        assert!(!keys.contains("GET /100"));

        // A metrics reset empties both maps and every ring, so names are
        // admitted again.
        graph.reset_rate_series();
        let latency = histograms(&mut graph);
        assert!(latency.by_request.is_empty());
        assert!(latency.all_second(1).is_none());
        let (all, by_request) = latency.synthetic.as_ref().unwrap();
        assert!(by_request.is_empty());
        assert!(all.get(1).is_none());
        graph.record_latency("GET /100", 2, 10);
        assert!(graph.latency_series_keys().contains("GET /100"));
    }

    /// The heatmap's JSON in the worst case, every bucket in every second of
    /// the window: what changes in a second, and a 60 second synthetic
    /// backfill, stay within 200 KB; a whole window, sent once to a client
    /// with no heatmap, does not, which is why only changes are sent.
    #[test]
    fn heatmap_worst_case_sizes() {
        let mut graph = latency_graph(true);
        let every_bucket = |count: usize| {
            let mut data = GooseRequestMetricTimingData::default();
            for index in 0..FIXED_BUCKETS {
                data.times.insert(dense_value(index), count);
            }
            data.counter = FIXED_BUCKETS * count;
            data.maximum_time = 60_000;
            data
        };
        let data = every_bucket(12_345);
        for second in 0..300 {
            graph.record_requests_per_second_and_latency_batch(
                "GET /",
                second,
                data.counter as u32,
                Some(&data),
                None,
            );
        }
        let size = |heatmap: &Heatmap| serde_json::to_vec(heatmap).unwrap().len();
        let whole = graph
            .export_series_window_with_heatmap(300, Some(299), true, None)
            .heatmap
            .unwrap();
        assert_eq!(whole.columns.len(), 300);
        let cursor = HeatmapCursor {
            generation: whole.generation,
            rev: whole.rev,
        };
        // The next second: a new column, and late responses to the two
        // before it.
        for second in [298, 299, 300] {
            graph.record_requests_per_second_and_latency_batch(
                "GET /",
                second,
                data.counter as u32,
                Some(&data),
                None,
            );
        }
        let next = graph
            .export_series_window_with_heatmap(300, Some(300), true, Some(cursor))
            .heatmap
            .unwrap();
        assert_eq!(next.columns.len(), 3);
        // A 60 second stall backfilled with synthetic times in every bucket.
        let cursor = HeatmapCursor {
            generation: next.generation,
            rev: next.rev,
        };
        for second in 241..=300 {
            for index in 0..FIXED_BUCKETS {
                graph.record_synthetic_latency("GET /", second, dense_value(index) as u64);
            }
        }
        let backfill = graph
            .export_series_window_with_heatmap(300, Some(300), true, Some(cursor))
            .heatmap
            .unwrap();
        assert_eq!(backfill.columns.len(), 60);
        eprintln!(
            "worst case heatmap JSON: whole window {} bytes, a second's changes {} bytes, a 60 s backfill {} bytes",
            size(&whole),
            size(&next),
            size(&backfill)
        );
        assert!(size(&next) <= 200 * 1024);
        assert!(size(&backfill) <= 200 * 1024);
        assert!(size(&whole) > 200 * 1024);
    }

    #[test]
    fn heatmap_off_exports_none() {
        let mut graph = GraphData::new();
        graph.record_requests_per_second("GET /", 1);
        graph.record_latency("GET /", 1, 10);
        let window = graph.export_series_window_with_heatmap(300, Some(3), true, None);
        assert_eq!(window.rps.len(), 4);
        assert_eq!(window.heatmap, None);
        assert!(graph.latency_series_keys().is_empty());
        assert!(graph
            .export_request_series("GET".into(), "/".into(), 300, Some(3), true, None)
            .is_none());
    }

    #[test]
    fn heatmap_request_series_export() {
        let mut graph = latency_graph(true);
        for second in [1, 2] {
            for _ in 0..3 {
                graph.record_requests_per_second("GET /a", second);
                graph.record_latency("GET /a", second, 30);
            }
            graph.record_requests_per_second("GET /b", second);
            graph.record_latency("GET /b", second, 900);
        }
        graph.record_synthetic_latency("GET /a", 1, 4000);
        let series = graph
            .export_request_series("GET".into(), "/a".into(), 300, Some(5), true, None)
            .expect("series for /a");
        assert_eq!(series.method, "GET");
        assert_eq!(series.name, "/a");
        let heatmap = &series.heatmap;
        assert_eq!((heatmap.start_second, heatmap.end_second), (0, 5));
        assert_eq!(measured_pairs(heatmap, 1), [(30, 3)]);
        assert_eq!(measured_pairs(heatmap, 2), [(30, 3)]);
        // 4 s is index 148.
        assert_eq!(
            HeatmapColumn::pairs(&heatmap.column(1).unwrap().synthetic),
            [(148, 1)]
        );
        // Not co_active: no synthetic times.
        let series = graph
            .export_request_series("GET".into(), "/a".into(), 300, Some(5), false, None)
            .unwrap();
        assert!(series
            .heatmap
            .columns
            .iter()
            .all(|c| c.synthetic.is_empty()));
        // All requests: both names.
        let all = graph
            .export_series_window_until(300, Some(5))
            .heatmap
            .unwrap();
        assert_eq!(all.counts(1), (4, 0));
        assert!(graph
            .export_request_series("GET".into(), "/missing".into(), 300, Some(5), true, None)
            .is_none());
    }

    #[test]
    fn test_moving_average() {
        let mut moving_average = MovingAverage::new();
        assert_eq!(
            moving_average,
            MovingAverage {
                count: 0,
                average: 0.
            }
        );

        moving_average.add_item(&1.23);
        assert_eq!(
            moving_average,
            MovingAverage {
                count: 1,
                average: 1.23
            }
        );

        moving_average.add_item(&2.34);
        assert_eq!(
            moving_average,
            MovingAverage {
                count: 2,
                average: 1.785
            }
        );

        moving_average.add_item(&89.23);
        assert_eq!(
            moving_average,
            MovingAverage {
                count: 3,
                average: 30.933332
            }
        );

        moving_average.add_item(&12.34);
        assert_eq!(
            moving_average,
            MovingAverage {
                count: 4,
                average: 26.285
            }
        );
    }

    #[test]
    fn test_moving_average_cmp() {
        assert!(
            MovingAverage {
                count: 0,
                average: 0.
            } < MovingAverage {
                count: 0,
                average: 0.1,
            }
        );

        assert!(
            MovingAverage {
                count: 0,
                average: 1.1,
            } > MovingAverage {
                count: 0,
                average: 0.1,
            }
        );

        assert_eq!(
            MovingAverage {
                count: 1,
                average: 1.1,
            },
            MovingAverage {
                count: 2,
                average: 1.1,
            }
        );

        assert!(
            MovingAverage {
                count: 0,
                average: 1.1,
            } != MovingAverage {
                count: 0,
                average: 1.0,
            }
        );
    }

    #[test]
    fn test_add_timestamp_to_html_graph_data() {
        let data = vec![Some(123), Some(234), Some(345), Some(456), Some(567)];
        let empty = HashMap::new();
        let graph: Graph<usize, usize> = Graph::new("html_id", "Label", true, &empty);

        assert_eq!(
            graph.add_timestamp_to_html_graph_data(
                &data,
                Utc.with_ymd_and_hms(2021, 12, 14, 15, 12, 23).unwrap()
            ),
            vec![
                (
                    Local
                        .timestamp_opt(
                            Utc.with_ymd_and_hms(2021, 12, 14, 15, 12, 23)
                                .unwrap()
                                .timestamp(),
                            0
                        )
                        .unwrap()
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    123
                ),
                (
                    Local
                        .timestamp_opt(
                            Utc.with_ymd_and_hms(2021, 12, 14, 15, 12, 24)
                                .unwrap()
                                .timestamp(),
                            0
                        )
                        .unwrap()
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    234
                ),
                (
                    Local
                        .timestamp_opt(
                            Utc.with_ymd_and_hms(2021, 12, 14, 15, 12, 25)
                                .unwrap()
                                .timestamp(),
                            0
                        )
                        .unwrap()
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    345
                ),
                (
                    Local
                        .timestamp_opt(
                            Utc.with_ymd_and_hms(2021, 12, 14, 15, 12, 26)
                                .unwrap()
                                .timestamp(),
                            0
                        )
                        .unwrap()
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    456
                ),
                (
                    Local
                        .timestamp_opt(
                            Utc.with_ymd_and_hms(2021, 12, 14, 15, 12, 27)
                                .unwrap()
                                .timestamp(),
                            0
                        )
                        .unwrap()
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    567
                )
            ]
        );
    }

    fn expected_graph_html_prefix(
        html_id: &str,
        y_axis_label: &str,
        main_series_label: &str,
    ) -> String {
        format!(
            r#"<div class="graph">
                <div id="{html_id}" style="width: 1000px; height:500px; background: white;"></div>

                <script type="text/javascript">
                    var chartDom = document.getElementById('{html_id}');
                    var myChart = echarts.init(chartDom);

                    myChart.setOption({{
                        color: ['#2c664f', '#5470c6', '#91cc75', '#fac858', '#ee6666', '#73c0de', '#3ba272', '#fc8452', '#9a60b4', '#ea7ccc'],
                        tooltip: {{ trigger: 'axis' }},
                        toolbox: {{
                            feature: {{
                                dataZoom: {{ yAxisIndex: 'none' }},
                                restore: {{}},
                                saveAsImage: {{}}
                            }}
                        }},
                        dataZoom: [
                            {{
                                type: 'inside',
                                start: 0,
                                end: 100,
                                fillerColor: 'rgba(34, 80, 61, 0.25)',
                                selectedDataBackground: {{
                                    lineStyle: {{ color: '#2c664f' }},
                                    areaStyle: {{ color: '#378063' }}
                                }}
                            }},
                            {{
                                start: 0,
                                end: 100,
                                fillerColor: 'rgba(34, 80, 61, 0.25)',
                                selectedDataBackground: {{
                                    lineStyle: {{ color: '#2c664f' }},
                                    areaStyle: {{ color: '#378063' }}
                                }}
                            }},
                        ],
                        xAxis: {{ type: 'time' }},
                        yAxis: {{
                            name: '{y_axis_label}',
                            nameLocation: 'center',
                            nameRotate: 90,
                            nameGap: 45,
                            type: 'value'
                        }},
                        
                        series: [
                            {{
                                name: '{main_series_label}',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                lineStyle: {{ color: '#2c664f' }},
                                areaStyle: {{ color: '#378063' }},
                                markArea: {{
"#
        )
    }

    #[test]
    fn test_graph_markup() {
        let expected_prefix = expected_graph_html_prefix("graph-rps", "Requests #", "GET /");

        let data: TimeSeries<usize, usize> = TimeSeries {
            origin: 0,
            data: VecDeque::from(vec![123, 111, 99, 134]),
            phantom: PhantomData,
            total: 0,
        };

        let mut graph = HashMap::new();
        graph.insert("GET /".to_string(), data);

        let mut expected = expected_prefix.to_owned();
        expected += format!(
            r#"                                    data: [
                                        
                                    ]
                                }},
                                data: [["{data_series_prefix}:32",123],["{data_series_prefix}:33",111],["{data_series_prefix}:34",99],["{data_series_prefix}:35",134]],
                            }},
                            
                        ]
                    }});
                </script>
            </div>"#,
            data_series_prefix = Local
                .timestamp_opt(Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap().timestamp(), 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M")
        ).as_str();

        assert_eq!(
            Graph::new("graph-rps", "Requests #", true, &graph,).get_markup(
                &Vec::new(),
                Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap()
            ),
            expected
        );

        // It should make no difference if we disable granular graphs, since we only have one
        // request.
        assert_eq!(
            Graph::new("graph-rps", "Requests #", false, &graph,).get_markup(
                &Vec::new(),
                Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap()
            ),
            expected
        );

        let mut expected = expected_prefix;
        expected += format!(
            r#"                                    data: [
                                        [
                            {{
                                xAxis: '{increasing}',
                                itemStyle: {{ borderColor: 'rgba(44, 102, 79, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{increasing}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{increasing}',
                                itemStyle: {{ color: 'rgba(44, 102, 79, 0.05)' }},
                            }},
                            {{
                                xAxis: '{decreasing}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{decreasing}',
                                itemStyle: {{ borderColor: 'rgba(44, 102, 79, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{decreasing}'
                            }}
                        ],[
                            {{
                                xAxis: '{decreasing}',
                                itemStyle: {{ borderColor: 'rgba(179, 65, 65, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{decreasing}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{decreasing}',
                                itemStyle: {{ color: 'rgba(179, 65, 65, 0.05)' }},
                            }},
                            {{
                                xAxis: '{cancelling}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{cancelling}',
                                itemStyle: {{ borderColor: 'rgba(179, 65, 65, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{cancelling}'
                            }}
                        ],[
                            {{
                                xAxis: '{cancelling}',
                                itemStyle: {{ borderColor: 'rgba(179, 65, 65, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{cancelling}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{cancelling}',
                                itemStyle: {{ color: 'rgba(179, 65, 65, 0.05)' }},
                            }},
                            {{
                                xAxis: '{finishing}'
                            }}
                        ],
                        [
                            {{
                                xAxis: '{finishing}',
                                itemStyle: {{ borderColor: 'rgba(179, 65, 65, 0.25)', borderWidth: 1 }},
                            }},
                            {{
                                xAxis: '{finishing}'
                            }}
                        ],
                                    ]
                                }},
                                data: [["{data_series_prefix}:32",123],["{data_series_prefix}:33",111],["{data_series_prefix}:34",99],["{data_series_prefix}:35",134]],
                            }},
                            
                        ]
                    }});
                </script>
            </div>"#,
            data_series_prefix = Local
                .timestamp_opt(Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap().timestamp(), 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M"),
            increasing = Local
                .timestamp_opt(Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap().timestamp(), 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S"),
            decreasing = Local
                .timestamp_opt(Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 33).unwrap().timestamp(), 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S"),
            cancelling = Local
                .timestamp_opt(Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 34).unwrap().timestamp(), 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S"),
            finishing = Local
                .timestamp_opt(Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 35).unwrap().timestamp(), 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S"),
        ).as_str();

        let steps = vec![
            TestPlanHistory {
                action: TestPlanStepAction::Increasing,
                timestamp: Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap(),
                users: 123,
            },
            TestPlanHistory {
                action: TestPlanStepAction::Decreasing,
                timestamp: Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 33).unwrap(),
                users: 123,
            },
            TestPlanHistory {
                action: TestPlanStepAction::Canceling,
                timestamp: Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 34).unwrap(),
                users: 123,
            },
            TestPlanHistory {
                action: TestPlanStepAction::Finished,
                timestamp: Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 35).unwrap(),
                users: 123,
            },
        ];

        assert_eq!(
            Graph::new("graph-rps", "Requests #", true, &graph,).get_markup(
                &steps,
                Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap()
            ),
            expected
        );

        // It should make no difference if we disable granular graphs, since we only have one
        // request.
        assert_eq!(
            Graph::new("graph-rps", "Requests #", false, &graph,).get_markup(
                &steps,
                Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap()
            ),
            expected
        );

        let user_data: TimeSeries<usize, usize> = TimeSeries {
            origin: 0,
            data: VecDeque::from(vec![23, 12, 44, 22]),
            phantom: PhantomData,
            total: 0,
        };
        graph.insert("GET /user".to_string(), user_data);

        let markup = Graph::new("graph-rps", "Requests #", true, &graph).get_markup(
            &Vec::new(),
            Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap(),
        );
        let expected_legend = r#"
                        legend: {
                            type: 'plain',
                            width: '75%',
                            data: ["Total","GET /"#;
        assert!(
            markup.contains(expected_legend),
            "legend {} not found in {}",
            expected_legend,
            markup
        );

        let expected_line = format!(
            r#"{{
                                name: 'Total',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                lineStyle: {{ color: '#2c664f' }},
                                areaStyle: {{ color: '#378063' }},
                                markArea: {{
                                    data: [
                                        
                                    ]
                                }},
                                data: [["{data_series_prefix}:32",146],["{data_series_prefix}:33",123],["{data_series_prefix}:34",143],["{data_series_prefix}:35",156]],
                            }},"#,
            data_series_prefix = Local
                .timestamp_opt(
                    Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32)
                        .unwrap()
                        .timestamp(),
                    0
                )
                .unwrap()
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        let expected_line = format!(
            r#"{{
                                name: 'GET /',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                data: [["{data_series_prefix}:32",123],["{data_series_prefix}:33",111],["{data_series_prefix}:34",99],["{data_series_prefix}:35",134]],
                            }},"#,
            data_series_prefix = Local
                .timestamp_opt(
                    Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32)
                        .unwrap()
                        .timestamp(),
                    0
                )
                .unwrap()
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        let expected_line = format!(
            r#"{{
                                name: 'GET /user',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                data: [["{data_series_prefix}:32",23],["{data_series_prefix}:33",12],["{data_series_prefix}:34",44],["{data_series_prefix}:35",22]],
                            }},"#,
            data_series_prefix = Local
                .timestamp_opt(
                    Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32)
                        .unwrap()
                        .timestamp(),
                    0
                )
                .unwrap()
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        // Now same graph with granular data disabled.
        let expected_nongranular_prefix =
            expected_graph_html_prefix("graph-rps", "Requests #", "Total");
        let mut expected = expected_nongranular_prefix.to_owned();
        expected += format!(
            r#"                                    data: [
                                        
                                    ]
                                }},
                                data: [["{data_series_prefix}:32",146],["{data_series_prefix}:33",123],["{data_series_prefix}:34",143],["{data_series_prefix}:35",156]],
                            }},
                            
                        ]
                    }});
                </script>
            </div>"#,
            data_series_prefix = Local
                .timestamp_opt(Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap().timestamp(), 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M")
        ).as_str();

        assert_eq!(
            Graph::new("graph-rps", "Requests #", false, &graph,).get_markup(
                &Vec::new(),
                Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap()
            ),
            expected
        );

        let more_data: TimeSeries<usize, usize> = TimeSeries {
            origin: 0,
            data: VecDeque::from(vec![1, 1, 1, 1]),
            phantom: PhantomData,
            total: 0,
        };
        graph.insert("GET /one".to_string(), more_data.clone());
        graph.insert("GET /two".to_string(), more_data.clone());
        graph.insert("GET /three".to_string(), more_data);

        let markup = Graph::new("graph-rps", "Requests #", true, &graph).get_markup(
            &Vec::new(),
            Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap(),
        );
        let expected_legend = r#"
                        legend: {
                            type: 'scroll',
                            width: '75%',
                            data: ["Total","GET /"#;
        assert!(
            markup.contains(expected_legend),
            "legend {} not found in {}",
            expected_legend,
            markup
        );

        let expected_line = format!(
            r#"{{
                                name: 'Total',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                lineStyle: {{ color: '#2c664f' }},
                                areaStyle: {{ color: '#378063' }},
                                markArea: {{
                                    data: [
                                        
                                    ]
                                }},
                                data: [["{data_series_prefix}:32",149],["{data_series_prefix}:33",126],["{data_series_prefix}:34",146],["{data_series_prefix}:35",159]],
                            }},"#,
            data_series_prefix = Local
                .timestamp_opt(
                    Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32)
                        .unwrap()
                        .timestamp(),
                    0
                )
                .unwrap()
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        let expected_line = format!(
            r#"{{
                                name: 'GET /',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                data: [["{data_series_prefix}:32",123],["{data_series_prefix}:33",111],["{data_series_prefix}:34",99],["{data_series_prefix}:35",134]],
                            }},"#,
            data_series_prefix = Local
                .timestamp_opt(
                    Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32)
                        .unwrap()
                        .timestamp(),
                    0
                )
                .unwrap()
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        let expected_line = format!(
            r#"{{
                                name: 'GET /user',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                data: [["{data_series_prefix}:32",23],["{data_series_prefix}:33",12],["{data_series_prefix}:34",44],["{data_series_prefix}:35",22]],
                            }},"#,
            data_series_prefix = Local
                .timestamp_opt(
                    Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32)
                        .unwrap()
                        .timestamp(),
                    0
                )
                .unwrap()
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        let expected_line = format!(
            r#"{{
                                name: 'GET /one',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                data: [["{data_series_prefix}:32",1],["{data_series_prefix}:33",1],["{data_series_prefix}:34",1],["{data_series_prefix}:35",1]],
                            }},"#,
            data_series_prefix = Local
                .timestamp_opt(
                    Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32)
                        .unwrap()
                        .timestamp(),
                    0
                )
                .unwrap()
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        let expected_line = format!(
            r#"{{
                                name: 'GET /two',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                data: [["{data_series_prefix}:32",1],["{data_series_prefix}:33",1],["{data_series_prefix}:34",1],["{data_series_prefix}:35",1]],
                            }},"#,
            data_series_prefix = Local
                .timestamp_opt(
                    Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32)
                        .unwrap()
                        .timestamp(),
                    0
                )
                .unwrap()
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        let expected_line = format!(
            r#"{{
                                name: 'GET /three',
                                type: 'line',
                                symbol: 'none',
                                sampling: 'lttb',
                                data: [["{data_series_prefix}:32",1],["{data_series_prefix}:33",1],["{data_series_prefix}:34",1],["{data_series_prefix}:35",1]],
                            }},"#,
            data_series_prefix = Local
                .timestamp_opt(
                    Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32)
                        .unwrap()
                        .timestamp(),
                    0
                )
                .unwrap()
                .format("%Y-%m-%d %H:%M"),
        );
        assert!(
            markup.contains(expected_line.as_str()),
            "line {} not found in {}",
            expected_line,
            markup
        );

        // Now same graph with granular data disabled.
        let mut expected = expected_nongranular_prefix;
        expected += format!(
            r#"                                    data: [
                                        
                                    ]
                                }},
                                data: [["{data_series_prefix}:32",149],["{data_series_prefix}:33",126],["{data_series_prefix}:34",146],["{data_series_prefix}:35",159]],
                            }},
                            
                        ]
                    }});
                </script>
            </div>"#,
            data_series_prefix = Local
                .timestamp_opt(Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap().timestamp(), 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M")
        ).as_str();

        assert_eq!(
            Graph::new("graph-rps", "Requests #", false, &graph,).get_markup(
                &Vec::new(),
                Utc.with_ymd_and_hms(2021, 11, 21, 21, 20, 32).unwrap()
            ),
            expected
        );
    }
}
