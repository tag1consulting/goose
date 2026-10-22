// Response times by second as a heatmap (#713). Each column is one second of
// the window, each row one of Goose's response time buckets on a logarithmic
// latency axis, and the colour of a cell how many requests in that second took
// that long, on a logarithmic count scale.
//
// Everything above drawHeatmap is data shaping with no DOM access, so the
// client tests load this module in Node.

/** The card's title, and its note, always shown with the heatmap. */
export const TITLE = "Response times by second";
export const NOTE =
  "Each column is one second. Color shows how many requests took that long; the lines are that second's p50, p95 and p99.";
/** The note shown under the title while the heatmap shows adjusted times. */
export const ADJUSTED_NOTE =
  "Adjusted: includes the synthetic times coordinated omission mitigation adds.";
/** The note shown with the average chart when percentiles are off. */
export const OFF_NOTE =
  "Percentiles are off for this run (--no-percentiles-over-time).";
export const ARIA_LABEL = "Response times by second, heatmap";

/** One request, as the select and `GET /api/v1/series` name it. */
export interface SelectedRequest {
  method: string;
  name: string;
}

/** The heatmap a client holds: the columns of every update applied so far. */
export interface HeatmapState {
  generation: number;
  rev: number;
  startSecond: number;
  endSecond: number;
  holdFrom: number;
  /**
   * `[time, lowest, highest]` in milliseconds for each bucket index, from
   * `buckets_us`; `highest + 1` is where the bucket ends.
   */
  buckets: number[][];
  columns: Map<number, HeatmapColumn>;
}

/** What to send as `heatmap_generation` and `heatmap_since`. */
export interface HeatmapCursor {
  generation: number;
  since: number;
}

/**
 * The state after an update from the server, or null when the update follows
 * on from a heatmap other than `state` (the client then asks again with its
 * cursor). An update with `since` 0 replaces whatever was held.
 */
export function applyHeatmap(
  state: HeatmapState | null,
  heatmap: Heatmap
): HeatmapState | null {
  let columns: Map<number, HeatmapColumn>;
  let buckets: number[][];
  const table = bucketTable(heatmap);
  if (heatmap.since === 0) {
    columns = new Map();
    buckets = table;
  } else if (
    state !== null &&
    state.generation === heatmap.generation &&
    state.rev === heatmap.since
  ) {
    columns = new Map(state.columns);
    buckets = table.length >= state.buckets.length ? table : state.buckets;
  } else {
    return null;
  }
  for (const column of heatmap.columns) {
    columns.set(column.second, column);
  }
  for (const second of Array.from(columns.keys())) {
    if (second < heatmap.start_second) columns.delete(second);
  }
  return {
    generation: heatmap.generation,
    rev: heatmap.rev,
    startSecond: heatmap.start_second,
    endSecond: heatmap.end_second,
    holdFrom: heatmap.hold_from,
    buckets,
    columns,
  };
}

/**
 * The bucket table in milliseconds: each microsecond entry of `buckets_us`,
 * `[time, lowest, highest]`, as milliseconds with `highest` one millisecond
 * short of where the bucket ends.
 */
export function bucketTable(heatmap: Heatmap): number[][] {
  return heatmap.buckets_us.map(([time, lowest, highest]) => [
    time / 1000,
    lowest / 1000,
    (highest + 1) / 1000 - 1,
  ]);
}

export function heatmapCursor(state: HeatmapState | null): HeatmapCursor | null {
  return state === null ? null : { generation: state.generation, since: state.rev };
}

/** One drawn cell: a second and a bucket, with its request counts. */
export interface HeatmapCell {
  /** Seconds from the window's first, so the column drawn. */
  column: number;
  second: number;
  /** The bucket index. */
  bucket: number;
  /** Requests shown: measured, plus synthetic in the adjusted view. */
  count: number;
  /** Of `count`, the synthetic ones (0 in the measured view). */
  synthetic: number;
}

