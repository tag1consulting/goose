// A Gaggle manager's view of its workers: the header's expected workers,
// the attention banner, the KPI row's subtitles and Workers tile, the
// Workers panel (summary, grid, hover card, compact and full tables, recent
// events), the RPS chart's toggle and markers, and the Control card's split
// previews and throttle field.
//
// Built only once a snapshot carries `gaggle`, so a single process's page
// stays exactly as index.html has it. Every element is made here with
// createElement, every string from a worker reaches the page through
// textContent or setAttribute, and every colour comes from a class or
// style.setProperty, as the page's Content-Security-Policy refuses style
// attributes. Every figure, flag and sentence comes from the snapshot,
// which reads the manager's worker health model; nothing here judges a
// worker.

import { setRpsByWorker, setRpsMarkers, markerNear, markerX, type ChartMarker } from "./charts";
import { isControlEnabled, sendThrottle, setSubtitleLead, targetEdit } from "./control";
import { optionalElement, requireElement } from "./dom";
import { formatInt, formatMs, formatRate } from "./format";
import { groups, splitText, throttleSplit, usersSplit, type SplitGroup } from "./split";
import { announce } from "./status";
import { sortableTable, type Column, type SortableTable } from "./tables";

/** The most users and throttle the manager takes, as its refusals say. */
const MAX_USERS = 100000;
const MAX_THROTTLE = 1000000;
/** Rows the compact workers table shows. */
const COMPACT_ROWS = 6;
/** Events shown before the scrolling list of older ones. */
const EVENTS_SHOWN = 8;
/** Where the RPS chart's view is remembered, per viewer. */
const VIEW_KEY = "goose-dashboard-rps-view";

/** The grid's state codes, as words. */
const STATES = ["joining", "ready", "running", "stopping", "finished", "users exited", "lost"];
const LOST = 6;
const RUNNING = 2;

/** The event kinds announced once each, as they happen. */
const ANNOUNCED = new Set(["worker_lost", "worker_excluded", "worker_saturated", "worker_lagging"]);

