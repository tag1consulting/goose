// The connection to the dashboard server: the access token, the SSE stream,
// the poll fallback and the snapshot fetch, ending at the JSON boundary.

import { setBanner, setConnection, type ConnectionMode } from "./status";

/** Renders one snapshot; the mode says which transport delivered it. */
export type SnapshotHandler = (
  snap: DashboardSnapshot,
  modeLabel?: ConnectionMode | string
) => void;

// Fails to compile when the Rust SNAPSHOT_VERSION changes, so a new wire
// format cannot ship without this client being reviewed against it.
const SNAPSHOT_VERSION: DashboardSnapshotVersion = 1;

// ---------------------------------------------------------------------------
// Auth bootstrap: read ?token= from the page URL, then strip it from the bar.
// The token is kept in sessionStorage, which belongs to this tab and origin
// only, so a reload keeps it and a new tab does not get it. Never
// localStorage: the token must not outlive the tab. A 401 clears the stored
// copy so a stale token is not sent again on the next reload.
// ---------------------------------------------------------------------------

const TOKEN_STORAGE_KEY = "goose-dashboard-token";

let token = "";

let pollTimer: ReturnType<typeof setInterval> | null = null;
let eventSource: EventSource | null = null;
let usingPoll = false;
let authRequired = false;
let authBlocked = false;
let finished = false;
let onSnapshot: SnapshotHandler | null = null;

// sessionStorage access throws when storage is blocked; the token then lives
// in memory only.
function readStoredToken(): string {
  try {
    return window.sessionStorage.getItem(TOKEN_STORAGE_KEY) || "";
  } catch {
    return "";
  }
}

function storeToken(value: string): void {
  try {
    window.sessionStorage.setItem(TOKEN_STORAGE_KEY, value);
  } catch {
    /* ignore */
  }
}

export function forgetStoredToken(): void {
  try {
    window.sessionStorage.removeItem(TOKEN_STORAGE_KEY);
  } catch {
    /* ignore */
  }
}

/** Takes the token from the page URL, or else from sessionStorage. */
export function initToken(): void {
  const params = new URLSearchParams(window.location.search);
  token = params.get("token") || "";
  if (token) {
    params.delete("token");
    const clean =
      window.location.pathname +
      (params.toString() ? "?" + params.toString() : "") +
      window.location.hash;
    try {
      window.history.replaceState({}, "", clean);
    } catch {
      /* ignore */
    }
    storeToken(token);
  } else {
    token = readStoredToken();
  }
}

export function getToken(): string {
  return token;
}

/** True after a metrics 401, until a snapshot renders. */
export function isAuthRequired(): boolean {
  return authRequired;
}

export function clearAuthRequired(): void {
  authRequired = false;
}

function withToken(path: string): string {
  if (!token) return path;
  const sep = path.indexOf("?") >= 0 ? "&" : "?";
  return path + sep + "token=" + encodeURIComponent(token);
}

function snapshotUrl(): string {
  return withToken("/api/v1/snapshot");
}

function eventsUrl(): string {
  return withToken("/api/v1/events");
}

function render(snap: DashboardSnapshot, modeLabel: string): void {
  if (onSnapshot) onSnapshot(snap, modeLabel);
}

function stopPoll(): void {
  if (pollTimer != null) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
}

function showAuthMissing(): void {
  forgetStoredToken();
  authRequired = true;
  authBlocked = true;
  stopPoll();
  usingPoll = false;
  if (eventSource) {
    try {
      eventSource.close();
    } catch {
      /* ignore */
    }
    eventSource = null;
  }
  setConnection("disconnected");
  setBanner(
    "Open this dashboard as http://host:port/?token=… (token required for metrics).",
    "error"
  );
}

// The JSON boundary: refuse a snapshot of another wire format version
// instead of rendering its fields as placeholders.
function checkSnapshot(data: unknown): DashboardSnapshot | null {
  const version =
    typeof data === "object" && data !== null
      ? (data as { version?: unknown }).version
      : undefined;
  if (version !== SNAPSHOT_VERSION) {
    setBanner(
      "This page reads snapshot version " +
        SNAPSHOT_VERSION +
        " but the server sent " +
        String(version) +
        ". Reload the page.",
      "error"
    );
    return null;
  }
  return data as DashboardSnapshot;
}

