// The response time card: the heatmap of all requests or of one request
// chosen in its select, the Adjusted and Measured toggle, and, when
// percentiles over time are off, the average latency chart instead.

import { recentTrend, setLatencySummary } from "./chart-data";
import { chartTheme } from "./charts";
import {
  catchUpHeatmap,
  fetchRequestSeries,
  type HeatmapCursor,
} from "./connection";
import { requireElement } from "./dom";
import { formatInt, formatPct } from "./format";
import {
  ADJUSTED_NOTE,
  ARIA_LABEL,
  OFF_NOTE,
  TITLE,
  applyHeatmap,
  cellAt,
  drawHeatmap,
  heatmapCursor,
  heatmapView,
  scaleColor,
  tooltipText,
  type HeatmapLayout,
  type HeatmapState,
  type HeatmapView,
  type SelectedRequest,
} from "./heatmap";
import { chartDataTable } from "./tables";

/** The card's title when it shows the average latency chart, as before #713. */
const AVERAGE_TITLE = "Average latency (ms)";

const titleEl = requireElement("chart-latency-title", HTMLElement);
const noteEl = requireElement("chart-latency-note", HTMLElement);
const alwaysNoteEl = requireElement("heatmap-note", HTMLElement);
const viewGroup = requireElement("heatmap-view", HTMLElement);
const adjustedButton = requireElement("heatmap-adjusted", HTMLButtonElement);
const measuredButton = requireElement("heatmap-measured", HTMLButtonElement);
const selectEl = requireElement("heatmap-request", HTMLSelectElement);
const wrapEl = requireElement("heatmap-wrap", HTMLElement);
const canvas = requireElement("chart-heatmap", HTMLCanvasElement);
const tooltipEl = requireElement("heatmap-tooltip", HTMLElement);
const legendEl = requireElement("heatmap-legend", HTMLElement);
const legendMin = requireElement("heatmap-legend-min", HTMLElement);
const legendBar = requireElement("heatmap-legend-bar", HTMLElement);
const legendMax = requireElement("heatmap-legend-max", HTMLElement);
const averageWrap = requireElement("chart-latency-wrap", HTMLElement);
const heatmapTable = requireElement("heatmap-table", HTMLTableElement);
const averageTable = requireElement("chart-latency-table", HTMLTableElement);

/**
 * The heatmap's "Show data" table: each drawn second's requests and its p50,
 * p95 and p99, the lines the picture draws. Every cell would be hundreds of
 * seconds times dozens of buckets, too many to read.
 */
const showHeatmapData = chartDataTable<{
  second: number;
  requests: number;
  p50: number;
  p95: number;
  p99: number;
}>("chart-latency-details", "heatmap", [
  { key: "second", type: "num", format: (r) => String(r.second) },
  { key: "requests", type: "num", format: (r) => formatInt(r.requests) },
  { key: "p50", type: "num", format: (r) => formatInt(r.p50) },
  { key: "p95", type: "num", format: (r) => formatInt(r.p95) },
  { key: "p99", type: "num", format: (r) => formatInt(r.p99) },
]);

const FORMATS = { int: formatInt, pct: formatPct };

/** The all requests heatmap, as applied from snapshots. */
let allState: HeatmapState | null = null;
/** The request shown alone, or null for all requests, and its heatmap. */
let selected: SelectedRequest | null = null;
let selectedState: HeatmapState | null = null;
/** Ask for the selected request's whole window next time. */
let selectedFull = true;
let seriesTimer: ReturnType<typeof setInterval> | null = null;
/** The request whose heatmap is being fetched, so a tick skips while it is. */
let inFlight: SelectedRequest | null = null;
/** The Adjusted (true, the default) or Measured view. */
let adjustedView = true;
/** Coordinated omission mitigation has recorded an event. */
let coActive = false;
/** Percentiles over time are on, as the last snapshot said. */
let percentilesOn = true;
/** The requests with a heatmap of their own, as the last snapshot listed them. */
let listed: SelectedRequest[] = [];
/** The select's options, as one string, to rebuild them only on change. */
let optionsKey = "";
let layout: HeatmapLayout | null = null;
let shownAdjusted = false;

function sameRequest(a: SelectedRequest | null, b: SelectedRequest): boolean {
  return a !== null && a.method === b.method && a.name === b.name;
}

function optionValue(request: SelectedRequest): string {
  return request.method + "\n" + request.name;
}

