// Generated from src/metrics/dashboard_snapshot.rs by the unit test
// `dashboard_types_match_rust`. Do not edit. To regenerate, run:
//   GOOSE_UPDATE_DASHBOARD_TYPES=1 cargo test --lib dashboard_types_match_rust

/** The only `DashboardSnapshot.version` this client accepts. */
type DashboardSnapshotVersion = 1;

type AggregateMetrics = { total_requests: number, total_failures: number, requests_per_second: number, failures_per_second: number, failure_rate: number, response_time_avg_ms: number, response_time_min_ms: number, response_time_max_ms: number, percentile_ms: Percentiles,
/**
 * True iff coordinated-omission metrics object is present AND has recorded
 * at least one CO event (or synthetic request).
 */
co_active: boolean, co_adjusted: CoAdjusted | null, };

type CoAdjusted = { response_time_avg_ms: number, response_time_max_ms: number, percentile_ms: Percentiles, };

type DashboardSnapshot = { version: number, generated_at: string, goose_version: string, phase: string,
/**
 * True while a cancel (Stop, `shutdown`, Ctrl-C) ramps the run down,
 * until it reaches idle. A test plan's own ramp down leaves it false, so
 * clients can tell the two `decrease` phases apart.
 */
stopping: boolean, duration_secs: number,
/**
 * Users currently running (main-loop active count).
 */
active_users: number,
/**
 * Peak concurrent users observed during this run (high-water mark).
 */
maximum_users: number,
/**
 * Current test-plan step target — users the attack is increasing or
 * decreasing toward (not the historical peak).
 */
target_users: number, total_users: number, hosts: Array<string>, aggregate: AggregateMetrics, requests: Array<RequestRow>, errors: Array<ErrorRow>,
/**
 * One row per registered scenario, in registration order. Never truncated.
 */
scenarios: Array<ScenarioRow>,
/**
 * One row per registered transaction, grouped by scenario, each group in
 * registration order. Never truncated.
 */
transactions: Array<TransactionRow>, series: SeriesWindow, flags: SnapshotFlags,
/**
 * Whether runs are saved, and the last run Goose saved or tried to save.
 */
save: SnapshotSave, };

type ErrorRow = { method: string, name: string, error: string, occurrences: number, };

type Heatmap = {
/**
 * Which set of histograms the columns come from. It changes when they
 * are cleared (metrics reset, or a new test), so a client starts over.
 */
generation: number,
/**
 * The newest change these columns include. Send it back as
 * `heatmap_since` with `heatmap_generation` to get only later changes.
 */
rev: number,
/**
 * The `rev` these columns follow on from, or 0 for a whole window.
 */
since: number,
/**
 * First second of the window, in seconds from the test start. Columns
 * before it are dropped.
 */
start_second: number,
/**
 * Last second of the window.
 */
end_second: number,
/**
 * Seconds from this one on may still be missing their slowest requests,
 * so they are not drawn yet.
 */
hold_from: number,
/**
 * Each bucket by index, as `[time, lowest, highest]` in milliseconds:
 * the rounded response time Goose stores for the bucket, and the range
 * of measured times that round to it. A longer table replaces a shorter
 * one; entries never change.
 */
buckets: Array<[number, number, number]>,
/**
 * The columns that changed, oldest first.
 */
columns: Array<HeatmapColumn>, };

type HeatmapColumn = {
/**
 * The second, from the test start.
 */
second: number,
/**
 * Measured response times.
 */
measured: Array<number>,
/**
 * Synthetic times coordinated omission mitigation added to this second.
 * Empty unless mitigation is on and has recorded an event.
 */
synthetic: Array<number>, };

type Percentiles = { p50: number, p95: number, p99: number, };

type RequestRow = { method: string, name: string, request_count: number, failure_count: number, requests_per_second: number, failures_per_second: number, response_time_avg_ms: number, response_time_min_ms: number, response_time_max_ms: number, percentile_ms: Percentiles, status_codes: Array<[number, number]>, co_adjusted: CoAdjusted | null,
/**
 * True when the response time heatmap can show this request alone:
 * they are kept for the first 100 request names seen since metrics were
 * last reset.
 */
has_series: boolean, };

type RequestSeries = { method: string, name: string, heatmap: Heatmap, };

type ScenarioRow = {
/**
 * Goose's own scenario index, counted from 0 in registration order. The
 * dashboard shows it counted from 1.
 */
scenario_index: number, scenario_name: string,
/**
 * Distinct users that have finished at least one run of this scenario
 * since the last metrics reset; not the users running it now.
 */
users: number,
/**
 * Completed passes through the scenario's weighted transactions.
 */
run_count: number, runs_per_second: number, response_time_avg_ms: number, response_time_min_ms: number, response_time_max_ms: number, percentile_ms: Percentiles, };

type SeriesWindow = {
/**
 * First exported bucket index in seconds-from-test-start.
 */
start_second: number, rps: Array<number>, fps: Array<number>, users: Array<number>, avg_latency_ms: Array<number>,
/**
 * Response times of all requests by second, for the dashboard's
 * heatmap. Null when percentiles over time are off.
 */
heatmap: Heatmap | null, };

type SnapshotFlags = { metrics_disabled: boolean,
/**
 * True under `--no-transaction-metrics`, and under `--no-metrics` too;
 * `transactions` is then empty.
 */
transaction_metrics_disabled: boolean,
/**
 * True under `--no-scenario-metrics`, and under `--no-metrics` too;
 * `scenarios` is then empty.
 */
scenario_metrics_disabled: boolean, requests_truncated: boolean, errors_truncated: boolean, series_seconds: number,
/**
 * True when the dashboard tracks p50, p95 and p99 by second: on with
 * `--dashboard` unless `--no-percentiles-over-time` or `--no-metrics`.
 */
percentiles_over_time: boolean, };

type SnapshotSave = {
/**
 * `on`, `off` (`--no-save` or `--no-metrics`), or `failed` (the runs
 * directory or the run's directory couldn't be created).
 */
state: "on" | "off" | "failed",
/**
 * Why the last run couldn't be saved, or why this run isn't saved.
 */
reason: string | null,
/**
 * The runs directory as given, never resolved to an absolute path.
 */
dir: string,
/**
 * The id of the last run Goose saved or tried to save.
 */
last_run: string | null, };

type TransactionRow = {
/**
 * Goose's own scenario index, counted from 0 in registration order. The
 * dashboard shows it counted from 1.
 */
scenario_index: number, scenario_name: string,
/**
 * Goose's own transaction index within its scenario, counted from 0 in
 * registration order. The dashboard shows it counted from 1.
 */
transaction_index: number,
/**
 * Empty when the transaction has no name.
 */
transaction_name: string,
/**
 * Successful plus failed runs.
 */
run_count: number, failure_count: number, runs_per_second: number, failures_per_second: number, response_time_avg_ms: number, response_time_min_ms: number, response_time_max_ms: number, percentile_ms: Percentiles, };
