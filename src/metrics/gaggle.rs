use super::*;

/// The metrics processor as a Gaggle worker.
impl MetricsProcessor {
    /// Run as Gaggle worker `worker`: from now on every metric is also kept
    /// in what the next delta sends.
    pub(crate) fn gaggle_enable(&mut self, worker: u32) {
        let template = self.gaggle_template();
        let synthetic = template.coordinated_omission_metrics.is_some();
        let lag = self.configuration.request_timeout().as_secs() as usize + 60;
        self.gaggle = Some(Box::new(crate::gaggle::worker::WorkerState::new(
            worker, template, synthetic, lag,
        )));
    }

    /// Set the worker's lag (unit tests).
    #[cfg(test)]
    pub(crate) fn gaggle_set_lag_for_test(&mut self, lag: usize) {
        if let Some(state) = self.gaggle.as_mut() {
            state.set_lag_for_test(lag);
        }
    }

    /// Run the `--gaggle-shadow-merge` self check, writing how it ended to
    /// `outcome` at shutdown.
    pub(crate) fn gaggle_enable_shadow(
        &mut self,
        outcome: Arc<Mutex<Option<crate::gaggle::shadow::ShadowOutcome>>>,
    ) {
        self.gaggle_enable(0);
        let template = self.gaggle_template();
        let settings = self.gaggle_graph_settings();
        if let Some(state) = self.gaggle.as_mut() {
            state.shadow = Some(crate::gaggle::shadow::ShadowMerge::new(
                template, settings, outcome,
            ));
        }
    }

    /// The metrics this processor started with, empty: every scenario and
    /// transaction, the hosts and, with mitigation on, coordinated omission
    /// metrics.
    pub(crate) fn gaggle_template(&self) -> GooseMetrics {
        let mut template = GooseMetrics {
            coordinated_omission_metrics: self.metrics.coordinated_omission_metrics.as_ref().map(
                |co| CoordinatedOmissionMetrics {
                    started_secs: co.started_secs,
                    ..CoordinatedOmissionMetrics::new(co.mitigation_strategy.clone())
                },
            ),
            ..Default::default()
        };
        template.initialize_scenario_metrics(&self.scenarios, &self.configuration);
        // The processor's own metrics passed this at startup.
        let _ = template.initialize_transaction_metrics(
            &self.scenarios,
            &self.configuration,
            &self.defaults,
        );
        template
    }

    /// How a manager builds graph data for a run with this configuration.
    pub(crate) fn gaggle_graph_settings(&self) -> crate::graph::GraphSettings {
        self.graph_data
            .gaggle_settings(self.configuration.records_graph_data())
    }

    /// Process `metric` a second time, into the Gaggle worker's pending
    /// state, with the same code.
    pub(super) fn shadow_metric(&mut self, metric: &GooseMetric) {
        let Some(mut state) = self.gaggle.take() else {
            return;
        };
        // A transaction is stamped with the second it started, which can be
        // any time ago: the pending series reach back to it.
        if let Some(second) = earliest_second(metric) {
            state.pending_graph.gaggle_lower_origin(second);
        }
        std::mem::swap(&mut self.metrics, &mut state.pending_metrics);
        std::mem::swap(&mut self.graph_data, &mut state.pending_graph);
        self.gaggle = Some(state);
        self.shadow_pass = true;
        self.dispatch_metric(metric);
        self.shadow_pass = false;
        if let Some(state) = self.gaggle.as_mut() {
            std::mem::swap(&mut self.metrics, &mut state.pending_metrics);
            std::mem::swap(&mut self.graph_data, &mut state.pending_graph);
        }
    }

    /// What changed since the last delta, `now` being the series second;
    /// `None` unless the processor runs as a Gaggle worker.
    pub(crate) fn gaggle_take_delta(
        &mut self,
        now: usize,
    ) -> Option<crate::gaggle::delta::GaggleDelta> {
        let epoch = self.metrics_epoch.load(AtomicOrdering::Relaxed);
        let state = self.gaggle.as_mut()?;
        Some(state.take_delta(
            (&self.metrics, &self.graph_data),
            &self.request_counter_registry,
            epoch,
            now as u64,
        ))
    }

    /// The worker's whole state, as a final delta; `None` unless the
    /// processor runs as a Gaggle worker.
    pub(crate) fn gaggle_take_final(
        &mut self,
        now: usize,
    ) -> Option<crate::gaggle::delta::GaggleDelta> {
        self.sync_atomic_counters();
        let epoch = self.metrics_epoch.load(AtomicOrdering::Relaxed);
        let state = self.gaggle.as_mut()?;
        Some(state.take_final((&self.metrics, &self.graph_data), epoch, now as u64))
    }