/** Where the page's all requests heatmap stands, for snapshot fetches. */
export function snapshotHeatmapCursor(): HeatmapCursor | null {
  return percentilesOn ? heatmapCursor(allState) : null;
}

function stopPolling(): void {
  if (seriesTimer !== null) {
    clearInterval(seriesTimer);
    seriesTimer = null;
  }
}

/**
 * Fetches the selected request's heatmap and draws it. A tick while the last
 * fetch for it is still out does nothing, so a slow answer is never asked for
 * twice and the next fetch carries the cursor it brings.
 */
function pollSelected(): void {
  const request = selected;
  if (request === null || inFlight === request) return;
  inFlight = request;
  const cursor = selectedFull ? null : heatmapCursor(selectedState);
  void fetchRequestSeries(request.method, request.name, cursor).then((result) => {
    if (inFlight === request) inFlight = null;
    if (!sameRequest(selected, request)) return;

    if (result.kind === "ok") {
      const next = applyHeatmap(selectedState, result.series.heatmap);
      if (next === null) {
        selectedFull = true;
        return;
      }
      selectedState = next;
      selectedFull = false;
      draw();
    } else if (result.kind === "missing") {
      // The request has no heatmap (yet): keep it chosen, show nothing.
      selectedState = null;
      selectedFull = true;
      draw();
    }
  });
}

function selectRequest(request: SelectedRequest | null): void {
  stopPolling();
  selected = request;
  selectedState = null;
  selectedFull = true;
  selectEl.value = request === null ? "" : optionValue(request);
  if (request !== null) {
    seriesTimer = setInterval(pollSelected, 1000);
    pollSelected();
  }
  draw();
}

/** A snapshot's requests with a heatmap of their own, by method then name. */
function selectableRequests(snap: DashboardSnapshot): SelectedRequest[] {
  const order = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);
  return snap.requests
    .filter((r) => r.has_series)
    .map((r) => ({ method: r.method, name: r.name }))
    .sort((a, b) => order(a.method, b.method) || order(a.name, b.name));
}

/**
 * Fills the select: All requests, then each request with its own heatmap by
 * method then name. The options are rebuilt only when that set of requests
 * changes, and never while the select has focus: a change waits for blur, so
 * the list never moves under the pointer. Goes back to All requests when the
 * chosen one is no longer listed.
 */
function updateSelect(): void {
  const requests = listed;
  const key = requests.map(optionValue).join("\u0000");
  if (key !== optionsKey && document.activeElement !== selectEl) {
    optionsKey = key;
    selectEl.textContent = "";
    const all = document.createElement("option");
    all.value = "";
    all.textContent = "All requests";
    selectEl.appendChild(all);
    for (const request of requests) {
      const option = document.createElement("option");
      option.value = optionValue(request);
      option.textContent = request.method + " " + request.name;
      selectEl.appendChild(option);
    }
  }
  const request = selected;
  if (request !== null && !requests.some((r) => sameRequest(request, r))) {
    selectRequest(null);
    return;
  }
  selectEl.value = request === null ? "" : optionValue(request);
}

function setViewButtons(): void {
  adjustedButton.setAttribute("aria-pressed", adjustedView ? "true" : "false");
  measuredButton.setAttribute("aria-pressed", adjustedView ? "false" : "true");
}

/**
 * The card's summary and data table from the heatmap shown: the mean of the
 * last 10 drawn seconds' percentiles, and whether p95 is rising or falling.
 */
function showData(view: HeatmapView): void {
  const seconds = Array.from(view.percentiles.values());
  showHeatmapData(
    seconds.map((p) => ({ ...p, requests: view.secondTotals.get(p.second) || 0 }))
  );
  if (seconds.length === 0) {
    setLatencySummary("No data yet.");
    return;
  }
  const p50 = recentTrend(seconds.map((p) => p.p50));
  const p95 = recentTrend(seconds.map((p) => p.p95));
  const p99 = recentTrend(seconds.map((p) => p.p99));
  const text =
    "p50 " + formatInt(p50.mean) + " ms, p95 " + formatInt(p95.mean) +
    " ms, p99 " + formatInt(p99.mean) + " ms";
  const trend = p95.trend === null ? "" : "; p95 " + p95.trend;
  setLatencySummary(text + trend + ".");
}

function hideTooltip(): void {
  tooltipEl.hidden = true;
}