/** One bucket's row, placed on the latency axis. */
export interface HeatmapRow {
  bucket: number;
  /** The time the bucket stands for, its middle, and its range of measured times, in ms. */
  time: number;
  lowest: number;
  highest: number;
  /** The row's top and bottom edges, as fractions of the plot from the top. */
  top: number;
  bottom: number;
}

/** A drawn second's p50, p95 and p99, in milliseconds. */
export interface SecondPercentiles {
  second: number;
  p50: number;
  p95: number;
  p99: number;
}

export interface HeatmapTick {
  ms: number;
  label: string;
  /** Where the tick sits, as a fraction of the plot from the top. */
  y: number;
}

export interface HeatmapView {
  startSecond: number;
  /** Seconds in the window, so the number of columns. */
  columns: number;
  /** Rows from the fastest bucket with requests to the slowest. */
  rows: HeatmapRow[];
  cells: HeatmapCell[];
  /** Lowest and highest count of any cell, 0 without cells. */
  minCount: number;
  maxCount: number;
  ticks: HeatmapTick[];
  /** Requests shown in each drawn second, for the tooltip's share. */
  secondTotals: Map<number, number>;
  /** Each drawn second's percentiles, by second; none for an empty one. */
  percentiles: Map<number, SecondPercentiles>;
}

/** Latency axis ticks, in milliseconds. */
export const TICKS_MS = [
  0.1, 0.2, 0.5, 1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000, 60000,
];

export function tickLabel(ms: number): string {
  return ms < 1000 ? ms + " ms" : ms / 1000 + " s";
}

/**
 * A bucket's span on the latency axis: from its lowest measured time to just
 * past its highest. Bucket 0 starts at 0, which a log axis cannot reach down
 * to, so it spans from half its end, 0.05 to 0.1 ms.
 */
export function bucketSpan(bucket: number[]): [number, number] {
  const lowest = bucket[1];
  const end = bucket[2] + 1;
  return [lowest === 0 ? end / 2 : lowest, end];
}

/** Adds flattened `index, count` pairs into `into`. */
function addPairs(into: Map<number, [number, number]>, cells: number[], synthetic: boolean): void {
  for (let i = 0; i + 1 < cells.length; i += 2) {
    const entry = into.get(cells[i]) || [0, 0];
    entry[0] += cells[i + 1];
    if (synthetic) entry[1] += cells[i + 1];
    into.set(cells[i], entry);
  }
}

/**
 * The cells, rows and ticks to draw from a heatmap, measured times only or,
 * when `adjusted`, measured and synthetic together. Seconds from `holdFrom`
 * on are still filling and are not drawn; a second with no requests has no
 * cells.
 */
