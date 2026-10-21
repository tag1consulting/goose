// The response time heatmap (#713): applying updates, row placement on the
// log latency axis, colours, tooltips, and the card: its select, the Adjusted
// and Measured toggle, polling one request, and the average chart when
// percentiles are off. Run with `npm test`; `pretest` builds src/heatmap.ts
// into test-build/, and the page tests need `npm run build`.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  ADJUSTED_NOTE,
  ARIA_LABEL,
  NOTE,
  OFF_NOTE,
  TITLE,
  applyHeatmap,
  bucketSpan,
  cellColor,
  countPosition,
  heatmapCursor,
  heatmapView,
  percentileOf,
  scaleColor,
  secondLabels,
  secondStep,
  tooltipText,
} from "./test-build/heatmap.mjs";
import { IDLE_SNAPSHOT } from "./idle-snapshot.mjs";
import { FRESH, openPage, openPages, settle } from "./test-harness.mjs";

const TOKEN = "s3cret";

test.afterEach(() => {
  while (openPages.length) openPages.pop().window.close();
});

// --- Goose's buckets, as the server's bucket table lists them -------------

// round_metric_time in src/metrics.rs.
function roundMetricTime(t) {
  if (t <= 100) return t;
  if (t <= 500) return Math.round(t / 10) * 10;
  if (t <= 1000) return Math.round(t / 100) * 100;
  return Math.round(t / 1000) * 1000;
}

// [time, lowest, highest] for every bucket index from 0 to 60 s.
function bucketTable() {
  const values = [];
  for (let v = 0; v <= 100; v++) values.push(v);
  for (let v = 110; v <= 500; v += 10) values.push(v);
  for (let v = 600; v <= 1000; v += 100) values.push(v);
  for (let v = 2000; v <= 60000; v += 1000) values.push(v);
  return values.map((v) => {
    let lowest = v;
    while (lowest > 0 && roundMetricTime(lowest - 1) === v) lowest--;
    let highest = v;
    while (roundMetricTime(highest + 1) === v) highest++;
    return [v, lowest, highest];
  });
}
const BUCKETS = bucketTable();
// Bucket indexes used below.
const MS_1 = 1;
const MS_7 = 7;
const MS_100 = 100; // 100 to 104 ms
const MS_110 = 101; // 105 to 114 ms
const S_1 = 145; // 950 to 1,499 ms

function heatmap(fields) {
  return {
    generation: 1,
    rev: 1,
    since: 0,
    start_second: 0,
    end_second: 9,
    hold_from: 8,
    buckets: BUCKETS,
    columns: [],
    ...fields,
  };
}

function column(second, measured, synthetic = []) {
  return { second, measured, synthetic };
}

// Number formats as format.ts writes them.
const FORMATS = {
  int: (n) => Math.round(n).toLocaleString(),
  pct: (f) =>
    (f * 100).toLocaleString(undefined, {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    }) + "%",
};

test("the bucket table matches the server's", () => {
  assert.equal(BUCKETS.length, 205);
  assert.deepEqual(BUCKETS[0], [0, 0, 0]);
  assert.deepEqual(BUCKETS[MS_100], [100, 100, 104]);
  assert.deepEqual(BUCKETS[MS_110], [110, 105, 114]);
  assert.deepEqual(BUCKETS[140], [500, 495, 549]);
  assert.deepEqual(BUCKETS[S_1], [1000, 950, 1499]);
  assert.deepEqual(BUCKETS[204], [60000, 59500, 60499]);
});

// --- Applying updates -------------------------------------------------------

test("a whole window replaces, a delta applies only on the heatmap it follows", () => {
  const first = applyHeatmap(
    null,
    heatmap({ rev: 5, columns: [column(1, [MS_1, 3]), column(2, [MS_7, 1])] })
  );
  assert.deepEqual(heatmapCursor(first), { generation: 1, since: 5 });
  assert.equal(first.columns.size, 2);

  const next = applyHeatmap(
    first,
    heatmap({ rev: 9, since: 5, columns: [column(2, [MS_7, 4]), column(3, [MS_1, 1])] })
  );
  assert.deepEqual(Array.from(next.columns.keys()).sort(), [1, 2, 3]);
  assert.deepEqual(next.columns.get(2).measured, [MS_7, 4]);
  assert.equal(first.columns.get(2).measured[1], 1, "the old state is untouched");

  // Following on from another rev, or another generation: ask again.
  assert.equal(applyHeatmap(next, heatmap({ rev: 12, since: 5 })), null);
  assert.equal(applyHeatmap(next, heatmap({ generation: 2, rev: 12, since: 9 })), null);
  assert.equal(applyHeatmap(null, heatmap({ rev: 12, since: 9 })), null);

  // The window moves on: older columns go. A shorter table never replaces a
  // longer one.
  const moved = applyHeatmap(
    next,
    heatmap({ rev: 10, since: 9, start_second: 2, end_second: 11, buckets: BUCKETS.slice(0, 10) })
  );
  assert.deepEqual(Array.from(moved.columns.keys()).sort(), [2, 3]);
  assert.equal(moved.buckets.length, 205);

  // A whole window of a new generation replaces everything.
  const reset = applyHeatmap(moved, heatmap({ generation: 2, rev: 1, columns: [column(0, [MS_1, 1])] }));
  assert.deepEqual(Array.from(reset.columns.keys()), [0]);
});

