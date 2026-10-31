//! The manager side: one running total for each worker, and the metrics and
//! graph data of all of them merged, as one process running the whole load
//! test would have them.

use super::delta::{
    severity_from_code, Bounds, CoPart, DeltaError, DeltaKind, DeltaLimits, ErrorKey, GaggleDelta,
    GraphPart, MetricsPart, Pass, RequestPart, ScenarioPart, Second, Timing, TransactionErrorKey,
    TransactionPart,
};
use crate::graph::{GraphData, GraphSettings};
use crate::metrics::coordinated_omission::CoordinatedOmissionMetrics;
use crate::metrics::{
    GooseErrorMetricAggregate, GooseMetrics, GooseRequestMetricAggregate,
    GooseTransactionErrorMetricAggregate, ScenarioMetricAggregate, TransactionMetricAggregate,
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
    /// The `max_second` it was built with, which
    /// [`Self::set_max_second`] never passes.
    cap_second: Second,
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
            cap_second: limits.max_second,
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
            let got_final = delta.kind == DeltaKind::Final;
            total.add_pass(&delta, Pass::Apply, Bounds::WORKER)?;
            if delta.first_seq != 1 {
                self.gaps.push(SeqGap {
                    worker: from,
                    expected: 1,
                    received: delta.first_seq,
                });
            }
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

    /// Refuse, from now on, a delta carrying a second past `second`: the
    /// manager's own series clock, plus slack, at most the cap the
    /// accumulator was built with.
    // Called by the manager each second in Gaggle step 2; tested here.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn set_max_second(&mut self, second: Second) {
        self.limits.max_second = second.min(self.cap_second);
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
        // Each worker's change of level at each second it sent one, added
        // up, then walked once: a worker counts 0 before its first level.
        let mut changes: BTreeMap<Second, i128> = BTreeMap::new();
        for worker in self.workers.values() {
            let mut level = 0;
            for (&second, &users) in &worker.total.graph.users {
                *changes.entry(second).or_default() += i128::from(users) - level;
                level = i128::from(users);
            }
        }
        let first = changes.keys().next().copied().unwrap_or(0);
        let mut changes = changes.into_iter().peekable();
        let mut users: i128 = 0;
        (first..=last)
            .map(|second| {
                while let Some((_, change)) = changes.next_if(|&(at, _)| at <= second) {
                    users += change;
                }
                (second, users as u64)
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

/// `template` with every figure of `part` added, as `GooseMetrics`. Every
/// field of every aggregate is named, so a field added to one fails to
/// compile here until it is merged or left out on purpose.
fn build_metrics(template: &GooseMetrics, part: &MetricsPart) -> GooseMetrics {
    let MetricsPart {
        requests: request_parts,
        transactions: transaction_parts,
        scenarios: scenario_parts,
        errors: error_parts,
        transaction_errors: transaction_error_parts,
        hosts: host_parts,
        co,
    } = part;
    let mut metrics = template.clone();
    let GooseMetrics {
        // The manager's own, from its template.
        hash: _,
        // As `hash`.
        history: _,
        // As `hash`.
        duration: _,
        // As `hash`.
        maximum_users: _,
        // As `hash`.
        total_users: _,
        requests,
        transactions,
        scenarios,
        errors,
        transaction_errors,
        hosts,
        coordinated_omission_metrics,
        // Judged on the merged metrics afterwards.
        thresholds: _,
        // As `thresholds`.
        threshold_abort: _,
        // As `thresholds`.
        rps_window: _,
        // The manager's own, from its template.
        final_metrics: _,
        // As `final_metrics`.
        display_status_codes: _,
        // As `final_metrics`.
        display_metrics: _,
    } = &mut metrics;
    for (key, request) in request_parts {
        let RequestPart {
            path,
            method,
            custom_method,
            raw,
            adjusted,
            status_codes,
            status_timings,
            success,
            fail,
        } = request;
        requests.insert(
            key.clone(),
            GooseRequestMetricAggregate {
                path: path.clone(),
                method: *method,
                raw_data: raw.to_data(),
                coordinated_omission_data: adjusted.as_ref().map(Timing::to_data),
                status_code_counts: status_codes
                    .iter()
                    .map(|(&code, &count)| (code, count as usize))
                    .collect(),
                status_code_timings: status_timings
                    .iter()
                    .map(|(&code, summary)| (code, summary.clone()))
                    .collect(),
                success_count: (*success).max(0) as usize,
                fail_count: (*fail).max(0) as usize,
                load_test_hash: 0,
                custom_method: custom_method.clone(),
            },
        );
    }
    for (&(scenario_index, transaction_index), transaction) in transaction_parts {
        let Some(aggregate) = transactions
            .get_mut(scenario_index as usize)
            .and_then(|transactions| transactions.get_mut(transaction_index as usize))
        else {
            continue;
        };
        let TransactionMetricAggregate {
            // The template's.
            scenario_index: _,
            // As `scenario_index`.
            scenario_name: _,
            // As `scenario_index`.
            transaction_index: _,
            // As `scenario_index`.
            transaction_name: _,
            run_times,
            success_count,
            fail_count,
        } = aggregate;
        let TransactionPart {
            run_times: times,
            success,
            fail,
        } = transaction;
        run_times.merge(&times.to_data());
        *success_count += *success as usize;
        *fail_count += *fail as usize;
    }
    for (&index, scenario) in scenario_parts {
        let Some(aggregate) = scenarios.get_mut(index as usize) else {
            continue;
        };
        let ScenarioMetricAggregate {
            // The template's.
            index: _,
            // As `index`.
            name: _,
            users,
            run_times,
        } = aggregate;
        let ScenarioPart {
            users: ordinals,
            run_times: times,
        } = scenario;
        users.extend(ordinals.iter().map(|&user| user as usize));
        run_times.merge(&times.to_data());
    }
    for (error, &occurrences) in error_parts {
        let ErrorKey {
            key,
            method,
            name,
            error,
            custom_method,
        } = error;
        errors.insert(
            key.clone(),
            GooseErrorMetricAggregate {
                method: *method,
                name: name.clone(),
                error: error.clone(),
                occurrences: occurrences as usize,
                custom_method: custom_method.clone(),
            },
        );
    }
    for (error, &occurrences) in transaction_error_parts {
        let TransactionErrorKey {
            key,
            scenario_index,
            scenario_name,
            transaction_index,
            transaction_name,
            error,
        } = error;
        transaction_errors.insert(
            key.clone(),
            GooseTransactionErrorMetricAggregate {
                scenario_index: *scenario_index as usize,
                scenario_name: scenario_name.clone(),
                transaction_index: *transaction_index as usize,
                transaction_name: transaction_name.clone(),
                error: error.clone(),
                occurrences: occurrences as usize,
            },
        );
    }
    hosts.extend(host_parts.iter().cloned());
    if let Some(co) = co.as_ref() {
        let CoPart {
            strategy,
            started_secs,
            actual,
            synthetic,
            severity,
            events,
            // Counted by the shadow merge, which checks what they leave out.
            events_dropped: _,
        } = co;
        let mut events = events.clone();
        // Oldest first; a stable sort keeps each worker's own order.
        events.sort_by_key(|event| event.timestamp_secs);
        let mut merged = CoordinatedOmissionMetrics {
            actual_requests: *actual,
            synthetic_requests: *synthetic,
            synthetic_percentage: 0.0,
            co_events: events,
            severity_histogram: severity
                .iter()
                .map(|(&severity, &count)| (severity_from_code(severity), count as usize))
                .collect(),
            mitigation_strategy: strategy.clone(),
            started_secs: *started_secs,
        };
        merged.update_synthetic_percentage();
        *coordinated_omission_metrics = Some(merged);
    }
    metrics
}
