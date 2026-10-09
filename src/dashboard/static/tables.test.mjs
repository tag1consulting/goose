// The dashboard client's tables (app.js), run in jsdom with `npm test` after
// `npm run build`. Each test loads the page against a stub server that serves
// one snapshot, then reads the rendered rows back from the DOM.
//
// Expected numbers are built with the same toLocaleString call the client
// uses, so the tests pass under any locale.

import { test } from "node:test";
import assert from "node:assert/strict";
import { IDLE_SNAPSHOT } from "./idle-snapshot.mjs";
import { FRESH, openPage, openPages } from "./test-harness.mjs";

const TOKEN = "s3cret";

const int = (n) => Math.round(n).toLocaleString();
const rate = (n) =>
  n.toLocaleString(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });

/** IDLE_SNAPSHOT with `fields` replaced and `flags` merged into its flags. */
function snapshotWith(fields, flags = {}) {
  return {
    ...IDLE_SNAPSHOT,
    phase: "maintain",
    ...fields,
    flags: { ...IDLE_SNAPSHOT.flags, ...flags },
  };
}

function requestRow(method, name, request_count, failure_count, extra = {}) {
  return {
    method,
    name,
    request_count,
    failure_count,
    requests_per_second: request_count / 10,
    failures_per_second: failure_count / 10,
    response_time_avg_ms: 12.5,
    response_time_min_ms: 1,
    response_time_max_ms: 40,
    percentile_ms: { p50: 10, p95: 30, p99: 40 },
    status_codes: [],
    ...extra,
  };
}

function errorRow(name, occurrences) {
  return { method: "GET", name, error: "500 Internal Server Error", occurrences };
}

async function load(snapshot) {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { snapshot });
  return page.window;
}

/** The text of every cell of every row in the tbody with this id. */
function rows(window, bodyId) {
  const body = window.document.getElementById(bodyId);
  return Array.from(body.rows).map((tr) =>
    Array.from(tr.cells).map((td) => td.textContent)
  );
}

/** Column `index` of every row in the tbody with this id. */
function column(window, bodyId, index) {
  return rows(window, bodyId).map((cells) => cells[index]);
}

/** The single empty-state text of a tbody, or fails if it holds data rows. */
function emptyText(window, bodyId) {
  const body = window.document.getElementById(bodyId);
  assert.equal(body.rows.length, 1, "one empty-state row in #" + bodyId);
  const cell = body.rows[0].cells[0];
  assert.ok(cell.classList.contains("empty"), "the row is an empty state");
  return cell.textContent;
}

function header(window, tableId, sortKey) {
  const th = window.document.querySelector(
    "#" + tableId + ' thead th[data-sort="' + sortKey + '"]'
  );
  assert.ok(th, "header " + sortKey + " in #" + tableId);
  return th;
}

function sortedHeaders(window, tableId) {
  return Array.from(
    window.document.querySelectorAll("#" + tableId + " thead th.sorted")
  ).map((th) => [th.getAttribute("data-sort"), th.classList.contains("asc") ? "asc" : "desc"]);
}

function typeFilter(window, id, value) {
  const input = window.document.getElementById(id);
  input.value = value;
  input.dispatchEvent(new window.Event("input"));
}

test.afterEach(() => {
  while (openPages.length) openPages.pop().window.close();
});

// Requests and errors: pinned before the tables moved onto the shared
// sortable table, and unchanged since.

const REQUESTS = [
  requestRow("GET", "/small", 3, 0),
  requestRow("POST", "/login", 12345, 7, {
    requests_per_second: 1234.5,
  }),
  requestRow("GET", "/middle", 50, 2),
];

test("requests default to #reqs descending", async () => {
  const window = await load(snapshotWith({ requests: REQUESTS }));
  assert.deepEqual(column(window, "requests-body", 1), [
    "/login",
    "/middle",
    "/small",
  ]);
  assert.deepEqual(sortedHeaders(window, "requests-table"), [
    ["request_count", "desc"],
  ]);
});

test("request cells are formatted with formatInt and formatRate", async () => {
  const window = await load(snapshotWith({ requests: REQUESTS }));
  const login = rows(window, "requests-body")[0];
  assert.equal(login[0], "POST");
  assert.equal(login[2], int(12345));
  assert.equal(login[4], rate(1234.5));
});

