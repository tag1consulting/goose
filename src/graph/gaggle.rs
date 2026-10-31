use super::*;
use crate::gaggle::delta::{Average, GraphPart, Histogram, Second};
use std::collections::BTreeSet;

impl SecondHistogram {
    /// This second's response times as a delta carries them.
    fn to_gaggle(&self) -> Histogram {
        Histogram {
            rows: self
                .buckets()
                .coalesce(merge_bucket)
                .map(|(row, count)| (row as u32, count as u64))
                .collect(),
            minimum_us: self.minimum_time_us,
            maximum_us: self.maximum_time_us,
            total_us: self.total_time_us,
            count: self.counter as u64,
        }
    }

    /// [`Self::to_gaggle`] read back.
    fn from_gaggle(histogram: &Histogram) -> Self {
        let mut second = SecondHistogram::default();
        for &(row, count) in &histogram.rows {
            second.add_bucket(row as usize, count as usize);
        }
        second.add_totals(
            histogram.minimum_us,
            histogram.maximum_us,
            histogram.total_us,
            histogram.count as usize,
        );
        second
    }
}

impl LatencyRing {
    /// Every second of a worker's ring, as a delta carries it: its open
    /// seconds, and `folded`, every second it packed or could not hold.
    fn gaggle_seconds(
        &self,
        folded: Option<&BTreeMap<usize, SecondHistogram>>,
    ) -> BTreeMap<Second, Histogram> {
        let mut seconds: BTreeMap<usize, SecondHistogram> = folded.cloned().unwrap_or_default();
        for slot in self.slots.iter().flatten() {
            if slot.data.open.is_some() && slot.data.counter > 0 {
                seconds.entry(slot.second).or_default().merge(&slot.data);
            }
        }
        seconds
            .into_iter()
            .filter(|(_, data)| data.counter > 0)
            .map(|(second, data)| (second as Second, data.to_gaggle()))
            .collect()
    }

    /// Every second the ring holds, for a comparison.
    fn seconds_for_diff(&self) -> BTreeMap<usize, &SecondHistogram> {
        self.slots
            .iter()
            .flatten()
            .map(|slot| (slot.second, &slot.data))
            .collect()
    }
}

impl LatencyHistograms {
    /// Add one second's measured response times for `key`, admitting the
    /// name to a ring of its own while there is room.
    fn merge_second(&mut self, key: &str, second: usize, histogram: &SecondHistogram) {
        self.fit_columns(second);
        let index = self.ring_index(key);
        let rev = self.next_rev();
        let (ring, mut sink) = self.ring(index);
        ring.write(second, rev, &mut sink, |held| held.merge(histogram));
    }

    /// Add one second's synthetic response times for `key`: to all
    /// requests', and to the name's own when it has a measured ring, as
    /// [`Self::record_synthetic`] does.
    fn merge_synthetic_second(&mut self, key: &str, second: usize, histogram: &SecondHistogram) {
        self.fit_columns(second);
        let rev = self.next_rev();
        let (origin, columns) = (self.origin, self.all_columns.is_some());
        let (all_width, request_width) = (self.all_width, self.request_width);
        let admitted = self.by_request.contains_key(key) || self.admit_all;
        let Some((all, by_request)) = self.synthetic.as_mut() else {
            return;
        };
        all.write(
            second,
            rev,
            &mut ColumnSink::own(origin, all_width),
            |held| held.merge(histogram),
        );
        if !admitted {
            return;
        }
        let ring = by_request
            .entry(key.to_string())
            .or_insert_with(|| LatencyRing::with_columns(columns));
        ring.write(
            second,
            rev,
            &mut ColumnSink::own(origin, request_width),
            |held| held.merge(histogram),
        );
    }
}

/// Every `(second, value)` of a series with data.
fn rows<'a, T: Clone + TimeSeriesValue<T, U>, U>(
    series: &'a TimeSeries<T, U>,
    keep: impl Fn(&T) -> bool + 'a,
) -> impl Iterator<Item = (Second, T)> + 'a {
    series
        .data
        .iter()
        .enumerate()
        .filter(move |(_, value)| keep(value))
        .map(move |(at, value)| ((series.origin + at) as Second, value.clone()))
}

