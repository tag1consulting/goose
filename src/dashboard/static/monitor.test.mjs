// Following a running test with a screen reader (#711): one polite live
// region that speaks only on a change, tables whose rows stay the same
// elements across snapshots, a banner written only when its text changes, and
// each chart's data as a table and a one sentence summary. Run in jsdom with
// `npm test` after `npm run build`.
//
// jsdom has no canvas, so the charts draw nothing; their data tables and
// summaries are read back from the DOM, which the client fills from the same
// series it hands the charts.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import { IDLE_SNAPSHOT } from "./idle-snapshot.mjs";
import { FRESH, openPage, openPages, settle } from "./test-harness.mjs";

const TOKEN = "s3cret";

const int = (n) => Math.round(n).toLocaleString();
const rate = (n) =>
  n.toLocaleString(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });

test.afterEach(() => {
  while (openPages.length) openPages.pop().window.close();
});

function snapshotWith(fields = {}, flags = {}) {
  return {
    ...IDLE_SNAPSHOT,
    phase: "maintain",
    ...fields,
    flags: { ...IDLE_SNAPSHOT.flags, ...flags },
  };
}

function requestRow(name, request_count) {
  return {
    method: "GET",
    name,
    request_count,
    failure_count: 0,
    requests_per_second: request_count / 10,
    failures_per_second: 0,
    response_time_avg_ms: 12.5,
    response_time_min_ms: 1,
    response_time_max_ms: 40,
    percentile_ms: { p50: 10, p95: 30, p99: 40 },
    status_codes: [],
  };
}

/** A page fed by an event stream the test drives. */
async function livePage(options = {}) {
  return openPage("/?token=" + TOKEN, FRESH, TOKEN, { eventStream: true, ...options });
}

/** Sends one snapshot on the stream and lets the page render it. */
async function send(page, snapshot) {
  page.events.emit("snapshot", JSON.stringify(snapshot));
  await settle(page.window);
}

function el(page, id) {
  const found = page.window.document.getElementById(id);
  assert.ok(found, "#" + id);
  return found;
}

/** Counts the DOM mutations under `node`. */
function mutations(window, node) {
  const counter = { count: 0 };
  new window.MutationObserver((records) => {
    counter.count += records.length;
  }).observe(node, { childList: true, characterData: true, subtree: true, attributes: true });
  return counter;
}

function cells(tr) {
  return Array.from(tr.cells).map((td) => td.textContent);
}

/** The data row of a requests table whose name cell is `name`. */
function requestTr(page, name) {
  return Array.from(el(page, "requests-body").rows).find((tr) => tr.cells[1].textContent === name);
}

function isLive(node) {
  return node.closest("[aria-live], [role=status], [role=alert], [role=log]") !== null;
}

// --- 1. One polite live region ----------------------------------------------

test("the live region speaks only when the phase or the connection changes", async () => {
  const page = await livePage();
  const announce = el(page, "announce");
  assert.equal(announce.getAttribute("role"), "status");
  const writes = mutations(page.window, announce);
  const seen = [];
  const step = async (action) => {
    action();
    await settle(page.window);
    seen.push(announce.textContent + " #" + writes.count);
  };
  const phase = (p) => () => page.events.emit("snapshot", JSON.stringify(snapshotWith({ phase: p })));
  // The first phase the page sees is where it joined, not a change.
  await step(phase("increase"));
  await step(phase("increase"));
  await step(phase("maintain"));
  await step(phase("maintain"));
  await step(() => page.events.source.onerror());
  await step(() => page.events.source.onerror());
  await step(phase("maintain"));
  await step(phase("idle"));
  await step(phase("idle"));
  assert.deepEqual(seen, [
    " #0",
    " #0",
    "Phase: maintain. #1",
    "Phase: maintain. #1",
    "Connection lost. #2",
    "Connection lost. #2",
    "Connection restored. #3",
    "Test finished. #4",
    "Test finished. #4",
  ]);
});

test("a recovery and a phase change in one snapshot are both said", async () => {
  const page = await livePage();
  const announce = el(page, "announce");
  await send(page, snapshotWith({ phase: "maintain" }));
  page.events.source.onerror();
  await settle(page.window);
  page.events.source.onerror();
  await settle(page.window);
  assert.equal(announce.textContent, "Connection lost.");
  // The test finished during the outage: the first snapshot back says both.
  await send(page, snapshotWith({ phase: "idle" }));
  assert.equal(announce.textContent, "Connection restored. Test finished.");
});

test("a recovery through polling is said", async () => {
  // The poll answers the snapshot the stream last sent, so only the
  // connection changes.
  const page = await livePage({ snapshot: snapshotWith() });
  await send(page, snapshotWith());
  const announce = el(page, "announce");
  const said = [];
  new page.window.MutationObserver(() => said.push(announce.textContent))
    .observe(announce, { childList: true, characterData: true, subtree: true });
  // The third error falls back to polling, and the poll answers.
  for (let i = 0; i < 3; i++) {
    page.events.source.onerror();
    await settle(page.window);
  }
  assert.deepEqual(said, ["Connection lost.", "Connection restored."]);
  assert.equal(el(page, "connection-label").textContent, "poll");
});