test("no requests shows No requests yet, even with a filter", async () => {
  const window = await load(snapshotWith({ requests: [] }));
  assert.equal(emptyText(window, "requests-body"), "No requests yet");
  typeFilter(window, "requests-filter", "nothing");
  assert.equal(emptyText(window, "requests-body"), "No requests yet");
});

test("requests under --no-metrics say metrics are disabled", async () => {
  const window = await load(
    snapshotWith({ requests: [] }, { metrics_disabled: true })
  );
  assert.equal(
    emptyText(window, "requests-body"),
    "Metrics disabled (--no-metrics). Charts and request tables are empty."
  );
});

test("no errors shows No errors, with or without --no-metrics", async () => {
  for (const metrics_disabled of [false, true]) {
    const window = await load(snapshotWith({ errors: [] }, { metrics_disabled }));
    assert.equal(emptyText(window, "errors-body"), "No errors");
  }
});

test("the requests filter matches method and name", async () => {
  const window = await load(snapshotWith({ requests: REQUESTS }));
  typeFilter(window, "requests-filter", "post");
  assert.deepEqual(column(window, "requests-body", 1), ["/login"]);
  typeFilter(window, "requests-filter", "MIDD");
  assert.deepEqual(column(window, "requests-body", 1), ["/middle"]);
  typeFilter(window, "requests-filter", "zzz");
  assert.equal(emptyText(window, "requests-body"), "No matching requests");
  typeFilter(window, "requests-filter", "");
  assert.equal(rows(window, "requests-body").length, 3);
});

test("a requests header click reverses the order and moves the class", async () => {
  const window = await load(snapshotWith({ requests: REQUESTS }));
  header(window, "requests-table", "request_count").click();
  assert.deepEqual(column(window, "requests-body", 2), [
    int(3),
    int(50),
    int(12345),
  ]);
  assert.deepEqual(sortedHeaders(window, "requests-table"), [
    ["request_count", "asc"],
  ]);

  header(window, "requests-table", "name").click();
  assert.deepEqual(column(window, "requests-body", 1), [
    "/login",
    "/middle",
    "/small",
  ]);
  assert.deepEqual(sortedHeaders(window, "requests-table"), [["name", "asc"]]);
});

test("a first click on a number column sorts descending", async () => {
  const window = await load(snapshotWith({ requests: REQUESTS }));
  header(window, "requests-table", "failure_count").click();
  assert.deepEqual(column(window, "requests-body", 3), [int(7), int(2), int(0)]);
  assert.deepEqual(sortedHeaders(window, "requests-table"), [
    ["failure_count", "desc"],
  ]);
});

test("errors default to Occurrences descending and sort on click", async () => {
  const window = await load(
    snapshotWith({
      errors: [errorRow("/a", 2), errorRow("/b", 1500), errorRow("/c", 40)],
    })
  );
  assert.deepEqual(column(window, "errors-body", 1), ["/b", "/c", "/a"]);
  assert.deepEqual(column(window, "errors-body", 3), [
    int(1500),
    int(40),
    int(2),
  ]);
  assert.deepEqual(sortedHeaders(window, "errors-table"), [
    ["occurrences", "desc"],
  ]);
  header(window, "errors-table", "occurrences").click();
  assert.deepEqual(column(window, "errors-body", 1), ["/a", "/c", "/b"]);
  assert.deepEqual(sortedHeaders(window, "errors-table"), [
    ["occurrences", "asc"],
  ]);
});

// Scenarios and transactions.

function scenarioRow(scenario_index, scenario_name, users, run_count, extra = {}) {
  return {
    scenario_index,
    scenario_name,
    users,
    run_count,
    runs_per_second: run_count / 10,
    response_time_avg_ms: run_count ? 250.5 : 0,
    response_time_min_ms: run_count ? 100 : 0,
    response_time_max_ms: run_count ? 400 : 0,
    percentile_ms: run_count
      ? { p50: 200, p95: 1400, p99: 1500 }
      : { p50: 0, p95: 0, p99: 0 },
    ...extra,
  };
}

