// The page around the panels (#708): the footer naming the controllers that
// /api/v1/health reports, the favicon link, and the theme following an OS
// change. Run with `npm test` after `npm run build`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { FRESH, openPage, openPages } from "./test-harness.mjs";
import { IDLE_SNAPSHOT } from "./idle-snapshot.mjs";

const TOKEN = "s3cret";

const CONTROL_LEAD =
  "Start, stop, and adjust users from this panel (authenticated). Stop begins a cancel ramp (decrease) before idle, and users cannot be changed until then. ";

test.afterEach(() => {
  while (openPages.length) openPages.pop().window.close();
});

async function footer(options) {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, options);
  return page.window.document.getElementById("footer-note").textContent;
}

test("the read only footer names the controllers health reports", async () => {
  const cases = [
    [
      { telnet: "0.0.0.0:5116", websocket: "0.0.0.0:5117" },
      "Control this test via the telnet controller listening on 0.0.0.0:5116 or the WebSocket controller listening on 0.0.0.0:5117. This dashboard is read-only.",
    ],
    [
      { telnet: null, websocket: "127.0.0.1:6000" },
      "Control this test via the WebSocket controller listening on 127.0.0.1:6000. This dashboard is read-only.",
    ],
    [
      { telnet: "[::1]:7000", websocket: null },
      "Control this test via the telnet controller listening on [::1]:7000. This dashboard is read-only.",
    ],
    [
      { telnet: null, websocket: null },
      "No telnet or WebSocket controller is running. This dashboard is read-only.",
    ],
  ];
  for (const [controllers, text] of cases) {
    assert.equal(await footer({ controllers }), text);
  }
});

test("the control footer names the controllers health reports", async () => {
  const cases = [
    [
      { telnet: "0.0.0.0:5116", websocket: "0.0.0.0:5117" },
      "Advanced control remains on the telnet controller listening on 0.0.0.0:5116 or the WebSocket controller listening on 0.0.0.0:5117.",
    ],
    [
      { telnet: null, websocket: "127.0.0.1:6000" },
      "Advanced control remains on the WebSocket controller listening on 127.0.0.1:6000.",
    ],
    [
      { telnet: null, websocket: null },
      "No telnet or WebSocket controller is running.",
    ],
  ];
  for (const [controllers, text] of cases) {
    assert.equal(
      await footer({ controlEnabled: true, controllers }),
      CONTROL_LEAD + text
    );
  }
});

test("a footer without controller addresses names no port", async () => {
  // A health answer without `controllers`, as from an older server.
  assert.equal(await footer({}), "This dashboard is read-only.");
  assert.equal(
    await footer({ controlEnabled: true }),
    CONTROL_LEAD.trimEnd()
  );
});

test("the page has an empty icon, so the browser asks for no favicon", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN);
  const links = page.window.document.querySelectorAll('head link[rel="icon"]');
  assert.equal(links.length, 1);
  assert.equal(links[0].getAttribute("href"), "data:,");
});

// The heatmap legend is the client's own DOM output of its colour scale, and
// differs between the themes.
function legend(page) {
  return page.window.document.getElementById("heatmap-legend-bar").style.background;
}

test("an OS theme change recolours the heatmap at once, with no new snapshot", async () => {
  const light = legend(await openPage("/?token=" + TOKEN, FRESH, TOKEN, { colorScheme: "light" }));
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { colorScheme: "dark" });
  const dark = legend(page);
  assert.ok(light.startsWith("linear-gradient"), light);
  assert.notEqual(dark, light);

  const requests = page.requests.length;
  page.colorScheme.set("light");
  assert.equal(legend(page), light);
  page.colorScheme.set("dark");
  assert.equal(legend(page), dark);
  assert.equal(page.requests.length, requests, "no request for the change");
});

// The line charts' colours, as chartColors in src/charts.ts has them.
const DARK = { rps: "#3d8bfd", fps: "#f07178", users: "#3dd68c", latency: "#e6b450" };
const LIGHT = { rps: "#0b5fff", fps: "#b42318", users: "#0a7a43", latency: "#9a6700" };
const GRID = { dark: "rgba(154, 167, 184, 0.2)", light: "rgba(91, 107, 124, 0.2)" };
const TICK = { dark: "#9aa7b8", light: "#5b6b7c" };
const SERIES_COLORS = new Set([...Object.values(DARK), ...Object.values(LIGHT)]);

