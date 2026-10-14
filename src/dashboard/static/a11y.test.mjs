// Accessibility of the dashboard (#709): the main landmark, names and roles,
// the Target field's invalid state, contrast read from app.css, and the dashed
// failures line. Run with `npm test`; `pretest` builds src/charts.ts into
// test-build/, and the page tests need `npm run build`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import * as charts from "./test-build/charts.mjs";
import * as heatmap from "./test-build/heatmap.mjs";
import { IDLE_SNAPSHOT } from "./idle-snapshot.mjs";
import { FRESH, openPage, openPages, settle } from "./test-harness.mjs";

const TOKEN = "s3cret";

test.afterEach(() => {
  while (openPages.length) openPages.pop().window.close();
});

// app.css.

// jsdom neither loads app.css nor evaluates media queries, so the contrast
// tests read the stylesheet through jsdom's CSSOM: every rule with its
// selectors, declarations and the media query it sits in, in source order.
const SHEET = new JSDOM(
  "<style>" + readFileSync(new URL("app.css", import.meta.url), "utf8") + "</style>"
).window.document.styleSheets[0];
const RULES = [...SHEET.cssRules]
  .flatMap((r) => (r.media ? [...r.cssRules].map((s) => [r.media.mediaText, s]) : [[null, r]]))
  .filter(([, r]) => r.selectorText)
  .map(([media, r]) => ({
    media,
    selectors: r.selectorText.split(",").map((s) => s.trim()),
    decls: Object.fromEntries([...r.style].map((p) => [p, r.style.getPropertyValue(p)])),
  }));
const LIGHT = "(prefers-color-scheme: light)";
const THEMES = { dark: [null], light: [null, LIGHT] };

test("the light theme's media blocks are parsed", () => {
  assert.ok(
    RULES.some((r) => r.media === LIGHT && r.selectors.includes(":root")),
    "a light :root block was parsed"
  );
  for (const r of RULES) {
    if (r.media && r.media.includes("prefers-color-scheme")) {
      assert.equal(r.media, LIGHT, "a colour scheme query is the light one");
    }
  }
});

// The rules that apply in a theme: the unconditional ones, plus the light
// media blocks in the light theme.
function themeRules(theme) {
  return RULES.filter((r) => THEMES[theme].includes(r.media));
}

// The custom properties of :root in a theme, later blocks overriding.
function tokens(theme) {
  const out = {};
  for (const rule of themeRules(theme)) {
    if (!rule.selectors.includes(":root")) continue;
    for (const [name, value] of Object.entries(rule.decls)) {
      if (name.startsWith("--")) out[name] = value;
    }
  }
  return out;
}

