// Saved runs in the dashboard client (app.js), run in jsdom with `npm test`
// after `npm run build`: downloads, the saved runs panel, and the banner
// shown when Goose exits.

import { test } from "node:test";
import assert from "node:assert/strict";
import { IDLE_SNAPSHOT } from "./idle-snapshot.mjs";
import { FRESH, openPage, openPages, settle } from "./test-harness.mjs";

const TOKEN = "s3cret";

function run(id, fields = {}) {
  return {
    format: 1,
    id,
    goose_version: "0.19.0-dev",
    test: "loadtest",
    started: "2026-10-09T12:12:03Z",
    ended: "2026-10-09T12:13:03Z",
    duration_secs: 60,
    max_users: 10,
    hosts: ["http://localhost"],
    requests: 1000,
    failed_requests: 3,
    ended_by: "completed",
    canceled_reason: null,
    baseline: null,
    files: [],
    ...fields,
  };
}

const LISTING = {
  dir: "goose-runs",
  total_bytes: 2048,
  runs: [
    run("2026-10-09-141203-2", { started: "2026-10-09T12:12:03Z", test: "other" }),
    run("2026-10-09-141203", { started: "2026-10-09T12:12:03Z", ended_by: "canceled", canceled_reason: "SIGINT received" }),
    run("2026-10-08-090000", { started: "2026-10-08T07:00:00Z", requests: 0, ended_by: "canceled", canceled_reason: "error budget spent" }),
    run("2026-10-07-090000", { started: "2026-10-07T07:00:00Z", ended_by: "users_exited" }),
  ],
};

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

test("the panel lists runs, newest first, and disables Compare without requests", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { runs: LISTING });
  const rows = runRows(page.window);
  assert.equal(rows.length, 4);
  const cells = rows.map((row) => Array.from(row.cells).map((td) => td.textContent));
  assert.deepEqual(
    cells.map((c) => c[0]),
    LISTING.runs.map((r) => r.id)
  );
  assert.deepEqual(
    cells.map((c) => c[6]),
    ["Finished", "Ctrl-C", "Canceled: error budget spent", "Users exited"]
  );
  const boxes = rows.map((row) => row.querySelector("input.run-compare"));
  assert.deepEqual(
    boxes.map((b) => b.disabled),
    [false, false, true, false]
  );
  const footer = page.window.document.getElementById("runs-footer").textContent;
  assert.equal(footer, "4 saved runs, 2.0 KB in goose-runs");
  assert.equal(page.window.document.getElementById("runs-empty").hidden, true);
  // Listed once at startup, not on a timer.
  await settle(page.window);
  assert.equal(page.requests.filter((r) => r.path === "/api/v1/runs").length, 1);
});

test("an empty runs directory says so", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN);
  assert.equal(runRows(page.window).length, 0);
  const empty = page.window.document.getElementById("runs-empty");
  assert.equal(empty.hidden, false);
  assert.equal(empty.textContent, "No saved runs yet.");
});

test("downloads send the Bearer header and no token in any URL", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { runs: LISTING });
  const before = page.requests.length;
  const button = runRows(page.window)[0].querySelector("button.run-download");
  assert.equal(button.textContent, "HTML");
  button.click();
  await settle(page.window);
  const downloads = page.requests.slice(before);
  assert.equal(downloads.length, 1);
  assert.equal(downloads[0].path, "/api/v1/runs/2026-10-09-141203-2/report.html");
  assert.equal(downloads[0].bearer, "Bearer " + TOKEN);
  assert.ok(!downloads[0].url.includes(TOKEN), downloads[0].url);
  assert.deepEqual(page.downloads.map((d) => d.name), [
    "goose-test-2026-10-09-141203-2-report.html",
  ]);
  for (const d of page.downloads) {
    assert.ok(d.href.startsWith("blob:"), d.href);
    assert.ok(!d.href.includes(TOKEN));
  }
  // The listing too goes with the header, not in the URL.
  for (const r of page.requests.filter((r) => r.path.startsWith("/api/v1/runs"))) {
    assert.equal(r.bearer, "Bearer " + TOKEN);
    assert.ok(!r.url.includes(TOKEN), r.url);
  }
});

