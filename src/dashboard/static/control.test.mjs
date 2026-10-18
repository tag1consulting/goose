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

// A snapshot in `phase` whose target, and configured users, are `users`.
function snapshot(phase, users, settings = {}) {
  return JSON.stringify({
    ...IDLE_SNAPSHOT,
    phase,
    target_users: users,
    settings: { ...IDLE_SNAPSHOT.settings, users, stage: stageOf(phase), ...settings },
  });
}

function stageOf(phase) {
  return { increase: "ramp_up", maintain: "hold_until_stopped", decrease: "ramping_down" }[phase] || "idle";
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

test("while running, the Target field falls back from target to maximum to active users", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, {
    controlEnabled: true,
    eventStream: true,
  });
  const target = el(page, "ctrl-target");
  const show = async (users) => {
    page.events.emit(
      "snapshot",
      JSON.stringify({ ...IDLE_SNAPSHOT, phase: "maintain", ...users }),
    );
    await settle(page.window);
    return target.value;
  };
  // First, while the field is still empty: no target leaves the field as it was.
  assert.equal(await show({ target_users: 0, maximum_users: 0, active_users: 0 }), "", "never 0");
  assert.equal(await show({ target_users: 20, maximum_users: 50, active_users: 0 }), "20");
  assert.equal(await show({ target_users: 0, maximum_users: 50, active_users: 7 }), "50");
  assert.equal(await show({ target_users: 0, maximum_users: 0, active_users: 7 }), "7");
});

// Fields follow the server unless being edited (#706).

/** The users each `/users` POST sent, in order. */
function usersSent(page) {
  return page.requests
    .filter((r) => r.method === "POST" && r.path === "/api/v1/control/users")
    .map((r) => JSON.parse(r.body).users);
}

async function emit(page, json) {
  page.events.emit("snapshot", json);
  await settle(page.window);
}

test("after an Apply, the Target follows a change from elsewhere, and plus steps from it", async () => {
  const page = await controlPage();
  const target = el(page, "ctrl-target");
  await emit(page, snapshot("maintain", 20));
  target.value = "50";
  target.dispatchEvent(new page.window.Event("input"));
  el(page, "ctrl-apply").click();
  await settle(page.window);
  await emit(page, snapshot("maintain", 50));
  assert.equal(target.value, "50");
  // telnet `users 200`.
  await emit(page, snapshot("maintain", 200));
  assert.equal(target.value, "200", "the field follows the server again");
  el(page, "ctrl-plus").click();
  await settle(page.window);
  assert.deepEqual(usersSent(page), [50, 210]);
});

test("a snapshot from before an Apply does not put the old target back", async () => {
  const page = await controlPage();
  const target = el(page, "ctrl-target");
  await emit(page, snapshot("maintain", 20));
  target.value = "50";
  target.dispatchEvent(new page.window.Event("input"));
  el(page, "ctrl-apply").click();
  await settle(page.window);
  // The answer came first; this snapshot was taken before the change.
  await emit(page, snapshot("maintain", 20));
  assert.equal(target.value, "50", "a trailing snapshot is not a change");
  assert.doesNotMatch(el(page, "ctrl-status").textContent, /elsewhere/);
  await emit(page, snapshot("maintain", 50));
  await emit(page, snapshot("maintain", 20));
  assert.equal(target.value, "20", "once confirmed, the field follows again");
});

test("two quick plus clicks step from what was sent, and their own values are not changes", async () => {
  const page = await controlPage();
  const target = el(page, "ctrl-target");
  await emit(page, snapshot("maintain", 20));
  el(page, "ctrl-plus").click();
  await settle(page.window);
  el(page, "ctrl-plus").click();
  await settle(page.window);
  assert.deepEqual(usersSent(page), [30, 40]);
  // Snapshots catch up one value at a time.
  for (const users of [20, 30]) {
    await emit(page, snapshot("maintain", users));
    assert.equal(target.value, "40", "snapshot at " + users);
  }
  assert.doesNotMatch(el(page, "ctrl-status").textContent, /elsewhere/);
  await emit(page, snapshot("maintain", 40));
  assert.equal(target.value, "40");
});

test("an Apply that timed out but landed is not a change from elsewhere", async () => {
  const page = await controlPage({
    controlStatus: 503,
    controlBody: { error: "timeout", message: "timed out" },
  });
  const target = el(page, "ctrl-target");
  await emit(page, snapshot("maintain", 10));
  target.value = "50";
  target.dispatchEvent(new page.window.Event("input"));
  el(page, "ctrl-apply").click();
  await settle(page.window);
  assert.deepEqual(usersSent(page), [50]);
  await emit(page, snapshot("maintain", 10));
  assert.equal(target.value, "50", "the edit holds while the server is unchanged");
  await emit(page, snapshot("maintain", 50));
  assert.equal(target.value, "50");
  assert.doesNotMatch(el(page, "ctrl-status").textContent, /elsewhere/);
  // Not pending: the field follows the server again.
  await emit(page, snapshot("maintain", 30));
  assert.equal(target.value, "30");
  assert.doesNotMatch(el(page, "ctrl-status").textContent, /elsewhere/);
});

