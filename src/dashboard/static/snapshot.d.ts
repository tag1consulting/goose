// Generated from src/metrics/dashboard_snapshot.rs by the unit test
// `dashboard_types_match_rust`. Do not edit. To regenerate, run:
//   GOOSE_UPDATE_DASHBOARD_TYPES=1 cargo test --lib dashboard_types_match_rust

/** The only `DashboardSnapshot.version` this client accepts. */
type DashboardSnapshotVersion = 1;

type AggregateMetrics = { total_requests: number, total_failures: number, requests_per_second: number, failures_per_second: number, failure_rate: number, response_time_avg_ms: number, response_time_min_ms: number, response_time_max_ms: number, percentile_ms: Percentiles,
/**
 * The same times to the microsecond, null without times.
 */
response_time_us: MicrosecondTimes | null,
/**
 * True iff coordinated-omission metrics object is present AND has recorded
 * at least one CO event (or synthetic request).
 */
co_active: boolean, co_adjusted: CoAdjusted | null, };

type CoAdjusted = { response_time_avg_ms: number, response_time_max_ms: number, percentile_ms: Percentiles,
/**
 * The same times to the microsecond, null without times.
 */
response_time_us: MicrosecondTimes | null, };

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
 * The report the run writes: Save report, its folder and paths.
 */
report: SnapshotReport,
/**
 * The run settings: what the next Start runs, and where the running
 * test is.
 */
settings: SnapshotSettings, };

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
 * Each row by index, as `[time, lowest, highest]` in microseconds:
 * the time the row stands for, which is its middle, then the first and
 * last microsecond the row holds. A longer table replaces a shorter
 * one; entries never change.
 */
buckets_us: Array<[number, number, number]>,
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

type MicrosecondTimes = { min: number, max: number, p50: number, p95: number, p99: number, };

type Percentiles = { p50: number, p95: number, p99: number, };

type RequestRow = { method: string, name: string, request_count: number, failure_count: number, requests_per_second: number, failures_per_second: number, response_time_avg_ms: number, response_time_min_ms: number, response_time_max_ms: number, percentile_ms: Percentiles,
/**
 * The same times to the microsecond, null without times.
 */
response_time_us: MicrosecondTimes | null, status_codes: Array<[number, number]>, co_adjusted: CoAdjusted | null,
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
run_count: number, runs_per_second: number, response_time_avg_ms: number, response_time_min_ms: number, response_time_max_ms: number, percentile_ms: Percentiles,
/**
 * The same times to the microsecond, all 0 without times.
 */
response_time_us: MicrosecondTimes, };

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
 * True when the dashboard tracks response times by second for its
 * heatmap: on with `--dashboard` unless `--no-percentiles-over-time` or
 * `--no-metrics`.
 */
percentiles_over_time: boolean, };

type SnapshotReport = {
/**
 * Save report: write the run's `report.json` and `report.html` into its
 * folder.
 */
save: boolean,
/**
 * Include graph data: the JSON report holds the per second graph data.
 */
graph_data: boolean,
/**
 * The current or last run's folder, as given, never made absolute;
 * `null` before the first run.
 */
folder: string | null,
/**
 * The folder the runs panel lists: the one run folders go in.
 */
parent: string,
/**
 * The `--report-file` paths, which are always written.
 */
paths: Array<string>,
/**
 * The run's `--notes`.
 */
notes: string | null,
/**
 * The folder of the last report Save report wrote.
 */
last_report: string | null,
/**
 * Why the last report Save report tried to write failed.
 */
error: string | null, };

type SnapshotSettings = {
/**
 * The `--host` override, or `null`.
 */
host: string | null,
/**
 * The hosts the next Start will load test, one per scenario from
 * `--host`, else the scenario's host, else the default host, in the
 * same form, sorted and deduplicated, as the top level `hosts` (which
 * keeps meaning the current or last run's). A scenario with no host
 * from any source adds nothing.
 */
hosts: Array<string>,
/**
 * The configured user count, `null` only with a test plan. Not
 * `target_users`, the count the current step ramps toward.
 */
users: number | null, increase_rate: number | null, increase_time_secs: number | null, decrease_rate: number | null, decrease_time_secs: number | null,
/**
 * The hold at full users after the ramp up, not the total run time.
 */
run_time_secs: number | null,
/**
 * The test plan as `--test-plan` takes it, in canonical form: each
 * step's duration in whole seconds.
 */
test_plan: string | null,
/**
 * Seconds until the hold ends once it has a fixed end, 0 while a ramp
 * runs past it. `null` in Idle, during the first ramp up, once the
 * ramp down starts, with no run time, and with a test plan.
 */
hold_ends_in_secs: number | null,
/**
 * Where the run is: `idle`, `ramp_up`, `hold`, `hold_until_stopped`,
 * `ramping_down` or `stopping` for a users ramp; `idle`, `test_plan` or
 * `stopping` for a test plan.
 */
stage: "idle" | "ramp_up" | "hold" | "hold_until_stopped" | "ramping_down" | "test_plan" | "stopping", };

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
run_count: number, failure_count: number, runs_per_second: number, failures_per_second: number, response_time_avg_ms: number, response_time_min_ms: number, response_time_max_ms: number, percentile_ms: Percentiles,
/**
 * The same times to the microsecond, all 0 without times.
 */
response_time_us: MicrosecondTimes, };
