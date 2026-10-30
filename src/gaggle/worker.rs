//! The worker side: what the metrics processor keeps beside its own state
//! while it runs as a Gaggle worker, and how it turns that into deltas.

use super::delta::{
    severity_code, CoPart, DeltaKind, ErrorKey, GaggleDelta, GraphPart, MetricsPart, RequestPart,
    ScenarioPart, Second, Timing, TransactionErrorKey, TransactionPart,
};
use super::shadow::ShadowMerge;
use crate::graph::GraphData;
use crate::metrics::{GooseMetrics, GooseRequestCounterRegistry};
use std::collections::HashMap;
use std::sync::atomic::Ordering;

/// A worker's metrics since its last delta, kept by the metrics processor
/// beside its whole run state, and fed by the same code.
pub(crate) struct WorkerState {
    pub(crate) worker: u32,
    /// The last sequence number used.
    seq: u64,
    /// Metrics since the last delta, laid out as the processor's (every
    /// scenario and transaction present), filled by the processor's own
    /// merge code.
    pub(crate) pending_metrics: GooseMetrics,
    /// Graph data since the last delta, with every request name's response
    /// times kept by second.
    pub(crate) pending_graph: GraphData,
    /// An empty `pending_metrics`, to start each delta from.
    template: GooseMetrics,
    /// Whether synthetic response times are kept by second.
    synthetic: bool,
    /// How many seconds before a delta's own second the next delta's
    /// pending graph data starts: the request timeout plus a minute, which
    /// covers most late requests and corrections. A metric from further
    /// back moves the start back first (see
    /// `MetricsProcessor::shadow_metric`), so nothing is dropped.
    lag: usize,
    /// Each request's success and failure counts as last sent, from the
    /// atomic counters the user threads write.
    counts_sent: HashMap<String, (u64, u64)>,
    /// The users series as last sent.
    users_sent: (usize, Vec<usize>),
    /// Errors `set_success()` took back since the last delta, by request and
    /// second.
    pub(crate) removals: Vec<(String, usize)>,
    /// The graph rows sent since the epoch started, for the final delta.
    sent_graph: GraphPart,
    /// The series second of the last metrics reset, if any.
    series_origin: Option<Second>,
    /// The `--gaggle-shadow-merge` self check, when it runs.
    pub(crate) shadow: Option<ShadowMerge>,
}

impl WorkerState {
    /// Worker `worker`, whose processor's empty metrics are `template`;
    /// `synthetic` when coordinated omission mitigation is on, and `lag`
    /// as [`Self::lag`] describes.
    pub(crate) fn new(worker: u32, template: GooseMetrics, synthetic: bool, lag: usize) -> Self {
        WorkerState {
            worker,
            seq: 0,
            pending_metrics: template.clone(),
            pending_graph: GraphData::gaggle_pending(synthetic, 0),
            template,
            synthetic,
            lag,
            counts_sent: HashMap::new(),
            users_sent: (0, Vec::new()),
            removals: Vec::new(),
            sent_graph: GraphPart::default(),
            series_origin: None,
            shadow: None,
        }
    }

    /// Set [`Self::lag`] (unit tests).
    #[cfg(test)]
    pub(crate) fn set_lag_for_test(&mut self, lag: usize) {
        self.lag = lag;
    }

    /// A metrics reset at series second `second`: what was pending belongs
    /// to the old epoch, and the counters start again from 0. `running`
    /// are the `(scenario index, user)` of the users still running, and
    /// `co_started` when the processor's coordinated omission metrics
    /// started again.
    pub(crate) fn reset(
        &mut self,
        second: usize,
        running: &[(usize, usize)],
        co_started: Option<u64>,
    ) {
        if let (Some(co), Some(started)) = (
            self.template.coordinated_omission_metrics.as_mut(),
            co_started,
        ) {
            co.started_secs = started;
        }
        self.pending_metrics = self.template.clone();
        for &(scenario_index, user) in running {
            if let Some(scenario) = self.pending_metrics.scenarios.get_mut(scenario_index) {
                scenario.users.insert(user);
            }
        }
        self.pending_graph =
            GraphData::gaggle_pending(self.synthetic, second.saturating_sub(self.lag));
        self.counts_sent.clear();
        self.removals.clear();
        self.sent_graph.clear_rates();
        self.series_origin = Some(second as Second);
    }

