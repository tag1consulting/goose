//! The manager side: one running total for each worker, and the metrics and
//! graph data of all of them merged, as one process running the whole load
//! test would have them.

use super::delta::{
    severity_from_code, Bounds, DeltaError, DeltaKind, DeltaLimits, GaggleDelta, GraphPart,
    MetricsPart, Pass, Second,
};
use crate::graph::{GraphData, GraphSettings};
use crate::metrics::coordinated_omission::CoordinatedOmissionMetrics;
use crate::metrics::{
    GooseErrorMetricAggregate, GooseMetrics, GooseRequestMetricAggregate,
    GooseTransactionErrorMetricAggregate,
};
use std::collections::BTreeMap;

/// A delta that did not follow the one before it from the same worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SeqGap {
    pub(crate) worker: u32,
    /// The sequence number expected.
    pub(crate) expected: u64,
    /// The one received: higher when deltas went missing, lower or equal
    /// for a repeat, which is ignored.
    pub(crate) received: u64,
}

/// What the manager holds for one worker.
struct WorkerTotal {
    /// Everything received this epoch, as one delta, and the users levels
    /// over the whole run, which a metrics reset keeps.
    total: GaggleDelta,
    last_seq: u64,
    /// The worker has sent everything: a final delta arrived, or it is
    /// [`GaggleAccumulator::done`].
    finished: bool,
    /// A final delta arrived: no delta may follow it.
    got_final: bool,
    /// The worker is gone without a final delta.
    lost: bool,
}

/// Every worker's metrics, merged on demand.
pub(crate) struct GaggleAccumulator {
    /// The manager's own empty metrics for the load test: every scenario and
    /// transaction, the hosts, and the figures no worker sends.
    template: GooseMetrics,
    settings: GraphSettings,
    /// What a delta may carry.
    limits: DeltaLimits,
    workers: BTreeMap<u32, WorkerTotal>,
    gaps: Vec<SeqGap>,
}

impl GaggleAccumulator {
    /// Merge into `template`, the empty metrics a process running the whole
    /// load test starts with, building graph data with `settings`, and
    /// refusing a delta past `limits`.
    pub(crate) fn new(
        template: GooseMetrics,
        settings: GraphSettings,
        limits: DeltaLimits,
    ) -> Self {
        GaggleAccumulator {
            template,
            settings,
            limits,
            workers: BTreeMap::new(),
            gaps: Vec::new(),
        }
    }