function transactionRow(
  scenario_index,
  scenario_name,
  transaction_index,
  transaction_name,
  run_count,
  failure_count = 0
) {
  return {
    scenario_index,
    scenario_name,
    transaction_index,
    transaction_name,
    run_count,
    failure_count,
    runs_per_second: run_count / 10,
    failures_per_second: failure_count / 10,
    response_time_avg_ms: run_count ? 12.25 : 0,
    response_time_min_ms: run_count ? 5 : 0,
    response_time_max_ms: run_count ? 30 : 0,
    percentile_ms: run_count
      ? { p50: 10, p95: 25, p99: 30 }
      : { p50: 0, p95: 0, p99: 0 },
  };
}

// Eleven transactions in the first scenario, so `1.10` and `1.11` must sort
// after `1.9`, and two in the second. Listed out of order on purpose.
const TRANSACTIONS = [
  transactionRow(1, "Admin", 1, "edit", 4, 1),
  ...Array.from({ length: 11 }, (_, i) =>
    transactionRow(0, "Anonymous", 10 - i, "page " + (11 - i), 100 + i)
  ),
  transactionRow(1, "Admin", 0, "", 0),
];

const SCENARIOS = [
  scenarioRow(1, "Admin", 0, 0),
  scenarioRow(0, "Anonymous", 4, 2000),
];

test("each disabled text shows for its flag and no rows render", async () => {
  const cases = [
    [
      { metrics_disabled: true, transaction_metrics_disabled: true, scenario_metrics_disabled: true },
      "Metrics disabled (--no-metrics).",
      "Metrics disabled (--no-metrics).",
    ],
    [
      { scenario_metrics_disabled: true },
      "Scenario metrics disabled (--no-scenario-metrics).",
      null,
    ],
    [
      { transaction_metrics_disabled: true },
      null,
      "Transaction metrics disabled (--no-transaction-metrics).",
    ],
  ];
  for (const [flags, scenarioText, transactionText] of cases) {
    for (const phase of ["idle", "maintain"]) {
      // Rows the flag says are off never render, even if a snapshot held them.
      const window = await load(
        snapshotWith(
          {
            phase,
            scenarios: scenarioText ? [] : SCENARIOS,
            transactions: transactionText ? [] : TRANSACTIONS,
          },
          flags
        )
      );
      if (scenarioText) {
        assert.equal(emptyText(window, "scenarios-body"), scenarioText);
      } else {
        assert.equal(rows(window, "scenarios-body").length, SCENARIOS.length);
      }
      if (transactionText) {
        assert.equal(emptyText(window, "transactions-body"), transactionText);
        typeFilter(window, "transactions-filter", "zzz");
        assert.equal(emptyText(window, "transactions-body"), transactionText);
      } else {
        assert.equal(rows(window, "transactions-body").length, TRANSACTIONS.length);
      }
    }
  }
});

test("disabled texts show on the idle snapshot", async () => {
  const window = await load({
    ...IDLE_SNAPSHOT,
    flags: {
      ...IDLE_SNAPSHOT.flags,
      transaction_metrics_disabled: true,
      scenario_metrics_disabled: true,
    },
  });
  assert.equal(
    emptyText(window, "scenarios-body"),
    "Scenario metrics disabled (--no-scenario-metrics)."
  );
  assert.equal(
    emptyText(window, "transactions-body"),
    "Transaction metrics disabled (--no-transaction-metrics)."
  );
});

test("enabled tables with no rows say nothing has run yet", async () => {
  const window = await load(IDLE_SNAPSHOT);
  assert.equal(emptyText(window, "scenarios-body"), "No scenarios yet");
  assert.equal(emptyText(window, "transactions-body"), "No transactions yet");
  typeFilter(window, "transactions-filter", "zzz");
  assert.equal(emptyText(window, "transactions-body"), "No transactions yet");
});