function fetchSnapshotOnce(
  modeLabel?: string
): Promise<DashboardSnapshot | null> {
  if (authBlocked) return Promise.resolve(null);
  return fetch(snapshotUrl())
    .then((res): Promise<unknown> | null => {
      if (res.status === 401) {
        showAuthMissing();
        return null;
      }
      if (!res.ok) {
        throw new Error("HTTP " + res.status);
      }
      return res.json();
    })
    .then((data) => {
      const snap = data == null ? null : checkSnapshot(data);
      if (snap) {
        render(snap, modeLabel || "poll");
      }
      return snap;
    });
}

function startPollFallback(reason?: string): void {
  if (finished || authBlocked) return;
  if (usingPoll && pollTimer != null) return;
  usingPoll = true;
  if (eventSource) {
    try {
      eventSource.close();
    } catch {
      /* ignore */
    }
    eventSource = null;
  }
  setConnection("poll");
  if (reason && !authRequired) {
    setBanner(reason + " — polling every 2s…", "warn");
  }
  function tick(): void {
    if (authBlocked) {
      stopPoll();
      return;
    }
    fetchSnapshotOnce("poll").catch((err: unknown) => {
      if (authBlocked) return;
      setConnection("disconnected");
      setBanner("Poll failed: " + String(err), "error");
    });
  }
  tick();
  stopPoll();
  if (!authBlocked) {
    pollTimer = setInterval(tick, 2000);
  }
}

/** Opens the SSE stream, falling back to polling; `handler` renders. */
export function startSse(handler: SnapshotHandler): void {
  onSnapshot = handler;
  if (typeof EventSource === "undefined") {
    startPollFallback("SSE unavailable");
    return;
  }

  setConnection("connecting");
  try {
    eventSource = new EventSource(eventsUrl());
  } catch {
    startPollFallback("SSE open failed");
    return;
  }

  let sawSnapshot = false;
  // After the first snapshot, EventSource auto-reconnects; if errors keep
  // stacking without a fresh snapshot, fall back to poll so the UI recovers.
  let sseErrorStreak = 0;
  const SSE_ERROR_FALLBACK_THRESHOLD = 3;

  eventSource.addEventListener("snapshot", (ev: MessageEvent) => {
    try {
      const snap = checkSnapshot(JSON.parse(ev.data));
      if (!snap) return;
      sawSnapshot = true;
      sseErrorStreak = 0;
      usingPoll = false;
      stopPoll();
      render(snap, "live");
    } catch (err: unknown) {
      setBanner("Bad snapshot event: " + String(err), "error");
    }
  });

  eventSource.addEventListener("closed", () => {
    finished = true;
    try {
      if (eventSource) eventSource.close();
    } catch {
      /* ignore */
    }
    eventSource = null;
    stopPoll();
    setConnection("closed");
    setBanner("Load test finished.", "info");
  });

  eventSource.onerror = () => {
    if (finished) {
      return;
    }
    // EventSource reconnects automatically on transient errors; fall back to
    // poll when we never received a snapshot, or after repeated errors once
    // live (server gone, 503 cap, sticky-closed without closed event).
    if (!sawSnapshot) {
      try {
        if (eventSource) eventSource.close();
      } catch {
        /* ignore */
      }
      eventSource = null;
      // Probe once to distinguish auth failure from other SSE issues.
      fetch(snapshotUrl())
        .then((res) => {
          if (res.status === 401) {
            showAuthMissing();
            return;
          }
          startPollFallback("SSE failed");
        })
        .catch(() => {
          startPollFallback("SSE failed");
        });
    } else {
      // Hard close (e.g. 503 at client cap, non-200 reconnect): EventSource
      // fires onerror once and does not reconnect. Fall back immediately.
      if (eventSource && eventSource.readyState === EventSource.CLOSED) {
        eventSource = null;
        startPollFallback("SSE reconnect failed");
        return;
      }
      sseErrorStreak += 1;
      setConnection("disconnected");
      if (sseErrorStreak >= SSE_ERROR_FALLBACK_THRESHOLD) {
        try {
          if (eventSource) eventSource.close();
        } catch {
          /* ignore */
        }
        eventSource = null;
        startPollFallback("SSE reconnect failed");
      }
    }
  };
}
