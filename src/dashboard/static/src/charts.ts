// The three line charts: RPS and failures, active users, latency by second.

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
import type { LatencyChart } from "./latency";

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

type LineChart = Chart<"line", (number | null)[], string>;
type LineDataset = ChartConfiguration<"line", (number | null)[], string>["data"]["datasets"][number];

interface ChartColors {
  rps: string;
  fps: string;
  users: string;
  latency: string;
  p50: string;
  p95: string;
  p99: string;
  grid: string;
  tick: string;
}

let chartRps: LineChart | null = null;
let chartUsers: LineChart | null = null;
let chartLatency: LineChart | null = null;

function chartColors(): ChartColors {
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
  return {
    rps: dark ? "#3d8bfd" : "#0b5fff",
    fps: dark ? "#f07178" : "#b42318",
    users: dark ? "#3dd68c" : "#0a7a43",
    latency: dark ? "#e6b450" : "#9a6700",
    p50: dark ? "#7f6df2" : "#8a74f0",
    p95: dark ? "#a996ff" : "#5b3cc4",
    p99: dark ? "#ddd3ff" : "#2e1470",
    grid: dark ? "rgba(154, 167, 184, 0.2)" : "rgba(91, 107, 124, 0.2)",
    tick: dark ? "#9aa7b8" : "#5b6b7c",
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
    const config: ChartConfiguration<"line", (number | null)[], string> = {
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
    const config: ChartConfiguration<"line", (number | null)[], string> = {
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
    const options = baseChartOpts(colors);
    options.spanGaps = false;
    options.plugins = {
      ...options.plugins,
      tooltip: {
        callbacks: {
          footer: (items) =>
            latencyShown && items.length
              ? latencyShown.footer(items[0].dataIndex) || ""
              : "",
        },
      },
    };
    const config: ChartConfiguration<"line", (number | null)[], string> = {
      type: "line",
      data: { labels: [], datasets: latencyDatasets(colors, true) },
      options,
    };
    chartLatency = new Chart(
      requireElement("chart-latency", HTMLCanvasElement),
      config
    );
    latencyPercentiles = true;
  }
}

/** One line of the latency chart. */
function latencyLine(
  label: string,
  color: string,
  width: number,
  dash: number[]
): LineDataset {
  return {
    label,
    data: [],
    borderColor: color,
    backgroundColor: color,
    borderWidth: width,
    borderDash: dash,
    pointRadius: 0,
    tension: 0.15,
  };
}

/**
 * The latency chart's lines: p50, p95, p99, average and maximum (hidden
 * until clicked in the legend), or only the average, solid, when percentiles
 * are off.
 */
function latencyDatasets(colors: ChartColors, percentiles: boolean): LineDataset[] {
  if (!percentiles) return [latencyLine("avg", colors.latency, 1.5, [])];
  const max = latencyLine("max", colors.tick, 1, [2, 3]);
  max.hidden = true;
  return [
    latencyLine("p50", colors.p50, 1.5, []),
    latencyLine("p95", colors.p95, 1.5, []),
    latencyLine("p99", colors.p99, 2, []),
    latencyLine("avg", colors.latency, 1.5, [6, 4]),
    max,
  ];
}

/** Whether the latency chart's lines are the percentile set, once drawn. */
let latencyPercentiles: boolean | null = null;
/** What the latency chart shows now, for its tooltip footer. */
let latencyShown: LatencyChart | null = null;

const latencyTitle = requireElement("chart-latency-title", HTMLElement);
const latencyNote = requireElement("chart-latency-note", HTMLElement);
const latencyCanvas = requireElement("chart-latency", HTMLCanvasElement);

/**
 * Draws the latency chart. The lines are set up again only when percentiles
 * turn on or off, so a line hidden from the legend stays hidden.
 */
export function updateLatencyChart(shown: LatencyChart, labels: string[]): void {
  ensureCharts();
  latencyShown = shown;
  latencyTitle.textContent = shown.title;
  latencyNote.textContent = shown.note;
  latencyNote.hidden = shown.note === "";
  latencyCanvas.setAttribute(
    "aria-label",
    shown.percentiles
      ? "Latency by second: p50, p95, p99 and average"
      : "Latency by second: average"
  );
  if (!drawable(chartLatency)) return;
  if (latencyPercentiles !== shown.percentiles) {
    chartLatency.data.datasets = latencyDatasets(chartColors(), shown.percentiles);
    latencyPercentiles = shown.percentiles;
  }
  const lines = shown.percentiles
    ? [shown.p50, shown.p95, shown.p99, shown.avg, shown.max]
    : [shown.avg];
  chartLatency.data.labels = labels;
  for (let i = 0; i < lines.length; i++) {
    chartLatency.data.datasets[i].data = lines[i];
  }
  chartLatency.update("none");
}

// Chart.js leaves a chart without a context when its canvas has none (jsdom
// has no canvas, so the client tests run without one), and updating such a
// chart throws; the charts then stay empty, as when Chart.js was a separate
// script that failed to load.
function drawable(chart: LineChart | null): chart is LineChart {
  return chart !== null && chart.ctx != null;
}

/** Second labels for a window that starts at `start`. */
export function secondLabels(start: number, len: number): string[] {
  const labels: string[] = [];
  for (let i = 0; i < len; i++) {
    labels.push(String(start + i));
  }
  return labels;
}

/** Draws the requests and users charts; the latency chart draws apart. */
export function updateCharts(series: SeriesWindow): void {
  ensureCharts();
  const rps = series.rps;
  const fps = series.fps;
  const users = series.users;
  const len = Math.max(rps.length, fps.length, users.length);
  const labels = secondLabels(series.start_second, len);

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
}
