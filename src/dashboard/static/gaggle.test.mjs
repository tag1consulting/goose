// A Gaggle manager's workers on the dashboard: the panel, grid, hover card,
// tables, events, KPI subtitles, header and banner, the RPS chart's toggle
// and markers, the Control card's split previews and replies, keyboard
// access, and worker names that look like markup. Run with `npm test`
// after `npm run build`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { gaggleSnapshot } from "./gaggle-snapshot.mjs";
import { IDLE_SNAPSHOT } from "./idle-snapshot.mjs";
import { FRESH, openPage, openPages, settle } from "./test-harness.mjs";

const TOKEN = "s3cret";

test.afterEach(() => {
  while (openPages.length) openPages.pop().window.close();
});

/** A page fed by an event stream the test drives. */
function open(options = {}, from = FRESH) {
  return openPage("/?token=" + TOKEN, from, TOKEN, { eventStream: true, ...options });
}

async function send(page, snapshot) {
  page.events.emit("snapshot", JSON.stringify(snapshot));
  await settle(page.window);
}

function $(page, selector) {
  return page.window.document.querySelector(selector);
}

function $$(page, selector) {
  return [...page.window.document.querySelectorAll(selector)];
}

function text(page, selector) {
  const node = $(page, selector);
  assert.ok(node, selector);
  return node.textContent;
}

test("a single process shows no Gaggle part", async () => {
  const page = await open();
  await send(page, { ...IDLE_SNAPSHOT, phase: "maintain" });
  const doc = page.window.document;
  for (const id of [
    "workers-panel",
    "gaggle-attention",
    "gaggle-expected",
    "kpi-workers",
    "worker-grid",
    "ctrl-target-split",
    "ctrl-throttle",
    "chart-rps-events",
  ]) {
    assert.equal(doc.getElementById(id), null, "#" + id);
  }
  assert.equal($$(page, ".kpi-sub").length, 0);
  assert.equal(doc.getElementById("kpis").children.length, 5);
  assert.ok(!doc.body.classList.contains("gaggle-mode"));
  assert.ok(!text(page, "#subtitle").includes("Gaggle"));
  // The RPS card stays in the charts grid, with no toggle.
  assert.equal($(page, "#chart-rps").closest("section").id, "charts");
  assert.ok(!$(page, "#chart-users").closest(".chart-card").classList.contains("chart-wide"));
  assert.equal($$(page, ".segmented button").filter((b) => b.textContent === "By worker").length, 0);
});

test("the header, banner, KPI row and workers panel", async () => {
  const page = await open();
  await send(page, gaggleSnapshot());
  assert.match(text(page, "#subtitle"), /^Gaggle manager/);
  assert.equal(text(page, "#gaggle-expected"), "48 workers expected");
  const banner = $(page, "#gaggle-attention");
  assert.ok(!banner.hidden);
  assert.match(banner.textContent, /^3 workers need attention: load-17 saturated \(CPU 97% for 42 s\)/);
  assert.equal(banner.getAttribute("role"), null, "not a live region");

  const subs = $$(page, ".kpi-sub").map((s) => s.textContent);
  assert.deepEqual(subs.slice(0, 5), [
    "+300 at 02:00",
    "throttle 3,000 /s",
    "9.00 /s failing",
    "merged across workers",
    "merged across workers",
  ]);
  assert.equal(text(page, "#kpi-workers"), "47 / 48");
  const tile = $(page, ".kpi-workers");
  assert.equal(tile.querySelector(".kpi-sub").textContent, "1 lost, 1 saturated, 1 lagging");
  assert.ok(tile.classList.contains("kpi-warn"));

  assert.equal(text(page, "#workers-summary"), "47 of 48 running | 1 saturated | 1 lagging | 1 lost");
  // No report sentences in this snapshot: their fold is hidden.
  assert.ok($(page, ".workers-checks-wrap").hidden);
  // The Workers panel and the RPS card share the first row of the charts
  // grid, right after the KPI row: the same RPS chart, not a second one.
  const charts = $(page, "#charts");
  assert.equal(charts.children[0].id, "workers-panel");
  assert.ok(charts.children[1].contains($(page, "#chart-rps")));
  assert.equal($$(page, "canvas#chart-rps").length, 1);
  assert.equal(charts.previousElementSibling.id, "kpis");
  assert.ok($(page, "#chart-users").closest(".chart-card").classList.contains("chart-wide"));
});