/** `second` as `mm:ss`. */
export function clock(second: number): string {
  const s = Math.max(0, Math.floor(second));
  const pad = (n: number): string => (n < 10 ? "0" : "") + n;
  return pad(Math.floor(s / 60)) + ":" + pad(s % 60);
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/** Sets `node`'s text only when it changes. */
function setText(node: HTMLElement, text: string): void {
  if (node.textContent !== text) node.textContent = text;
}

/** `n` and `noun`, plural past one. */
function count(n: number, noun: string): string {
  return formatInt(n) + " " + noun + (n === 1 ? "" : "s");
}

/** A tile's look: its class and glyph, from its state and flag. */
export function tileLook(state: number, flag: number, load: number): { cls: string; glyph: string; word: string } {
  if (state === LOST) return { cls: "tile tile-lost", glyph: "x", word: "lost" };
  if (flag === 1) return { cls: "tile tile-excluded", glyph: "/", word: "left out" };
  if (flag === 2) return { cls: "tile tile-saturated", glyph: "!", word: "saturated" };
  if (flag === 3) return { cls: "tile tile-lagging", glyph: "~", word: "lagging" };
  if (state === RUNNING) {
    const shade = "tile tile-running tile-load-" + Math.min(4, Math.floor(load / 20));
    if (flag === 4) return { cls: shade, glyph: "*", word: "running, outlier" };
    if (flag === 5) return { cls: shade, glyph: "?", word: "running, deltas refused or missing" };
    return { cls: shade, glyph: "", word: "running" };
  }
  if (state < RUNNING) return { cls: "tile tile-idle", glyph: ".", word: STATES[state] || "joining" };
  return { cls: "tile tile-stopped", glyph: "s", word: STATES[state] || "stopped" };
}

/** The legend's items beyond the four always shown, by the bit `glyphBit` sets. */
const LEGEND_EXTRAS = ["* outlier", "/ left out", "? deltas refused or missing", ". joining", "s stopped"];

/** The bit of LEGEND_EXTRAS a tile of `state` and `flag` needs, or 0. */
function glyphBit(state: number, flag: number): number {
  if (state === LOST) return 0;
  if (flag === 1) return 1 << 1;
  if (flag === 2 || flag === 3) return 0;
  if (state === RUNNING) return flag === 4 ? 1 << 0 : flag === 5 ? 1 << 2 : 0;
  return state < RUNNING ? 1 << 3 : 1 << 4;
}

/** A grid figure, or null where the snapshot sends -1 for none. */
function known(value: number): number | null {
  return value < 0 ? null : value;
}

/** A row of the workers tables, joined from the rows and the grid. */
interface WorkerRow {
  worker: string;
  state: string;
  users: number;
  rps: number | null;
  p95: number | null;
  cpu: number | null;
  fail: number | null;
  avg: number | null;
  rpus: number | null;
  delta: number | null;
  flags: string;
  /** Sorts most urgent first. */
  order: number;
}

/** A flag as the tables say it: `lost 03:05`, `saturated 42 s`. */
function flagWords(rows: GaggleRows, at: number): { text: string; cls: string }[] {
  return (rows.flags[at] || []).map((flag) => {
    if (flag === "lost") {
      const second = rows.lost_second[at];
      return { text: second == null ? "lost" : "lost " + clock(second), cls: "flag flag-alarm" };
    }
    if (flag === "excluded") return { text: "left out", cls: "flag flag-alarm" };
    if (flag === "saturated" || flag === "lagging") return { text: flag, cls: "flag flag-warn" };
    return { text: flag, cls: "flag flag-note" };
  });
}

/** The rows of the snapshot, joined with their grid entries. */
export function workerRows(g: GaggleSnapshot): WorkerRow[] {
  const rows = g.workers;
  const grid = g.grid;
  return rows.workers.map((index, at) => ({
    worker: grid.names[index] || "",
    state: STATES[grid.states[index]] || "",
    users: grid.users[index] || 0,
    rps: known(grid.rps[index]),
    p95: known(grid.p95[index]),
    cpu: known(grid.cpu[index]),
    fail: rows.fail_percent[at],
    avg: rows.avg_ms[at],
    rpus: rows.requests_per_user_per_second[at],
    delta: rows.last_delta_secs[at],
    flags: flagWords(rows, at)
      .map((flag) => flag.text)
      .join(", "),
    order: at,
  }));
}

const dash = "-";
/** A p95 in whole milliseconds as the panel says it: 0 is under half a millisecond. */
const p95Text = (n: number): string => (n === 0 ? "<1" : formatInt(n));
const num = (value: number | null, format: (n: number) => string): string =>
  value == null ? dash : format(value);

/** The summary line: `47 of 48 running | 1 saturated | 1 lagging | 1 lost`. */
export function summaryLine(g: GaggleSnapshot): string {
  const s = g.summary;
  const parts = [formatInt(s.running) + " of " + formatInt(s.expected) + " running"];
  if (s.saturated) parts.push(formatInt(s.saturated) + " saturated");
  if (s.lagging) parts.push(formatInt(s.lagging) + " lagging");
  if (s.lost) parts.push(formatInt(s.lost) + " lost");
  if (s.excluded) parts.push(formatInt(s.excluded) + " left out");
  if (s.outliers) parts.push(count(s.outliers, "outlier"));
  if (s.stopped) parts.push(formatInt(s.stopped) + " stopped");
  return parts.join(" | ");
}

/** The Workers tile's subtitle: `1 lost, 1 saturated, 1 lagging`. */
export function workersTileText(s: GaggleSummary): string {
  const parts: string[] = [];
  if (s.lost) parts.push(formatInt(s.lost) + " lost");
  if (s.saturated) parts.push(formatInt(s.saturated) + " saturated");
  if (s.lagging) parts.push(formatInt(s.lagging) + " lagging");
  if (s.excluded) parts.push(formatInt(s.excluded) + " left out");
  return parts.length ? parts.join(", ") : "all healthy";
}

/** The users tile's subtitle: `+300 at 02:00`, or empty. */
export function usersChangeText(change: GaggleUsersChange | undefined): string {
  if (!change) return "";
  const delta = change.to - change.from;
  return (delta >= 0 ? "+" : "-") + formatInt(Math.abs(delta)) + " at " + clock(change.second);
}

/** The hover card's three lines for grid entry `at`. */
export function cardLines(g: GaggleSnapshot, at: number): [string, string, string] {
  const grid = g.grid;
  const look = tileLook(grid.states[at], grid.flags[at], grid.load[at]);
  const name = grid.names[at] || "";
  const row = g.workers.workers.indexOf(at);
  const cores = row >= 0 ? g.workers.cores[row] : null;
  const satSecs = row >= 0 ? g.workers.saturated_secs[row] : null;
  if (grid.states[at] === LOST) {
    const second = row >= 0 ? g.workers.lost_second[row] : null;
    const at = row >= 0 ? g.workers.lost_reason[row] : null;
    const reason = at == null ? null : g.workers.reasons[at];
    return [
      name + "  lost",
      second == null ? "lost" : "lost at " + clock(second),
      reason ? String(reason) : "",
    ];
  }
  const cpu = known(grid.cpu[at]);
  let cpuText = "CPU not reported";
  if (cpu != null) {
    cpuText = "CPU " + cpu + "%";
    if (cores != null) cpuText += " of " + count(cores, "core");
    if (satSecs != null) cpuText += " for " + satSecs + " s";
  }
  const rps = known(grid.rps[at]);
  const gap = grid.gap[at];
  const p95 = known(grid.p95[at]);
  let load = rps == null ? "no requests yet" : formatInt(rps) + " req/s";
  if (rps != null && gap !== 0) load += " (" + (gap > 0 ? "+" : "") + gap + "%)";
  if (p95 != null) load += ", p95 " + p95Text(p95) + " ms";
  return [name + "  " + look.word, cpuText, load];
}

/** The RPS chart's markers: the events within its window. */
export function chartMarkers(g: GaggleSnapshot): ChartMarker[] {
  const events = g.events;
  const out: ChartMarker[] = [];
  for (let i = 0; i < events.seconds.length; i++) {
    const label = events.labels[i];
    if (label == null) continue;
    out.push({
      second: events.seconds[i],
      label,
      text: events.texts[i],
      alarm: events.kinds[i] === "worker_lost",
    });
  }
  return out.reverse();
}

/** The users of the workers a change goes to, in worker order. */
function runningCounts(g: GaggleSnapshot): number[] {
  const out: number[] = [];
  g.grid.states.forEach((state, at) => {
    if (state === RUNNING) out.push(g.grid.users[at] || 0);
  });
  return out;
}

/** The users split preview for `typed`, or the current split. */
export function usersPreview(g: GaggleSnapshot, typed: number | null): SplitGroup[] {
  const counts = runningCounts(g);
  const now = counts.reduce((sum, n) => sum + n, 0);
  if (typed === null || typed === now) return g.previews.users;
  return groups(usersSplit(typed, counts));
}

/** The throttle split preview for `typed`, or the current split. */
export function throttlePreview(g: GaggleSnapshot, typed: number | null, current: number | null): SplitGroup[] {
  if ((typed === null || typed === current) && g.previews.throttle) return g.previews.throttle;
  if (typed === null) return [];
  return groups(throttleSplit(typed, runningCounts(g)));
}

// The page's parts, made once a snapshot carries `gaggle`.
interface Parts {
  expected: HTMLElement;
  attention: HTMLElement;
  usersSub: HTMLElement;
  rpsSub: HTMLElement;
  failSub: HTMLElement;
  workersValue: HTMLElement;
  workersSub: HTMLElement;
  workersTile: HTMLElement;
  summary: HTMLElement;
  checks: HTMLElement;
  checksWrap: HTMLDetailsElement;
  checksSummary: HTMLElement;
  grid: HTMLElement;
  legendExtra: HTMLElement;
  card: HTMLElement;
  cardLines: HTMLElement[];
  compactBody: HTMLTableSectionElement;
  rowsNote: HTMLElement;
  dataDetails: HTMLDetailsElement;
  dataTable: SortableTable<WorkerRow>;
  eventsShown: HTMLOListElement;
  eventsMore: HTMLOListElement;
  eventsMoreWrap: HTMLElement;
  markerTip: HTMLElement;
  markerRows: HTMLTableSectionElement;
  toggle: HTMLButtonElement[];
}

let parts: Parts | null = null;
let latest: GaggleSnapshot | null = null;
let latestThrottle: number | null = null;
/** Each tile, by grid index, and what it shows. */
const tiles: HTMLButtonElement[] = [];
/** What each tile shows: its state, flag and load shade as one number, and its name. */
const tileCodes: number[] = [];
const tileNames: string[] = [];
let focusedTile = 0;
let cardAt: number | null = null;
/** The events of the last snapshot, so each is announced once; at most 200. */
let seen = new Set<string>();
let firstEvents = true;
let eventsKey = "";
let byWorker = false;
let markerFocus = -1;
// The Control card's parts, made once control is on.
let controlParts: {
  usersSplit: HTMLElement;
  throttleRow: HTMLElement;
  throttleInput: HTMLInputElement;
  throttleSplit: HTMLElement;
  hostNote: HTMLElement | null;
} | null = null;

function readView(): boolean {
  try {
    return window.localStorage.getItem(VIEW_KEY) === "by-worker";
  } catch {
    return false;
  }
}

function saveView(on: boolean): void {
  try {
    window.localStorage.setItem(VIEW_KEY, on ? "by-worker" : "total");
  } catch {
    // Remembering the view is a convenience; without storage it resets.
  }
}

/** A KPI tile's subtitle, made under its value. */
function kpiSub(valueId: string): HTMLElement {
  const value = requireElement(valueId, HTMLElement);
  const sub = el("span", "kpi-sub");
  value.insertAdjacentElement("afterend", sub);
  return sub;
}

function th(text: string, sort?: string, type?: string): HTMLTableCellElement {
  const cell = el("th", undefined, text);
  cell.setAttribute("scope", "col");
  if (sort) cell.setAttribute("data-sort", sort);
  if (type) cell.setAttribute("data-type", type);
  return cell;
}

function makeParts(): Parts {
  document.body.classList.add("gaggle-mode");
  setSubtitleLead("Gaggle manager");

  // Header: the workers expected, after the subtitle.
  const expected = el("span", "subtitle gaggle-expected");
  expected.id = "gaggle-expected";
  requireElement("subtitle", HTMLElement).insertAdjacentElement("afterend", expected);

  // The attention banner, under the page's banner. Not a live region: a
  // worker's state change is announced once, from the events.
  const attention = el("div", "banner warn gaggle-attention");
  attention.id = "gaggle-attention";
  attention.hidden = true;
  requireElement("banner", HTMLElement).insertAdjacentElement("afterend", attention);

  // KPI subtitles, and the Workers tile.
  const usersSub = kpiSub("kpi-users");
  const rpsSub = kpiSub("kpi-rps");
  const failSub = kpiSub("kpi-fail");
  kpiSub("kpi-p95").textContent = "merged across workers";
  kpiSub("kpi-avg").textContent = "merged across workers";
  const workersTile = el("div", "kpi card kpi-workers");
  workersTile.appendChild(el("span", "k", "Workers"));
  const workersValue = el("span", "v");
  workersValue.id = "kpi-workers";
  const workersSub = el("span", "kpi-sub");
  workersTile.append(workersValue, workersSub);
  requireElement("kpis", HTMLElement).appendChild(workersTile);

  // The Workers panel and the RPS card side by side: the panel takes the
  // charts grid's first cell, beside the RPS card, and the users card the
  // row below. No chart moves, so Chart.js keeps its canvases.
  const charts = requireElement("charts", HTMLElement);
  charts.classList.add("gaggle-charts");
  const panel = el("section", "card workers-panel");
  panel.id = "workers-panel";
  panel.setAttribute("aria-labelledby", "workers-heading");
  const head = el("div", "workers-head");
  const heading = el("h2", undefined, "Workers");
  heading.id = "workers-heading";
  const summary = el("p", "workers-summary");
  summary.id = "workers-summary";
  head.append(heading, summary);
  // What the reports would open with now: the "Degraded:" and "Check:"
  // sentences, folded so the grid stays near the top.
  const checksWrap = el("details", "workers-checks-wrap");
  const checksSummary = el("summary");
  const checks = el("ul", "workers-checks");
  checksWrap.append(checksSummary, checks);
  checksWrap.hidden = true;

  const grid = el("div", "worker-grid");
  grid.id = "worker-grid";
  grid.setAttribute("role", "group");
  grid.setAttribute("aria-label", "Workers, one tile each: arrow keys move between them");
  grid.addEventListener("keydown", onGridKey);
  grid.addEventListener("mouseleave", hideCard);
  grid.addEventListener("focusout", (event) => {
    if (!grid.contains(event.relatedTarget as Node | null)) hideCard();
  });

  const legend = el("ul", "worker-legend");
  legend.setAttribute("aria-label", "Grid legend");
  const legendItem = (cls: string, glyph: string, text: string): HTMLLIElement => {
    const li = el("li");
    const swatch = el("span", cls, glyph);
    swatch.setAttribute("aria-hidden", "true");
    li.append(swatch, document.createTextNode(" " + text));
    return li;
  };
  legend.append(
    legendItem("tile tile-running tile-load-3", "", "running (shade = load share)"),
    legendItem("tile tile-saturated", "!", "saturated"),
    legendItem("tile tile-lagging", "~", "lagging"),
    legendItem("tile tile-lost", "x", "lost")
  );
  const legendExtra = el("span", "worker-legend-extra");
  legend.appendChild(legendExtra);

  const card = el("div", "worker-card");
  card.id = "worker-card";
  card.setAttribute("role", "tooltip");
  card.hidden = true;
  const cardLines = [el("strong"), el("span"), el("span")];
  card.append(...cardLines);

  // The compact table: the most urgent rows that fit.
  const compactWrap = el("div", "table-wrap workers-compact");
  const compact = el("table");
  compact.id = "workers-compact";
  const compactHead = el("thead");
  const headRow = el("tr");
  for (const name of ["Worker", "State", "Users", "Req/s", "p95", "CPU", "Last delta", "Flags"]) {
    headRow.appendChild(th(name));
  }
  compactHead.appendChild(headRow);
  const compactBody = el("tbody");
  compact.append(compactHead, compactBody);
  compactWrap.appendChild(compact);
  const rowsNote = el("p", "table-note workers-rows-note");

  // The full table of every row sent, sortable: the grid's text fallback.
  const dataDetails = el("details", "chart-data workers-data");
  dataDetails.id = "workers-data";
  dataDetails.appendChild(el("summary", undefined, "Show data table"));
  const dataWrap = el("div", "table-wrap");
  dataWrap.tabIndex = 0;
  dataWrap.setAttribute("role", "region");
  dataWrap.setAttribute("aria-label", "Workers data table");
  const dataTableEl = el("table");
  dataTableEl.id = "workers-table";
  const dataHead = el("thead");
  const dataHeadRow = el("tr");
  const columns: [string, keyof WorkerRow, "num" | "str"][] = [
    ["Worker", "worker", "str"],
    ["State", "state", "str"],
    ["Users", "users", "num"],
    ["Req/s", "rps", "num"],
    ["p95 ms", "p95", "num"],
    ["CPU %", "cpu", "num"],
    ["Fail %", "fail", "num"],
    ["Avg ms", "avg", "num"],
    ["Req/user/s", "rpus", "num"],
    ["Last delta s", "delta", "num"],
    ["Flags", "flags", "str"],
    ["Urgency", "order", "num"],
  ];
  for (const [name, key, type] of columns) dataHeadRow.appendChild(th(name, key, type));
  dataHead.appendChild(dataHeadRow);
  const dataBody = el("tbody");
  dataTableEl.append(dataHead, dataBody);
  dataWrap.appendChild(dataTableEl);
  dataDetails.appendChild(dataWrap);
  const formats: Record<string, (row: WorkerRow) => string> = {
    worker: (r) => r.worker,
    state: (r) => r.state,
    users: (r) => formatInt(r.users),
    rps: (r) => num(r.rps, formatInt),
    p95: (r) => num(r.p95, p95Text),
    cpu: (r) => num(r.cpu, String),
    fail: (r) => num(r.fail, (n) => n.toFixed(1)),
    avg: (r) => num(r.avg, formatMs),
    rpus: (r) => num(r.rpus, formatRate),
    delta: (r) => num(r.delta, (n) => n.toFixed(1)),
    flags: (r) => r.flags,
    order: (r) => String(r.order + 1),
  };
  const tableColumns: Column<WorkerRow>[] = columns.map(([, key, type]) => ({
    key,
    type,
    format: formats[key],
  }));
  const dataTable = sortableTable<WorkerRow>({
    tableId: "workers-table",
    body: dataBody,
    columns: tableColumns,
    sort: { key: "order", type: "num", dir: "asc" },
    emptyText: "No workers yet",
    disabledText: () => null,
    rowKey: (r) => r.worker,
  });
  dataTable.init();
  dataDetails.addEventListener("toggle", () => {
    if (dataDetails.open && latest) dataTable.setRows(workerRows(latest), null);
  });

  // Recent events: the newest few, then the rest in a scrolling region.
  const eventsHead = el("h3", "workers-events-head", "Recent gaggle events");
  eventsHead.id = "gaggle-events-heading";
  const eventsShown = el("ol", "gaggle-events");
  eventsShown.id = "gaggle-events";
  eventsShown.setAttribute("aria-labelledby", "gaggle-events-heading");
  const eventsMoreWrap = el("div", "gaggle-events-more");
  eventsMoreWrap.tabIndex = 0;
  eventsMoreWrap.setAttribute("role", "region");
  eventsMoreWrap.setAttribute("aria-label", "Older gaggle events");
  eventsMoreWrap.hidden = true;
  const eventsMore = el("ol", "gaggle-events");
  eventsMoreWrap.appendChild(eventsMore);

  panel.append(head, checksWrap, grid, legend, card, compactWrap, rowsNote, dataDetails, eventsHead, eventsShown, eventsMoreWrap);
  charts.insertBefore(panel, charts.firstChild);
  const usersCanvas = optionalElement("chart-users", HTMLCanvasElement);
  const usersCard = usersCanvas ? usersCanvas.closest(".chart-card") : null;
  if (usersCard) usersCard.classList.add("chart-wide");

  // The RPS card, beside it, gains a Total / By worker toggle.
  const canvas = requireElement("chart-rps", HTMLCanvasElement);
  const rpsCard = canvas.closest(".chart-card") as HTMLElement;
  const toggle: HTMLButtonElement[] = [];
  let markerTip = el("div");
  let markerRows = el("tbody");
  if (rpsCard) {
    const title = rpsCard.querySelector("h2");
    const chartHead = el("div", "chart-head");
    const group = el("div", "segmented");
    group.id = "rps-view";
    group.setAttribute("role", "group");
    group.setAttribute("aria-label", "Requests per second shown");
    for (const [label, on] of [
      ["Total", false],
      ["By worker", true],
    ] as [string, boolean][]) {
      const button = el("button", undefined, label);
      button.type = "button";
      button.addEventListener("click", () => setByWorker(on, true));
      toggle.push(button);
      group.appendChild(button);
    }
    if (title) {
      title.insertAdjacentElement("beforebegin", chartHead);
      chartHead.append(title, group);
    }
    // The markers' sentences: on hover and on focus of the chart, and in
    // a table under its data table.
    rpsCard.classList.add("chart-marker-card");
    markerTip = el("div", "heatmap-tooltip chart-marker-tip");
    markerTip.id = "chart-rps-marker";
    markerTip.setAttribute("role", "tooltip");
    markerTip.hidden = true;
    canvas.insertAdjacentElement("afterend", markerTip);
    canvas.tabIndex = 0;
    canvas.setAttribute("aria-describedby", "chart-rps-marker");
    canvas.addEventListener("mousemove", onChartHover);
    canvas.addEventListener("mouseleave", hideMarker);
    canvas.addEventListener("focus", () => stepMarker(0));
    canvas.addEventListener("blur", hideMarker);
    canvas.addEventListener("keydown", onChartKey);
    const region = rpsCard.querySelector("#chart-rps-details .table-wrap");
    if (region) {
      const markersTable = el("table", "chart-markers-table");
      markersTable.id = "chart-rps-events";
      const caption = el("caption", undefined, "Gaggle events on this chart");
      const markersHead = el("thead");
      const markersHeadRow = el("tr");
      markersHeadRow.append(th("Time"), th("Second"), th("Event"));
      markersHead.appendChild(markersHeadRow);
      markersTable.append(caption, markersHead, markerRows);
      region.appendChild(markersTable);
    }
  }

  const made: Parts = {
    expected,
    attention,
    usersSub,
    rpsSub,
    failSub,
    workersValue,
    workersSub,
    workersTile,
    summary,
    checks,
    checksWrap,
    checksSummary,
    grid,
    legendExtra,
    card,
    cardLines,
    compactBody,
    rowsNote,
    dataDetails,
    dataTable,
    eventsShown,
    eventsMore,
    eventsMoreWrap,
    markerTip,
    markerRows,
    toggle,
  };
  parts = made;
  setByWorker(readView(), false);

  return made;
}

function setByWorker(on: boolean, remember: boolean): void {
  byWorker = on;
  if (parts) {
    parts.toggle[0].setAttribute("aria-pressed", String(!on));
    parts.toggle[1].setAttribute("aria-pressed", String(on));
  }
  if (remember) saveView(on);
  applyByWorker();
}

/** Hands the RPS chart its lines by worker, or none for its total. */
function applyByWorker(): void {
  if (!latest || !byWorker) {
    setRpsByWorker(null);
    return;
  }
  const lines = latest.by_worker;
  setRpsByWorker({
    names: lines.workers,
    lines: lines.rps,
    othersWorkers: lines.others_workers,
  });
}

// The grid.

function tileFor(at: number): HTMLButtonElement {
  const tile = el("button");
  tile.type = "button";
  tile.tabIndex = at === focusedTile ? 0 : -1;
  tile.setAttribute("aria-describedby", "worker-card");
  tile.addEventListener("mouseenter", () => showCard(at));
  tile.addEventListener("focus", () => {
    focusedTile = at;
    showCard(at);
  });
  return tile;
}

/** Updates the grid in place: a tile is touched only when it changes. */
function renderGrid(g: GaggleSnapshot): void {
  if (!parts) return;
  const grid = g.grid;
  const n = grid.names.length;
  parts.grid.style.setProperty("--cols", String(n > 256 ? 32 : 16));
  while (tiles.length < n) {
    const tile = tileFor(tiles.length);
    tiles.push(tile);
    tileCodes.push(-1);
    tileNames.push("");
    parts.grid.appendChild(tile);
  }
  while (tiles.length > n) {
    const tile = tiles.pop();
    tileCodes.pop();
    tileNames.pop();
    if (tile) tile.remove();
  }
  // Only the tiles whose look changed are touched; the rest are compared as numbers.
  let seenGlyphs = 0;
  for (let at = 0; at < n; at++) {
    const state = grid.states[at];
    const flag = grid.flags[at];
    const code = state * 100 + flag * 10 + Math.min(4, Math.floor(grid.load[at] / 20));
    const name = grid.names[at] || "";
    if (tileCodes[at] === code && tileNames[at] === name) {
      seenGlyphs |= glyphBit(state, flag);
      continue;
    }
    tileCodes[at] = code;
    tileNames[at] = name;
    const look = tileLook(state, flag, grid.load[at]);
    seenGlyphs |= glyphBit(state, flag);
    const tile = tiles[at];
    tile.className = look.cls;
    tile.textContent = look.glyph;
    tile.setAttribute("aria-label", name + ": " + look.word);
  }
  const extras = LEGEND_EXTRAS.filter((_, bit) => seenGlyphs & (1 << bit)).join(", ");
  if (focusedTile >= n) focusedTile = 0;
  setText(parts.legendExtra, extras);
  if (cardAt !== null && cardAt < n) fillCard(cardAt);
}

function onGridKey(event: KeyboardEvent): void {
  if (!latest || tiles.length === 0) return;
  const cols = tiles.length > 256 ? 32 : 16;
  const moves: Record<string, number> = {
    ArrowRight: 1,
    ArrowLeft: -1,
    ArrowDown: cols,
    ArrowUp: -cols,
  };
  let next = focusedTile;
  if (event.key in moves) next = focusedTile + moves[event.key];
  else if (event.key === "Home") next = 0;
  else if (event.key === "End") next = tiles.length - 1;
  else if (event.key === "Escape") {
    hideCard();
    return;
  } else return;
  event.preventDefault();
  next = Math.max(0, Math.min(tiles.length - 1, next));
  tiles[focusedTile].tabIndex = -1;
  focusedTile = next;
  tiles[next].tabIndex = 0;
  tiles[next].focus();
}

function fillCard(at: number): void {
  if (!parts || !latest) return;
  const lines = cardLines(latest, at);
  lines.forEach((line, i) => {
    setText(parts!.cardLines[i], line);
    parts!.cardLines[i].hidden = line === "";
  });
}

/** Moves the one hover card to tile `at` and fills it. */
function showCard(at: number): void {
  if (!parts || !latest || !tiles[at]) return;
  cardAt = at;
  fillCard(at);
  const tile = tiles[at];
  parts.card.hidden = false;
  const panel = parts.grid.offsetParent as HTMLElement | null;
  const left = tile.offsetLeft + tile.offsetWidth + 6;
  const maxLeft = panel ? panel.clientWidth - parts.card.offsetWidth - 8 : left;
  parts.card.style.setProperty("left", Math.max(0, Math.min(left, maxLeft)) + "px");
  parts.card.style.setProperty("top", tile.offsetTop + tile.offsetHeight + 4 + "px");
}

function hideCard(): void {
  cardAt = null;
  if (parts) parts.card.hidden = true;
}

// The tables.

function renderCompact(g: GaggleSnapshot): void {
  if (!parts) return;
  const rows = workerRows(g).slice(0, COMPACT_ROWS);
  const body = parts.compactBody;
  while (body.rows.length > rows.length) body.deleteRow(-1);
  rows.forEach((row, i) => {
    const tr = body.rows[i] || body.insertRow();
    const cells = [
      row.worker,
      row.state,
      formatInt(row.users),
      num(row.rps, formatInt),
      num(row.p95, (n) => p95Text(n) + " ms"),
      num(row.cpu, (n) => n + "%"),
      num(row.delta, (n) => n.toFixed(1) + " s"),
    ];
    cells.forEach((text, c) => {
      const td = tr.cells[c] || tr.insertCell();
      setText(td, text);
    });
    const flagsTd = tr.cells[7] || tr.insertCell();
    const words = flagWords(g.workers, row.order);
    const key = words.map((w) => w.cls + w.text).join("|");
    if (flagsTd.getAttribute("data-key") !== key) {
      flagsTd.setAttribute("data-key", key);
      flagsTd.textContent = "";
      words.forEach((word, w) => {
        if (w > 0) flagsTd.appendChild(document.createTextNode(", "));
        flagsTd.appendChild(el("span", word.cls, word.text));
      });
    }
  });
  const total = g.summary.expected > 0 ? Math.max(g.summary.expected, g.grid.names.length) : g.grid.names.length;
  setText(
    parts.rowsNote,
    formatInt(rows.length) + " of " + formatInt(total) + " rows, most urgent first."
  );
  if (parts.dataDetails.open) parts.dataTable.setRows(workerRows(g), null);
}

// The events.

function eventItem(g: GaggleSnapshot, i: number): HTMLLIElement {
  const li = el("li", g.events.kinds[i] === "worker_lost" ? "event-alarm" : undefined);
  const time = el("span", "event-time", clock(g.events.seconds[i]));
  li.append(time, document.createTextNode(" " + g.events.texts[i]));
  return li;
}

function renderEvents(g: GaggleSnapshot): void {
  if (!parts) return;
  const events = g.events;
  const n = events.texts.length;
  const key = n + "|" + (n ? events.seconds[0] + "|" + events.texts[0] : "");
  // Announced once each, newest last; none of what was there on opening.
  const now = new Set<string>();
  for (let i = n - 1; i >= 0; i--) {
    const id = events.seconds[i] + "|" + events.kinds[i] + "|" + events.texts[i];
    now.add(id);
    if (seen.has(id)) continue;
    if (!firstEvents && ANNOUNCED.has(events.kinds[i])) announce(events.texts[i] + ".");
  }
  seen = now;
  firstEvents = false;
  if (key === eventsKey) return;
  eventsKey = key;
  parts.eventsShown.textContent = "";
  parts.eventsMore.textContent = "";
  for (let i = 0; i < n; i++) {
    (i < EVENTS_SHOWN ? parts.eventsShown : parts.eventsMore).appendChild(eventItem(g, i));
  }
  if (n === 0) parts.eventsShown.appendChild(el("li", "event-none", "No events yet."));
  parts.eventsMoreWrap.hidden = n <= EVENTS_SHOWN;
}

// The RPS chart's markers.

function showMarker(marker: ChartMarker): void {
  if (!parts) return;
  const x = markerX(marker);
  setText(parts.markerTip, clock(marker.second) + " " + marker.text);
  parts.markerTip.hidden = false;
  const canvas = optionalElement("chart-rps", HTMLCanvasElement);
  const left = (canvas ? canvas.offsetLeft : 0) + (x === null ? 0 : x);
  parts.markerTip.style.setProperty("left", Math.max(0, left - 40) + "px");
  parts.markerTip.style.setProperty("top", (canvas ? canvas.offsetTop : 0) + 4 + "px");
}

function hideMarker(): void {
  markerFocus = -1;
  if (parts) parts.markerTip.hidden = true;
}

function onChartHover(event: MouseEvent): void {
  const marker = markerNear(event.offsetX, 6);
  if (marker) showMarker(marker);
  else if (parts) parts.markerTip.hidden = true;
}

/** Shows the marker `by` places from the one shown, newest first. */
function stepMarker(by: number): void {
  if (!latest) return;
  const markers = chartMarkers(latest).reverse();
  if (markers.length === 0) {
    hideMarker();
    return;
  }
  markerFocus = markerFocus < 0 ? 0 : (markerFocus + by + markers.length) % markers.length;
  showMarker(markers[markerFocus]);
}

function onChartKey(event: KeyboardEvent): void {
  if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
    event.preventDefault();
    stepMarker(event.key === "ArrowLeft" ? 1 : -1);
  } else if (event.key === "Escape") {
    hideMarker();
  }
}