// --- Rows on the log axis ---------------------------------------------------

test("rows span their buckets on a log axis, fastest at the bottom", () => {
  const state = applyHeatmap(
    null,
    heatmap({ columns: [column(1, [MS_1, 2, MS_100, 1, MS_110, 3])] })
  );
  const view = heatmapView(state, false);
  // Every bucket from the fastest with requests to the slowest is a row.
  assert.equal(view.rows.length, MS_110 - MS_1 + 1);
  assert.equal(view.rows[0].bucket, MS_1);
  assert.equal(view.rows[view.rows.length - 1].bucket, MS_110);
  // The axis runs from 1 ms to just past 114 ms.
  const span = Math.log(115) - Math.log(1);
  for (let i = 0; i < view.rows.length; i++) {
    const row = view.rows[i];
    const [from, to] = bucketSpan(BUCKETS[row.bucket]);
    // A row's height matches its range on the log axis.
    assert.ok(Math.abs(row.bottom - row.top - (Math.log(to) - Math.log(from)) / span) < 1e-9);
    if (i > 0) assert.ok(Math.abs(view.rows[i - 1].top - row.bottom) < 1e-9, "rows meet");
  }
  assert.ok(Math.abs(view.rows[0].bottom - 1) < 1e-9);
  assert.ok(Math.abs(view.rows[view.rows.length - 1].top) < 1e-9);
  // 1 ms is a millisecond tall; 105 to 114 ms is ten.
  const ms1 = view.rows[0];
  const ms110 = view.rows[view.rows.length - 1];
  assert.ok(Math.abs((ms1.bottom - ms1.top) - Math.log(2) / span) < 1e-9);
  assert.ok(Math.abs((ms110.bottom - ms110.top) - Math.log(115 / 105) / span) < 1e-9);
  // Ticks only within the range the data reaches.
  assert.deepEqual(
    view.ticks.map((t) => t.label),
    ["1 ms", "2 ms", "5 ms", "10 ms", "20 ms", "50 ms", "100 ms"]
  );
  assert.equal(view.ticks[0].y, 1);
});

test("the under 1 ms row spans half a millisecond to one; seconds tick in s", () => {
  assert.deepEqual(bucketSpan(BUCKETS[0]), [0.5, 1]);
  const state = applyHeatmap(null, heatmap({ columns: [column(1, [0, 1, S_1 + 30, 1])] }));
  const view = heatmapView(state, false);
  assert.equal(view.rows[0].bucket, 0);
  assert.deepEqual(
    view.ticks.map((t) => t.label),
    ["<1 ms", "1 ms", "2 ms", "5 ms", "10 ms", "20 ms", "50 ms", "100 ms", "200 ms", "500 ms", "1 s", "2 s", "5 s", "10 s", "20 s"]
  );
  // The <1 ms label sits in the middle of the bucket 0 row.
  assert.equal(view.ticks[0].y, (view.rows[0].top + view.rows[0].bottom) / 2);
  assert.equal(view.ticks[0].ms, 0);
});

test("held and empty seconds have no cells; adjusted adds synthetic times", () => {
  const state = applyHeatmap(
    null,
    heatmap({
      hold_from: 3,
      columns: [
        column(1, [MS_7, 2], [S_1, 3]),
        column(2, [MS_7, 1, S_1, 1], [S_1, 2]),
        column(3, [MS_7, 9]),
        column(4, [MS_7, 9]),
      ],
    })
  );
  const measured = heatmapView(state, false);
  assert.deepEqual(
    measured.cells.map((c) => [c.second, c.bucket, c.count, c.synthetic]),
    [
      [1, MS_7, 2, 0],
      [2, MS_7, 1, 0],
      [2, S_1, 1, 0],
    ]
  );
  assert.equal(measured.columns, 10);
  assert.equal(measured.secondTotals.get(2), 2);
  const adjusted = heatmapView(state, true);
  assert.deepEqual(
    adjusted.cells.map((c) => [c.second, c.bucket, c.count, c.synthetic]),
    [
      [1, MS_7, 2, 0],
      [1, S_1, 3, 3],
      [2, MS_7, 1, 0],
      [2, S_1, 3, 2],
    ]
  );
  assert.equal(adjusted.secondTotals.get(1), 5);
  assert.equal(adjusted.maxCount, 3);
  assert.equal(adjusted.minCount, 1);
  assert.equal(heatmapView(null, false).cells.length, 0);
});

