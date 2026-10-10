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
  response_time_max_ms: 5000,
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
    response_time_max_ms: 10,
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
      response_time_avg_ms: 13.33,
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
  assert.equal(doc.getElementById("kpi-p95-label").textContent, "p95 ms (adjusted)");
  assert.equal(doc.getElementById("kpi-avg-label").textContent, "Avg ms (adjusted)");
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
  assert.equal(a[8].textContent, (5000).toLocaleString());
  assert.equal(a[8].title, "Measured: 10 ms");
  const b = requestCells(doc, "/b");
  assert.equal(b[8].textContent, "10");
  assert.equal(b[8].title, "");
});

test("a snapshot without adjusted times shows neither the label nor the note", async () => {
  const doc = await load(snapshot(null));
  assert.equal(doc.getElementById("kpi-p95-label").textContent, "p95 ms");
  assert.equal(doc.getElementById("kpi-avg-label").textContent, "Avg ms");
  assert.equal(doc.getElementById("kpi-p95").title, "");
  assert.equal(doc.getElementById("requests-co-note").hidden, true);
  const labels = Array.from(
    doc.querySelectorAll("#aggregate-body .kv .k")
  ).map((k) => k.textContent);
  assert.ok(labels.includes("Avg ms"));
  assert.ok(!labels.some((l) => l.includes("adjusted")));
  assert.equal(requestCells(doc, "/a")[8].title, "");
});
