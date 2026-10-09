// Browser token handling of the dashboard client (app.js), run in jsdom with
// `npm test`. Each page load is a fresh jsdom window; a reload is simulated by
// carrying the previous window's sessionStorage and localStorage over (as a
// browser does for a reload of the same tab), and a new tab keeps only
// localStorage.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  FRESH,
  ORIGIN,
  dump,
  openPage,
  openPages,
  stubServer,
  settle,
} from "./test-harness.mjs";

const METRICS_BANNER = "token required for metrics";
const CONTROL_BANNER = "token required for control";

// The storage a reload of this tab starts with, after the events a browser
// fires on the page being unloaded.
function reloadOf(window) {
  for (const type of ["beforeunload", "pagehide", "unload"]) {
    window.dispatchEvent(new window.Event(type));
  }
  return { session: dump(window.sessionStorage), local: dump(window.localStorage) };
}

function newTabOf(window) {
  return { session: {}, local: dump(window.localStorage) };
}

// The `token` query parameter of every snapshot request a page made.
function snapshotTokens(requests) {
  const tokens = requests
    .filter((r) => r.path === "/api/v1/snapshot")
    .map((r) => new URL(r.url).searchParams.get("token"));
  assert.ok(tokens.length > 0, "the page asked for a snapshot");
  return tokens;
}

function bannerText(window) {
  const el = window.document.getElementById("banner");
  return el.classList.contains("hidden") ? "" : el.textContent;
}

test.afterEach(() => {
  while (openPages.length) openPages.pop().window.close();
});

test("the token survives a reload of the same tab", async () => {
  const first = await openPage("/?token=s3cret", FRESH, "s3cret");
  assert.equal(first.window.location.href, ORIGIN + "/");
  assert.equal(bannerText(first.window), "");

  const reload = await openPage("/", reloadOf(first.window), "s3cret");
  for (const t of snapshotTokens(reload.requests)) {
    assert.equal(t, "s3cret");
  }
  assert.ok(
    !bannerText(reload.window).includes(METRICS_BANNER),
    "no 401 banner after the reload: " + bannerText(reload.window)
  );

  const second = await openPage("/", reloadOf(reload.window), "s3cret");
  for (const t of snapshotTokens(second.requests)) {
    assert.equal(t, "s3cret");
  }
  assert.deepEqual(Object.values(dump(second.window.sessionStorage)), ["s3cret"]);
});

test("a new tab does not get the token", async () => {
  const first = await openPage("/?token=s3cret", FRESH, "s3cret");
  assert.equal(bannerText(first.window), "");

  const newTab = await openPage("/", newTabOf(first.window), "s3cret");
  for (const t of snapshotTokens(newTab.requests)) {
    assert.equal(t, null);
  }
  assert.ok(bannerText(newTab.window).includes(METRICS_BANNER));
});

test("the token is kept only in sessionStorage", async () => {
  const first = await openPage("/?token=s3cret", FRESH, "s3cret");
  assert.deepEqual(dump(first.window.localStorage), {});
  assert.deepEqual(Object.values(dump(first.window.sessionStorage)), ["s3cret"]);
});

test("a token in the URL replaces the stored one", async () => {
  const first = await openPage("/?token=old", FRESH, "old");
  const next = await openPage("/?token=new", reloadOf(first.window), "new");
  assert.deepEqual(Object.values(dump(next.window.sessionStorage)), ["new"]);
  for (const t of snapshotTokens(next.requests)) {
    assert.equal(t, "new");
  }
});

test("a metrics 401 clears the stored token so a reload does not resend it", async () => {
  const first = await openPage("/?token=s3cret", FRESH, "s3cret");

  // Goose restarted with another token: the stored one is now stale.
  const stale = await openPage("/", reloadOf(first.window), "rotated");
  assert.equal(snapshotTokens(stale.requests)[0], "s3cret");
  assert.ok(bannerText(stale.window).includes(METRICS_BANNER));
  assert.deepEqual(dump(stale.window.sessionStorage), {});

  const again = await openPage("/", reloadOf(stale.window), "rotated");
  for (const t of snapshotTokens(again.requests)) {
    assert.equal(t, null);
  }
});

