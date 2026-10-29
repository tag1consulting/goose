// The report in the dashboard client (app.js), run in jsdom with `npm test`
// after `npm run build`: the header's switches and Save report now, the runs
// panel with its downloads, and the banners.

import { test } from "node:test";
import assert from "node:assert/strict";
import { IDLE_SNAPSHOT } from "./idle-snapshot.mjs";
import { FRESH, openPage, openPages, settle } from "./test-harness.mjs";

const TOKEN = "s3cret";

function report(name, fields = {}) {
  return {
    name,
    notes: null,
    goose_version: "0.19.0-dev",
    test: "loadtest",
    started: "2026-10-09T12:12:03Z",
    ended: "2026-10-09T12:13:03Z",
    ended_by: "completed",
    in_progress: false,
    running: false,
    duration_secs: 60,
    max_users: 10,
    requests: 1000,
    failed_requests: 3,
    ...fields,
  };
}

const LISTING = {
  dir: "goose-runs",
  reports: [
    report("nightly-2", { test: "other", notes: "after the cache change" }),
    report("nightly", { ended_by: "canceled", canceled_reason: "SIGINT received" }),
    report("20261008-090000", {
      requests: 0,
      ended_by: "canceled",
      canceled_reason: "error budget spent",
    }),
    report("20261007-090000", { ended_by: "users_exited" }),
    report("20261006-090000", { in_progress: true, ended: null, ended_by: null }),
    report("20261005-090000", { in_progress: true, running: true, ended: null, ended_by: null }),
  ],
};

function withReport(fields, phase = "idle") {
  return { ...IDLE_SNAPSHOT, phase, report: { ...IDLE_SNAPSHOT.report, ...fields } };
}

test.afterEach(() => {
  while (openPages.length) openPages.pop().window.close();
});

function runRows(window) {
  return Array.from(window.document.querySelectorAll("#runs-body tr"));
}

function bannerText(window) {
  const el = window.document.getElementById("banner");
  return el.classList.contains("hidden") ? "" : el.textContent;
}

test("the panel lists reports, newest first, with notes, and disables Compare without requests", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { reports: LISTING });
  const rows = runRows(page.window);
  assert.equal(rows.length, 6);
  const cells = rows.map((row) => Array.from(row.cells).map((td) => td.textContent));
  assert.deepEqual(
    cells.map((c) => c[0]),
    LISTING.reports.map((r) => r.name)
  );
  assert.equal(cells[0][2], "after the cache change");
  assert.deepEqual(
    cells.map((c) => c[7]),
    [
      "Finished",
      "Ctrl-C",
      "Canceled: error budget spent",
      "Users exited",
      "Incomplete",
      "Running",
    ]
  );
  const boxes = rows.map((row) => row.querySelector("input.run-compare"));
  assert.deepEqual(
    boxes.map((b) => b.disabled),
    [false, false, true, false, false, false]
  );
  const footer = page.window.document.getElementById("runs-footer").textContent;
  assert.equal(footer, "6 reports in goose-runs");
  assert.equal(page.window.document.getElementById("runs-empty").hidden, true);
  // Listed once at startup, not on a timer.
  await settle(page.window);
  assert.equal(page.requests.filter((r) => r.path === "/api/v1/reports").length, 1);
});

test("an empty folder says so", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN);
  assert.equal(runRows(page.window).length, 0);
  const empty = page.window.document.getElementById("runs-empty");
  assert.equal(empty.hidden, false);
  assert.equal(empty.textContent, "No reports yet.");
});