function renderMarkerRows(g: GaggleSnapshot): void {
  if (!parts) return;
  const markers = chartMarkers(g);
  const body = parts.markerRows;
  while (body.rows.length > markers.length) body.deleteRow(-1);
  markers.forEach((marker, i) => {
    const tr = body.rows[i] || body.insertRow();
    [clock(marker.second), String(marker.second), marker.text].forEach((text, c) => {
      setText(tr.cells[c] || tr.insertCell(), text);
    });
  });
}

// The Control card.

function makeControlParts(): void {
  const panel = optionalElement("control-panel", HTMLElement);
  const usersRow = panel ? panel.querySelector(".control-users-row") : null;
  if (!panel || !usersRow) return;
  const heading = panel.querySelector("h2");
  const note = el("p", "control-help gaggle-note", "changes go to every worker");
  if (heading) heading.insertAdjacentElement("afterend", note);
  const usersSplitEl = el("p", "control-help gaggle-split");
  usersSplitEl.id = "ctrl-target-split";
  usersRow.insertAdjacentElement("afterend", usersSplitEl);
  const target = optionalElement("ctrl-target", HTMLInputElement);
  if (target) {
    target.addEventListener("input", renderControl);
    target.setAttribute(
      "aria-describedby",
      (target.getAttribute("aria-describedby") || "") + " ctrl-target-split"
    );
  }

  const throttleRow = el("div", "control-row gaggle-throttle");
  throttleRow.hidden = true;
  const label = el("label", "control-label", "Throttle ");
  label.setAttribute("for", "ctrl-throttle");
  const throttleInput = el("input");
  throttleInput.id = "ctrl-throttle";
  throttleInput.type = "number";
  throttleInput.min = "1";
  throttleInput.step = "1";
  throttleInput.inputMode = "numeric";
  throttleInput.autocomplete = "off";
  throttleInput.setAttribute("aria-describedby", "ctrl-throttle-split");
  label.appendChild(throttleInput);
  label.appendChild(document.createTextNode(" req/s"));
  const apply = el("button", "ctrl-btn", "Apply throttle");
  apply.type = "button";
  apply.id = "ctrl-throttle-apply";
  apply.addEventListener("click", () => {
    const value = parseInt(throttleInput.value, 10);
    if (Number.isFinite(value) && value >= 1 && sendThrottle(value)) {
      throttleInput.removeAttribute("data-edited");
    }
  });
  throttleInput.addEventListener("input", () => {
    throttleInput.setAttribute("data-edited", "true");
    renderControl();
  });
  throttleRow.append(label, apply);
  const throttleSplitEl = el("p", "control-help gaggle-split");
  throttleSplitEl.id = "ctrl-throttle-split";
  usersSplitEl.insertAdjacentElement("afterend", throttleRow);
  throttleRow.insertAdjacentElement("afterend", throttleSplitEl);

  let hostNote: HTMLElement | null = null;
  const hostInputs = panel.querySelector(".settings-host .settings-inputs");
  if (hostInputs) {
    hostNote = el("span", "control-help gaggle-host");
    hostNote.id = "set-host-workers";
    hostInputs.appendChild(hostNote);
  }
  controlParts = {
    usersSplit: usersSplitEl,
    throttleRow,
    throttleInput,
    throttleSplit: throttleSplitEl,
    hostNote,
  };
}

