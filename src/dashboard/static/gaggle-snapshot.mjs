// @ts-check
// A Gaggle manager's snapshot for the client tests: 48 workers at 04:12,
// load-31 lost at 03:05, load-17 saturated and load-42 lagging, as the
// layout the workers panel was built to. `npm run check` type-checks this
// file against the generated snapshot.d.ts.

import { IDLE_SNAPSHOT } from "./idle-snapshot.mjs";

/** @param {number} n */
const name = (n) => "load-" + String(n).padStart(2, "0");

/**
 * The snapshot, with `options` to change it: `names` replaces worker
 * names by index, `events` the newest events (prepended), `saturatedFlag`
 * whether load-17 is saturated.
 * @param {{ names?: Record<number, string>, events?: { second: number, kind: string, label: string | null, text: string }[], saturatedFlag?: boolean, duration?: number }} [options]
 * @returns {DashboardSnapshot}
 */
export function gaggleSnapshot(options = {}) {
  const names = Array.from({ length: 48 }, (_, i) => (options.names && options.names[i]) || name(i));
  const saturated = options.saturatedFlag !== false;
  const states = names.map((_, i) => (i === 31 ? 6 : 2));
  const flags = names.map((_, i) => (i === 17 && saturated ? 2 : i === 42 ? 3 : 0));
  const users = names.map((_, i) => (i === 31 ? 0 : i < 7 ? 20 : 19));
  const rps = names.map((_, i) => (i === 31 ? null : i === 17 ? 41 : 63));
  // Rows: lost, saturated, lagging, then the rest by name.
  const order = [31, ...(saturated ? [17] : []), 42, ...names.map((_, i) => i).filter((i) => i !== 31 && i !== 42 && (i !== 17 || !saturated))];
  const base = [
    { second: 252, kind: "worker_saturated", label: "load-17 saturated", text: "load-17 saturated (CPU 97% for 5 s)" },
    { second: 185, kind: "throttle_shares_rescaled", label: "throttle rescaled", text: "throttle shares of 3,000 req/s rescaled over 47 workers" },
    { second: 185, kind: "worker_lost", label: "load-31 lost", text: "load-31 lost: connection reset" },
    { second: 150, kind: "throttle_changed", label: "throttle 3,000", text: "throttle set to 3,000 req/s, acknowledged by 48 of 48 in 0.2 s" },
    { second: 120, kind: "users_changed", label: "users 900", text: "users 600 to 900, acknowledged by 48 of 48 in 0.4 s" },
  ];
  const older = Array.from({ length: 6 }, (_, i) => ({
    second: 60 - i * 10,
    kind: "worker_recovered",
    label: null,
    text: name(i) + " recovered: no longer lagging",
  }));
  const events = [
    ...(options.events || []),
    ...base,
    ...older,
    { second: 2, kind: "run_started", label: null, text: "48 of 48 workers joined, run started" },
  ];
  const seconds = 253;
  const series = {
    start_second: 0,
    rps: Array.from({ length: seconds }, () => 2987),
    fps: Array.from({ length: seconds }, () => 9),
    users: Array.from({ length: seconds }, () => 900),
    avg_latency_ms: Array.from({ length: seconds }, () => 18),
    heatmap: null,
  };
  /** @type {DashboardSnapshot} */
  const snapshot = {
    ...IDLE_SNAPSHOT,
    phase: "maintain",
    duration_secs: options.duration === undefined ? 252 : options.duration,
    active_users: 900,
    maximum_users: 900,
    target_users: 900,
    total_users: 900,
    aggregate: {
      ...IDLE_SNAPSHOT.aggregate,
      total_requests: 750_000,
      total_failures: 2_250,
      requests_per_second: 2987,
      failures_per_second: 9,
      failure_rate: 0.3,
      response_time_avg_ms: 18,
      percentile_ms: { p50: 12, p95: 41, p99: 90 },
    },
    series,
    settings: { ...IDLE_SNAPSHOT.settings, users: 900, throttle: 3000, stage: "hold_until_stopped" },
    gaggle: {
      expected_workers: 48,
      summary: {
        expected: 48,
        running: 47,
        stopped: 0,
        lost: 1,
        excluded: 0,
        saturated: saturated ? 1 : 0,
        lagging: 1,
        outliers: 0,
        healthy: saturated ? 45 : 46,
        attention: saturated
          ? "3 workers need attention: load-17 saturated (CPU 97% for 42 s), load-42 lagging (last delta 5 s ago), " +
            names[31] + " lost at 03:05 (connection reset). 45 workers healthy."
          : "2 workers need attention: load-42 lagging (last delta 5 s ago), " + names[31] + " lost at 03:05 (connection reset). 46 workers healthy.",
        degraded: [],
        checks: [],
        last_users_change: { second: 120, from: 600, to: 900 },
      },
      workers: {
        workers: order,
        cores: order.map(() => 4),
        fail_percent: order.map((i) => (i === 31 ? null : 0.3)),
        avg_ms: order.map((i) => (i === 31 ? null : 18)),
        requests_per_user_per_second: order.map((i) => (i === 31 ? null : 3.3)),
        last_delta_secs: order.map((i) => (i === 31 ? null : i === 42 ? 5.2 : 0.4)),
        flags: order.map((i) =>
          i === 31 ? ["lost"] : i === 17 && saturated ? ["saturated"] : i === 42 ? ["lagging"] : []
        ),
        saturated_secs: order.map((i) => (i === 17 && saturated ? 42 : null)),
        p95_gap_percent: order.map(() => null),
        lost_reason: order.map((i) => (i === 31 ? 0 : null)),
        lost_second: order.map((i) => (i === 31 ? 185 : null)),
        reasons: ["connection reset"],
      },
      workers_truncated: false,
      grid: {
        names,
        states,
        flags,
        load: rps.map((r) => (r === null ? 0 : Math.round((r * 100) / 63))),
        users,
        cpu: names.map((_, i) => (i === 31 ? -1 : i === 17 ? 97 : i === 5 ? -1 : 40)),
        rps: rps.map((r) => (r === null ? -1 : r)),
        p95: names.map((_, i) => (i === 31 ? -1 : i === 17 ? 212 : 40)),
        gap: names.map((_, i) => (i === 17 ? -35 : 0)),
      },
      by_worker: {
        workers: ["load-00", "load-01", "load-02", "load-03", "load-04", "load-05", "load-06", "load-07"],
        rps: Array.from({ length: 8 }, () => Array.from({ length: seconds }, () => 63)),
        others_workers: 39,
      },
      events: {
        seconds: events.map((e) => e.second),
        kinds: events.map((e) => e.kind),
        labels: events.map((e) => e.label),
        texts: events.map((e) => e.text),
      },
      events_truncated: false,
      previews: {
        workers: 47,
        users: [
          { value: 20, workers: 7 },
          { value: 19, workers: 40 },
        ],
        throttle: [
          { value: 64, workers: 39 },
          { value: 63, workers: 8 },
        ],
      },
    },
  };
  return snapshot;
}
