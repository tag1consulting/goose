// The report: the header's Save report and Include graph data switches, Save
// report now, the banner when a run's report is written, and the runs panel
// with its downloads, comparisons and, with control on, deletes.
//
// Downloads go through fetch with the token in an Authorization header, then
// a blob and a temporary object URL, so the token never goes into a URL.
// Deletes and the switches send the token the same way.

import { getToken } from "./connection";
import { optionalElement } from "./dom";
import { formatDuration, formatInt, textCell } from "./format";

/** One report the runs panel lists, from its folder's `report.json`. */
export interface ListedReport {
  name: string;
  notes?: string | null;
  goose_version: string;
  test: string;
  started: string | null;
  ended: string | null;
  ended_by: "completed" | "stopped" | "canceled" | "users_exited" | string | null;
  canceled_reason?: string | null;
  in_progress: boolean;
  running: boolean;
  duration_secs: number;
  max_users: number;
  requests: number;
  failed_requests: number;
}

/** `GET /api/v1/reports`. */
export interface ReportsListing {
  dir: string;
  reports: ListedReport[];
}

/** Rows shown before `Show all`. */
export const PAGE_ROWS = 50;

const FORMATS: { label: string; file: string }[] = [
  { label: "HTML", file: "report.html" },
  { label: "JSON", file: "report.json" },
  { label: "Markdown", file: "report.md" },
];

const saveReportBox = optionalElement("save-report", HTMLInputElement);
const graphDataBox = optionalElement("graph-data", HTMLInputElement);
const saveNowButton = optionalElement("save-now", HTMLButtonElement);
const reportPathsEl = optionalElement("report-paths", HTMLElement);
const notesEl = optionalElement("run-notes", HTMLElement);
const runBannerEl = optionalElement("run-banner", HTMLElement);
const runsPanel = optionalElement("runs-panel", HTMLElement);
const runsBody = optionalElement("runs-body", HTMLElement);
const runsEmpty = optionalElement("runs-empty", HTMLElement);
const runsFooter = optionalElement("runs-footer", HTMLElement);
const runsShowAll = optionalElement("runs-show-all", HTMLButtonElement);
const runsCompare = optionalElement("runs-compare", HTMLButtonElement);
const runsCompareNote = optionalElement("runs-compare-note", HTMLElement);
const runsError = optionalElement("runs-error", HTMLElement);
const runsDeleteCol = optionalElement("runs-delete-col", HTMLElement);

let listing: ReportsListing | null = null;
let showAll = false;
const checked = new Set<string>();
// The report state last seen, so the list reloads when it changes; undefined
// until the first snapshot.
let reportSeen: string | undefined = undefined;
let loading = false;
// A reload was asked for while a load was in flight.
let reloadAgain = false;
// Control is on: the switches can change and each run gets a Delete button.
let controlEnabled = false;
// The error of the last delete, shown once the list it reloads has loaded.
let deleteError: string | null = null;
// A switch change in flight: snapshots leave the switches alone until it
// is answered.
let switchPending = false;
// The phase of the last snapshot.
let lastPhase = "idle";

function authHeaders(): Record<string, string> {
  const token = getToken();
  return token ? { Authorization: "Bearer " + token } : {};
}

/** The last part of a folder's path, the name the report routes take. */
function folderName(folder: string): string {
  const parts = folder.split(/[\\/]/).filter((part) => part !== "");
  return parts.length ? parts[parts.length - 1] : folder;
}

// ---------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------

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

function reportPath(name: string, file: string): string {
  return "/api/v1/reports/" + encodeURIComponent(name) + "/" + file;
}

function downloadReport(name: string, file: string): void {
  void download(reportPath(name, file), "goose-" + name + "-" + file).then(showError);
}

function showError(message: string | null): void {
  showRunsError(message ? "Download failed: " + message : null);
}

function showRunsError(text: string | null): void {
  if (!runsError) return;
  runsError.textContent = text || "";
  runsError.hidden = !text;
}

/** Ask, then delete a folder's report from disk and list the reports again. */
function deleteReport(name: string): void {
  if (
    !window.confirm(
      "Delete the report in " + name + "? This removes its reports and logs from disk."
    )
  ) {
    return;
  }
  fetch("/api/v1/reports/" + encodeURIComponent(name), {
    method: "DELETE",
    headers: authHeaders(),
  })
    .then(
      (res) => {
        if (res.status === 204) return null;
        return res.text().then(
          (text) => text.trim() || "HTTP " + res.status,
          () => "HTTP " + res.status
        );
      },
      (err: unknown) => String(err)
    )
    .then((message) => {
      deleteError = message ? "Delete failed: " + message : null;
      checked.delete(name);
      loadRuns();
    });
}

function deleteButton(report: ListedReport): HTMLElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "ctrl-btn run-delete";
  button.textContent = "Delete";
  button.setAttribute("aria-label", "Delete the report in " + report.name);
  button.disabled = report.running;
  button.addEventListener("click", () => deleteReport(report.name));
  return button;
}