test("a control response does not announce its phase; the next snapshot does", async () => {
  const page = await livePage({ controlEnabled: true });
  await send(page, snapshotWith({ phase: "idle" }));
  const announce = el(page, "announce");
  const writes = mutations(page.window, announce);
  const start = el(page, "ctrl-start");
  assert.equal(start.disabled, false, "Start is enabled when idle");
  // The stub answers Start with phase increase.
  start.click();
  await settle(page.window);
  assert.equal(writes.count, 0, "the control response wrote nothing");
  assert.equal(announce.textContent, "");
  // A snapshot from just before the command: still idle, nothing to say.
  await send(page, snapshotWith({ phase: "idle" }));
  assert.equal(writes.count, 0);
  await send(page, snapshotWith({ phase: "increase" }));
  assert.equal(announce.textContent, "Phase: increase.");
});

test("a lost connection is said once, in the live region, not also in the banner", async () => {
  const page = await livePage({ snapshotStatus: "reject" });
  await send(page, snapshotWith());
  const banner = el(page, "banner");
  // The stream closes for good: the page polls, and the poll fails.
  page.events.source.readyState = 2;
  page.events.source.onerror();
  assert.equal(banner.textContent, "", "no polling banner the next render would clear");
  await settle(page.window);
  assert.equal(el(page, "announce").textContent, "Connection lost.");
  assert.equal(banner.textContent, "", "no Poll failed banner saying it again");
  assert.ok(banner.classList.contains("hidden"));
});

test("repeated stream errors that end in a failing poll say the loss once", async () => {
  const page = await livePage({ snapshotStatus: "reject" });
  await send(page, snapshotWith());
  const announce = el(page, "announce");
  const said = [];
  new page.window.MutationObserver(() => said.push(announce.textContent))
    .observe(announce, { childList: true, characterData: true, subtree: true });
  // The stream stays open and reconnecting (readyState 1); the third error
  // falls back to polling, and the poll fails.
  for (let i = 0; i < 3; i++) {
    page.events.source.onerror();
    await settle(page.window);
  }
  assert.deepEqual(said, ["Connection lost."], "no false Connection restored. between");
  assert.equal(announce.textContent, "Connection lost.");
});

test("the pill says poll as soon as the stream closes, before a poll answers", async () => {
  const page = await livePage({ snapshotStatus: "pending" });
  await send(page, snapshotWith());
  const label = el(page, "connection-label");
  assert.equal(label.textContent, "SSE");
  const polls = () => page.requests.filter((r) => r.path === "/api/v1/snapshot").length;
  const before = polls();
  page.events.source.readyState = 2;
  page.events.source.onerror();
  await settle(page.window);
  assert.equal(polls(), before + 1, "one closed error polls at once, without the error streak");
  assert.equal(label.textContent, "poll", "not SSE while the first poll is out");
  assert.ok(el(page, "connection").classList.contains("poll"));
  assert.ok(!el(page, "connection").classList.contains("live"));
});

// --- Polling and the way back to the stream (#707) ---------------------------

const POLL_TIMER = -707;

/**
 * Keeps the 2s poll timer's tick and each 8s poll timeout to run by hand;
 * every other timer runs as usual.
 */
function driveTimers(window) {
  const timers = { tick: null, cleared: false, timeouts: new Map(), next: 1e6 };
  const realSetInterval = window.setInterval;
  const realClearInterval = window.clearInterval;
  const realSetTimeout = window.setTimeout;
  const realClearTimeout = window.clearTimeout;
  window.setInterval = (fn, ms) => {
    if (ms !== 2000) return realSetInterval(fn, ms);
    timers.tick = fn;
    timers.cleared = false;
    return POLL_TIMER;
  };
  window.clearInterval = (id) => {
    if (id === POLL_TIMER) timers.cleared = true;
    else realClearInterval(id);
  };
  window.setTimeout = (fn, ms, ...args) => {
    if (ms !== 8000) return realSetTimeout(fn, ms, ...args);
    const id = timers.next++;
    timers.timeouts.set(id, fn);
    return id;
  };
  window.clearTimeout = (id) => {
    if (!timers.timeouts.delete(id)) realClearTimeout(id);
  };
  // Runs every pending poll timeout, as if 8s passed; says how many.
  timers.expire = () => {
    const due = [...timers.timeouts.values()];
    timers.timeouts.clear();
    for (const fn of due) fn();
    return due.length;
  };
  return timers;
}

/** Holds each snapshot poll until the test answers it. */
function holdPolls(window) {
  const polls = [];
  const realFetch = window.fetch;
  window.fetch = (url, init) => {
    if (!String(url).includes("/api/v1/snapshot")) return realFetch(url, init);
    return new Promise((resolve, reject) => polls.push({ resolve, reject, settled: false }));
  };
  return polls;
}

const answered = (snap) => ({ status: 200, ok: true, json: () => Promise.resolve(snap) });

function snapshotRequests(page) {
  return page.requests.filter((r) => r.path === "/api/v1/snapshot").length;
}

