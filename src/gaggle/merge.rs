//! The manager side: one running total for each worker, and the metrics and
//! graph data of all of them merged, as one process running the whole load
//! test would have them.

use super::delta::{
    severity_from_code, Bounds, CoPart, DeltaError, DeltaKind, DeltaLimits, ErrorKey, GaggleDelta,
    GraphPart, MetricsPart, Pass, RequestPart, ScenarioPart, Second, Timing, TransactionErrorKey,
    TransactionPart,
};
use crate::graph::{GraphData, GraphSettings};
use crate::metrics::coordinated_omission::{CoEvent, CoordinatedOmissionMetrics};
use crate::metrics::{
    GooseErrorMetricAggregate, GooseMetrics, GooseRequestMetricAggregate,
    GooseTransactionErrorMetricAggregate, ScenarioMetricAggregate, TransactionMetricAggregate,
};
use std::cell::RefCell;
use std::collections::BTreeMap;

pub(crate) use super::health::MergeHealth;
#[cfg(test)]
pub(crate) use super::health::WorkerMerge;

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
    /// The deltas from it that never arrived, as [`GaggleAccumulator::missing`]
    /// counts them.
    missing: u64,
    /// Its delta's second when a row of it for a second already built last
    /// built the graph data again from nothing.
    late_rebuilt: Option<Second>,
    /// Repeats of deltas already taken, ignored (tests: a repeat changes
    /// nothing in the results, so nothing reports it).
    #[cfg(test)]
    repeats: u64,
}

/// Every merged worker's figures added up, kept as their deltas arrive, so a
/// refresh never adds each worker's whole history again. Built again from
/// the per worker totals when a delta cannot simply be added (a reset, a
/// final delta, a worker's metrics restarting on a new epoch, or a worker
/// newly merged or left out) and when a worker is lost or done, so the
/// figures are added in worker order, as a build from nothing adds them.
/// While deltas are only added, the live figures match a build from nothing
/// to within `f64` rounding: the average sums are added in arrival order,
/// not worker order. The final figures, built again once every worker is
/// done or lost, match it exactly.
#[derive(Default)]
struct Merged {
    /// The figures, coordinated omission events left out: those are taken
    /// from each worker's total, in worker order, when the merge is built.
    metrics: MetricsPart,
    /// The rate rows; the users levels are worked out when it is built.
    graph: GraphPart,
    /// Each worker left out, with why.
    excluded: Vec<(u32, String)>,
    /// Whether it adds up the per worker totals: false until it is built
    /// again.
    current: bool,
    /// What the graph data last built holds, so the next build adds only
    /// the seconds completed since to it.
    built: Option<GraphCache>,
    /// Rows for seconds `built` holds were merged and not yet built: the
    /// next build from nothing holds them.
    late: bool,
    /// The key of each coordinated omission event the last build gave its
    /// caller, in its order; `None` until a build gathered them.
    event_keys: Option<Vec<EventKey>>,
    /// The merged workers' events since, with their keys.
    new_events: Vec<(EventKey, CoEvent)>,
    /// The events the last build gave, handed back by its caller
    /// ([`GaggleAccumulator::hand_back_events`]).
    handed_back: Option<Vec<CoEvent>>,
}

/// Where a coordinated omission event goes in the merged list: by its
/// timestamp, then its worker, then its place in the worker's own events,
/// the order a stable sort by timestamp of every worker's events, in worker
/// order, gives.
type EventKey = (u64, u32, usize);

/// Put `new` events in their places among `events`, held in the order of
/// `keys`, in one pass: `new` sorted by key, every held event from the first
/// new one's place on taken out once and merged back with them, so the work
/// is the held list's tail and `new`, never their product. New events that
/// all follow the held ones, as each second's do, move none.
fn place_events(
    keys: &mut Vec<EventKey>,
    events: &mut Vec<CoEvent>,
    mut new: Vec<(EventKey, CoEvent)>,
) {
    // Each key is unique: a worker numbers its own events.
    new.sort_unstable_by_key(|(key, _)| *key);
    let Some((first, _)) = new.first() else {
        return;
    };
    let from = keys.partition_point(|held| held < first);
    let tail_keys = keys.split_off(from);
    let tail_events = events.split_off(from);
    #[cfg(test)]
    EVENTS_MOVED.with(|moved| moved.set(moved.get() + 2 * tail_keys.len()));
    keys.reserve(tail_keys.len() + new.len());
    events.reserve(tail_keys.len() + new.len());
    let mut tail = tail_keys.into_iter().zip(tail_events).peekable();
    for (key, event) in new {
        while let Some((held, held_event)) = tail.next_if(|(held, _)| *held < key) {
            keys.push(held);
            events.push(held_event);
        }
        keys.push(key);
        events.push(event);
    }
    for (held, held_event) in tail {
        keys.push(held);
        events.push(held_event);
    }
}