function downloadButtons(name: string): HTMLElement {
  const wrap = document.createElement("span");
  wrap.className = "run-downloads";
  for (const format of FORMATS) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "ctrl-btn run-download";
    button.textContent = format.label;
    button.setAttribute("aria-label", "Download " + format.label + " report of " + name);
    button.addEventListener("click", () => downloadReport(name, format.file));
    wrap.appendChild(button);
  }
  return wrap;
}

// ---------------------------------------------------------------------------
// Header: the switches, Save report now, the paths and the notes
// ---------------------------------------------------------------------------

/** Post a report control command; resolves to whether it was applied. */
function postReport(path: string, body: string): Promise<{ ok: boolean; message: string }> {
  return fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaders() },
    body,
  })
    .then((res) =>
      res.json().then(
        (data: { ok?: boolean; message?: string }) => ({
          ok: res.ok && !!data.ok,
          message: data.message || "HTTP " + res.status,
        }),
        () => ({ ok: false, message: "HTTP " + res.status })
      )
    )
    .catch((err: unknown) => ({ ok: false, message: String(err) }));
}

function onSwitch(field: "save" | "graph_data", box: HTMLInputElement): void {
  switchPending = true;
  box.disabled = true;
  void postReport("/api/v1/control/report", JSON.stringify({ [field]: box.checked })).then(
    (answer) => {
      switchPending = false;
      box.disabled = !controlEnabled;
      if (!answer.ok) {
        box.checked = !box.checked;
        showRunsError("Couldn't change the switch: " + answer.message);
      }
    }
  );
}

function onSaveNow(): void {
  if (!saveNowButton) return;
  const button = saveNowButton;
  button.disabled = true;
  void postReport("/api/v1/control/report/now", "{}").then((answer) => {
    button.disabled = !controlEnabled || lastPhase === "idle";
    if (!answer.ok) {
      showRunsError("Couldn't save the report now: " + answer.message);
      return;
    }
    showRunsError(null);
    // The report is written in the background; list it once it is there.
    setTimeout(loadRuns, 1000);
  });
}

function setSwitch(box: HTMLInputElement | null, on: boolean): void {
  if (!box) return;
  if (!switchPending) box.checked = on;
  box.disabled = !controlEnabled || switchPending;
}

let bannerKey: string | null = null;

/** The banner once a run's report is written, with its downloads. */
function setRunBanner(report: SnapshotReport, phase: string): void {
  if (!runBannerEl) return;
  const key = phase === "idle" ? (report.last_report || "") + "|" + (report.error || "") : "";
  if (key === bannerKey) return;
  bannerKey = key;
  // The element stays in the page, empty when there is nothing to say, so
  // screen readers announce what is written into it (aria-live).
  runBannerEl.textContent = "";
  runBannerEl.className = "run-banner";
  if (phase !== "idle") return;
  if (report.error) {
    runBannerEl.className = "run-banner error";
    runBannerEl.textContent = "Couldn't save the report: " + report.error;
    return;
  }
  if (!report.last_report) return;
  runBannerEl.className = "run-banner info";
  const text = document.createElement("span");
  text.textContent = "Report saved in " + report.last_report + ". Download HTML, JSON or Markdown.";
  runBannerEl.appendChild(text);
  runBannerEl.appendChild(downloadButtons(folderName(report.last_report)));
}

/** Render the report state from a snapshot. */
export function updateReportFromSnapshot(snap: DashboardSnapshot): void {
  const report = snap.report;
  lastPhase = snap.phase;
  setSwitch(saveReportBox, report.save);
  setSwitch(graphDataBox, report.graph_data);
  if (saveNowButton) {
    saveNowButton.hidden = !controlEnabled;
    saveNowButton.disabled = snap.phase === "idle";
  }
  if (reportPathsEl) {
    reportPathsEl.textContent = report.paths.length ? "Also writes " + report.paths.join(", ") : "";
    reportPathsEl.hidden = report.paths.length === 0;
  }
  if (notesEl) {
    notesEl.textContent = report.notes || "";
    notesEl.hidden = !report.notes;
  }
  setRunBanner(report, snap.phase);
  // List the reports on the first snapshot, and again whenever a run starts
  // or a report is written, however short the run was: a poll every 2 s can
  // miss its phases.
  const seen = (report.folder || "") + "|" + (report.last_report || "");
  if (seen !== reportSeen) {
    reportSeen = seen;
    loadRuns();
  }
}

/** The text of the banner shown when Goose exits. */
export function closedBannerText(): string {
  return "Goose has exited.";
}

// ---------------------------------------------------------------------------
// Runs panel
// ---------------------------------------------------------------------------

function endedText(report: ListedReport): string {
  if (report.in_progress) return report.running ? "Running" : "Incomplete";
  switch (report.ended_by) {
    case "completed":
      return "Finished";
    case "stopped":
      return "Stopped";
    case "users_exited":
      return "Users exited";
    case "canceled":
      return report.canceled_reason === "SIGINT received"
        ? "Ctrl-C"
        : "Canceled: " + (report.canceled_reason || "");
    default:
      return report.ended_by || "";
  }
}

