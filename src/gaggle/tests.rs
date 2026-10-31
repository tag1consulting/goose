//! Gaggle merge tests: one process against N workers on the same seeded
//! metrics, through the user threads' batching and the metrics processor's
//! own code.

use super::delta::{DeltaError, DeltaKind, DeltaLimits, GaggleDelta};
use super::merge::{coalesce, GaggleAccumulator};
use crate::config::{GooseConfiguration, GooseDefaults};
use crate::goose::{GooseMethod, GooseUser, Scenario, Transaction, TransactionName};
use crate::graph::GraphData;
use crate::metrics::{
    GooseMetric, GooseMetricBatch, GooseMetrics, GooseRawRequest, GooseRequestCounterRegistry,
    GooseRequestMetric, MetricsCommand, MetricsEpoch, MetricsProcessor, ScenarioMetric,
    TransactionDetail, TransactionMetric,
};
use crate::test_plan::{TestPlanHistory, TestPlanStepAction};
use chrono::{TimeZone, Utc};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A small seeded generator.
struct Rng(u64);

impl Rng {
    fn below(&mut self, modulus: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % modulus
    }

    fn chance(&mut self, one_in: u64) -> bool {
        self.below(one_in) == 0
    }
}

/// Users in the synthetic run, numbered by global ordinal.
const USERS: usize = 12;
/// Request names: more than get a heatmap ring of their own.
const NAMES: usize = 105;
/// Seconds of load.
const SECONDS: usize = 32;
/// The series second of the metrics reset.
const RESET_AT: usize = 10;
/// Seconds with no users running: a hold at 0 users.
const IDLE: std::ops::Range<usize> = 26..29;
/// Each scenario's weight; user `u` runs scenario `SCENARIO_OF[u % 6]`.
const WEIGHTS: [usize; 3] = [3, 2, 1];
const SCENARIO_OF: [usize; 6] = [0, 0, 0, 1, 1, 2];

/// One thing that happens in the synthetic run, in order.
#[derive(Clone)]
enum Event {
    /// User `user` starts running its scenario.
    Start(usize),
    /// A request's metric, through the user's batch.
    Request(usize, GooseRequestMetric),
    /// `set_failure()` (`false`) or `set_success()` (`true`) on a request
    /// sent before.
    Correct(usize, GooseRequestMetric, bool),
    /// A transaction, and its error when it failed.
    Transaction(usize, TransactionMetric, Option<String>),
    /// A scenario iteration.
    Scenario(usize, ScenarioMetric),
    /// A user sends its batch.
    Flush(usize),
    /// The end of a series second: users recorded, one fewer than run
    /// when any do, then deltas taken.
    Second(usize),
    /// The users of a second recorded again, all of them, after its delta
    /// was taken: a level sent twice.
    Users(usize),
    /// The metrics reset, at this series second.
    Reset(usize),
}

fn noop() -> crate::goose::TransactionFunction {
    Arc::new(|_user| Box::pin(async { Ok(()) }))
}

/// Three weighted scenarios with transactions, indexed as a load test
/// indexes them.
fn scenarios() -> Vec<Scenario> {
    (0..3)
        .map(|index| {
            let mut scenario = Scenario::new(&format!("Scenario {index}"));
            scenario.scenarios_index = index;
            scenario.weight = WEIGHTS[index];
            for t in 0..=index {
                let mut transaction = Transaction::new(noop());
                transaction.name =
                    TransactionName::TransactionOnly(Arc::from(format!("t{index}{t}").as_str()));
                transaction.transactions_index = t;
                scenario.transactions.push(transaction);
            }
            scenario
        })
        .collect()
}

fn configuration() -> GooseConfiguration {
    GooseConfiguration::parse_args_default(&[
        "--host",
        "http://127.0.0.1",
        "--report-file",
        "report.html",
        "--dashboard",
        "--co-mitigation",
        "average",
    ])
    .unwrap()
}

fn name(index: usize) -> String {
    format!("/n{index:03}")
}

/// A request metric for `user` (global ordinal) of `name`, started at
/// `elapsed` ms, that took `us` microseconds and answered `status`.
fn request(
    user: usize,
    name: &str,
    method: GooseMethod,
    elapsed: u64,
    us: u64,
    status: u16,
) -> GooseRequestMetric {
    let scenario = SCENARIO_OF[user % 6];
    let raw = GooseRawRequest::new(method, &format!("http://127.0.0.1{name}"), vec![], "");
    let mut metric = GooseRequestMetric::new(
        raw,
        TransactionDetail {
            scenario_index: scenario,
            scenario_name: Arc::from(format!("scenario{scenario}").as_str()),
            transaction_index: Some(0),
            transaction_name: TransactionName::TransactionOnly(Arc::from("t")),
        },
        name,
        elapsed as u128,
        user,
    );
    metric.set_response_micros(us);
    metric.status_code = status;
    metric.success = (200..400).contains(&status);
    if !metric.success {
        metric.error = format!("{status} error");
    }
    metric
}

/// A response time from one of the ranges the buckets treat differently.
fn response_us(rng: &mut Rng) -> u64 {
    match rng.below(20) {
        0..=2 => rng.below(128),
        // Both sides of bucket edges.
        3..=5 => {
            let edges = [128u64, 256, 1024, 4096, 65_536, 1_048_576];
            let edge = edges[rng.below(edges.len() as u64) as usize];
            edge + rng.below(3) - 1
        }
        6..=13 => 1_000 + rng.below(9_000),
        14..=16 => 999_000 + rng.below(2_000),
        17 => 10_000_000 + rng.below(5_000_000),
        _ => 128 + rng.below(1_000),
    }
}

/// Whether user `user` runs at series second `second`.
fn active(user: usize, second: usize) -> bool {
    !IDLE.contains(&second) && user < (2 + 2 * second).min(USERS)
}

/// The seeded run.
fn events(seed: u64) -> Vec<Event> {
    let mut rng = Rng(seed);
    let mut events = Vec::new();
    let mut started = [false; USERS];
    let mut sent: Vec<(usize, GooseRequestMetric)> = Vec::new();
    for second in 0..SECONDS {
        for (user, started) in started.iter_mut().enumerate() {
            if active(user, second) && !*started {
                *started = true;
                events.push(Event::Start(user));
            }
        }
        if second == RESET_AT {
            events.push(Event::Reset(second));
        }
        if second == 0 || second == RESET_AT {
            // Every name first, in order, failed so each goes one by one:
            // a single process gives the first 100 names it sees a heatmap
            // ring of their own, as it sees them, from the start and again
            // from the reset, and a manager in order of name within a
            // second.
            for index in 0..NAMES {
                let metric = request(
                    0,
                    &name(index),
                    GooseMethod::Get,
                    (second * 1000) as u64 + 10 + index as u64,
                    300 + index as u64,
                    500,
                );
                events.push(Event::Request(0, metric));
            }
            events.push(Event::Flush(0));
        }
        for user in (0..USERS).filter(|&user| active(user, second)) {
            let scenario = SCENARIO_OF[user % 6];
            for _ in 0..1 + rng.below(6) {
                let index = if rng.chance(3) {
                    rng.below(NAMES as u64) as usize
                } else {
                    rng.below(8) as usize
                };
                let method = if rng.chance(5) {
                    GooseMethod::Post
                } else {
                    GooseMethod::Get
                };
                let elapsed = (second * 1000) as u64 + rng.below(1000);
                let status = match rng.below(30) {
                    0 => 500,
                    1 => 404,
                    2 => 302,
                    _ => 200,
                };
                let us = response_us(&mut rng);
                let metric = request(user, &name(index), method, elapsed, us, status);
                events.push(Event::Request(user, metric.clone()));
                if rng.chance(15) {
                    // A stall the next request ends: its copy with the
                    // cadence missed.
                    let cadence = 20 + rng.below(80);
                    let mut copy = metric.clone();
                    copy.coordinated_omission_elapsed = cadence * (3 + rng.below(30));
                    copy.user_cadence = cadence;
                    events.push(Event::Request(user, copy));
                }
                sent.push((user, metric));
                let transaction_index = rng.below(scenario as u64 + 1) as usize;
                let mut transaction = TransactionMetric::new(
                    elapsed as u128,
                    scenario,
                    transaction_index,
                    Arc::from(format!("t{scenario}{transaction_index}").as_str()),
                    user,
                );
                transaction.set_duration(Duration::from_micros(us + rng.below(500)), status < 400);
                let error = (status >= 400).then(|| format!("transaction {status}"));
                events.push(Event::Transaction(user, transaction, error));
                if rng.chance(3) {
                    let scenario_metric = ScenarioMetric::new(
                        elapsed as u128,
                        Arc::from(format!("Scenario {scenario}").as_str()),
                        scenario,
                        Duration::from_micros(us * 2 + rng.below(1_000)),
                        user,
                    );
                    events.push(Event::Scenario(user, scenario_metric));
                }
            }
            // Corrections of an earlier request of the same user.
            if rng.chance(4) {
                if let Some((owner, metric)) =
                    sent.iter().rev().find(|(owner, _)| *owner == user).cloned()
                {
                    let success = !metric.success;
                    events.push(Event::Correct(owner, metric, success));
                }
            }
            if rng.chance(2) {
                events.push(Event::Flush(user));
            }
        }
        events.push(Event::Second(second));
        events.push(Event::Users(second));
    }
    events
}