function watchAnnounce(page) {
  const announce = el(page, "announce");
  const said = [];
  new page.window.MutationObserver(() => said.push(announce.textContent))
    .observe(announce, { childList: true, characterData: true, subtree: true });
  return said;
}

/** A live page whose stream closes for good, so it polls. */
async function pollingPage(options = {}, hold = false) {
  const page = await livePage({ snapshot: snapshotWith(), ...options });
  await send(page, snapshotWith());
  const timers = driveTimers(page.window);
  const polls = hold ? holdPolls(page.window) : null;
  page.events.source.readyState = 2;
  page.events.source.onerror();
  await settle(page.window);
  assert.ok(timers.tick, "polling started");
  return { page, timers, polls };
}

/** Answers every poll still out. */
async function answerPolls(p) {
  for (const poll of p.polls) {
    if (poll.settled) continue;
    poll.settled = true;
    poll.resolve(answered(snapshotWith()));
  }
  await settle(p.page.window);
}

/**
 * Ticks the poll timer until the page opens a retry of the stream, at most
 * five times, and returns the retry. `hold` pages have each poll answered
 * before the next tick; the poll sent with the retry stays out.
 */
async function tickUntilRetry(p) {
  const { sources } = p.page.events;
  for (let i = 0; i < 5 && sources.length === 1; i++) {
    if (p.polls) await answerPolls(p);
    p.timers.tick();
    await settle(p.page.window);
  }
  assert.equal(sources.length, 2, "the polling page opened a retry of the stream");
  return p.page.events.source;
}

test("a polling page whose retry hears closed shows the end and sends nothing more", async () => {
  const p = await pollingPage();
  await tickUntilRetry(p);
  p.page.events.emit("closed", "");
  await settle(p.page.window);
  assert.equal(el(p.page, "connection-label").textContent, "finished");
  assert.equal(el(p.page, "banner").textContent, "Goose has exited.");
  assert.ok(p.timers.cleared, "the poll timer is stopped");
  assert.equal(p.page.events.source.readyState, 2, "the retry is closed, so it does not reconnect");
  const sent = snapshotRequests(p.page);
  for (let i = 0; i < 12; i++) {
    p.timers.tick();
    await settle(p.page.window);
  }
  assert.equal(snapshotRequests(p.page), sent, "no snapshot request after the end");
  assert.equal(p.page.events.sources.length, 2, "no further retry after the end");
  assert.equal(el(p.page, "connection-label").textContent, "finished");
});

for (const outcome of ["resolve", "reject"]) {
  test(`a polling page whose retry gets a snapshot is live again and a late poll that ${outcome}s leaves it so`, async () => {
    const p = await pollingPage({}, true);
    await tickUntilRetry(p);
    const said = watchAnnounce(p.page);
    const late = p.polls[p.polls.length - 1];
    assert.equal(late.settled, false, "a poll is out when the stream comes back");
    await send(p.page, snapshotWith());
    assert.equal(el(p.page, "connection-label").textContent, "SSE");
    assert.ok(p.timers.cleared, "polling stopped");
    late.settled = true;
    if (outcome === "resolve") late.resolve(answered(snapshotWith()));
    else late.reject(new TypeError("network error"));
    await settle(p.page.window);
    assert.equal(el(p.page, "connection-label").textContent, "SSE", "the late poll is dropped");
    assert.deepEqual(said, [], "nothing to say: the connection never dropped");
    // The retry is now the live stream: its loss falls back to polling.
    p.page.events.source.readyState = 2;
    p.page.events.source.onerror();
    await settle(p.page.window);
    assert.equal(el(p.page, "connection-label").textContent, "poll");
  });
}

test("a poll that never answers times out and later polls still go", async () => {
  const p = await pollingPage({ snapshotStatus: "pending" });
  const sent = snapshotRequests(p.page);
  assert.equal(p.timers.expire(), 1, "the poll out has a timeout");
  await settle(p.page.window);
  assert.equal(el(p.page, "announce").textContent, "Connection lost.", "a timed out poll failed");
  assert.equal(p.page.events.sources.length, 1, "only a 503 retries the stream at once");
  p.timers.tick();
  await settle(p.page.window);
  assert.equal(snapshotRequests(p.page), sent + 1, "the next tick polls again");
});

// Was "a slow poll failing after a later one answered does not say the
// connection was lost", which sent two polls at once; polls no longer
// overlap (#707), so a failure is always the newest word.
test("a slow poll holds back the next tick instead of overlapping it", async () => {
  const p = await pollingPage({}, true);
  const said = watchAnnounce(p.page);
  assert.equal(p.polls.length, 1);
  p.timers.tick();
  p.timers.tick();
  await settle(p.page.window);
  assert.equal(p.polls.length, 1, "no second poll while the first is out");
  await answerPolls(p);
  p.timers.tick();
  await settle(p.page.window);
  assert.equal(p.polls.length, 2, "the next tick polls once the first answered");
  assert.deepEqual(said, []);
  p.polls[1].settled = true;
  p.polls[1].reject(new TypeError("network error"));
  await settle(p.page.window);
  assert.deepEqual(said, ["Connection lost."]);
});

