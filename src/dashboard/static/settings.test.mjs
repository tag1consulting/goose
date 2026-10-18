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

// A server that accepts every settings body, answering with the configured
// settings and the body applied, each time in its canonical form, and its
// snapshots in `stage` until told otherwise: `page.server.settings` is
// what it has.
async function acceptingPage(stage = "idle", settings = {}, options = {}) {
  const server = { settings: { ...CONFIGURED, ...settings } };
  const page = await formPage({
    controlAnswer: (command, body) => {
      if (command !== "settings" && command !== "start") return undefined;
      const applied = { ...server.settings };
      for (const [key, value] of Object.entries(body)) {
        if (key !== "users") applied[key] = value;
      }
      server.settings = applied;
      if (options.timeout) {
        return { status: 503, body: { ok: false, command, error: "timeout", message: "timed out" } };
      }
      return {
        status: 200,
        body: {
          ok: true,
          command,
          message: "settings configured",
          phase: stage === "idle" ? "idle" : "maintain",
          settings: { ...IDLE_SNAPSHOT.settings, ...applied, stage },
        },
      };
    },
  });
  page.server = server;
  if (stage !== "idle" || Object.keys(settings).length) {
    await emit(page, snapshot(stage === "idle" ? "idle" : "maintain", stage, server.settings));
  }
  return page;
}

// The three quantities with a number and a unit, each configured as 600
// seconds, which the form shows as 10 min.
const QUANTITIES = [
  { label: "Ramp up", id: "set-up", key: "increase_time_secs", settings: { increase_rate: null, increase_time_secs: 600 } },
  { label: "Ramp down", id: "set-down", key: "decrease_time_secs", settings: { decrease_time_secs: 600 } },
  { label: "Run time", id: "set-run", key: "run_time_secs", settings: { run_time_secs: 600 } },
];

function shown(page, id) {
  return el(page, id).value + " " + el(page, id + "-unit").value;
}

for (const q of QUANTITIES) {
  test(q.label + ": an applied 120 s is acknowledged as the server's 2 min, after stale and repeated snapshots", async () => {
    const page = await acceptingPage("idle", q.settings);
    assert.equal(shown(page, q.id), "10 min");
    type(page, q.id, "120");
    choose(page, q.id + "-unit", "s");
    assert.equal(label(page), "Edit settings (1 unapplied)");
    el(page, "set-apply").click();
    await settle(page.window);
    assert.equal(controlPosts(page).at(-1).body[q.key], 120);
    assert.equal(label(page), "Edit settings");
    // A snapshot from before the apply reached the server changes nothing.
    await emit(page, snapshot("idle", "idle", q.settings));
    assert.equal(shown(page, q.id), "120 s");
    assert.equal(label(page), "Edit settings");
    for (let i = 0; i < 2; i++) {
      await emit(page, snapshot("idle", "idle", page.server.settings));
      assert.equal(shown(page, q.id), "2 min");
      assert.equal(label(page), "Edit settings");
    }
    assert.doesNotMatch(el(page, "ctrl-status").textContent, /changed elsewhere/);

    // A later edit is its own.
    type(page, q.id, "3");
    choose(page, q.id + "-unit", "h");
    assert.equal(label(page), "Edit settings (1 unapplied)");
    el(page, "set-apply").click();
    await settle(page.window);
    assert.equal(controlPosts(page).at(-1).body[q.key], 10800);
    await emit(page, snapshot("idle", "idle", page.server.settings));
    assert.equal(shown(page, q.id), "3 h");
    assert.equal(label(page), "Edit settings");
  });

  test(q.label + ": an apply that timed out but landed is acknowledged by the snapshot", async () => {
    const page = await acceptingPage("idle", q.settings, { timeout: true });
    type(page, q.id, "120");
    choose(page, q.id + "-unit", "s");
    el(page, "set-apply").click();
    await settle(page.window);
    assert.equal(label(page), "Edit settings (1 unapplied)", "the edit stays while unconfirmed");
    await emit(page, snapshot("idle", "idle", page.server.settings));
    assert.equal(shown(page, q.id), "2 min");
    assert.equal(label(page), "Edit settings");
    assert.doesNotMatch(el(page, "ctrl-status").textContent, /changed elsewhere/);
  });

  test(q.label + ": a change elsewhere to another unit replaces a pending edit and says so", async () => {
    const page = await acceptingPage("idle", q.settings);
    type(page, q.id, "20");
    assert.equal(shown(page, q.id), "20 min");
    assert.equal(label(page), "Edit settings (1 unapplied)");
    await emit(page, snapshot("idle", "idle", { ...q.settings, [q.key]: 36000 }));
    assert.equal(shown(page, q.id), "10 h");
    assert.equal(label(page), "Edit settings");
    assert.equal(el(page, "ctrl-status").textContent, q.label + " changed elsewhere to 10 h");
  });

  test(q.label + ": Discard edits puts back the number and its unit", async () => {
    const page = await acceptingPage("idle", q.settings);
    type(page, q.id, "120");
    choose(page, q.id + "-unit", "s");
    el(page, "set-discard").click();
    await settle(page.window);
    assert.equal(shown(page, q.id), "10 min");
    assert.equal(label(page), "Edit settings");
    await emit(page, snapshot("idle", "idle", q.settings));
    assert.equal(shown(page, q.id), "10 min");
    assert.equal(label(page), "Edit settings");
  });
}