// A colour value with var() resolved, as #rrggbb.
function resolve(value, vars) {
  const v = value.trim();
  const ref = /^var\((--[\w-]+)\)$/.exec(v);
  if (ref) return resolve(vars[ref[1]], vars);
  const rgb = /^rgb\((\d+), (\d+), (\d+)\)$/.exec(v);
  if (rgb) return "#" + rgb.slice(1).map((c) => (+c).toString(16).padStart(2, "0")).join("");
  if (/^#fff$/i.test(v)) return "#ffffff";
  assert.match(v, /^#[0-9a-f]{6}$/i, "a plain hex colour: " + v);
  return v.toLowerCase();
}

// WCAG 2.x relative luminance and contrast ratio.
function luminance(hex) {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

// The value of `prop` for an element matching `selectors` (most specific
// last), by cascade order: specificity, then source order.
function cascaded(theme, selectors, prop) {
  let value;
  let rank = -1;
  for (const rule of themeRules(theme)) {
    for (const sel of rule.selectors) {
      const specificity = selectors.indexOf(sel);
      if (specificity < 0 || !(prop in rule.decls)) continue;
      if (specificity >= rank) {
        rank = specificity;
        value = rule.decls[prop];
      }
    }
  }
  return value;
}

const PHASES = ["idle", "increase", "maintain", "decrease", "shutdown"];

for (const theme of Object.keys(THEMES)) {
  test(`every phase badge text is at least 4.5:1 in the ${theme} theme`, () => {
    const vars = tokens(theme);
    for (const phase of PHASES) {
      const selectors = [".phase-badge", ".phase-badge.phase-" + phase];
      const fg = resolve(cascaded(theme, selectors, "color"), vars);
      const bg = resolve(cascaded(theme, selectors, "background"), vars);
      const ratio = contrast(fg, bg);
      assert.ok(
        ratio >= 4.5,
        `${phase} badge: ${fg} on ${bg} is ${ratio.toFixed(2)}:1`
      );
    }
  });

  test(`the pressed heatmap toggle is at least 4.5:1 in the ${theme} theme`, () => {
    const vars = tokens(theme);
    const selectors = [".segmented button", '.segmented button[aria-pressed="true"]'];
    const fg = resolve(cascaded(theme, selectors, "color"), vars);
    const bg = resolve(cascaded(theme, selectors, "background"), vars);
    const ratio = contrast(fg, bg);
    assert.ok(ratio >= 4.5, `${fg} on ${bg} is ${ratio.toFixed(2)}:1`);
  });

  test(`input borders are at least 3:1 against the card in the ${theme} theme`, () => {
    const vars = tokens(theme);
    for (const selector of [
      ".control-label input",
      ".chart-controls select",
      ".filter-label input",
    ]) {
      const color =
        cascaded(theme, [selector], "border-color") ||
        cascaded(theme, [selector], "border").split(/\s+/).pop();
      const border = resolve(color, vars);
      const card = resolve("var(--card)", vars);
      const ratio = contrast(border, card);
      assert.ok(
        ratio >= 3,
        `${selector}: border ${border} on card ${card} is ${ratio.toFixed(2)}:1`
      );
    }
  });
}

// The page.

test("the content sits in one main landmark, between header and footer", async () => {
  const { window } = await openPage("/?token=" + TOKEN, FRESH, TOKEN);
  const doc = window.document;
  const mains = doc.querySelectorAll("main");
  assert.equal(mains.length, 1);
  const main = mains[0];
  for (const id of ["banner", "control-panel", "kpis", "charts", "runs-panel"]) {
    assert.ok(main.contains(doc.getElementById(id)), id + " is inside main");
  }
  assert.ok(!main.contains(doc.querySelector("header")), "header is outside main");
  assert.ok(!main.contains(doc.querySelector("footer")), "footer is outside main");
});

test("nothing before main takes keyboard focus, so no skip link is needed", async () => {
  const { window } = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    controlEnabled: true,
  });
  const doc = window.document;
  const main = doc.querySelector("main");
  assert.ok(main, "the page has a main landmark");
  const focusable = doc.querySelectorAll(
    "a[href], button, input, select, textarea, [tabindex], [contenteditable]"
  );
  for (const el of focusable) {
    const before =
      main.compareDocumentPosition(el) & window.Node.DOCUMENT_POSITION_PRECEDING;
    assert.ok(!before, (el.id || el.tagName) + " comes before main");
  }
});

test("the step buttons are named by aria-label, not by their glyphs", async () => {
  const { window } = await openPage("/?token=" + TOKEN, FRESH, TOKEN);
  const doc = window.document;
  assert.equal(
    doc.getElementById("ctrl-minus").getAttribute("aria-label"),
    "Decrease target by step"
  );
  assert.equal(
    doc.getElementById("ctrl-plus").getAttribute("aria-label"),
    "Increase target by step"
  );
});

test("every chart canvas is an image with a name", async () => {
  const { window } = await openPage("/?token=" + TOKEN, FRESH, TOKEN);
  const canvases = window.document.querySelectorAll("canvas");
  assert.equal(canvases.length, 4);
  for (const canvas of canvases) {
    assert.equal(canvas.getAttribute("role"), "img", canvas.id + " role");
    assert.ok(canvas.getAttribute("aria-label"), canvas.id + " has a label");
  }
});

// The Target field's invalid state.

async function controlPage() {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    controlEnabled: true,
  });
  const doc = page.window.document;
  const target = doc.getElementById("ctrl-target");
  target.value = "0";
  doc.getElementById("ctrl-apply").click();
  await settle(page.window);
  assert.equal(target.getAttribute("aria-invalid"), "true", "Target is invalid");
  return { ...page, doc, target };
}

