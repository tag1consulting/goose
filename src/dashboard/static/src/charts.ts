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
  type ChartDataset,
  type ChartOptions,
  type LegendItem,
  type Plugin,
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

type LineChart = Chart<"line", (number | null)[], string>;

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

/** A Gaggle event the RPS chart marks: a thin dashed line at its second. */
export interface ChartMarker {
  /** The series second it happened in. */
  second: number;
  /** A few words drawn at the top of the line. */
  label: string;
  /** The event's sentence, for hover and focus. */
  text: string;
  /** Drawn in the error colour: a worker lost. */
  alarm: boolean;
}

/** The RPS chart's lines by worker, on a Gaggle manager. */
export interface ByWorker {
  names: string[];
  /** Each worker's requests in each second, from the series start. */
  lines: number[][];
  /** How many workers `others` adds up. */
  othersWorkers: number;
}

let chartRps: LineChart | null = null;
/** The RPS chart's markers, drawn by the markers plugin. */
let rpsMarkers: ChartMarker[] = [];
/** The second each index of the RPS chart stands for starts at. */
let rpsStart = 0;
/** Whether the RPS chart shows its lines by worker. */
let rpsByWorker = false;
let chartUsers: LineChart | null = null;
let chartLatency: LineChart | null = null;
/** The index of each chart's first provisional second, or null. */
let rpsProvisional: number | null = null;
let latencyProvisional: number | null = null;

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

/**
 * Eight colours for the RPS chart's lines by worker, distinct in hue and
 * readable on the theme's card, then grey for the rest.
 */