test("a run time applied while running is acknowledged in the server's unit", async () => {
  const page = await acceptingPage("hold", { run_time_secs: 600, hold_ends_in_secs: 300 });
  type(page, "set-run", "120");
  choose(page, "set-run-unit", "s");
  el(page, "set-apply").click();
  await settle(page.window);
  assert.deepEqual(controlPosts(page).at(-1).body, { run_time_secs: 120 });
  await emit(page, snapshot("maintain", "hold", { ...page.server.settings, hold_ends_in_secs: 0 }));
  assert.equal(shown(page, "set-run"), "2 min");
  assert.equal(label(page), "Edit settings");
  assert.doesNotMatch(el(page, "ctrl-status").textContent, /changed elsewhere/);
});

test("a host typed with spaces, and a rate the server writes another way, are not changes from elsewhere", async () => {
  const page = await acceptingPage();
  type(page, "set-host", "  http://new.test ");
  type(page, "set-up", "2.50");
  type(page, "set-down", "0.123456789");
  el(page, "set-apply").click();
  await settle(page.window);
  el(page, "set-host-confirm-ok").click();
  await settle(page.window);
  const body = controlPosts(page).at(-1).body;
  assert.equal(body.host, "http://new.test");
  // The server reads a rate as an f32 and writes it back as one.
  const server = { ...page.server.settings, increase_rate: 2.5, decrease_rate: 0.12345679 };
  for (let i = 0; i < 2; i++) {
    await emit(page, snapshot("idle", "idle", server));
    assert.equal(label(page), "Edit settings");
    assert.doesNotMatch(el(page, "ctrl-status").textContent, /changed elsewhere/);
  }
  assert.equal(el(page, "set-host").value, "http://new.test");
  assert.equal(el(page, "set-up").value, "2.5");
});

// The destination the open confirmation shows, or null while it is closed.
function confirmationShows(page) {
  if (el(page, "set-host-confirm").hidden) return null;
  return el(page, "set-host-confirm-text").textContent.replace(/^Load will go to /, "");
}

// Press Confirm: every request it sends goes where the confirmation said.
async function confirmAsShown(page) {
  const showing = confirmationShows(page);
  const before = controlPosts(page).length;
  el(page, "set-host-confirm-ok").click();
  await settle(page.window);
  const sent = controlPosts(page).slice(before);
  for (const post of sent) {
    assert.notEqual(showing, null, "nothing is sent without a visible confirmation");
    assert.equal(post.body.host, showing, "the request goes where the confirmation said");
  }
  return sent;
}