    /// Add a delta from worker `from`, the worker the caller knows sent it,
    /// in the order the worker sent them. A final delta's metrics replace
    /// those held for the worker, and its graph rows, those pending since
    /// the worker's last delta, add to the rows held, as any delta's do: a
    /// lost delta's metrics are made good by the final, its graph rows are
    /// not, and the loss stays recorded in [`Self::gaps`]. A delta from an
    /// epoch older than the worker's is discarded, and a newer epoch starts
    /// the worker's metrics and rate series again. A gap or repeat in the
    /// sequence is recorded in [`Self::gaps`]; a repeat is ignored.
    ///
    /// The delta is untrusted: one that fails [`GaggleDelta::validate`],
    /// names another worker, follows the worker's final delta, covers
    /// sequence numbers partly applied already, or would take the worker
    /// past [`DeltaLimits::max_names`] or [`Bounds::WORKER`] is refused,
    /// and changes nothing.
    pub(crate) fn apply(&mut self, from: u32, delta: GaggleDelta) -> Result<(), DeltaError> {
        if delta.worker != from {
            return Err(DeltaError::WrongWorker {
                from,
                named: delta.worker,
            });
        }
        delta.validate(&self.limits)?;
        let Some(worker) = self.workers.get_mut(&from) else {
            // The first delta from this worker: adding a valid delta to
            // nothing cannot fail.
            let mut total = GaggleDelta::new(from, 0, delta.epoch, DeltaKind::Delta);
            if delta.first_seq != 1 {
                self.gaps.push(SeqGap {
                    worker: from,
                    expected: 1,
                    received: delta.first_seq,
                });
            }
            let got_final = delta.kind == DeltaKind::Final;
            total.add_pass(&delta, Pass::Apply, Bounds::WORKER)?;
            self.workers.insert(
                from,
                WorkerTotal {
                    total,
                    last_seq: delta.seq,
                    finished: got_final,
                    got_final,
                    lost: false,
                },
            );
            return Ok(());
        };
        if worker.got_final {
            return Err(DeltaError::AfterFinal { seq: delta.seq });
        }
        if delta.epoch < worker.total.epoch {
            return Ok(());
        }
        if delta.first_seq <= worker.last_seq && worker.last_seq < delta.seq {
            return Err(DeltaError::Overlap {
                first_seq: delta.first_seq,
                seq: delta.seq,
                held: worker.last_seq,
            });
        }
        // `last_seq` is below `u64::MAX`, which no valid delta has.
        let expected = worker.last_seq + 1;
        if delta.seq < expected {
            self.gaps.push(SeqGap {
                worker: from,
                expected,
                received: delta.first_seq,
            });
            return Ok(());
        }
        // Check before changing anything: what this delta starts again
        // from nothing cannot pass the bounds, as it is valid.
        let new_epoch = delta.epoch > worker.total.epoch;
        let replaces_metrics = new_epoch || delta.kind == DeltaKind::Final;
        let names = worker
            .total
            .names_with(&delta, !replaces_metrics, !new_epoch);
        if names > self.limits.max_names {
            return Err(DeltaError::TooManyNames {
                names,
                limit: self.limits.max_names,
            });
        }
        if !replaces_metrics {
            worker
                .total
                .metrics
                .add_pass(&delta.metrics, Pass::Check, Bounds::WORKER)?;
        }
        if !new_epoch {
            worker
                .total
                .graph
                .add_rates(&delta.graph, Pass::Check, Bounds::WORKER)?;
        }
        if delta.first_seq != expected {
            self.gaps.push(SeqGap {
                worker: from,
                expected,
                received: delta.first_seq,
            });
        }
        worker.last_seq = delta.seq;
        if new_epoch {
            worker.total.epoch = delta.epoch;
            worker.total.metrics = MetricsPart::default();
            worker.total.graph.clear_rates();
        }
        if delta.kind == DeltaKind::Final {
            worker.finished = true;
            worker.got_final = true;
            worker.total.metrics = MetricsPart::default();
        }
        worker.total.add_pass(&delta, Pass::Apply, Bounds::WORKER)
    }

