// Goose live dashboard: SSE stream with poll fallback, charts, control.
// Source of truth for app.js: `npm run build` in src/dashboard/static bundles
// this entry, its modules and Chart.js with esbuild.
//
// The snapshot types (DashboardSnapshot and the structs it holds) are global
// declarations in snapshot.d.ts, generated from
// src/metrics/dashboard_snapshot.rs; edit the Rust structs, not those types.

import {
  ensureCharts,
  secondLabels,
  updateCharts,
  updateLatencyChart,
} from "./charts";
import { shownTime } from "./coadjust";
import {
  clearAuthRequired,
  fetchRequestSeries,
  initToken,
  isAuthRequired,
  startSse,
} from "./connection";
import {
  initControlPanel,
  isControlTokenMissing,
  setPhase,
  updateControlFromSnapshot,
} from "./control";
import { requireElement } from "./dom";
import { formatDuration, formatInt, formatPct, formatRate, kv } from "./format";
import { latencyChartData, type SelectedRequest } from "./latency";
import {
  closedBannerText,
  initRunsPanel,
  setRunsControl,
  updateSaveFromSnapshot,
} from "./runs";
import { setBanner, setConnection, type ConnectionMode } from "./status";
import {
  initTables,
  onRequestSelected,
  setSelectedRequest,
  setTableData,
} from "./tables";

const hostsEl = requireElement("hosts", HTMLElement);
const durationEl = requireElement("duration", HTMLElement);
const summaryEl = requireElement("summary-body", HTMLElement);
const aggregateEl = requireElement("aggregate-body", HTMLElement);
const kpiUsers = requireElement("kpi-users", HTMLElement);
const kpiRps = requireElement("kpi-rps", HTMLElement);
const kpiFail = requireElement("kpi-fail", HTMLElement);
const kpiP95 = requireElement("kpi-p95", HTMLElement);
const kpiAvg = requireElement("kpi-avg", HTMLElement);
const kpiP95Label = requireElement("kpi-p95-label", HTMLElement);
const kpiAvgLabel = requireElement("kpi-avg-label", HTMLElement);
const requestsCoNote = requireElement("requests-co-note", HTMLElement);
const latencyCard = requireElement("chart-latency-card", HTMLElement);
const latencyAll = requireElement("chart-latency-all", HTMLButtonElement);

// The request charted alone in the latency chart, or null for all requests,
// and its latest series (null until it arrives, or after a 404).
let selected: SelectedRequest | null = null;
let selectedSeries: RequestSeries | null = null;
let seriesTimer: ReturnType<typeof setInterval> | null = null;
let lastSnapshot: DashboardSnapshot | null = null;

const EMPTY_LATENCY: LatencySeries = {
  count: [],
  avg: [],
  p50: [],
  p95: [],
  p99: [],
  max: [],
};

function sameRequest(a: SelectedRequest | null, b: SelectedRequest): boolean {
  return a !== null && a.method === b.method && a.name === b.name;
}

/** Fetches the selected request's series and redraws the latency chart. */
function pollSelectedSeries(): void {
  const request = selected;
  if (request === null) return;
  void fetchRequestSeries(request.method, request.name).then((series) => {
    if (!sameRequest(selected, request)) return;
    selectedSeries = series;
    drawLatency();
  });
}

function selectRequest(request: SelectedRequest): void {
  if (sameRequest(selected, request)) {
    showAllRequests();
    return;
  }
  selected = request;
  selectedSeries = null;
  setSelectedRequest(request);
  latencyAll.hidden = false;
  // jsdom, which runs the client tests, has no scrollIntoView.
  if (typeof latencyCard.scrollIntoView === "function") {
    latencyCard.scrollIntoView({ block: "nearest" });
  }
  if (seriesTimer === null) seriesTimer = setInterval(pollSelectedSeries, 1000);
  pollSelectedSeries();
  drawLatency();
}

function showAllRequests(): void {
  if (seriesTimer !== null) {
    clearInterval(seriesTimer);
    seriesTimer = null;
  }
  selected = null;
  selectedSeries = null;
  setSelectedRequest(null);
  latencyAll.hidden = true;
  drawLatency();
}

/** Draws the latency chart for the selection from the latest data. */
function drawLatency(): void {
  const snap = lastSnapshot;
  if (snap === null) return;
  const flags = snap.flags;
  const coActive = snap.aggregate.co_active;
  if (selected === null) {
    const series = snap.series;
    updateLatencyChart(
      latencyChartData({
        rps: series.rps,
        avg_latency_ms: series.avg_latency_ms,
        latency_ms: series.latency_ms,
        latency_co_adjusted_ms: series.latency_co_adjusted_ms,
        percentiles_over_time: flags.percentiles_over_time,
        metrics_disabled: flags.metrics_disabled,
        co_active: coActive,
        selected: null,
      }),
      secondLabels(series.start_second, series.rps.length)
    );
    return;
  }
  const series = selectedSeries;
  updateLatencyChart(
    latencyChartData({
      rps: series ? series.rps : [],
      avg_latency_ms: [],
      latency_ms: series ? series.latency_ms : EMPTY_LATENCY,
      latency_co_adjusted_ms: series ? series.latency_co_adjusted_ms : EMPTY_LATENCY,
      percentiles_over_time: flags.percentiles_over_time,
      metrics_disabled: flags.metrics_disabled,
      co_active: coActive,
      selected,
    }),
    series ? secondLabels(series.start_second, series.rps.length) : []
  );
}

/** Goes back to all requests when the selection can no longer be charted. */
function checkSelection(snap: DashboardSnapshot): void {
  const request = selected;
  if (request === null) return;
  const row = snap.requests.find(
    (r) => r.method === request.method && r.name === request.name
  );
  if (!snap.flags.percentiles_over_time || row === undefined || !row.has_series) {
    showAllRequests();
  }
}