// --- Colour -----------------------------------------------------------------

function hslToRgb(css) {
  const [h, s, l] = css.match(/[\d.]+/g).map(Number);
  const c = (1 - Math.abs((2 * l) / 100 - 1)) * (s / 100);
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l / 100 - c / 2;
  const [r, g, b] = h < 180 ? [0, c, x] : h < 240 ? [0, x, c] : [x, 0, c];
  return [r + m, g + m, b + m];
}

function luminance([r, g, b]) {
  const lin = (v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

function contrast(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

const CARD = { light: [1, 1, 1], dark: [0x1a / 255, 0x23 / 255, 0x32 / 255] };

test("one request and the busiest cell, in both themes", () => {
  assert.equal(countPosition(1, 1, 500), 0);
  assert.equal(countPosition(500, 1, 500), 1);
  assert.ok(Math.abs(countPosition(Math.sqrt(500), 1, 500) - 0.5) < 1e-9, "log scale");
  assert.equal(countPosition(1, 1, 1), 1);

  assert.equal(cellColor(1, 1, 500, false), "hsl(222, 80.0%, 64.0%)");
  assert.equal(cellColor(500, 1, 500, false), "hsl(222, 85.0%, 22.0%)");
  assert.equal(cellColor(1, 1, 500, true), "hsl(222, 75.0%, 56.0%)");
  assert.equal(cellColor(500, 1, 500, true), "hsl(222, 100.0%, 94.0%)");
  assert.equal(scaleColor(0, false), cellColor(1, 1, 500, false));

  // Light: fewer lighter, more darker. Dark: fewer darker, more brighter.
  const few = (dark) => luminance(hslToRgb(cellColor(1, 1, 500, dark)));
  const many = (dark) => luminance(hslToRgb(cellColor(500, 1, 500, dark)));
  assert.ok(few(false) > many(false));
  assert.ok(few(true) < many(true));
});

test("the ramp's floor stands out from the card, and its ends from each other", () => {
  for (const dark of [false, true]) {
    const card = dark ? CARD.dark : CARD.light;
    const floor = hslToRgb(scaleColor(0, dark));
    const top = hslToRgb(scaleColor(1, dark));
    assert.ok(contrast(floor, card) >= 3, "floor against the card, dark " + dark);
    assert.ok(contrast(floor, top) >= 3, "floor against the top, dark " + dark);
  }
});

test("the scale runs from the sparsest drawn cell to the busiest, as the legend says", () => {
  // Cells of 5, 50 and 500: the legend reads 5 at the left end and 500 at
  // the right, and the colours sit where it says.
  assert.equal(countPosition(5, 5, 500), 0);
  assert.equal(countPosition(500, 5, 500), 1);
  assert.ok(Math.abs(countPosition(50, 5, 500) - 0.5) < 1e-9);
  assert.equal(cellColor(5, 5, 500, false), scaleColor(0, false));
  assert.equal(cellColor(500, 5, 500, true), scaleColor(1, true));
  // Every cell the same: all at the highest.
  assert.equal(countPosition(7, 7, 7), 1);
});


// --- Tooltips ---------------------------------------------------------------

test("tooltips, in every form", () => {
  const state = applyHeatmap(
    null,
    heatmap({
      columns: [
        column(7, [0, 2, MS_7, 5, MS_110, 3, S_1, 2], [S_1, 1234]),
      ],
    })
  );
  const view = heatmapView(state, false);
  const rows = new Map(view.rows.map((r) => [r.bucket, r]));
  const cell = (bucket) => view.cells.find((c) => c.bucket === bucket);
  const text = (bucket, v = view, adjusted = false) =>
    tooltipText(v, v.cells.find((c) => c.bucket === bucket), rows.get(bucket), adjusted, FORMATS);
  // Every tooltip ends with its second's percentiles.
  const measured = " p50 / p95 / p99: 7 / 1,000 / 1,000 ms";
  assert.equal(text(MS_7), "Second 7: 7 ms, 5 requests (41.67% of the second's 12)" + measured);
  assert.equal(
    text(MS_110),
    "Second 7: 110 ms bucket (105 to 114 ms), 3 requests (25.00% of the second's 12)" + measured
  );
  assert.equal(
    text(0),
    "Second 7: under 1 ms, 2 requests (16.67% of the second's 12)" + measured
  );
  assert.ok(cell(S_1));

  const adjusted = heatmapView(state, true);
  const withSynthetic = " p50 / p95 / p99: 1,000 / 1,000 / 1,000 ms";
  assert.equal(
    text(S_1, adjusted, true),
    "Second 7: 1,000 ms bucket (950 to 1,499 ms), 1,236 requests, of which 1,234 synthetic (99.20% of the second's 1,246)" +
      withSynthetic
  );
  // No synthetic times in a cell: no synthetic clause.
  assert.equal(
    text(MS_7, adjusted, true),
    "Second 7: 7 ms, 5 requests (0.40% of the second's 1,246)" + withSynthetic
  );
});

// --- Percentiles by second --------------------------------------------------

test("each second's percentiles, by Goose's own rule", () => {
  const state = applyHeatmap(
    null,
    heatmap({
      hold_from: 9,
      columns: [
        // Most requests under a millisecond: p50 is bucket 0.
        column(1, [0, 5, MS_7, 1]),
        // Three requests: p50's rank is round(1.5) = 2, so the second.
        column(3, [MS_1, 1, MS_7, 1, MS_110, 1]),
        // One request: every percentile is its bucket.
        column(4, [MS_110, 1]),
        // Synthetic times in a slow bucket move the adjusted p99 only.
        column(5, [MS_7, 99], [S_1, 10]),
        // Held: not drawn, so no percentiles.
        column(9, [MS_7, 3]),
      ],
    })
  );
  const view = heatmapView(state, false);
  assert.deepEqual(view.percentiles.get(1), { second: 1, p50: 0, p95: 7, p99: 7 });
  assert.deepEqual(view.percentiles.get(3), { second: 3, p50: 7, p95: 110, p99: 110 });
  assert.deepEqual(view.percentiles.get(4), { second: 4, p50: 110, p95: 110, p99: 110 });
  assert.equal(view.percentiles.get(5).p99, 7, "measured");
  assert.equal(heatmapView(state, true).percentiles.get(5).p99, 1000, "adjusted");
  assert.equal(heatmapView(state, true).percentiles.get(5).p50, 7);
  // A second with no requests, and a held one, have no point.
  assert.equal(view.percentiles.has(2), false);
  assert.equal(view.percentiles.has(9), false);
  assert.deepEqual(Array.from(view.percentiles.keys()), [1, 3, 4, 5]);
});

// The histogram `response_time_percentile_matches_the_dashboard_heatmap` in
// src/metrics.rs runs through the Rust rule: the two must give one answer.
const CROSS_CHECK = [
  [0, 40], [1, 60], [2, 80], [3, 100], [5, 120], [8, 110], [12, 100], [18, 90],
  [25, 70], [40, 60], [60, 50], [90, 40], [100, 30], [110, 25], [150, 20],
  [200, 15], [300, 12], [400, 10], [500, 8], [700, 6], [1000, 5], [2000, 3],
  [5000, 2], [10000, 1],
];

test("per second percentiles match the Rust rule on a fixed histogram", () => {
  const total = CROSS_CHECK.reduce((sum, [, count]) => sum + count, 0);
  assert.equal(total, 1057);
  assert.equal(CROSS_CHECK.length, 24);
  assert.equal(percentileOf(CROSS_CHECK, total, 0.5), 12);
  assert.equal(percentileOf(CROSS_CHECK, total, 0.95), 200);
  assert.equal(percentileOf(CROSS_CHECK, total, 0.99), 700);
  // The same counts as a heatmap column give the same answers.
  const index = new Map(BUCKETS.map(([time], i) => [time, i]));
  const measured = CROSS_CHECK.flatMap(([time, count]) => [index.get(time), count]);
  const view = heatmapView(applyHeatmap(null, heatmap({ columns: [column(2, measured)] })), false);
  assert.deepEqual(view.percentiles.get(2), { second: 2, p50: 12, p95: 200, p99: 700 });
});

// --- Time ticks -------------------------------------------------------------

test("second labels fall on round seconds from the test start", () => {
  assert.equal(secondStep(100), 15);
  assert.equal(secondStep(300), 60);
  assert.equal(secondStep(8), 1);
  assert.equal(secondStep(9), 2);
  assert.deepEqual(secondLabels(37, 100), [45, 60, 75, 90, 105, 120, 135]);
  // As the window slides, the labels stay on the same seconds.
  assert.deepEqual(secondLabels(38, 100), [45, 60, 75, 90, 105, 120, 135]);
  assert.deepEqual(secondLabels(46, 100), [60, 75, 90, 105, 120, 135]);
  for (const second of secondLabels(1234, 300)) assert.equal(second % 60, 0);
  assert.ok(secondLabels(1234, 300).length <= 8);
  assert.deepEqual(secondLabels(0, 8), [0, 1, 2, 3, 4, 5, 6, 7]);
});

// --- Cost ---------------------------------------------------------------------

test("cost of a view with percentiles (informational)", () => {
  const columns = [];
  for (let second = 0; second < 300; second++) {
    const measured = [];
    for (let bucket = 0; bucket < 200; bucket++) measured.push(bucket, 1 + ((second * 7 + bucket) % 13));
    columns.push(column(second, measured));
  }
  const state = applyHeatmap(
    null,
    heatmap({ start_second: 0, end_second: 299, hold_from: 300, columns })
  );
  const times = [];
  for (let run = 0; run < 50; run++) {
    const started = process.hrtime.bigint();
    const view = heatmapView(state, false);
    times.push(Number(process.hrtime.bigint() - started) / 1e6);
    assert.equal(view.percentiles.size, 300);
  }
  times.sort((a, b) => a - b);
  console.log("heatmapView with percentiles, 300 columns of 200 buckets: median " + times[25].toFixed(2) + " ms of 50 runs");
});

// --- The card ---------------------------------------------------------------

function requestRow(name, has_series, request_count = 10) {
  return {
    method: "GET",
    name,
    request_count,
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

function snapshotWith(requests, fields = {}, flags = {}) {
  return {
    ...IDLE_SNAPSHOT,
    phase: "maintain",
    requests,
    ...fields,
    flags: { ...IDLE_SNAPSHOT.flags, ...flags },
  };
}

function withHeatmap(snapshot, h) {
  return { ...snapshot, series: { ...snapshot.series, heatmap: h } };
}

function coActive(snapshot) {
  return { ...snapshot, aggregate: { ...snapshot.aggregate, co_active: true } };
}

function seriesRequests(page) {
  return page.requests.filter((r) => r.path === "/api/v1/series");
}

function snapshotRequests(page) {
  return page.requests.filter((r) => r.path === "/api/v1/snapshot");
}

const wait = (window, ms) => new Promise((resolve) => window.setTimeout(resolve, ms));

function el(page, id) {
  return page.window.document.getElementById(id);
}

function optionTexts(page) {
  return Array.from(el(page, "heatmap-request").options).map((o) => o.textContent);
}

test("the card shows the heatmap, its notes and a select of requests with heatmaps", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: snapshotWith([
      requestRow("/few", true, 5),
      requestRow("/late", false, 50),
      requestRow("/many", true, 100),
    ]),
  });
  assert.equal(el(page, "chart-latency-title").textContent, TITLE);
  assert.equal(TITLE, "Response times by second");
  assert.equal(el(page, "heatmap-note").hidden, false);
  assert.equal(el(page, "heatmap-note").textContent, NOTE);
  assert.equal(
    NOTE,
    "Each column is one second. Color shows how many requests took that long; the lines are that second's p50, p95 and p99."
  );

  assert.equal(el(page, "chart-latency-note").hidden, true);
  assert.equal(el(page, "chart-heatmap").getAttribute("aria-label"), ARIA_LABEL);
  assert.equal(ARIA_LABEL, "Response times by second, heatmap");
  assert.equal(el(page, "chart-latency-wrap").hidden, true);
  assert.equal(el(page, "heatmap-wrap").hidden, false);
  // All requests first, then the requests with heatmaps by method then
  // name, whatever the table's order (most requests first by default).
  assert.deepEqual(optionTexts(page), ["All requests", "GET /few", "GET /many"]);
  assert.equal(el(page, "heatmap-request").value, "");
  // Names in the Requests table are plain text again.
  assert.equal(page.window.document.querySelector("#requests-body button"), null);
  assert.equal(seriesRequests(page).length, 0, "no fetches while nothing is chosen");

});

test("re-sorting the Requests table leaves the select's options untouched", async () => {
  const requests = [
    requestRow("/b", true, 5),
    requestRow("/c", true, 50),
    requestRow("/a", true, 100),
  ];
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: snapshotWith(requests),
    eventStream: true,
  });
  page.events.emit("snapshot", JSON.stringify(snapshotWith(requests)));
  await settle(page.window);
  const before = Array.from(el(page, "heatmap-request").options);
  assert.deepEqual(optionTexts(page), ["All requests", "GET /a", "GET /b", "GET /c"]);
  for (const column of ["name", "request_count", "name"]) {
    page.window.document.querySelector('#requests-table th[data-sort="' + column + '"]').click();
    // A snapshot after the sort, with the rows in another order.
    page.events.emit("snapshot", JSON.stringify(snapshotWith(requests.slice().reverse())));
    await settle(page.window);
    const after = Array.from(el(page, "heatmap-request").options);
    assert.equal(after.length, before.length);
    after.forEach((option, i) => assert.equal(option, before[i], "the same element"));
  }
});

