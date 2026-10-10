// The latency chart (#713): which series it draws, its title, note and
// tooltip footer, and charting one request picked in the requests table. Run
// with `npm test`; `pretest` builds src/latency.ts into test-build/, and the
// page tests need `npm run build`.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  ADJUSTED_NOTE,
  OFF_NOTE,
  latencyChartData,
} from "./test-build/latency.mjs";
import { IDLE_SNAPSHOT } from "./idle-snapshot.mjs";
import { FRESH, openPage, openPages, settle } from "./test-harness.mjs";

const TOKEN = "s3cret";
const NOT_KEPT =
  "Trends are kept for the first 100 request names seen this run, or since metrics were last reset.";

test.afterEach(() => {
  while (openPages.length) openPages.pop().window.close();
});

function series(fields) {
  return { count: [], avg: [], p50: [], p95: [], p99: [], max: [], ...fields };
}

const MEASURED = series({
  count: [0, 1, 4],
  avg: [null, 20, 30],
  p50: [null, 20, 25],
  p95: [null, 20, 40],
  p99: [null, 20, 41],
  max: [null, 20, 41],
});
const ADJUSTED = series({
  count: [0, 1, 7],
  avg: [null, 20, 900],
  p50: [null, 20, 1000],
  p95: [null, 20, 2000],
  p99: [null, 20, 2000],
  max: [null, 20, 2041],
});

function input(fields) {
  return {
    rps: [0, 1, 4],
    avg_latency_ms: [0, 20, 30],
    latency_ms: MEASURED,
    latency_co_adjusted_ms: series({}),
    percentiles_over_time: true,
    metrics_disabled: false,
    co_active: false,
    selected: null,
    ...fields,
  };
}

test("the adjusted series is used only when mitigation is active and it is not empty", () => {
  const measured = latencyChartData(input({ latency_co_adjusted_ms: ADJUSTED }));
  assert.deepEqual(measured.p99, [null, 20, 41]);
  const empty = latencyChartData(input({ co_active: true }));
  assert.deepEqual(empty.p99, [null, 20, 41]);
  assert.equal(empty.note, "");
  const adjusted = latencyChartData(
    input({ co_active: true, latency_co_adjusted_ms: ADJUSTED })
  );
  assert.deepEqual(adjusted.p50, [null, 20, 1000]);
  assert.deepEqual(adjusted.p95, [null, 20, 2000]);
  assert.deepEqual(adjusted.p99, [null, 20, 2000]);
  assert.deepEqual(adjusted.max, [null, 20, 2041]);
  assert.equal(adjusted.note, ADJUSTED_NOTE);
});

test("the average comes from the chosen series", () => {
  assert.deepEqual(latencyChartData(input({})).avg, [null, 20, 30]);
  assert.deepEqual(
    latencyChartData(input({ co_active: true, latency_co_adjusted_ms: ADJUSTED })).avg,
    [null, 20, 900]
  );
});

test("with percentiles off the average is avg_latency_ms, null where rps is 0", () => {
  const off = latencyChartData(
    input({ percentiles_over_time: false, latency_ms: series({}) })
  );
  assert.equal(off.percentiles, false);
  assert.deepEqual(off.avg, [null, 20, 30]);
  assert.deepEqual(off.p99, []);
  assert.equal(off.footer(1), null);
});

test("null percentiles pass through", () => {
  const held = series({
    count: [3, 2],
    avg: [10, 12],
    p50: [10, null],
    p95: [11, null],
    p99: [12, null],
    max: [12, 14],
  });
  const shown = latencyChartData(input({ rps: [3, 2], latency_ms: held }));
  assert.deepEqual(shown.p50, [10, null]);
  assert.deepEqual(shown.p95, [11, null]);
  assert.deepEqual(shown.p99, [12, null]);
  assert.deepEqual(shown.max, [12, 14]);
});

test("titles", () => {
  assert.equal(latencyChartData(input({})).title, "Latency by second (ms)");
  assert.equal(
    latencyChartData(input({ selected: { method: "GET", name: "/about" } })).title,
    "Latency by second (ms): GET /about"
  );
});

test("notes", () => {
  assert.equal(
    latencyChartData(input({ percentiles_over_time: false })).note,
    "Percentiles are off for this run (--no-percentiles-over-time)."
  );
  assert.equal(OFF_NOTE, "Percentiles are off for this run (--no-percentiles-over-time).");
  assert.equal(
    latencyChartData(input({ percentiles_over_time: false, metrics_disabled: true })).note,
    ""
  );
  assert.equal(
    latencyChartData(input({ co_active: true, latency_co_adjusted_ms: ADJUSTED })).note,
    "Adjusted: includes the synthetic times coordinated omission mitigation adds. Hover for the measured p99."
  );
  assert.equal(latencyChartData(input({})).note, "");
});

