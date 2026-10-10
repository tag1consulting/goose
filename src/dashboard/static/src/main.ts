// Goose live dashboard: SSE stream with poll fallback, charts, control.
// Source of truth for app.js: `npm run build` in src/dashboard/static bundles
// this entry, its modules and Chart.js with esbuild.
//
// The snapshot types (DashboardSnapshot and the structs it holds) are global
// declarations in snapshot.d.ts, generated from
// src/metrics/dashboard_snapshot.rs; edit the Rust structs, not those types.

import { ensureCharts, updateCharts } from "./charts";
import { shownTime } from "./coadjust";
import {
  clearAuthRequired,
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
import {
  closedBannerText,
  initRunsPanel,
  setRunsControl,
  updateSaveFromSnapshot,
} from "./runs";
import { setBanner, setConnection, type ConnectionMode } from "./status";
import { initTables, setTableData } from "./tables";

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
  setTableData(snap);
  updateCharts(snap.series);
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
ensureCharts();
initRunsPanel();
void initControlPanel().then(setRunsControl);
startSse(renderSnapshot, (data) => setBanner(closedBannerText(data), "info"));