function percentileText(p: Percentiles): string {
  return formatInt(p.p50) + " / " + formatInt(p.p95) + " / " + formatInt(p.p99);
}

/** Sets a KPI's text, and its hover only while it shows an adjusted value. */
function setKpi(el: HTMLElement, text: string, title: string | null): void {
  el.textContent = text;
  if (title === null) {
    el.removeAttribute("title");
  } else {
    el.title = title;
  }
}

function renderSnapshot(
  snap: DashboardSnapshot,
  modeLabel?: ConnectionMode | string
): void {
  const flags = snap.flags;
  setPhase(snap.phase);
  hostsEl.textContent = snap.hosts.length ? snap.hosts.join(", ") : "—";
  durationEl.textContent = formatDuration(snap.duration_secs);

  // active / target (not peak maximum_users — that stays equal during ramp).
  kpiUsers.textContent =
    formatInt(snap.active_users) + " / " + formatInt(snap.target_users);
  const agg = snap.aggregate;
  kpiRps.textContent = formatRate(agg.requests_per_second);
  kpiFail.textContent = formatPct(agg.failure_rate);
  const p = agg.percentile_ms;
  // Once coordinated omission mitigation has events, response times are
  // shown adjusted, labelled so, with the measured value on hover.
  const co = agg.co_adjusted != null ? agg.co_adjusted : null;
  const suffix = co ? " (adjusted)" : "";
  const p95 = shownTime(p.p95, co ? co.percentile_ms.p95 : null, formatInt);
  const avg = shownTime(
    agg.response_time_avg_ms,
    co ? co.response_time_avg_ms : null,
    formatRate
  );
  kpiP95Label.textContent = "p95 ms" + suffix;
  kpiAvgLabel.textContent = "Avg ms" + suffix;
  setKpi(kpiP95, formatInt(p95.value), p95.title);
  setKpi(kpiAvg, formatRate(avg.value), avg.title);
  requestsCoNote.hidden = co === null;

  summaryEl.textContent = "";
  summaryEl.appendChild(kv("Phase", snap.phase));
  summaryEl.appendChild(kv("Duration", formatDuration(snap.duration_secs)));
  summaryEl.appendChild(
    kv(
      "Users",
      formatInt(snap.active_users) + " / " + formatInt(snap.target_users)
    )
  );
  summaryEl.appendChild(kv("Total users", formatInt(snap.total_users)));
  summaryEl.appendChild(kv("Goose", snap.goose_version));
  summaryEl.appendChild(
    kv(
      "Hosts",
      snap.hosts.length ? snap.hosts.join(", ") : "—"
    )
  );
  if (flags.series_seconds) {
    summaryEl.appendChild(
      kv("Series window", formatInt(flags.series_seconds) + "s")
    );
  }

  aggregateEl.textContent = "";
  aggregateEl.appendChild(kv("Requests", formatInt(agg.total_requests)));
  aggregateEl.appendChild(kv("Failures", formatInt(agg.total_failures)));
  aggregateEl.appendChild(kv("RPS", formatRate(agg.requests_per_second)));
  aggregateEl.appendChild(kv("Fail %", formatPct(agg.failure_rate)));
  if (co) {
    aggregateEl.appendChild(
      kv("Avg ms (adjusted)", formatRate(co.response_time_avg_ms))
    );
    aggregateEl.appendChild(
      kv("p50 / p95 / p99 (adjusted)", percentileText(co.percentile_ms))
    );
    aggregateEl.appendChild(
      kv("Avg ms (measured)", formatRate(agg.response_time_avg_ms))
    );
    aggregateEl.appendChild(kv("p50 / p95 / p99 (measured)", percentileText(p)));
  } else {
    aggregateEl.appendChild(kv("Avg ms", formatRate(agg.response_time_avg_ms)));
    aggregateEl.appendChild(kv("p50 / p95 / p99", percentileText(p)));
  }
  if (agg.co_active) {
    aggregateEl.appendChild(kv("Coordinated omission", "active"));
  }

  updateSaveFromSnapshot(snap);
  lastSnapshot = snap;
  checkSelection(snap);
  setTableData(snap);
  updateCharts(snap.series);
  drawLatency();
  updateControlFromSnapshot(snap);

  // Prefer metrics/auth banners.
  // Keep the control-token-missing banner sticky while control is on without a token.
  if (flags.metrics_disabled) {
    setBanner(
      "Metrics are disabled (--no-metrics). The dashboard shell is live, but request/series data is empty.",
      "warn"
    );
  } else if (flags.requests_truncated || flags.errors_truncated) {
    const parts: string[] = [];
    if (flags.requests_truncated) parts.push("request rows truncated");
    if (flags.errors_truncated) parts.push("error rows truncated");
    setBanner(parts.join("; ") + " (showing top rows only).", "info");
  } else if (isControlTokenMissing()) {
    setBanner(
      "Open this dashboard as http://host:port/?token=… (token required for control).",
      "error"
    );
  } else if (isAuthRequired()) {
    // Clear previous auth banner once we have data.
    setBanner("");
    clearAuthRequired();
  } else {
    setBanner("");
  }

  if (modeLabel === "live") {
    setConnection("live");
  } else if (modeLabel === "poll") {
    setConnection("poll");
  }
}

initToken();
initTables();
onRequestSelected(selectRequest);
latencyAll.addEventListener("click", showAllRequests);
ensureCharts();
initRunsPanel();
void initControlPanel().then(setRunsControl);
startSse(renderSnapshot, (data) => setBanner(closedBannerText(data), "info"));