test("downloads send the Bearer header and no token in any URL", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { reports: LISTING });
  const before = page.requests.length;
  const button = runRows(page.window)[0].querySelector("button.run-download");
  assert.equal(button.textContent, "HTML");
  button.click();
  await settle(page.window);
  const downloads = page.requests.slice(before);
  assert.equal(downloads.length, 1);
  assert.equal(downloads[0].path, "/api/v1/reports/nightly-2/report.html");
  assert.equal(downloads[0].bearer, "Bearer " + TOKEN);
  assert.ok(!downloads[0].url.includes(TOKEN), downloads[0].url);
  assert.deepEqual(
    page.downloads.map((d) => d.name),
    ["goose-test-nightly-2-report.html"]
  );
  for (const d of page.downloads) {
    assert.ok(d.href.startsWith("blob:"), d.href);
    assert.ok(!d.href.includes(TOKEN));
  }
  // The listing too goes with the header, not in the URL.
  for (const r of page.requests.filter((r) => r.path.startsWith("/api/v1/reports"))) {
    assert.equal(r.bearer, "Bearer " + TOKEN);
    assert.ok(!r.url.includes(TOKEN), r.url);
  }
});

test("two checked reports download a comparison, newer as the run", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { reports: LISTING });
  const doc = page.window.document;
  const rows = runRows(page.window);
  const compare = doc.getElementById("runs-compare");
  const note = doc.getElementById("runs-compare-note");
  assert.equal(compare.hidden, true);

  // Same test: no warning.
  rows[3].querySelector("input.run-compare").click();
  rows[1].querySelector("input.run-compare").click();
  assert.equal(compare.hidden, false);
  assert.equal(note.hidden, true);
  const before = page.requests.length;
  compare.click();
  await settle(page.window);
  const sent = page.requests.slice(before);
  assert.equal(sent.length, 1);
  const url = new URL(sent[0].url);
  assert.equal(url.pathname, "/api/v1/reports/nightly/compare.md");
  assert.equal(url.searchParams.get("baseline"), "20261007-090000");
  assert.equal(url.searchParams.get("token"), null);
  assert.equal(sent[0].bearer, "Bearer " + TOKEN);

  // A third check: no comparison until exactly two.
  rows[0].querySelector("input.run-compare").click();
  assert.equal(compare.hidden, true);
  // Different tests: the warning shows.
  rows[3].querySelector("input.run-compare").click();
  assert.equal(compare.hidden, false);
  assert.equal(note.hidden, false);
  assert.equal(
    note.textContent,
    "These runs are from different tests, so the comparison may not mean much."
  );
});

test("a failed comparison shows the server's reason", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    reports: LISTING,
    reportFileStatus: 422,
  });
  const rows = runRows(page.window);
  rows[0].querySelector("input.run-compare").click();
  rows[1].querySelector("input.run-compare").click();
  page.window.document.getElementById("runs-compare").click();
  await settle(page.window);
  const error = page.window.document.getElementById("runs-error");
  assert.equal(error.hidden, false);
  assert.equal(error.textContent, "Download failed: Run x has no requests to compare.");
  assert.equal(page.downloads.length, 0);
});

test("the header shows the switches, the paths and the notes, read only without control", async () => {
  const snapshot = withReport({
    save: true,
    graph_data: false,
    paths: ["results.html"],
    notes: "baseline before cache change",
  });
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { snapshot });
  const doc = page.window.document;
  const save = doc.getElementById("save-report");
  const graphs = doc.getElementById("graph-data");
  assert.equal(save.checked, true);
  assert.equal(graphs.checked, false);
  assert.equal(save.getAttribute("aria-disabled"), "true");
  assert.equal(graphs.getAttribute("aria-disabled"), "true");
  assert.equal(doc.getElementById("save-now").hidden, true);
  assert.equal(doc.getElementById("report-paths").textContent, "Also writes results.html");
  const notes = doc.getElementById("run-notes");
  assert.equal(notes.hidden, false);
  assert.equal(notes.textContent, "baseline before cache change");

  const off = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: withReport({ save: false }),
  });
  assert.equal(off.window.document.getElementById("save-report").checked, false);
  assert.equal(off.window.document.getElementById("report-paths").hidden, true);
  assert.equal(off.window.document.getElementById("run-notes").hidden, true);
});

