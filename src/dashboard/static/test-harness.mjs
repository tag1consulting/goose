// Shared jsdom harness for the dashboard client tests (`npm test`): loads
// index.html and the built app.js into a fresh jsdom window per page, against
// a stub of the dashboard server. Run `npm run build` first: app.js is read
// from disk.

import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import { IDLE_SNAPSHOT } from "./idle-snapshot.mjs";

const here = new URL(".", import.meta.url);
const INDEX_HTML = readFileSync(new URL("index.html", here), "utf8");
const APP_JS = readFileSync(new URL("app.js", here), "utf8");

export const ORIGIN = "http://127.0.0.1:5118";

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
// authorized request to answer with that status instead of 200 (`snapshotStatus`
// "reject" makes the snapshot fetch fail as on a network error), and
// `snapshot` for the snapshot an authorized request gets (default
// IDLE_SNAPSHOT).
export function stubServer(token, options = {}) {
  const {
    controlEnabled = false,
    controlToken = token,
    controlStatus = 200,
    snapshotStatus = 200,
    snapshot = IDLE_SNAPSHOT,
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
    if (snapshotStatus === "reject") {
      return Promise.reject(new TypeError("network error"));
    }
    if (snapshotStatus !== 200) {
      return Promise.resolve(jsonResponse(snapshotStatus, {}));
    }
    return Promise.resolve(jsonResponse(200, snapshot));
  }
  return { fetch, requests };
}

export const openPages = [];

// Browser storage a page starts with. A reload of the same tab keeps both
// stores; a new tab keeps localStorage (shared by the origin) and starts with
// an empty sessionStorage.
export const FRESH = { session: {}, local: {} };

export function dump(storage) {
  const out = {};
  for (let i = 0; i < storage.length; i++) {
    const key = storage.key(i);
    out[key] = storage.getItem(key);
  }
  return out;
}

// Load the dashboard at `path` with the browser storage `from` against a stub
// server (`serverToken` and `options` as in stubServer), and let its startup
// requests finish. `blockStorage` makes every sessionStorage access throw, as a
// browser does when site data is blocked. jsdom has no EventSource, so the
// client polls; `eventSource` installs one that fails at once, as a browser's
// does when the server answers 401, so the client takes its SSE error path.
export async function openPage(path, from, serverToken, options = {}) {
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
export async function settle(window) {
  for (let i = 0; i < 5; i++) {
    await new Promise((resolve) => window.setTimeout(resolve, 0));
  }
}