/**
 * The series colours canvas `id` stroked in, each once, in the order it first
 * used them, from `page.canvas.calls` past index `from`.
 */
function strokes(page, id, from = 0) {
  const seen = [];
  for (const call of page.canvas.calls.slice(from)) {
    if (call.id !== id || call.name !== "strokeStyle") continue;
    if (SERIES_COLORS.has(call.value) && !seen.includes(call.value)) seen.push(call.value);
  }
  return seen;
}

test("an OS theme change redraws the line charts in the new theme's colours", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    colorScheme: "dark",
    canvas: true,
    // A few seconds of series, so both axes draw ticks and the legend shows.
    // The RPS chart leaves out the last, still counting second.
    snapshot: {
      ...IDLE_SNAPSHOT,
      series: {
        ...IDLE_SNAPSHOT.series,
        start_second: 10,
        rps: [1, 2, 3, 1],
        fps: [0, 1, 0, 0],
        users: [1, 1, 2, 2],
        avg_latency_ms: [5, 6, 7, 7],
      },
      flags: { ...IDLE_SNAPSHOT.flags, series_seconds: 4 },
    },
  });
  try {
    assert.deepEqual(strokes(page, "chart-rps"), [DARK.rps, DARK.fps]);
    assert.deepEqual(strokes(page, "chart-users"), [DARK.users]);

    const from = page.canvas.calls.length;
    page.colorScheme.set("light");
    assert.deepEqual(strokes(page, "chart-rps", from), [LIGHT.rps, LIGHT.fps]);
    assert.deepEqual(strokes(page, "chart-users", from), [LIGHT.users]);
    assert.deepEqual(strokes(page, "chart-latency", from), [LIGHT.latency]);
    const after = page.canvas.calls.slice(from);
    const rps = after.filter((call) => call.id === "chart-rps");
    assert.ok(
      rps.some((call) => call.name === "strokeStyle" && call.value === GRID.light),
      "light grid"
    );
    assert.ok(
      rps.some((call) => call.name === "fillStyle" && call.value === TICK.light),
      "light ticks"
    );
    const dark = new Set([...Object.values(DARK), GRID.dark, TICK.dark]);
    const stale = after.filter(
      (call) => (call.name === "strokeStyle" || call.name === "fillStyle") && dark.has(call.value)
    );
    assert.deepEqual(stale, [], "no dark series, grid or tick colour after the switch");

    // The fill colour in force at each fillText, following save and restore.
    const textColor = {};
    const stack = [];
    let fill = null;
    for (const call of rps) {
      if (call.name === "save") stack.push(fill);
      else if (call.name === "restore") fill = stack.pop();
      else if (call.name === "fillStyle") fill = call.value;
      else if (call.name === "fillText") textColor[call.value[0]] = fill;
    }
    assert.equal(textColor["RPS"], TICK.light, "legend label");
    assert.equal(textColor["11"], TICK.light, "x tick");
    assert.equal(textColor["3.0"], TICK.light, "y tick");
    // The legend boxes fill in each series' own colour.
    const fills = [];
    for (const call of rps) {
      if (call.name !== "fillStyle" || !SERIES_COLORS.has(call.value)) continue;
      if (!fills.includes(call.value)) fills.push(call.value);
    }
    assert.deepEqual(fills, [LIGHT.rps, LIGHT.fps], "legend boxes");
  } finally {
    page.window.close();
  }
});

test("an OS theme change with percentiles over time off does not draw the heatmap", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    colorScheme: "dark",
    canvas: true,
    snapshot: {
      ...IDLE_SNAPSHOT,
      flags: { ...IDLE_SNAPSHOT.flags, percentiles_over_time: false },
    },
  });
  try {
    const from = page.canvas.calls.length;
    page.colorScheme.set("light");
    const heatmap = page.canvas.calls.slice(from).filter((call) => call.id === "chart-heatmap");
    assert.deepEqual(heatmap, []);
    // The average latency chart, which the card shows instead, does redraw.
    assert.deepEqual(strokes(page, "chart-latency", from), [LIGHT.latency]);
  } finally {
    page.window.close();
  }
});