    /// The next sequence number.
    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// The graph rows pending, with the errors taken back, and pending
    /// starting again `lag` seconds before `now`.
    fn take_graph(&mut self, now: Second) -> GraphPart {
        let origin = (now as usize).saturating_sub(self.lag);
        let mut pending = std::mem::replace(
            &mut self.pending_graph,
            GraphData::gaggle_pending(self.synthetic, origin),
        );
        let mut graph = pending.gaggle_rows();
        for (key, second) in self.removals.drain(..) {
            *graph
                .errors
                .entry(key)
                .or_default()
                .entry(second as Second)
                .or_default() -= 1;
        }
        graph
    }

    /// What changed since the last delta. `cumulative` is the processor's
    /// whole run state (for the users series and each request's name),
    /// `registry` the atomic counters, `epoch` the current metrics epoch and
    /// `now` the series second.
    pub(crate) fn take_delta(
        &mut self,
        cumulative: (&GooseMetrics, &GraphData),
        registry: &GooseRequestCounterRegistry,
        epoch: u64,
        now: Second,
    ) -> GaggleDelta {
        let mut delta = GaggleDelta::new(self.worker, self.next_seq(), epoch, DeltaKind::Delta);
        delta.now = now;
        delta.series_origin = self.series_origin;
        let pending = std::mem::replace(&mut self.pending_metrics, self.template.clone());
        delta.metrics = metrics_part(&pending);
        self.add_counts(&mut delta.metrics, cumulative.0, registry);
        if let Some(co) = delta.metrics.co.as_mut() {
            co.cap_events();
        }
        let mut graph = self.take_graph(now);
        self.sent_graph.add_rates(&graph);
        graph.users = cumulative.1.gaggle_users_since(&mut self.users_sent);
        delta.graph = graph;
        delta
    }

    /// The worker's whole state, which replaces everything the manager
    /// holds for it: the metrics from the processor's own whole run state
    /// `cumulative`, whose counters must be up to date, and the graph rows
    /// of every delta sent this epoch and those still pending.
    pub(crate) fn take_final(
        &mut self,
        cumulative: (&GooseMetrics, &GraphData),
        epoch: u64,
        now: Second,
    ) -> GaggleDelta {
        let mut delta = GaggleDelta::new(self.worker, self.next_seq(), epoch, DeltaKind::Final);
        delta.now = now;
        delta.series_origin = self.series_origin;
        delta.metrics = metrics_part(cumulative.0);
        for (key, aggregate) in &cumulative.0.requests {
            if let Some(request) = delta.metrics.requests.get_mut(key) {
                request.success = aggregate.success_count as i64;
                request.fail = aggregate.fail_count as i64;
            }
        }
        let pending = self.take_graph(now);
        self.sent_graph.add_rates(&pending);
        let mut graph = self.sent_graph.clone();
        let mut everything = (0, Vec::new());
        graph.users = cumulative.1.gaggle_users_since(&mut everything);
        self.users_sent = everything;
        delta.graph = graph;
        self.pending_metrics = self.template.clone();
        delta
    }

