// Saved runs: the header line saying whether this run is saved, the banner
// when a run ends, the closed banner, and the saved runs panel with its
// downloads and comparisons.
//
// Downloads go through fetch with the token in an Authorization header, then
// a blob and a temporary object URL, so the token never goes into a URL.

import { getToken } from "./connection";
import { optionalElement } from "./dom";
import { formatDuration, formatInt, textCell } from "./format";

/** One saved run, as `run.json` holds it. */
export interface SavedRun {
  format: number;
  id: string;
  goose_version: string;
  test: string;
  started: string;
  ended: string;
  duration_secs: number;
  max_users: number;
  hosts: string[];
  requests: number;
  failed_requests: number;
  ended_by: "completed" | "stopped" | "canceled" | "users_exited" | string;
  canceled_reason: string | null;
  baseline: string | null;
  files: { name: string; bytes: number }[];
}

/** `GET /api/v1/runs`. */
export interface RunsListing {
  dir: string;
  total_bytes: number;
  runs: SavedRun[];
}

/** Rows shown before `Show all`. */
export const PAGE_ROWS = 50;

const FORMATS: { label: string; file: string }[] = [
  { label: "HTML", file: "report.html" },
  { label: "JSON", file: "report.json" },
  { label: "Markdown", file: "report.md" },
];

const saveStatusEl = optionalElement("save-status", HTMLElement);
const runBannerEl = optionalElement("run-banner", HTMLElement);
const runsPanel = optionalElement("runs-panel", HTMLElement);
const runsBody = optionalElement("runs-body", HTMLElement);
const runsEmpty = optionalElement("runs-empty", HTMLElement);
const runsFooter = optionalElement("runs-footer", HTMLElement);
const runsShowAll = optionalElement("runs-show-all", HTMLButtonElement);
const runsCompare = optionalElement("runs-compare", HTMLButtonElement);
const runsCompareNote = optionalElement("runs-compare-note", HTMLElement);
const runsError = optionalElement("runs-error", HTMLElement);

let listing: RunsListing | null = null;
let showAll = false;
const checked = new Set<string>();
let lastPhase: string | null = null;
let loading = false;

// ---------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------

function authHeaders(): Record<string, string> {
  const token = getToken();
  return token ? { Authorization: "Bearer " + token } : {};
}

/** The file name from `Content-Disposition`, or `fallback`. */
function dispositionName(res: Response, fallback: string): string {
  const value = res.headers ? res.headers.get("Content-Disposition") : null;
  const match = value ? /filename="([^"]+)"/.exec(value) : null;
  return match ? match[1] : fallback;
}

/**
 * Fetch `path` with the token in a header and save the response as a file.
 * Resolves to null on success, or an error message.
 */
export function download(path: string, fallbackName: string): Promise<string | null> {
  return fetch(path, { headers: authHeaders() })
    .then((res) => {
      if (!res.ok) {
        return res.text().then(
          (text) => text.trim() || "HTTP " + res.status,
          () => "HTTP " + res.status
        );
      }
      const name = dispositionName(res, fallbackName);
      return res.blob().then((blob) => {
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = name;
        a.rel = "noopener";
        a.style.display = "none";
        document.body.appendChild(a);
        a.click();
        a.remove();
        // Revoke after the click has handed the blob to the download.
        setTimeout(() => URL.revokeObjectURL(url), 0);
        return null;
      });
    })
    .catch((err: unknown) => String(err));
}

function reportPath(id: string, file: string): string {
  return "/api/v1/runs/" + encodeURIComponent(id) + "/" + file;
}

function downloadReport(id: string, file: string): void {
  void download(reportPath(id, file), "goose-" + id + "-" + file).then(showError);
}

function showError(message: string | null): void {
  if (!runsError) return;
  runsError.textContent = message ? "Download failed: " + message : "";
  runsError.hidden = !message;
}