test("footers", () => {
  const measured = latencyChartData(input({}));
  assert.equal(measured.footer(0), null);
  assert.equal(measured.footer(1), "1 request, max 20 ms");
  assert.equal(measured.footer(2), "4 requests, max 41 ms");
  const noMax = latencyChartData(
    input({ latency_ms: series({ count: [2], avg: [5], p50: [5], p95: [5], p99: [5], max: [null] }) })
  );
  assert.equal(noMax.footer(0), "2 requests");
  const adjusted = latencyChartData(
    input({ co_active: true, latency_co_adjusted_ms: ADJUSTED })
  );
  assert.equal(adjusted.footer(1), "1 request, max 20 ms");
  assert.equal(
    adjusted.footer(2),
    "4 requests + 3 synthetic, max 2041 ms, measured p99 41 ms"
  );
  const heldP99 = latencyChartData(
    input({
      co_active: true,
      latency_ms: series({ count: [0, 2], avg: [null, 5], p50: [null, null], p95: [null, null], p99: [null, null], max: [null, 6] }),
      latency_co_adjusted_ms: series({ count: [2, 5], avg: [700, 400], p50: [null, null], p95: [null, null], p99: [null, null], max: [800, 900] }),
    })
  );
  assert.equal(heldP99.footer(0), "0 requests + 2 synthetic, max 800 ms");
  assert.equal(heldP99.footer(1), "2 requests + 3 synthetic, max 900 ms");
});

function requestRow(name, has_series) {
  return {
    method: "GET",
    name,
    request_count: 10,
    failure_count: 0,
    requests_per_second: 1,
    failures_per_second: 0,
    response_time_avg_ms: 12.5,
    response_time_min_ms: 1,
    response_time_max_ms: 40,
    percentile_ms: { p50: 10, p95: 30, p99: 40 },
    status_codes: [],
    co_adjusted: null,
    has_series,
  };
}

function snapshotWith(requests, flags = {}) {
  return {
    ...IDLE_SNAPSHOT,
    phase: "maintain",
    requests,
    flags: { ...IDLE_SNAPSHOT.flags, ...flags },
  };
}

function seriesRequests(page) {
  return page.requests.filter((r) => r.path === "/api/v1/series");
}

const wait = (window, ms) => new Promise((resolve) => window.setTimeout(resolve, ms));

test("a snapshot with percentiles off shows the off note", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: snapshotWith([], { percentiles_over_time: false }),
  });
  const note = page.window.document.getElementById("chart-latency-note");
  assert.equal(note.hidden, false);
  assert.equal(note.textContent, OFF_NOTE);
  assert.equal(
    page.window.document.getElementById("chart-latency").getAttribute("aria-label"),
    "Latency by second: average"
  );
});

test("a name with a series is a button, one without is text with a title", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: snapshotWith([requestRow("/kept", true), requestRow("/late", false)]),
  });
  const cells = page.window.document.querySelectorAll("#requests-body tr td:nth-child(2)");
  const byName = {};
  for (const cell of cells) byName[cell.textContent] = cell;
  const button = byName["/kept"].querySelector("button");
  assert.ok(button);
  assert.equal(button.type, "button");
  assert.equal(button.className, "link-button");
  assert.equal(button.getAttribute("aria-pressed"), "false");
  assert.equal(byName["/late"].querySelector("button"), null);
  assert.equal(byName["/late"].title, NOT_KEPT);
  assert.equal(seriesRequests(page).length, 0, "no fetches while nothing is selected");
});

test("clicking a name charts it until All requests is clicked", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: snapshotWith([requestRow("/a b", true)]),
  });
  const { document } = page.window;
  const all = document.getElementById("chart-latency-all");
  assert.equal(all.hidden, true);
  document.querySelector("#requests-body button").click();
  await settle(page.window);
  const fetched = seriesRequests(page);
  assert.equal(fetched.length, 1);
  const url = new URL(fetched[0].url);
  assert.equal(url.searchParams.get("method"), "GET");
  assert.equal(url.searchParams.get("name"), "/a b");
  assert.equal(url.searchParams.get("token"), TOKEN);
  assert.equal(all.hidden, false);
  assert.equal(
    document.getElementById("chart-latency-title").textContent,
    "Latency by second (ms): GET /a b"
  );
  const row = document.querySelector("#requests-body tr");
  assert.equal(row.getAttribute("aria-selected"), "true");
  assert.equal(row.querySelector("button").getAttribute("aria-pressed"), "true");

  all.click();
  await settle(page.window);
  assert.equal(all.hidden, true);
  assert.equal(
    document.getElementById("chart-latency-title").textContent,
    "Latency by second (ms)"
  );
  assert.equal(document.querySelector("#requests-body tr").getAttribute("aria-selected"), null);
  const after = seriesRequests(page).length;
  await wait(page.window, 1300);
  assert.equal(seriesRequests(page).length, after, "polling stopped");
});

test("a 404 keeps the selection", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: snapshotWith([requestRow("/gone", true)]),
    seriesStatus: 404,
  });
  const { document } = page.window;
  document.querySelector("#requests-body button").click();
  await settle(page.window);
  assert.equal(seriesRequests(page).length, 1);
  assert.equal(document.getElementById("chart-latency-all").hidden, false);
  assert.equal(
    document.getElementById("chart-latency-title").textContent,
    "Latency by second (ms): GET /gone"
  );
  document.getElementById("chart-latency-all").click();
});

test("a snapshot whose row has no series ends the selection", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: snapshotWith([requestRow("/a", true)]),
    eventStream: true,
  });
  const { document } = page.window;
  page.events.emit("snapshot", JSON.stringify(snapshotWith([requestRow("/a", true)])));
  await settle(page.window);
  document.querySelector("#requests-body button").click();
  await settle(page.window);
  assert.equal(document.getElementById("chart-latency-all").hidden, false);
  page.events.emit("snapshot", JSON.stringify(snapshotWith([requestRow("/a", false)])));
  await settle(page.window);
  assert.equal(document.getElementById("chart-latency-all").hidden, true);
  assert.equal(
    document.getElementById("chart-latency-title").textContent,
    "Latency by second (ms)"
  );
});