/// A processor, and the users whose batches it receives.
struct Side {
    processor: MetricsProcessor,
    users: BTreeMap<usize, GooseUser>,
    rx: flume::Receiver<GooseMetric>,
}

impl Side {
    /// A processor for the users with global ordinals `users`, as a Gaggle
    /// worker numbered `worker` or, `None`, as a single process.
    fn new(users: std::ops::Range<usize>, worker: Option<u32>) -> Side {
        let configuration = configuration();
        let scenarios = scenarios();
        let defaults = GooseDefaults::default();
        let registry: GooseRequestCounterRegistry = Arc::new(Mutex::new(HashMap::new()));
        let epoch: MetricsEpoch = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let mut metrics = GooseMetrics {
            coordinated_omission_metrics: Some(
                crate::metrics::coordinated_omission::CoordinatedOmissionMetrics::new(
                    crate::metrics::GooseCoordinatedOmissionMitigation::Average,
                ),
            ),
            ..Default::default()
        };
        metrics.initialize_scenario_metrics(&scenarios, &configuration);
        metrics
            .initialize_transaction_metrics(&scenarios, &configuration, &defaults)
            .unwrap();
        let (tx, rx) = flume::unbounded();
        let (_cmd_tx, cmd_rx) = flume::unbounded();
        let mut processor = MetricsProcessor::new(
            rx.clone(),
            cmd_rx,
            metrics,
            GraphData::for_metrics_processor(true, true, true),
            configuration.clone(),
            scenarios,
            defaults,
            registry.clone(),
            None,
            epoch.clone(),
            true,
        );
        if let Some(worker) = worker {
            processor.gaggle_enable(worker);
        }
        static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
        let client = CLIENT.get_or_init(reqwest::Client::new);
        let users = users
            .map(|ordinal| {
                let scenario = SCENARIO_OF[ordinal % 6];
                let mut user = GooseUser::new(
                    scenario,
                    Arc::from(format!("scenario{scenario}").as_str()),
                    "http://127.0.0.1".parse().unwrap(),
                    &configuration,
                    0,
                    Some(client.clone()),
                )
                .unwrap();
                user.weighted_users_index = ordinal;
                user.is_throttled = false;
                user.metrics_channel = Some(tx.clone());
                user.metrics_batch = Some(GooseMetricBatch::new(0));
                user.metrics_epoch = Some(epoch.clone());
                user.request_counters = Some(registry.clone());
                (ordinal, user)
            })
            .collect();
        Side {
            processor,
            users,
            rx,
        }
    }

    /// Hand every metric sent so far to the processor.
    fn drain(&mut self) {
        while let Ok(metric) = self.rx.try_recv() {
            self.processor.process_metric_for_test(metric);
        }
    }

    /// The `(scenario index, user)` of this side's users running at
    /// `second`.
    fn running(&self, second: usize) -> Vec<(usize, usize)> {
        self.users
            .keys()
            .filter(|&&user| active(user, second))
            .map(|&user| (SCENARIO_OF[user % 6], user))
            .collect()
    }

    /// Play `event`, returning the delta taken at the end of a second.
    fn play(&mut self, event: &Event) -> Option<GaggleDelta> {
        let user = match event {
            Event::Start(user)
            | Event::Request(user, _)
            | Event::Correct(user, _, _)
            | Event::Transaction(user, _, _)
            | Event::Scenario(user, _)
            | Event::Flush(user) => Some(*user),
            Event::Second(_) | Event::Users(_) | Event::Reset(_) => None,
        };
        if user.is_some_and(|user| !self.users.contains_key(&user)) {
            return None;
        }
        let mut delta = None;
        match event {
            Event::Start(user) => {
                self.processor
                    .handle_command_for_test(MetricsCommand::UserStarted {
                        scenario_index: SCENARIO_OF[user % 6],
                        user: *user,
                    });
            }
            Event::Request(user, metric) => {
                let user = self.users.get_mut(user).unwrap();
                user.pending_request = Some(metric.clone());
                user.flush_pending_request().unwrap();
            }
            Event::Correct(user, metric, success) => {
                let user = self.users.get_mut(user).unwrap();
                let mut metric = metric.clone();
                if *success {
                    user.set_success(&mut metric).unwrap();
                } else {
                    let _ = user.set_failure("corrected", &mut metric, None, None);
                }
            }
            Event::Transaction(user, transaction, error) => {
                let user = self.users.get_mut(user).unwrap();
                user.pending_transaction = Some(transaction.clone());
                user.flush_pending_transaction();
                if let Some(error) = error {
                    user.record_transaction_error(transaction, error).unwrap();
                }
            }
            Event::Scenario(user, scenario) => {
                self.users
                    .get_mut(user)
                    .unwrap()
                    .accumulate_scenario(scenario);
            }
            Event::Flush(user) => {
                self.users.get_mut(user).unwrap().flush_batch().unwrap();
            }
            Event::Second(second) => {
                self.drain();
                let active_users = self.running(*second).len().saturating_sub(1);
                self.processor
                    .handle_command_for_test(MetricsCommand::RecordUsers {
                        active_users,
                        elapsed_secs: *second,
                    });
                delta = self.processor.gaggle_take_delta(*second + 1);
            }
            Event::Users(second) => {
                self.processor
                    .handle_command_for_test(MetricsCommand::RecordUsers {
                        active_users: self.running(*second).len(),
                        elapsed_secs: *second,
                    });
            }
            Event::Reset(second) => {
                self.drain();
                let (ack, _ack_rx) = tokio::sync::oneshot::channel();
                self.processor
                    .handle_command_for_test(MetricsCommand::Reset {
                        duration: *second,
                        total_users: USERS,
                        maximum_users: USERS,
                        active_users: self.running(*second).len(),
                        series_elapsed_secs: *second,
                        running_users: self.running(*second),
                        ack,
                    });
            }
        }
        self.drain();
        delta
    }

    /// Flush every user; then the processor's last delta and final delta,
    /// and its metrics and graph data as a run ends.
    fn finish(
        mut self,
    ) -> (
        Option<GaggleDelta>,
        Option<GaggleDelta>,
        GooseMetrics,
        GraphData,
    ) {
        for user in self.users.values_mut() {
            user.flush_batch().unwrap();
        }
        self.drain();
        let last = self.processor.gaggle_take_delta(SECONDS + 2);
        let last_final = self.processor.gaggle_take_final(SECONDS + 2);
        let (respond, mut received) = tokio::sync::oneshot::channel();
        self.processor
            .handle_command_for_test(MetricsCommand::Shutdown {
                duration: SECONDS,
                total_users: USERS,
                maximum_users: USERS,
                respond,
            });
        let (mut metrics, graph) = received.try_recv().unwrap();
        let started = Utc.with_ymd_and_hms(2026, 10, 30, 12, 0, 0).unwrap();
        metrics.history = vec![
            TestPlanHistory {
                action: TestPlanStepAction::Increasing,
                timestamp: started,
                users: 0,
            },
            TestPlanHistory {
                action: TestPlanStepAction::Finished,
                timestamp: started + chrono::Duration::seconds(SECONDS as i64),
                users: 0,
            },
        ];
        (last, last_final, metrics, graph)
    }
}

/// The global ordinals of worker `worker` of `workers`.
fn share(worker: usize, workers: usize) -> std::ops::Range<usize> {
    worker * USERS / workers..(worker + 1) * USERS / workers
}

/// The single process run of `events`.
fn single(events: &[Event]) -> (GooseMetrics, GraphData) {
    let mut side = Side::new(0..USERS, None);
    for event in events {
        side.play(event);
    }
    let (_, _, metrics, graph) = side.finish();
    (metrics, graph)
}