test("a 401 on the SSE path clears the stored token", async () => {
  const first = await openPage("/?token=s3cret", FRESH, "s3cret");
  const stale = await openPage("/", reloadOf(first.window), "rotated", {
    eventSource: true,
  });
  assert.equal(snapshotTokens(stale.requests)[0], "s3cret");
  assert.ok(bannerText(stale.window).includes(METRICS_BANNER));
  assert.deepEqual(dump(stale.window.sessionStorage), {});
});

for (const snapshotStatus of [200, "reject"]) {
  test(`an SSE failure without a 401 keeps the stored token (probe ${snapshotStatus})`, async () => {
    const page = await openPage("/?token=s3cret", FRESH, "s3cret", {
      eventSource: true,
      snapshotStatus,
    });
    assert.equal(snapshotTokens(page.requests)[0], "s3cret");
    assert.deepEqual(Object.values(dump(page.window.sessionStorage)), ["s3cret"]);

    const reload = await openPage("/", reloadOf(page.window), "s3cret");
    for (const t of snapshotTokens(reload.requests)) {
      assert.equal(t, "s3cret");
    }
  });
}

test("a control 401 clears the stored token", async () => {
  const page = await openPage("/?token=s3cret", FRESH, "s3cret", {
    controlEnabled: true,
  });
  assert.ok(!bannerText(page.window).includes(CONTROL_BANNER));
  assert.deepEqual(Object.values(dump(page.window.sessionStorage)), ["s3cret"]);

  // Metrics still accept the token, so only the control 401 can clear it.
  const start = page.window.document.getElementById("ctrl-start");
  assert.equal(start.disabled, false, "Start is enabled with a token");
  const server = stubServer("s3cret", {
    controlEnabled: true,
    controlToken: "other",
  });
  page.window.fetch = server.fetch;
  start.click();
  await settle(page.window);
  const posts = server.requests.filter((r) => r.method === "POST");
  assert.deepEqual(
    posts.map((r) => [r.path, r.bearer]),
    [["/api/v1/control/start", "Bearer s3cret"]]
  );
  assert.deepEqual(dump(page.window.sessionStorage), {});
});

for (const status of [200, 503]) {
  test(`a control ${status} keeps the stored token`, async () => {
    const page = await openPage("/?token=s3cret", FRESH, "s3cret", {
      controlEnabled: true,
    });
    const server = stubServer("s3cret", {
      controlEnabled: true,
      controlStatus: status,
    });
    page.window.fetch = server.fetch;
    page.window.document.getElementById("ctrl-start").click();
    await settle(page.window);
    const posts = server.requests.filter((r) => r.method === "POST");
    assert.deepEqual(
      posts.map((r) => [r.path, r.bearer]),
      [["/api/v1/control/start", "Bearer s3cret"]]
    );
    assert.deepEqual(Object.values(dump(page.window.sessionStorage)), ["s3cret"]);

    const reload = await openPage("/", reloadOf(page.window), "s3cret");
    for (const t of snapshotTokens(reload.requests)) {
      assert.equal(t, "s3cret");
    }
  });
}

test("a metrics error other than 401 keeps the stored token", async () => {
  const first = await openPage("/?token=s3cret", FRESH, "s3cret", {
    snapshotStatus: 500,
  });
  assert.equal(snapshotTokens(first.requests)[0], "s3cret");
  assert.deepEqual(Object.values(dump(first.window.sessionStorage)), ["s3cret"]);
});

test("blocked storage leaves the URL token working for this page", async () => {
  const page = await openPage("/?token=s3cret", FRESH, "s3cret", {
    blockStorage: true,
  });
  assert.equal(page.window.location.href, ORIGIN + "/");
  for (const t of snapshotTokens(page.requests)) {
    assert.equal(t, "s3cret");
  }
  assert.equal(bannerText(page.window), "");
});

test("blocked storage on a bare URL still shows the 401 banner", async () => {
  const page = await openPage("/", FRESH, "s3cret", { blockStorage: true });
  for (const t of snapshotTokens(page.requests)) {
    assert.equal(t, null);
  }
  assert.ok(bannerText(page.window).includes(METRICS_BANNER));
});
