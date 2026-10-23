// The report: the report bar's Save report and Include graph data switches,
// Save report now, the banner when a run's report is written or a run has no
// folder, and the runs panel with its downloads, comparisons and, with
// control on, deletes.
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
const folderBannerEl = optionalElement("folder-banner", HTMLElement);
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
// A Save report now in flight: snapshots leave its button unavailable until
// it is answered.
let saveNowPending = false;
// The phase of the last snapshot.
let lastPhase = "idle";
// Why the run in progress has no folder, so Save report is off for it.
let noFolderReason: string | null = null;
// The error a snapshot put in the runs panel's error line while a run is in
// progress.
let runningError: string | null = null;

function authHeaders(): Record<string, string> {
  const token = getToken();
  return token ? { Authorization: "Bearer " + token } : {};
}

// aria-disabled, not disabled: a disabled control drops keyboard focus.
function setAvailable(el: HTMLElement | null, on: boolean): void {
  if (el) el.setAttribute("aria-disabled", on ? "false" : "true");
}

function unavailable(el: HTMLElement | null): boolean {
  return !el || el.getAttribute("aria-disabled") === "true";
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
      "Delete the reports and logs in " + name + "?"
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
  setAvailable(button, !report.running);
  button.addEventListener("click", () => {
    if (!unavailable(button)) deleteReport(report.name);
  });
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
// Report bar: the switches, Save report now, the paths and the notes
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

/** Whether `box` can be changed now. */
function switchAvailable(box: HTMLInputElement | null): boolean {
  return !!box && controlEnabled && !switchPending && !(box === saveReportBox && noFolderReason);
}

function onSwitch(field: "save" | "graph_data", box: HTMLInputElement): void {
  if (unavailable(box)) {
    // The click already changed it; put it back.
    box.checked = !box.checked;
    return;
  }
  switchPending = true;
  for (const other of [saveReportBox, graphDataBox]) setAvailable(other, false);
  void postReport("/api/v1/control/report", JSON.stringify({ [field]: box.checked })).then(
    (answer) => {
      switchPending = false;
      for (const other of [saveReportBox, graphDataBox]) setAvailable(other, switchAvailable(other));
      if (!answer.ok) {
        box.checked = !box.checked;
        showRunsError("Couldn't change the switch: " + answer.message);
      }
    }
  );
}

function saveNowAvailable(): boolean {
  return controlEnabled && lastPhase !== "idle" && !saveNowPending && !noFolderReason;
}

function onSaveNow(): void {
  if (!saveNowButton || unavailable(saveNowButton)) return;
  const button = saveNowButton;
  saveNowPending = true;
  setAvailable(button, false);
  // The report is written in the background; the snapshot's `writes` lists
  // it once it is there.
  void postReport("/api/v1/control/report/now", "{}").then((answer) => {
    saveNowPending = false;
    setAvailable(button, saveNowAvailable());
    showRunsError(answer.ok ? null : "Couldn't save the report now: " + answer.message);
  });
}

function setSwitch(box: HTMLInputElement | null, on: boolean): void {
  if (!box) return;
  if (!switchPending) box.checked = on;
  setAvailable(box, switchAvailable(box));
}

/** The runs panel's error line for a report that failed while a run is in
 * progress; the idle banner says it once the run ends. */
function setRunningError(text: string | null): void {
  if (text === runningError) return;
  const shown = !!runsError && !runsError.hidden && runsError.textContent === runningError;
  runningError = text;
  if (text) showRunsError(text);
  else if (shown) showRunsError(null);
}

/** The banner for a run with no folder, shown for the whole run. */
function setFolderBanner(): void {
  if (!folderBannerEl) return;
  folderBannerEl.textContent = noFolderReason
    ? "Save report is off for this run. " + noFolderReason
    : "";
  folderBannerEl.hidden = !noFolderReason;
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
  runBannerEl.appendChild(text);
  text.textContent = "Report saved in " + report.last_report + ".";
  // The routes take only some folder names; for any other, say where it is.
  if (!report.last_report_name) return;
  text.textContent += " Download HTML, JSON or Markdown.";
  runBannerEl.appendChild(downloadButtons(report.last_report_name));
}

/** Render the report state from a snapshot. */
export function updateReportFromSnapshot(snap: DashboardSnapshot): void {
  const report = snap.report;
  lastPhase = snap.phase;
  const running = snap.phase !== "idle";
  noFolderReason = running && !report.folder && report.error ? report.error : null;
  setFolderBanner();
  setSwitch(saveReportBox, report.save);
  setSwitch(graphDataBox, report.graph_data);
  const saveLabel = saveReportBox ? saveReportBox.closest("label") : null;
  if (saveLabel) {
    saveLabel.title = noFolderReason ? "Off for this run: " + noFolderReason : "";
  }
  if (saveNowButton) {
    saveNowButton.hidden = !controlEnabled;
    setAvailable(saveNowButton, saveNowAvailable());
  }
  setRunningError(
    running && report.folder && report.error ? "Couldn't save the report: " + report.error : null
  );
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
  const seen = (report.folder || "") + "|" + (report.last_report || "") + "|" + report.writes;
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
    // One line, cut short; the whole note on hover.
    if (report.notes) notes.title = report.notes;
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
    runsFooter.textContent =
      formatInt(reports.length) + (reports.length === 1 ? " report in " : " reports in ") + listing.dir;
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
      showRunsError(deleteError || runningError);
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
  for (const box of [saveReportBox, graphDataBox]) setAvailable(box, switchAvailable(box));
  if (saveNowButton) {
    saveNowButton.hidden = !enabled;
    setAvailable(saveNowButton, saveNowAvailable());
  }
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