export function heatmapView(state: HeatmapState | null, adjusted: boolean): HeatmapView {
  const empty: HeatmapView = {
    startSecond: state ? state.startSecond : 0,
    columns: state ? state.endSecond - state.startSecond + 1 : 0,
    rows: [],
    cells: [],
    minCount: 0,
    maxCount: 0,
    ticks: [],
    secondTotals: new Map(),
    percentiles: new Map(),
  };
  if (state === null) return empty;
  const cells: HeatmapCell[] = [];
  const secondTotals = new Map<number, number>();
  let low = Infinity;
  let high = -Infinity;
  let minCount = Infinity;
  let maxCount = 0;
  const seconds = Array.from(state.columns.keys()).sort((a, b) => a - b);
  for (const second of seconds) {
    if (second < state.startSecond || second > state.endSecond) continue;
    if (second >= state.holdFrom) continue;
    const column = state.columns.get(second) as HeatmapColumn;
    const buckets = new Map<number, [number, number]>();
    addPairs(buckets, column.measured, false);
    if (adjusted) addPairs(buckets, column.synthetic, true);
    let total = 0;
    const indexes = Array.from(buckets.keys()).sort((a, b) => a - b);
    for (const bucket of indexes) {
      const [count, synthetic] = buckets.get(bucket) as [number, number];
      if (count === 0) continue;
      cells.push({ column: second - state.startSecond, second, bucket, count, synthetic });
      total += count;
      low = Math.min(low, bucket);
      high = Math.max(high, bucket);
      minCount = Math.min(minCount, count);
      maxCount = Math.max(maxCount, count);
    }
    if (total > 0) secondTotals.set(second, total);
  }
  if (cells.length === 0) return empty;

  const axisLow = Math.log(bucketSpan(bucketOf(state, low))[0]);
  const axisHigh = Math.log(bucketSpan(bucketOf(state, high))[1]);
  const fraction = (ms: number): number =>
    (axisHigh - Math.log(ms)) / (axisHigh - axisLow);
  const rows: HeatmapRow[] = [];
  for (let bucket = low; bucket <= high; bucket++) {
    const entry = bucketOf(state, bucket);
    const [from, to] = bucketSpan(entry);
    rows.push({
      bucket,
      time: entry[0],
      lowest: entry[1],
      highest: entry[2],
      top: fraction(to),
      bottom: fraction(from),
    });
  }
  const ticks: HeatmapTick[] = [];
  for (const ms of TICKS_MS) {
    const y = fraction(ms);
    if (y >= 0 && y <= 1) ticks.push({ ms, label: tickLabel(ms), y });
  }
  return {
    startSecond: state.startSecond,
    columns: state.endSecond - state.startSecond + 1,
    rows,
    cells,
    minCount,
    maxCount,
    ticks,
    secondTotals,
    percentiles: secondPercentiles(cells, rows),
  };
}

/**
 * The response time at `percent` (0 to 1) of `total` requests whose counts
 * by bucket are `buckets`, as `[time, count]` in ascending bucket order, by
 * Goose's own rule for its whole run figures (`response_time_percentile` in
 * src/metrics.rs): the rank is `total * percent` rounded, in doubles as
 * Rust's `f64`, and the answer the time of the first bucket whose running
 * count reaches it.
 */
export function percentileOf(
  buckets: Array<[number, number]>,
  total: number,
  percent: number
): number {
  const rank = Math.round(total * percent);
  let running = 0;
  for (const [time, count] of buckets) {
    running += count;
    if (running >= rank) return time;
  }
  return 0;
}

/**
 * Each drawn second's p50, p95 and p99 from the same counts its cells show:
 * with synthetic times in the adjusted view, without them in the measured
 * one. `cells` are in order of second, then bucket, as heatmapView builds
 * them; a second with no cells has no entry.
 */
export function secondPercentiles(
  cells: HeatmapCell[],
  rows: HeatmapRow[]
): Map<number, SecondPercentiles> {
  const times = new Map<number, number>();
  for (const row of rows) times.set(row.bucket, row.time);
  const bySecond = new Map<number, Array<[number, number]>>();
  for (const cell of cells) {
    let buckets = bySecond.get(cell.second);
    if (buckets === undefined) {
      buckets = [];
      bySecond.set(cell.second, buckets);
    }
    buckets.push([times.get(cell.bucket) as number, cell.count]);
  }
  const result = new Map<number, SecondPercentiles>();
  for (const [second, buckets] of bySecond) {
    const total = buckets.reduce((sum, [, count]) => sum + count, 0);
    result.set(second, {
      second,
      p50: percentileOf(buckets, total, 0.5),
      p95: percentileOf(buckets, total, 0.95),
      p99: percentileOf(buckets, total, 0.99),
    });
  }
  return result;
}

/**
 * A heatmap percentile, the middle of its row, in milliseconds at the rows'
 * precision: to the tenth under 1 ms and whole from 1 ms, truncated, so it
 * names the row it is in.
 */
export function rowMs(ms: number): string {
  if (!isFinite(ms)) return "—";
  const us = Math.round(ms * 1000);
  if (us < 1000) {
    return (Math.floor(us / 100) / 10).toLocaleString(undefined, { maximumFractionDigits: 1 });
  }
  return Math.floor(us / 1000).toLocaleString();
}

/** Steps between second labels: the smallest that fits is used. */
export const SECOND_STEPS = [1, 2, 5, 10, 15, 30, 60, 120, 300];