function downloadButtons(id: string): HTMLElement {
  const wrap = document.createElement("span");
  wrap.className = "run-downloads";
  for (const format of FORMATS) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "ctrl-btn run-download";
    button.textContent = format.label;
    button.setAttribute("aria-label", "Download " + format.label + " report of run " + id);
    button.addEventListener("click", () => downloadReport(id, format.file));
    wrap.appendChild(button);
  }
  return wrap;
}

// ---------------------------------------------------------------------------
// Header and banners
// ---------------------------------------------------------------------------

/** The header line for the save state. */
export function saveStatusText(save: SnapshotSave): string {
  if (save.state === "on") return "Saving to " + save.dir;
  if (save.state === "failed") {
    return "Not saving: can't create " + save.dir + " (" + (save.reason || "unknown error") + ")";
  }
  return "Not saving (turned off)";
}

let bannerRun: string | null = null;

function setRunBanner(save: SnapshotSave | null, phase: string): void {
  if (!runBannerEl) return;
  const show = save !== null && phase === "idle" && save.last_run !== null;
  if (!show || save === null || save.last_run === null) {
    if (bannerRun !== null) {
      runBannerEl.textContent = "";
      runBannerEl.className = "run-banner";
      bannerRun = null;
    }
    return;
  }
  const key = save.last_run + "|" + (save.reason || "");
  if (bannerRun === key) return;
  bannerRun = key;
  // The element stays in the page, empty when there is nothing to say, so
  // screen readers announce what is written into it (aria-live).
  runBannerEl.textContent = "";
  const id = save.last_run;
  if (save.reason) {
    runBannerEl.className = "run-banner error";
    runBannerEl.textContent = "Couldn't save run " + id + ": " + save.reason + ".";
    return;
  }
  runBannerEl.className = "run-banner info";
  const text = document.createElement("span");
  text.textContent = "Run " + id + " saved. Download HTML, JSON or Markdown.";
  runBannerEl.appendChild(text);
  runBannerEl.appendChild(downloadButtons(id));
}

/** Render the save state from a snapshot. */
export function updateSaveFromSnapshot(snap: DashboardSnapshot): void {
  const save = snap.save;
  if (saveStatusEl) {
    saveStatusEl.textContent = saveStatusText(save);
    saveStatusEl.className = "meta-item save-status save-" + save.state;
  }
  setRunBanner(save, snap.phase);
  // A run has just ended: list it.
  if (lastPhase !== null && lastPhase !== "idle" && snap.phase === "idle") {
    loadRuns();
  }
  lastPhase = snap.phase;
}

/**
 * The text of the banner shown when Goose exits, from the `closed` event's
 * data: the save state, or `1` from an older Goose.
 */
export function closedBannerText(data: unknown): string {
  let save: Partial<SnapshotSave> | null = null;
  if (typeof data === "string") {
    try {
      const parsed: unknown = JSON.parse(data);
      if (typeof parsed === "object" && parsed !== null) {
        save = parsed as Partial<SnapshotSave>;
      }
    } catch {
      save = null;
    }
  }
  if (!save || typeof save.last_run !== "string" || !save.last_run) {
    return "Goose has exited.";
  }
  if (save.reason) {
    return "Goose has exited. Couldn't save run " + save.last_run + ": " + save.reason + ".";
  }
  return (
    "Goose has exited. This run was saved in " +
    (save.dir || "") +
    "/" +
    save.last_run +
    " on the machine running Goose."
  );
}

// ---------------------------------------------------------------------------
// Saved runs panel
// ---------------------------------------------------------------------------

function endedText(run: SavedRun): string {
  switch (run.ended_by) {
    case "completed":
      return "Finished";
    case "stopped":
      return "Stopped";
    case "users_exited":
      return "Users exited";
    case "canceled":
      return run.canceled_reason === "SIGINT received"
        ? "Ctrl-C"
        : "Canceled: " + (run.canceled_reason || "");
    default:
      return run.ended_by;
  }
}

function startedText(started: string): string {
  const date = new Date(started);
  return isNaN(date.getTime()) ? started : date.toLocaleString();
}

/** Bytes as B, KB, MB or GB. */
export function formatBytes(bytes: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return (unit === 0 ? String(value) : value.toFixed(1)) + " " + units[unit];
}