test("a retry of the stream that fails says nothing and polling goes on", async () => {
  const p = await pollingPage();
  const said = watchAnnounce(p.page);
  const banner = el(p.page, "banner");
  const bannerBefore = [banner.textContent, banner.className];
  const retry = await tickUntilRetry(p);
  const sent = snapshotRequests(p.page);
  retry.onerror();
  await settle(p.page.window);
  assert.equal(retry.readyState, 2, "closed, so the browser does not reconnect it");
  assert.deepEqual(said, []);
  assert.deepEqual([banner.textContent, banner.className], bannerBefore);
  assert.equal(el(p.page, "connection-label").textContent, "poll");
  p.timers.tick();
  await settle(p.page.window);
  assert.equal(snapshotRequests(p.page), sent + 1, "polling goes on");
  for (let i = 0; i < 3; i++) {
    p.timers.tick();
    await settle(p.page.window);
  }
  assert.equal(p.page.events.sources.length, 2, "no retry before the fifth tick");
  p.timers.tick();
  await settle(p.page.window);
  assert.equal(p.page.events.sources.length, 3, "the fifth tick retries again");
});

test("a poll answering 503 opens a retry of the stream at once", async () => {
  const p = await pollingPage({ snapshotStatus: 503 });
  assert.equal(p.page.events.sources.length, 2, "a retry opened without waiting");
  const open = () => p.page.events.sources.filter((s) => s.readyState === 1).length;
  // The 503 came on the first tick; ticks two to four answer 503 again.
  for (let i = 0; i < 3; i++) {
    p.timers.tick();
    await settle(p.page.window);
  }
  assert.equal(p.page.events.sources.length, 2, "one retry at a time");
  assert.equal(open(), 1, "one retry at a time");
  // The fifth tick swaps the silent retry for a fresh one, still one open.
  p.timers.tick();
  await settle(p.page.window);
  assert.equal(p.page.events.sources.length, 3, "the fifth tick replaced the retry");
  assert.equal(open(), 1, "one retry at a time");
});

test("a poll still out does not hold back the fifth tick's retry", async () => {
  const p = await pollingPage({ snapshotStatus: "pending" });
  const sent = snapshotRequests(p.page);
  for (let i = 0; i < 4; i++) {
    p.timers.tick();
    await settle(p.page.window);
  }
  assert.equal(snapshotRequests(p.page), sent, "the first poll is still out");
  assert.equal(p.page.events.sources.length, 2, "the fifth tick retried anyway");
  assert.equal(el(p.page, "connection-label").textContent, "poll", "a retry leaves the pill alone");
});

test("polling again after a retry came back counts five ticks afresh", async () => {
  const p = await pollingPage({}, true);
  p.polls[0].settled = true;
  p.polls[0].resolve({ status: 503, ok: false, json: () => Promise.resolve(null) });
  await settle(p.page.window);
  assert.equal(p.page.events.sources.length, 2, "the 503 retried at once");
  await send(p.page, snapshotWith());
  assert.equal(el(p.page, "connection-label").textContent, "SSE");
  p.page.events.source.readyState = 2;
  p.page.events.source.onerror();
  await settle(p.page.window);
  let ticks = 0;
  while (p.page.events.sources.length === 2 && ticks < 6) {
    await answerPolls(p);
    p.timers.tick();
    ticks += 1;
    await settle(p.page.window);
  }
  assert.equal(ticks, 4, "the retry waits for the fifth tick of the new polling");
});

test("a poll failing after closed leaves the page finished", async () => {
  const p = await pollingPage({}, true);
  await tickUntilRetry(p);
  const late = p.polls[p.polls.length - 1];
  p.page.events.emit("closed", "");
  await settle(p.page.window);
  late.settled = true;
  late.reject(new TypeError("network error"));
  await settle(p.page.window);
  assert.equal(el(p.page, "connection-label").textContent, "finished");
  assert.notEqual(el(p.page, "announce").textContent, "Connection lost.");
  assert.equal(el(p.page, "banner").textContent, "Goose has exited.");
});

// --- 2. Tables update in place ----------------------------------------------

test("a request keeps its row element across snapshots; only changed cells are written", async () => {
  const page = await livePage();
  await send(page, snapshotWith({ requests: [requestRow("/a", 5), requestRow("/b", 3)] }));
  const a = requestTr(page, "/a");
  const b = requestTr(page, "/b");
  assert.equal(a.cells[2].textContent, int(5));
  const bWrites = mutations(page.window, b);

  await send(page, snapshotWith({ requests: [requestRow("/a", 6), requestRow("/b", 3)] }));
  assert.equal(requestTr(page, "/a"), a, "the same <tr> for /a");
  assert.equal(requestTr(page, "/b"), b, "the same <tr> for /b");
  assert.equal(a.cells[2].textContent, int(6), "the cell shows the new count");
  assert.equal(bWrites.count, 0, "an unchanged row is not touched");
  const still = mutations(page.window, el(page, "requests-body"));
  const emptyWrites = mutations(page.window, el(page, "errors-body"));
  await send(page, snapshotWith({ requests: [requestRow("/a", 6), requestRow("/b", 3)] }));
  assert.equal(still.count, 0, "an identical snapshot moves and writes nothing");
  assert.equal(emptyWrites.count, 0, "nor does the empty state row");

  // /a goes and /c comes: only those rows change.
  await send(page, snapshotWith({ requests: [requestRow("/b", 3), requestRow("/c", 9)] }));
  const body = el(page, "requests-body");
  assert.equal(body.rows.length, 2);
  assert.equal(requestTr(page, "/b"), b);
  assert.equal(a.parentNode, null, "the row for /a is removed");
  const c = requestTr(page, "/c");
  assert.ok(c && c !== a, "a new row for /c");
});