/** The step between second labels for a window of `columns` seconds. */
export function secondStep(columns: number): number {
  for (const step of SECOND_STEPS) {
    if (Math.ceil(columns / step) <= 8) return step;
  }
  return SECOND_STEPS[SECOND_STEPS.length - 1];
}

/**
 * The seconds to label in a window of `columns` seconds from `startSecond`:
 * every multiple of the step counted from the test start, so the labels stay
 * on the same seconds as the window slides.
 */
export function secondLabels(startSecond: number, columns: number): number[] {
  const step = secondStep(columns);
  const labels: number[] = [];
  const first = Math.ceil(startSecond / step) * step;
  for (let second = first; second < startSecond + columns; second += step) {
    labels.push(second);
  }
  return labels;
}

/** A bucket's table entry, or one built past the table's end (1 s apart). */
function bucketOf(state: HeatmapState, bucket: number): number[] {
  const entry = state.buckets[bucket];
  if (entry !== undefined) return entry;
  // The table always holds every fixed bucket, and the last ones are 1 s
  // apart, so a bucket past it is that many seconds past its last entry.
  const last = state.buckets.length - 1;
  const time = Math.round(state.buckets[last][0]) + (bucket - last) * 1000;
  return [time, time - 500, time + 499];
}

/** One sequential hue, light to dark in the light theme, dark to bright in the dark. */
const HUE = 222;
const RAMP = {
  light: { from: [80, 64], to: [85, 22] },
  dark: { from: [75, 56], to: [100, 94] },
};

/**
 * Where `count` sits on the colour scale, from 0 for `min`, the sparsest
 * drawn cell, to 1 for `max`, the busiest, on a logarithmic scale. When every
 * cell holds the same count, all are the highest.
 */
export function countPosition(count: number, min: number, max: number): number {
  if (max <= min) return 1;
  return Math.min(1, Math.max(0, Math.log(count / min) / Math.log(max / min)));
}

/** The colour at position `t` (0 to 1) of the scale, as an `hsl()` string. */
export function scaleColor(t: number, dark: boolean): string {
  const ramp = dark ? RAMP.dark : RAMP.light;
  const s = ramp.from[0] + (ramp.to[0] - ramp.from[0]) * t;
  const l = ramp.from[1] + (ramp.to[1] - ramp.from[1]) * t;
  return "hsl(" + HUE + ", " + s.toFixed(1) + "%, " + l.toFixed(1) + "%)";
}

/**
 * The colour of a cell holding `count` requests when the sparsest drawn cell
 * holds `min` and the busiest `max`.
 */
export function cellColor(count: number, min: number, max: number, dark: boolean): string {
  return scaleColor(countPosition(count, min, max), dark);
}

/** Number formats, as the tables use them (format.ts); passed in so this module needs no other. */
export interface Formats {
  int: (n: number) => string;
  pct: (fraction: number) => string;
  /** A response time in milliseconds, to the hundredth under 10 ms; `int` when absent. */
  ms?: (n: number) => string;
}

/** A cell's tooltip. */
export function tooltipText(
  view: HeatmapView,
  cell: HeatmapCell,
  row: HeatmapRow,
  adjusted: boolean,
  formats: Formats
): string {
  const ms = formats.ms || formats.int;
  let bucket: string;
  if (row.highest + 1 <= 1) {
    // A row that ends by 1 ms: its range, in tenths of a millisecond.
    bucket = ms(row.lowest) + " to " + ms(row.highest + 1) + " ms";
  } else if (row.lowest === row.highest) {
    bucket = formats.int(row.lowest) + " ms";
  } else {
    bucket =
      formats.int(row.time) +
      " ms bucket (" +
      formats.int(row.lowest) +
      " to " +
      formats.int(row.highest) +
      " ms)";
  }
  const total = view.secondTotals.get(cell.second) || cell.count;
  let text =
    "Second " +
    cell.second +
    ": " +
    bucket +
    ", " +
    formats.int(cell.count) +
    " requests";
  if (adjusted && cell.synthetic > 0) {
    text += ", of which " + formats.int(cell.synthetic) + " synthetic";
  }
  text +=
    " (" + formats.pct(cell.count / total) + " of the second's " + formats.int(total) + ")";
  const percentiles = view.percentiles.get(cell.second);
  if (percentiles !== undefined) {
    text +=
      " p50 / p95 / p99: " +
      rowMs(percentiles.p50) +
      " / " +
      rowMs(percentiles.p95) +
      " / " +
      rowMs(percentiles.p99) +
      " ms";
  }
  return text;
}

