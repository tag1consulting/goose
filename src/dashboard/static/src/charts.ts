// The three line charts: RPS and failures, active users, and average latency,
// which the response time card shows when percentiles over time are off.

import {
  CategoryScale,
  Chart,
  Filler,
  Legend,
  LinearScale,
  LineController,
  LineElement,
  PointElement,
  Tooltip,
  type ChartConfiguration,
  type ChartOptions,
} from "chart.js";
import { requireElement } from "./dom";
import { formatInt, formatRate } from "./format";
import { chartDataTable } from "./tables";

Chart.register(
  LineController,
  LineElement,
  PointElement,
  LinearScale,
  CategoryScale,
  Legend,
  Tooltip,
  Filler
);

type LineChart = Chart<"line", number[], string>;

interface ChartColors {
  rps: string;
  fps: string;
  users: string;
  latency: string;
  grid: string;
  tick: string;
  /** The heatmap's percentile lines. */
  percentile: string;
}

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
    { key: "avg", type: "num", format: (r) => formatRate(r.avg) },
  ]
);

let chartRps: LineChart | null = null;
let chartUsers: LineChart | null = null;
let chartLatency: LineChart | null = null;

function isDark(): boolean {
  let dark =
    window.matchMedia &&
    window.matchMedia("(prefers-color-scheme: dark)").matches;
  // If no preference API, assume dark (default CSS is dark).
  if (
    window.matchMedia &&
    !window.matchMedia("(prefers-color-scheme: light)").matches &&
    !window.matchMedia("(prefers-color-scheme: dark)").matches
  ) {
    dark = true;
  }
  return dark;
}

/** The theme, and the tick, grid and percentile line colours the charts use in it. */
export function chartTheme(): { dark: boolean; tick: string; grid: string; percentile: string } {
  const colors = chartColors();
  return {
    dark: isDark(),
    tick: colors.tick,
    grid: colors.grid,
    percentile: colors.percentile,
  };
}

function chartColors(): ChartColors {
  const dark = isDark();
  return {
    rps: dark ? "#3d8bfd" : "#0b5fff",
    fps: dark ? "#f07178" : "#b42318",
    users: dark ? "#3dd68c" : "#0a7a43",
    latency: dark ? "#e6b450" : "#9a6700",
    grid: dark ? "rgba(154, 167, 184, 0.2)" : "rgba(91, 107, 124, 0.2)",
    tick: dark ? "#9aa7b8" : "#5b6b7c",
    percentile: dark ? "#ff8f45" : "#c2410c",
  };

}

function baseChartOpts(colors: ChartColors): ChartOptions<"line"> {
  // Fresh object per chart so Chart.js cannot share/mutate nested scales.
  return {
    responsive: true,
    maintainAspectRatio: false,
    animation: false,
    interaction: { mode: "index", intersect: false },
    plugins: {
      legend: {
        labels: { color: colors.tick, boxWidth: 12, font: { size: 11 } },
      },
    },
    scales: {
      x: {
        ticks: {
          color: colors.tick,
          maxTicksLimit: 8,
          font: { size: 10 },
        },
        grid: { color: colors.grid },
      },
      y: {
        beginAtZero: true,
        ticks: { color: colors.tick, font: { size: 10 } },
        grid: { color: colors.grid },
      },
    },
  };
}

export function ensureCharts(): void {
  const colors = chartColors();

  if (!chartRps) {
    const config: ChartConfiguration<"line", number[], string> = {
      type: "line",
      data: {
        labels: [],
        datasets: [
          {
            label: "RPS",
            data: [],
            borderColor: colors.rps,
            backgroundColor: colors.rps,
            borderWidth: 1.5,
            pointRadius: 0,
            tension: 0.15,
          },
          {
            label: "Failures/s",
            data: [],
            borderColor: colors.fps,
            backgroundColor: colors.fps,
            borderWidth: 1.5,
            pointRadius: 0,
            tension: 0.15,
          },
        ],
      },
      options: baseChartOpts(colors),
    };
    chartRps = new Chart(requireElement("chart-rps", HTMLCanvasElement), config);
  }
  if (!chartUsers) {
    const config: ChartConfiguration<"line", number[], string> = {
      type: "line",
      data: {
        labels: [],
        datasets: [
          {
            label: "Users",
            data: [],
            borderColor: colors.users,
            backgroundColor: colors.users,
            borderWidth: 1.5,
            pointRadius: 0,
            tension: 0.15,
            fill: false,
          },
        ],
      },
      options: baseChartOpts(colors),
    };
    chartUsers = new Chart(
      requireElement("chart-users", HTMLCanvasElement),
      config
    );
  }
  if (!chartLatency) {
    const config: ChartConfiguration<"line", number[], string> = {
      type: "line",
      data: {
        labels: [],
        datasets: [
          {
            label: "Avg ms",
            data: [],
            borderColor: colors.latency,
            backgroundColor: colors.latency,
            borderWidth: 1.5,
            pointRadius: 0,
            tension: 0.15,
          },
        ],
      },
      options: baseChartOpts(colors),
    };
    chartLatency = new Chart(
      requireElement("chart-latency", HTMLCanvasElement),
      config
    );
  }
}

// Chart.js leaves a chart without a context when its canvas has none (jsdom
// has no canvas, so the client tests run without one), and updating such a
// chart throws; the charts then stay empty, as when Chart.js was a separate
// script that failed to load.
function drawable(chart: LineChart | null): chart is LineChart {
  return chart !== null && chart.ctx != null;
}

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
 */
function updateChartData(series: SeriesWindow, averageLatency: boolean, targetUsers: number): void {
  const start = series.start_second;
  const rps = recentTrend(series.rps);
  setSummary(
    rpsSummary,
    series.rps.length === 0
      ? "No data yet."
      : withTrend("RPS " + formatInt(rps.mean), rps.trend) +
          "; failures " + formatRate(recentTrend(series.fps).mean) + "/s."
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

/**
 * Draws the charts; the average latency chart only when `averageLatency`
 * (percentiles over time are off and the response time card shows it).
 * `targetUsers` is the users summary's "of".
 */
export function updateCharts(
  series: SeriesWindow,
  averageLatency: boolean,
  targetUsers: number
): void {
  updateChartData(series, averageLatency, targetUsers);
  ensureCharts();
  const start = series.start_second;
  const rps = series.rps;
  const fps = series.fps;
  const users = series.users;
  const lat = series.avg_latency_ms;
  const len = Math.max(rps.length, fps.length, users.length, lat.length);
  const labels: string[] = [];
  for (let i = 0; i < len; i++) {
    labels.push(String(start + i));
  }

  if (drawable(chartRps)) {
    chartRps.data.labels = labels;
    chartRps.data.datasets[0].data = rps;
    chartRps.data.datasets[1].data = fps;
    chartRps.update("none");
  }
  if (drawable(chartUsers)) {
    chartUsers.data.labels = labels;
    chartUsers.data.datasets[0].data = users;
    chartUsers.update("none");
  }
  if (averageLatency && drawable(chartLatency)) {
    chartLatency.data.labels = labels;
    chartLatency.data.datasets[0].data = lat;
    chartLatency.update("none");
  }
}
