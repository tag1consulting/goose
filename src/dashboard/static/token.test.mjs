// Browser token handling of the dashboard client (app.js), run in jsdom with
// `npm test`. Each page load is a fresh jsdom window; a reload is simulated by
// carrying the previous window's sessionStorage over (as a browser does for a
// reload of the same tab), and a new tab starts with empty storage.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

const here = new URL(".", import.meta.url);
const INDEX_HTML = readFileSync(new URL("index.html", here), "utf8");
const APP_JS = readFileSync(new URL("app.js", here), "utf8");

const ORIGIN = "http://127.0.0.1:5118";
const METRICS_BANNER = "token required for metrics";
const CONTROL_BANNER = "token required for control";

function jsonResponse(status, body) {
  return {
    status,
    ok: status >= 200 && status < 300,
    json: () => Promise.resolve(body),
  };
}

// A stub of the dashboard server, as in src/dashboard.rs: metric GETs need
// `?token=` (or Bearer) equal to `token`; control POSTs need a Bearer token and
// no `?token=`. Options: `controlEnabled` for /api/v1/health, `controlToken`
// (default `token`; the real server has one token, this lets a test produce a
// control 401 alone), and `controlStatus` / `snapshotStatus` to force an
// authorized request to answer with that status instead of 200.
function stubServer(token, options = {}) {
  const {
    controlEnabled = false,
    controlToken = token,
    controlStatus = 200,
    snapshotStatus = 200,
  } = options;
  const requests = [];
  function fetch(input, init) {
    const url = new URL(String(input), ORIGIN);
    const headers = (init && init.headers) || {};
    const method = (init && init.method) || "GET";
    const bearer = headers["Authorization"] || "";
    requests.push({ method, path: url.pathname, url: url.href, bearer });
    if (url.pathname === "/api/v1/health") {
      return Promise.resolve(
        jsonResponse(200, { status: "ok", control_enabled: controlEnabled })
      );
    }
    if (url.pathname.startsWith("/api/v1/control/")) {
      const ok =
        !url.searchParams.has("token") && bearer === "Bearer " + controlToken;
      if (!ok) {
        return Promise.resolve(jsonResponse(401, { error: "unauthorized" }));
      }
      return Promise.resolve(
        controlStatus === 200
          ? jsonResponse(200, { ok: true, phase: "increase" })
          : jsonResponse(controlStatus, { error: "busy", message: "busy" })
      );
    }
    const ok =
      url.searchParams.get("token") === token || bearer === "Bearer " + token;
    if (!ok) return Promise.resolve(jsonResponse(401, {}));
    if (snapshotStatus !== 200) {
      return Promise.resolve(jsonResponse(snapshotStatus, {}));
    }
    return Promise.resolve(jsonResponse(200, { version: 1, phase: "idle" }));
  }
  return { fetch, requests };
}

const openPages = [];

// Browser storage a page starts with. A reload of the same tab keeps both
// stores; a new tab keeps localStorage (shared by the origin) and starts with
// an empty sessionStorage.
const FRESH = { session: {}, local: {} };

function dump(storage) {
  const out = {};
  for (let i = 0; i < storage.length; i++) {
    const key = storage.key(i);
    out[key] = storage.getItem(key);
  }
  return out;
}

function reloadOf(window) {
  return { session: dump(window.sessionStorage), local: dump(window.localStorage) };
}

function newTabOf(window) {
  return { session: {}, local: dump(window.localStorage) };
}

// Load the dashboard at `path` with the browser storage `from` against a stub
// server (`serverToken` and `options` as in stubServer), and let its startup
// requests finish. `blockStorage` makes every sessionStorage access throw, as a
// browser does when site data is blocked. jsdom has no EventSource, so the
// client polls; `eventSource` installs one that fails at once, as a browser's
// does when the server answers 401, so the client takes its SSE error path.
async function openPage(path, from, serverToken, options = {}) {
  const dom = new JSDOM(INDEX_HTML, {
    url: ORIGIN + path,
    runScripts: "outside-only",
  });
  openPages.push(dom);
  const { window } = dom;
  for (const [key, value] of Object.entries(from.session)) {
    window.sessionStorage.setItem(key, value);
  }
  for (const [key, value] of Object.entries(from.local)) {
    window.localStorage.setItem(key, value);
  }
  if (options.blockStorage) {
    Object.defineProperty(window, "sessionStorage", {
      get() {
        throw new window.DOMException("blocked", "SecurityError");
      },
    });
  }
  if (options.eventSource) {
    window.EventSource = class {
      static CLOSED = 2;
      constructor() {
        this.readyState = 2;
        window.setTimeout(() => this.onerror && this.onerror(), 0);
      }
      addEventListener() {}
      close() {}
    };
  }
  const server = stubServer(serverToken, options);
  window.fetch = server.fetch;
  window.eval(APP_JS);
  await settle(window);
  return { window, requests: server.requests };
}

// Let the client's fetch promise chains run to completion: health, then the
// snapshot, then the work each response starts, with room to spare.
async function settle(window) {
  for (let i = 0; i < 5; i++) {
    await new Promise((resolve) => window.setTimeout(resolve, 0));
  }
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