// ---------------------------------------------------------------------------
// Drawing. Everything below needs a canvas.
// ---------------------------------------------------------------------------

/** Where the last drawing put things, for finding the cell under the pointer. */
export interface HeatmapLayout {
  view: HeatmapView;
  /** The plot area in CSS pixels, inside the canvas. */
  left: number;
  top: number;
  width: number;
  height: number;
  cells: Map<string, HeatmapCell>;
}

/** The right margin holds the percentile lines' labels. */
const MARGIN = { left: 52, right: 72, top: 6, bottom: 22 };

export type PercentileKey = "p50" | "p95" | "p99";

/**
 * The percentile lines, in drawing order. All three share one colour and one
 * width and differ by pattern alone, so they are told apart without colour:
 * p50 solid, p95 dashed, p99 dotted. Where two lines meet in one row, the gaps
 * of the line on top let the one below show through.
 */
export const PERCENTILE_LINES: ReadonlyArray<{ key: PercentileKey; width: number; dash: number[] }> = [
  { key: "p50", width: 2, dash: [] },
  { key: "p95", width: 2, dash: [7, 4] },
  { key: "p99", width: 2, dash: [2, 3] },
];

/**
 * The newest drawn second's p50, p95 and p99, where the lines end, with no
 * height yet; none when the window has no percentiles.
 */
export function newestPercentiles(
  view: HeatmapView
): Array<{ key: PercentileKey; value: number; y: number }> {
  for (let c = view.columns - 1; c >= 0; c--) {
    const percentiles = view.percentiles.get(view.startSecond + c);
    if (percentiles) {
      return PERCENTILE_LINES.map((line) => ({ key: line.key, value: percentiles[line.key], y: 0 }));
    }
  }
  return [];
}

/** A label at the plot's right edge, for one or more lines ending in one row. */
export interface PercentileLabel {
  text: string;
  y: number;
}

/** The caption over the right edge labels, which give the newest second. */
export const LABELS_CAPTION = "last second";

/**
 * The right edge labels for the lines' newest points: lines that end at the
 * same value share one label ("p95, p99: 4 ms"), so no label sits beside a
 * line it does not name; labels run top to bottom by height, none above
 * `minY` (room for the caption), nudged apart when two would sit closer than
 * 12 pixels.
 */
export function percentileLabels(
  newest: ReadonlyArray<{ key: PercentileKey; value: number; y: number }>,
  minY = -Infinity
): PercentileLabel[] {
  const order: PercentileKey[] = ["p50", "p95", "p99"];
  const byValue = new Map<number, { keys: PercentileKey[]; y: number }>();
  for (const point of newest) {
    const group = byValue.get(point.value);
    if (group) group.keys.push(point.key);
    else byValue.set(point.value, { keys: [point.key], y: point.y });
  }
  const labels = [...byValue.entries()].map(([value, group]) => ({
    text:
      group.keys.sort((a, b) => order.indexOf(a) - order.indexOf(b)).join(", ") +
      ": " +
      rowMs(value) +
      " ms",
    y: group.y,
  }));
  labels.sort((a, b) => a.y - b.y);
  if (labels.length && labels[0].y < minY) labels[0].y = minY;
  for (let i = 1; i < labels.length; i++) {
    if (labels[i].y - labels[i - 1].y < 12) labels[i].y = labels[i - 1].y + 12;
  }
  return labels;
}


function cellKey(column: number, bucket: number): string {
  return column + ":" + bucket;
}