    /// The worker is gone without a final delta: what it sent stays merged,
    /// and its seconds no longer hold back which seconds are complete. The
    /// loss of a worker that sent anything is recorded in [`Self::gaps`] as
    /// a gap from its next sequence number, with nothing (0) received.
    // Called by the manager in Gaggle step 2; tested here.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn lose(&mut self, worker: u32) {
        if let Some(total) = self.workers.get_mut(&worker) {
            if !total.lost && !total.finished {
                self.gaps.push(SeqGap {
                    worker,
                    expected: total.last_seq + 1,
                    received: 0,
                });
            }
            total.lost = true;
        }
    }

    /// The worker has sent its last delta and will send no final one: its
    /// seconds no longer hold back which seconds are complete.
    pub(crate) fn done(&mut self, worker: u32) {
        if let Some(worker) = self.workers.get_mut(&worker) {
            worker.finished = true;
        }
    }

    /// Every gap and repeat seen in a worker's sequence.
    pub(crate) fn gaps(&self) -> &[SeqGap] {
        &self.gaps
    }

    /// The first second not yet complete: one that a worker still sending
    /// could still add to. `None` once every worker finished or was lost.
    pub(crate) fn complete_before(&self) -> Option<Second> {
        self.workers
            .values()
            .filter(|worker| !worker.finished && !worker.lost)
            .map(|worker| worker.total.now.saturating_sub(1))
            .min()
    }

    /// The newest metrics epoch any worker reached.
    fn epoch(&self) -> Option<u64> {
        self.workers.values().map(|w| w.total.epoch).max()
    }

    /// The workers [`Self::merged`] leaves out, because their metrics epoch
    /// is older than the newest any worker reached. Each worker counts its
    /// own metrics resets, so one that missed a reset is left out rather
    /// than mixed into another epoch's figures; in Gaggle step 2 the
    /// manager owns the epochs and no worker can fall behind unseen. Also
    /// any whose figures would take a sum past [`Bounds::MERGED`], which
    /// [`Bounds::WORKER`] and [`super::delta::MAX_WORKERS`] rule out.
    pub(crate) fn excluded(&self) -> Vec<u32> {
        self.merge_parts().3
    }

    /// The metrics and graph data of every worker merged: the metrics of
    /// the newest epoch any worker reached, and the graphs' complete
    /// seconds only. Every derived figure is worked out again from the
    /// merged counts. A worker left out is named by [`Self::excluded`].
    pub(crate) fn merged(&self) -> (GooseMetrics, GraphData) {
        let (metrics, mut graph, series_origin, _) = self.merge_parts();
        let complete = self.complete_before();
        graph.users = self.users_levels();
        (
            build_metrics(&self.template, &metrics),
            GraphData::gaggle_build(&graph, self.settings, series_origin, complete),
        )
    }

    /// Every worker's figures of the newest epoch added up, the series
    /// origin of the merge, and the workers left out.
    fn merge_parts(&self) -> (MetricsPart, GraphPart, Option<Second>, Vec<u32>) {
        let epoch = self.epoch();
        let mut metrics = MetricsPart::default();
        let mut graph = GraphPart::default();
        let mut series_origin = None;
        let mut excluded = Vec::new();
        for (&id, worker) in &self.workers {
            let (part, rows) = (&worker.total.metrics, &worker.total.graph);
            let fits = Some(worker.total.epoch) == epoch
                && metrics.add_pass(part, Pass::Check, Bounds::MERGED).is_ok()
                && graph.add_rates(rows, Pass::Check, Bounds::MERGED).is_ok();
            if !fits {
                excluded.push(id);
                continue;
            }
            // Checked above, so these add every figure.
            let added = metrics
                .add_pass(part, Pass::Apply, Bounds::MERGED)
                .and_then(|()| graph.add_rates(rows, Pass::Apply, Bounds::MERGED));
            debug_assert!(added.is_ok());
            series_origin = series_origin.max(worker.total.series_origin);
        }
        (metrics, graph, series_origin, excluded)
    }

    /// The users running at each second: every worker's level at that
    /// second added up, a worker past its last level holding it.
    fn users_levels(&self) -> BTreeMap<Second, u64> {
        let last = self
            .workers
            .values()
            .filter_map(|worker| worker.total.graph.users.keys().next_back().copied())
            .max();
        let Some(last) = last else {
            return BTreeMap::new();
        };
        let first = self
            .workers
            .values()
            .filter_map(|worker| worker.total.graph.users.keys().next().copied())
            .min()
            .unwrap_or(0);
        (first..=last)
            .map(|second| {
                let users = self
                    .workers
                    .values()
                    .filter_map(|worker| worker.total.graph.users.range(..=second).next_back())
                    .map(|(_, &users)| users)
                    .sum();
                (second, users)
            })
            .collect()
    }
}

/// Two consecutive deltas from one worker as one: applying it is applying
/// `a` then `b`. A newer epoch in `b` starts again from `b`, keeping `a`'s
/// users levels, and a final `b`'s metrics replace `a`'s while its graph
/// rows add to `a`'s. Two exceptions: coordinated omission events stay
/// capped at [`super::delta::MAX_CO_EVENTS_PER_DELTA`] in a delta, the
/// oldest kept, where applying both keeps up to the cap from each; and a
/// final `a` followed by a delta of a newer epoch does not mark the worker
/// finished. Refused, as [`GaggleDelta::add`], past [`Bounds::WORKER`].
// Bounds a worker's send queue in Gaggle step 2; tested here.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn coalesce(a: &GaggleDelta, b: &GaggleDelta) -> Result<GaggleDelta, DeltaError> {
    if b.epoch < a.epoch {
        return Ok(a.clone());
    }
    if b.epoch > a.epoch {
        let mut users = a.graph.users.clone();
        let mut joined = b.clone();
        users.append(&mut joined.graph.users);
        joined.graph.users = users;
        joined.first_seq = a.first_seq;
        return Ok(joined);
    }
    let mut joined = a.clone();
    if b.kind == DeltaKind::Final {
        joined.kind = DeltaKind::Final;
        joined.metrics = MetricsPart::default();
    }
    joined.add(b)?;
    if joined.kind == DeltaKind::Delta {
        if let Some(co) = joined.metrics.co.as_mut() {
            co.cap_events();
        }
    }
    Ok(joined)
}

