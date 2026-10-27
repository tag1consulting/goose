// Goose live dashboard: SSE stream with poll fallback, charts, control.
// Source of truth for app.js: `npm run build` in src/dashboard/static bundles
// this entry, its modules and Chart.js with esbuild.
//
// The snapshot types (DashboardSnapshot and the structs it holds) are global
// declarations in snapshot.d.ts, generated from
// src/metrics/dashboard_snapshot.rs; edit the Rust structs, not those types.

import { updateChartData } from "./chart-data";
import { ensureCharts, recolorCharts, updateCharts } from "./charts";
import { inMs, shownTime } from "./coadjust";
import {
  clearAuthRequired,
  initToken,
  isAuthRequired,
  setHeatmapCursorSource,
  startSse,
} from "./connection";
import {
  initControlPanel,
  isControlEnabled,
  isControlTokenMissing,
  setPhase,
  updateControlFromSnapshot,
} from "./control";
import { requireElement } from "./dom";
import { formatDuration, formatFailPct, formatInt, formatMs, formatRate, kv } from "./format";
import {
  initHeatmapCard,
  redrawHeatmapCard,
  snapshotHeatmapCursor,
  updateHeatmapCard,
} from "./heatmap-card";
import { sameHosts, settingsSummary } from "./run-settings";
import {
  closedBannerText,
  initRunsPanel,
  setRunsControl,
  updateReportFromSnapshot,
} from "./runs";
import { setBanner, setConnection, type ConnectionMode } from "./status";
import { initTables, setTableData } from "./tables";

const hostsEl = requireElement("hosts", HTMLElement);
const runSettingsEl = requireElement("run-settings", HTMLElement);
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

/** p50 / p95 / p99, to the microsecond where the snapshot has them. */
function percentileText(p: Percentiles, us: MicrosecondTimes | null): string {
  return [
    inMs(p.p50, us ? us.p50 : null),
    inMs(p.p95, us ? us.p95 : null),
    inMs(p.p99, us ? us.p99 : null),
  ]
    .map(formatMs)
    .join(" / ");
}

function minMaxText(min: number, max: number): string {
  return formatMs(min) + " / " + formatMs(max);
}

/** The users KPI: the active count, then the target, smaller. */
function setUsersKpi(active: number, target: number): void {
  const small = document.createElement("small");
  small.textContent = " / " + formatInt(target);
  kpiUsers.textContent = formatInt(active);
  kpiUsers.appendChild(small);
}

/**
 * The RPS KPI: whole requests from 100 a second, two decimals below, with
 * the two decimals the tables show in the tile's hover.
 */
