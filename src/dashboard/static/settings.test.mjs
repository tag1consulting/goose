// The run settings form (app.js), run in jsdom with `npm test` after
// `npm run build`: pending edits and the labels that count them, what Apply
// settings and Start with changes send, read only fields by stage, discards
// when the stage changes, the host confirmation, and focus on a refused field.

import { test } from "node:test";
import assert from "node:assert/strict";
import { IDLE_SNAPSHOT } from "./idle-snapshot.mjs";
import { FRESH, openPage, openPages, settle } from "./test-harness.mjs";

const TOKEN = "s3cret";

test.afterEach(() => {
  while (openPages.length) openPages.pop().window.close();
});

const CONFIGURED = {
  host: "http://old.test",
  hosts: ["http://old.test/"],
  users: 20,
  increase_rate: 5,
  increase_time_secs: null,
  decrease_rate: null,
  decrease_time_secs: null,
  run_time_secs: 600,
};

// A snapshot in `phase` at `stage` with the configured settings and `settings`.
function snapshot(phase, stage, settings = {}) {
  return JSON.stringify({
    ...IDLE_SNAPSHOT,
    phase,
    active_users: phase === "idle" ? 0 : 20,
    target_users: phase === "idle" ? 0 : 20,
    settings: { ...IDLE_SNAPSHOT.settings, ...CONFIGURED, stage, ...settings },
  });
}

async function formPage(options = {}, local = {}) {
  const page = await openPage(
    "/?token=" + TOKEN,
    { session: {}, local },
    TOKEN,
    { controlEnabled: true, eventStream: true, ...options }
  );
  await emit(page, snapshot("idle", "idle"));
  return page;
}

async function emit(page, json) {
  page.events.emit("snapshot", json);
  await settle(page.window);
}

function el(page, id) {
  return page.window.document.getElementById(id);
}

function type(page, id, value) {
  const input = el(page, id);
  input.value = value;
  input.dispatchEvent(new page.window.Event("input"));
}

function choose(page, id, value) {
  const select = el(page, id);
  select.value = value;
  select.dispatchEvent(new page.window.Event("change"));
}

function controlPosts(page) {
  return page.requests
    .filter((r) => r.method === "POST")
    .map((r) => ({ path: r.path, body: r.body === undefined ? undefined : JSON.parse(r.body) }));
}

function label(page) {
  return el(page, "ctrl-settings-label").textContent;
}

test("the form follows the server, showing a time in its largest even unit", async () => {
  const page = await formPage();
  assert.equal(el(page, "set-host").value, "http://old.test");
  assert.equal(el(page, "set-up").value, "5");
  assert.equal(el(page, "set-up-unit").value, "rate");
  assert.equal(el(page, "set-down").value, "");
  assert.equal(el(page, "set-down").placeholder, "default: all stop at once");
  assert.equal(el(page, "set-run").value, "10");
  assert.equal(el(page, "set-run-unit").value, "min");
  assert.match(el(page, "set-run-help").textContent, /^Time at full users after the ramp up, not total time\. Empty: holds until stopped\.$/);
  await emit(page, snapshot("idle", "idle", { increase_rate: null, increase_time_secs: 5400, decrease_rate: 2, run_time_secs: 90 }));
  assert.equal(el(page, "set-up").value, "90");
  assert.equal(el(page, "set-up-unit").value, "min");
  assert.equal(el(page, "set-down").value, "2");
  assert.equal(el(page, "set-run").value, "90");
  assert.equal(el(page, "set-run-unit").value, "s");
  assert.match(el(page, "set-run-help").textContent, /Empty: ramps straight down after the ramp up\./);
});