test("with control, a switch sends its value with the Bearer header", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    controlEnabled: true,
    snapshot: withReport({ save: true }),
  });
  const doc = page.window.document;
  const save = doc.getElementById("save-report");
  assert.equal(save.getAttribute("aria-disabled"), "false");
  save.click();
  await settle(page.window);
  const posts = page.requests.filter((r) => r.method === "POST");
  assert.deepEqual(
    posts.map((r) => [r.path, r.bearer, r.url.includes("token="), JSON.parse(r.body)]),
    [["/api/v1/control/report", "Bearer " + TOKEN, false, { save: false }]]
  );
  const graphs = doc.getElementById("graph-data");
  graphs.click();
  await settle(page.window);
  const last = page.requests.filter((r) => r.method === "POST").pop();
  assert.deepEqual(JSON.parse(last.body), { graph_data: true });
});

test("a refused switch goes back and says why", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    controlEnabled: true,
    snapshot: withReport({ save: false }),
    controlAnswer: () => ({
      status: 200,
      body: { ok: false, error: "invalid_report", message: "--no-metrics keeps no report to save" },
    }),
  });
  const doc = page.window.document;
  const save = doc.getElementById("save-report");
  save.click();
  assert.equal(save.checked, true);
  await settle(page.window);
  assert.equal(save.checked, false);
  const error = doc.getElementById("runs-error");
  assert.equal(error.hidden, false);
  assert.equal(
    error.textContent,
    "Couldn't change the switch: --no-metrics keeps no report to save"
  );
});

test("Save report now shows with control, works only while a run is in progress", async () => {
  const idle = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { controlEnabled: true });
  const idleButton = idle.window.document.getElementById("save-now");
  assert.equal(idleButton.hidden, false);
  assert.equal(idleButton.getAttribute("aria-disabled"), "true");

  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    controlEnabled: true,
    snapshot: withReport({ folder: "goose-runs/20261022-101500" }, "maintain"),
  });
  const button = page.window.document.getElementById("save-now");
  assert.equal(button.getAttribute("aria-disabled"), "false");
  assert.equal(button.textContent, "Save report now");
  button.click();
  await settle(page.window);
  const posts = page.requests.filter((r) => r.method === "POST");
  assert.deepEqual(
    posts.map((r) => [r.path, r.bearer, r.url.includes("token=")]),
    [["/api/v1/control/report/now", "Bearer " + TOKEN, false]]
  );
});

test("the banner after a run offers the report's downloads, or says why it failed", async () => {
  const saved = withReport({ last_report: "goose-runs/nightly", last_report_name: "nightly" });
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { snapshot: saved });
  const banner = page.window.document.getElementById("run-banner");
  assert.equal(banner.getAttribute("aria-live"), "polite");
  assert.ok(
    banner.textContent.startsWith(
      "Report saved in goose-runs/nightly. Download HTML, JSON or Markdown."
    ),
    banner.textContent
  );
  const buttons = Array.from(banner.querySelectorAll("button"));
  assert.deepEqual(
    buttons.map((b) => b.textContent),
    ["HTML", "JSON", "Markdown"]
  );
  const before = page.requests.length;
  buttons[2].click();
  await settle(page.window);
  assert.equal(page.requests[before].path, "/api/v1/reports/nightly/report.md");

  const failed = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: withReport({ error: "No space left on device (os error 28)" }),
  });
  assert.equal(
    failed.window.document.getElementById("run-banner").textContent,
    "Couldn't save the report: No space left on device (os error 28)"
  );
  // While a run is in progress, no banner.
  const running = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: withReport({ last_report: "goose-runs/nightly" }, "maintain"),
  });
  assert.equal(running.window.document.getElementById("run-banner").textContent, "");
});

test("the closed banner says Goose has exited", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { closedData: "1" });
  await settle(page.window);
  assert.equal(bannerText(page.window), "Goose has exited.");
});