test("a host edited while its confirmation is open is confirmed as edited, never as first shown", async () => {
  const page = await acceptingPage();
  type(page, "set-host", "http://a.test");
  el(page, "set-apply").click();
  await settle(page.window);
  assert.equal(confirmationShows(page), "http://a.test");
  type(page, "set-host", "http://b.test");
  await settle(page.window);
  let sent = await confirmAsShown(page);
  if (sent.length === 0) {
    // The confirmation was refreshed or closed: confirming again sends B.
    if (confirmationShows(page) === null) {
      el(page, "set-apply").click();
      await settle(page.window);
    }
    assert.equal(confirmationShows(page), "http://b.test");
    sent = await confirmAsShown(page);
  }
  assert.equal(sent.length, 1);
  assert.equal(sent[0].body.host, "http://b.test");
});

test("a host confirmation for Start with changes is not sent once the run has started elsewhere", async () => {
  const page = await acceptingPage();
  type(page, "set-host", "http://a.test");
  el(page, "ctrl-start").click();
  await settle(page.window);
  assert.equal(confirmationShows(page), "http://a.test");
  await emit(page, snapshot("increase", "ramp_up"));
  if (confirmationShows(page) !== null) {
    el(page, "set-host-confirm-ok").click();
    await settle(page.window);
  }
  assert.deepEqual(controlPosts(page), [], "nothing is sent");
  assert.equal(confirmationShows(page), null);
});

test("a host confirmation whose fallback the server already has closes, sending nothing", async () => {
  const page = await acceptingPage();
  type(page, "set-host", "");
  el(page, "set-apply").click();
  await settle(page.window);
  assert.equal(confirmationShows(page), "each scenario's own host");
  const confirm = el(page, "set-host-confirm-ok");
  assert.equal(page.window.document.activeElement, confirm);
  // Another client clears the host: the edit is now the server's value.
  await emit(page, snapshot("idle", "idle", { host: null, hosts: ["http://scenario.test/"] }));
  if (confirmationShows(page) !== null) {
    await confirmAsShown(page);
  }
  assert.equal(confirmationShows(page), null);
  assert.deepEqual(
    controlPosts(page).filter((post) => "host" in post.body),
    [],
    "no host is sent"
  );
  assert.notEqual(page.window.document.activeElement, confirm, "focus leaves the hidden Confirm");
});

test("on an empty ramp, a unit chosen before the number stays chosen and the number goes as a time", async () => {
  const page = await acceptingPage("idle", { decrease_rate: null, decrease_time_secs: null });
  assert.equal(shown(page, "set-down"), " rate");
  choose(page, "set-down-unit", "min");
  el(page, "set-down-unit").dispatchEvent(new page.window.Event("blur"));
  await emit(page, snapshot("idle", "idle", page.server.settings));
  assert.equal(shown(page, "set-down"), " min", "a snapshot does not put the unit back");
  type(page, "set-down", "5");
  await emit(page, snapshot("idle", "idle", page.server.settings));
  assert.equal(label(page), "Edit settings (1 unapplied)");
  el(page, "set-apply").click();
  await settle(page.window);
  assert.deepEqual(controlPosts(page).at(-1).body, { decrease_time_secs: 300, decrease_rate: null });
  await emit(page, snapshot("idle", "idle", page.server.settings));
  assert.equal(shown(page, "set-down"), "5 min");
  assert.equal(label(page), "Edit settings");
});

test("a time cleared by emptying its number is acknowledged without a change from elsewhere", async () => {
  const page = await acceptingPage("idle", { decrease_time_secs: 600 });
  type(page, "set-down", "");
  el(page, "set-apply").click();
  await settle(page.window);
  assert.deepEqual(controlPosts(page).at(-1).body, { decrease_rate: null, decrease_time_secs: null });
  for (const settings of [{ decrease_time_secs: 600 }, page.server.settings, page.server.settings]) {
    await emit(page, snapshot("idle", "idle", settings));
    assert.equal(label(page), "Edit settings");
    assert.doesNotMatch(el(page, "ctrl-status").textContent, /changed elsewhere/);
  }
  assert.equal(el(page, "set-down").value, "");
});