test("a refused Apply keeps the edit while the server is unchanged", async () => {
  const page = await controlPage({
    controlStatus: 409,
    controlBody: { ok: false, error: "invalid_phase", message: "not now" },
  });
  const target = el(page, "ctrl-target");
  await emit(page, snapshot("maintain", 10));
  target.value = "50";
  target.dispatchEvent(new page.window.Event("input"));
  el(page, "ctrl-apply").click();
  await settle(page.window);
  assert.deepEqual(usersSent(page), [50]);
  for (let i = 0; i < 2; i++) {
    await emit(page, snapshot("maintain", 10));
    assert.equal(target.value, "50", "the edit holds");
  }
  assert.equal(el(page, "ctrl-status").textContent, "not now");
  // Still pending: a change from elsewhere replaces it, saying so.
  await emit(page, snapshot("maintain", 200));
  assert.equal(target.value, "200");
  assert.equal(el(page, "ctrl-status").textContent, "Target changed elsewhere to 200");
});

test("an unapplied edit gives way to a change from elsewhere, saying so", async () => {
  const page = await controlPage();
  const target = el(page, "ctrl-target");
  await emit(page, snapshot("maintain", 20));
  target.value = "70";
  target.dispatchEvent(new page.window.Event("input"));
  await emit(page, snapshot("maintain", 20));
  assert.equal(target.value, "70", "the edit holds while the server is unchanged");
  await emit(page, snapshot("maintain", 200));
  assert.equal(target.value, "200");
  assert.equal(el(page, "ctrl-status").textContent, "Target changed elsewhere to 200");
});

test("a run that starts stopping discards an unapplied Target edit, once", async () => {
  const page = await controlPage();
  const target = el(page, "ctrl-target");
  await emit(page, snapshot("maintain", 20));
  target.value = "70";
  target.dispatchEvent(new page.window.Event("input"));
  const stopping = JSON.stringify({
    ...JSON.parse(snapshot("decrease", 0, { stage: "stopping" })),
    stopping: true,
    maximum_users: 20,
  });
  await emit(page, stopping);
  assert.equal(el(page, "ctrl-status").textContent, "Run stopping; unapplied Target edit discarded");
  assert.equal(target.value, "20");
  el(page, "ctrl-status").textContent = "";
  await emit(page, stopping);
  assert.equal(el(page, "ctrl-status").textContent, "", "said once");
});

test("while a test plan runs the Target row is read only, with the reason", async () => {
  const page = await controlPage();
  await emit(page, snapshot("maintain", 10, { users: null, test_plan: "10,60", stage: "test_plan" }));
  const target = el(page, "ctrl-target");
  for (const id of ["ctrl-target", "ctrl-step"]) {
    assert.equal(el(page, id).disabled, false, id + " is never disabled");
    assert.equal(el(page, id).readOnly, true, id);
    assert.equal(el(page, id).getAttribute("aria-disabled"), "true", id);
  }
  for (const id of ["ctrl-apply", "ctrl-minus", "ctrl-plus"]) {
    assert.equal(el(page, id).getAttribute("aria-disabled"), "true", id);
  }
  const reason = el(page, "ctrl-target-reason");
  assert.equal(reason.hidden, false);
  assert.equal(reason.textContent, "The test plan sets users; edit the plan, or stop and switch mode");
  assert.equal(target.getAttribute("aria-describedby"), "ctrl-target-help ctrl-target-reason");
  el(page, "ctrl-plus").click();
  await settle(page.window);
  assert.deepEqual(usersSent(page), []);

  // A users ramp again: editable, the reason gone.
  await emit(page, snapshot("maintain", 10));
  assert.equal(target.readOnly, false);
  assert.equal(target.hasAttribute("aria-disabled"), false);
  assert.equal(reason.hidden, true);
  assert.equal(target.getAttribute("aria-describedby"), "ctrl-target-help");
});

test("with no token the Target and step are read only, saying why", async () => {
  const page = await openPage("/", FRESH, TOKEN, { controlEnabled: true, eventStream: true });
  await emit(page, snapshot("idle", 20));
  assert.equal(el(page, "ctrl-target").readOnly, true);
  assert.equal(el(page, "ctrl-target").disabled, false);
  assert.equal(el(page, "ctrl-target-reason").textContent, "Enter the control token to change settings");
});

