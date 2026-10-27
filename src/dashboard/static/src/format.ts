// Display formatting and the small DOM builders the summary and tables share.

export function textCell(
  value: string | number | null | undefined
): HTMLTableCellElement {
  const td = document.createElement("td");
  td.textContent = value == null ? "" : String(value);
  return td;
}

export function kv(
  label: string,
  value: string | number | null | undefined
): HTMLDivElement {
  const wrap = document.createElement("div");
  wrap.className = "kv";
  const k = document.createElement("span");
  k.className = "k";
  k.textContent = label;
  const v = document.createElement("span");
  v.className = "v";
  v.textContent = value == null ? "—" : String(value);
  wrap.appendChild(k);
  wrap.appendChild(v);
  return wrap;
}

export function formatInt(n: number | undefined): string {
  if (typeof n !== "number" || !isFinite(n)) return "—";
  return Math.round(n).toLocaleString();
}

/**
 * A time in milliseconds as Goose shows every time: rounded to the
 * microsecond, then half up to the hundredth under 10 ms (0.42) and to whole
 * milliseconds from there, as the reports show it.
 */
export function formatMs(n: number | undefined): string {
  if (typeof n !== "number" || !isFinite(n)) return "—";
  const us = Math.round(Math.abs(n) * 1000);
  const hundredths = Math.floor((us + 5) / 10);
  const text =
    hundredths < 1000
      ? (hundredths / 100).toLocaleString(undefined, { maximumFractionDigits: 2 })
      : Math.floor((us + 500) / 1000).toLocaleString();
  return n < 0 && text !== "0" ? "-" + text : text;
}

export function formatRate(n: number | undefined): string {
  if (typeof n !== "number" || !isFinite(n)) return "—";
  return n.toLocaleString(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
}

/**
 * A failure rate as a percent, by the console's `# fails` column's rule: a
 * rate under 1% with one decimal, rounded half up, or as `<0.1%` when that
 * decimal would be 0, so failures never show as 0. No failures, and 1% or
 * more, as {@link formatPct}.
 */
export function formatFailPct(n: number | undefined): string {
  if (typeof n !== "number" || !isFinite(n) || n <= 0 || n >= 0.01) {
    return formatPct(n);
  }
  const tenths = Math.round(n * 1000);
  if (tenths === 0) return "<0.1%";
  return (
    (tenths / 10).toLocaleString(undefined, {
      minimumFractionDigits: 1,
      maximumFractionDigits: 1,
    }) + "%"
  );
}

export function formatPct(n: number | undefined): string {
  if (typeof n !== "number" || !isFinite(n)) return "—";
  return (
    (n * 100).toLocaleString(undefined, {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    }) + "%"
  );
}

export function formatDuration(secs: number | undefined): string {
  if (typeof secs !== "number" || !isFinite(secs)) return "—";
  let remaining = Math.max(0, Math.floor(secs));
  const h = Math.floor(remaining / 3600);
  const m = Math.floor((remaining % 3600) / 60);
  const s = remaining % 60;
  if (h > 0) return h + "h " + m + "m " + s + "s";
  if (m > 0) return m + "m " + s + "s";
  return s + "s";
}
