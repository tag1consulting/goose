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
 * The errors transactions returned, most frequent first, capped at
 * [`MAX_ERROR_ROWS`].
 */
transaction_errors: Array<TransactionErrorRow>,
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
settings: SnapshotSettings,
/**
 * On a Gaggle manager, its workers: their health, how the load is
 * spread across them, what happened to the Gaggle, and how a change
 * splits between them. Absent on one process. Added in 0.20.
 */
gaggle?: GaggleSnapshot, };

type ErrorRow = { method: string, name: string, error: string, occurrences: number, };

type GaggleByWorker = {
/**
 * At most 8, busiest first.
 */
workers: Array<string>,
/**
 * Each worker's requests started in each second of the series window,
 * from `series.start_second`, as `series.rps` counts them. Every
 * other worker's together are `series.rps` less these.
 */
rps: Array<Array<number>>,
/**
 * How many workers `others` adds up.
 */
others_workers: number, };

type GaggleEvents = {
/**
 * The run's series second each happened in.
 */
seconds: Array<number>,
/**
 * `run_started`, `worker_lost`, `worker_excluded`, `deltas_refused`,
 * `worker_saturated`, `worker_lagging`, `worker_recovered`,
 * `users_changed`, `throttle_changed`, `host_changed`,
 * `throttle_shares_rescaled`, `users_rebalanced`, `stop` or `start`.
 */
kinds: Array<string>,
/**
 * A few words for the chart's marker, `users 900` or `load-31 lost`,
 * for an event within the series window, which the chart draws.
 */
labels: Array<string | null>,
/**
 * The sentence the event log keeps.
 */
texts: Array<string>, };

type GaggleGrid = { names: Array<string>,
/**
 * 0 joining, 1 ready, 2 running, 3 stopping, 4 finished, 5 users
 * exited, 6 lost.
 */
states: Array<number>,
/**
 * Its most severe live flag: 0 none, 1 excluded, 2 saturated,
 * 3 lagging, 4 outlier, 5 refused or missing.
 */
flags: Array<number>,
/**
 * Its requests per second over the last 10 seconds as a percent of the
 * busiest worker's, 0 to 100.
 */
load: Array<number>,
/**
 * The users it runs now.
 */
users: Array<number>,
/**
 * Its CPU use, whole percent, while it runs; `null` when it sends none
 * or does not run.
 */
cpu: Array<number | null>,
/**
 * Its requests per second over the last 10 seconds, whole; `null`
 * while it does not run.
 */
rps: Array<number | null>,
/**
 * Its p95 over the last 10 seconds, whole milliseconds: 0 is under
 * half a millisecond.
 */
p95: Array<number | null>,
/**
 * Its requests per user per second's distance from the median of the
 * running workers, in percent, signed, when past the outlier
 * threshold; 0 when it is not.
 */
gap: Array<number>, };

type GagglePreviews = {
/**
 * The workers a change goes to: running, and neither lost nor stopped.
 */
workers: number, users: Array<GaggleSplit>,
/**
 * `null` without a throttle.
 */
throttle: Array<GaggleSplit> | null, };

type GaggleRows = {
/**
 * Each row's worker: its index in every array of the grid.
 */
workers: Array<number>,
/**
 * The CPU cores it said its process may use.
 */
cores: Array<number | null>, fail_percent: Array<number | null>, avg_ms: Array<number | null>, requests_per_user_per_second: Array<number | null>,
/**
 * Seconds since its last delta was merged, while it runs.
 */
last_delta_secs: Array<number | null>,
/**
 * Its live flags, most severe first: `lost`, `excluded`, `saturated`,
 * `lagging`, `outlier`, `refused` and `missing`.
 */
flags: Array<Array<string>>,
/**
 * While saturated, how long it has been.
 */
saturated_secs: Array<number | null>,
/**
 * How far its p95 is from the median of the running workers, in
 * percent, signed, when past the outlier threshold.
 */
p95_gap_percent: Array<number | null>, lost_reason: Array<string | null>,
/**
 * The run's series second it was lost in.
 */
lost_second: Array<number | null>, };