test("pending edits are counted, Start says it sends them, and Discard edits returns the server's values", async () => {
  const page = await formPage();
  assert.equal(label(page), "Edit settings");
  assert.equal(el(page, "ctrl-start").textContent, "Start");
  assert.equal(el(page, "set-apply").getAttribute("aria-disabled"), "true");
  type(page, "set-up", "8");
  assert.equal(label(page), "Edit settings (1 unapplied)");
  assert.equal(el(page, "ctrl-start").textContent, "Start with changes");
  assert.equal(el(page, "set-apply").getAttribute("aria-disabled"), "false");
  choose(page, "set-run-unit", "h");
  assert.equal(label(page), "Edit settings (2 unapplied)");
  type(page, "ctrl-target", "30");
  assert.equal(label(page), "Edit settings (3 unapplied)");
  // The Target goes with the other edits: its own Apply waits.
  assert.equal(el(page, "ctrl-apply").getAttribute("aria-disabled"), "true");
  assert.equal(
    el(page, "ctrl-target-reason").textContent,
    "Sent with your other edits: use Start with changes or Apply settings"
  );
  el(page, "ctrl-plus").click();
  await settle(page.window);
  assert.equal(el(page, "ctrl-target").value, "40", "plus only changes the field");
  assert.deepEqual(controlPosts(page), []);

  el(page, "set-discard").click();
  await settle(page.window);
  assert.equal(label(page), "Edit settings");
  assert.equal(el(page, "ctrl-start").textContent, "Start");
  assert.equal(el(page, "set-up").value, "5");
  assert.equal(el(page, "set-run-unit").value, "min");
  assert.equal(el(page, "ctrl-target").value, "20");
  assert.equal(el(page, "ctrl-apply").getAttribute("aria-disabled"), "false");
  assert.deepEqual(controlPosts(page), []);
});

test("Start with changes sends only the edited settings, an emptied field as null, with the Target's users", async () => {
  const page = await formPage();
  type(page, "set-up", "5");
  choose(page, "set-up-unit", "min");
  type(page, "set-run", "");
  type(page, "ctrl-target", "30");
  el(page, "ctrl-start").click();
  await settle(page.window);
  assert.deepEqual(controlPosts(page), [
    {
      path: "/api/v1/control/start",
      body: { increase_time_secs: 300, increase_rate: null, run_time_secs: null, users: 30 },
    },
  ]);
});

test("a plain Start sends an empty body", async () => {
  const page = await formPage();
  el(page, "ctrl-start").click();
  await settle(page.window);
  assert.deepEqual(controlPosts(page), [{ path: "/api/v1/control/start", body: {} }]);
});

test("while a users ramp runs, Apply settings sends only rates and the run time, never users", async () => {
  const page = await formPage();
  await emit(page, snapshot("maintain", "hold", { hold_ends_in_secs: 412 }));
  assert.equal(el(page, "set-run-held").textContent, "held 3 min 8 s of 10 min");
  type(page, "ctrl-target", "50");
  assert.equal(label(page), "Edit settings", "the Target row is never pending while running");
  type(page, "set-up", "");
  type(page, "set-down", "4");
  type(page, "set-run", "20");
  assert.equal(label(page), "Edit settings (3 unapplied)");
  assert.equal(el(page, "ctrl-start").textContent, "Start");
  el(page, "set-apply").click();
  await settle(page.window);
  assert.deepEqual(controlPosts(page), [
    {
      path: "/api/v1/control/settings",
      body: { increase_rate: null, decrease_rate: 4, run_time_secs: 1200 },
    },
  ]);
});

test("fields the stage cannot change are read only, never disabled, with the reason", async () => {
  const page = await formPage();
  const ids = ["set-host", "set-up", "set-up-unit", "set-down", "set-down-unit", "set-run", "set-run-unit"];
  const readOnly = () => ids.filter((id) => el(page, id).getAttribute("aria-disabled") === "true");
  assert.deepEqual(readOnly(), []);

  await emit(page, snapshot("increase", "ramp_up", { decrease_rate: null, decrease_time_secs: 60 }));
  assert.deepEqual(readOnly(), ["set-host", "set-up-unit", "set-down", "set-down-unit"]);
  assert.equal(el(page, "set-host").readOnly, true);
  assert.equal(el(page, "set-up").readOnly, false);
  assert.equal(el(page, "set-host-reason").textContent, "Host changes only when idle");
  assert.equal(el(page, "set-down-reason").textContent, "Set by time: 1 min; change it when idle");
  assert.match(el(page, "set-down").getAttribute("aria-describedby"), /set-down-reason/);
  assert.equal(el(page, "set-run-reason").hidden, true);
  assert.doesNotMatch(el(page, "set-run").getAttribute("aria-describedby") || "", /set-run-reason/);
  // A read only select puts back a change.
  choose(page, "set-up-unit", "min");
  assert.equal(el(page, "set-up-unit").value, "rate");
  assert.equal(label(page), "Edit settings");

  for (const [phase, stage] of [["maintain", "test_plan"], ["decrease", "ramping_down"], ["decrease", "stopping"]]) {
    await emit(page, snapshot(phase, stage));
    assert.deepEqual(readOnly(), ids, stage);
    for (const id of ids) assert.equal(el(page, id).disabled, false, id + " is never disabled");
  }
  await emit(page, snapshot("idle", "idle"));
  assert.deepEqual(readOnly(), []);
});

