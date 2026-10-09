// The three line charts: RPS and failures, active users, average latency.

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

export function updateCharts(series: SeriesWindow): void {
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
  if (drawable(chartLatency)) {
    chartLatency.data.labels = labels;
    chartLatency.data.datasets[0].data = lat;
    chartLatency.update("none");
  }
}