test("the select is never rebuilt while it has focus; a change waits for blur", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: snapshotWith([requestRow("/a", true)]),
    eventStream: true,
  });
  page.events.emit("snapshot", JSON.stringify(snapshotWith([requestRow("/a", true)])));
  await settle(page.window);
  const select = el(page, "heatmap-request");
  const before = Array.from(select.options);
  select.focus();
  page.events.emit(
    "snapshot",
    JSON.stringify(snapshotWith([requestRow("/a", true), requestRow("/b", true)]))
  );
  await settle(page.window);
  assert.deepEqual(Array.from(select.options), before, "untouched while focused");
  select.blur();
  assert.deepEqual(optionTexts(page), ["All requests", "GET /a", "GET /b"]);
});


test("the Adjusted and Measured toggle shows only once mitigation has an event", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: snapshotWith([requestRow("/a", true)]),
    eventStream: true,
  });
  page.events.emit("snapshot", JSON.stringify(snapshotWith([requestRow("/a", true)])));
  await settle(page.window);
  assert.equal(el(page, "heatmap-view").hidden, true);
  assert.equal(el(page, "chart-latency-note").hidden, true);

  page.events.emit("snapshot", JSON.stringify(coActive(snapshotWith([requestRow("/a", true)]))));
  await settle(page.window);
  assert.equal(el(page, "heatmap-view").hidden, false);
  assert.equal(el(page, "heatmap-adjusted").getAttribute("aria-pressed"), "true");
  assert.equal(el(page, "heatmap-measured").getAttribute("aria-pressed"), "false");
  assert.equal(el(page, "heatmap-adjusted").textContent, "Adjusted");
  assert.equal(el(page, "heatmap-measured").textContent, "Measured");
  assert.equal(el(page, "chart-latency-note").hidden, false);
  assert.equal(el(page, "chart-latency-note").textContent, ADJUSTED_NOTE);
  assert.equal(
    ADJUSTED_NOTE,
    "Adjusted: includes the synthetic times coordinated omission mitigation adds."
  );

  el(page, "heatmap-measured").click();
  assert.equal(el(page, "heatmap-measured").getAttribute("aria-pressed"), "true");
  assert.equal(el(page, "heatmap-adjusted").getAttribute("aria-pressed"), "false");
  assert.equal(el(page, "chart-latency-note").hidden, true);
  el(page, "heatmap-adjusted").click();
  assert.equal(el(page, "chart-latency-note").hidden, false);
});