test("Quit shows only with control on while idle and sends a Bearer POST", async () => {
  // Read-only, the card shows its notice and hides Quit with the other buttons.
  const observe = await openPage("/?token=" + TOKEN, FRESH, TOKEN);
  assert.equal(observe.window.document.getElementById("ctrl-quit").parentElement.hidden, true);

  const running = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    controlEnabled: true,
    snapshot: { ...IDLE_SNAPSHOT, phase: "maintain" },
  });
  assert.equal(running.window.document.getElementById("ctrl-quit").hidden, true);

  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { controlEnabled: true });
  const doc = page.window.document;
  const quit = doc.getElementById("ctrl-quit");
  assert.equal(quit.hidden, false);
  assert.equal(quit.getAttribute("aria-disabled"), "false");
  quit.click();
  await settle(page.window);
  const posts = page.requests.filter((r) => r.method === "POST");
  assert.deepEqual(
    posts.map((r) => [r.path, r.bearer, r.url.includes("token=")]),
    [["/api/v1/control/quit", "Bearer " + TOKEN, false]]
  );
  assert.equal(doc.getElementById("ctrl-status").textContent, "Goose is shutting down.");
  assert.equal(quit.hidden, true);
});

test("Delete shows only with control on, confirms, and sends DELETE with the Bearer header", async () => {
  const observe = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { reports: LISTING });
  assert.equal(observe.window.document.querySelectorAll("button.run-delete").length, 0);
  assert.equal(observe.window.document.getElementById("runs-delete-col").hidden, true);

  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    reports: LISTING,
    controlEnabled: true,
  });
  const doc = page.window.document;
  assert.equal(doc.getElementById("runs-delete-col").hidden, false);
  const buttons = Array.from(doc.querySelectorAll("button.run-delete"));
  assert.equal(buttons.length, LISTING.reports.length);
  // The running run's folder can't be deleted.
  assert.deepEqual(
    buttons.map((b) => b.getAttribute("aria-disabled") === "true"),
    [false, false, false, false, false, true]
  );
  const asked = [];
  let answer = false;
  page.window.confirm = (text) => {
    asked.push(text);
    return answer;
  };
  const listed = () => page.requests.filter((r) => r.path === "/api/v1/reports").length;
  const before = listed();

  // The running row's Delete does nothing.
  buttons[5].click();
  await settle(page.window);
  assert.deepEqual(asked, []);

  // Declined: nothing is sent.
  buttons[1].click();
  await settle(page.window);
  assert.deepEqual(asked, [
    "Delete the reports and logs in nightly?",
  ]);
  assert.equal(page.requests.filter((r) => r.method === "DELETE").length, 0);

  answer = true;
  buttons[1].click();
  await settle(page.window);
  const deletes = page.requests.filter((r) => r.method === "DELETE");
  assert.deepEqual(
    deletes.map((r) => [r.path, r.bearer, r.url.includes("token=")]),
    [["/api/v1/reports/nightly", "Bearer " + TOKEN, false]]
  );
  // The list is fetched again after the delete.
  assert.equal(listed(), before + 1);
  assert.equal(doc.getElementById("runs-error").hidden, true);
});

test("a failed Delete shows its error after the list reloads", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    reports: LISTING,
    controlEnabled: true,
    controlToken: "other",
  });
  const doc = page.window.document;
  page.window.confirm = () => true;
  const listed = () => page.requests.filter((r) => r.path === "/api/v1/reports").length;
  const before = listed();
  doc.querySelectorAll("button.run-delete")[1].click();
  await settle(page.window);
  assert.equal(page.requests.filter((r) => r.method === "DELETE").length, 1);
  assert.equal(listed(), before + 1);
  const error = doc.getElementById("runs-error");
  assert.equal(error.hidden, false);
  assert.equal(error.textContent, "Delete failed: HTTP 401");
});