test("a GET and a POST of the same name are two rows with their own counts", async () => {
  const post = (name, n) => ({ ...requestRow(name, n), method: "POST" });
  const page = await livePage();
  await send(page, snapshotWith({ requests: [requestRow("/a", 5), post("/a", 3)] }));
  await send(page, snapshotWith({ requests: [requestRow("/a", 7), post("/a", 4)] }));
  const rows = Array.from(el(page, "requests-body").rows).map((tr) => cells(tr).slice(0, 3));
  assert.deepEqual(rows, [
    ["GET", "/a", int(7)],
    ["POST", "/a", int(4)],
  ]);
});

test("a row losing its adjusted times keeps its element and drops the titles", async () => {
  const adjusted = {
    ...requestRow("/a", 5),
    co_adjusted: {
      response_time_avg_ms: 20,
      response_time_max_ms: 80,
      percentile_ms: { p50: 15, p95: 60, p99: 80 },
    },
  };
  const titled = (tr) => Array.from(tr.cells).filter((td) => td.hasAttribute("title"));
  const page = await livePage();
  await send(page, snapshotWith({ requests: [adjusted] }, { co_active: true }));
  const tr = requestTr(page, "/a");
  assert.ok(titled(tr).length > 0, "adjusted cells carry the measured time");
  const writes = mutations(page.window, tr);
  await send(page, snapshotWith({ requests: [adjusted] }, { co_active: true }));
  assert.equal(writes.count, 0, "an identical row is not touched");
  await send(page, snapshotWith({ requests: [requestRow("/a", 5)] }, { co_active: true }));
  assert.equal(requestTr(page, "/a"), tr, "the same <tr>");
  assert.deepEqual(titled(tr), [], "no cell keeps a title");
});

test("sorting moves the existing rows instead of building new ones", async () => {
  const page = await livePage();
  await send(page, snapshotWith({
    requests: [requestRow("/a", 5), requestRow("/b", 3), requestRow("/c", 9)],
  }));
  const body = el(page, "requests-body");
  const before = Array.from(body.rows);
  assert.deepEqual(before.map((tr) => tr.cells[1].textContent), ["/c", "/a", "/b"]);
  page.window.document.querySelector('#requests-table th[data-sort="name"]').click();
  const after = Array.from(body.rows);
  assert.deepEqual(after.map((tr) => tr.cells[1].textContent), ["/a", "/b", "/c"]);
  assert.ok(after.every((tr) => before.includes(tr)), "every row is the same element");
  // And the next snapshot keeps them too.
  await send(page, snapshotWith({
    requests: [requestRow("/a", 6), requestRow("/b", 3), requestRow("/c", 9)],
  }));
  assert.ok(Array.from(body.rows).every((tr, i) => tr === after[i]), "and stays so");
});

test("scenario, transaction and error rows keep their elements too", async () => {
  const timing = { response_time_avg_ms: 1, response_time_min_ms: 1, response_time_max_ms: 1,
    percentile_ms: { p50: 1, p95: 1, p99: 1 } };
  const scenario = (users) => ({
    scenario_index: 0, scenario_name: "S", users, run_count: 1, runs_per_second: 1, ...timing,
  });
  const transaction = (runs) => ({
    scenario_index: 0, scenario_name: "S", transaction_index: 0, transaction_name: "T",
    run_count: runs, failure_count: 0, runs_per_second: 1, failures_per_second: 0, ...timing,
  });
  const error = (n) => ({ method: "GET", name: "/a", error: "500", occurrences: n });
  const snap = (n) => snapshotWith({
    scenarios: [scenario(n)], transactions: [transaction(n)], errors: [error(n)],
  });
  const page = await livePage();
  await send(page, snap(1));
  const ids = ["scenarios-body", "transactions-body", "errors-body"];
  const first = ids.map((id) => el(page, id).rows[0]);
  await send(page, snap(2));
  ids.forEach((id, i) => {
    assert.equal(el(page, id).rows[0], first[i], "same row in #" + id);
    assert.ok(cells(first[i]).includes(int(2)), "new numbers in #" + id);
  });
});

test("error rows whose fields join to the same text stay separate rows", async () => {
  const error = (name, text, n) => ({ method: "GET", name, error: text, occurrences: n });
  // Joined with spaces, both read "GET Login page timeout".
  const snap = (n) => snapshotWith({
    errors: [
      error("Login", "page timeout", n),
      error("Login page", "timeout", n + 1),
      error("Login", "page error", n + 2),
    ],
  });
  const page = await livePage();
  await send(page, snap(1));
  await send(page, snap(2));
  const rows = Array.from(el(page, "errors-body").rows).map(cells);
  assert.deepEqual(rows, [
    ["GET", "Login", "page error", int(4)],
    ["GET", "Login page", "timeout", int(3)],
    ["GET", "Login", "page timeout", int(2)],
  ]);
});