function startedText(started: string | null): string {
  if (!started) return "";
  const date = new Date(started);
  return isNaN(date.getTime()) ? started : date.toLocaleString();
}

function renderRuns(): void {
  if (!runsBody || !listing) return;
  const reports = listing.reports;
  runsBody.textContent = "";
  // Forget checks for reports no longer listed.
  for (const name of Array.from(checked)) {
    if (!reports.some((report) => report.name === name && report.requests > 0)) {
      checked.delete(name);
    }
  }
  const shown = showAll ? reports : reports.slice(0, PAGE_ROWS);
  for (const report of shown) {
    const tr = document.createElement("tr");
    const nameCell = textCell(report.name);
    nameCell.className = "run-id";
    tr.appendChild(nameCell);
    tr.appendChild(textCell(startedText(report.started)));
    const notes = textCell(report.notes || "");
    notes.className = "run-notes";
    tr.appendChild(notes);
    tr.appendChild(textCell(formatDuration(report.duration_secs)));
    tr.appendChild(textCell(formatInt(report.max_users)));
    tr.appendChild(textCell(formatInt(report.requests)));
    tr.appendChild(textCell(formatInt(report.failed_requests)));
    tr.appendChild(textCell(endedText(report)));
    tr.appendChild(textCell(report.test));
    const downloads = document.createElement("td");
    downloads.appendChild(downloadButtons(report.name));
    tr.appendChild(downloads);
    const compare = document.createElement("td");
    const box = document.createElement("input");
    box.type = "checkbox";
    box.className = "run-compare";
    box.setAttribute("aria-label", "Compare " + report.name);
    box.disabled = report.requests === 0;
    box.checked = checked.has(report.name);
    box.addEventListener("change", () => {
      if (box.checked) checked.add(report.name);
      else checked.delete(report.name);
      updateCompare();
    });
    compare.appendChild(box);
    tr.appendChild(compare);
    if (controlEnabled) {
      const remove = document.createElement("td");
      remove.appendChild(deleteButton(report));
      tr.appendChild(remove);
    }
    runsBody.appendChild(tr);
  }
  if (runsEmpty) runsEmpty.hidden = reports.length > 0;
  if (runsShowAll) runsShowAll.hidden = showAll || reports.length <= PAGE_ROWS;
  if (runsFooter) {
    runsFooter.textContent = formatInt(reports.length) + " reports in " + listing.dir;
  }
  updateCompare();
}

/** The two checked reports as [newer, older], or null. */
function comparePair(): [ListedReport, ListedReport] | null {
  if (!listing || checked.size !== 2) return null;
  const pair = listing.reports.filter((report) => checked.has(report.name));
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
  const path = reportPath(newer.name, "compare.md") + "?baseline=" + encodeURIComponent(older.name);
  void download(path, "goose-" + newer.name + "-vs-" + older.name + ".md").then(showError);
}

/** Fetch the reports and render them. Never on a timer. */
export function loadRuns(): void {
  if (!runsPanel) return;
  if (loading) {
    reloadAgain = true;
    return;
  }
  loading = true;
  fetch("/api/v1/reports", { headers: authHeaders() })
    .then((res) => {
      if (!res.ok) throw new Error("HTTP " + res.status);
      return res.json() as Promise<ReportsListing>;
    })
    .then((data) => {
      listing = data;
      renderRuns();
      showRunsError(deleteError);
    })
    .catch((err: unknown) => {
      if (runsError) {
        runsError.textContent = "Couldn't list the reports: " + String(err);
        runsError.hidden = false;
      }
    })
    .then(() => {
      loading = false;
      if (reloadAgain) {
        reloadAgain = false;
        loadRuns();
      } else {
        deleteError = null;
      }
    });
}

/** With control on, the switches can change and each report gets Delete. */
export function setRunsControl(enabled: boolean): void {
  controlEnabled = enabled;
  if (runsDeleteCol) runsDeleteCol.hidden = !enabled;
  for (const box of [saveReportBox, graphDataBox]) {
    if (box) box.disabled = !enabled;
  }
  if (saveNowButton) saveNowButton.hidden = !enabled;
  renderRuns();
}

export function initRunsPanel(): void {
  if (runsShowAll) {
    runsShowAll.addEventListener("click", () => {
      showAll = true;
      renderRuns();
    });
  }
  if (runsCompare) runsCompare.addEventListener("click", onCompareClick);
  if (saveReportBox) {
    const box = saveReportBox;
    box.addEventListener("change", () => onSwitch("save", box));
  }
  if (graphDataBox) {
    const box = graphDataBox;
    box.addEventListener("change", () => onSwitch("graph_data", box));
  }
  if (saveNowButton) saveNowButton.addEventListener("click", onSaveNow);
}