/** The Control card's previews, for what is typed or the current values. */
function renderControl(): void {
  if (!latest || !isControlEnabled()) return;
  if (!controlParts) makeControlParts();
  if (!controlParts) return;
  const g = latest;
  const edit = targetEdit();
  // Only a value the manager would take is previewed: 1 to 100,000 users.
  const parsed = edit !== null && /^\d+$/.test(edit.trim()) ? parseInt(edit, 10) : null;
  const typed = parsed !== null && parsed >= 1 && parsed <= MAX_USERS ? parsed : null;
  setText(controlParts.usersSplit, splitText(usersPreview(g, typed), "users"));
  const throttle = latestThrottle;
  controlParts.throttleRow.hidden = throttle === null;
  controlParts.throttleSplit.hidden = throttle === null;
  if (throttle !== null) {
    const input = controlParts.throttleInput;
    const edited = input.hasAttribute("data-edited");
    if (!edited && document.activeElement !== input && input.value !== String(throttle)) {
      input.value = String(throttle);
    }
    const parsedThrottle = edited && /^\d+$/.test(input.value.trim()) ? parseInt(input.value, 10) : null;
    // 1 to 1,000,000 requests per second, as the manager takes.
    const value =
      parsedThrottle !== null && parsedThrottle >= 1 && parsedThrottle <= MAX_THROTTLE ? parsedThrottle : null;
    setText(controlParts.throttleSplit, splitText(throttlePreview(g, value, throttle), "req/s"));
  }
  if (controlParts.hostNote) {
    setText(controlParts.hostNote, "same on all " + formatInt(g.previews.workers));
  }
}

