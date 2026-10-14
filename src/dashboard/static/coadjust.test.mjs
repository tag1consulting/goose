// Coordinated omission adjusted response times (#716): which value the
// dashboard shows, its hover text, and the KPI label and requests table note
// that say the values are adjusted. Run with `npm test`; `pretest` builds
// src/coadjust.ts into test-build/, and the page tests need `npm run build`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { shownTime } from "./test-build/coadjust.mjs";
import { IDLE_SNAPSHOT } from "./idle-snapshot.mjs";
import { FRESH, openPage, openPages } from "./test-harness.mjs";

const TOKEN = "s3cret";
const fixed = (n) => n.toFixed(2);

test.afterEach(() => {
  while (openPages.length) openPages.pop().window.close();
});

test("the adjusted value is shown when present, with the measured one as its title", () => {
  assert.deepEqual(shownTime(12, 480, String), {
    value: 480,
    title: "Measured: 12 ms",
  });
  assert.deepEqual(shownTime(10.5, 463.64, fixed), {
    value: 463.64,
    title: "Measured: 10.50 ms",
  });
});

test("the measured value is shown, with no title, when there is no adjusted one", () => {
  assert.deepEqual(shownTime(12, null, String), { value: 12, title: null });
  assert.deepEqual(shownTime(12, undefined, String), { value: 12, title: null });
});

test("an adjusted value of 0 is still adjusted", () => {
  assert.deepEqual(shownTime(12, 0, String), {
    value: 0,
    title: "Measured: 12 ms",
  });
});

const CO_ADJUSTED = {
  response_time_avg_ms: 325,
  response_time_max_ms: 6000,
  percentile_ms: { p50: 10, p95: 20, p99: 5000 },
};

function requestRow(name, co_adjusted) {
  return {
    method: "GET",
    name,
    request_count: 20,
    failure_count: 0,
    requests_per_second: 2,
    failures_per_second: 0,
    response_time_avg_ms: 10,
    response_time_min_ms: 10,
    response_time_max_ms: 12,
    percentile_ms: { p50: 10, p95: 10, p99: 10 },
    status_codes: [],
    co_adjusted,
  };
}

function snapshot(coAdjusted) {
  return {
    ...IDLE_SNAPSHOT,
    phase: "maintain",
    aggregate: {
      ...IDLE_SNAPSHOT.aggregate,
      failures_per_second: 0.5,
      response_time_avg_ms: 13.33,
      response_time_min_ms: 2,
      response_time_max_ms: 30,
      percentile_ms: { p50: 10, p95: 20, p99: 20 },
      co_active: coAdjusted !== null,
      co_adjusted: coAdjusted,
    },
    requests: [
      requestRow("/a", coAdjusted ? { ...CO_ADJUSTED, response_time_avg_ms: 463.64 } : null),
      requestRow("/b", null),
    ],
  };
}

async function load(snap) {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { snapshot: snap });
  return page.window.document;
}

function requestCells(doc, name) {
  const tr = Array.from(doc.getElementById("requests-body").rows).find(
    (row) => row.cells[1].textContent === name
  );
  assert.ok(tr, "row " + name);
  return Array.from(tr.cells);
}