test("the list reloads whenever a run starts or a report is written, with no phase change", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    reports: LISTING,
    eventStream: true,
  });
  const listed = () => page.requests.filter((r) => r.path === "/api/v1/reports").length;
  const snapshot = (folder, lastReport) =>
    JSON.stringify(withReport({ folder, last_report: lastReport }));

  // The first snapshot lists the reports.
  page.events.emit("snapshot", snapshot("goose-runs/a", "goose-runs/a"));
  await settle(page.window);
  assert.equal(listed(), 1);
  page.events.emit("snapshot", snapshot("goose-runs/a", "goose-runs/a"));
  await settle(page.window);
  assert.equal(listed(), 1, "the same report lists nothing");

  // A run starts: its folder changes, the last report does not.
  page.events.emit("snapshot", snapshot("goose-runs/b", "goose-runs/a"));
  await settle(page.window);
  assert.equal(listed(), 2, "a run started");
  // Its report is written: the last report changes, the folder does not.
  page.events.emit("snapshot", snapshot("goose-runs/b", "goose-runs/b"));
  await settle(page.window);
  assert.equal(listed(), 3, "a report was written");
  // A short run started and ended between two snapshots: both change.
  page.events.emit("snapshot", snapshot("goose-runs/c", "goose-runs/c"));
  await settle(page.window);
  assert.equal(listed(), 4);
});

test("without control the switches stay read only as snapshots arrive", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { eventStream: true });
  const doc = page.window.document;
  for (const save of [true, false]) {
    page.events.emit("snapshot", JSON.stringify(withReport({ save }, "maintain")));
    await settle(page.window);
    assert.equal(doc.getElementById("save-report").checked, save);
    assert.equal(doc.getElementById("save-report").getAttribute("aria-disabled"), "true");
    assert.equal(doc.getElementById("graph-data").getAttribute("aria-disabled"), "true");
    // A click on a read only switch changes nothing and sends nothing.
    doc.getElementById("save-report").click();
    await settle(page.window);
    assert.equal(doc.getElementById("save-report").checked, save);
    assert.equal(page.requests.filter((r) => r.method === "POST").length, 0);
    assert.equal(doc.getElementById("save-now").hidden, true);
  }
});

test("Save report now lists the reports again once its report is written", async () => {
  const folder = "goose-runs/20261022-101500";
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    controlEnabled: true,
    eventStream: true,
  });
  page.events.emit("snapshot", JSON.stringify(withReport({ folder }, "maintain")));
  await settle(page.window);
  const listed = () => page.requests.filter((r) => r.path === "/api/v1/reports").length;
  const before = listed();
  page.window.document.getElementById("save-now").click();
  await settle(page.window);
  // No timer: the list waits for the snapshot that counts the write.
  assert.equal(listed(), before);
  page.events.emit("snapshot", JSON.stringify(withReport({ folder, writes: 1 }, "maintain")));
  await settle(page.window);
  assert.equal(listed(), before + 1);
});

test("a switch keeps focus while its change is sent, and Save report now sends once", async () => {
  let answer = null;
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    controlEnabled: true,
    eventStream: true,
    controlAnswer: () =>
      new Promise((resolve) => {
        answer = resolve;
      }),
  });
  const doc = page.window.document;
  const folder = "goose-runs/20261022-101500";
  page.events.emit("snapshot", JSON.stringify(withReport({ folder }, "maintain")));
  await settle(page.window);
  const posts = () => page.requests.filter((r) => r.method === "POST").length;

  const save = doc.getElementById("save-report");
  save.focus();
  save.click();
  await settle(page.window);
  assert.equal(doc.activeElement, save);
  assert.equal(save.disabled, false);
  assert.equal(save.getAttribute("aria-disabled"), "true");
  // A click while it is unavailable changes nothing and sends nothing.
  const checked = save.checked;
  save.click();
  await settle(page.window);
  assert.equal(save.checked, checked);
  assert.equal(posts(), 1);
  answer({ status: 200, body: { ok: true, message: "save report off" } });
  await settle(page.window);
  assert.equal(save.getAttribute("aria-disabled"), "false");

  const now = doc.getElementById("save-now");
  now.click();
  await settle(page.window);
  assert.equal(posts(), 2);
  // A snapshot while it is pending leaves it unavailable.
  page.events.emit("snapshot", JSON.stringify(withReport({ folder }, "maintain")));
  await settle(page.window);
  assert.equal(now.getAttribute("aria-disabled"), "true");
  now.click();
  await settle(page.window);
  assert.equal(posts(), 2);
  answer({ status: 200, body: { ok: true, message: "saving" } });
  await settle(page.window);
  assert.equal(now.getAttribute("aria-disabled"), "false");
});