/**
 * Draws `view` on `canvas`, sized to its box, and returns where things went;
 * null when the canvas has no 2D context (jsdom).
 */
export function drawHeatmap(
  canvas: HTMLCanvasElement,
  view: HeatmapView,
  dark: boolean,
  colors: { tick: string; grid: string; percentile: string }
): HeatmapLayout | null {
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  const ratio = window.devicePixelRatio || 1;
  const cssWidth = canvas.clientWidth || 600;
  const cssHeight = canvas.clientHeight || 300;
  canvas.width = Math.round(cssWidth * ratio);
  canvas.height = Math.round(cssHeight * ratio);
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  ctx.clearRect(0, 0, cssWidth, cssHeight);

  const font = getComputedStyle(canvas).fontFamily;
  // The right margin fits the widest label and the caption, so a shared one
  // ("p50, p95, p99: 200 ms") is never cut off at the edge.
  ctx.font = "11px " + font;
  let labelWidth = 0;
  const labelsNow = percentileLabels(newestPercentiles(view));
  for (const label of labelsNow) {
    labelWidth = Math.max(labelWidth, ctx.measureText(label.text).width);
  }
  if (labelsNow.length) {
    ctx.font = "10px " + font;
    labelWidth = Math.max(labelWidth, ctx.measureText(LABELS_CAPTION).width);
  }
  const right = Math.max(MARGIN.right, Math.ceil(labelWidth) + 12);

  const left = MARGIN.left;
  const top = MARGIN.top;
  const width = Math.max(1, cssWidth - MARGIN.left - right);
  const height = Math.max(1, cssHeight - MARGIN.top - MARGIN.bottom);
  const layout: HeatmapLayout = { view, left, top, width, height, cells: new Map() };
  const card =
    getComputedStyle(canvas).getPropertyValue("--card").trim() || (dark ? "#1a2332" : "#ffffff");

  // Every edge on a whole pixel from one function, so neighbouring cells
  // share their edges exactly and no seam shows between them.
  const columns = Math.max(1, view.columns);
  const columnWidth = width / columns;
  const x = (c: number): number => Math.round(left + c * columnWidth);
  const y = (f: number): number => Math.round(top + f * height);

  const rows = new Map<number, HeatmapRow>();
  for (const row of view.rows) rows.set(row.bucket, row);
  for (const cell of view.cells) {
    const row = rows.get(cell.bucket) as HeatmapRow;
    const x0 = x(cell.column);
    const y0 = y(row.top);
    ctx.fillStyle = cellColor(cell.count, view.minCount, view.maxCount, dark);
    // At least a pixel each way, so a single request is never lost.
    ctx.fillRect(
      x0,
      y0,
      Math.max(1, x(cell.column + 1) - x0),
      Math.max(1, y(row.bottom) - y0)
    );
    layout.cells.set(cellKey(cell.column, cell.bucket), cell);
  }

  // Latency ticks, with grid lines over the cells as a faint overlay.
  ctx.font = "10px " + font;
  ctx.fillStyle = colors.tick;
  ctx.strokeStyle = colors.grid;
  ctx.lineWidth = 1;
  ctx.setLineDash([]);
  ctx.textAlign = "right";
  ctx.textBaseline = "middle";
  for (const tick of view.ticks) {
    const tickY = y(tick.y) + 0.5;
    // The <1 ms label names a row; it is not a boundary.
    if (tick.ms > 0) {
      ctx.beginPath();
      ctx.moveTo(left, tickY);
      ctx.lineTo(left + width, tickY);
      ctx.stroke();
    }
    ctx.fillText(tick.label, left - 6, tickY);
  }

  // Second labels along the bottom, at round seconds from the test start.
  ctx.textAlign = "center";
  ctx.textBaseline = "top";
  for (const second of secondLabels(view.startSecond, view.columns)) {
    const c = second - view.startSecond;
    ctx.fillText(String(second), (x(c) + x(c + 1)) / 2, top + height + 6);
  }

  // Each second's p50, p95 and p99 through the column centres, each value
  // in the middle of its bucket's row, broken at a second with no requests.
  const rowsByTime = new Map<number, HeatmapRow>();
  for (const row of view.rows) rowsByTime.set(row.time, row);
  const newestPoints: Array<{ key: PercentileKey; value: number; y: number }> = [];
  const lineSegments: Array<Array<Array<[number, number]>>> = [];
  ctx.lineJoin = "round";
  ctx.lineCap = "butt";
  for (const line of PERCENTILE_LINES) {
    const segments: Array<Array<[number, number]>> = [];
    let segment: Array<[number, number]> = [];
    let newest: { value: number; y: number } | null = null;
    for (let c = 0; c < view.columns; c++) {
      const percentiles = view.percentiles.get(view.startSecond + c);
      const row = percentiles && rowsByTime.get(percentiles[line.key]);
      if (!percentiles || !row) {
        if (segment.length) segments.push(segment);
        segment = [];
        continue;
      }
      const pointY = top + ((row.top + row.bottom) / 2) * height;
      segment.push([(x(c) + x(c + 1)) / 2, pointY]);
      newest = { value: percentiles[line.key], y: pointY };
    }
    if (segment.length) segments.push(segment);
    lineSegments.push(segments);
    if (newest) newestPoints.push({ key: line.key, value: newest.value, y: newest.y });
  }
  const strokeSegments = (segments: Array<Array<[number, number]>>): void => {
    for (const points of segments) {
      ctx.beginPath();
      if (points.length === 1) {
        // A lone second: a short level line across its column.
        const half = Math.max(1, columnWidth / 2);
        ctx.moveTo(points[0][0] - half, points[0][1]);
        ctx.lineTo(points[0][0] + half, points[0][1]);
      } else {
        ctx.moveTo(points[0][0], points[0][1]);
        for (const [pointX, pointY] of points.slice(1)) ctx.lineTo(pointX, pointY);
      }
      ctx.stroke();
    }
  };
  // Halos in the card's colour keep the lines clear of the cells. Every halo
  // goes down before any line, or a later line's halo would hide an earlier
  // line in the same row.
  ctx.strokeStyle = card;
  ctx.setLineDash([]);
  PERCENTILE_LINES.forEach((line, i) => {
    ctx.lineWidth = line.width + 3;
    strokeSegments(lineSegments[i]);
  });
  ctx.strokeStyle = colors.percentile;
  PERCENTILE_LINES.forEach((line, i) => {
    ctx.lineWidth = line.width;
    ctx.setLineDash(line.dash);
    strokeSegments(lineSegments[i]);
  });
  ctx.setLineDash([]);

  // The labels give the newest second's values, under a muted caption that
  // says so, so they do not read as a second key.
  const labels = percentileLabels(newestPoints, top + 19);
  ctx.textAlign = "left";
  ctx.textBaseline = "middle";
  if (labels.length) {
    ctx.fillStyle = colors.tick;
    ctx.font = "10px " + font;
    ctx.fillText(LABELS_CAPTION, left + width + 6, labels[0].y - 14);
  }
  ctx.fillStyle = colors.percentile;
  ctx.font = "11px " + font;
  for (const label of labels) {
    ctx.fillText(label.text, left + width + 6, label.y);
  }
  return layout;
}

/** The cell and row under a point in the canvas's CSS pixels, if any. */
export function cellAt(
  layout: HeatmapLayout,
  x: number,
  y: number
): { cell: HeatmapCell; row: HeatmapRow } | null {
  const { view, left, top, width, height } = layout;
  if (x < left || x >= left + width || y < top || y >= top + height) return null;
  const column = Math.floor(((x - left) / width) * Math.max(1, view.columns));
  const fraction = (y - top) / height;
  for (const row of view.rows) {
    if (fraction >= row.top && fraction <= row.bottom) {
      const cell = layout.cells.get(cellKey(column, row.bucket));
      if (cell) return { cell, row };
    }
  }
  return null;
}