test("with no token the form is read only and says why", async () => {
  const page = await openPage("/", FRESH, TOKEN, { controlEnabled: true, eventStream: true });
  await emit(page, snapshot("idle", "idle"));
  assert.equal(el(page, "set-host").getAttribute("aria-disabled"), "true");
  assert.equal(el(page, "set-reason").textContent, "Enter the control token to change settings");
  assert.equal(el(page, "set-apply").getAttribute("aria-disabled"), "true");
  assert.equal(el(page, "set-discard").getAttribute("aria-disabled"), "true");
});

test("an edit the new stage cannot take is discarded, saying so once", async () => {
  const page = await formPage();
  type(page, "set-host", "http://new.test");
  type(page, "set-run", "30");
  await emit(page, snapshot("increase", "ramp_up"));
  assert.equal(el(page, "ctrl-status").textContent, "Run started elsewhere; unapplied Host edit discarded");
  assert.equal(el(page, "set-host").value, "http://old.test");
  assert.equal(el(page, "set-run").value, "30", "run time still changes while running");
  assert.equal(label(page), "Edit settings (1 unapplied)");

  type(page, "set-up", "9");
  await emit(page, snapshot("maintain", "hold", { hold_ends_in_secs: 100 }));
  assert.equal(label(page), "Edit settings (2 unapplied)");
  await emit(page, snapshot("decrease", "ramping_down"));
  assert.match(el(page, "ctrl-status").textContent, /^Ramp down started; unapplied (Ramp up|Run time) edit discarded$/);
  assert.equal(label(page), "Edit settings");
  assert.equal(el(page, "set-up").value, "5");
});

test("the first host change asks first; Cancel sends nothing and returns focus, Confirm sends", async () => {
  const page = await formPage();
  el(page, "ctrl-settings").open = false;
  // The edit was made with the form open, then it was closed.
  type(page, "set-host", "http://new.test");
  const start = el(page, "ctrl-start");
  start.focus();
  start.click();
  await settle(page.window);
  assert.equal(el(page, "ctrl-settings").open, true, "the form opens to ask");
  assert.equal(el(page, "set-host-confirm").hidden, false);
  assert.equal(el(page, "set-host-confirm-text").textContent, "Load will go to http://new.test");
  assert.equal(page.window.document.activeElement, el(page, "set-host-confirm-ok"));
  assert.deepEqual(controlPosts(page), []);

  el(page, "set-host-confirm-cancel").click();
  await settle(page.window);
  assert.equal(el(page, "set-host-confirm").hidden, true);
  assert.equal(page.window.document.activeElement, start);
  assert.deepEqual(controlPosts(page), []);

  const apply = el(page, "set-apply");
  apply.click();
  await settle(page.window);
  assert.equal(el(page, "set-host-confirm").hidden, false, "asked again until confirmed");
  el(page, "set-host-confirm-ok").click();
  await settle(page.window);
  assert.deepEqual(controlPosts(page), [
    { path: "/api/v1/control/settings", body: { host: "http://new.test" } },
  ]);

  // Once confirmed, a later host change in this page load goes at once.
  await emit(page, snapshot("idle", "idle", { host: "http://new.test", hosts: ["http://new.test/"] }));
  type(page, "set-host", "http://third.test");
  apply.click();
  await settle(page.window);
  assert.equal(el(page, "set-host-confirm").hidden, true);
  assert.equal(controlPosts(page).length, 2);
});