// --- 3. The banner is written only when its text changes ---------------------

test("the truncation banner is written once, not on every snapshot", async () => {
  const page = await livePage();
  const banner = el(page, "banner");
  const truncated = snapshotWith({}, { requests_truncated: true });
  await send(page, truncated);
  const text = banner.textContent;
  assert.match(text, /request rows truncated/);
  const writes = mutations(page.window, banner);
  await send(page, truncated);
  await send(page, truncated);
  assert.equal(writes.count, 0, "the same text is not written again");
  assert.equal(banner.textContent, text);
  await send(page, snapshotWith({}, { requests_truncated: true, errors_truncated: true }));
  assert.ok(writes.count > 0, "a new text is written");
  assert.match(banner.textContent, /error rows truncated/);
});

// --- 4 and 5. Each chart's data and summary ----------------------------------

/** `count` seconds of a series from `start`, each value from `value(i)`. */
function seriesOf(start, count, value) {
  const values = [];
  for (let i = 0; i < count; i++) values.push(value(i));
  return values;
}

function lineSeries(start, rps, fps, users, latency) {
  return {
    start_second: start,
    rps,
    fps,
    users,
    avg_latency_ms: latency,
    heatmap: null,
  };
}

/** 25 seconds from second 100: second `s` has RPS `s`. */
function series25(start = 100) {
  const n = 25;
  return lineSeries(
    start,
    seriesOf(start, n, (i) => start + i),
    seriesOf(start, n, () => 0.5),
    seriesOf(start, n, () => 50),
    seriesOf(start, n, () => 20)
  );
}

test("a chart's data table is built only once opened, then follows the data in place", async () => {
  const page = await livePage();
  const details = el(page, "chart-rps-details");
  const body = el(page, "chart-rps-rows");
  assert.equal(details.tagName, "DETAILS");
  assert.equal(details.open, false, "collapsed by default");
  assert.equal(isLive(details), false, "not a live region");
  assert.equal(isLive(el(page, "chart-rps-summary")), false, "nor is the summary");

  await send(page, snapshotWith({ series: series25(100) }));
  assert.equal(body.rows.length, 0, "nothing built while closed");

  details.open = true;
  await settle(page.window);
  assert.equal(body.rows.length, 25, "one row per second");
  assert.deepEqual(cells(body.rows[0]), ["100", rate(100), rate(0.5)]);
  const row105 = Array.from(body.rows).find((tr) => tr.cells[0].textContent === "105");
  const row100 = body.rows[0];

  // The window moves on by a second.
  await send(page, snapshotWith({ series: series25(101) }));
  assert.equal(body.rows.length, 25);
  assert.equal(body.rows[0].cells[0].textContent, "101");
  assert.equal(body.rows[24].cells[0].textContent, "125", "the newest second added");
  assert.equal(row100.parentNode, null, "the oldest second removed");
  assert.equal(
    Array.from(body.rows).find((tr) => tr.cells[0].textContent === "105"),
    row105,
    "a second still in the window keeps its row"
  );

  // Closed, it is left alone; opened again, it catches up.
  details.open = false;
  await settle(page.window);
  await send(page, snapshotWith({ series: series25(102) }));
  assert.equal(body.rows[0].cells[0].textContent, "101");
  details.open = true;
  await settle(page.window);
  assert.equal(body.rows[0].cells[0].textContent, "102");
});

test("as the window moves on, only the second leaving and the one arriving touch the table", async () => {
  const page = await livePage();
  const details = el(page, "chart-rps-details");
  const body = el(page, "chart-rps-rows");
  await send(page, snapshotWith({ series: series25(100) }));
  details.open = true;
  await settle(page.window);
  let added = 0;
  let removed = 0;
  new page.window.MutationObserver((records) => {
    for (const r of records) {
      added += r.addedNodes.length;
      removed += r.removedNodes.length;
    }
  }).observe(body, { childList: true });
  await send(page, snapshotWith({ series: series25(101) }));
  assert.equal(body.rows.length, 25);
  // Moving a row detaches it, and a screen reader on it loses its place.
  assert.equal(removed, 1, "only second 100 left the table");
  assert.equal(added, 1, "only second 125 joined it");
});

test("the users and average latency charts have data tables too", async () => {
  const page = await livePage();
  el(page, "chart-users-details").open = true;
  el(page, "chart-latency-details").open = true;
  await settle(page.window);
  await send(page, snapshotWith({ series: series25(100) }, { percentiles_over_time: false }));
  assert.deepEqual(cells(el(page, "chart-users-rows").rows[0]), ["100", int(50)]);
  const latencyTable = el(page, "chart-latency-table");
  assert.equal(latencyTable.hidden, false, "the card shows the average chart");
  assert.equal(el(page, "heatmap-table").hidden, true);
  assert.deepEqual(cells(el(page, "chart-latency-rows").rows[0]), ["100", "20"]);
});