test("in Idle with a test plan, the Target is empty and says what Apply does", async () => {
  const page = await controlPage();
  await emit(page, snapshot("idle", 0, { users: null, test_plan: "10,30;10,3600;0,30" }));
  assert.equal(el(page, "ctrl-target").value, "");
  assert.equal(el(page, "ctrl-target").readOnly, false);
  assert.equal(
    el(page, "ctrl-target-help").textContent,
    "Applying a user count replaces the test plan with a users ramp.",
  );
});

test("the summary line says what the next Start runs, and where the run is", async () => {
  const page = await controlPage();
  const summary = el(page, "ctrl-summary");
  const show = async (phase, settings) => {
    await emit(page, snapshot(phase, settings.users == null ? 0 : settings.users, settings));
    return summary.textContent;
  };
  const ramp = {
    hosts: ["https://example.com/"],
    users: 100,
    increase_rate: 10,
    run_time_secs: 600,
    decrease_rate: 10,
  };
  assert.equal(
    await show("idle", ramp),
    "https://example.com/ · 100 users at 10/s · hold 10 min · then 10/s down",
  );
  assert.equal(
    await show("idle", { ...ramp, increase_rate: null, increase_time_secs: 300, decrease_rate: null }),
    "https://example.com/ · 100 users over 5 min · hold 10 min · then all stop",
  );
  assert.equal(
    await show("idle", { ...ramp, increase_rate: null, run_time_secs: null, decrease_rate: null, decrease_time_secs: 60 }),
    "https://example.com/ · 100 users at 1/s (default) · then straight down over 1 min",
  );
  assert.equal(
    await show("idle", { ...ramp, run_time_secs: null, decrease_rate: null }),
    "https://example.com/ · 100 users at 10/s · until stopped",
  );
  assert.equal(
    await show("increase", { ...ramp, stage: "ramp_up" }),
    "https://example.com/ · ramping to 100 users at 10/s · hold 10 min · then 10/s down",
  );
  assert.equal(
    await show("maintain", { ...ramp, stage: "hold", hold_ends_in_secs: 412 }),
    "https://example.com/ · 100 users · ramp down in 6 min 52 s",
  );
  assert.equal(
    await show("increase", { ...ramp, decrease_rate: null, stage: "hold", hold_ends_in_secs: 0 }),
    "https://example.com/ · 100 users · all stop after this ramp",
  );
  assert.equal(
    await show("maintain", { ...ramp, stage: "hold_until_stopped" }),
    "https://example.com/ · 100 users · holding until stopped",
  );
  assert.equal(
    await show("decrease", { ...ramp, stage: "ramping_down" }),
    "https://example.com/ · ramping down at 10/s",
  );
  assert.equal(
    await show("idle", { hosts: [], users: null, test_plan: "100,60;100,3600;0,0" }),
    "no host set · test plan: 3 steps, peak 100 users, 1 h 1 min total",
  );
  assert.equal(
    await show("idle", { host: "https://override.test", hosts: ["https://override.test/"], users: 2 }),
    "https://override.test · 2 users at 1/s (default) · until stopped",
  );
  // Not a live region: the countdown is not announced.
  assert.equal(summary.hasAttribute("role"), false);
  assert.equal(summary.hasAttribute("aria-live"), false);
});

test("with control off, the header shows the run settings, and the last run's hosts say so", async () => {
  const page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { eventStream: true });
  const hosts = el(page, "hosts");
  const runSettings = el(page, "run-settings");
  const settings = { hosts: ["https://next.test/"], users: 5, run_time_secs: 60 };
  // Idle, after a run against another host: both say which is which.
  await emit(
    page,
    JSON.stringify({
      ...JSON.parse(snapshot("idle", 5, settings)),
      hosts: ["https://last.test/"],
    }),
  );
  assert.equal(hosts.textContent, "Last run: https://last.test/");
  assert.equal(runSettings.hidden, false);
  assert.equal(
    runSettings.textContent,
    "https://next.test/ · 5 users at 1/s (default) · hold 1 min · then all stop",
  );
  // Running, the hosts are the run's own, and the line leaves them out.
  await emit(
    page,
    JSON.stringify({
      ...JSON.parse(snapshot("maintain", 5, { ...settings, stage: "hold", hold_ends_in_secs: 30 })),
      hosts: ["https://next.test/"],
    }),
  );
  assert.equal(hosts.textContent, "https://next.test/");
  assert.equal(runSettings.textContent, "5 users · all stop in 30 s");
});

test("with control on, the header leaves the run settings to the control card", async () => {
  const page = await controlPage();
  await emit(page, snapshot("idle", 5, { hosts: ["https://next.test/"] }));
  assert.equal(el(page, "run-settings").hidden, true);
  assert.equal(el(page, "hosts").textContent, "—");
});
