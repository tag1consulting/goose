// @ts-check
// A complete idle snapshot for the client tests. `npm run check` type-checks
// this file against the generated snapshot.d.ts, so a field added to or
// removed from src/metrics/dashboard_snapshot.rs fails the check here, at the
// line to update.

/** @type {DashboardSnapshot} */
export const IDLE_SNAPSHOT = {
  version: 1,
  generated_at: "1970-01-01T00:00:00Z",
  goose_version: "0.0.0",
  phase: "idle",
  stopping: false,
  duration_secs: 0,
  active_users: 0,
  maximum_users: 0,
  target_users: 0,
  total_users: 0,
  hosts: [],
  aggregate: {
    total_requests: 0,
    total_failures: 0,
    requests_per_second: 0,
    failures_per_second: 0,
    failure_rate: 0,
    response_time_avg_ms: 0,
    response_time_min_ms: 0,
    response_time_max_ms: 0,
    percentile_ms: { p50: 0, p95: 0, p99: 0 },
    co_active: false,
  },
  requests: [],
  errors: [],
  series: { start_second: 0, rps: [], fps: [], users: [], avg_latency_ms: [] },
  flags: {
    metrics_disabled: false,
    requests_truncated: false,
    errors_truncated: false,
    series_seconds: 0,
  },
};
