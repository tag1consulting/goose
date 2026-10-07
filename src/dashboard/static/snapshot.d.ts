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
co_active: boolean, };

type DashboardSnapshot = { version: number, generated_at: string, goose_version: string, phase: string, duration_secs: number,
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
target_users: number, total_users: number, hosts: Array<string>, aggregate: AggregateMetrics, requests: Array<RequestRow>, errors: Array<ErrorRow>, series: SeriesWindow, flags: SnapshotFlags, };

type ErrorRow = { method: string, name: string, error: string, occurrences: number, };

type Percentiles = { p50: number, p95: number, p99: number, };

type RequestRow = { method: string, name: string, request_count: number, failure_count: number, requests_per_second: number, failures_per_second: number, response_time_avg_ms: number, response_time_min_ms: number, response_time_max_ms: number, percentile_ms: Percentiles, status_codes: Array<[number, number]>, };

type SeriesWindow = {
/**
 * First exported bucket index in seconds-from-test-start.
 */
start_second: number, rps: Array<number>, fps: Array<number>, users: Array<number>, avg_latency_ms: Array<number>, };

type SnapshotFlags = { metrics_disabled: boolean, requests_truncated: boolean, errors_truncated: boolean, series_seconds: number, };