/**
 * Shows the Gaggle parts of the page for `snap`: nothing at all until a
 * snapshot carries `gaggle`. Called before the charts update, so the RPS
 * chart draws this snapshot's lines and markers.
 */
export function updateGaggle(snap: DashboardSnapshot): void {
  const g = snap.gaggle;
  if (!g) return;
  latest = g;
  latestThrottle = snap.settings.throttle == null ? null : snap.settings.throttle;
  const p = parts || makeParts();
  const s = g.summary;

  setText(p.expected, count(g.expected_workers, "worker") + " expected");
  setText(p.attention, s.attention || "");
  p.attention.hidden = !s.attention;

  setText(p.usersSub, usersChangeText(s.last_users_change));
  setText(p.rpsSub, latestThrottle === null ? "" : "throttle " + formatInt(latestThrottle) + " /s");
  const fps = snap.aggregate.failures_per_second;
  setText(p.failSub, (fps >= 10 ? formatInt(fps) : formatRate(fps)) + " /s failing");
  setText(p.workersValue, formatInt(s.running) + " / " + formatInt(s.expected));
  setText(p.workersSub, workersTileText(s));
  p.workersTile.classList.toggle("kpi-warn", s.lost + s.saturated + s.lagging > 0);

  setText(p.summary, summaryLine(g));
  const checks = s.degraded.concat(s.checks);
  const checksKey = checks.join("\n");
  if (p.checks.getAttribute("data-key") !== checksKey) {
    p.checks.setAttribute("data-key", checksKey);
    p.checks.textContent = "";
    for (const sentence of checks) p.checks.appendChild(el("li", undefined, sentence));
    p.checksWrap.hidden = checks.length === 0;
    setText(
      p.checksSummary,
      checks.length === 1 ? "1 sentence the reports would open with" : formatInt(checks.length) + " sentences the reports would open with"
    );
  }

  renderGrid(g);
  renderCompact(g);
  renderEvents(g);
  renderMarkerRows(g);
  setRpsMarkers(chartMarkers(g));
  applyByWorker();
  renderControl();
}