/// What graph data built from the merged figures holds: every second before
/// `before`. The graph data itself is the caller's, handed back to each
/// build ([`GaggleAccumulator::merged_and_excluded`]).
struct GraphCache {
    before: Second,
    /// The users series the build left, which the caller's graph data may
    /// since have added to.
    users: Option<crate::graph::MergedUsers>,
    series_origin: Option<Second>,
    columns: bool,
    /// The workers it left out.
    excluded: Vec<u32>,
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
    /// Every gap and repeat in the workers' sequences, kept only for the
    /// shadow merge, which reports each ([`Self::keeping_gaps`]): a manager
    /// counts them per worker instead, so a worker cannot grow its memory.
    gaps: Option<Vec<SeqGap>>,
    /// Gaps past the [`MAX_GAPS_KEPT`] kept.
    gaps_dropped: u64,
    /// The manager's own metrics epoch, the one merged: after a reset every
    /// worker is left out until it reports the new epoch, rather than show
    /// the old epoch's figures as current, and a delta from a later epoch is
    /// refused.
    epoch: u64,
    /// The merged figures, kept as deltas arrive and built again on demand.
    merged: RefCell<Merged>,
    /// How many seconds before its latest delta's second a worker's graph
    /// rows may still arrive: the workers' own lag, their request timeout
    /// and a minute. A second is complete only once it is older.
    lag: Second,
    /// The graph data [`Self::merged`] built last (tests).
    #[cfg(test)]
    test_graph: RefCell<Option<GraphData>>,
}