type GaggleSnapshot = {
/**
 * The workers the manager waits for (`--expect-workers`).
 */
expected_workers: number, summary: GaggleSummary,
/**
 * The most urgent workers, at most 100: lost, left out, saturated,
 * lagging, outlier, then the rest, each group by name.
 */
workers: GaggleRows,
/**
 * True when `workers` leaves some out; `summary` and `grid` count every
 * worker.
 */
workers_truncated: boolean, grid: GaggleGrid, by_worker: GaggleByWorker,
/**
 * What happened to the Gaggle, newest first, at most 200.
 */
events: GaggleEvents,
/**
 * True when older events are left out of `events`.
 */
events_truncated: boolean, previews: GagglePreviews, };

type GaggleSplit = { value: number, workers: number, };

type GaggleSummary = { expected: number,
/**
 * Workers running their share now.
 */
running: number,
/**
 * Workers stopping, finished, or whose users all exited.
 */
stopped: number, lost: number,
/**
 * Workers whose metrics the merge leaves out now.
 */
excluded: number,
/**
 * Workers saturated, lagging or an outlier now, live: on the last 10
 * seconds.
 */
saturated: number, lagging: number, outliers: number,
/**
 * Running workers that are neither saturated, lagging nor left out.
 */
healthy: number,
/**
 * One sentence naming the workers that need attention now, `null` when
 * none does: `2 workers need attention: load-17 saturated (CPU 97% for
 * 42 s), load-31 lost at 03:05 (connection reset). 45 workers healthy.`
 */
attention: string | null,
/**
 * The "Degraded:" sentences the reports would open with now.
 */
degraded: Array<string>,
/**
 * The "Check:" sentences the reports would open with now.
 */
checks: Array<string>,
/**
 * The last users change: its second, and the users before and after.
 */
last_users_change?: GaggleUsersChange, };

type GaggleUsersChange = { second: number, from: number, to: number, };

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
 * Distinct users that ran this scenario since the last metrics reset,
 * whether or not they finished a run; not only the users running it now.
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
heatmap: Heatmap | null,
/**
 * On a Gaggle manager while a worker still runs, the first second, from
 * the test start, whose figures may still be arriving from the workers:
 * from it on each second holds what has arrived so far, and a second
 * with nothing yet has no data rather than 0. Absent on one process and
 * once every worker finished or was lost. Added in 0.20.
 */
provisional_from?: number, };

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
scenario_metrics_disabled: boolean, requests_truncated: boolean, errors_truncated: boolean, transaction_errors_truncated: boolean, series_seconds: number,
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
 * `null` before the first run, and for a run that has none.
 */
folder: string | null,
/**
 * The `--report-file` paths, which are always written.
 */
paths: Array<string>,
/**
 * The run's `--notes`.
 */
notes: string | null,
/**
 * The folder of the last report written into a run's folder at the end
 * of a run.
 */
last_report: string | null,
/**
 * The name the report routes take for `last_report`; `null` when they
 * take none, as for a name with a space.
 */
last_report_name: string | null,
/**
 * Why the last report Goose tried to write into the run's folder
 * failed, or why the run has no folder.
 */
error: string | null,
/**
 * How many reports Goose has written into a run's folder since the
 * process started, at the end of a run or by Save report now.
 */
writes: number, };

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
 * The most requests per second all users make together, `null` without
 * a throttle. A running load test changes only its rate. Added in 0.20:
 * optional in the types, so a snapshot shaped before it still checks.
 */
throttle?: number | null,
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

type TransactionErrorRow = {
/**
 * The transaction's number, `scenario.transaction` counted from 1.
 */
transaction: string, scenario: string,
/**
 * The transaction's name, empty when it has none.
 */
name: string, error: string, occurrences: number, };

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
