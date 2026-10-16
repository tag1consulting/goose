// The dashboard client's control buttons (app.js), run in jsdom with
// `npm test` after `npm run build`: keyboard focus across a request, and
// presses while a request is in flight or a button is unavailable.
//
// jsdom does not move focus off a button that becomes disabled, as browsers
// do, so focus tests assert that the button is never `disabled`, not only
// where focus is.

import { test } from "node:test";
import assert from "node:assert/strict";
import { IDLE_SNAPSHOT } from "./idle-snapshot.mjs";
import { FRESH, openPage, openPages, settle } from "./test-harness.mjs";

const TOKEN = "s3cret";
const DELAY_MS = 30;
const BUTTONS = ["ctrl-start", "ctrl-stop", "ctrl-quit", "ctrl-apply", "ctrl-minus", "ctrl-plus"];

test.afterEach(() => {
  while (openPages.length) openPages.pop().window.close();
});

function snapshot(phase, target_users) {
  return JSON.stringify({ ...IDLE_SNAPSHOT, phase, target_users });
}

async function controlPage(options = {}) {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    controlEnabled: true,
    eventStream: true,
    ...options,
  });
  page.events.emit("snapshot", snapshot("idle", 20));
  await settle(page.window);
  return page;
}

function el(page, id) {
  return page.window.document.getElementById(id);
}

function posts(page) {
  return page.requests.filter((r) => r.method === "POST").map((r) => r.path);
}

/** Let a delayed control answer arrive and its promise chain finish. */
async function answered(page) {
  await new Promise((resolve) => setTimeout(resolve, DELAY_MS * 2));
  await settle(page.window);
}

test("Start keeps keyboard focus through its request and the phase change", async () => {
  const page = await controlPage({ controlDelayMs: DELAY_MS });
  const start = el(page, "ctrl-start");
  assert.equal(start.getAttribute("aria-disabled"), "false");
  start.focus();
  start.click();

  for (const id of BUTTONS) {
    assert.equal(el(page, id).disabled, false, id + " is never disabled");
    assert.equal(el(page, id).getAttribute("aria-disabled"), "true", id + " in flight");
  }
  assert.equal(page.window.document.activeElement, start);

  await answered(page);
  page.events.emit("snapshot", snapshot("increase", 20));
  await settle(page.window);
  assert.equal(start.disabled, false);
  assert.equal(start.getAttribute("aria-disabled"), "true", "Start is unavailable once running");
  assert.equal(el(page, "ctrl-stop").getAttribute("aria-disabled"), "false");
  assert.equal(page.window.document.activeElement, start);
});

test("minus and plus while a request is in flight change nothing", async () => {
  const page = await controlPage({ controlDelayMs: DELAY_MS });
  const target = el(page, "ctrl-target");
  assert.equal(target.value, "20");

  el(page, "ctrl-start").click();
  assert.equal(el(page, "ctrl-minus").disabled, false);
  el(page, "ctrl-minus").click();
  assert.equal(target.value, "20", "minus left the Target field alone");
  el(page, "ctrl-plus").click();
  assert.equal(target.value, "20", "plus left the Target field alone");
  el(page, "ctrl-apply").click();
  el(page, "ctrl-start").click();
  el(page, "ctrl-quit").click();
  assert.equal(el(page, "ctrl-status").textContent, "Sending…");

  await answered(page);
  assert.deepEqual(posts(page), ["/api/v1/control/start"]);

  // The field was never marked edited, so a new snapshot still updates it.
  page.events.emit("snapshot", snapshot("increase", 35));
  await settle(page.window);
  assert.equal(target.value, "35");
});