/// How a manager builds graph data: as the metrics processor of a run
/// with the same settings would.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GraphSettings {
    /// Per second graphs are kept at all.
    pub(crate) records: bool,
    /// Latency histograms for the dashboard's percentiles over time.
    pub(crate) latency: bool,
    /// Synthetic histograms too (coordinated omission mitigation is on).
    pub(crate) synthetic: bool,
    /// Whole run columns for the report's heatmap.
    pub(crate) columns: bool,
}

/// The last heatmap row any response time falls in.
pub(crate) fn gaggle_max_row() -> u32 {
    u32::try_from(display_index(u64::MAX)).unwrap_or(u32::MAX)
}

/// `count` as a series count, at most `u32::MAX`.
fn series_count(count: u64) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

impl GraphData {
    /// How a manager builds graph data like this, which `records` per
    /// second graphs or not.
    pub(crate) fn gaggle_settings(&self, records: bool) -> GraphSettings {
        GraphSettings {
            records,
            latency: self.latency.is_some(),
            synthetic: self.records_synthetic_latency(),
            columns: self
                .latency
                .as_ref()
                .is_some_and(|latency| latency.all_columns.is_some()),
        }
    }

    /// Start every series at `origin` when it starts later, so a sample
    /// from as far back as `origin` is kept rather than dropped.
    pub(crate) fn gaggle_lower_origin(&mut self, origin: usize) {
        fn lower<T: Clone + TimeSeriesValue<T, U>, U>(
            series: &mut TimeSeries<T, U>,
            origin: usize,
        ) {
            while series.origin > origin {
                if !series.data.is_empty() {
                    series.data.push_front(T::initial_value());
                }
                series.origin -= 1;
            }
        }
        if origin >= self.users_per_second.origin {
            return;
        }
        lower(&mut self.users_per_second, origin);
        lower(&mut self.transactions_per_second, origin);
        lower(&mut self.scenarios_per_second, origin);
        for items in [&mut self.requests_per_second, &mut self.errors_per_second] {
            items.origin = items.origin.min(origin);
            for series in items.series.values_mut() {
                lower(&mut series.counts, origin);
            }
        }
        for series in self.average_response_time_per_second.values_mut() {
            lower(series, origin);
        }
    }

    /// The last second any series holds a value for.
    pub(crate) fn gaggle_last_second(&self) -> Option<usize> {
        self.requests_per_second
            .series
            .values()
            .filter_map(|series| series.last_absolute_second())
            .chain(self.users_per_second.last_absolute_second())
            .chain(self.transactions_per_second.last_absolute_second())
            .chain(self.scenarios_per_second.last_absolute_second())
            .max()
    }

    /// A worker's graph data for what its next delta sends: every
    /// request name gets a ring of its own, measured and synthetic, and
    /// no whole run columns are kept. Its series start at second
    /// `origin`, so each delta costs the seconds since then, not since
    /// the run started; a sample from before `origin` would be lost.
    pub(crate) fn gaggle_pending(synthetic: bool, origin: usize) -> Self {
        let mut graph_data = GraphData::for_metrics_processor(true, synthetic, false);
        graph_data.users_per_second = TimeSeries::with_origin(origin);
        graph_data.requests_per_second = ItemsPerSecond::with_origin(origin);
        graph_data.errors_per_second = ItemsPerSecond::with_origin(origin);
        graph_data.transactions_per_second = TimeSeries::with_origin(origin);
        graph_data.scenarios_per_second = TimeSeries::with_origin(origin);
        if let Some(latency) = graph_data.latency.as_mut() {
            latency.admit_all = true;
            latency.folded = Some(Default::default());
        }
        graph_data
    }