function choose(page, value) {
  const select = el(page, "heatmap-request");
  select.value = value;
  select.dispatchEvent(new page.window.Event("change"));
}

test("choosing a request fetches its heatmap now and every second, with its cursor", async () => {
  const served = (url) => ({
    method: url.searchParams.get("method"),
    name: url.searchParams.get("name"),
    heatmap: heatmap({ generation: 4, rev: 21, since: url.searchParams.get("heatmap_since") ? 20 : 0 }),
  });
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: snapshotWith([requestRow("/a b", true)]),
    series: served,
  });
  choose(page, "GET\n/a b");
  await settle(page.window);
  let fetched = seriesRequests(page);
  assert.equal(fetched.length, 1);
  let url = new URL(fetched[0].url);
  assert.equal(url.searchParams.get("method"), "GET");
  assert.equal(url.searchParams.get("name"), "/a b");
  assert.equal(url.searchParams.get("token"), TOKEN);
  assert.equal(url.searchParams.get("heatmap_since"), null, "the first fetch asks for everything");
  assert.equal(el(page, "chart-latency-title").textContent, "Response times by second: GET /a b");

  await wait(page.window, 1100);
  fetched = seriesRequests(page);
  assert.equal(fetched.length, 2);
  url = new URL(fetched[1].url);
  assert.equal(url.searchParams.get("heatmap_generation"), "4");
  assert.equal(url.searchParams.get("heatmap_since"), "21");

  choose(page, "");
  await settle(page.window);
  assert.equal(el(page, "chart-latency-title").textContent, TITLE);
  const after = seriesRequests(page).length;
  await wait(page.window, 1300);
  assert.equal(seriesRequests(page).length, after, "polling stopped");
});