std::thread_local! {
    /// The graph data built from nothing on this thread (tests).
    #[cfg(test)]
    pub(crate) static FRESH_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// The coordinated omission events builds on this thread took from the
    /// workers' totals, or added since (tests).
    #[cfg(test)]
    pub(crate) static EVENTS_GATHERED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// The held coordinated omission events builds on this thread moved to
    /// put new ones in their places (tests).
    #[cfg(test)]
    pub(crate) static EVENTS_MOVED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The most gaps an accumulator keeping them holds; past it they are
/// counted.
const MAX_GAPS_KEPT: usize = 1_000;

/// Keep `gap` in `gaps`, when gaps are kept, or count it past
/// [`MAX_GAPS_KEPT`].
fn keep_gap(gaps: &mut Option<Vec<SeqGap>>, dropped: &mut u64, gap: SeqGap) {
    if let Some(gaps) = gaps.as_mut() {
        if gaps.len() < MAX_GAPS_KEPT {
            gaps.push(gap);
        } else {
            *dropped += 1;
        }
    }
}

/// What [`GaggleAccumulator::apply`] did with a delta it took.
enum Taken {
    /// Nothing: an older epoch, or a repeat.
    Ignored,
    /// Its figures were added to the worker's, as they are.
    Added,
    /// It started the worker's figures again: its first of a new epoch, or
    /// its final delta, whose metrics replace those held.
    Restarted,
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
            gaps: None,
            gaps_dropped: 0,
            epoch: 0,
            merged: RefCell::new(Merged {
                current: true,
                ..Merged::default()
            }),
            lag: 0,
            #[cfg(test)]
            test_graph: RefCell::new(None),
        }
    }

    /// Count a second complete only once it is `lag` seconds older than
    /// every worker's latest: the workers' own lag (see
    /// [`Self::complete_before`]).
    pub(crate) fn set_lag(&mut self, lag: Second) {
        self.lag = lag;
    }

    /// Add a delta from worker `from`, the worker the caller knows sent it,
    /// in the order the worker sent them. A final delta's metrics replace
    /// those held for the worker, and its graph rows, those pending since
    /// the worker's last delta, add to the rows held, as any delta's do: a
    /// lost delta's metrics are made good by the final, its graph rows are
    /// not, and the loss stays counted (see [`Self::missing`]). A delta from
    /// an epoch older than the worker's is discarded, and a newer epoch
    /// starts the worker's metrics and rate series again. A gap or repeat in
    /// the sequence is counted for the worker, and recorded in
    /// [`Self::gaps`] when gaps are kept ([`Self::keeping_gaps`]); a repeat
    /// is ignored.
    ///
    /// The delta is untrusted: one that fails [`GaggleDelta::validate`],
    /// names another worker, is of an epoch past the manager's own (see
    /// [`Self::set_epoch`]), follows the worker's final delta, covers
    /// sequence numbers partly applied already, or would take the worker
    /// past [`DeltaLimits::max_names`] or [`Bounds::WORKER`] is refused,
    /// and changes nothing.
    ///
    /// With no graph data to add to, as here, a delta's rows for seconds the
    /// last build left provisional have the next build start from nothing:
    /// a manager's processor, which holds the graph data, takes deltas with
    /// [`Self::apply_live`] instead.
    pub(crate) fn apply(&mut self, from: u32, delta: GaggleDelta) -> Result<(), DeltaError> {
        self.apply_into(from, delta, None)
    }

    /// [`Self::apply`], adding the delta's rows for every second from the
    /// last build's complete point on to `graph`, the graph data that build
    /// returned, which the caller holds between builds: in place, by
    /// addition, as the delta arrives, so each provisional second is
    /// updated, never built again from nothing, and no build walks every
    /// worker's rows to show them. The work is the delta's own rows, at most
    /// a few seconds of one worker's; the provisional seconds the graph data
    /// then holds are the lag ([`Self::set_lag`]) and a second or two
    /// while every worker keeps sending; a worker that stalls without being
    /// lost holds the complete point back, and the span grows until it
    /// sends again or is lost, each delta still costing only its own rows. Rows for a second before the complete point still build the
    /// graph data again from nothing, at most once a lag window for each
    /// worker.
    pub(crate) fn apply_live(
        &mut self,
        from: u32,
        delta: GaggleDelta,
        graph: &mut GraphData,
    ) -> Result<(), DeltaError> {
        self.apply_into(from, delta, Some(graph))
    }

    /// [`Self::apply`] and [`Self::apply_live`].
    fn apply_into(
        &mut self,
        from: u32,
        delta: GaggleDelta,
        graph: Option<&mut GraphData>,
    ) -> Result<(), DeltaError> {
        let run_epoch = self.epoch;
        let settings = self.settings;
        let known = self.workers.contains_key(&from);
        let taken = self.take(from, &delta)?;
        let merged = self.merged.get_mut();
        // Rows or users levels for a second already built: build again from
        // nothing, at most once a lag window for each worker, as a buggy or
        // hostile worker could ask for it every second. The rows merge at
        // once either way; a build that waits leaves them to the next from
        // nothing, which a forced read or the run's end makes
        // ([`Self::settle`]). A worker's first delta, its final one or a new
        // epoch builds every figure again anyway, as an honest final holds
        // rows for seconds long built.
        if known && matches!(taken, Taken::Added) {
            let earliest = delta.graph.seconds().min();
            if let (Some(built), Some(earliest)) = (merged.built.as_ref(), earliest) {
                if earliest < built.before {
                    let lag = self.lag;
                    if let Some(worker) = self.workers.get_mut(&from) {
                        let due = worker
                            .late_rebuilt
                            .is_none_or(|at| delta.now >= at.saturating_add(lag));
                        if due {
                            warn!(
                                "gaggle: worker {from} sent rows for second {earliest}, which the graph data already holds: it is built again, and at most once in {lag} seconds for this worker"
                            );
                            worker.late_rebuilt = Some(delta.now);
                            merged.built = None;
                        } else {
                            merged.late = true;
                        }
                    }
                }
            }
        }
        match taken {
            Taken::Ignored => {}
            // A worker's first delta: whether it is merged, and where, is
            // worked out with every other worker's.
            _ if !known => merged.current = false,
            Taken::Restarted => merged.current = false,
            Taken::Added if delta.epoch != run_epoch => {}
            Taken::Added if !merged.current => {}
            Taken::Added => {
                let excluded = merged.excluded.iter().any(|&(worker, _)| worker == from);
                // Its events are the last in the worker's total.
                let held = self.workers.get(&from).map_or(0, |worker| {
                    worker
                        .total
                        .metrics
                        .co
                        .as_ref()
                        .map_or(0, |co| co.events.len())
                });
                let new = delta.metrics.co.as_ref().map_or(0, |co| co.events.len());
                if excluded {
                    return Ok(());
                }
                if !merged.follow(&delta, from, held.saturating_sub(new)) {
                    merged.current = false;
                    return Ok(());
                }
                // Followed: its rows for the provisional seconds go into the
                // graph data the last build returned, if it still holds.
                if let Some(built) = merged.built.as_ref() {
                    match graph {
                        Some(graph) => {
                            graph.gaggle_extend(
                                &delta.graph,
                                &BTreeMap::new(),
                                settings,
                                built.before,
                                None,
                            );
                            if settings.records {
                                graph.gaggle_take_back_failures(&delta.graph, built.before);
                            }
                        }
                        None => merged.built = None,
                    }
                }
            }
        }
        Ok(())
    }

    /// [`Self::apply`] to worker `from`'s own total: what it did, or why the
    /// delta was refused.
    fn take(&mut self, from: u32, delta: &GaggleDelta) -> Result<Taken, DeltaError> {
        if delta.worker != from {
            return Err(DeltaError::WrongWorker {
                from,
                named: delta.worker,
            });
        }
        delta.validate(&self.limits)?;
        let ahead = (delta.epoch > self.epoch).then_some(DeltaError::EpochAhead {
            epoch: delta.epoch,
            run: self.epoch,
        });
        let Some(worker) = self.workers.get_mut(&from) else {
            if let Some(ahead) = ahead {
                return Err(ahead);
            }
            // The first delta from this worker: adding a valid delta to
            // nothing cannot fail.
            let mut total = GaggleDelta::new(from, 0, delta.epoch, DeltaKind::Delta);
            let got_final = delta.kind == DeltaKind::Final;
            total.add_pass(delta, Pass::Apply, Bounds::WORKER)?;
            if delta.first_seq != 1 {
                keep_gap(
                    &mut self.gaps,
                    &mut self.gaps_dropped,
                    SeqGap {
                        worker: from,
                        expected: 1,
                        received: delta.first_seq,
                    },
                );
            }
            let missing = delta.first_seq.saturating_sub(1);
            self.workers.insert(
                from,
                WorkerTotal {
                    total,
                    last_seq: delta.seq,
                    finished: got_final,
                    got_final,
                    lost: false,
                    missing,
                    late_rebuilt: None,
                    #[cfg(test)]
                    repeats: 0,
                },
            );
            return Ok(Taken::Added);
        };
        if worker.got_final {
            return Err(DeltaError::AfterFinal { seq: delta.seq });
        }
        if let Some(ahead) = ahead {
            return Err(ahead);
        }
        if delta.epoch < worker.total.epoch {
            return Ok(Taken::Ignored);
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
            #[cfg(test)]
            {
                worker.repeats += 1;
            }
            keep_gap(
                &mut self.gaps,
                &mut self.gaps_dropped,
                SeqGap {
                    worker: from,
                    expected,
                    received: delta.first_seq,
                },
            );
            return Ok(Taken::Ignored);
        }
        // Check before changing anything: what this delta starts again
        // from nothing cannot pass the bounds, as it is valid.
        let new_epoch = delta.epoch > worker.total.epoch;
        let replaces_metrics = new_epoch || delta.kind == DeltaKind::Final;
        let names = worker
            .total
            .names_with(delta, !replaces_metrics, !new_epoch);
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
            keep_gap(
                &mut self.gaps,
                &mut self.gaps_dropped,
                SeqGap {
                    worker: from,
                    expected,
                    received: delta.first_seq,
                },
            );
            // Above `expected`: a repeat or overlap returned earlier.
            worker.missing += delta.first_seq - expected;
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
        worker.total.add_pass(delta, Pass::Apply, Bounds::WORKER)?;
        Ok(if replaces_metrics {
            Taken::Restarted
        } else {
            Taken::Added
        })
    }

    /// The worker is gone without a final delta: what it sent stays merged,
    /// and its seconds no longer hold back which seconds are complete. The
    /// loss of a worker that sent anything is recorded in [`Self::gaps`], when
    /// gaps are kept, as a gap from its next sequence number, with nothing
    /// (0) received.
    pub(crate) fn lose(&mut self, worker: u32) {
        if let Some(total) = self.workers.get_mut(&worker) {
            if !total.lost && !total.finished {
                keep_gap(
                    &mut self.gaps,
                    &mut self.gaps_dropped,
                    SeqGap {
                        worker,
                        expected: total.last_seq + 1,
                        received: 0,
                    },
                );
            }
            total.lost = true;
        }
        // Built from the totals again, so the figures are added in worker
        // order, as a build from nothing adds them, not in arrival order.
        self.merged.get_mut().current = false;
    }

    /// The worker has sent its last delta and will send no final one: its
    /// seconds no longer hold back which seconds are complete.
    pub(crate) fn done(&mut self, worker: u32) {
        if let Some(worker) = self.workers.get_mut(&worker) {
            worker.finished = true;
        }
        // As in `lose`: the last build adds the figures in worker order.
        self.merged.get_mut().current = false;
    }

    /// Refuse, from now on, a delta carrying a second past `second`: the
    /// manager's own series clock, plus slack, at most the cap the
    /// accumulator was built with.
    pub(crate) fn set_max_second(&mut self, second: Second) {
        self.limits.max_second = second.min(self.cap_second);
    }

    /// Take user ordinals below `max_users` from now on, never fewer than
    /// before: the users a Controller added on the workers.
    pub(crate) fn set_max_users(&mut self, max_users: u64) {
        self.limits.max_users = self.limits.max_users.max(max_users);
    }

    /// Build whole run columns for the report's heatmap, or not, as Save
    /// report is turned on or off.
    pub(crate) fn set_columns(&mut self, keep: bool) {
        self.settings.columns = keep;
    }

    /// Merge only metrics epoch `epoch` from now on, the manager having
    /// reset its metrics: a worker on an older epoch is left out, and a
    /// delta of a newer one is refused.
    pub(crate) fn set_epoch(&mut self, epoch: u64) {
        if epoch > self.epoch {
            self.epoch = epoch;
            self.merged.get_mut().current = false;
        }
    }

    /// Keep every gap and repeat for [`Self::gaps`], up to
    /// [`MAX_GAPS_KEPT`], as the shadow merge reports them.
    pub(crate) fn keeping_gaps(mut self) -> Self {
        self.gaps = Some(Vec::new());
        self
    }

    /// Every gap and repeat seen in a worker's sequence, when they are
    /// kept ([`Self::keeping_gaps`]); none otherwise.
    pub(crate) fn gaps(&self) -> &[SeqGap] {
        self.gaps.as_deref().unwrap_or_default()
    }

    /// Gaps and repeats left out of [`Self::gaps`] past [`MAX_GAPS_KEPT`].
    pub(crate) fn gaps_dropped(&self) -> u64 {
        self.gaps_dropped
    }

    /// The second of `worker`'s delta whose late row last built the graph
    /// data again (tests).
    #[cfg(test)]
    pub(crate) fn late_rebuilt(&self, worker: u32) -> Option<Second> {
        self.workers
            .get(&worker)
            .and_then(|total| total.late_rebuilt)
    }

    /// Repeats of deltas from `worker` already taken, ignored (tests).
    #[cfg(test)]
    pub(crate) fn repeats(&self, worker: u32) -> u64 {
        self.workers.get(&worker).map_or(0, |total| total.repeats)
    }

    /// Whether `worker`'s final delta was merged.
    pub(crate) fn got_final(&self, worker: u32) -> bool {
        self.workers
            .get(&worker)
            .is_some_and(|total| total.got_final)
    }

    /// Every request `worker` made this epoch, by name, and the epoch.
    pub(crate) fn requests_of(&self, worker: u32) -> Option<(&BTreeMap<String, RequestPart>, u64)> {
        self.workers
            .get(&worker)
            .map(|total| (&total.total.metrics.requests, total.total.epoch))
    }

    /// The requests `worker` started in each second from `start` for `len`
    /// seconds, every request name together, as its graph rows this epoch
    /// hold them: the dashboard's RPS lines by worker. Its rows from before
    /// a metrics reset are cleared, as the merged series' are.
    pub(crate) fn requests_by_second(&self, worker: u32, start: Second, len: usize) -> Vec<u64> {
        let mut counts = vec![0; len];
        let Some(total) = self.workers.get(&worker) else {
            return counts;
        };
        let end = start.saturating_add(len as Second);
        for rows in total.total.graph.requests.values() {
            for (&second, &count) in rows.range(start..end) {
                counts[(second - start) as usize] += count;
            }
        }
        counts
    }

    /// The sequence number of the last delta merged from `worker`.
    pub(crate) fn last_seq(&self, worker: u32) -> Option<u64> {
        self.workers.get(&worker).map(|total| total.last_seq)
    }

    /// The deltas from `worker` that never arrived: every gap in its
    /// sequence, not a repeat, nor the loss of the worker itself.
    pub(crate) fn missing(&self, worker: u32) -> u64 {
        self.workers.get(&worker).map_or(0, |total| total.missing)
    }

    /// The first second not yet complete: one that a worker still sending
    /// could still add to, its rows reaching back the lag before its latest
    /// delta's second ([`Self::set_lag`]). `None` once every worker
    /// finished or was lost. A row for a second already built, past the
    /// lag, builds the graph data again from nothing.
    pub(crate) fn complete_before(&self) -> Option<Second> {
        self.workers
            .values()
            .filter(|worker| !worker.finished && !worker.lost)
            .map(|worker| worker.total.now.saturating_sub(1).saturating_sub(self.lag))
            .min()
    }

    /// The workers [`Self::merged_and_excluded`] leaves out (tests).
    #[cfg(test)]
    pub(crate) fn excluded(&self) -> Vec<u32> {
        self.current()
            .excluded
            .iter()
            .map(|&(worker, _)| worker)
            .collect()
    }

    /// The merged figures, built again first when they are not current.
    fn current(&self) -> std::cell::RefMut<'_, Merged> {
        let mut merged = self.merged.borrow_mut();
        if !merged.current {
            *merged = self.merge_parts();
        }
        merged
    }

    /// The metrics and graph data of every worker merged: the metrics of
    /// the manager's own epoch, and the graphs' seconds, the provisional
    /// ones as far as their rows have arrived. Every derived figure is worked out again from the
    /// merged counts. A worker left out is named by
    /// [`Self::merged_and_excluded`] (tests).
    #[cfg(test)]
    pub(crate) fn merged(&self) -> (GooseMetrics, GraphData) {
        let graph = self.test_graph.take().unwrap_or_else(GraphData::new);
        let (metrics, graph, _) = self.merged_and_excluded(graph);
        self.test_graph.replace(Some(graph.clone()));
        (metrics, graph)
    }

    /// The metrics and graph data of every worker merged, as
    /// [`Self::merged`] gives them, and each worker left out, with why:
    /// because its metrics epoch is older than the manager's, after a
    /// reset it missed. Each worker counts its
    /// own metrics resets, so one that missed a reset is left out rather
    /// than mixed into another epoch's figures. Also any whose figures
    /// would take a sum past [`Bounds::MERGED`], which [`Bounds::WORKER`]
    /// and [`super::delta::MAX_WORKERS`] rule out. Leaving a worker out
    /// undercounts the results: the manager records each one, and its run
    /// record marks the run degraded and names the worker.
    ///
    /// `graph` must be the graph data the last build returned, though the
    /// caller may since have pruned it: a build whose cache still holds
    /// extends it in place with the seconds completed since, copying only its
    /// users series; one whose cache was dropped drops it and builds from
    /// nothing.
    pub(crate) fn merged_and_excluded(
        &self,
        graph: GraphData,
    ) -> (GooseMetrics, GraphData, Vec<(u32, String)>) {
        let mut merged = self.current();
        let merged = &mut *merged;
        let complete = self.complete_before();
        // The merged workers' series origin.
        let series_origin = self
            .workers
            .iter()
            .filter(|(id, _)| !merged.excluded.iter().any(|(left_out, _)| left_out == *id))
            .filter_map(|(_, worker)| worker.total.series_origin)
            .max();
        let events = self.merged_events(merged);
        // The graph data built so far, when it still holds: only the
        // seconds completed since are added. Every second before the last
        // build's `before` is unchanged, or the cache was dropped.
        // Once every worker is done, nothing is left out of it.
        let excluded: Vec<u32> = merged.excluded.iter().map(|&(id, _)| id).collect();
        // Once every worker is done the graph data is built from nothing,
        // once, so the final graph equals a build from nothing exactly,
        // though its provisional seconds were added a delta at a time.
        let reusable = merged.built.take().filter(|built| {
            built.series_origin == series_origin
                && built.columns == self.settings.columns
                && built.excluded == excluded
                && complete.is_none_or(|complete| complete >= built.before)
                && !(merged.late && complete.is_none())
                && !(complete.is_none() && built.before != Second::MAX)
        });
        let fresh = reusable.is_none();
        if fresh {
            merged.late = false;
        }
        #[cfg(test)]
        if reusable.is_none() {
            FRESH_BUILDS.with(|builds| builds.set(builds.get() + 1));
        }
        let (mut graph, mut built) = match reusable {
            Some(mut built) => {
                let mut graph = graph;
                if let Some(users) = built.users.take() {
                    graph.restore_merged_users(users);
                }
                (graph, built)
            }
            None => (
                GraphData::gaggle_start(self.settings, series_origin),
                GraphCache {
                    before: 0,
                    users: None,
                    series_origin,
                    columns: self.settings.columns,
                    excluded,
                },
            ),
        };
        let users = self.users_levels(built.before, complete, &merged.excluded);
        if fresh {
            // From nothing: every row merged, the provisional seconds' too.
            graph.gaggle_extend(&merged.graph, &users, self.settings, 0, None);
        } else {
            // Every row from the last build's complete point on went into
            // the graph data as it arrived (see [`Self::apply_live`]): only
            // the users levels of the seconds completed since are added.
            graph.gaggle_extend(
                &GraphPart::default(),
                &users,
                self.settings,
                built.before,
                complete,
            );
        }
        // With every worker done, nothing later can be added but by a
        // delta, which builds again.
        built.before = complete.unwrap_or(Second::MAX);
        graph.set_provisional_from(complete);
        built.users = Some(graph.merged_users());
        // Finished each build: a later extend writes a second's response
        // times to the heatmap's columns once its ring slot is packed.
        graph.finish_latency_columns();
        merged.built = Some(built);
        (
            build_metrics(&self.template, &merged.metrics, events),
            graph,
            merged.excluded.clone(),
        )
    }

    /// Have the next build hold every row merged, building the graph data
    /// from nothing when a late row's build was put off (see [`Self::apply`]),
    /// as a report, a Controller and the end of a run read it: whether one
    /// was.
    pub(crate) fn settle(&self) -> bool {
        let mut merged = self.merged.borrow_mut();
        if merged.late {
            merged.built = None;
        }
        merged.late
    }

    /// Hand back the coordinated omission events the last build gave, as
    /// they were given: the next build adds only the events merged since,
    /// rather than gather and sort every worker's again. Without them, or
    /// once the merge is built again from the totals, it gathers them all.
    pub(crate) fn hand_back_events(&self, events: Vec<CoEvent>) {
        self.merged.borrow_mut().handed_back = Some(events);
    }

    /// The merged workers' coordinated omission events, oldest first, each
    /// worker's in its own order among events of the same second: those the
    /// last build gave, handed back, with only the events since put in
    /// their places, or every merged worker's gathered again.
    fn merged_events(&self, merged: &mut Merged) -> Vec<CoEvent> {
        let new = std::mem::take(&mut merged.new_events);
        if let (Some(mut events), Some(keys)) =
            (merged.handed_back.take(), merged.event_keys.as_mut())
        {
            if events.len() == keys.len() {
                #[cfg(test)]
                EVENTS_GATHERED.with(|gathered| gathered.set(gathered.get() + new.len()));
                place_events(keys, &mut events, new);
                return events;
            }
        }
        let mut keyed = Vec::new();
        for (&id, worker) in &self.workers {
            if merged.excluded.iter().any(|&(left_out, _)| left_out == id) {
                continue;
            }
            if let Some(co) = worker.total.metrics.co.as_ref() {
                #[cfg(test)]
                EVENTS_GATHERED.with(|gathered| gathered.set(gathered.get() + co.events.len()));
                keyed.extend(
                    co.events
                        .iter()
                        .enumerate()
                        .map(|(at, event)| ((event.timestamp_secs, id, at), event.clone())),
                );
            }
        }
        keyed.sort_unstable_by_key(|(key, _)| *key);
        let (keys, events) = keyed.into_iter().unzip();
        merged.event_keys = Some(keys);
        events
    }

    /// Every worker's figures of the run's epoch added up, from their
    /// totals, and the workers left out, with why.
    fn merge_parts(&self) -> Merged {
        let run_epoch = self.epoch;
        let mut merged = Merged {
            current: true,
            ..Merged::default()
        };
        let Merged {
            metrics,
            graph,
            excluded,
            ..
        } = &mut merged;
        for (&id, worker) in &self.workers {
            let (part, rows) = (&worker.total.metrics, &worker.total.graph);
            if worker.total.epoch != run_epoch {
                excluded.push((
                    id,
                    format!(
                        "its metrics epoch {} is not the run's {}: it missed a metrics reset",
                        worker.total.epoch, run_epoch
                    ),
                ));
                continue;
            }
            let fits = metrics.add_pass(part, Pass::Check, Bounds::MERGED).is_ok()
                && graph.add_rates(rows, Pass::Check, Bounds::MERGED).is_ok();
            if !fits {
                excluded.push((
                    id,
                    "its figures would take the merged sums past their bounds".to_string(),
                ));
                continue;
            }
            // Checked above, so these add every figure.
            let added = metrics
                .add_pass(part, Pass::Apply, Bounds::MERGED)
                .and_then(|()| graph.add_rates(rows, Pass::Apply, Bounds::MERGED));
            debug_assert!(added.is_ok());
        }
        merged.drop_events();
        merged
    }

    /// The users running at each second from `from` and before `to` (every
    /// one from `from` when `None`), between the first and last second any
    /// merged worker sent a level for: every merged worker's level at that
    /// second added up, a worker counting 0 before its first level and
    /// holding its last. A worker in `excluded` is left out, as its metrics
    /// are.
    fn users_levels(
        &self,
        from: Second,
        to: Option<Second>,
        excluded: &[(u32, String)],
    ) -> BTreeMap<Second, u64> {
        let merged = || {
            self.workers
                .iter()
                .filter(|(id, _)| !excluded.iter().any(|(left_out, _)| left_out == *id))
                .map(|(_, worker)| &worker.total.graph.users)
        };
        let first = merged().filter_map(|users| users.keys().next()).min();
        let last = merged().filter_map(|users| users.keys().next_back()).max();
        let (Some(&first), Some(&last)) = (first, last) else {
            return BTreeMap::new();
        };
        let start = from.max(first);
        let end = to.map_or(last, |to| last.min(to.saturating_sub(1)));
        if to == Some(0) || start > end {
            return BTreeMap::new();
        }
        // Levels are at most MAX_COUNT and workers at most MAX_WORKERS, so
        // every sum fits a u64.
        (start..=end)
            .map(|second| {
                let users = merged()
                    .filter_map(|users| users.range(..=second).next_back())
                    .map(|(_, &users)| users)
                    .sum();
                (second, users)
            })
            .collect()
    }

    /// The users levels over the whole run, as [`Self::users_levels`] built
    /// them before (tests).
    #[cfg(test)]
    fn all_users_levels(&self) -> BTreeMap<Second, u64> {
        // Each worker's change of level at each second it sent one, added
        // up, then walked once: a worker counts 0 before its first level.
        // Levels are at most MAX_COUNT and workers at most MAX_WORKERS, so
        // every sum fits an i64.
        let mut changes: BTreeMap<Second, i64> = BTreeMap::new();
        for worker in self.workers.values() {
            let mut level = 0;
            for (&second, &users) in &worker.total.graph.users {
                *changes.entry(second).or_default() += users as i64 - level;
                level = users as i64;
            }
        }
        let (Some(&first), Some(&last)) = (changes.keys().next(), changes.keys().next_back())
        else {
            return BTreeMap::new();
        };
        let mut changes = changes.into_iter().peekable();
        let mut users: i64 = 0;
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

impl Merged {
    /// Add a delta of merged worker `from`, as it was added to the worker's
    /// own total, its coordinated omission events from `first_event` there:
    /// whether it fit, past [`Bounds::MERGED`] nothing being added.
    fn follow(&mut self, delta: &GaggleDelta, from: u32, first_event: usize) -> bool {
        let fits = self
            .metrics
            .add_pass(&delta.metrics, Pass::Check, Bounds::MERGED)
            .is_ok()
            && self
                .graph
                .add_rates(&delta.graph, Pass::Check, Bounds::MERGED)
                .is_ok();
        if !fits {
            return false;
        }
        // Checked above, so these add every figure.
        let added = self
            .metrics
            .add_pass(&delta.metrics, Pass::Apply, Bounds::MERGED)
            .and_then(|()| {
                self.graph
                    .add_rates(&delta.graph, Pass::Apply, Bounds::MERGED)
            });
        debug_assert!(added.is_ok());
        self.drop_events();
        // Kept for the next build, once a build gathered the events.
        if let (Some(_), Some(co)) = (self.event_keys.as_ref(), delta.metrics.co.as_ref()) {
            self.new_events
                .extend(co.events.iter().enumerate().map(|(at, event)| {
                    (
                        (event.timestamp_secs, from, first_event + at),
                        event.clone(),
                    )
                }));
        }
        true
    }

    /// Hold no coordinated omission events: the merge takes them from the
    /// workers' totals, so they keep each worker's order.
    fn drop_events(&mut self) {
        if let Some(co) = self.metrics.co.as_mut() {
            co.events.clear();
        }
    }
}

/// Two consecutive deltas from one worker as one: applying it is applying
/// `a` then `b`. A newer epoch in `b` starts again from `b`, keeping `a`'s
/// users levels, and a final `b`'s metrics replace `a`'s while its graph
/// rows add to `a`'s. Two exceptions: coordinated omission events stay
/// capped at [`super::delta::MAX_CO_EVENTS_PER_DELTA`] in a delta, the
/// oldest kept, where applying both keeps up to the cap from each; and a
/// final `a` followed by a delta of a newer epoch does not mark the worker
/// finished. Refused, as [`GaggleDelta::add`], past [`Bounds::WORKER`]:
/// then `a` comes back unchanged with the error. `a` is joined in place,
/// never copied.
// The error hands `a` back whole, so the caller keeps it: boxing it would
// cost an allocation on every join.
#[allow(clippy::result_large_err)]
pub(crate) fn coalesce(
    mut a: GaggleDelta,
    b: &GaggleDelta,
) -> Result<GaggleDelta, (GaggleDelta, DeltaError)> {
    if b.epoch < a.epoch {
        return Ok(a);
    }
    if b.epoch > a.epoch {
        let mut users = std::mem::take(&mut a.graph.users);
        let mut joined = b.clone();
        users.append(&mut joined.graph.users);
        joined.graph.users = users;
        joined.first_seq = a.first_seq;
        return Ok(joined);
    }
    // A final `b`'s metrics replace `a`'s: set aside, so a refusal can put
    // them back.
    let (kind, metrics) = (
        a.kind,
        (b.kind == DeltaKind::Final).then(|| std::mem::take(&mut a.metrics)),
    );
    if b.kind == DeltaKind::Final {
        a.kind = DeltaKind::Final;
    }
    if let Err(e) = a.add(b) {
        a.kind = kind;
        if let Some(metrics) = metrics {
            a.metrics = metrics;
        }
        return Err((a, e));
    }
    if a.kind == DeltaKind::Delta {
        if let Some(co) = a.metrics.co.as_mut() {
            co.cap_events();
        }
    }
    Ok(a)
}

/// `template` with every figure of `part` added, as `GooseMetrics`, and
/// `events`, oldest first, as its coordinated omission events. Every field of every
/// aggregate is named, so a field added to one fails to compile here until
/// it is merged or left out on purpose.
fn build_metrics(
    template: &GooseMetrics,
    part: &MetricsPart,
    events: Vec<CoEvent>,
) -> GooseMetrics {
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
            // Passed in, each worker's in its own order.
            events: _,
            // Counted by the shadow merge, which checks what they leave out.
            events_dropped: _,
        } = co;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The users levels over any range of seconds are those of the whole
    /// run, cut to the range, so builds over consecutive ranges add up to
    /// one over the whole run.
    #[test]
    fn users_levels_by_range_are_the_whole_runs() {
        let settings = GraphSettings {
            records: true,
            latency: false,
            synthetic: false,
            columns: false,
        };
        let mut accumulator =
            GaggleAccumulator::new(GooseMetrics::default(), settings, DeltaLimits::default());
        // Workers starting at different seconds, with gaps and changes.
        // The first level is at second 2: none is counted before it.
        for (worker, levels) in vec![
            vec![(4, 3), (5, 4), (9, 1)],
            vec![(2, 5), (7, 0), (8, 2)],
            vec![(6, 9)],
        ]
        .into_iter()
        .enumerate()
        {
            let mut delta = GaggleDelta::new(worker as u32, 1, 0, DeltaKind::Delta);
            delta.now = 10;
            delta.graph.users = levels.into_iter().collect();
            accumulator.apply(worker as u32, delta).unwrap();
        }
        let whole = accumulator.all_users_levels();
        assert_eq!(whole.len(), 8);
        assert_eq!(whole.keys().next(), Some(&2));
        assert_eq!(accumulator.users_levels(0, None, &[]), whole);
        let mut pieces = BTreeMap::new();
        for (from, to) in [(0, 3), (3, 6), (6, 6), (6, 11), (11, 20)] {
            pieces.extend(accumulator.users_levels(from, Some(to), &[]));
        }
        assert_eq!(pieces, whole);
    }
}