test("number text a browser can not read is refused on the page, never sent as a clear", async () => {
  const page = await acceptingPage();
  const input = el(page, "set-up");
  // A browser reports bad number text as an empty value with badInput set.
  Object.defineProperty(input, "validity", { value: { badInput: true, valid: false } });
  type(page, "set-up", "");
  assert.equal(label(page), "Edit settings (1 unapplied)");
  el(page, "ctrl-settings").open = false;
  el(page, "set-apply").click();
  await settle(page.window);
  assert.deepEqual(controlPosts(page), [], "nothing is sent");
  assert.equal(el(page, "ctrl-status").textContent, "Ramp up must be a number");
  assert.equal(el(page, "ctrl-settings").open, true);
  assert.equal(input.getAttribute("aria-invalid"), "true");
  assert.equal(page.window.document.activeElement, input);
  el(page, "ctrl-start").click();
  await settle(page.window);
  assert.deepEqual(controlPosts(page), [], "nor with Start");
});

test("a run time edit with no run time on the server keeps its unit across snapshots", async () => {
  const page = await acceptingPage("idle", { run_time_secs: null });
  assert.equal(shown(page, "set-run"), " min");
  type(page, "set-run", "5");
  choose(page, "set-run-unit", "s");
  await emit(page, snapshot("idle", "idle", page.server.settings));
  assert.equal(shown(page, "set-run"), "5 s");
  assert.equal(label(page), "Edit settings (1 unapplied)");
  choose(page, "set-run-unit", "h");
  type(page, "set-run", "2");
  await emit(page, snapshot("idle", "idle", page.server.settings));
  assert.equal(shown(page, "set-run"), "2 h");
  assert.equal(label(page), "Edit settings (1 unapplied)");
  assert.doesNotMatch(el(page, "ctrl-status").textContent, /changed elsewhere/);
  el(page, "set-apply").click();
  await settle(page.window);
  assert.deepEqual(controlPosts(page).at(-1).body, { run_time_secs: 7200 });
});

test("a number made unreadable while the host confirmation is open is refused on Confirm", async () => {
  const page = await acceptingPage();
  type(page, "set-host", "http://new.test");
  el(page, "set-apply").click();
  await settle(page.window);
  assert.equal(confirmationShows(page), "http://new.test");
  const input = el(page, "set-up");
  const validity = { badInput: true, valid: false };
  Object.defineProperty(input, "validity", { value: validity });
  type(page, "set-up", "");
  el(page, "set-host-confirm-ok").click();
  await settle(page.window);
  assert.deepEqual(controlPosts(page), [], "nothing is sent");
  assert.equal(el(page, "ctrl-status").textContent, "Ramp up must be a number");
  assert.equal(page.window.document.activeElement, input);
  assert.equal(confirmationShows(page), null);
  // The host was never sent, so the next host change still asks.
  validity.badInput = false;
  validity.valid = true;
  type(page, "set-up", "5");
  el(page, "set-apply").click();
  await settle(page.window);
  assert.equal(confirmationShows(page), "http://new.test");
});

test("an invalid Target is refused before the host confirmation, which then still asks", async () => {
  const page = await acceptingPage();
  type(page, "set-host", "http://new.test");
  type(page, "ctrl-target", "0");
  el(page, "ctrl-start").click();
  await settle(page.window);
  assert.equal(confirmationShows(page), null, "nothing to confirm while the Target is refused");
  assert.equal(el(page, "ctrl-status").textContent, "Target must be an integer ≥ 1");
  assert.deepEqual(controlPosts(page), []);
  // Emptied while the confirmation is open: refused on Confirm.
  type(page, "ctrl-target", "5");
  el(page, "ctrl-start").click();
  await settle(page.window);
  assert.equal(confirmationShows(page), "http://new.test");
  type(page, "ctrl-target", "");
  el(page, "set-host-confirm-ok").click();
  await settle(page.window);
  assert.deepEqual(controlPosts(page), []);
  assert.equal(el(page, "ctrl-status").textContent, "Target must be an integer ≥ 1");
  // Nothing went out, so the next host change still asks.
  type(page, "ctrl-target", "5");
  type(page, "set-host", "http://other.test");
  el(page, "ctrl-start").click();
  await settle(page.window);
  assert.equal(confirmationShows(page), "http://other.test");
  const sent = await confirmAsShown(page);
  assert.equal(sent.length, 1);
  assert.deepEqual(sent[0].body, { host: "http://other.test", users: 5 });
});