test("a slow answer is asked for once, and the next fetch carries its cursor", async () => {
  const served = (url) => ({
    method: url.searchParams.get("method"),
    name: url.searchParams.get("name"),
    heatmap: heatmap({ generation: 4, rev: 21, since: url.searchParams.get("heatmap_since") ? 20 : 0 }),
  });
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: snapshotWith([requestRow("/slow", true)]),
    series: served,
    seriesDelayMs: 2500,
  });
  choose(page, "GET\n/slow");
  // Ticks at 1 s and 2 s find the first fetch still out.
  await wait(page.window, 2400);
  assert.equal(seriesRequests(page).length, 1, "one request, not three");
  // It answers at 2.5 s; the 3 s tick asks for what follows on from it.
  await wait(page.window, 900);
  const fetched = seriesRequests(page);
  assert.equal(fetched.length, 2);
  const url = new URL(fetched[1].url);
  assert.equal(url.searchParams.get("heatmap_generation"), "4");
  assert.equal(url.searchParams.get("heatmap_since"), "21");
  choose(page, "");
});

test("a 404 keeps the request chosen and shows an empty heatmap", async () => {

  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: snapshotWith([requestRow("/gone", true)]),
    seriesStatus: 404,
  });
  choose(page, "GET\n/gone");
  await settle(page.window);
  assert.equal(seriesRequests(page).length, 1);
  assert.equal(el(page, "heatmap-request").value, "GET\n/gone");
  assert.equal(el(page, "chart-latency-title").textContent, "Response times by second: GET /gone");
  assert.equal(el(page, "heatmap-legend-max").textContent, "");
  choose(page, "");
});