/// `template` with every figure of `part` added, as `GooseMetrics`.
fn build_metrics(template: &GooseMetrics, part: &MetricsPart) -> GooseMetrics {
    let mut metrics = template.clone();
    for (key, request) in &part.requests {
        metrics.requests.insert(
            key.clone(),
            GooseRequestMetricAggregate {
                path: request.path.clone(),
                method: request.method,
                raw_data: request.raw.to_data(),
                coordinated_omission_data: request.adjusted.as_ref().map(|t| t.to_data()),
                status_code_counts: request
                    .status_codes
                    .iter()
                    .map(|(&code, &count)| (code, count as usize))
                    .collect(),
                status_code_timings: request
                    .status_timings
                    .iter()
                    .map(|(&code, summary)| (code, summary.clone()))
                    .collect(),
                success_count: request.success.max(0) as usize,
                fail_count: request.fail.max(0) as usize,
                load_test_hash: 0,
                custom_method: request.custom_method.clone(),
            },
        );
    }
    for (&(scenario_index, transaction_index), transaction) in &part.transactions {
        if let Some(aggregate) = metrics
            .transactions
            .get_mut(scenario_index as usize)
            .and_then(|transactions| transactions.get_mut(transaction_index as usize))
        {
            aggregate.run_times.merge(&transaction.run_times.to_data());
            aggregate.success_count += transaction.success as usize;
            aggregate.fail_count += transaction.fail as usize;
        }
    }
    for (&index, scenario) in &part.scenarios {
        if let Some(aggregate) = metrics.scenarios.get_mut(index as usize) {
            aggregate
                .users
                .extend(scenario.users.iter().map(|&user| user as usize));
            aggregate.run_times.merge(&scenario.run_times.to_data());
        }
    }
    for (error, &occurrences) in &part.errors {
        metrics.errors.insert(
            error.key.clone(),
            GooseErrorMetricAggregate {
                method: error.method,
                name: error.name.clone(),
                error: error.error.clone(),
                occurrences: occurrences as usize,
                custom_method: error.custom_method.clone(),
            },
        );
    }
    for (error, &occurrences) in &part.transaction_errors {
        metrics.transaction_errors.insert(
            error.key.clone(),
            GooseTransactionErrorMetricAggregate {
                scenario_index: error.scenario_index as usize,
                scenario_name: error.scenario_name.clone(),
                transaction_index: error.transaction_index as usize,
                transaction_name: error.transaction_name.clone(),
                error: error.error.clone(),
                occurrences: occurrences as usize,
            },
        );
    }
    metrics.hosts.extend(part.hosts.iter().cloned());
    if let Some(co) = part.co.as_ref() {
        let mut events = co.events.clone();
        // Oldest first; a stable sort keeps each worker's own order.
        events.sort_by_key(|event| event.timestamp_secs);
        let mut merged = CoordinatedOmissionMetrics {
            actual_requests: co.actual,
            synthetic_requests: co.synthetic,
            synthetic_percentage: 0.0,
            co_events: events,
            severity_histogram: co
                .severity
                .iter()
                .map(|(&severity, &count)| (severity_from_code(severity), count as usize))
                .collect(),
            mitigation_strategy: co.strategy.clone(),
            started_secs: co.started_secs,
        };
        merged.update_synthetic_percentage();
        metrics.coordinated_omission_metrics = Some(merged);
    }
    metrics
}