/// Every worker's deltas, in order, each worker's last one final, for
/// `events` played on `workers` workers.
fn worker_deltas(events: &[Event], workers: usize) -> Vec<Vec<GaggleDelta>> {
    let mut sides: Vec<Side> = (0..workers)
        .map(|worker| Side::new(share(worker, workers), Some(worker as u32)))
        .collect();
    let mut deltas: Vec<Vec<GaggleDelta>> = vec![Vec::new(); workers];
    for event in events {
        for (worker, side) in sides.iter_mut().enumerate() {
            if let Some(delta) = side.play(event) {
                deltas[worker].push(delta);
            }
        }
    }
    for (worker, side) in sides.into_iter().enumerate() {
        let (last, last_final, _, _) = side.finish();
        deltas[worker].extend(last);
        deltas[worker].extend(last_final);
    }
    deltas
}

/// `delta` sent as bytes.
fn wire(delta: &GaggleDelta) -> GaggleDelta {
    let bytes = delta.encode().unwrap();
    let decoded = GaggleDelta::decode(&bytes).unwrap();
    assert_eq!(&decoded, delta, "postcard round trip");
    decoded
}

/// A manager's accumulator, empty, as the single process's metrics start.
fn accumulator() -> GaggleAccumulator {
    let side = Side::new(0..0, Some(0));
    GaggleAccumulator::new(
        side.processor.gaggle_template(),
        side.processor.gaggle_graph_settings(),
        DeltaLimits::default(),
    )
}

/// Apply `delta`, sent as bytes, from the worker it names: it must be
/// taken.
fn apply(accumulator: &mut GaggleAccumulator, delta: &GaggleDelta) {
    accumulator.apply(delta.worker, wire(delta)).unwrap();
}

/// Apply every worker's deltas, interleaving workers in a seeded order and
/// keeping each one's own; the final ones only when `finals`, and a worker
/// without its final delta marked done.
fn merge(deltas: &[Vec<GaggleDelta>], finals: bool, seed: u64) -> GaggleAccumulator {
    let mut rng = Rng(seed);
    let mut accumulator = accumulator();
    let mut queues: Vec<std::collections::VecDeque<&GaggleDelta>> = deltas
        .iter()
        .map(|deltas| {
            deltas
                .iter()
                .filter(|delta| finals || delta.kind == DeltaKind::Delta)
                .collect()
        })
        .collect();
    while queues.iter().any(|queue| !queue.is_empty()) {
        let open: Vec<usize> = (0..queues.len())
            .filter(|&queue| !queues[queue].is_empty())
            .collect();
        let queue = open[rng.below(open.len() as u64) as usize];
        let delta = queues[queue].pop_front().unwrap();
        apply(&mut accumulator, delta);
    }
    if !finals {
        for worker in 0..deltas.len() {
            accumulator.done(worker as u32);
        }
    }
    accumulator
}

/// The thresholds every merge is judged by.
fn thresholds() -> Vec<crate::threshold::Threshold> {
    ["p95<2s", "error-rate<5%", "rps>1", "p50<5ms for GET /n001"]
        .iter()
        .map(|rule| rule.parse().unwrap())
        .collect()
}

/// `metrics` with its wall clock timestamps fixed: coordinated omission
/// metrics start at the wall clock second each process started counting,
/// which differs between one process and workers built later (the merge
/// rule and the start after a reset have tests of their own); events carry
/// the wall clock second they were processed in, in the order they were,
/// which within a second differs between one process and its workers.
fn fixed(metrics: &mut GooseMetrics) {
    if let Some(co) = metrics.coordinated_omission_metrics.as_mut() {
        co.started_secs = 0;
        for event in co.co_events.iter_mut() {
            event.timestamp_secs = 0;
        }
        co.co_events.sort_by_key(|event| format!("{event:?}"));
    }
}

/// Every difference between the merged run and the single process's, in
/// everything a run produces from them.
fn differences(
    merged: (GooseMetrics, GraphData),
    single: &(GooseMetrics, GraphData),
) -> Vec<String> {
    let (mut metrics, mut graph) = merged;
    super::compare::adopt_unmerged(&mut metrics, &single.0);
    let mut single = (single.0.clone(), single.1.clone());
    fixed(&mut metrics);
    fixed(&mut single.0);
    let single = &single;
    let mut out = super::compare::metrics_diff(&metrics, &single.0);
    out.extend(graph.gaggle_diff(&single.1));
    let configuration = configuration();
    let record = crate::runs::RunRecord::default();
    let report = |metrics: &GooseMetrics, graph: &GraphData| {
        let source = crate::metrics::ReportSource {
            configuration: &configuration,
            metrics,
            graph_data: graph,
            graphs: true,
            heatmap: None,
            loaded: None,
            record: &record,
            graph_data_in_json: true,
        };
        let mut json = Vec::new();
        source.write_json(&mut json, None).unwrap();
        let mut markdown = Vec::new();
        source.write_markdown(&mut markdown, None).unwrap();
        let html = source.html(None).unwrap();
        let json = String::from_utf8(json).unwrap();
        let loaded = crate::metrics::common::LoadedReport::parse(&json).unwrap();
        let empty = GraphData::new();
        let rendered = loaded.source(&configuration, &empty).html(None).unwrap();
        (json, String::from_utf8(markdown).unwrap(), html, rendered)
    };
    let (a, b) = (report(&metrics, &graph), report(&single.0, &single.1));
    super::json_diff(
        "json report",
        &sorted_co_tables(serde_json::from_str(&a.0).unwrap()),
        &sorted_co_tables(serde_json::from_str(&b.0).unwrap()),
        &mut out,
    );
    super::text_diff(
        "markdown report",
        &sorted_table_rows(&a.1),
        &sorted_table_rows(&b.1),
        &mut out,
    );
    super::text_diff(
        "html report",
        &sorted_html_rows(&a.2),
        &sorted_html_rows(&b.2),
        &mut out,
    );
    super::text_diff(
        "html report rendered from json",
        &sorted_html_rows(&a.3),
        &sorted_html_rows(&b.3),
        &mut out,
    );
    let mut single_graph = single.1.clone();
    super::json_diff(
        "snapshot",
        &crate::metrics::gaggle_snapshot(&configuration, &metrics, &mut graph),
        &crate::metrics::gaggle_snapshot(&configuration, &single.0, &mut single_graph),
        &mut out,
    );
    let rules = thresholds();
    let judge = |metrics: &GooseMetrics| {
        crate::threshold::evaluate(
            &rules,
            metrics,
            crate::threshold::Check::Final,
            None,
            SECONDS as f64,
        )
    };
    let (x, y) = (judge(&metrics), judge(&single.0));
    if x != y {
        out.push(format!("thresholds: {x:?} != {y:?}"));
    }
    out
}

/// `text` with each list of status code counts, `97 [200], 3 [500]`, in
/// sorted order: Goose writes them in the order of a `HashMap`, which
/// differs from one call to the next even in one process.
fn sorted_status_codes(text: &str) -> String {
    static LIST: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let list =
        LIST.get_or_init(|| regex::Regex::new(r"[\d,]+ \[\d+\](, [\d,]+ \[\d+\])+").unwrap());
    list.replace_all(text, |found: &regex::Captures| {
        let mut parts: Vec<&str> = found[0].split(", ").collect();
        parts.sort_unstable();
        parts.join(", ")
    })
    .into_owned()
}

/// The JSON report with what Goose writes in the order of a `HashMap` or
/// `HashSet` sorted: the coordinated omission per user and per scenario
/// event rows, each scenario's users and the status code lists.
fn sorted_co_tables(mut json: serde_json::Value) -> serde_json::Value {
    for table in ["per_user_events", "per_scenario_events"] {
        if let Some(rows) = json
            .pointer_mut(&format!("/coordinated_omission_metrics/{table}"))
            .and_then(|rows| rows.as_array_mut())
        {
            rows.sort_by_key(|row| row.to_string());
        }
    }
    if let Some(scenarios) = json
        .pointer_mut("/raw_metrics/scenarios")
        .and_then(|scenarios| scenarios.as_array_mut())
    {
        for scenario in scenarios {
            if let Some(users) = scenario.get_mut("users").and_then(|u| u.as_array_mut()) {
                users.sort_by_key(|user| user.as_u64());
            }
        }
    }
    // Worked out from the wall clock when the report is written.
    for field in ["duration_secs", "events_per_minute"] {
        if let Some(value) = json.pointer_mut(&format!("/coordinated_omission_metrics/{field}")) {
            *value = serde_json::Value::Null;
        }
    }
    fn strings(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::String(text) => *text = sorted_status_codes(text),
            serde_json::Value::Array(values) => values.iter_mut().for_each(strings),
            serde_json::Value::Object(map) => map.values_mut().for_each(strings),
            _ => {}
        }
    }
    strings(&mut json);
    json
}