test("the grid and tables update in place", async () => {
  const page = await open();
  await send(page, gaggleSnapshot());
  const tiles = $$(page, "#worker-grid button");
  assert.equal(tiles.length, 48);
  assert.equal(tiles[17].textContent, "!");
  assert.ok(tiles[17].classList.contains("tile-saturated"));
  assert.equal(tiles[42].textContent, "~");
  assert.equal(tiles[31].textContent, "x");
  assert.equal(tiles[31].getAttribute("aria-label"), "load-31: lost");
  assert.equal(tiles[0].textContent, "");
  assert.ok(tiles[0].className.includes("tile-load-"));
  const compact = $$(page, "#workers-compact tbody tr");
  assert.equal(compact.length, 6);
  // load-17 recovers: the same tile and rows, rewritten.
  await send(page, gaggleSnapshot({ saturatedFlag: false }));
  const after = $$(page, "#worker-grid button");
  assert.equal(after.length, 48);
  after.forEach((tile, i) => assert.equal(tile, tiles[i], "tile " + i + " kept"));
  assert.equal(after[17].textContent, "");
  const rows = $$(page, "#workers-compact tbody tr");
  rows.forEach((row, i) => assert.equal(row, compact[i], "row " + i + " kept"));
  assert.equal(text(page, "#workers-summary"), "47 of 48 running | 1 lagging | 1 lost");
  assert.ok(!$(page, ".kpi-workers").querySelector(".kpi-sub").textContent.includes("saturated"));
});

test("the compact table, its note, and the full data table", async () => {
  const page = await open();
  await send(page, gaggleSnapshot());
  const rows = $$(page, "#workers-compact tbody tr").map((tr) =>
    [...tr.cells].map((td) => td.textContent)
  );
  assert.deepEqual(rows[0], ["load-31", "lost", "0", "-", "-", "-", "-", "lost 03:05"]);
  assert.deepEqual(rows[1], ["load-17", "running", "19", "41", "212 ms", "97%", "0.4 s", "saturated"]);
  assert.deepEqual(rows[2].slice(0, 2), ["load-42", "running"]);
  assert.equal(rows[2][7], "lagging");
  // Flags are coloured text, never colour alone.
  const lost = $(page, "#workers-compact tbody tr td:last-child span");
  assert.equal(lost.textContent, "lost 03:05");
  assert.ok(lost.classList.contains("flag-alarm"));
  assert.equal(text(page, ".workers-rows-note"), "6 of 48 rows, most urgent first.");
  const details = $(page, "#workers-data");
  assert.equal(details.querySelector("summary").textContent, "Show data table");
  assert.equal($$(page, "#workers-table tbody tr").length, 0, "built only when opened");
  details.open = true;
  details.dispatchEvent(new page.window.Event("toggle"));
  const full = $$(page, "#workers-table tbody tr");
  assert.equal(full.length, 48, "every row sent");
  const head = $$(page, "#workers-table thead th").map((th) => th.textContent);
  for (const column of ["Fail %", "Avg ms", "Req/user/s"]) assert.ok(head.includes(column), column);
  // A worker with no CPU shows a dash.
  const five = full.find((tr) => tr.cells[0].textContent === "load-05");
  assert.equal(five.cells[5].textContent, "-", "no CPU from load-05");
});

test("events newest first, the rest in a scrolling region", async () => {
  const page = await open();
  await send(page, gaggleSnapshot());
  const shown = $$(page, "#gaggle-events li").map((li) => li.textContent);
  assert.equal(shown.length, 8);
  assert.equal(shown[0], "04:12 load-17 saturated (CPU 97% for 5 s)");
  assert.equal(shown[2], "03:05 load-31 lost: connection reset");
  assert.equal(shown[4], "02:00 users 600 to 900, acknowledged by 48 of 48 in 0.4 s");
  assert.ok($$(page, "#gaggle-events li")[2].classList.contains("event-alarm"));
  const more = $(page, ".gaggle-events-more");
  assert.ok(!more.hidden);
  assert.equal(more.getAttribute("role"), "region");
  assert.equal(more.tabIndex, 0);
  const older = [...more.querySelectorAll("li")].map((li) => li.textContent);
  assert.equal(older[older.length - 1], "00:02 48 of 48 workers joined, run started");
});

