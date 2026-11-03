// Each line chart's one sentence summary and "Show data" table. Kept apart
// from charts.ts, which the tests bundle on its own without a page: these
// look their elements up when the module loads.

import { completeSeconds, provisionalIndex, provisionalSeconds } from "./charts";
import { requireElement } from "./dom";
import { formatInt, formatMs, formatRate } from "./format";
import { chartDataTable } from "./tables";

/** Seconds a summary averages, and compares with as many before them. */
const SUMMARY_SECONDS = 10;

const rpsSummary = requireElement("chart-rps-summary", HTMLElement);
const usersSummary = requireElement("chart-users-summary", HTMLElement);
const latencySummary = requireElement("chart-latency-summary", HTMLElement);
const rpsProvisionalNote = requireElement("chart-rps-provisional", HTMLElement);
const latencyProvisionalNote = requireElement("chart-latency-provisional", HTMLElement);

/**
 * A chart's one line note on a Gaggle manager: how many of its newest
 * seconds are still arriving from the workers, drawn lighter and dashed.
 * Hidden when none is.
 */
export function provisionalNote(seconds: number): string {
  return seconds > 0 ? "last " + seconds + " s still arriving" : "";
}

function setNote(el: HTMLElement, text: string): void {
  if (el.textContent !== text) el.textContent = text;
  el.hidden = text === "";
}

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
  // Left in, the newest second would lean every summary to "falling".
  const rpsDrawn = completeSeconds(series.rps);
  const fpsDrawn = completeSeconds(series.fps);
  // On a Gaggle manager the summaries read the complete seconds only, and
  // the tables leave out a provisional second with nothing yet, as the
  // chart draws it as a gap.
  const provisional = provisionalIndex(series);
  const complete = (values: number[]): number[] =>
    provisional === null ? values : values.slice(0, provisional);
  const rpsComplete = complete(rpsDrawn);
  const fpsComplete = complete(fpsDrawn);
  const note = provisionalNote(provisionalSeconds(rpsDrawn.length, provisional));
  setNote(rpsProvisionalNote, note);
  setNote(latencyProvisionalNote, note);
  const rps = recentTrend(rpsComplete);
  setSummary(
    rpsSummary,
    series.rps.length === 0
      ? "No data yet."
      : withTrend("RPS " + formatRate(rps.mean), rps.trend) +
          "; failures " + formatRate(recentTrend(fpsComplete).mean) + "/s."
  );
  showRpsData(
    rpsDrawn
      .map((rps, i) => ({ second: start + i, rps, fps: fpsDrawn[i] || 0, at: i }))
      .filter((row) => provisional === null || row.at < provisional || row.rps > 0 || row.fps > 0)
      .map(({ second, rps, fps }) => ({ second, rps, fps }))
  );
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
    const latDrawn = completeSeconds(series.avg_latency_ms);
    const latComplete = complete(latDrawn);
    const lat = recentTrend(latComplete);
    setLatencySummary(
      series.avg_latency_ms.length === 0
        ? "No data yet."
        : withTrend("Average " + formatMs(lat.mean) + " ms", lat.trend) + "."
    );
    showLatencyData(
      latDrawn
        .map((avg, i) => ({ second: start + i, avg, at: i }))
        .filter((row) => provisional === null || row.at < provisional || row.avg > 0)
        .map(({ second, avg }) => ({ second, avg }))
    );
  }
}