/// HTML with the rows of each table body sorted and the status code lists
/// too: the coordinated omission severity table follows a `HashMap`.
fn sorted_html_rows(html: &str) -> String {
    let html = sorted_status_codes(html);
    let body = regex::Regex::new(r"(?s)<tbody>(.*?)</tbody>").unwrap();
    let row = regex::Regex::new(r"(?s)<tr>.*?</tr>").unwrap();
    body.replace_all(&html, |found: &regex::Captures| {
        let mut rows: Vec<&str> = row.find_iter(&found[1]).map(|m| m.as_str()).collect();
        rows.sort_unstable();
        format!("<tbody>{}</tbody>", rows.join("\n"))
    })
    .into_owned()
}

/// Markdown with the rows of each table sorted and the status code lists
/// too, for the same reason as [`sorted_co_tables`].
fn sorted_table_rows(markdown: &str) -> String {
    let markdown = sorted_status_codes(markdown);
    let mut out = Vec::new();
    let mut rows: Vec<&str> = Vec::new();
    for line in markdown.lines() {
        if line.starts_with('|') && !line.starts_with("| -") && !line.starts_with("|-") {
            rows.push(line);
            continue;
        }
        rows.sort_unstable();
        out.append(&mut rows);
        out.push(line);
    }
    rows.sort_unstable();
    out.append(&mut rows);
    out.join("\n")
}