test("a state change is announced once, from the events", async () => {
  const page = await open();
  await send(page, gaggleSnapshot());
  const announce = $(page, "#announce");
  // What happened before the page opened is not announced.
  assert.ok(!announce.textContent.includes("load-17"), announce.textContent);
  const lagging = {
    second: 253,
    kind: "worker_lagging",
    label: "load-09 lagging",
    text: "load-09 lagging (last delta 4 s ago)",
  };
  await send(page, gaggleSnapshot({ events: [lagging] }));
  assert.equal(announce.textContent, "load-09 lagging (last delta 4 s ago).");
  announce.textContent = "";
  await send(page, gaggleSnapshot({ events: [lagging] }));
  await send(page, gaggleSnapshot({ events: [lagging] }));
  assert.equal(announce.textContent, "", "said once");
  // An event that is not a state change is not announced.
  await send(
    page,
    gaggleSnapshot({
      events: [
        { second: 254, kind: "users_changed", label: "users 960", text: "users 900 to 960, acknowledged by 47 of 47 in 0.3 s" },
        lagging,
      ],
    })
  );
  assert.equal(announce.textContent, "");
  // A worker recovering is said once too.
  await send(
    page,
    gaggleSnapshot({
      events: [
        { second: 255, kind: "worker_recovered", label: "load-09 recovered", text: "load-09 recovered: no longer lagging" },
        lagging,
      ],
    })
  );
  assert.equal(announce.textContent, "load-09 recovered: no longer lagging.");
  // Many at once: the first three said, the rest counted.
  const left = [0, 1, 2, 3, 4].map((i) => ({
    second: 256,
    kind: "worker_excluded",
    label: null,
    text: "load-0" + i + "'s metrics left out: it missed a metrics reset",
  }));
  await send(page, gaggleSnapshot({ events: [...left] }));
  assert.equal(
    announce.textContent,
    "load-04's metrics left out: it missed a metrics reset. load-03's metrics left out: it missed a metrics reset. load-02's metrics left out: it missed a metrics reset. 2 more worker changes in Recent gaggle events."
  );
});

test("the RPS chart's toggle is remembered, and draws by worker", async () => {
  const page = await open({ canvas: true });
  await send(page, gaggleSnapshot());
  const buttons = $$(page, "#rps-view button");
  assert.deepEqual(buttons.map((b) => b.textContent), ["Total", "By worker"]);
  assert.equal(buttons[0].getAttribute("aria-pressed"), "true", "Total by default");
  buttons[1].click();
  assert.equal(buttons[1].getAttribute("aria-pressed"), "true");
  assert.equal(page.window.localStorage.getItem("goose-dashboard-rps-view"), "by-worker");
  page.canvas.calls.length = 0;
  await send(page, gaggleSnapshot());
  const drawn = page.canvas.calls
    .filter((c) => c.id === "chart-rps" && c.name === "fillText")
    .map((c) => String(c.value[0]));
  assert.ok(drawn.includes("load-00"), "a worker's line in the legend: " + drawn.join("|"));
  assert.ok(drawn.includes("39 others"), drawn.join("|"));
  // The markers' labels are drawn at the top of the chart.
  assert.ok(drawn.includes("users 900"), drawn.join("|"));
  // The chart's data table names each line: its text fallback.
  const details = $(page, "#chart-rps-details");
  details.open = true;
  details.dispatchEvent(new page.window.Event("toggle"));
  const table = $(page, "#chart-rps-by-worker");
  assert.ok(!table.hidden);
  const head = [...table.querySelectorAll("thead th")].map((th) => th.textContent);
  assert.deepEqual(head, ["Second", "load-00", "load-01", "load-02", "load-03", "load-04", "load-05", "load-06", "load-07", "39 others"]);
  const first = [...table.querySelectorAll("tbody tr")[0].cells].map((td) => td.textContent);
  assert.deepEqual(first, ["0", "63", "63", "63", "63", "63", "63", "63", "63", "2,483"]);
  buttons[0].click();
  assert.ok(table.hidden, "Total hides it");
  // A new page for the same viewer opens by worker.
  const local = { "goose-dashboard-rps-view": "by-worker" };
  const again = await open({}, { session: {}, local });
  await send(again, gaggleSnapshot());
  assert.equal($$(again, "#rps-view button")[1].getAttribute("aria-pressed"), "true");
});

