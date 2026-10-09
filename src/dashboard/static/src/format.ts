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

export function formatRate(n: number | undefined): string {
  if (typeof n !== "number" || !isFinite(n)) return "—";
  return n.toLocaleString(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
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