    /// Add each request's success and failure counts since the last delta,
    /// from the atomic counters. A request the processor has not yet seen
    /// a metric for waits for the next delta, which then names it.
    fn add_counts(
        &mut self,
        part: &mut MetricsPart,
        cumulative: &GooseMetrics,
        registry: &GooseRequestCounterRegistry,
    ) {
        let registry = registry
            .lock()
            .expect("request_counter_registry mutex poisoned");
        for (key, counters) in registry.iter() {
            let now = (
                counters.success.load(Ordering::Relaxed),
                counters.failure.load(Ordering::Relaxed),
            );
            let sent = self.counts_sent.get(key).copied().unwrap_or((0, 0));
            if now == sent {
                continue;
            }
            let request = match part.requests.get_mut(key) {
                Some(request) => request,
                None => match cumulative.requests.get(key) {
                    Some(aggregate) => part.requests.entry(key.clone()).or_insert_with(|| {
                        RequestPart::new(
                            &aggregate.path,
                            aggregate.method,
                            &aggregate.custom_method,
                        )
                    }),
                    None => continue,
                },
            };
            request.success += now.0 as i64 - sent.0 as i64;
            request.fail += now.1 as i64 - sent.1 as i64;
            self.counts_sent.insert(key.clone(), now);
        }
    }
}

/// Everything in `metrics` as a delta carries it, apart from the success
/// and failure counts, which the atomic counters hold.
pub(crate) fn metrics_part(metrics: &GooseMetrics) -> MetricsPart {
    let mut part = MetricsPart::default();
    for (key, aggregate) in &metrics.requests {
        let mut request =
            RequestPart::new(&aggregate.path, aggregate.method, &aggregate.custom_method);
        request.raw = Timing::from_data(&aggregate.raw_data);
        request.adjusted = aggregate
            .coordinated_omission_data
            .as_ref()
            .map(Timing::from_data);
        request.status_codes = aggregate
            .status_code_counts
            .iter()
            .map(|(&code, &count)| (code, count as u64))
            .collect();
        request.status_timings = aggregate
            .status_code_timings
            .iter()
            .map(|(&code, summary)| (code, summary.clone()))
            .collect();
        part.requests.insert(key.clone(), request);
    }
    for transaction in metrics.transactions.iter().flatten() {
        if transaction.run_times.counter == 0
            && transaction.success_count == 0
            && transaction.fail_count == 0
        {
            continue;
        }
        part.transactions.insert(
            (
                transaction.scenario_index as u64,
                transaction.transaction_index as u64,
            ),
            TransactionPart {
                run_times: Timing::from_data(&transaction.run_times),
                success: transaction.success_count as u64,
                fail: transaction.fail_count as u64,
            },
        );
    }
    for scenario in &metrics.scenarios {
        if scenario.users.is_empty() && scenario.run_times.counter == 0 {
            continue;
        }
        part.scenarios.insert(
            scenario.index as u64,
            ScenarioPart {
                users: scenario.users.iter().map(|&user| user as u64).collect(),
                run_times: Timing::from_data(&scenario.run_times),
            },
        );
    }
    for (key, error) in &metrics.errors {
        part.errors.insert(
            ErrorKey {
                key: key.clone(),
                method: error.method,
                name: error.name.clone(),
                error: error.error.clone(),
                custom_method: error.custom_method.clone(),
            },
            error.occurrences as u64,
        );
    }
    for (key, error) in &metrics.transaction_errors {
        part.transaction_errors.insert(
            TransactionErrorKey {
                key: key.clone(),
                scenario_index: error.scenario_index as u64,
                scenario_name: error.scenario_name.clone(),
                transaction_index: error.transaction_index as u64,
                transaction_name: error.transaction_name.clone(),
                error: error.error.clone(),
            },
            error.occurrences as u64,
        );
    }
    part.hosts = metrics.hosts.iter().cloned().collect();
    part.co = metrics
        .coordinated_omission_metrics
        .as_ref()
        .map(|co| CoPart {
            strategy: co.mitigation_strategy.clone(),
            started_secs: co.started_secs,
            actual: co.actual_requests,
            synthetic: co.synthetic_requests,
            severity: co
                .severity_histogram
                .iter()
                .map(|(&severity, &count)| (severity_code(severity), count as u64))
                .collect(),
            events: co.co_events.clone(),
            events_dropped: 0,
        });
    part
}