test("the choice returns to All requests when the request cannot be shown", async () => {
  for (const next of [
    snapshotWith([requestRow("/a", false)]),
    snapshotWith([requestRow("/b", true)]),
    snapshotWith([requestRow("/a", true)], {}, { percentiles_over_time: false }),
  ]) {
    const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
      snapshot: snapshotWith([requestRow("/a", true)]),
      eventStream: true,
    });
    page.events.emit("snapshot", JSON.stringify(snapshotWith([requestRow("/a", true)])));
    await settle(page.window);
    choose(page, "GET\n/a");
    await settle(page.window);
    assert.equal(el(page, "heatmap-request").value, "GET\n/a");
    page.events.emit("snapshot", JSON.stringify(next));
    await settle(page.window);
    assert.equal(el(page, "heatmap-request").value, "");
    const fetched = seriesRequests(page).length;
    await wait(page.window, 1200);
    assert.equal(seriesRequests(page).length, fetched, "polling stopped");
    page.window.close();
  }
});

test("a delta that does not follow on asks for the columns the page lacks", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: (url) =>
      withHeatmap(snapshotWith([]), heatmap({ generation: 3, rev: 8, since: Number(url.searchParams.get("heatmap_since") || 0) })),
    eventStream: true,
  });
  page.events.emit(
    "snapshot",
    JSON.stringify(withHeatmap(snapshotWith([]), heatmap({ generation: 3, rev: 5, columns: [column(1, [MS_7, 2])] })))
  );
  await settle(page.window);
  assert.equal(el(page, "heatmap-legend-max").textContent, "2");
  const before = snapshotRequests(page).length;
  // Follows on from rev 4, which this page never had.
  page.events.emit(
    "snapshot",
    JSON.stringify(withHeatmap(snapshotWith([]), heatmap({ generation: 3, rev: 8, since: 4 })))
  );
  await settle(page.window);
  const fetched = snapshotRequests(page);
  assert.equal(fetched.length, before + 1);
  const url = new URL(fetched[fetched.length - 1].url);
  assert.equal(url.searchParams.get("heatmap_generation"), "3");
  assert.equal(url.searchParams.get("heatmap_since"), "5");
});

test("a catch-up answering after Goose exited renders nothing", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: snapshotWith([]),
    eventStream: true,
  });
  page.events.emit(
    "snapshot",
    JSON.stringify(withHeatmap(snapshotWith([]), heatmap({ generation: 3, rev: 5, columns: [column(1, [MS_7, 2])] })))
  );
  await settle(page.window);
  const held = [];
  const realFetch = page.window.fetch;
  page.window.fetch = (url, init) => {
    if (!String(url).includes("/api/v1/snapshot")) return realFetch(url, init);
    return new Promise((resolve) => held.push(resolve));
  };
  page.events.emit(
    "snapshot",
    JSON.stringify(withHeatmap(snapshotWith([]), heatmap({ generation: 3, rev: 8, since: 4 })))
  );
  await settle(page.window);
  assert.equal(held.length, 1, "a catch-up is out");
  page.events.emit("closed", "");
  await settle(page.window);
  assert.equal(el(page, "connection-label").textContent, "finished");
  held[0]({ status: 200, ok: true, json: () => Promise.resolve(snapshotWith([])) });
  await settle(page.window);
  assert.equal(el(page, "connection-label").textContent, "finished", "the late catch-up is dropped");
});