test("blocked storage leaves the toggle at Total", async () => {
  const page = await open();
  Object.defineProperty(page.window, "localStorage", {
    get() {
      throw new page.window.DOMException("blocked", "SecurityError");
    },
  });
  await send(page, gaggleSnapshot());
  const buttons = $$(page, "#rps-view button");
  assert.equal(buttons[0].getAttribute("aria-pressed"), "true");
  buttons[1].click();
  assert.equal(buttons[1].getAttribute("aria-pressed"), "true");
});

test("markers are in the chart's data table, with their sentences", async () => {
  const page = await open();
  await send(page, gaggleSnapshot());
  const rows = $$(page, "#chart-rps-events tbody tr").map((tr) => [...tr.cells].map((td) => td.textContent));
  assert.deepEqual(rows[0], ["02:00", "120", "users 600 to 900, acknowledged by 48 of 48 in 0.4 s"]);
  assert.deepEqual(rows[rows.length - 1], ["04:12", "252", "load-17 saturated (CPU 97% for 5 s)"]);
  // Only the events within the chart's window: those with a label.
  assert.equal(rows.length, 5);
  const canvas = $(page, "#chart-rps");
  assert.equal(canvas.tabIndex, 0, "the chart takes focus for its markers");
  assert.equal(canvas.getAttribute("aria-describedby"), "chart-rps-marker");
});

/** A page with control on, answering control requests with `answer`. */
function controlPage(answer) {
  return open({ controlEnabled: true, controlAnswer: answer });
}

test("the Control card says how a change splits", async () => {
  const page = await controlPage();
  await settle(page.window);
  await send(page, gaggleSnapshot());
  assert.equal(text(page, ".gaggle-note"), "changes go to every worker");
  assert.equal(text(page, "#ctrl-target-split"), "split: 20 users on 7 workers, 19 on 40");
  assert.ok(!$(page, ".gaggle-throttle").hidden, "the run has a throttle");
  assert.equal($(page, "#ctrl-throttle").value, "3000");
  assert.equal(text(page, "#ctrl-throttle-split"), "split: 64 req/s on 39 workers, 63 on 8");
  assert.equal(text(page, "#set-host-workers"), "same on all 47");
  // As the user types, from the same rules over each worker's users.
  const target = $(page, "#ctrl-target");
  target.value = "947";
  target.dispatchEvent(new page.window.Event("input", { bubbles: true }));
  assert.equal(text(page, "#ctrl-target-split"), "split: 21 users on 7 workers, 20 on 40");
  const throttle = $(page, "#ctrl-throttle");
  throttle.value = "47";
  throttle.dispatchEvent(new page.window.Event("input", { bubbles: true }));
  assert.equal(text(page, "#ctrl-throttle-split"), "split: 1 req/s on each of 47 workers");
  // A value the manager would refuse is not previewed: the current split stays.
  target.value = "99999999999";
  target.dispatchEvent(new page.window.Event("input", { bubbles: true }));
  assert.equal(text(page, "#ctrl-target-split"), "split: 20 users on 7 workers, 19 on 40");
  throttle.value = "5000000";
  throttle.dispatchEvent(new page.window.Event("input", { bubbles: true }));
  assert.equal(text(page, "#ctrl-throttle-split"), "split: 64 req/s on 39 workers, 63 on 8");
});

test("without a throttle the throttle field stays hidden", async () => {
  const page = await controlPage();
  const snap = gaggleSnapshot();
  snap.settings = { ...snap.settings, throttle: null };
  snap.gaggle.previews.throttle = null;
  await send(page, snap);
  assert.ok($(page, ".gaggle-throttle").hidden);
  assert.ok($(page, "#ctrl-throttle-split").hidden);
  assert.equal(text(page, ".kpi-sub"), "+300 at 02:00");
  assert.equal($$(page, ".kpi-sub")[1].textContent, "");
});