    /// Everything this worker copy holds, as a delta carries it, apart
    /// from the users, which the whole run's copy sends. Errors are
    /// counted here; those `set_success()` took back are added by the
    /// caller, as this copy never held them.
    pub(crate) fn gaggle_rows(&mut self) -> GraphPart {
        self.move_pending_latency();
        let mut part = GraphPart::default();
        for (key, series) in &self.requests_per_second.series {
            let held: BTreeMap<Second, u64> = rows(&series.counts, |&count| count > 0)
                .map(|(second, count)| (second, count as u64))
                .collect();
            if !held.is_empty() {
                part.requests.insert(key.clone(), held);
            }
        }
        for (key, series) in &self.errors_per_second.series {
            let held: BTreeMap<Second, i64> = rows(&series.counts, |&count| count > 0)
                .map(|(second, count)| (second, count as i64))
                .collect();
            if !held.is_empty() {
                part.errors.insert(key.clone(), held);
            }
        }
        for (key, series) in &self.average_response_time_per_second {
            let held: BTreeMap<Second, Average> = rows(series, |average| average.count > 0)
                .map(|(second, average)| {
                    (
                        second,
                        Average {
                            count: average.count as u64,
                            sum_ms: average.average as f64 * average.count as f64,
                        },
                    )
                })
                .collect();
            if !held.is_empty() {
                part.averages.insert(key.clone(), held);
            }
        }
        part.transactions = rows(&self.transactions_per_second, |&count| count > 0)
            .map(|(second, count)| (second, count as u64))
            .collect();
        part.scenarios = rows(&self.scenarios_per_second, |&count| count > 0)
            .map(|(second, count)| (second, count as u64))
            .collect();
        if let Some(latency) = self.latency.as_ref() {
            let folded = latency.folded.as_ref();
            for (key, &index) in &latency.by_request {
                let seconds = latency.rings[index]
                    .gaggle_seconds(folded.and_then(|(measured, _)| measured.get(index)));
                if !seconds.is_empty() {
                    part.latency.insert(key.clone(), seconds);
                }
            }
            debug_assert!(latency.others.gaggle_seconds(None).is_empty());
            if let Some((_, by_request)) = latency.synthetic.as_ref() {
                for (key, ring) in by_request {
                    let seconds =
                        ring.gaggle_seconds(folded.and_then(|(_, synthetic)| synthetic.get(key)));
                    if !seconds.is_empty() {
                        part.synthetic.insert(key.clone(), seconds);
                    }
                }
            }
        }
        part
    }

    /// The users levels that differ from `sent`, the copy of the users
    /// series last sent, which is brought up to date.
    pub(crate) fn gaggle_users_since(
        &self,
        sent: &mut (usize, Vec<usize>),
    ) -> BTreeMap<Second, u64> {
        let series = &self.users_per_second;
        let mut changed = BTreeMap::new();
        for (at, &users) in series.data.iter().enumerate() {
            let second = series.origin + at;
            let before = second
                .checked_sub(sent.0)
                .and_then(|index| sent.1.get(index).copied());
            if before != Some(users) {
                changed.insert(second as Second, users as u64);
            }
        }
        *sent = (series.origin, series.data.iter().copied().collect());
        changed
    }