function setRpsKpi(rps: number): void {
  // Decided on the value the tile would show, so 99.996 reads 100, not 100.00.
  kpiRps.textContent = Math.round(rps * 100) >= 10000 ? formatInt(rps) : formatRate(rps);
  const tile = kpiRps.parentElement;
  if (tile) tile.title = "RPS " + formatRate(rps);
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

/**
 * The header's hosts: the current or last run's. In Idle, when the next
 * Start would load test other hosts, they read "Last run: …", visibly, as a
 * hover title alone would be hover only. With control off, the run settings
 * summary follows in its own item, with the next Start's hosts only in Idle
 * when they differ.
 */
function renderHosts(snap: DashboardSnapshot): void {
  const idle = snap.phase === "idle";
  const differ = !sameHosts(snap.hosts, snap.settings.hosts);
  const hosts = snap.hosts.length ? snap.hosts.join(", ") : "—";
  hostsEl.textContent = idle && differ && snap.hosts.length ? "Last run: " + hosts : hosts;
  if (isControlEnabled()) {
    runSettingsEl.hidden = true;
    return;
  }
  runSettingsEl.hidden = false;
  runSettingsEl.textContent = settingsSummary(snap.settings, idle && differ);
}

function renderSnapshot(
  snap: DashboardSnapshot,
  modeLabel?: ConnectionMode | string
): void {
  const flags = snap.flags;
  // The connection first: announce() joins this task's messages, so a
  // recovery reads "Connection restored. Test finished.".
  if (modeLabel === "live") {
    setConnection("live");
  } else if (modeLabel === "poll") {
    setConnection("poll");
  }
  setPhase(snap.phase);
  renderHosts(snap);
  durationEl.textContent = formatDuration(snap.duration_secs);

  // active / target (not peak maximum_users — that stays equal during ramp).
  setUsersKpi(snap.active_users, snap.target_users);
  const agg = snap.aggregate;
  setRpsKpi(agg.requests_per_second);
  kpiFail.textContent = formatFailPct(agg.failure_rate);
  const p = agg.percentile_ms;
  // Once coordinated omission mitigation has events, response times are
  // shown adjusted, labelled so, with the measured value on hover.
  const co = agg.co_adjusted != null ? agg.co_adjusted : null;
  // Both cover the whole run, unlike the heatmap's seconds.
  const suffix = co ? " (whole run, adjusted)" : " (whole run)";
  const us = agg.response_time_us;
  const cus = co ? co.response_time_us : null;
  const p95 = shownTime(
    inMs(p.p95, us ? us.p95 : null),
    co ? inMs(co.percentile_ms.p95, cus ? cus.p95 : null) : null,
    formatMs
  );
  const avg = shownTime(
    agg.response_time_avg_ms,
    co ? co.response_time_avg_ms : null,
    formatMs
  );
  kpiP95Label.textContent = "p95 ms" + suffix;
  kpiAvgLabel.textContent = "Avg ms" + suffix;
  setKpi(kpiP95, formatMs(p95.value), p95.title);
  setKpi(kpiAvg, formatMs(avg.value), avg.title);
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
  aggregateEl.appendChild(kv("Failures/s", formatRate(agg.failures_per_second)));
  aggregateEl.appendChild(kv("Fail %", formatFailPct(agg.failure_rate)));
  // The minimum has no adjusted value: the adjusted line pairs the measured
  // minimum with the adjusted maximum.
  const measuredMinMax = minMaxText(
    inMs(agg.response_time_min_ms, us ? us.min : null),
    inMs(agg.response_time_max_ms, us ? us.max : null)
  );
  if (co) {
    aggregateEl.appendChild(
      kv("Avg ms (adjusted)", formatMs(co.response_time_avg_ms))
    );
    aggregateEl.appendChild(
      kv("p50 / p95 / p99 (adjusted)", percentileText(co.percentile_ms, cus))
    );
    aggregateEl.appendChild(
      kv(
        "Min / max ms (adjusted)",
        minMaxText(
          inMs(agg.response_time_min_ms, us ? us.min : null),
          inMs(co.response_time_max_ms, cus ? cus.max : null)
        )
      )
    );
    aggregateEl.appendChild(
      kv("Avg ms (measured)", formatMs(agg.response_time_avg_ms))
    );
    aggregateEl.appendChild(kv("p50 / p95 / p99 (measured)", percentileText(p, us)));
    aggregateEl.appendChild(kv("Min / max ms (measured)", measuredMinMax));
  } else {
    aggregateEl.appendChild(kv("Avg ms", formatMs(agg.response_time_avg_ms)));
    aggregateEl.appendChild(kv("p50 / p95 / p99", percentileText(p, us)));
    aggregateEl.appendChild(kv("Min / max ms", measuredMinMax));
  }
  if (agg.co_active) {
    aggregateEl.appendChild(kv("Coordinated omission", "active"));
  }

  updateReportFromSnapshot(snap);
  setTableData(snap);
  // The response time card shows the heatmap, or with percentiles over time
  // off the average latency chart, which updateCharts then draws.
  const averageLatency = updateHeatmapCard(snap);
  updateChartData(snap.series, averageLatency, snap.target_users);
  updateCharts(snap.series, averageLatency);
  updateControlFromSnapshot(snap);

  // Prefer metrics/auth banners.
  // Keep the control-token-missing banner sticky while control is on without a token.
  if (flags.metrics_disabled) {
    setBanner(
      "Metrics are disabled (--no-metrics). The dashboard shell is live, but request/series data is empty.",
      "warn"
    );
  } else if (
    flags.requests_truncated ||
    flags.errors_truncated ||
    flags.transaction_errors_truncated
  ) {
    const parts: string[] = [];
    if (flags.requests_truncated) parts.push("request rows truncated");
    if (flags.errors_truncated) parts.push("error rows truncated");
    if (flags.transaction_errors_truncated) parts.push("transaction error rows truncated");
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
}

initToken();
initTables();
ensureCharts();
initHeatmapCard();
setHeatmapCursorSource(snapshotHeatmapCursor);
// Recolour on an OS theme change; the light query sees every switch the CSS makes.
if (window.matchMedia) {
  window.matchMedia("(prefers-color-scheme: light)").addEventListener("change", () => {
    recolorCharts();
    redrawHeatmapCard();
  });
}
initRunsPanel();
void initControlPanel().then(setRunsControl);
startSse(renderSnapshot, () => setBanner(closedBannerText(), "info"));