function usersPosts(requests) {
  return requests.filter(
    (r) => r.method === "POST" && r.path === "/api/v1/control/users"
  );
}

function assertValid(target) {
  assert.equal(target.hasAttribute("aria-invalid"), false, "aria-invalid cleared");
  assert.equal(
    target.hasAttribute("aria-describedby"),
    false,
    "aria-describedby cleared"
  );
}

test("an invalid Target is marked invalid and described by its message", async () => {
  const { doc, target, requests } = await controlPage();
  assert.equal(target.getAttribute("aria-invalid"), "true");
  assert.equal(target.getAttribute("aria-describedby"), "ctrl-status");
  assert.equal(
    doc.getElementById("ctrl-status").textContent,
    "Target must be an integer ≥ 1"
  );
  assert.equal(usersPosts(requests).length, 0, "nothing was sent");
});

test("editing the Target clears its invalid state", async () => {
  const { window, target } = await controlPage();
  target.value = "5";
  target.dispatchEvent(new window.Event("input"));
  assertValid(target);
});

test("a step click clears the Target's invalid state", async () => {
  const { window, doc, target, requests } = await controlPage();
  doc.getElementById("ctrl-plus").click();
  await settle(window);
  assertValid(target);
  assert.equal(usersPosts(requests).length, 1, "the step was sent");
});

test("a valid Apply clears the Target's invalid state", async () => {
  const { window, doc, target, requests } = await controlPage();
  target.value = "5";
  doc.getElementById("ctrl-apply").click();
  await settle(window);
  assertValid(target);
  assert.equal(usersPosts(requests).length, 1, "the target was sent");
});

// Answer the users command with a refusal, as the server does for a target it
// will not take; everything else goes to the stub.
function refuseUsers(window) {
  const fetch = window.fetch;
  window.fetch = (input, init) =>
    new URL(String(input), "http://x").pathname === "/api/v1/control/users"
      ? Promise.resolve({
          status: 409,
          ok: false,
          json: () => Promise.resolve({ ok: false, message: "no" }),
        })
      : fetch(input, init);
}

test("a refused step clears the Target's invalid state", async () => {
  const { window, doc, target } = await controlPage();
  refuseUsers(window);
  doc.getElementById("ctrl-plus").click();
  await settle(window);
  assertValid(target);
});

test("a refused valid Apply clears the Target's invalid state", async () => {
  const { window, doc, target } = await controlPage();
  refuseUsers(window);
  target.value = "5";
  doc.getElementById("ctrl-apply").click();
  await settle(window);
  assertValid(target);
});

// An invalid Target with no input event, on a page whose event stream the test drives.
async function invalidStreamPage() {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    controlEnabled: true,
    eventStream: true,
  });
  const doc = page.window.document;
  const target = doc.getElementById("ctrl-target");
  target.value = "";
  doc.getElementById("ctrl-apply").click();
  await settle(page.window);
  assert.equal(target.getAttribute("aria-invalid"), "true", "Target is invalid");
  return { ...page, doc, target };
}

test("a snapshot that rewrites the Target clears its invalid state", async () => {
  const { window, events, target } = await invalidStreamPage();
  events.emit("snapshot", JSON.stringify({ ...IDLE_SNAPSHOT, target_users: 5 }));
  await settle(window);
  assert.equal(target.value, "5", "the snapshot wrote the Target");
  assertValid(target);
});

test("a Start answer that rewrites the Target clears its invalid state", async () => {
  const { window, doc, target } = await invalidStreamPage();
  // The stub answers control without target_users; this Start answer has it.
  const fetch = window.fetch;
  window.fetch = (input, init) =>
    new URL(String(input), "http://x").pathname === "/api/v1/control/start"
      ? Promise.resolve({
          status: 200,
          ok: true,
          json: () =>
            Promise.resolve({
              ok: true,
              command: "start",
              phase: "increase",
              target_users: 7,
            }),
        })
      : fetch(input, init);
  doc.getElementById("ctrl-start").click();
  await settle(window);
  assert.equal(target.value, "7", "the Start answer wrote the Target");
  assertValid(target);
});