function renderRuns(): void {
  if (!runsBody || !listing) return;
  const runs = listing.runs;
  runsBody.textContent = "";
  // Forget checks for runs no longer listed.
  for (const id of Array.from(checked)) {
    if (!runs.some((run) => run.id === id && run.requests > 0)) checked.delete(id);
  }
  const shown = showAll ? runs : runs.slice(0, PAGE_ROWS);
  for (const run of shown) {
    const tr = document.createElement("tr");
    const idCell = textCell(run.id);
    idCell.className = "run-id";
    tr.appendChild(idCell);
    tr.appendChild(textCell(startedText(run.started)));
    tr.appendChild(textCell(formatDuration(run.duration_secs)));
    tr.appendChild(textCell(formatInt(run.max_users)));
    tr.appendChild(textCell(formatInt(run.requests)));
    tr.appendChild(textCell(formatInt(run.failed_requests)));
    tr.appendChild(textCell(endedText(run)));
    tr.appendChild(textCell(run.test));
    const downloads = document.createElement("td");
    downloads.appendChild(downloadButtons(run.id));
    tr.appendChild(downloads);
    const compare = document.createElement("td");
    const box = document.createElement("input");
    box.type = "checkbox";
    box.className = "run-compare";
    box.setAttribute("aria-label", "Compare run " + run.id);
    box.disabled = run.requests === 0;
    box.checked = checked.has(run.id);
    box.addEventListener("change", () => {
      if (box.checked) checked.add(run.id);
      else checked.delete(run.id);
      updateCompare();
    });
    compare.appendChild(box);
    tr.appendChild(compare);
    runsBody.appendChild(tr);
  }
  if (runsEmpty) runsEmpty.hidden = runs.length > 0;
  if (runsShowAll) runsShowAll.hidden = showAll || runs.length <= PAGE_ROWS;
  if (runsFooter) {
    runsFooter.textContent =
      formatInt(runs.length) +
      " saved runs, " +
      formatBytes(listing.total_bytes) +
      " in " +
      listing.dir;
  }
  updateCompare();
}

/** The two checked runs as [newer, older], or null. */
function comparePair(): [SavedRun, SavedRun] | null {
  if (!listing || checked.size !== 2) return null;
  const pair = listing.runs.filter((run) => checked.has(run.id));
  if (pair.length !== 2) return null;
  // The listing is newest first.
  return [pair[0], pair[1]];
}

function updateCompare(): void {
  const pair = comparePair();
  if (runsCompare) runsCompare.hidden = pair === null;
  if (runsCompareNote) {
    const differ = pair !== null && pair[0].test !== pair[1].test;
    runsCompareNote.hidden = !differ;
    runsCompareNote.textContent = differ
      ? "These runs are from different tests, so the comparison may not mean much."
      : "";
  }
}

function onCompareClick(): void {
  const pair = comparePair();
  if (!pair) return;
  const [newer, older] = pair;
  const path =
    reportPath(newer.id, "compare.md") + "?baseline=" + encodeURIComponent(older.id);
  void download(path, "goose-" + newer.id + "-vs-" + older.id + ".md").then(showError);
}

/** Fetch the saved runs and render them. Never on a timer. */
export function loadRuns(): void {
  if (!runsPanel || loading) return;
  loading = true;
  fetch("/api/v1/runs", { headers: authHeaders() })
    .then((res) => {
      if (!res.ok) throw new Error("HTTP " + res.status);
      return res.json() as Promise<RunsListing>;
    })
    .then((data) => {
      listing = data;
      renderRuns();
      showError(null);
    })
    .catch((err: unknown) => {
      if (runsError) {
        runsError.textContent = "Couldn't list saved runs: " + String(err);
        runsError.hidden = false;
      }
    })
    .then(() => {
      loading = false;
    });
}

export function initRunsPanel(): void {
  if (runsShowAll) {
    runsShowAll.addEventListener("click", () => {
      showAll = true;
      renderRuns();
    });
  }
  if (runsCompare) runsCompare.addEventListener("click", onCompareClick);
  loadRuns();
}