// Buckets as `[time, lowest, highest]`: 0, 10, 50 and 100 ms.
const BUCKETS = [[0, 0, 0], [10, 10, 10], [50, 50, 50], [100, 100, 104]];

// 100 requests: 90 at 10 ms, 5 at 50 ms and 5 at 100 ms, so p50 10, p95 50
// and p99 100.
const STEADY = [1, 90, 2, 5, 3, 5];
// 90 at 10 ms and 10 at 100 ms: p50 10, p95 100, p99 100.
const SLOWER = [1, 90, 3, 10];

/** A heatmap of `count` seconds from `start`, second `s` measured as `measured(s)`. */
function heatmapOf(start, count, measured = () => STEADY) {
  const columns = [];
  for (let s = start; s < start + count; s++) {
    columns.push({ second: s, measured: measured(s), synthetic: [] });
  }
  return {
    generation: 1,
    rev: 1,
    since: 0,
    start_second: start,
    end_second: start + count,
    hold_from: start + count,
    buckets_us: BUCKETS.map(([time, lowest, highest]) => [time * 1000, lowest * 1000, (highest + 1) * 1000 - 1]),
    columns,
  };
}

test("the heatmap's data table is each second's requests and percentiles", async () => {
  const page = await livePage();
  const details = el(page, "chart-latency-details");
  details.open = true;
  await settle(page.window);
  // Second 101 had 40 requests, all at 10 ms.
  const measured = (s) => (s === 101 ? [1, 40] : STEADY);
  await send(page, snapshotWith({ series: { ...series25(100), heatmap: heatmapOf(100, 3, measured) } }));
  assert.equal(el(page, "heatmap-table").hidden, false);
  assert.equal(el(page, "chart-latency-table").hidden, true);
  const body = el(page, "heatmap-rows");
  assert.deepEqual(Array.from(body.rows).map(cells), [
    ["100", int(100), int(10), int(50), int(100)],
    ["101", int(40), int(10), int(10), int(10)],
    ["102", int(100), int(10), int(50), int(100)],
  ]);
  const row101 = body.rows[1];
  await send(page, snapshotWith({ series: { ...series25(100), heatmap: heatmapOf(101, 3) } }));
  assert.equal(body.rows[0], row101, "a second still shown keeps its row");
  assert.equal(body.rows[2].cells[0].textContent, "103");
});

test("the card's data table follows the switch to the heatmap", async () => {
  const page = await livePage();
  await send(page, snapshotWith({ series: series25(100) }, { percentiles_over_time: false }));
  assert.equal(el(page, "chart-latency-table").hidden, false);
  assert.equal(el(page, "heatmap-table").hidden, true);
  await send(page, snapshotWith({ series: { ...series25(100), heatmap: heatmapOf(100, 3) } }));
  assert.equal(el(page, "heatmap-table").hidden, false, "the heatmap's table shows");
  assert.equal(el(page, "chart-latency-table").hidden, true, "the average's table hides");
});

test("an open data table scrolls in its own box instead of widening the page", () => {
  // jsdom cannot lay out a page, so this checks the two facts the layout
  // rests on: each data table sits in a .table-wrap scroll box inside its
  // details, and no rule gives the chart grid a bare 1fr track, whose minimum
  // is its widest content (a 5 column table at 320 px).
  const here = new URL(".", import.meta.url);
  const css = readFileSync(new URL("app.css", here), "utf8");
  const html = readFileSync(new URL("index.html", here), "utf8");
  const { window } = new JSDOM(html.replace("</head>", "<style>" + css + "</style></head>"));
  const doc = window.document;
  const tables = doc.querySelectorAll(".chart-data table");
  assert.equal(tables.length, 4, "rps, users, heatmap and average latency");
  for (const table of tables) {
    const wrap = table.parentElement;
    assert.ok(wrap.classList.contains("table-wrap"), table.id + " sits in a .table-wrap");
    assert.ok(wrap.closest("details.chart-data"), table.id + "'s scroll box is in its details");
    assert.equal(window.getComputedStyle(wrap).overflowX, "auto", table.id + " scrolls sideways");
    // Up to 300 rows: it scrolls down in its box too, so the box has to take
    // focus for a keyboard user to scroll it.
    assert.equal(window.getComputedStyle(wrap).overflowY, "auto", table.id + " scrolls down");
    assert.ok(window.getComputedStyle(wrap).maxHeight !== "none", table.id + " has a height limit");
    assert.equal(wrap.getAttribute("tabindex"), "0", table.id + "'s box takes focus");
    assert.equal(wrap.getAttribute("role"), "region");
    assert.ok((wrap.getAttribute("aria-label") || "").length > 0, table.id + "'s box is named");
  }
  const grids = [];
  const walk = (rules) => {
    for (const rule of rules) {
      if (rule.selectorText === ".charts-grid" && rule.style.gridTemplateColumns) {
        grids.push(rule.style.gridTemplateColumns);
      } else if (rule.cssRules) {
        walk(rule.cssRules);
      }
    }
  };
  walk(doc.styleSheets[0].cssRules);
  assert.equal(grids.length, 2, "the wide layout and the narrow one");
  for (const columns of grids) {
    for (const track of columns.split(/\s+(?![^(]*\))/)) {
      assert.equal(track, "minmax(0, 1fr)", "track in " + columns);
    }
  }
  window.close();
});