test("rows render in # order, counted from 1", async () => {
  const window = await load(
    snapshotWith({ scenarios: SCENARIOS, transactions: TRANSACTIONS })
  );
  assert.deepEqual(column(window, "scenarios-body", 0), ["1", "2"]);
  assert.deepEqual(column(window, "scenarios-body", 1), ["Anonymous", "Admin"]);
  assert.deepEqual(column(window, "transactions-body", 0), [
    "1.1",
    "1.2",
    "1.3",
    "1.4",
    "1.5",
    "1.6",
    "1.7",
    "1.8",
    "1.9",
    "1.10",
    "1.11",
    "2.1",
    "2.2",
  ]);
  assert.deepEqual(sortedHeaders(window, "scenarios-table"), [
    ["scenario_index", "asc"],
  ]);
  assert.deepEqual(sortedHeaders(window, "transactions-table"), [
    ["order", "asc"],
  ]);

  const anonymous = rows(window, "scenarios-body")[0];
  assert.deepEqual(anonymous, [
    "1",
    "Anonymous",
    int(4),
    int(2000),
    rate(200),
    rate(500),
    rate(250.5),
    int(200),
    int(1400),
    int(1500),
  ]);
  const edit = rows(window, "transactions-body")[12];
  assert.deepEqual(edit, [
    "2.2",
    "Admin",
    "edit",
    int(4),
    int(1),
    rate(0.4),
    rate(12.25),
    int(10),
    int(25),
    int(30),
  ]);
});

test("a never run row shows 0 counts and empty timing cells", async () => {
  const window = await load(
    snapshotWith({ scenarios: SCENARIOS, transactions: TRANSACTIONS })
  );
  const admin = rows(window, "scenarios-body")[1];
  assert.deepEqual(admin, [
    "2",
    "Admin",
    int(0),
    int(0),
    rate(0),
    "",
    "",
    "",
    "",
    "",
  ]);
  // An unnamed transaction shows an empty name.
  const unnamed = rows(window, "transactions-body")[11];
  assert.deepEqual(unnamed, [
    "2.1",
    "Admin",
    "",
    int(0),
    int(0),
    rate(0),
    "",
    "",
    "",
    "",
  ]);
});

test("the transactions filter matches scenario and transaction names", async () => {
  const window = await load(
    snapshotWith({ scenarios: SCENARIOS, transactions: TRANSACTIONS })
  );
  typeFilter(window, "transactions-filter", "ADMIN");
  assert.deepEqual(column(window, "transactions-body", 0), ["2.1", "2.2"]);
  typeFilter(window, "transactions-filter", "page 1");
  assert.deepEqual(column(window, "transactions-body", 2), [
    "page 1",
    "page 10",
    "page 11",
  ]);
  typeFilter(window, "transactions-filter", "zzz");
  assert.equal(emptyText(window, "transactions-body"), "No matching transactions");
});

test("a Transactions header click reverses its order", async () => {
  const window = await load(
    snapshotWith({ scenarios: SCENARIOS, transactions: TRANSACTIONS })
  );
  header(window, "transactions-table", "order").click();
  assert.deepEqual(column(window, "transactions-body", 0).slice(0, 4), [
    "2.2",
    "2.1",
    "1.11",
    "1.10",
  ]);
  assert.deepEqual(sortedHeaders(window, "transactions-table"), [
    ["order", "desc"],
  ]);

  header(window, "transactions-table", "run_count").click();
  assert.deepEqual(column(window, "transactions-body", 3).slice(0, 3), [
    int(110),
    int(109),
    int(108),
  ]);
  header(window, "transactions-table", "run_count").click();
  assert.deepEqual(column(window, "transactions-body", 3).slice(0, 3), [
    int(0),
    int(4),
    int(100),
  ]);
  assert.deepEqual(sortedHeaders(window, "transactions-table"), [
    ["run_count", "asc"],
  ]);
});

test("a transaction name is rendered as text, never as markup", async () => {
  const name = "<img src=x onerror=alert(1)>";
  const window = await load(
    snapshotWith({
      scenarios: [scenarioRow(0, name, 1, 1)],
      transactions: [transactionRow(0, "S", 0, name, 1)],
    })
  );
  assert.equal(rows(window, "transactions-body")[0][2], name);
  assert.equal(rows(window, "scenarios-body")[0][1], name);
  assert.equal(window.document.querySelectorAll("img").length, 0);
});