test("a catch-up after a polling page's retry came back keeps the pill on SSE", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: snapshotWith([]),
    eventStream: true,
  });
  page.events.emit(
    "snapshot",
    JSON.stringify(withHeatmap(snapshotWith([]), heatmap({ generation: 3, rev: 5, columns: [column(1, [MS_7, 2])] })))
  );
  await settle(page.window);
  // Every snapshot poll answers 503 until the retry is live again.
  let pollStatus = 503;
  const realFetch = page.window.fetch;
  page.window.fetch = (url, init) => {
    if (!String(url).includes("/api/v1/snapshot") || pollStatus === 200) return realFetch(url, init);
    return Promise.resolve({ status: 503, ok: false, json: () => Promise.resolve(null) });
  };
  page.events.source.readyState = 2;
  page.events.source.onerror();
  await settle(page.window);
  assert.equal(page.events.sources.length, 2, "the 503 poll opened a retry");
  pollStatus = 200;
  page.events.emit(
    "snapshot",
    JSON.stringify(withHeatmap(snapshotWith([]), heatmap({ generation: 3, rev: 5, columns: [column(1, [MS_7, 2])] })))
  );
  await settle(page.window);
  assert.equal(el(page, "connection-label").textContent, "SSE");
  const before = snapshotRequests(page).length;
  // Follows on from rev 4, which this page never had: a catch-up goes.
  page.events.emit(
    "snapshot",
    JSON.stringify(withHeatmap(snapshotWith([]), heatmap({ generation: 3, rev: 8, since: 4 })))
  );
  await settle(page.window);
  assert.equal(snapshotRequests(page).length, before + 1, "a catch-up is out");
  assert.equal(el(page, "connection-label").textContent, "SSE", "the catch-up is a live fetch, not a poll");
});

test("with percentiles off the card is the average latency chart, with no heatmap", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: snapshotWith([requestRow("/a", true)], {}, { percentiles_over_time: false }),
  });
  assert.equal(el(page, "chart-latency-title").textContent, "Average latency (ms)");
  assert.equal(el(page, "chart-latency-wrap").hidden, false);
  assert.equal(el(page, "chart-latency").getAttribute("aria-label"), "Average latency chart");
  assert.equal(el(page, "chart-latency-note").hidden, false);
  assert.equal(el(page, "chart-latency-note").textContent, OFF_NOTE);
  assert.equal(OFF_NOTE, "Percentiles are off for this run (--no-percentiles-over-time).");
  for (const id of ["heatmap-wrap", "heatmap-legend", "heatmap-request", "heatmap-view", "heatmap-note"]) {
    assert.equal(el(page, id).hidden, true, id);
  }
  // Nothing was drawn: drawing sets the legend's colour scale.
  assert.equal(el(page, "heatmap-legend-bar").style.background, "");
  assert.equal(seriesRequests(page).length, 0);
  // Polls carry no heatmap cursor.
  for (const request of snapshotRequests(page)) {
    assert.equal(new URL(request.url).searchParams.get("heatmap_since"), null);
  }

  // With metrics off too, no off note.
  const off = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: snapshotWith([], {}, { percentiles_over_time: false, metrics_disabled: true }),
  });
  assert.equal(el(off, "chart-latency-note").hidden, true);
});

// --- Whole run labels ---------------------------------------------------------

test("the KPIs, the Whole run card and the tables say they cover the whole run", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    snapshot: snapshotWith([requestRow("/a", true)]),
  });
  const { document } = page.window;
  assert.equal(el(page, "kpi-p95-label").textContent, "p95 ms (whole run)");
  assert.equal(el(page, "kpi-avg-label").textContent, "Avg ms (whole run)");
  const aggregate = document.getElementById("aggregate");
  assert.equal(aggregate.querySelector("h2").textContent, "Whole run");
  assert.equal(
    aggregate.querySelector("p.note").textContent,
    "Since the test started, or since metrics were last reset. Request times under 10 ms are shown to the hundredth of a millisecond."
  );
  const percentiles = [
    ["p50", "Whole run"],
    ["p95", "Whole run"],
    ["p99", "Whole run"],
  ];
  const minMax = [
    ["avg ms", "Whole run"],
    ["min ms", "Whole run"],
    ...percentiles,
    ["max ms", "Whole run"],
  ];
  const expected = {
    "scenarios-table": [["avg ms", "Whole run"], ...percentiles],
    "transactions-table": minMax,
    "requests-table": [...minMax, ["status codes", "Whole run"]],
  };
  for (const [table, titles] of Object.entries(expected)) {
    const titled = Array.from(
      document.querySelectorAll("#" + table + " thead th[title]")
    ).map((th) => [th.textContent, th.title]);
    assert.deepEqual(titled, titles, table);
  }
});
