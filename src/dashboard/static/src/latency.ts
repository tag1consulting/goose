// What the latency chart draws: the five lines, the title, the note and each
// point's tooltip footer, for all requests or one selected request. Imports
// nothing, so the client tests load it without the bundle.

/** One request, as the requests table and `GET /api/v1/series` name it. */
export interface SelectedRequest {
  method: string;
  name: string;
}

export interface LatencyChartInput {
  /** Requests per second for each second of the window. */
  rps: number[];
  /** The snapshot's per second average, used when percentiles are off. */
  avg_latency_ms: number[];
  latency_ms: LatencySeries;
  latency_co_adjusted_ms: LatencySeries;
  percentiles_over_time: boolean;
  metrics_disabled: boolean;
  co_active: boolean;
  /** The charted request, or null for all requests. */
  selected: SelectedRequest | null;
}

export interface LatencyChart {
  p50: (number | null)[];
  p95: (number | null)[];
  p99: (number | null)[];
  avg: (number | null)[];
  max: (number | null)[];
  title: string;
  /** The note above the chart, or "" for none. */
  note: string;
  /** Whether the percentile lines are drawn at all. */
  percentiles: boolean;
  /** The tooltip footer for second `i` of the window, or null for none. */
  footer: (i: number) => string | null;
}

export const OFF_NOTE =
  "Percentiles are off for this run (--no-percentiles-over-time).";
export const ADJUSTED_NOTE =
  "Adjusted: includes the synthetic times coordinated omission mitigation adds. Hover for the measured p99.";

export function latencyChartData(input: LatencyChartInput): LatencyChart {
  const measured = input.latency_ms;
  const adjusted = input.latency_co_adjusted_ms;
  const useAdjusted = input.co_active && adjusted.count.length > 0;
  const chosen = useAdjusted ? adjusted : measured;
  const percentiles = input.percentiles_over_time;

  let title = "Latency by second (ms)";
  if (input.selected) {
    title += ": " + input.selected.method + " " + input.selected.name;
  }

  let note = "";
  if (!percentiles && !input.metrics_disabled) {
    note = OFF_NOTE;
  } else if (useAdjusted) {
    note = ADJUSTED_NOTE;
  }

  const avg = percentiles
    ? chosen.avg.slice()
    : input.avg_latency_ms.map((value, i) => (input.rps[i] === 0 ? null : value));

  function footer(i: number): string | null {
    if (!percentiles) return null;
    const n = measured.count[i] || 0;
    const total = useAdjusted ? adjusted.count[i] || 0 : n;
    const synthetic = total > n ? total - n : 0;
    if (n === 0 && synthetic === 0) return null;
    let text = n + (n === 1 ? " request" : " requests");
    if (synthetic > 0) text += " + " + synthetic + " synthetic";
    const max = chosen.max[i];
    if (max != null) text += ", max " + max + " ms";
    if (synthetic > 0) {
      const p99 = measured.p99[i];
      if (p99 != null) text += ", measured p99 " + p99 + " ms";
    }
    return text;
  }

  return {
    p50: percentiles ? chosen.p50.slice() : [],
    p95: percentiles ? chosen.p95.slice() : [],
    p99: percentiles ? chosen.p99.slice() : [],
    avg,
    max: percentiles ? chosen.max.slice() : [],
    title,
    note,
    percentiles,
    footer,
  };
}