test("each summary follows the data: value, then rising, falling or steady", async () => {
  const page = await livePage();
  const rps = el(page, "chart-rps-summary");
  const users = el(page, "chart-users-summary");
  const latency = el(page, "chart-latency-summary");
  assert.equal(rps.textContent, "No data yet.");

  // Ten seconds at 100 then ten at 120: the mean is up 20 percent.
  const step = (before, after) => seriesOf(0, 20, (i) => (i < 10 ? before : after * (i % 2 ? 1.1 : 0.9)));
  // Each RPS and failures series ends in the second still counting, here 7
  // requests and no failures in, which the summary leaves out.
  const counting = (values) => (values.length ? [...values, 7] : values);
  const snap = (rpsValues, usersValues, latencyValues, target = 200) =>
    snapshotWith(
      {
        target_users: target,
        series: lineSeries(
          0,
          counting(rpsValues),
          rpsValues.length ? [...step(0.5, 0.5), 0] : [],
          usersValues,
          latencyValues
        ),
      },
      { percentiles_over_time: false }
    );
  await send(page, snap(step(100, 120), step(100, 120), step(20, 20)));
  assert.equal(rps.textContent, "RPS " + rate(120) + ", rising; failures " + rate(0.5) + "/s.");
  assert.equal(users.textContent, "Users " + int(120) + " of " + int(200) + ", rising.");
  assert.equal(latency.textContent, "Average " + int(20) + " ms, steady.");

  // Up 5 percent is steady; down 20 percent is falling.
  await send(page, snap(step(100, 105), step(120, 96), step(30, 20), 1500));
  assert.equal(rps.textContent, "RPS " + rate(105) + ", steady; failures " + rate(0.5) + "/s.");
  assert.equal(users.textContent, "Users " + int(96) + " of " + int(1500) + ", falling.");
  assert.equal(latency.textContent, "Average " + int(20) + " ms, falling.");

  // No target users (a test plan, or not known yet): no "of 0".
  await send(page, snap(step(100, 120), step(100, 120), step(20, 20), 0));
  assert.equal(users.textContent, "Users " + int(120) + ", rising.");

  // Under 20 seconds there is nothing to compare: no trend word.
  const short = seriesOf(0, 15, () => 80);
  await send(page, snapshotWith(
    { target_users: 200, series: lineSeries(0, short, short.map(() => 0), short, short) },
    { percentiles_over_time: false }
  ));
  assert.equal(rps.textContent, "RPS " + rate(80) + "; failures " + rate(0) + "/s.");

  // From nothing, the older mean is 0: no trend word either.
  await send(page, snap(step(0, 50), step(0, 50), step(0, 50)));
  assert.equal(rps.textContent, "RPS " + rate(50) + "; failures " + rate(0.5) + "/s.");

  // A slow test reads its rate, not 0: the stat card's two decimals.
  const slow = seriesOf(0, 15, () => 0.4);
  await send(page, snapshotWith(
    { series: lineSeries(0, slow, slow.map(() => 0), slow, slow) },
    { percentiles_over_time: false }
  ));
  assert.equal(rps.textContent, "RPS " + rate(0.4) + "; failures " + rate(0) + "/s.");

  // A new run starts with an empty series: back to no data.
  await send(page, snap([], [], []));
  assert.equal(rps.textContent, "No data yet.");
  assert.equal(users.textContent, "No data yet.");
  assert.equal(latency.textContent, "No data yet.");
});

test("the heatmap summary gives recent percentiles and the p95 trend", async () => {
  const page = await livePage();
  const latency = el(page, "chart-latency-summary");
  await send(page, snapshotWith({ series: { ...series25(0), heatmap: heatmapOf(0, 20) } }));
  assert.equal(
    latency.textContent,
    "p50 " + int(10) + " ms, p95 " + int(50) + " ms, p99 " + int(100) + " ms; p95 steady."
  );
  // In the last ten seconds p95 nearly doubles: nine seconds SLOWER and a
  // last one all at 0 ms, so the means are p50 9, p95 90 and p99 90, not the
  // last second's 0.
  await send(page, snapshotWith({
    series: {
      ...series25(0),
      heatmap: heatmapOf(0, 20, (s) => (s < 10 ? STEADY : s === 19 ? [0, 100] : SLOWER)),
    },
  }));
  assert.equal(
    latency.textContent,
    "p50 " + int(9) + " ms, p95 " + int(90) + " ms, p99 " + int(90) + " ms; p95 rising."
  );
  // Nothing to show again: no data.
  await send(page, snapshotWith({ series: { ...series25(0), heatmap: heatmapOf(0, 0) } }));
  assert.equal(latency.textContent, "No data yet.");
  // Under 20 seconds with requests: no trend word.
  await send(page, snapshotWith({ series: { ...series25(0), heatmap: heatmapOf(0, 5) } }));
  assert.equal(
    latency.textContent,
    "p50 " + int(10) + " ms, p95 " + int(50) + " ms, p99 " + int(100) + " ms."
  );
});