test("an applied change, a refusal and a worker lost during a change", async () => {
  const answers = [];
  const page = await controlPage((command, body) => {
    answers.push([command, body]);
    if (command === "users" && body.users === 960) {
      return {
        status: 200,
        body: {
          ok: true,
          command,
          message: "users configured; accepted by 48 of 48 workers",
          phase: "maintain",
          active_users: 900,
          acks: { acknowledged: 48, workers: 48, secs: 0.4 },
        },
      };
    }
    if (command === "users") {
      return {
        status: 200,
        body: {
          ok: true,
          command,
          message: "users configured; accepted by 47 of 48 workers; load-31 lost",
          phase: "maintain",
          active_users: 900,
          acks: { acknowledged: 47, workers: 48, secs: 10.0 },
        },
      };
    }
    return {
      status: 200,
      body: {
        ok: false,
        command,
        error: "invalid_phase",
        field: "throttle",
        message: "failed to configure throttle, the run is stopping",
        phase: "decrease",
        active_users: 900,
      },
    };
  });
  await send(page, gaggleSnapshot({ duration: 120 }));
  const target = $(page, "#ctrl-target");
  target.value = "960";
  target.dispatchEvent(new page.window.Event("input", { bubbles: true }));
  $(page, "#ctrl-apply").click();
  await settle(page.window);
  const status = $(page, "#ctrl-status");
  assert.equal(status.textContent, "Applied users 960 at 02:00: acknowledged by 48 of 48 workers in 0.4 s.");
  assert.ok(status.classList.contains("ok"));
  // Lost during the change.
  target.value = "1000";
  target.dispatchEvent(new page.window.Event("input", { bubbles: true }));
  $(page, "#ctrl-apply").click();
  await settle(page.window);
  assert.equal(
    status.textContent,
    "Applied users 1,000 at 02:00: acknowledged by 47 of 48 workers in 10.0 s; 1 worker was lost during the change."
  );
  // A refusal says why, as a sentence.
  $(page, "#ctrl-throttle").value = "2000";
  $(page, "#ctrl-throttle").dispatchEvent(new page.window.Event("input", { bubbles: true }));
  $(page, "#ctrl-throttle-apply").click();
  await settle(page.window);
  assert.equal(status.textContent, "failed to configure throttle, the run is stopping");
  assert.ok(status.classList.contains("error"));
  assert.deepEqual(answers[2], ["settings", { throttle: 2000 }]);
});

test("keyboard focus reaches every tile, and the hover card follows it", async () => {
  const page = await open();
  await send(page, gaggleSnapshot());
  const tiles = $$(page, "#worker-grid button");
  // One tab stop for the grid; arrows move within it.
  assert.equal(tiles.filter((t) => t.tabIndex === 0).length, 1);
  const grid = $(page, "#worker-grid");
  assert.equal(grid.getAttribute("role"), "group");
  assert.ok(grid.getAttribute("aria-label"));
  tiles[0].focus();
  const card = $(page, "#worker-card");
  assert.ok(!card.hidden);
  assert.equal(card.getAttribute("role"), "tooltip");
  assert.equal(tiles[0].getAttribute("aria-describedby"), "worker-card");
  const key = (k) => grid.dispatchEvent(new page.window.KeyboardEvent("keydown", { key: k, bubbles: true }));
  const reached = new Set([0]);
  for (let i = 1; i < tiles.length; i++) {
    key("ArrowRight");
    reached.add(tiles.indexOf(page.window.document.activeElement));
  }
  assert.equal(reached.size, 48, "every tile");
  key("Home");
  assert.equal(page.window.document.activeElement, tiles[0]);
  key("ArrowDown");
  assert.equal(page.window.document.activeElement, tiles[16], "rows of 16");
  // The saturated worker's card.
  key("ArrowDown");
  key("ArrowRight");
  assert.equal(page.window.document.activeElement, tiles[33]);
  tiles[17].focus();
  assert.deepEqual(tiles.filter((t) => t.tabIndex === 0), [tiles[17]], "one tab stop after a tile is focused");
  const lines = [...card.children].map((c) => c.textContent);
  assert.deepEqual(lines, ["load-17  saturated", "CPU 97% of 4 cores for 42 s", "41 req/s (-35%), p95 212 ms"]);
  tiles[31].focus();
  assert.deepEqual([...card.children].map((c) => c.textContent), ["load-31  lost", "lost at 03:05", "connection reset"]);
  key("Escape");
  assert.ok(card.hidden);
  // The panel is labelled; its regions take focus.
  assert.equal($(page, "#workers-panel").getAttribute("aria-labelledby"), "workers-heading");
  assert.equal($(page, "#workers-data .table-wrap").tabIndex, 0);
  // Colour is never alone: each flagged tile has a glyph, each in the legend.
  const legend = $$(page, ".worker-legend li").map((li) => li.textContent.trim());
  assert.deepEqual(legend.slice(0, 4), ["running (shade = load share)", "! saturated", "~ lagging", "x lost"]);
});