test("Stop after an invalid Apply clears the Target's invalid state", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    controlEnabled: true,
    eventStream: true,
  });
  const { window, events } = page;
  const doc = window.document;
  // A running test, so Stop is enabled.
  events.emit(
    "snapshot",
    JSON.stringify({ ...IDLE_SNAPSHOT, phase: "maintain", active_users: 3 })
  );
  await settle(window);
  const target = doc.getElementById("ctrl-target");
  target.value = "0";
  doc.getElementById("ctrl-apply").click();
  await settle(window);
  assert.equal(target.getAttribute("aria-invalid"), "true", "Target is invalid");
  const stop = doc.getElementById("ctrl-stop");
  assert.equal(stop.disabled, false, "Stop is enabled");
  stop.click();
  await settle(window);
  assert.notEqual(
    doc.getElementById("ctrl-status").textContent,
    "Target must be an integer ≥ 1",
    "Stop rewrote the status line"
  );
  assertValid(target);
});

// The RPS chart.

test("the failures line is dashed and the RPS line solid", () => {
  const datasets = charts.rpsDatasets({
    rps: "#3d8bfd",
    fps: "#f07178",
    users: "#3dd68c",
    latency: "#e6b450",
    grid: "rgba(0, 0, 0, 0.2)",
    tick: "#9aa7b8",
    percentile: "#ff8f45",
  });
  assert.deepEqual(
    datasets.map((d) => d.label),
    ["RPS", "Failures/s"]
  );
  const [rps, failures] = datasets;
  assert.equal(rps.borderDash, undefined, "RPS is solid");
  assert.ok(
    Array.isArray(failures.borderDash) && failures.borderDash.length > 0,
    "Failures/s is dashed"
  );
  assert.equal(rps.borderColor, "#3d8bfd");
  assert.equal(failures.borderColor, "#f07178");
});

test("the legend draws each swatch as a line, so it shows the dash", () => {
  const opts = charts.baseChartOpts({
    rps: "#3d8bfd",
    fps: "#f07178",
    users: "#3dd68c",
    latency: "#e6b450",
    grid: "rgba(0, 0, 0, 0.2)",
    tick: "#9aa7b8",
    percentile: "#ff8f45",
  });
  const labels = opts.plugins.legend.labels;
  assert.equal(labels.usePointStyle, true, "the legend uses a point style");
  assert.equal(labels.pointStyle, "line", "the point style is a line");
  // Wide enough for two full dash periods of the failures line, or the swatch
  // reads as solid.
  const [, failures] = charts.rpsDatasets({ rps: "#000000", fps: "#000000" });
  const period = failures.borderDash.reduce((a, b) => a + b, 0);
  assert.ok(
    labels.pointStyleWidth >= 2 * period,
    `swatch ${labels.pointStyleWidth} px shows two ${period} px dash periods`
  );
  assert.equal(typeof labels.generateLabels, "function", "legend items get the dash");
});

// With usePointStyle, Chart.js styles legend items from the points, which have
// no dash; withLineDash puts each dataset's dash back on its item.
test("the Failures/s legend item carries its line's dash", () => {
  const datasets = charts.rpsDatasets({ rps: "#000000", fps: "#000000" });
  const items = charts.withLineDash(
    [
      { text: "RPS", datasetIndex: 0 },
      { text: "Failures/s", datasetIndex: 1 },
    ],
    datasets
  );
  assert.equal(items[0].lineDash, undefined, "RPS stays solid");
  assert.deepEqual(items[1].lineDash, datasets[1].borderDash, "Failures/s is dashed");
});

// The heatmap's percentile lines (#709): one colour, so told apart by pattern.

test("the heatmap's percentile lines differ by pattern, not width", () => {
  const lines = heatmap.PERCENTILE_LINES;
  assert.deepEqual(
    lines.map((l) => l.key),
    ["p50", "p95", "p99"]
  );
  assert.equal(new Set(lines.map((l) => l.width)).size, 1, "one width");
  assert.deepEqual(lines[0].dash, [], "p50 is solid");
  assert.ok(lines[1].dash.length && lines[2].dash.length, "p95 and p99 are broken");
  assert.notDeepEqual(lines[1].dash, lines[2].dash, "p95 and p99 differ");
});