test("two checked runs download a comparison, newer as the run", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { runs: LISTING });
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
  assert.equal(url.pathname, "/api/v1/runs/2026-10-09-141203/compare.md");
  assert.equal(url.searchParams.get("baseline"), "2026-10-07-090000");
  assert.equal(url.searchParams.get("token"), null);
  assert.equal(sent[0].bearer, "Bearer " + TOKEN);

  // A third check: no comparison until exactly two.
  rows[0].querySelector("input.run-compare").click();
  assert.equal(compare.hidden, true);
  // Different tests: the warning shows.
  rows[1].querySelector("input.run-compare").click();
  assert.equal(compare.hidden, false);
  assert.equal(note.hidden, false);
  assert.equal(
    note.textContent,
    "These runs are from different tests, so the comparison may not mean much."
  );
});

test("a failed comparison shows the server's reason", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    runs: LISTING,
    runFileStatus: 422,
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

test("the header and the end of run banner show the save state", async () => {
  const saved = {
    ...IDLE_SNAPSHOT,
    save: { state: "on", reason: null, dir: "goose-runs", last_run: "2026-10-09-141203" },
  };
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { snapshot: saved });
  const doc = page.window.document;
  assert.equal(doc.getElementById("save-status").textContent, "Saving to goose-runs");
  const banner = doc.getElementById("run-banner");
  assert.equal(banner.getAttribute("aria-live"), "polite");
  assert.ok(
    banner.textContent.startsWith("Run 2026-10-09-141203 saved. Download HTML, JSON or Markdown."),
    banner.textContent
  );
  assert.deepEqual(
    Array.from(banner.querySelectorAll("button")).map((b) => b.textContent),
    ["HTML", "JSON", "Markdown"]
  );

  const failed = {
    ...IDLE_SNAPSHOT,
    save: { state: "failed", reason: "Permission denied (os error 13)", dir: "goose-runs", last_run: null },
  };
  const other = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { snapshot: failed });
  assert.equal(
    other.window.document.getElementById("save-status").textContent,
    "Not saving: can't create goose-runs (Permission denied (os error 13))"
  );
  const off = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: { ...IDLE_SNAPSHOT, save: { ...IDLE_SNAPSHOT.save, state: "off" } },
  });
  assert.equal(
    off.window.document.getElementById("save-status").textContent,
    "Not saving (turned off)"
  );
  const couldNot = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: {
      ...IDLE_SNAPSHOT,
      save: { state: "on", reason: "No space left on device (os error 28)", dir: "goose-runs", last_run: "2026-10-09-141203" },
    },
  });
  assert.equal(
    couldNot.window.document.getElementById("run-banner").textContent,
    "Couldn't save run 2026-10-09-141203: No space left on device (os error 28)."
  );
});

for (const [name, data, expected] of [
  [
    "a saved run",
    JSON.stringify({ state: "on", reason: null, dir: "goose-runs", last_run: "2026-10-09-141203" }),
    "Goose has exited. This run was saved in goose-runs/2026-10-09-141203 on the machine running Goose.",
  ],
  [
    "a run that couldn't be saved",
    JSON.stringify({ state: "on", reason: "Permission denied (os error 13)", dir: "goose-runs", last_run: "2026-10-09-141203" }),
    "Goose has exited. Couldn't save run 2026-10-09-141203: Permission denied (os error 13).",
  ],
  [
    "no run",
    JSON.stringify({ state: "on", reason: null, dir: "goose-runs", last_run: null }),
    "Goose has exited.",
  ],
  ["an older Goose's 1", "1", "Goose has exited."],
]) {
  test(`the closed banner for ${name}`, async () => {
    const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { closedData: data });
    await settle(page.window);
    assert.equal(bannerText(page.window), expected);
  });
}

test("Quit shows only with control on while idle and sends a Bearer POST", async () => {
  const observe = await openPage("/?token=" + TOKEN, FRESH, TOKEN);
  assert.equal(observe.window.document.getElementById("control-panel").classList.contains("hidden"), true);

  const running = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    controlEnabled: true,
    snapshot: { ...IDLE_SNAPSHOT, phase: "maintain" },
  });
  assert.equal(running.window.document.getElementById("ctrl-quit").hidden, true);

  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { controlEnabled: true });
  const doc = page.window.document;
  const quit = doc.getElementById("ctrl-quit");
  assert.equal(quit.hidden, false);
  assert.equal(quit.disabled, false);
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