/// Fail with every difference, the first 40 shown.
fn assert_none(what: &str, differences: &[String]) {
    assert!(
        differences.is_empty(),
        "{what}: {} differences:\n{}",
        differences.len(),
        differences
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// B1, the core equivalence: `workers` workers' deltas merged give what one
/// process gives, in every figure, graph, report, snapshot and threshold
/// verdict, with and without final deltas, whatever order the workers'
/// deltas interleave in.
fn merge_equals_one_process(workers: usize) {
    let events = events(0x645);
    let single = single(&events);
    assert!(single.0.requests.len() >= NAMES);
    let deltas = worker_deltas(&events, workers);
    for finals in [false, true] {
        let accumulator = merge(&deltas, finals, workers as u64);
        assert!(accumulator.gaps().is_empty(), "{:?}", accumulator.gaps());
        assert_none(
            &format!("{workers} workers, finals {finals}"),
            &differences(accumulator.merged(), &single),
        );
    }
}

#[test]
fn gaggle_merge_equals_one_process_1_worker() {
    merge_equals_one_process(1);
}

#[test]
fn gaggle_merge_equals_one_process_2_workers() {
    merge_equals_one_process(2);
}

#[test]
fn gaggle_merge_equals_one_process_3_workers() {
    merge_equals_one_process(3);
}

#[test]
fn gaggle_merge_equals_one_process_7_workers() {
    merge_equals_one_process(7);
}

/// Every difference between two merges.
fn merge_diff(a: &GaggleAccumulator, b: &GaggleAccumulator) -> Vec<String> {
    let ((mut x, g), (mut y, h)) = (a.merged(), b.merged());
    // Each template starts its coordinated omission metrics at the wall
    // clock second it was built: two built a second apart differ only there.
    for metrics in [&mut x, &mut y].iter_mut() {
        if let Some(co) = metrics.coordinated_omission_metrics.as_mut() {
            co.started_secs = 0;
        }
    }
    let mut out = super::compare::metrics_diff(&x, &y);
    out.extend(g.gaggle_diff(&h));
    out
}

/// Apply `deltas`, in order, to a new accumulator.
fn apply_all<'a>(deltas: impl IntoIterator<Item = &'a GaggleDelta>) -> GaggleAccumulator {
    let mut accumulator = accumulator();
    for delta in deltas {
        apply(&mut accumulator, delta);
    }
    accumulator
}

/// B3: a delta lost in the middle: the worker's final delta makes its
/// metrics good, not its graph rows, which stay missing exactly as if
/// the delta had carried none, and the gap is reported.
#[test]
fn gaggle_lost_delta_then_final_repairs_metrics_only() {
    let events = events(0x645);
    let single = single(&events);
    let mut deltas = worker_deltas(&events, 3);
    // One past the reset, with graph rows, so the final's epoch holds it.
    let epoch = deltas[1].last().unwrap().epoch;
    let at = deltas[1]
        .iter()
        .position(|delta| {
            delta.kind == DeltaKind::Delta
                && delta.epoch == epoch
                && !delta.graph.requests.is_empty()
        })
        .expect("a delta with graph rows after the reset");
    let lost = deltas[1].remove(at);
    let accumulator = apply_all(deltas.iter().flatten());
    assert_eq!(
        accumulator.gaps(),
        &[super::merge::SeqGap {
            worker: 1,
            expected: lost.seq,
            received: lost.seq + 1,
        }]
    );
    let (mut metrics, graph) = accumulator.merged();
    super::compare::adopt_unmerged(&mut metrics, &single.0);
    let mut expected = single.0.clone();
    fixed(&mut metrics);
    fixed(&mut expected);
    assert_none(
        "lost delta, then final: metrics",
        &super::compare::metrics_diff(&metrics, &expected),
    );
    assert!(
        !graph.gaggle_diff(&single.1).is_empty(),
        "the lost delta's graph rows came back"
    );
    // The graph is the deltas' with the lost one's rate rows left out.
    let mut emptied = lost.clone();
    emptied.graph = super::delta::GraphPart {
        users: lost.graph.users.clone(),
        ..Default::default()
    };
    deltas[1].insert(at, emptied);
    let (_, expected) = apply_all(deltas.iter().flatten()).merged();
    assert_none(
        "lost delta, then final: graph",
        &graph.gaggle_diff(&expected),
    );
}

/// A worker keeps nothing of what it sent: after any number of deltas its
/// final delta carries only the graph rows pending since the last one, so
/// neither its memory nor its final delta grows with the run.
#[test]
fn gaggle_worker_keeps_no_sent_rows() {
    let events = events(0x645);
    let mut side = Side::new(share(0, 2), Some(0));
    let mut sent = 0;
    for event in &events {
        sent += side.play(event).iter().count();
    }
    assert!(sent > 20, "{} deltas: too few to show growth", sent);
    side.drain();
    side.processor.gaggle_take_delta(SECONDS + 2).unwrap();
    let last_final = side.processor.gaggle_take_final(SECONDS + 2).unwrap();
    let graph = &last_final.graph;
    let rows = graph.requests.values().map(BTreeMap::len).sum::<usize>()
        + graph.errors.values().map(BTreeMap::len).sum::<usize>()
        + graph.averages.values().map(BTreeMap::len).sum::<usize>()
        + graph.latency.values().map(BTreeMap::len).sum::<usize>()
        + graph.synthetic.values().map(BTreeMap::len).sum::<usize>()
        + graph.transactions.len()
        + graph.scenarios.len();
    assert_eq!(rows, 0, "the final delta resent rows");
    assert!(!last_final.metrics.requests.is_empty());
}

/// B3: a worker lost with its last deltas and no final delta: what it sent
/// stays merged, the loss is reported, and its seconds stop holding back
/// which seconds are complete.
#[test]
fn gaggle_lost_worker_keeps_what_it_sent() {
    let events = events(0x645);
    let mut deltas = worker_deltas(&events, 3);
    // Worker 1 sends nothing after its sixteenth delta, past the reset.
    deltas[1].truncate(16);
    let others = |deltas: &[Vec<GaggleDelta>]| -> Vec<GaggleDelta> {
        deltas
            .iter()
            .enumerate()
            .flat_map(|(worker, deltas)| {
                deltas
                    .iter()
                    .filter(move |delta| worker == 1 || delta.kind == DeltaKind::Delta)
                    .cloned()
            })
            .collect()
    };
    let received = others(&deltas);
    let mut accumulator = apply_all(&received);
    // Worker 1's last delta holds back completion until it is lost.
    let held = deltas[1].last().unwrap().now.saturating_sub(1);
    assert_eq!(accumulator.complete_before(), Some(held));
    accumulator.done(0);
    accumulator.done(2);
    assert_eq!(accumulator.complete_before(), Some(held));
    accumulator.lose(1);
    assert_eq!(accumulator.complete_before(), None);
    assert_eq!(
        accumulator.gaps(),
        &[super::merge::SeqGap {
            worker: 1,
            expected: 17,
            received: 0,
        }]
    );
    // The same deltas, every worker done: the merge of what was received.
    let mut expected = apply_all(&received);
    for worker in 0..3 {
        expected.done(worker);
    }
    assert_none("lost worker", &merge_diff(&accumulator, &expected));
    // And every request worker 1 counted in what it sent is there.
    let epoch = deltas[1].last().unwrap().epoch;
    let sent: i64 = deltas[1]
        .iter()
        .filter(|delta| delta.epoch == epoch)
        .flat_map(|delta| delta.metrics.requests.values())
        .map(|request| request.success + request.fail)
        .sum();
    let (metrics, _) = accumulator.merged();
    let (without, _) = {
        let mut without = apply_all(received.iter().filter(|delta| delta.worker != 1));
        without.done(0);
        without.done(2);
        without.merged()
    };
    let count = |metrics: &GooseMetrics| -> i64 {
        metrics
            .requests
            .values()
            .map(|request| (request.success_count + request.fail_count) as i64)
            .sum()
    };
    assert!(sent > 0);
    assert_eq!(count(&metrics), count(&without) + sent);
}

/// B3: a delta from an epoch before the worker's last metrics reset is
/// discarded.
#[test]
fn gaggle_older_epoch_is_discarded() {
    let events = events(0x645);
    let deltas = worker_deltas(&events, 2);
    // No final delta: nothing may follow one.
    let sent = || {
        deltas
            .iter()
            .flatten()
            .filter(|delta| delta.kind == DeltaKind::Delta)
    };
    let mut accumulator = apply_all(sent());
    let before = apply_all(sent());
    let newest = deltas[0].last().unwrap().epoch;
    let mut stale = deltas[0]
        .iter()
        .find(|delta| delta.epoch < newest && !delta.metrics.requests.is_empty())
        .cloned()
        .expect("a delta from before the reset");
    stale.seq = 1_000;
    stale.first_seq = 1_000;
    stale.kind = DeltaKind::Delta;
    apply(&mut accumulator, &stale);
    assert_none("older epoch", &merge_diff(&accumulator, &before));
    assert!(accumulator.gaps().is_empty());
}

/// B4: applying two consecutive deltas coalesced equals applying them one
/// after the other, for pairs across the run, across the metrics reset and
/// ending in a final delta.
#[test]
fn gaggle_coalesce_equals_apply_apply() {
    let events = events(0x645);
    let deltas = worker_deltas(&events, 3);
    let mut rng = Rng(0xc0a1);
    let reset = deltas[0]
        .windows(2)
        .position(|pair| pair[0].epoch != pair[1].epoch)
        .expect("a reset");
    let last = deltas[0].len() - 2;
    let mut pairs: Vec<(usize, usize)> = vec![(0, reset), (0, last), (2, 0)];
    for _ in 0..8 {
        let worker = rng.below(3) as usize;
        let at = rng.below(deltas[worker].len() as u64 - 1) as usize;
        pairs.push((worker, at));
    }
    let expected = apply_all(deltas.iter().flatten());
    for (worker, at) in pairs {
        let joined = coalesce(&deltas[worker][at], &deltas[worker][at + 1]).unwrap();
        let replaced: Vec<GaggleDelta> = deltas
            .iter()
            .enumerate()
            .flat_map(|(w, deltas)| {
                let mut deltas = deltas.clone();
                if w == worker {
                    deltas.splice(at..at + 2, [joined.clone()]);
                }
                deltas
            })
            .collect();
        let accumulator = apply_all(&replaced);
        assert!(accumulator.gaps().is_empty(), "{:?}", accumulator.gaps());
        assert_none(
            &format!("coalesced worker {worker} deltas {at} and {}", at + 1),
            &merge_diff(&accumulator, &expected),
        );
    }
}

/// A delta from `worker` of `epoch`, numbered `seq`, counting one success
/// of `GET /a`.
fn counted(worker: u32, seq: u64, epoch: u64) -> GaggleDelta {
    let mut delta = GaggleDelta::new(worker, seq, epoch, DeltaKind::Delta);
    delta.now = 5;
    let mut request = super::delta::RequestPart::new("/a", GooseMethod::Get, "");
    request.success = 1;
    delta.metrics.requests.insert("GET /a".to_string(), request);
    delta
}

/// A worker that missed a metrics reset another worker saw is left out of
/// the merge, and named.
#[test]
fn gaggle_older_epoch_worker_is_named() {
    let mut accumulator = accumulator();
    apply(&mut accumulator, &counted(0, 1, 1));
    apply(&mut accumulator, &counted(1, 1, 1));
    assert!(accumulator.excluded().is_empty());
    apply(&mut accumulator, &counted(0, 2, 2));
    assert_eq!(accumulator.excluded(), vec![1]);
    let (metrics, _) = accumulator.merged();
    assert_eq!(metrics.requests["GET /a"].success_count, 1);
}

/// The shadow merge fails, naming the worker, when one is left out.
#[test]
fn gaggle_shadow_merge_fails_on_an_excluded_worker() {
    let side = Side::new(0..0, Some(0));
    let outcome = Arc::new(Mutex::new(None));
    let mut shadow = super::shadow::ShadowMerge::new(
        side.processor.gaggle_template(),
        side.processor.gaggle_graph_settings(),
        outcome,
    );
    shadow.delta(&counted(1, 1, 1));
    shadow.delta(&counted(0, 1, 2));
    let mut last_final = counted(0, 3, 2);
    last_final.kind = DeltaKind::Final;
    let run = single(&[]);
    let ended = shadow.finish(&counted(0, 2, 2), &last_final, (&run.0, &run.1), None);
    for name in ["deltas", "final"] {
        let named = format!("{name}: worker 1 left out of the merge");
        assert!(
            ended.differences.iter().any(|d| d.starts_with(&named)),
            "{:?}",
            ended.differences
        );
    }
}

/// A worker's users level holds past its last second: two workers, one
/// whose users stop being recorded a second early, add up as one process
/// keeping each level until it changes.
#[test]
fn gaggle_users_levels_hold() {
    let delta = |worker: u32, users: &[(u64, u64)]| {
        let mut delta = GaggleDelta::new(worker, 1, 1, DeltaKind::Delta);
        delta.now = 10;
        delta.graph.users = users.iter().copied().collect();
        delta
    };
    let mut accumulator = accumulator();
    apply(&mut accumulator, &delta(0, &[(0, 2), (1, 3)]));
    apply(&mut accumulator, &delta(1, &[(0, 1), (1, 1), (2, 4)]));
    accumulator.done(0);
    accumulator.done(1);
    let (_, graph) = accumulator.merged();
    let mut expected = GraphData::for_metrics_processor(true, true, true);
    for (second, users) in [(0, 3), (1, 4), (2, 7)] {
        expected.record_users_per_second(users, second);
    }
    expected.finish_latency_columns();
    assert_none("users levels", &graph.gaggle_diff(&expected));
}

/// B3: a delta received twice is reported and counted once.
#[test]
fn gaggle_repeated_delta_is_ignored() {
    let events = events(0x645);
    let deltas: Vec<GaggleDelta> = worker_deltas(&events, 2)
        .into_iter()
        .flatten()
        .filter(|delta| delta.kind == DeltaKind::Delta)
        .collect();
    // One from the last epoch, so the reset does not discard what it adds.
    let epoch = deltas.iter().map(|delta| delta.epoch).max().unwrap();
    let at = deltas
        .iter()
        .position(|delta| delta.epoch == epoch && !delta.metrics.requests.is_empty())
        .expect("a delta with requests");
    let mut repeated = deltas.clone();
    repeated.insert(at + 1, deltas[at].clone());
    let mut expected = apply_all(&deltas);
    let mut accumulator = apply_all(&repeated);
    for worker in 0..2 {
        expected.done(worker);
        accumulator.done(worker);
    }
    assert_eq!(
        accumulator.gaps(),
        &[super::merge::SeqGap {
            worker: deltas[at].worker,
            expected: deltas[at].seq + 1,
            received: deltas[at].first_seq,
        }]
    );
    assert_none("repeated delta", &merge_diff(&accumulator, &expected));
}

/// A final delta carries the batches that arrived after the worker's last
/// delta: without it they are missing, with it the merge is exact.
#[test]
fn gaggle_final_carries_what_followed_the_last_delta() {
    let events = events(0x645);
    let single = single(&events);
    let workers = 3;
    let mut sides: Vec<Side> = (0..workers)
        .map(|worker| Side::new(share(worker, workers), Some(worker as u32)))
        .collect();
    let mut deltas: Vec<Vec<GaggleDelta>> = vec![Vec::new(); workers];
    for event in &events {
        for (worker, side) in sides.iter_mut().enumerate() {
            deltas[worker].extend(side.play(event));
        }
    }
    for (worker, mut side) in sides.into_iter().enumerate() {
        // The last delta goes before the users' last batches are sent,
        // and the final delta right after them, with no delta between.
        side.drain();
        deltas[worker].extend(side.processor.gaggle_take_delta(SECONDS + 2));
        for user in side.users.values_mut() {
            user.flush_batch().unwrap();
        }
        side.drain();
        deltas[worker].extend(side.processor.gaggle_take_final(SECONDS + 2));
    }
    assert!(
        !differences(merge(&deltas, false, 1).merged(), &single).is_empty(),
        "no batch arrived after the last delta: this test proves nothing"
    );
    assert_none(
        "final after late batches",
        &differences(merge(&deltas, true, 1).merged(), &single),
    );
}

/// The pending graph data trails its delta by the lag, and a metric from
/// further back moves its start back: with no lag at all, and with one
/// shorter than the run, which moves the start past 0 before and after the
/// reset, the merge stays exact.
#[test]
fn gaggle_pending_graph_keeps_late_metrics() {
    let events = events(0x645);
    let single = single(&events);
    for lag in [0, 3] {
        let workers = 3;
        let mut sides: Vec<Side> = (0..workers)
            .map(|worker| {
                let mut side = Side::new(share(worker, workers), Some(worker as u32));
                side.processor.gaggle_set_lag_for_test(lag);
                side
            })
            .collect();
        let mut deltas: Vec<Vec<GaggleDelta>> = vec![Vec::new(); workers];
        for event in &events {
            for (worker, side) in sides.iter_mut().enumerate() {
                deltas[worker].extend(side.play(event));
            }
        }
        for (worker, side) in sides.into_iter().enumerate() {
            let (last, last_final, _, _) = side.finish();
            deltas[worker].extend(last);
            deltas[worker].extend(last_final);
        }
        for finals in [false, true] {
            assert_none(
                &format!("lag {lag}, finals {finals}"),
                &differences(merge(&deltas, finals, 1).merged(), &single),
            );
        }
    }
}

/// After a metrics reset, deltas carry the coordinated omission start the
/// processor reset to, not the template's.
#[test]
fn gaggle_reset_takes_the_processors_co_start() {
    let template = GooseMetrics {
        coordinated_omission_metrics: Some(
            crate::metrics::coordinated_omission::CoordinatedOmissionMetrics {
                started_secs: 5,
                ..crate::metrics::coordinated_omission::CoordinatedOmissionMetrics::new(
                    crate::metrics::GooseCoordinatedOmissionMitigation::Average,
                )
            },
        ),
        ..Default::default()
    };
    let registry: GooseRequestCounterRegistry = Arc::new(Mutex::new(HashMap::new()));
    let graph = GraphData::for_metrics_processor(true, true, true);
    let mut state = super::worker::WorkerState::new(0, template.clone(), true, 120);
    state.reset(10, &[], Some(99));
    for now in [11, 12] {
        let delta = state.take_delta((&template, &graph), &registry, 1, now);
        assert_eq!(
            delta.metrics.co.as_ref().map(|co| co.started_secs),
            Some(99),
            "second {now}"
        );
    }
}

/// Response times in one delta whose seconds share a heatmap ring slot, 300
/// seconds apart: measured seconds 5 and 305, one packed out of the slot as
/// 305 takes it and one arriving after, and a stall that mitigation fills
/// with synthetic times from second 3 to 303. Every one reaches the manager,
/// so its report heatmap's columns equal one process's.
#[test]
fn gaggle_seconds_a_ring_apart_reach_the_manager() {
    let started = request(0, &name(1), GooseMethod::Get, 2_000, 900, 200);
    // Second 5 shares second 305's ring slot: `early` packs out of it as
    // 305 begins, `late` arrives after 305 holds it.
    let early = request(0, &name(1), GooseMethod::Get, 5_200, 1_100, 200);
    let late = request(0, &name(1), GooseMethod::Get, 5_500, 1_200, 200);
    let stalled = request(0, &name(1), GooseMethod::Get, 305_000, 800, 200);
    let mut copy = stalled.clone();
    // Synthetic seconds 3 to 303: 3 and 303 share a slot.
    copy.coordinated_omission_elapsed = 303_500;
    copy.user_cadence = 1_000;
    let events = vec![
        Event::Start(0),
        Event::Request(0, started),
        Event::Flush(0),
        Event::Second(2),
        Event::Request(0, early),
        Event::Flush(0),
        Event::Request(0, stalled),
        Event::Request(0, copy),
        Event::Flush(0),
        // Measured second 5 again, after 305 took its slot, in the same delta.
        Event::Request(0, late),
        Event::Flush(0),
        Event::Second(305),
    ];
    let single = single(&events);
    let deltas = worker_deltas(&events, 1);
    // The delta of second 305 holds each pair of seconds that share a slot.
    let graph = &deltas[0][1].graph;
    let measured = &graph.latency["GET /n001"];
    assert!(
        measured.contains_key(&5) && measured.contains_key(&305),
        "{:?}",
        measured
    );
    let synthetic = &graph.synthetic["GET /n001"];
    assert!(
        synthetic.contains_key(&3) && synthetic.contains_key(&303),
        "{:?}",
        synthetic
    );
    for finals in [false, true] {
        assert_none(
            &format!("finals {finals}"),
            &differences(merge(&deltas, finals, 1).merged(), &single),
        );
    }
}

/// Coordinated omission metrics started when the earliest worker's did.
#[test]
fn gaggle_co_start_is_the_earliest_workers() {
    let mut accumulator = accumulator();
    for (worker, started_secs) in [(0u32, 200u64), (1, 100), (2, 300)] {
        let mut delta = GaggleDelta::new(worker, 1, 1, DeltaKind::Delta);
        delta.metrics.co = Some(super::delta::CoPart {
            strategy: crate::metrics::GooseCoordinatedOmissionMitigation::Average,
            started_secs,
            actual: 0,
            synthetic: 0,
            severity: BTreeMap::new(),
            events: Vec::new(),
            events_dropped: 0,
        });
        apply(&mut accumulator, &delta);
    }
    let (metrics, _) = accumulator.merged();
    assert_eq!(
        metrics
            .coordinated_omission_metrics
            .map(|co| co.started_secs),
        Some(100)
    );
}

/// The shadow merge in the processor: a delta each second, then the last
/// and final ones at shutdown, and a match counting every delta sent.
#[test]
fn gaggle_shadow_merge_counts_every_second() {
    let events = events(0x645);
    let mut side = Side::new(0..USERS, None);
    let outcome = Arc::new(Mutex::new(None));
    side.processor.gaggle_enable_shadow(outcome.clone());
    let mut ticks = 0;
    for event in &events {
        if let Event::Second(second) = event {
            side.drain();
            side.processor
                .handle_command_for_test(MetricsCommand::RecordUsers {
                    active_users: side.running(*second).len().saturating_sub(1),
                    elapsed_secs: *second,
                });
            side.processor
                .handle_command_for_test(MetricsCommand::GaggleShadowTick { now: *second + 1 });
            ticks += 1;
        } else {
            side.play(event);
        }
    }
    for user in side.users.values_mut() {
        user.flush_batch().unwrap();
    }
    side.drain();
    let (respond, _received) = tokio::sync::oneshot::channel();
    side.processor
        .handle_command_for_test(MetricsCommand::Shutdown {
            duration: SECONDS,
            total_users: USERS,
            maximum_users: USERS,
            respond,
        });
    let outcome = outcome
        .lock()
        .unwrap()
        .take()
        .expect("the shadow merge finished");
    assert_none("shadow merge", &outcome.differences);
    assert_eq!(outcome.deltas, ticks + 2);
}

/// The limits the refusal tests use: 100 seconds, 3 names and 4 workers.
const SMALL: DeltaLimits = DeltaLimits {
    max_second: 100,
    max_names: 3,
    max_users: 8,
    max_workers: 4,
};

/// An accumulator with [`SMALL`] limits holding worker 0's first delta,
/// and worker 1's first and final ones.
fn held() -> GaggleAccumulator {
    let side = Side::new(0..0, Some(0));
    let mut accumulator = GaggleAccumulator::new(
        side.processor.gaggle_template(),
        side.processor.gaggle_graph_settings(),
        SMALL,
    );
    let mut first = counted(0, 1, 1);
    first
        .graph
        .requests
        .insert("GET /a".to_string(), [(4, 1)].into());
    let mut last_final = counted(1, 2, 1);
    last_final.kind = DeltaKind::Final;
    for delta in [first, counted(1, 1, 1), last_final].iter() {
        accumulator.apply(delta.worker, wire(delta)).unwrap();
    }
    accumulator
}

/// `delta` from `from` is refused with `error`, and the accumulator merges,
/// leaves out and reports gaps exactly as before.
fn refused(from: u32, delta: GaggleDelta, error: DeltaError) {
    let mut accumulator = held();
    let mut expected = held();
    let gaps = accumulator.gaps().to_vec();
    assert_eq!(accumulator.apply(from, delta), Err(error));
    assert_eq!(accumulator.gaps(), &gaps[..]);
    assert!(accumulator.excluded().is_empty());
    assert_eq!(accumulator.complete_before(), expected.complete_before());
    // Worker 0's sequence and held names are where they were: its next
    // delta follows on and fills its names to the limit exactly.
    let mut next = counted(0, 2, 1);
    for name in ["GET /x", "GET /y"].iter() {
        next.graph
            .requests
            .insert(name.to_string(), [(3, 1)].into());
    }
    for accumulator in [&mut accumulator, &mut expected].iter_mut() {
        accumulator.apply(0, next.clone()).unwrap();
        // Every second complete, so every graph row compares.
        accumulator.done(0);
    }
    assert_eq!(accumulator.gaps(), &gaps[..]);
    assert_none("refused delta", &merge_diff(&accumulator, &expected));
}

#[test]
fn gaggle_refuses_another_workers_delta() {
    refused(
        0,
        counted(1, 3, 1),
        DeltaError::WrongWorker { from: 0, named: 1 },
    );
}

#[test]
fn gaggle_refuses_a_worker_out_of_range() {
    refused(
        4,
        counted(4, 1, 1),
        DeltaError::WorkerOutOfRange {
            worker: 4,
            limit: 4,
        },
    );
}

#[test]
fn gaggle_refuses_a_backward_sequence() {
    let mut delta = counted(0, 2, 1);
    delta.first_seq = 3;
    refused(
        0,
        delta,
        DeltaError::BadSequence {
            first_seq: 3,
            seq: 2,
        },
    );
}

#[test]
fn gaggle_refuses_the_last_sequence_number() {
    let mut delta = counted(0, u64::MAX, 1);
    delta.first_seq = 2;
    refused(
        0,
        delta,
        DeltaError::BadSequence {
            first_seq: 2,
            seq: u64::MAX,
        },
    );
}

#[test]
fn gaggle_refuses_a_delta_after_the_final() {
    refused(1, counted(1, 3, 1), DeltaError::AfterFinal { seq: 3 });
    // A repeat of the final too, and one of a newer epoch.
    let mut repeat = counted(1, 2, 1);
    repeat.kind = DeltaKind::Final;
    refused(1, repeat, DeltaError::AfterFinal { seq: 2 });
    refused(1, counted(1, 3, 2), DeltaError::AfterFinal { seq: 3 });
}

#[test]
fn gaggle_refuses_a_partly_applied_range() {
    let mut delta = counted(0, 3, 1);
    delta.first_seq = 1;
    refused(
        0,
        delta,
        DeltaError::Overlap {
            first_seq: 1,
            seq: 3,
            held: 1,
        },
    );
}

#[test]
fn gaggle_refuses_a_second_past_its_clock() {
    let mut delta = counted(0, 2, 1);
    delta.graph.latency.insert(
        "GET /a".to_string(),
        [(6, super::delta::Histogram::default())].into(),
    );
    refused(
        0,
        delta,
        DeltaError::SecondOutOfRange {
            second: 6,
            limit: 5,
        },
    );
    let mut delta = counted(0, 2, 1);
    delta.series_origin = Some(9);
    refused(
        0,
        delta,
        DeltaError::SecondOutOfRange {
            second: 9,
            limit: 5,
        },
    );
}

#[test]
fn gaggle_refuses_a_clock_past_the_limit() {
    // Near 2^63, where widening the heatmap's columns used to loop for
    // ever, and far enough out to make a series allocate without bound.
    for now in [101, 1 << 62, u64::MAX] {
        let mut delta = counted(0, 2, 1);
        delta.now = now;
        delta.graph.users.insert(now, 1);
        refused(
            0,
            delta,
            DeltaError::SecondOutOfRange {
                second: now,
                limit: 100,
            },
        );
    }
}

#[test]
fn gaggle_refuses_a_bad_float() {
    for sum_ms in [f64::NAN, f64::INFINITY, -1.0] {
        let mut delta = counted(0, 2, 1);
        let average = super::delta::Average { count: 1, sum_ms };
        delta
            .graph
            .averages
            .insert("GET /a".to_string(), [(4, average)].into());
        refused(0, delta, DeltaError::BadFloat);
    }
}

#[test]
fn gaggle_refuses_too_many_names() {
    let named = |names: std::ops::Range<usize>| {
        let mut delta = counted(0, 2, 1);
        for index in names {
            delta
                .graph
                .requests
                .insert(format!("GET /{index}"), [(4, 1)].into());
        }
        delta
    };
    // Four in one delta, past the limit of three.
    refused(
        0,
        named(0..4),
        DeltaError::TooManyNames { names: 4, limit: 3 },
    );
    // Three in a delta, and the one held.
    refused(
        0,
        named(5..8),
        DeltaError::TooManyNames { names: 4, limit: 3 },
    );
    // Two held, then two more.
    let mut accumulator = held();
    accumulator.apply(0, named(0..1)).unwrap();
    let mut delta = named(5..7);
    delta.seq = 3;
    delta.first_seq = 3;
    assert_eq!(
        accumulator.apply(0, delta),
        Err(DeltaError::TooManyNames { names: 4, limit: 3 })
    );
}

#[test]
fn gaggle_refuses_a_bad_bucket() {
    let mut unsorted = counted(0, 2, 1);
    let request = unsorted.metrics.requests.get_mut("GET /a").unwrap();
    request.raw.buckets = vec![(2_000, 1), (1_000, 1)];
    refused(0, unsorted, DeltaError::BadBucket);
    let mut between = counted(0, 2, 1);
    let request = between.metrics.requests.get_mut("GET /a").unwrap();
    // Not where a timing data bucket starts.
    request.raw.buckets = vec![(1_000_001, 1)];
    refused(0, between, DeltaError::BadBucket);
    let mut row = counted(0, 2, 1);
    let histogram = super::delta::Histogram {
        rows: vec![(crate::graph::gaggle_max_row() + 1, 1)],
        count: 1,
        ..Default::default()
    };
    row.graph
        .latency
        .insert("GET /a".to_string(), [(4, histogram)].into());
    refused(0, row, DeltaError::BadBucket);
}

#[test]
fn gaggle_refuses_an_overflow() {
    use super::delta::{MAX_COUNT, MAX_SECOND_COUNT};
    // One count past the bound on its own, and in a later field, so a
    // refusal after other fields were checked still changes nothing.
    let mut alone = counted(0, 2, 1);
    alone.metrics.requests.get_mut("GET /a").unwrap().fail = -(MAX_COUNT as i64) - 1;
    refused(0, alone, DeltaError::Overflow);
    let mut second = counted(0, 2, 1);
    second
        .graph
        .requests
        .insert("GET /a".to_string(), [(4, MAX_SECOND_COUNT + 1)].into());
    refused(0, second, DeltaError::Overflow);
    // Within bounds alone, past them added to what is held: worker 0 holds
    // one success and one request at second 4.
    // With figures added before the one past its bound, which must not be.
    let mut sum = counted(0, 2, 1);
    let request = sum.metrics.requests.get_mut("GET /a").unwrap();
    request.success = MAX_COUNT as i64;
    request.status_codes.insert(200, 1);
    request.raw = super::delta::Timing {
        buckets: vec![(1_000, 1)],
        minimum_us: 1_000,
        maximum_us: 1_000,
        total_us: 1_000,
        count: 1,
    };
    sum.graph
        .requests
        .insert("GET /a".to_string(), [(4, 1)].into());
    refused(0, sum, DeltaError::Overflow);
    let mut rows = counted(0, 2, 1);
    rows.graph
        .requests
        .insert("GET /a".to_string(), [(4, MAX_SECOND_COUNT)].into());
    refused(0, rows, DeltaError::Overflow);
    let mut users = counted(0, 2, 1);
    users.graph.users.insert(4, MAX_COUNT + 1);
    refused(0, users, DeltaError::Overflow);
}

/// Coalescing refuses a sum out of bounds too, rather than wrapping.
#[test]
fn gaggle_coalesce_refuses_an_overflow() {
    let mut a = counted(0, 1, 1);
    a.metrics.requests.get_mut("GET /a").unwrap().success = super::delta::MAX_COUNT as i64;
    assert_eq!(coalesce(&a, &counted(0, 2, 1)), Err(DeltaError::Overflow));
}

/// A new worker's refused first delta leaves no trace of the worker.
#[test]
fn gaggle_refuses_a_new_workers_first_delta() {
    let mut delta = counted(2, 1, 1);
    let average = super::delta::Average {
        count: 1,
        sum_ms: f64::NAN,
    };
    delta
        .graph
        .averages
        .insert("GET /a".to_string(), [(4, average)].into());
    refused(2, delta, DeltaError::BadFloat);
}

/// A refused final delta leaves its worker unfinished and its metrics held.
#[test]
fn gaggle_refuses_an_overflow_in_a_final() {
    let mut last_final = counted(0, 2, 1);
    last_final.kind = DeltaKind::Final;
    last_final.graph.requests.insert(
        "GET /a".to_string(),
        [(4, super::delta::MAX_SECOND_COUNT)].into(),
    );
    refused(0, last_final, DeltaError::Overflow);
}

/// Every kind of name counts toward the limit, hosts included.
#[test]
fn gaggle_refuses_too_many_names_of_any_kind() {
    use super::delta::{
        Average, ErrorKey, Histogram, RequestPart, ScenarioPart, TransactionErrorKey,
        TransactionPart,
    };
    type Add = fn(&mut GaggleDelta, usize);
    let kinds: [(&str, Add); 11] = [
        ("requests", |d, i| {
            let name = format!("GET /{}", i);
            let part = RequestPart::new(&name, GooseMethod::Get, "");
            d.metrics.requests.insert(name, part);
        }),
        ("transactions", |d, i| {
            d.metrics
                .transactions
                .insert((0, i as u64), TransactionPart::default());
        }),
        ("scenarios", |d, i| {
            d.metrics
                .scenarios
                .insert(i as u64, ScenarioPart::default());
        }),
        ("errors", |d, i| {
            let key = ErrorKey {
                key: format!("e{}", i),
                method: GooseMethod::Get,
                name: "/a".to_string(),
                error: "500".to_string(),
                custom_method: String::new(),
            };
            d.metrics.errors.insert(key, 1);
        }),
        ("transaction errors", |d, i| {
            let key = TransactionErrorKey {
                key: format!("t{}", i),
                scenario_index: 0,
                scenario_name: "s".to_string(),
                transaction_index: 0,
                transaction_name: "t".to_string(),
                error: "e".to_string(),
            };
            d.metrics.transaction_errors.insert(key, 1);
        }),
        ("hosts", |d, i| {
            d.metrics.hosts.insert(format!("http://host{}", i));
        }),
        ("graph requests", |d, i| {
            d.graph
                .requests
                .insert(format!("GET /{}", i), [(4, 1)].into());
        }),
        ("graph errors", |d, i| {
            d.graph
                .errors
                .insert(format!("GET /{}", i), [(4, 1)].into());
        }),
        ("averages", |d, i| {
            let average = Average {
                count: 1,
                sum_ms: 1.0,
            };
            d.graph
                .averages
                .insert(format!("GET /{}", i), [(4, average)].into());
        }),
        ("latency", |d, i| {
            d.graph
                .latency
                .insert(format!("GET /{}", i), [(4, Histogram::default())].into());
        }),
        ("synthetic", |d, i| {
            d.graph
                .synthetic
                .insert(format!("GET /{}", i), [(4, Histogram::default())].into());
        }),
    ];
    for (kind, add) in kinds.iter() {
        let mut delta = counted(0, 2, 1);
        delta.metrics.requests.clear();
        for index in 0..4 {
            add(&mut delta, index);
        }
        let mut accumulator = held();
        assert_eq!(
            accumulator.apply(0, delta.clone()),
            Err(DeltaError::TooManyNames { names: 4, limit: 3 }),
            "{}",
            kind
        );
        refused(0, delta, DeltaError::TooManyNames { names: 4, limit: 3 });
    }
}

/// A worker cannot grow a scenario's users past the cap: every ordinal is
/// below it, so the set holds at most that many, however many deltas come.
#[test]
fn gaggle_refuses_a_user_past_the_cap() {
    let with_user = |seq: u64, user: u64| {
        let mut delta = counted(0, seq, 1);
        let mut scenario = super::delta::ScenarioPart::default();
        scenario.users.insert(user);
        delta.metrics.scenarios.insert(0, scenario);
        delta
    };
    refused(
        0,
        with_user(2, 8),
        DeltaError::UserOutOfRange { user: 8, limit: 8 },
    );
    refused(
        0,
        with_user(2, u64::MAX),
        DeltaError::UserOutOfRange {
            user: u64::MAX,
            limit: 8,
        },
    );
    let mut accumulator = held();
    for user in 0..8 {
        accumulator.apply(0, with_user(user + 2, user)).unwrap();
    }
    assert_eq!(
        accumulator.apply(0, with_user(10, 8)),
        Err(DeltaError::UserOutOfRange { user: 8, limit: 8 })
    );
    accumulator.done(0);
    let (metrics, _) = accumulator.merged();
    assert_eq!(metrics.scenarios[0].users.len(), 8);
}

/// A delta that is not final carries at most the capped number of
/// coordinated omission events, and only severity codes there are.
#[test]
fn gaggle_refuses_too_many_events_and_bad_severities() {
    use crate::metrics::coordinated_omission::{CoEvent, CoSeverity};
    use crate::metrics::GooseCoordinatedOmissionMitigation;
    let co = || super::delta::CoPart {
        strategy: GooseCoordinatedOmissionMitigation::Average,
        started_secs: 0,
        actual: 0,
        synthetic: 0,
        severity: BTreeMap::new(),
        events: Vec::new(),
        events_dropped: 0,
    };
    let event = CoEvent {
        timestamp_secs: 0,
        expected_cadence: Duration::from_millis(10),
        actual_duration: Duration::from_millis(100),
        synthetic_injected: 1,
        user_id: 0,
        scenario_name: "s".to_string(),
        severity: CoSeverity::Minor,
    };
    let limit = super::delta::MAX_CO_EVENTS_PER_DELTA;
    let mut many = counted(0, 2, 1);
    let mut part = co();
    part.events = vec![event; limit + 1];
    many.metrics.co = Some(part);
    refused(
        0,
        many,
        DeltaError::TooManyEvents {
            events: limit + 1,
            limit,
        },
    );
    let mut severity = counted(0, 2, 1);
    let mut part = co();
    part.severity.insert(4, 1);
    severity.metrics.co = Some(part);
    refused(0, severity, DeltaError::BadSeverity { code: 4 });
}

/// An average's total is bounded as a count is, so the merge of every
/// worker's stays finite and no worker is left out for it.
#[test]
fn gaggle_refuses_an_average_total_past_the_bound() {
    let mut delta = counted(0, 2, 1);
    let average = super::delta::Average {
        count: 1,
        sum_ms: 1.5e308,
    };
    delta
        .graph
        .averages
        .insert("GET /a".to_string(), [(4, average)].into());
    refused(0, delta, DeltaError::Overflow);
}

/// Figures every worker may send, at their bounds, merge without
/// overflowing the graph series' totals: users at the most a worker may
/// hold across hours, and a name's most requests a second, second after
/// second.
#[test]
fn gaggle_merge_at_the_bounds_does_not_overflow() {
    use super::delta::{MAX_COUNT, MAX_SECOND_COUNT};
    let mut accumulator = accumulator();
    for worker in 0..2 {
        let mut delta = GaggleDelta::new(worker, 1, 1, DeltaKind::Delta);
        delta.now = 9_000;
        delta.graph.users = [(0, MAX_COUNT), (9_000, MAX_COUNT)].into();
        delta.graph.requests.insert(
            "GET /a".to_string(),
            (0..4).map(|second| (second, MAX_SECOND_COUNT)).collect(),
        );
        delta.graph.averages.insert(
            "GET /a".to_string(),
            (0..4)
                .map(|second| {
                    let average = super::delta::Average {
                        count: MAX_SECOND_COUNT,
                        sum_ms: MAX_SECOND_COUNT as f64,
                    };
                    (second, average)
                })
                .collect(),
        );
        apply(&mut accumulator, &delta);
        accumulator.done(worker);
    }
    assert!(accumulator.excluded().is_empty());
    let (metrics, mut graph) = accumulator.merged();
    let configuration = configuration();
    crate::metrics::gaggle_snapshot(&configuration, &metrics, &mut graph);
}
