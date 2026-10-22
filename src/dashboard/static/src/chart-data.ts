// Each line chart's one sentence summary and "Show data" table. Kept apart
// from charts.ts, which the tests bundle on its own without a page: these
// look their elements up when the module loads.

import { requireElement } from "./dom";
import { formatInt, formatMs, formatRate } from "./format";
import { chartDataTable } from "./tables";

/** Seconds a summary averages, and compares with as many before them. */
const SUMMARY_SECONDS = 10;

const rpsSummary = requireElement("chart-rps-summary", HTMLElement);
const usersSummary = requireElement("chart-users-summary", HTMLElement);
const latencySummary = requireElement("chart-latency-summary", HTMLElement);

const showRpsData = chartDataTable<{ second: number; rps: number; fps: number }>(
  "chart-rps-details",
  "chart-rps",
  [
    { key: "second", type: "num", format: (r) => String(r.second) },
    { key: "rps", type: "num", format: (r) => formatRate(r.rps) },
    { key: "fps", type: "num", format: (r) => formatRate(r.fps) },
  ]
);
const showUsersData = chartDataTable<{ second: number; users: number }>(
  "chart-users-details",
  "chart-users",
  [
    { key: "second", type: "num", format: (r) => String(r.second) },
    { key: "users", type: "num", format: (r) => formatInt(r.users) },
  ]
);
const showLatencyData = chartDataTable<{ second: number; avg: number }>(
  "chart-latency-details",
  "chart-latency",
  [
    { key: "second", type: "num", format: (r) => String(r.second) },
    { key: "avg", type: "num", format: (r) => formatMs(r.avg) },
  ]
);

/**
 * The mean of the last 10 values, and its trend: "rising" or "falling" when it
 * is more than 10 percent above or below the mean of the 10 before, else
 * "steady"; null with fewer than 20 values or when the older mean is 0. One
 * second alone can be partial or noisy, so a summary reads the mean.
 */
export function recentTrend(values: number[]): { mean: number; trend: string | null } {
  const mean = (from: number, to: number): number => {
    let sum = 0;
    for (let i = from; i < to; i++) sum += values[i];
    return to > from ? sum / (to - from) : 0;
  };
  const n = values.length;
  const now = mean(Math.max(0, n - SUMMARY_SECONDS), n);
  if (n < 2 * SUMMARY_SECONDS) return { mean: now, trend: null };
  const before = mean(n - 2 * SUMMARY_SECONDS, n - SUMMARY_SECONDS);
  if (before === 0) return { mean: now, trend: null };
  const trend = now > before * 1.1 ? "rising" : now < before * 0.9 ? "falling" : "steady";
  return { mean: now, trend };
}

/** `text`, then ", " and the trend when there is one. */
function withTrend(text: string, trend: string | null): string {
  return trend === null ? text : text + ", " + trend;
}

/**
 * Sets a chart's one sentence summary, only when it changes. It is not a
 * live region: it is read where it stands, as the chart is looked at.
 */
function setSummary(el: HTMLElement, text: string): void {
  if (el.textContent !== text) el.textContent = text;
}

/** The latency summary, which the heatmap writes while the card shows it. */
export function setLatencySummary(text: string): void {
  setSummary(latencySummary, text);
}

/**
 * Writes each chart's summary and, while it is open, its data table, from
 * the series; the average latency's only when `averageLatency`. These do not
 * need a canvas, so they are written even where the charts cannot draw.
 * `targetUsers` is the users summary's "of".
 */
export function updateChartData(series: SeriesWindow, averageLatency: boolean, targetUsers: number): void {
  const start = series.start_second;
  // The newest second is still counting, so its requests and failures are
  // short of a whole second's; left in, every summary leans to "falling".
  const rps = recentTrend(series.rps.slice(0, -1));
  setSummary(
    rpsSummary,
    series.rps.length === 0
      ? "No data yet."
      : withTrend("RPS " + formatRate(rps.mean), rps.trend) +
          "; failures " + formatRate(recentTrend(series.fps.slice(0, -1)).mean) + "/s."
  );
  showRpsData(series.rps.map((rps, i) => ({ second: start + i, rps, fps: series.fps[i] || 0 })));
  const users = recentTrend(series.users);
  setSummary(
    usersSummary,
    series.users.length === 0
      ? "No data yet."
      : withTrend(
          "Users " + formatInt(users.mean) + (targetUsers > 0 ? " of " + formatInt(targetUsers) : ""),
          users.trend
        ) + "."
  );
  showUsersData(series.users.map((users, i) => ({ second: start + i, users })));
  if (averageLatency) {
    const lat = recentTrend(series.avg_latency_ms);
    setLatencySummary(
      series.avg_latency_ms.length === 0
        ? "No data yet."
        : withTrend("Average " + formatInt(lat.mean) + " ms", lat.trend) + "."
    );
    showLatencyData(series.avg_latency_ms.map((avg, i) => ({ second: start + i, avg })));
  }
}