test("unreadable text in a row the server has none for stays an edit through blur and snapshots, refused on the page", async () => {
  const page = await acceptingPage("idle", { decrease_rate: null, decrease_time_secs: null });
  const input = el(page, "set-down");
  Object.defineProperty(input, "validity", { value: { badInput: true, valid: false } });
  type(page, "set-down", "");
  input.dispatchEvent(new page.window.Event("blur"));
  assert.equal(label(page), "Edit settings (1 unapplied)");
  // Nor does a snapshot that has none take it as done.
  await emit(page, snapshot("idle", "idle", page.server.settings));
  assert.equal(label(page), "Edit settings (1 unapplied)");
  el(page, "set-apply").click();
  await settle(page.window);
  assert.deepEqual(controlPosts(page), []);
  assert.equal(el(page, "ctrl-status").textContent, "Ramp down must be a number");
});

test("Discard edits clears unreadable text from a row the server has none for", async () => {
  const page = await acceptingPage("idle", { decrease_rate: null, decrease_time_secs: null });
  const input = el(page, "set-down");
  // A browser shows the text, reads it as "", and clears it only when written.
  const writes = [];
  Object.defineProperty(input, "validity", { value: { badInput: true, valid: false } });
  Object.defineProperty(input, "value", { get: () => "", set: (v) => writes.push(v), configurable: true });
  input.dispatchEvent(new page.window.Event("input"));
  assert.equal(label(page), "Edit settings (1 unapplied)");
  el(page, "set-discard").click();
  await settle(page.window);
  assert.equal(label(page), "Edit settings");
  assert.deepEqual(writes, [""], "the field is written, clearing the text");
});

test("a Start with changes that timed out but landed is not reported as started elsewhere", async () => {
  const page = await acceptingPage("idle", {}, { timeout: true });
  type(page, "set-host", "http://new.test");
  type(page, "set-up", "5");
  choose(page, "set-up-unit", "min");
  el(page, "ctrl-start").click();
  await settle(page.window);
  await confirmAsShown(page);
  assert.equal(controlPosts(page).length, 1);
  await emit(page, snapshot("increase", "ramp_up", page.server.settings));
  assert.doesNotMatch(el(page, "ctrl-status").textContent, /elsewhere|discarded/);
  assert.equal(label(page), "Edit settings");
  assert.equal(el(page, "set-host").value, "http://new.test");
  assert.equal(shown(page, "set-up"), "5 min");
});

test("unreadable text fixed after another client's change is said to be overtaken, never lost silently", async () => {
  const page = await acceptingPage("idle", { increase_rate: 5 });
  const input = el(page, "set-up");
  const validity = { badInput: true, valid: false };
  Object.defineProperty(input, "validity", { value: validity });
  type(page, "set-up", "");
  await emit(page, snapshot("idle", "idle", { increase_rate: 7 }));
  validity.badInput = false;
  validity.valid = true;
  type(page, "set-up", "5");
  input.dispatchEvent(new page.window.Event("blur"));
  await emit(page, snapshot("idle", "idle", { increase_rate: 7 }));
  // Overtaken while it could not be read: said so, never lost silently.
  assert.equal(el(page, "set-up").value, "7");
  assert.equal(el(page, "ctrl-status").textContent, "Ramp up changed elsewhere to 7 users per second");
});