/** Draws the heatmap of the current choice, with its title, notes and legend. */
function draw(): void {
  const adjusted = coActive && adjustedView;
  shownAdjusted = adjusted;
  const state = selected === null ? allState : selectedState;
  const view = heatmapView(state, adjusted);
  titleEl.textContent =
    selected === null ? TITLE : TITLE + ": " + selected.method + " " + selected.name;
  noteEl.textContent = ADJUSTED_NOTE;
  noteEl.hidden = !adjusted;
  canvas.setAttribute("aria-label", ARIA_LABEL);
  const theme = chartTheme();
  layout = drawHeatmap(canvas, view, theme.dark, theme);
  showData(view);
  const stops: string[] = [];
  for (let i = 0; i <= 4; i++) stops.push(scaleColor(i / 4, theme.dark));
  legendBar.style.background = "linear-gradient(to right, " + stops.join(", ") + ")";
  legendMin.textContent = view.cells.length ? formatInt(view.minCount) : "";
  legendMax.textContent = view.cells.length ? formatInt(view.maxCount) : "";
  hideTooltip();
}

function showTooltip(event: MouseEvent): void {
  if (layout === null) return;
  const rect = canvas.getBoundingClientRect();
  const x = event.clientX - rect.left;
  const y = event.clientY - rect.top;
  const hit = cellAt(layout, x, y);
  if (hit === null) {
    hideTooltip();
    return;
  }
  tooltipEl.textContent = tooltipText(layout.view, hit.cell, hit.row, shownAdjusted, FORMATS);
  tooltipEl.hidden = false;
  const width = tooltipEl.offsetWidth || 200;
  const left = Math.min(x + 12, Math.max(0, wrapEl.clientWidth - width));
  tooltipEl.style.left = left + "px";
  tooltipEl.style.top = y + 12 + "px";
}

/** The card as before #713: the average latency chart and the off note. */
function showAverage(metricsDisabled: boolean): void {
  // Back to all requests, without drawing: no heatmap code runs while off.
  stopPolling();
  selected = null;
  selectedState = null;
  selectedFull = true;
  selectEl.value = "";
  allState = null;
  layout = null;
  titleEl.textContent = AVERAGE_TITLE;
  noteEl.textContent = OFF_NOTE;
  noteEl.hidden = metricsDisabled;
  alwaysNoteEl.hidden = true;
  viewGroup.hidden = true;
  selectEl.hidden = true;
  wrapEl.hidden = true;
  legendEl.hidden = true;
  averageWrap.hidden = false;
  heatmapTable.hidden = true;
  averageTable.hidden = false;
}

function showHeatmap(): void {
  alwaysNoteEl.hidden = false;
  selectEl.hidden = false;
  wrapEl.hidden = false;
  legendEl.hidden = false;
  averageWrap.hidden = true;
  heatmapTable.hidden = false;
  averageTable.hidden = true;
}

/**
 * Shows a snapshot in the card. Returns true when the card shows the average
 * latency chart, which the snapshot's series then draws.
 */
export function updateHeatmapCard(snap: DashboardSnapshot): boolean {
  percentilesOn = snap.flags.percentiles_over_time;
  if (!percentilesOn) {
    showAverage(snap.flags.metrics_disabled);
    return true;
  }
  showHeatmap();
  coActive = snap.aggregate.co_active;
  viewGroup.hidden = !coActive;
  const heatmap = snap.series.heatmap;
  if (heatmap !== null) {
    const next = applyHeatmap(allState, heatmap);
    if (next === null) {
      // It follows on from a heatmap this page never had: ask for what it lacks.
      catchUpHeatmap();
    } else {
      allState = next;
    }
  }
  listed = selectableRequests(snap);
  updateSelect();
  // The chosen request redraws when its own heatmap arrives.
  if (selected === null) draw();
  return false;
}

/** Redraws the heatmap in the current theme; nothing while it is off. */
export function redrawHeatmapCard(): void {
  if (percentilesOn) draw();
}

export function initHeatmapCard(): void {
  setViewButtons();
  selectEl.addEventListener("change", () => {
    const value = selectEl.value;
    if (value === "") {
      selectRequest(null);
      return;
    }
    const at = value.indexOf("\n");
    selectRequest({ method: value.slice(0, at), name: value.slice(at + 1) });
  });
  adjustedButton.addEventListener("click", () => {
    adjustedView = true;
    setViewButtons();
    draw();
  });
  measuredButton.addEventListener("click", () => {
    adjustedView = false;
    setViewButtons();
    draw();
  });
  canvas.addEventListener("mousemove", showTooltip);
  canvas.addEventListener("mouseleave", hideTooltip);
  selectEl.addEventListener("blur", updateSelect);

}