    /// This second's delta, through the shadow merge.
    pub(super) fn gaggle_shadow_tick(&mut self, now: usize) {
        if let Some(delta) = self.gaggle_take_delta(now) {
            if let Some(shadow) = self.gaggle.as_mut().and_then(|state| state.shadow.as_mut()) {
                shadow.delta(&delta);
            }
        }
    }

    /// At shutdown, with the metrics final: end the shadow merge's self
    /// check.
    pub(super) fn gaggle_shadow_finish(&mut self) {
        let Some(shadow) = self.gaggle.as_mut().and_then(|state| state.shadow.take()) else {
            return;
        };
        let now = self
            .graph_data
            .gaggle_last_second()
            .map_or(0, |second| second + 2);
        let (Some(last), Some(last_final)) =
            (self.gaggle_take_delta(now), self.gaggle_take_final(now))
        else {
            return;
        };
        let configuration = &self.configuration;
        let build = |metrics: &GooseMetrics, graph: &mut GraphData| -> serde_json::Value {
            gaggle_snapshot(configuration, metrics, graph)
        };
        let snapshot: Option<crate::gaggle::shadow::SnapshotBuilder<'_>> =
            if self.configuration.dashboard {
                Some(&build)
            } else {
                None
            };
        shadow.finish(
            &last,
            &last_final,
            (&self.metrics, &self.graph_data),
            snapshot,
        );
    }
}

/// The earliest series second `metric` records per second graph data for.
pub(super) fn earliest_second(metric: &GooseMetric) -> Option<usize> {
    let second = |elapsed: u64| (elapsed / 1000) as usize;
    match metric {
        GooseMetric::Request(request) => Some(second(request.elapsed)),
        GooseMetric::Transaction(transaction) => Some(second(transaction.elapsed)),
        GooseMetric::Scenario(scenario) => Some(second(scenario.elapsed)),
        GooseMetric::All {
            request,
            transaction,
            scenario,
        } => Some(second(
            request
                .elapsed
                .min(transaction.elapsed)
                .min(scenario.elapsed),
        )),
        GooseMetric::Batch(batch) => batch
            .second
            .into_iter()
            .chain(batch.graph_request_data.keys().map(|(_, second)| *second))
            .chain(batch.graph_tps.keys().copied())
            .chain(batch.graph_sps.keys().copied())
            .chain(
                batch
                    .individual_requests
                    .iter()
                    .map(|request| second(request.elapsed)),
            )
            .min(),
        GooseMetric::TransactionError(_) => None,
    }
}

/// A dashboard snapshot of `metrics` and `graph`, as JSON, with what changes
/// from one build to the next (the build time, timestamps and the heatmap's
/// change counters) left out, for the shadow merge to compare.
pub(crate) fn gaggle_snapshot(
    configuration: &GooseConfiguration,
    metrics: &GooseMetrics,
    graph: &mut GraphData,
) -> serde_json::Value {
    let co_active = metrics
        .coordinated_omission_metrics
        .as_ref()
        .is_some_and(|m| m.has_events());
    let series = graph.export_series_window_with_heatmap(
        dashboard_snapshot::SERIES_WINDOW_SECS,
        None,
        co_active,
        None,
    );
    let snapshot =
        dashboard_snapshot::build_dashboard_snapshot(dashboard_snapshot::DashboardSnapshotInput {
            metrics,
            series,
            active_users: 0,
            maximum_users: metrics.maximum_users,
            target_users: 0,
            total_users: metrics.total_users,
            phase: "shutdown".to_string(),
            stopping: false,
            series_window_secs: dashboard_snapshot::SERIES_WINDOW_SECS,
            no_status_codes: configuration.no_status_codes,
            metrics_disabled: configuration.no_metrics,
            no_transaction_metrics: configuration.no_transaction_metrics,
            no_scenario_metrics: configuration.no_scenario_metrics,
            report: dashboard_snapshot::SnapshotReport::default(),
            settings: dashboard_snapshot::SnapshotSettings::idle(),
            percentiles_over_time: configuration.records_latency_series(),
            series_keys: graph.latency_series_keys(),
        });
    let mut value = serde_json::to_value(&snapshot).unwrap_or_default();
    for pointer in ["/generated_at", "/build_time"] {
        if let Some(field) = value.pointer_mut(pointer) {
            *field = serde_json::Value::Null;
        }
    }
    if let Some(heatmap) = value
        .pointer_mut("/series/heatmap")
        .and_then(|heatmap| heatmap.as_object_mut())
    {
        for field in ["generation", "rev", "since"] {
            heatmap.insert(field.to_string(), serde_json::Value::Null);
        }
    }
    value
}
