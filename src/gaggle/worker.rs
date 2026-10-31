//! The worker side: what the metrics processor keeps beside its own state
//! while it runs as a Gaggle worker, and how it turns that into deltas.

use super::delta::{
    severity_code, CoPart, DeltaKind, ErrorKey, GaggleDelta, GraphPart, MetricsPart, RequestPart,
    ScenarioPart, Second, Timing, TransactionErrorKey, TransactionPart,
};
use super::shadow::ShadowMerge;
use crate::graph::GraphData;
use crate::metrics::coordinated_omission::CoordinatedOmissionMetrics;
use crate::metrics::{
    GooseErrorMetricAggregate, GooseMetrics, GooseRequestCounterRegistry,
    GooseRequestMetricAggregate, GooseTransactionErrorMetricAggregate, ScenarioMetricAggregate,
    TransactionMetricAggregate,
};
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
        delta.series_origin = self.series_origin;
        let pending = std::mem::replace(&mut self.pending_metrics, self.template.clone());
        delta.metrics = metrics_part(&pending);
        self.add_counts(&mut delta.metrics, cumulative.0, registry);
        if let Some(co) = delta.metrics.co.as_mut() {
            co.cap_events();
        }
        let mut graph = self.take_graph(now);
        graph.users = cumulative.1.gaggle_users_since(&mut self.users_sent);
        delta.graph = graph;
        delta.now = clock(&delta, now);
        delta
    }

    /// The worker's last delta: its whole metrics this epoch, from the
    /// processor's own whole run state `cumulative`, whose counters must be
    /// up to date, which replace the metrics the manager holds for it; the
    /// graph rows pending since the last delta, which add to what it holds,
    /// as any delta's do; and the whole users series. A lost delta's
    /// metrics are made good by it, its graph rows are not: the manager
    /// records the loss in its gaps. Nothing sent is kept here, so the
    /// worker's memory does not grow with the deltas it sends.
    pub(crate) fn take_final(
        &mut self,
        cumulative: (&GooseMetrics, &GraphData),
        epoch: u64,
        now: Second,
    ) -> GaggleDelta {
        let mut delta = GaggleDelta::new(self.worker, self.next_seq(), epoch, DeltaKind::Final);
        delta.series_origin = self.series_origin;
        delta.metrics = metrics_part(cumulative.0);
        for (key, aggregate) in &cumulative.0.requests {
            if let Some(request) = delta.metrics.requests.get_mut(key) {
                request.success = aggregate.success_count as i64;
                request.fail = aggregate.fail_count as i64;
            }
        }
        let mut graph = self.take_graph(now);
        let mut everything = (0, Vec::new());
        graph.users = cumulative.1.gaggle_users_since(&mut everything);
        self.users_sent = everything;
        delta.graph = graph;
        delta.now = clock(&delta, now);
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

/// A delta's clock: `now`, or its latest second when it holds a later one.
/// A tick's `now` is read before the processor takes the delta, so metrics
/// it handled since can be stamped a second later; a manager refuses a
/// delta holding a second past its own clock.
fn clock(delta: &GaggleDelta, now: Second) -> Second {
    delta
        .graph
        .seconds()
        .chain(delta.series_origin)
        .fold(now, Second::max)
}

/// Everything in `metrics` as a delta carries it, apart from the success
/// and failure counts, which the atomic counters hold. Every field of every
/// aggregate is named, so a field added to one fails to compile here until
/// it is sent or left out on purpose.
pub(crate) fn metrics_part(metrics: &GooseMetrics) -> MetricsPart {
    let GooseMetrics {
        // The manager hashes the plan itself.
        hash: _,
        // The manager runs the plan, and keeps its history.
        history: _,
        // The manager times the run.
        duration: _,
        // The manager counts users from what it starts.
        maximum_users: _,
        // As `maximum_users`.
        total_users: _,
        requests,
        transactions,
        scenarios,
        errors,
        transaction_errors,
        hosts,
        coordinated_omission_metrics,
        // The manager judges thresholds on the merged metrics.
        thresholds: _,
        // As `thresholds`.
        threshold_abort: _,
        // As `thresholds`.
        rps_window: _,
        // The manager's own display flags.
        final_metrics: _,
        // As `final_metrics`.
        display_status_codes: _,
        // As `final_metrics`.
        display_metrics: _,
    } = metrics;
    let mut part = MetricsPart::default();
    for (key, aggregate) in requests {
        let GooseRequestMetricAggregate {
            path,
            method,
            raw_data,
            coordinated_omission_data,
            status_code_counts,
            status_code_timings,
            // The atomic counters hold them (see `WorkerState::add_counts`).
            success_count: _,
            // As `success_count`.
            fail_count: _,
            // 0 in every aggregate the metrics processor builds.
            load_test_hash: _,
            custom_method,
        } = aggregate;
        let mut request = RequestPart::new(path, *method, custom_method);
        request.raw = Timing::from_data(raw_data);
        request.adjusted = coordinated_omission_data.as_ref().map(Timing::from_data);
        request.status_codes = status_code_counts
            .iter()
            .map(|(&code, &count)| (code, count as u64))
            .collect();
        request.status_timings = status_code_timings
            .iter()
            .map(|(&code, summary)| (code, summary.clone()))
            .collect();
        part.requests.insert(key.clone(), request);
    }
    for transaction in transactions.iter().flatten() {
        let TransactionMetricAggregate {
            scenario_index,
            // The manager's template names every transaction.
            scenario_name: _,
            transaction_index,
            // As `scenario_name`.
            transaction_name: _,
            run_times,
            success_count,
            fail_count,
        } = transaction;
        if run_times.counter == 0 && *success_count == 0 && *fail_count == 0 {
            continue;
        }
        part.transactions.insert(
            (*scenario_index as u64, *transaction_index as u64),
            TransactionPart {
                run_times: Timing::from_data(run_times),
                success: *success_count as u64,
                fail: *fail_count as u64,
            },
        );
    }
    for scenario in scenarios {
        let ScenarioMetricAggregate {
            index,
            // The manager's template names every scenario.
            name: _,
            users,
            run_times,
        } = scenario;
        if users.is_empty() && run_times.counter == 0 {
            continue;
        }
        part.scenarios.insert(
            *index as u64,
            ScenarioPart {
                users: users.iter().map(|&user| user as u64).collect(),
                run_times: Timing::from_data(run_times),
            },
        );
    }
    for (key, error) in errors {
        let GooseErrorMetricAggregate {
            method,
            name,
            error,
            occurrences,
            custom_method,
        } = error;
        part.errors.insert(
            ErrorKey {
                key: key.clone(),
                method: *method,
                name: name.clone(),
                error: error.clone(),
                custom_method: custom_method.clone(),
            },
            *occurrences as u64,
        );
    }
    for (key, error) in transaction_errors {
        let GooseTransactionErrorMetricAggregate {
            scenario_index,
            scenario_name,
            transaction_index,
            transaction_name,
            error,
            occurrences,
        } = error;
        part.transaction_errors.insert(
            TransactionErrorKey {
                key: key.clone(),
                scenario_index: *scenario_index as u64,
                scenario_name: scenario_name.clone(),
                transaction_index: *transaction_index as u64,
                transaction_name: transaction_name.clone(),
                error: error.clone(),
            },
            *occurrences as u64,
        );
    }
    part.hosts = hosts.iter().cloned().collect();
    part.co = coordinated_omission_metrics.as_ref().map(|co| {
        let CoordinatedOmissionMetrics {
            actual_requests,
            synthetic_requests,
            // Worked out again from the merged counts.
            synthetic_percentage: _,
            co_events,
            severity_histogram,
            mitigation_strategy,
            started_secs,
        } = co;
        CoPart {
            strategy: mitigation_strategy.clone(),
            started_secs: *started_secs,
            actual: *actual_requests,
            synthetic: *synthetic_requests,
            severity: severity_histogram
                .iter()
                .map(|(&severity, &count)| (severity_code(severity), count as u64))
                .collect(),
            events: co_events.clone(),
            events_dropped: 0,
        }
    });
    part
}