for (const theme of Object.keys(THEMES)) {
  test(`the heatmap key draws p50 solid, p95 dashed, p99 dotted in the ${theme} theme`, () => {
    const style = (key) =>
      cascaded(
        theme,
        [".heatmap-key::before", `.heatmap-key-${key}::before`],
        "border-top-style"
      ) || cascaded(theme, [".heatmap-key::before"], "border-top").split(/\s+/)[1];
    assert.equal(style("p50"), "solid");
    assert.equal(style("p95"), "dashed");
    assert.equal(style("p99"), "dotted");
  });
}

test("lines ending at one value share one label", () => {
  const labels = heatmap.percentileLabels([
    { key: "p50", value: 1, y: 100 },
    { key: "p95", value: 1, y: 100 },
    { key: "p99", value: 1, y: 100 },
  ]);
  assert.deepEqual(labels, [{ text: "p50, p95, p99: 1 ms", y: 100 }]);
});

test("labels run top to bottom by height, tied lines grouped", () => {
  const labels = heatmap.percentileLabels([
    { key: "p50", value: 1, y: 100 },
    { key: "p99", value: 4, y: 60 },
    { key: "p95", value: 4, y: 60 },
  ]);
  assert.deepEqual(
    labels.map((l) => l.text),
    ["p95, p99: 4 ms", "p50: 1 ms"]
  );
});

test("labels closer than 12 pixels are nudged apart, in order", () => {
  const labels = heatmap.percentileLabels([
    { key: "p50", value: 1, y: 100 },
    { key: "p95", value: 2, y: 95 },
    { key: "p99", value: 3, y: 92 },
  ]);
  assert.deepEqual(
    labels.map((l) => [l.text, l.y]),
    [
      ["p99: 3 ms", 92],
      ["p95: 2 ms", 104],
      ["p50: 1 ms", 116],
    ]
  );
});

// drawHeatmap against a context that records each stroke.
function recordStrokes(view) {
  const strokes = [];
  let dash = [];
  const ctx = new Proxy(
    {
      setLineDash: (d) => {
        dash = d;
      },
      measureText: (text) => ({ width: text.length * 6 }),
      stroke() {
        strokes.push({ style: ctx.strokeStyle, width: ctx.lineWidth, dash });
      },
    },
    {
      get: (target, prop) => (prop in target ? target[prop] : () => {}),
      set: (target, prop, value) => {
        target[prop] = value;
        return true;
      },
    }
  );
  const canvas = { clientWidth: 600, clientHeight: 300, getContext: () => ctx };
  const saved = [globalThis.window, globalThis.getComputedStyle];
  globalThis.window = { devicePixelRatio: 1 };
  globalThis.getComputedStyle = () => ({
    fontFamily: "sans-serif",
    getPropertyValue: () => "#1a2332",
  });
  try {
    heatmap.drawHeatmap(canvas, view, true, {
      tick: "#9aa7b8",
      grid: "#333333",
      percentile: "#ff8f45",
    });
  } finally {
    [globalThis.window, globalThis.getComputedStyle] = saved;
  }
  return strokes;
}

test("every halo is drawn before any percentile line", () => {
  const row = (bucket, time, top) => ({
    bucket,
    time,
    lowest: time,
    highest: time,
    top,
    bottom: top + 0.2,
  });
  const percentiles = new Map();
  for (const second of [0, 1]) {
    // p95 and p99 in one row: p99's halo must not hide p95.
    percentiles.set(second, { second, p50: 1, p95: 4, p99: 4 });
  }
  const strokes = recordStrokes({
    startSecond: 0,
    columns: 2,
    rows: [row(0, 1, 0.6), row(1, 4, 0.2)],
    cells: [],
    minCount: 0,
    maxCount: 0,
    ticks: [],
    secondTotals: new Map(),
    percentiles,
  });
  const lines = strokes.filter((s) => s.style === "#ff8f45");
  const halos = strokes.filter((s) => s.style === "#1a2332");
  assert.equal(lines.length, 3, "one stroke per line");
  assert.equal(halos.length, 3, "one halo per line");
  const firstLine = strokes.findIndex((s) => s.style === "#ff8f45");
  const lastHalo = strokes.map((s) => s.style).lastIndexOf("#1a2332");
  assert.ok(lastHalo < firstLine, "halos come first");
  assert.deepEqual(
    lines.map((s) => s.dash),
    heatmap.PERCENTILE_LINES.map((l) => l.dash)
  );
});