    /// Graph data built from `part`, merged from every worker, as the
    /// metrics processor of a run with `settings` builds it: the users
    /// levels, the rate series from `series_origin` when a metrics reset
    /// restarted them there, then every second before `complete` (all
    /// when `None`) in order, names admitted to the heatmap's rings in
    /// the order of the first second they have response times in and by
    /// name within it.
    pub(crate) fn gaggle_build(
        part: &GraphPart,
        settings: GraphSettings,
        series_origin: Option<Second>,
        complete: Option<Second>,
    ) -> GraphData {
        let mut graph_data = GraphData::for_metrics_processor(
            settings.latency,
            settings.synthetic,
            settings.columns,
        );
        let done = |second: Second| complete.is_none_or(|complete| second < complete);
        // The users series is kept whether or not per second graphs are.
        for (&second, &users) in part.users.iter().filter(|(&second, _)| done(second)) {
            graph_data.record_users_per_second(users as usize, second as usize);
        }
        if !settings.records {
            graph_data.finish_latency_columns();
            return graph_data;
        }
        if let Some(origin) = series_origin {
            graph_data.reset_rate_series(origin as usize);
        }
        for (key, rows) in &part.requests {
            for (&second, &count) in rows.iter().filter(|(&second, _)| done(second)) {
                graph_data
                    .requests_per_second
                    .increment(key, second as usize, series_count(count));
            }
        }
        for (key, rows) in &part.errors {
            for (&second, &count) in rows.iter().filter(|(&second, _)| done(second)) {
                graph_data.errors_per_second.increment(
                    key,
                    second as usize,
                    series_count(count.max(0) as u64),
                );
            }
        }
        let origin = graph_data.users_per_second.origin;
        for (key, rows) in &part.averages {
            for (&second, average) in rows.iter().filter(|(&second, _)| done(second)) {
                if average.count == 0 {
                    continue;
                }
                let series = graph_data
                    .average_response_time_per_second
                    .entry(key.clone())
                    .or_insert_with(|| TimeSeries::with_origin(origin));
                series.merge_value(
                    second as usize,
                    &MovingAverage {
                        count: series_count(average.count),
                        average: (average.sum_ms / average.count as f64) as f32,
                    },
                );
            }
        }
        for (&second, &count) in part.transactions.iter().filter(|(&second, _)| done(second)) {
            graph_data.record_transactions_per_second_batch(second as usize, count as usize);
        }
        for (&second, &count) in part.scenarios.iter().filter(|(&second, _)| done(second)) {
            graph_data.record_scenarios_per_second_batch(second as usize, count as usize);
        }
        if let Some(latency) = graph_data.latency.as_mut() {
            // Measured times first, each second in order and by name
            // within it, so names are admitted to rings as a single
            // process admits names first seen in that order. Synthetic
            // times after: a single process records them, back dated to
            // when a stall began, only while it handles a later request
            // of the same name, so the name was admitted first.
            for (synthetic, by_key) in [(false, &part.latency), (true, &part.synthetic)] {
                let mut seconds: BTreeMap<Second, Vec<(&str, &Histogram)>> = BTreeMap::new();
                for (key, rows) in by_key {
                    for (&second, histogram) in rows.iter().filter(|(&second, _)| done(second)) {
                        seconds
                            .entry(second)
                            .or_default()
                            .push((key.as_str(), histogram));
                    }
                }
                for (second, entries) in seconds {
                    for (key, histogram) in entries {
                        let histogram = SecondHistogram::from_gaggle(histogram);
                        if synthetic {
                            latency.merge_synthetic_second(key, second as usize, &histogram);
                        } else {
                            latency.merge_second(key, second as usize, &histogram);
                        }
                    }
                }
            }
        }
        graph_data.finish_latency_columns();
        graph_data
    }

    /// Every difference between this graph data and `other`, as
    /// `field: this != other`. Both must be finished
    /// ([`Self::finish_latency_columns`]). Average response times, an
    /// f32 moving average whose last digits depend on the order its
    /// times arrive in, match to f32 rounding; the users series' total,
    /// which counts how often a level was set, and the heatmap's change
    /// counters are left out.
    pub(crate) fn gaggle_diff(&self, other: &GraphData) -> Vec<String> {
        let mut out = Vec::new();
        fn series<T: Clone + TimeSeriesValue<T, U> + std::fmt::Debug + PartialEq, U>(
            out: &mut Vec<String>,
            field: &str,
            a: &TimeSeries<T, U>,
            b: &TimeSeries<T, U>,
        ) {
            if a.origin != b.origin || a.data != b.data {
                out.push(format!(
                    "{field}: origin {} {:?} != origin {} {:?}",
                    a.origin, a.data, b.origin, b.data
                ));
            }
        }
        for (name, a, b) in [
            (
                "requests_per_second",
                &self.requests_per_second,
                &other.requests_per_second,
            ),
            (
                "errors_per_second",
                &self.errors_per_second,
                &other.errors_per_second,
            ),
        ] {
            let keys: BTreeSet<&String> = a.series.keys().chain(b.series.keys()).collect();
            for key in keys {
                match (a.series.get(key), b.series.get(key)) {
                    (Some(x), Some(y)) => {
                        series(&mut out, &format!("{name}[{key}]"), &x.counts, &y.counts)
                    }
                    (x, y) => out.push(format!(
                        "{name}[{key}]: {:?} != {:?}",
                        x.map(|x| &x.counts.data),
                        y.map(|y| &y.counts.data)
                    )),
                }
            }
        }
        let keys: BTreeSet<&String> = self
            .average_response_time_per_second
            .keys()
            .chain(other.average_response_time_per_second.keys())
            .collect();
        for key in keys {
            let field = format!("average_response_time_per_second[{key}]");
            match (
                self.average_response_time_per_second.get(key),
                other.average_response_time_per_second.get(key),
            ) {
                (Some(a), Some(b)) => {
                    let close = a.origin == b.origin
                        && a.data.len() == b.data.len()
                        && a.data.iter().zip(&b.data).all(|(x, y)| {
                            x.count == y.count && crate::gaggle::f32_close(x.average, y.average)
                        });
                    if !close {
                        out.push(format!(
                            "{field}: origin {} {:?} != origin {} {:?}",
                            a.origin, a.data, b.origin, b.data
                        ));
                    }
                }
                (a, b) => out.push(format!(
                    "{field}: {:?} != {:?}",
                    a.map(|a| &a.data),
                    b.map(|b| &b.data)
                )),
            }
        }
        series(
            &mut out,
            "transactions_per_second",
            &self.transactions_per_second,
            &other.transactions_per_second,
        );
        series(
            &mut out,
            "scenarios_per_second",
            &self.scenarios_per_second,
            &other.scenarios_per_second,
        );
        series(
            &mut out,
            "users_per_second",
            &self.users_per_second,
            &other.users_per_second,
        );
        match (self.latency.as_ref(), other.latency.as_ref()) {
            (None, None) => {}
            (Some(a), Some(b)) => a.gaggle_diff(b, &mut out),
            (a, b) => out.push(format!(
                "latency: kept {} != kept {}",
                a.is_some(),
                b.is_some()
            )),
        }
        out
    }
}