test("worker names that look like markup are shown as text", async () => {
  const hostile = "<img src=x onerror=alert(1)>\"'&";
  let alerted = 0;
  const page = await open();
  page.window.alert = () => {
    alerted += 1;
  };
  const before = page.window.document.querySelectorAll("*").length;
  const snap = gaggleSnapshot({
    names: { 17: hostile, 31: hostile + "2" },
    events: [
      { second: 253, kind: "worker_lagging", label: hostile + " lagging", text: hostile + " lagging (last delta 4 s ago)" },
    ],
  });
  await send(page, gaggleSnapshot());
  await send(page, snap);
  const doc = page.window.document;
  assert.equal(doc.querySelectorAll("img").length, 0, "no element from a name");
  assert.equal(alerted, 0, "no handler ran");
  const tiles = $$(page, "#worker-grid button");
  assert.equal(tiles[17].getAttribute("aria-label"), hostile + ": saturated");
  tiles[17].focus();
  assert.equal($(page, "#worker-card").children[0].textContent, hostile + "  saturated");
  const compact = $$(page, "#workers-compact tbody tr").map((tr) => tr.cells[0].textContent);
  assert.ok(compact.includes(hostile) && compact.includes(hostile + "2"), compact.join("|"));
  assert.equal($$(page, "#gaggle-events li")[0].textContent, "04:13 " + hostile + " lagging (last delta 4 s ago)");
  assert.equal(text(page, "#announce"), hostile + " lagging (last delta 4 s ago).");
  const markers = $$(page, "#chart-rps-events tbody tr").map((tr) => tr.cells[2].textContent);
  assert.ok(markers.includes(hostile + " lagging (last delta 4 s ago)"));
  assert.ok(text(page, "#gaggle-attention").includes(hostile + "2 lost at 03:05"));
  // Only the elements the page made: as many as for ordinary names.
  const plain = await open();
  await send(plain, gaggleSnapshot());
  await send(
    plain,
    gaggleSnapshot({
      names: { 17: "plain-17", 31: "plain-31" },
      events: [{ second: 253, kind: "worker_lagging", label: "x lagging", text: "x lagging (last delta 4 s ago)" }],
    })
  );
  assert.equal(
    doc.querySelectorAll("*").length,
    plain.window.document.querySelectorAll("*").length,
    "the same elements as with plain names"
  );
  assert.ok(doc.querySelectorAll("*").length > before);
});

test("the report sentences fold under the summary line, and a p95 under 1 ms reads <1", async () => {
  const page = await open();
  const snap = gaggleSnapshot();
  snap.gaggle.summary.degraded = ["Degraded: 1 of the 48 Gaggle workers were lost during the run: load-31 (connection reset)."];
  snap.gaggle.summary.checks = ["Check: load-42's metrics arrived late (longest gap 5.2 s)."];
  snap.gaggle.grid.p95[0] = 0;
  await send(page, snap);
  const fold = $(page, ".workers-checks-wrap");
  assert.ok(!fold.hidden);
  assert.equal(fold.tagName, "DETAILS");
  assert.ok(!fold.open, "folded");
  assert.equal(fold.querySelector("summary").textContent, "2 sentences the reports would open with");
  assert.deepEqual([...fold.querySelectorAll("li")].map((li) => li.textContent), [
    snap.gaggle.summary.degraded[0],
    snap.gaggle.summary.checks[0],
  ]);
  const row = $$(page, "#workers-compact tbody tr").find((tr) => tr.cells[0].textContent === "load-00");
  assert.equal(row.cells[4].textContent, "<1 ms");
});

test("a healthy Gaggle shows no banner and no warning", async () => {
  const page = await open();
  const snap = gaggleSnapshot({ saturatedFlag: false });
  const g = snap.gaggle;
  g.summary = { ...g.summary, attention: null, lost: 0, lagging: 0, running: 48, healthy: 48 };
  g.grid.states = g.grid.states.map(() => 2);
  g.grid.flags = g.grid.flags.map(() => 0);
  await send(page, snap);
  assert.ok($(page, "#gaggle-attention").hidden);
  assert.ok(!$(page, ".kpi-workers").classList.contains("kpi-warn"));
  assert.equal($(page, ".kpi-workers .kpi-sub").textContent, "all healthy");
});