test("an unavailable button ignores a press", async () => {
  const page = await controlPage();
  page.events.emit("snapshot", snapshot("maintain", 20));
  await settle(page.window);
  const start = el(page, "ctrl-start");
  assert.equal(start.getAttribute("aria-disabled"), "true");
  start.click();
  await settle(page.window);
  assert.deepEqual(posts(page), []);

  page.events.emit("snapshot", snapshot("decrease", 20));
  await settle(page.window);
  const stop = el(page, "ctrl-stop");
  assert.equal(stop.getAttribute("aria-disabled"), "true");
  stop.click();
  await settle(page.window);
  assert.deepEqual(posts(page), []);
});

test("Apply and plus ignore a press while the run is stopping", async () => {
  const page = await controlPage();
  page.events.emit(
    "snapshot",
    JSON.stringify({ ...IDLE_SNAPSHOT, phase: "increase", target_users: 20, stopping: true }),
  );
  await settle(page.window);
  const apply = el(page, "ctrl-apply");
  assert.equal(apply.getAttribute("aria-disabled"), "true");
  assert.equal(el(page, "ctrl-plus").getAttribute("aria-disabled"), "true");
  apply.click();
  el(page, "ctrl-plus").click();
  await settle(page.window);
  assert.deepEqual(posts(page), []);
  assert.equal(el(page, "ctrl-target").value, "20");
});

test("a focused Quit that hides itself hands focus to Start", async () => {
  const page = await controlPage();
  const quit = el(page, "ctrl-quit");
  assert.equal(quit.getAttribute("aria-disabled"), "false");
  quit.focus();
  quit.click();
  await settle(page.window);
  assert.deepEqual(posts(page), ["/api/v1/control/quit"]);
  assert.equal(quit.hidden, true);
  assert.equal(page.window.document.activeElement, el(page, "ctrl-start"));
});

test("Quit keeps focus while its request is in flight", async () => {
  const page = await controlPage({ controlDelayMs: DELAY_MS });
  const quit = el(page, "ctrl-quit");
  quit.focus();
  quit.click();
  assert.equal(quit.disabled, false);
  assert.equal(quit.getAttribute("aria-disabled"), "true");
  assert.equal(page.window.document.activeElement, quit);
  quit.click();
  await answered(page);
  assert.deepEqual(posts(page), ["/api/v1/control/quit"]);
});

test("hiding Quit leaves focus on another control alone", async () => {
  const page = await controlPage();
  const target = el(page, "ctrl-target");
  target.focus();
  assert.equal(page.window.document.activeElement, target);
  page.events.emit("snapshot", snapshot("increase", 20));
  await settle(page.window);
  assert.equal(el(page, "ctrl-quit").hidden, true);
  assert.equal(page.window.document.activeElement, target);
});

test("Stop keeps focus and ignores a second press while its request is in flight", async () => {
  const page = await controlPage({ controlDelayMs: DELAY_MS });
  page.events.emit("snapshot", snapshot("increase", 20));
  await settle(page.window);
  const stop = el(page, "ctrl-stop");
  assert.equal(stop.getAttribute("aria-disabled"), "false");
  stop.focus();
  stop.click();
  assert.equal(stop.disabled, false);
  assert.equal(stop.getAttribute("aria-disabled"), "true");
  assert.equal(page.window.document.activeElement, stop);
  stop.click();
  await answered(page);
  assert.deepEqual(posts(page), ["/api/v1/control/stop"]);
});

test("the Target field falls back from target to maximum to active users", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    controlEnabled: true,
    eventStream: true,
  });
  const target = el(page, "ctrl-target");
  const show = async (users) => {
    page.events.emit("snapshot", JSON.stringify({ ...IDLE_SNAPSHOT, ...users }));
    await settle(page.window);
    return target.value;
  };
  // First, while the field is still empty: no target leaves the field as it was.
  assert.equal(await show({ target_users: 0, maximum_users: 0, active_users: 0 }), "", "never 0");
  assert.equal(await show({ target_users: 20, maximum_users: 50, active_users: 0 }), "20");
  assert.equal(await show({ target_users: 0, maximum_users: 50, active_users: 7 }), "50");
  assert.equal(await show({ target_users: undefined, maximum_users: 0, active_users: 7 }), "7");
});