test("a snapshot with adjusted times labels them and shows the table note", async () => {
  const doc = await load(snapshot(CO_ADJUSTED));
  assert.equal(
    doc.getElementById("kpi-p95-label").textContent,
    "p95 ms (whole run, adjusted)"
  );
  assert.equal(
    doc.getElementById("kpi-avg-label").textContent,
    "Avg ms (whole run, adjusted)"
  );
  const p95 = doc.getElementById("kpi-p95");
  assert.equal(p95.textContent, (20).toLocaleString());
  assert.equal(p95.title, "Measured: 20 ms");
  assert.equal(doc.getElementById("requests-co-note").hidden, false);

  const labels = Array.from(
    doc.querySelectorAll("#aggregate-body .kv .k")
  ).map((k) => k.textContent);
  for (const label of [
    "Avg ms (adjusted)",
    "p50 / p95 / p99 (adjusted)",
    "Avg ms (measured)",
    "p50 / p95 / p99 (measured)",
    "Coordinated omission",
  ]) {
    assert.ok(labels.includes(label), label);
  }

  // "/a" shows its adjusted p99 with the measured one on hover; "/b" has no
  // adjusted times and shows its measured ones with no hover.
  const a = requestCells(doc, "/a");
  assert.equal(a[10].textContent, (5000).toLocaleString());
  assert.equal(a[10].title, "Measured: 10 ms");
  const b = requestCells(doc, "/b");
  assert.equal(b[10].textContent, "10");
  assert.equal(b[10].title, "");
});

test("a snapshot without adjusted times shows neither the label nor the note", async () => {
  const doc = await load(snapshot(null));
  assert.equal(doc.getElementById("kpi-p95-label").textContent, "p95 ms (whole run)");
  assert.equal(doc.getElementById("kpi-avg-label").textContent, "Avg ms (whole run)");
  assert.equal(doc.getElementById("kpi-p95").title, "");
  assert.equal(doc.getElementById("requests-co-note").hidden, true);
  const labels = Array.from(
    doc.querySelectorAll("#aggregate-body .kv .k")
  ).map((k) => k.textContent);
  assert.ok(labels.includes("Avg ms"));
  assert.ok(!labels.some((l) => l.includes("adjusted")));
  assert.equal(requestCells(doc, "/a")[10].title, "");
});

/** The value of the aggregate panel line with this label, or undefined. */
function aggregateValue(doc, label) {
  const line = Array.from(doc.querySelectorAll("#aggregate-body .kv")).find(
    (kv) => kv.querySelector(".k").textContent === label
  );
  return line ? line.querySelector(".v").textContent : undefined;
}

test("the max column shows the adjusted max, the measured one on hover", async () => {
  // The adjusted max goes with the adjusted percentiles, or a row's max could
  // sit below its own p99; min has no adjusted value and stays measured.
  const doc = await load(snapshot(CO_ADJUSTED));
  const a = requestCells(doc, "/a");
  assert.equal(a[11].textContent, (6000).toLocaleString());
  assert.equal(a[11].title, "Measured: 12 ms");
  assert.equal(a[7].textContent, "10");
  assert.equal(a[7].title, "");
  const b = requestCells(doc, "/b");
  assert.equal(b[11].textContent, "12");
  assert.equal(b[11].title, "");

  const measured = await load(snapshot(null));
  assert.equal(requestCells(measured, "/a")[11].textContent, "12");
  assert.equal(requestCells(measured, "/a")[11].title, "");
});

test("the aggregate panel shows failures/s and min / max, adjusted when there are adjusted times", async () => {
  const doc = await load(snapshot(CO_ADJUSTED));
  const rate = (0.5).toLocaleString(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
  assert.equal(aggregateValue(doc, "Failures/s"), rate);
  const labels = Array.from(doc.querySelectorAll("#aggregate-body .kv .k")).map(
    (k) => k.textContent
  );
  assert.deepEqual(labels.slice(2, 5), ["RPS", "Failures/s", "Fail %"]);
  assert.equal(
    aggregateValue(doc, "Min / max ms (adjusted)"),
    "2 / " + (6000).toLocaleString()
  );
  assert.equal(aggregateValue(doc, "Min / max ms (measured)"), "2 / 30");
  assert.equal(aggregateValue(doc, "Min / max ms"), undefined);

  const measured = await load(snapshot(null));
  assert.equal(aggregateValue(measured, "Failures/s"), rate);
  assert.equal(aggregateValue(measured, "Min / max ms"), "2 / 30");
  assert.equal(aggregateValue(measured, "Min / max ms (adjusted)"), undefined);
});