test("a report that fails while the run goes on shows in the runs panel", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { eventStream: true });
  const error = page.window.document.getElementById("runs-error");
  const folder = "goose-runs/20261022-101500";
  const reason = "Can't write to " + folder + "/report.html: Permission denied (os error 13)";
  page.events.emit("snapshot", JSON.stringify(withReport({ folder, error: reason }, "maintain")));
  await settle(page.window);
  assert.equal(error.hidden, false);
  assert.equal(error.textContent, "Couldn't save the report: " + reason);
  page.events.emit("snapshot", JSON.stringify(withReport({ folder }, "maintain")));
  await settle(page.window);
  assert.equal(error.hidden, true);
});

test("a run with no folder shows why for the whole run, with Save report off", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    controlEnabled: true,
    eventStream: true,
  });
  const doc = page.window.document;
  const reason = "Can't write to goose-runs/20261022-101500: Permission denied (os error 13)";
  page.events.emit(
    "snapshot",
    JSON.stringify(withReport({ save: false, folder: null, error: reason }, "maintain"))
  );
  await settle(page.window);
  const banner = doc.getElementById("folder-banner");
  assert.equal(banner.hidden, false);
  assert.equal(banner.textContent, "Save report is off for this run. " + reason);
  const save = doc.getElementById("save-report");
  assert.equal(save.checked, false);
  assert.equal(save.getAttribute("aria-disabled"), "true");
  assert.equal(save.closest("label").title, "Off for this run: " + reason);
  assert.equal(doc.getElementById("save-now").getAttribute("aria-disabled"), "true");
  page.events.emit("snapshot", JSON.stringify(withReport({ save: true }, "idle")));
  await settle(page.window);
  assert.equal(banner.hidden, true);
  assert.equal(save.getAttribute("aria-disabled"), "false");
});

test("one report is counted as one, and a long note stays on one line with all of it on hover", async () => {
  const notes = "a long note ".repeat(20).trim();
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    reports: { dir: "goose-runs", reports: [report("nightly", { notes })] },
  });
  const doc = page.window.document;
  assert.equal(doc.getElementById("runs-footer").textContent, "1 report in goose-runs");
  const cell = doc.querySelector("#runs-body td.run-notes");
  assert.equal(cell.title, notes);
  assert.equal(cell.textContent, notes);
});

test("the banner offers downloads only for a folder the routes take", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: withReport({ last_report: "goose-runs/nightly run", last_report_name: null }),
  });
  const banner = page.window.document.getElementById("run-banner");
  assert.equal(banner.textContent, "Report saved in goose-runs/nightly run.");
  assert.equal(banner.querySelectorAll("button").length, 0);
  const named = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: withReport({ last_report: "goose-runs/nightly", last_report_name: "nightly" }),
  });
  assert.equal(named.window.document.querySelectorAll("#run-banner button").length, 3);
});

test("each run's HTML, JSON and Markdown buttons fetch that format", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { reports: LISTING });
  const buttons = Array.from(runRows(page.window)[0].querySelectorAll("button.run-download"));
  const before = page.requests.length;
  for (const button of buttons) button.click();
  await settle(page.window);
  assert.deepEqual(
    page.requests.slice(before).map((r) => r.path),
    [
      "/api/v1/reports/nightly-2/report.html",
      "/api/v1/reports/nightly-2/report.json",
      "/api/v1/reports/nightly-2/report.md",
    ]
  );
});