impl LatencyHistograms {
    /// [`GraphData::gaggle_diff`] for the latency histograms.
    fn gaggle_diff(&self, other: &LatencyHistograms, out: &mut Vec<String>) {
        fn ring(out: &mut Vec<String>, field: &str, a: &LatencyRing, b: &LatencyRing) {
            let (x, y) = (a.seconds_for_diff(), b.seconds_for_diff());
            if x != y {
                out.push(format!("{field} seconds: {x:?} != {y:?}"));
            }
            if a.columns.as_ref().map(|c| &c.columns) != b.columns.as_ref().map(|c| &c.columns) {
                out.push(format!(
                    "{field} columns: {:?} != {:?}",
                    a.columns.as_ref().map(|c| &c.columns),
                    b.columns.as_ref().map(|c| &c.columns)
                ));
            }
        }
        // Which names have a ring of their own; the order rings were
        // made in shows nowhere.
        let names = |latency: &LatencyHistograms| {
            latency.by_request.keys().cloned().collect::<BTreeSet<_>>()
        };
        let (a_names, b_names) = (names(self), names(other));
        if a_names != b_names {
            out.push(format!("latency names: {a_names:?} != {b_names:?}"));
        } else {
            for name in &a_names {
                ring(
                    out,
                    &format!("latency[{name}]"),
                    &self.rings[self.by_request[name]],
                    &other.rings[other.by_request[name]],
                );
            }
        }
        ring(out, "latency others", &self.others, &other.others);
        for (field, a, b) in [
            ("origin", self.origin, other.origin),
            ("all_width", self.all_width, other.all_width),
            ("request_width", self.request_width, other.request_width),
        ] {
            if a != b {
                out.push(format!("latency {field}: {a} != {b}"));
            }
        }
        if self.all_columns.as_ref().map(|c| &c.columns)
            != other.all_columns.as_ref().map(|c| &c.columns)
        {
            out.push(format!(
                "latency all columns: {:?} != {:?}",
                self.all_columns.as_ref().map(|c| &c.columns),
                other.all_columns.as_ref().map(|c| &c.columns)
            ));
        }
        match (self.synthetic.as_ref(), other.synthetic.as_ref()) {
            (None, None) => {}
            (Some((a_all, a_by)), Some((b_all, b_by))) => {
                ring(out, "synthetic all", a_all, b_all);
                let keys: BTreeSet<&String> = a_by.keys().chain(b_by.keys()).collect();
                for key in keys {
                    match (a_by.get(key), b_by.get(key)) {
                        (Some(a), Some(b)) => ring(out, &format!("synthetic[{key}]"), a, b),
                        (a, b) => out.push(format!(
                            "synthetic[{key}]: kept {} != kept {}",
                            a.is_some(),
                            b.is_some()
                        )),
                    }
                }
            }
            (a, b) => out.push(format!(
                "synthetic: kept {} != kept {}",
                a.is_some(),
                b.is_some()
            )),
        }
    }
}
