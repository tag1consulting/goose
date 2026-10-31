//! `--gaggle-shadow-merge`: a normal run that is also its own Gaggle worker
//! and manager. Each second the metrics processor takes a delta, encodes it,
//! decodes it and adds it to two managers' accumulators; at the end the
//! first gets the last delta, and the second gets it and then the final
//! delta, whose metrics replace what it holds. Both merges must equal the
//! run's own metrics and graph data, and its dashboard snapshot.

use super::delta::GaggleDelta;
use super::merge::GaggleAccumulator;
use crate::graph::{GraphData, GraphSettings};
use crate::metrics::GooseMetrics;
use std::sync::{Arc, Mutex};

/// Builds a dashboard snapshot, as JSON with what changes from one build to
/// the next left out, from metrics and graph data.
pub(crate) type SnapshotBuilder<'a> =
    &'a dyn Fn(&GooseMetrics, &mut GraphData) -> serde_json::Value;

/// The file a failed self check writes its differences to.
pub(crate) const DIFF_FILE: &str = "gaggle-shadow-diff.txt";

/// The manager's side of the self check, kept by the worker.
pub(crate) struct ShadowMerge {
    deltas: GaggleAccumulator,
    finals: GaggleAccumulator,
    /// Each delta's size encoded, in bytes.
    sizes: Vec<usize>,
    /// Encoding and decoding errors.
    errors: Vec<String>,
    /// Coordinated omission events the deltas left out at the cap.
    dropped: u64,
    /// The epoch `dropped` counts.
    dropped_epoch: u64,
    /// Where the run finds how the check ended.
    outcome: Arc<Mutex<Option<ShadowOutcome>>>,
}

/// How the self check ended.
#[derive(Clone, Debug, Default)]
pub(crate) struct ShadowOutcome {
    /// Every difference found, as `field: merged != run`; empty on a match.
    pub(crate) differences: Vec<String>,
    /// Deltas sent, the final one included.
    pub(crate) deltas: usize,
    /// Delta sizes in bytes: p50, p99 and maximum.
    pub(crate) sizes: (usize, usize, usize),
}

impl ShadowOutcome {
    /// The line printed on a match.
    pub(crate) fn summary(&self) -> String {
        format!(
            "gaggle shadow merge: OK, {} deltas, delta size p50 {} B, p99 {} B, max {} B",
            self.deltas, self.sizes.0, self.sizes.1, self.sizes.2
        )
    }
}

impl ShadowMerge {
    /// Merging into `template`, the run's empty metrics, building graph data
    /// with `settings`.
    /// `outcome` gets how the check ended.
    pub(crate) fn new(
        template: GooseMetrics,
        settings: GraphSettings,
        outcome: Arc<Mutex<Option<ShadowOutcome>>>,
    ) -> Self {
        ShadowMerge {
            deltas: GaggleAccumulator::new(template.clone(), settings),
            finals: GaggleAccumulator::new(template, settings),
            sizes: Vec::new(),
            errors: Vec::new(),
            dropped: 0,
            dropped_epoch: 0,
            outcome,
        }
    }

    /// Send `delta` over the wire, as bytes, to the accumulator.
    fn send(&mut self, delta: &GaggleDelta, to_finals: bool) {
        let decoded = delta
            .encode()
            .map_err(|e| format!("encode delta {}: {e}", delta.seq))
            .and_then(|bytes| {
                self.sizes.push(bytes.len());
                GaggleDelta::decode(&bytes).map_err(|e| format!("decode delta {}: {e}", delta.seq))
            });
        match decoded {
            Ok(decoded) => {
                if &decoded != delta {
                    self.errors
                        .push(format!("delta {} changed on the wire", delta.seq));
                }
                if to_finals {
                    self.finals.apply(decoded);
                } else {
                    self.finals.apply(decoded.clone());
                    // A metrics reset starts the count again, as it starts
                    // the merged events again.
                    let dropped = decoded
                        .metrics
                        .co
                        .as_ref()
                        .map_or(0, |co| co.events_dropped);
                    if decoded.epoch > self.dropped_epoch {
                        self.dropped_epoch = decoded.epoch;
                        self.dropped = dropped;
                    } else if decoded.epoch == self.dropped_epoch {
                        self.dropped += dropped;
                    }
                    self.deltas.apply(decoded);
                }
            }
            Err(e) => self.errors.push(e),
        }
    }

    /// One second's delta.
    pub(crate) fn delta(&mut self, delta: &GaggleDelta) {
        self.send(delta, false);
    }

    /// The end of the run: the last delta, then the final one, which only
    /// the second accumulator gets, after every delta. `run` is the run's own metrics and
    /// graph data, finished; `snapshot` builds a dashboard snapshot with
    /// fixed timestamps from metrics and graph data, when the dashboard is
    /// on. How it ended also goes to the outcome given to [`Self::new`].
    pub(crate) fn finish(
        mut self,
        last: &GaggleDelta,
        last_final: &GaggleDelta,
        run: (&GooseMetrics, &GraphData),
        snapshot: Option<SnapshotBuilder<'_>>,
    ) -> ShadowOutcome {
        self.delta(last);
        self.deltas.done(last.worker);
        self.send(last_final, true);
        let mut differences = std::mem::take(&mut self.errors);
        for gap in self.deltas.gaps() {
            differences.push(format!("sequence gap: {gap:?}"));
        }
        let mut run_graph = run.1.clone();
        let run_snapshot = snapshot.map(|build| build(run.0, &mut run_graph));
        for (name, accumulator) in [("deltas", &self.deltas), ("final", &self.finals)] {
            let (mut metrics, mut graph) = accumulator.merged();
            super::compare::adopt_unmerged(&mut metrics, run.0);
            // Events the deltas left out at the cap are missing from them,
            // and only from them; the count left out must account for it.
            let capped = name == "deltas" && self.dropped > 0;
            if capped {
                let held = |metrics: &GooseMetrics| {
                    metrics
                        .coordinated_omission_metrics
                        .as_ref()
                        .map_or(0, |co| co.co_events.len() as u64)
                };
                if held(run.0) != held(&metrics) + self.dropped {
                    differences.push(format!(
                        "deltas: {} coordinated omission events held, {} merged and {} left out at the cap",
                        held(run.0),
                        held(&metrics),
                        self.dropped
                    ));
                }
            }
            for difference in super::compare::metrics_diff(&metrics, run.0) {
                if capped && difference.starts_with("coordinated_omission_metrics.co_events") {
                    continue;
                }
                differences.push(format!("{name}: metrics.{difference}"));
            }
            for difference in graph.gaggle_diff(run.1) {
                differences.push(format!("{name}: graph.{difference}"));
            }
            if let (Some(build), Some(expected)) = (snapshot, run_snapshot.as_ref()) {
                let merged = build(&metrics, &mut graph);
                let mut found = Vec::new();
                super::json_diff("snapshot", &merged, expected, &mut found);
                differences.extend(found.into_iter().map(|d| format!("{name}: {d}")));
            }
        }
        let mut sizes = self.sizes.clone();
        sizes.sort_unstable();
        let at = |percent: usize| {
            sizes
                .get((sizes.len() * percent / 100).min(sizes.len().saturating_sub(1)))
                .copied()
                .unwrap_or(0)
        };
        let outcome = ShadowOutcome {
            differences,
            deltas: self.sizes.len(),
            sizes: (at(50), at(99), sizes.last().copied().unwrap_or(0)),
        };
        if let Ok(mut slot) = self.outcome.lock() {
            *slot = Some(outcome.clone());
        }
        outcome
    }
}