test("the right margin fits the widest percentile label", () => {
  const percentiles = new Map([[0, { second: 0, p50: 200, p95: 200, p99: 200 }]]);
  const view = {
    startSecond: 0,
    columns: 1,
    rows: [],
    cells: [],
    minCount: 0,
    maxCount: 0,
    ticks: [],
    secondTotals: new Map(),
    percentiles,
  };
  assert.deepEqual(
    heatmap.percentileLabels(heatmap.newestPercentiles(view)).map((l) => l.text),
    ["p50, p95, p99: 200 ms"]
  );
  const ctx = { measureText: (text) => ({ width: text.length * 6 }) };
  const canvas = {
    clientWidth: 600,
    clientHeight: 300,
    getContext: () =>
      new Proxy(ctx, { get: (t, p) => (p in t ? t[p] : () => {}), set: () => true }),
  };
  const saved = [globalThis.window, globalThis.getComputedStyle];
  globalThis.window = { devicePixelRatio: 1 };
  globalThis.getComputedStyle = () => ({ fontFamily: "x", getPropertyValue: () => "" });
  let layout;
  try {
    layout = heatmap.drawHeatmap(canvas, view, true, {
      tick: "#000000",
      grid: "#000000",
      percentile: "#000000",
    });
  } finally {
    [globalThis.window, globalThis.getComputedStyle] = saved;
  }
  const labelRoom = 600 - (layout.left + layout.width) - 6;
  assert.ok(labelRoom >= "p50, p95, p99: 200 ms".length * 6, `${labelRoom} px for the label`);
});

test("labels never sit above the room left for the caption", () => {
  const labels = heatmap.percentileLabels(
    [
      { key: "p99", value: 200, y: 2 },
      { key: "p50", value: 5, y: 8 },
    ],
    25
  );
  assert.deepEqual(
    labels.map((l) => [l.text, l.y]),
    [
      ["p99: 200 ms", 25],
      ["p50: 5 ms", 37],
    ]
  );
});

test("a muted caption says the labels are the last second's", () => {
  const texts = [];
  const ctx = {
    measureText: (text) => ({ width: text.length * 6 }),
    fillText(text, x, y) {
      texts.push({ text, y, fill: ctx.fillStyle });
    },
  };
  const canvas = {
    clientWidth: 600,
    clientHeight: 300,
    getContext: () =>
      new Proxy(ctx, {
        get: (t, p) => (p in t ? t[p] : () => {}),
        set: (t, p, v) => {
          t[p] = v;
          return true;
        },
      }),
  };
  const row = { bucket: 0, time: 5, lowest: 5, highest: 5, top: 0.4, bottom: 0.6 };
  const view = {
    startSecond: 0,
    columns: 1,
    rows: [row],
    cells: [],
    minCount: 0,
    maxCount: 0,
    ticks: [],
    secondTotals: new Map(),
    percentiles: new Map([[0, { second: 0, p50: 5, p95: 5, p99: 5 }]]),
  };
  const saved = [globalThis.window, globalThis.getComputedStyle];
  globalThis.window = { devicePixelRatio: 1 };
  globalThis.getComputedStyle = () => ({ fontFamily: "x", getPropertyValue: () => "" });
  try {
    heatmap.drawHeatmap(canvas, view, true, {
      tick: "#9aa7b8",
      grid: "#333333",
      percentile: "#ff8f45",
    });
  } finally {
    [globalThis.window, globalThis.getComputedStyle] = saved;
  }
  const caption = texts.find((t) => t.text === heatmap.LABELS_CAPTION);
  const label = texts.find((t) => t.text === "p50, p95, p99: 5 ms");
  assert.ok(caption, "the caption is drawn");
  assert.ok(label, "the shared label is drawn");
  assert.equal(caption.fill, "#9aa7b8", "the caption is muted");
  assert.ok(caption.y < label.y, "the caption sits above the labels");
});