test("a refused setting opens the closed form, marks the field and moves focus to it", async () => {
  const page = await formPage({
    controlAnswer: (command) =>
      command === "settings"
        ? {
            status: 200,
            body: {
              ok: false,
              command,
              error: "conflicting_settings",
              message: "failed to configure increase_rate, increase_time is set and changes only when idle",
              phase: "idle",
              field: "increase_rate",
            },
          }
        : command === "start"
          ? {
              status: 400,
              body: { ok: false, command, error: "invalid_value", message: "run_time_secs must be whole seconds", field: "run_time" },
            }
          : undefined,
  });
  type(page, "set-up", "3");
  el(page, "ctrl-settings").open = false;
  el(page, "set-apply").click();
  await settle(page.window);
  assert.equal(el(page, "ctrl-settings").open, true);
  assert.equal(el(page, "set-up").getAttribute("aria-invalid"), "true");
  assert.equal(page.window.document.activeElement, el(page, "set-up"));
  assert.equal(label(page), "Edit settings (1 unapplied)", "a refused edit stays");
  assert.match(el(page, "ctrl-status").textContent, /^failed to configure increase_rate/);

  type(page, "set-run", "0.5");
  assert.equal(el(page, "set-up").getAttribute("aria-invalid"), "true");
  el(page, "ctrl-settings").open = false;
  el(page, "ctrl-start").click();
  await settle(page.window);
  assert.equal(el(page, "ctrl-settings").open, true);
  assert.equal(el(page, "set-run").getAttribute("aria-invalid"), "true");
  assert.equal(page.window.document.activeElement, el(page, "set-run"));
  assert.match(el(page, "set-run").getAttribute("aria-describedby"), /ctrl-status/);
  type(page, "set-run", "1");
  assert.equal(el(page, "set-run").hasAttribute("aria-invalid"), false);
  assert.doesNotMatch(el(page, "set-run").getAttribute("aria-describedby") || "", /ctrl-status/);
  // Discard edits clears a mark left by an earlier refusal.
  el(page, "set-discard").click();
  await settle(page.window);
  assert.equal(el(page, "set-up").hasAttribute("aria-invalid"), false);
});

test("an accepted reply's settings are followed before the next snapshot", async () => {
  const page = await formPage({
    controlAnswer: (command, body) =>
      command === "settings"
        ? {
            status: 200,
            body: {
              ok: true,
              command,
              message: "increase_rate configured",
              phase: "idle",
              settings: { ...IDLE_SNAPSHOT.settings, ...CONFIGURED, increase_rate: body.increase_rate, stage: "idle" },
            },
          }
        : undefined,
  });
  type(page, "set-up", "7");
  el(page, "set-apply").click();
  await settle(page.window);
  assert.equal(label(page), "Edit settings");
  assert.equal(el(page, "set-up").value, "7");
  assert.match(el(page, "ctrl-summary").textContent, /20 users at 7\/s/);
  // A snapshot from before the reply does not put the old rate back.
  await emit(page, snapshot("idle", "idle"));
  assert.equal(el(page, "set-up").value, "7");
});

test("the form starts open in Idle and closed while running, unless the viewer chose", async () => {
  let page = await formPage();
  assert.equal(el(page, "ctrl-settings").open, true);
  el(page, "ctrl-settings").open = false;
  el(page, "ctrl-settings").dispatchEvent(new page.window.Event("toggle"));
  assert.equal(page.window.localStorage.getItem("goose-dashboard-settings-open"), "closed");

  page = await openPage("/?token=" + TOKEN, FRESH, TOKEN, { controlEnabled: true, eventStream: true });
  await emit(page, snapshot("maintain", "hold", { hold_ends_in_secs: 30 }));
  assert.equal(el(page, "ctrl-settings").open, false);

  page = await formPage({}, { "goose-dashboard-settings-open": "closed" });
  assert.equal(el(page, "ctrl-settings").open, false);
});