export function workerColors(dark: boolean): { lines: string[]; others: string } {
  return dark
    ? {
        lines: ["#3d8bfd", "#3dd68c", "#e6b450", "#c678dd", "#56b6c2", "#f78c6c", "#ff6ac1", "#a3be8c"],
        others: "#6b7785",
      }
    : {
        lines: ["#0b5fff", "#0a7a43", "#9a6700", "#8e44ad", "#0e7490", "#c2410c", "#be185d", "#4d7c0f"],
        others: "#8a96a3",
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

/**
 * Gives each legend item its dataset's dash. With usePointStyle, Chart.js
 * styles a legend item from the dataset's points, which have no dash, so the
 * dashed Failures/s line would otherwise get a solid swatch.
 */
export function withLineDash(
  items: LegendItem[],
  datasets: ReadonlyArray<object>
): LegendItem[] {
  for (const item of items) {
    const dataset = (item.datasetIndex == null ? undefined : datasets[item.datasetIndex]) as
      | { borderDash?: unknown }
      | undefined;
    if (dataset && Array.isArray(dataset.borderDash)) {
      item.lineDash = dataset.borderDash as number[];
    }
  }
  return items;
}

export function baseChartOpts(colors: ChartColors): ChartOptions<"line"> {
  // Fresh object per chart so Chart.js cannot share/mutate nested scales.
  return {
    responsive: true,
    maintainAspectRatio: false,
    animation: false,
    interaction: { mode: "index", intersect: false },
    plugins: {
      legend: {
        // Each swatch is a line drawn with its dataset's dash, so the legend
        // tells the dashed Failures/s line apart without colour. The width is
        // set because Chart.js otherwise caps a point style swatch at the font
        // size, 11 px, too short to show a 6 on, 4 off dash.
        labels: {
          color: colors.tick,
          boxWidth: 12,
          font: { size: 11 },
          usePointStyle: true,
          pointStyle: "line",
          pointStyleWidth: 30,
          generateLabels: (chart) =>
            withLineDash(
              Chart.defaults.plugins.legend.labels.generateLabels(chart),
              chart.data.datasets
            ),
        },
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

/**
 * A per second series without its newest second, which is still filling. A
 * rate there holds a fraction of a second's requests or failures, so the line
 * would end in a drop to near zero on every snapshot. An average there reads
 * low: requests are recorded when they finish, under the second they started
 * in, so that second is still short of its slowest ones.
 */
export function completeSeconds(values: number[]): number[] {
  return values.slice(0, -1);
}

/**
 * The index in `series` of its first provisional second: on a Gaggle
 * manager, from there on each second holds what the workers have sent so
 * far. Null when every second is complete.
 */
export function provisionalIndex(series: SeriesWindow): number | null {
  const from = series.provisional_from;
  return from === undefined || from === null ? null : Math.max(0, from - series.start_second);
}

/**
 * `values` drawn with the provisional seconds from index `from` on: one with
 * no data yet, where `empty` says so, is a gap, never 0. With `from` null
 * every value stays.
 */
export function provisionalValues(
  values: number[],
  from: number | null,
  empty: (i: number) => boolean
): (number | null)[] {
  return values.map((value, i) => (from !== null && i >= from && empty(i) ? null : value));
}

/**
 * How many seconds of a window of `length` are provisional from index
 * `from`: what the chart's note says are still arriving.
 */
export function provisionalSeconds(length: number, from: number | null): number {
  return from === null ? 0 : Math.max(0, length - from);
}

/** A colour of the charts', `#rrggbb`, half as strong. */
export function lighter(color: string): string {
  return /^#[0-9a-f]{6}$/i.test(color) ? color + "80" : color;
}

/**
 * A line's style from its first provisional second on: lighter, and dashed
 * as `dash` says, which differs from the line's own.
 */
function provisionalSegment(
  color: () => string,
  dash: number[],
  from: () => number | null
): object {
  return {
    borderColor: (ctx: { p1DataIndex: number }) => {
      const at = from();
      return at !== null && ctx.p1DataIndex >= at ? lighter(color()) : undefined;
    },
    borderDash: (ctx: { p1DataIndex: number }) => {
      const at = from();
      return at !== null && ctx.p1DataIndex >= at ? dash : undefined;
    },
  };
}

/**
 * The RPS chart's two lines. Failures/s is dashed so the lines differ by more
 * than colour.
 */
export function rpsDatasets(
  colors: ChartColors
): ChartConfiguration<"line", (number | null)[], string>["data"]["datasets"] {
  return [
    {
      label: "RPS",
      data: [],
      borderColor: colors.rps,
      backgroundColor: colors.rps,
      borderWidth: 1.5,
      pointRadius: 0,
      tension: 0.15,
      segment: provisionalSegment(() => chartColors().rps, [3, 3], () => rpsProvisional),
    },
    {
      label: "Failures/s",
      data: [],
      borderColor: colors.fps,
      backgroundColor: colors.fps,
      borderWidth: 1.5,
      borderDash: [6, 4],
      pointRadius: 0,
      tension: 0.15,
      segment: provisionalSegment(() => chartColors().fps, [2, 3], () => rpsProvisional),
    },
  ];
}

/**
 * Which of `labels`, each spanning `[x, x + width]` in pixels and sorted by
 * `x`, are drawn: each one that clears the last one kept by `gap` pixels.
 * The rest are dropped from the top row; their sentences stay on hover and
 * in the data table.
 */
export function keptLabels(labels: { x: number; width: number }[], gap: number): boolean[] {
  let end = -Infinity;
  return labels.map((label) => {
    if (label.x < end + gap) return false;
    end = label.x + label.width;
    return true;
  });
}

/** The pixel x of index `at` of `chart`'s category axis, or null. */
function indexX(chart: Chart, at: number): number | null {
  const scale = chart.scales.x;
  if (!scale) return null;
  const x = scale.getPixelForValue(at);
  return Number.isFinite(x) ? x : null;
}

/** The markers of the RPS chart within its drawn seconds, with their x. */
function placedMarkers(chart: Chart): { marker: ChartMarker; x: number }[] {
  const count = (chart.data.labels || []).length;
  const placed: { marker: ChartMarker; x: number }[] = [];
  for (const marker of rpsMarkers) {
    const at = marker.second - rpsStart;
    if (at < 0 || at >= count) continue;
    const x = indexX(chart, at);
    if (x !== null) placed.push({ marker, x });
  }
  return placed.sort((a, b) => a.x - b.x);
}

/**
 * Draws the RPS chart's Gaggle markers: a thin dashed line at each event's
 * second, its label at the top where it fits, in the error colour for a
 * lost worker.
 */
const markersPlugin: Plugin<"line"> = {
  id: "gaggleMarkers",
  afterDatasetsDraw(chart) {
    const area = chart.chartArea;
    const ctx = chart.ctx;
    if (!area || !ctx || rpsMarkers.length === 0) return;
    const colors = chartColors();
    const alarm = isDark() ? "#f07178" : "#b42318";
    const placed = placedMarkers(chart as unknown as Chart);
    ctx.save();
    ctx.font = "10px " + getComputedStyleFont();
    const widths = placed.map((p) => ctx.measureText(p.marker.label).width);
    const kept = keptLabels(
      placed.map((p, i) => ({ x: p.x + 3, width: widths[i] })),
      6
    );
    placed.forEach((p, i) => {
      const color = p.marker.alarm ? alarm : colors.tick;
      ctx.strokeStyle = color;
      ctx.lineWidth = 1;
      ctx.setLineDash([3, 3]);
      ctx.beginPath();
      ctx.moveTo(p.x, area.top);
      ctx.lineTo(p.x, area.bottom);
      ctx.stroke();
      if (kept[i]) {
        ctx.setLineDash([]);
        ctx.fillStyle = color;
        ctx.textBaseline = "top";
        ctx.fillText(p.marker.label, Math.min(p.x + 3, area.right - widths[i]), area.top + 2);
      }
    });
    ctx.restore();
  },
};

/** The page's font family, for the markers' labels. */
function getComputedStyleFont(): string {
  try {
    return getComputedStyle(document.body).fontFamily || "sans-serif";
  } catch {
    return "sans-serif";
  }
}

/**
 * The marker of the RPS chart within `slack` pixels of `x` on its canvas,
 * nearest first, or null.
 */
export function markerNear(x: number, slack: number): ChartMarker | null {
  if (!drawable(chartRps)) return null;
  let best: { marker: ChartMarker; d: number } | null = null;
  for (const p of placedMarkers(chartRps as unknown as Chart)) {
    const d = Math.abs(p.x - x);
    if (d <= slack && (best === null || d < best.d)) best = { marker: p.marker, d };
  }
  return best ? best.marker : null;
}

/** The pixel x of `marker` on the RPS chart's canvas, or null. */
export function markerX(marker: ChartMarker): number | null {
  if (!drawable(chartRps)) return null;
  return indexX(chartRps as unknown as Chart, marker.second - rpsStart);
}

/** Sets the RPS chart's markers, drawn at the next update. */
export function setRpsMarkers(markers: ChartMarker[]): void {
  rpsMarkers = markers;
}

/**
 * The RPS chart's datasets by worker: each worker's line, then the others,
 * stacked and filled, so the top edge is the total.
 */
function byWorkerDatasets(view: ByWorker, others: number[]): ChartDataset<"line", (number | null)[]>[] {
  const palette = workerColors(isDark());
  const line = (label: string, data: number[], color: string): ChartDataset<"line", (number | null)[]> => ({
    label,
    data,
    borderColor: color,
    backgroundColor: lighter(color),
    borderWidth: 1,
    pointRadius: 0,
    tension: 0.15,
    fill: true,
    stack: "workers",
  });
  const sets = view.names.map((name, i) => line(name, view.lines[i] || [], palette.lines[i % palette.lines.length]));
  if (view.othersWorkers > 0) {
    sets.push(line(view.othersWorkers.toLocaleString("en-US") + " others", others, palette.others));
  }
  return sets;
}

/**
 * Shows the RPS chart by worker, from `view`, or its total when `view` is
 * null: the datasets are swapped only when the view changes, and updated in
 * place otherwise.
 */
function applyRpsView(chart: LineChart, view: ByWorker | null, series: SeriesWindow): void {
  const y = chart.options.scales && chart.options.scales.y;
  if (view === null) {
    if (rpsByWorker) {
      chart.data.datasets = rpsDatasets(chartColors());
      if (y) (y as { stacked?: boolean }).stacked = false;
      rpsByWorker = false;
    }
    return;
  }
  const len = Math.max(0, series.rps.length - 1);
  const shown = view.lines.map((line) => line.slice(0, len));
  const others: number[] = [];
  for (let i = 0; i < len; i++) {
    let sum = 0;
    for (const line of shown) sum += line[i] || 0;
    others.push(Math.max(0, series.rps[i] - sum));
  }
  const next = byWorkerDatasets({ ...view, lines: shown }, others);
  const same =
    rpsByWorker &&
    chart.data.datasets.length === next.length &&
    chart.data.datasets.every((set, i) => set.label === next[i].label);
  if (same) {
    chart.data.datasets.forEach((set, i) => {
      set.data = next[i].data;
    });
  } else {
    chart.data.datasets = next;
  }
  if (y) (y as { stacked?: boolean }).stacked = true;
  rpsByWorker = true;
}

/** The RPS chart's lines by worker, or null to show its total. */
let rpsView: ByWorker | null = null;

/** Shows the RPS chart by worker from the next update, or its total. */
export function setRpsByWorker(view: ByWorker | null): void {
  rpsView = view;
}

export function ensureCharts(): void {
  const colors = chartColors();

  if (!chartRps) {
    const config: ChartConfiguration<"line", (number | null)[], string> = {
      type: "line",
      data: {
        labels: [],
        datasets: rpsDatasets(colors),
      },
      options: baseChartOpts(colors),
      plugins: [markersPlugin],
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
    const config: ChartConfiguration<"line", (number | null)[], string> = {
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
            segment: provisionalSegment(
              () => chartColors().latency,
              [3, 3],
              () => latencyProvisional
            ),
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
 * Draws the charts; the average latency chart only when `averageLatency`
 * (percentiles over time are off and the response time card shows it).
 */
export function updateCharts(series: SeriesWindow, averageLatency: boolean): void {
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

  // On a Gaggle manager the newest seconds are still arriving: drawn
  // lighter and dashed, and a second with nothing yet is a gap, never 0.
  const provisional = provisionalIndex(series);
  rpsProvisional = provisional;
  latencyProvisional = provisional;
  if (drawable(chartRps)) {
    const rpsComplete = completeSeconds(rps);
    const fpsComplete = completeSeconds(fps);
    const empty = (i: number): boolean => !rpsComplete[i] && !fpsComplete[i];
    rpsStart = start;
    chartRps.data.labels = labels.slice(0, Math.max(rpsComplete.length, fpsComplete.length));
    applyRpsView(chartRps, rpsView, series);
    if (!rpsByWorker) {
      chartRps.data.datasets[0].data = provisionalValues(rpsComplete, provisional, empty);
      chartRps.data.datasets[1].data = provisionalValues(fpsComplete, provisional, empty);
    }
    chartRps.update("none");
  }
  if (drawable(chartUsers)) {
    chartUsers.data.labels = labels;
    chartUsers.data.datasets[0].data = users;
    chartUsers.update("none");
  }
  if (averageLatency && drawable(chartLatency)) {
    const latComplete = completeSeconds(lat);
    chartLatency.data.labels = labels.slice(0, latComplete.length);
    chartLatency.data.datasets[0].data = provisionalValues(
      latComplete,
      provisional,
      (i) => !latComplete[i]
    );
    chartLatency.update("none");
  }
}

/**
 * Sets every colour `baseChartOpts` and the datasets carry to `colors`, the
 * datasets in `lines` order, and redraws the chart. The chart, its data and
 * its other options stay as they are.
 */
function recolor(chart: LineChart | null, colors: ChartColors, lines: string[]): void {
  if (!drawable(chart)) return;
  chart.data.datasets.forEach((dataset, i) => {
    dataset.borderColor = lines[i];
    dataset.backgroundColor = lines[i];
  });
  const options = chart.options;
  const labels = options.plugins && options.plugins.legend && options.plugins.legend.labels;
  if (labels) labels.color = colors.tick;
  const scales = options.scales || {};
  for (const axis of [scales.x, scales.y]) {
    if (!axis) continue;
    if (axis.ticks) axis.ticks.color = colors.tick;
    if (axis.grid) axis.grid.color = colors.grid;
  }
  chart.update("none");
}

/**
 * Recolours the charts for the current theme, in place: their data stays and
 * nothing is rebuilt.
 */
export function recolorCharts(): void {
  const colors = chartColors();
  if (rpsByWorker && rpsView !== null && drawable(chartRps)) {
    // Rebuilt in the theme's colours: the lines by worker are filled.
    const palette = workerColors(isDark());
    const workers = rpsView.names.length;
    const lines = chartRps.data.datasets.map((_, i) =>
      i < workers ? palette.lines[i % palette.lines.length] : palette.others
    );
    recolor(chartRps, colors, lines);
    chartRps.data.datasets.forEach((dataset, i) => {
      dataset.backgroundColor = lighter(lines[i]);
    });
    chartRps.update("none");
  } else {
    recolor(chartRps, colors, [colors.rps, colors.fps]);
  }
  recolor(chartUsers, colors, [colors.users]);
  recolor(chartLatency, colors, [colors.latency]);
}
