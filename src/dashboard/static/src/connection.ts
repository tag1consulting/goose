// The connection to the dashboard server: the access token, the SSE stream,
// the poll fallback and the snapshot fetch, ending at the JSON boundary.

import { setBanner, setConnection, showPolling, type ConnectionMode } from "./status";

/**
 * Called once when Goose exits, with the `closed` event's data: the save
 * state as JSON, or `1` from an older Goose.
 */
export type ClosedHandler = (data: string) => void;

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
// The poll in flight, aborted when polling stops or it times out.
let pollAbort: AbortController | null = null;
let pollTicks = 0;
let eventSource: EventSource | null = null;
// A stream opened while polling, until its first snapshot makes it the
// live stream.
let retrySource: EventSource | null = null;
let usingPoll = false;
let authRequired = false;
let authBlocked = false;
let finished = false;
let onSnapshot: SnapshotHandler | null = null;
let onClosed: ClosedHandler | null = null;

/** The heatmap a snapshot fetch says the client holds, if any. */
export interface HeatmapCursor {
  generation: number;
  since: number;
}

let heatmapCursor: () => HeatmapCursor | null = () => null;
let catchingUp = false;

/** Tells snapshot fetches which heatmap the page holds. */
export function setHeatmapCursorSource(source: () => HeatmapCursor | null): void {
  heatmapCursor = source;
}

/** `path` with the heatmap cursor's query parameters, when there is one. */
function withCursor(path: string, cursor: HeatmapCursor | null): string {
  if (cursor === null) return path;
  const sep = path.indexOf("?") >= 0 ? "&" : "?";
  return (
    path +
    sep +
    "heatmap_generation=" +
    cursor.generation +
    "&heatmap_since=" +
    cursor.since
  );
}

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
  return withToken(withCursor("/api/v1/snapshot", heatmapCursor()));
}

function eventsUrl(): string {
  return withToken("/api/v1/events");
}

function render(snap: DashboardSnapshot, modeLabel: string): void {
  if (onSnapshot) onSnapshot(snap, modeLabel);
}

// Longer than the server's own 5s wait for a snapshot.
const POLL_TIMEOUT_MS = 8000;
// A retry of the stream every fifth tick, 10s apart.
const RETRY_EVERY_TICKS = 5;

class HttpError extends Error {
  constructor(readonly status: number) {
    super("HTTP " + status);
  }
}

function stopPoll(): void {
  if (pollTimer != null) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
  if (pollAbort) {
    pollAbort.abort();
    pollAbort = null;
  }
}

function closeStreams(): void {
  for (const source of [eventSource, retrySource]) {
    try {
      if (source) source.close();
    } catch {
      /* ignore */
    }
  }
  eventSource = null;
  retrySource = null;
}

function showAuthMissing(): void {
  forgetStoredToken();
  authRequired = true;
  authBlocked = true;
  stopPoll();
  usingPoll = false;
  closeStreams();
  setConnection("disconnected");
  setBanner(
    "Open this dashboard as http://host:port/?token=… (token required for metrics).",
    "error"
  );
}

/** What `GET /api/v1/series` answered. */
export type SeriesResult =
  | { kind: "ok"; series: RequestSeries }
  | { kind: "missing" }
  | { kind: "failed" };

/**
 * One request's heatmap from `GET /api/v1/series`, only what changed since
 * `cursor` when given, with the token handling of the snapshot: a 401 shows
 * the missing token banner. `missing` is a 404 (no heatmap for that request).
 */
export function fetchRequestSeries(
  method: string,
  name: string,
  cursor: HeatmapCursor | null
): Promise<SeriesResult> {
  if (authBlocked) return Promise.resolve({ kind: "failed" });
  const path = withCursor(
    "/api/v1/series?method=" +
      encodeURIComponent(method) +
      "&name=" +
      encodeURIComponent(name),
    cursor
  );
  return fetch(withToken(path))
    .then((res): Promise<SeriesResult> | SeriesResult => {
      if (res.status === 401) {
        showAuthMissing();
        return { kind: "failed" };
      }
      if (res.status === 404) return { kind: "missing" };
      if (!res.ok) return { kind: "failed" };
      return res
        .json()
        .then((data: unknown): SeriesResult => ({ kind: "ok", series: data as RequestSeries }));
    })
    .catch((): SeriesResult => ({ kind: "failed" }));
}

/**
 * Fetches a snapshot now with the page's heatmap cursor and renders it, when
 * an SSE snapshot's heatmap followed on from another than the one held. At
 * most one such fetch at a time.
 */
export function catchUpHeatmap(): void {
  if (catchingUp || authBlocked || finished) return;
  catchingUp = true;
  fetchSnapshotOnce(usingPoll ? "poll" : "live")
    .catch(() => null)
    .then(() => {
      catchingUp = false;
    });
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

// Renders nothing once Goose has exited, or once `signal` aborted.
function fetchSnapshotOnce(
  modeLabel?: string,
  signal?: AbortSignal
): Promise<DashboardSnapshot | null> {
  if (authBlocked) return Promise.resolve(null);
  return fetch(snapshotUrl(), { signal })
    .then((res): Promise<unknown> | null => {
      if (res.status === 401) {
        showAuthMissing();
        return null;
      }
      if (!res.ok) {
        throw new HttpError(res.status);
      }
      return res.json();
    })
    .then((data) => {
      if (finished || (signal && signal.aborted)) return null;
      const snap = data == null ? null : checkSnapshot(data);
      if (snap) {
        render(snap, modeLabel || "poll");
      }
      return snap;
    });
}

/**
 * Opens a retry of the stream while polling. With `replace` a retry still
 * open, which has had no snapshot, is closed for a fresh one: it may hear
 * only keep alive pings, or hang half open and never error. Without it, a
 * retry already open stays and nothing new opens.
 */
function retrySse(replace = false): void {
  if (finished || authBlocked) return;
  if (retrySource) {
    if (!replace) return;
    // eventSource is null while polling, so this closes only the retry.
    closeStreams();
  }
  openStream(true);
}

function startPollFallback(): void {
  if (finished || authBlocked) return;
  if (usingPoll && pollTimer != null) return;
  usingPoll = true;
  pollTicks = 0;
  closeStreams();
  // The pill alone: setConnection("poll") before the first poll answers
  // would announce a recovery a failing poll then takes back.
  showPolling();
  function tick(): void {
    if (authBlocked || finished || !usingPoll) {
      stopPoll();
      return;
    }
    pollTicks += 1;
    if (pollTicks % RETRY_EVERY_TICKS === 0) retrySse(true);
    // One poll at a time: a slow one holds back the next tick.
    if (pollAbort) return;
    const abort = new AbortController();
    pollAbort = abort;
    const timeout = setTimeout(() => abort.abort(), POLL_TIMEOUT_MS);
    // False once polling stopped: what this poll says is then old news.
    const done = (): boolean => {
      clearTimeout(timeout);
      const current = pollAbort === abort;
      if (current) pollAbort = null;
      return current;
    };
    fetchSnapshotOnce("poll", abort.signal).then(done, (err: unknown) => {
      if (!done()) return;
      setConnection("disconnected");
      // The hub may have closed: the stream then says so at once.
      if (err instanceof HttpError && err.status === 503) retrySse();
    });
  }
  stopPoll();
  tick();
  if (!authBlocked) {
    pollTimer = setInterval(tick, 2000);
  }
}

/**
 * Opens the SSE stream, falling back to polling; `handler` renders, and
 * `closed` is told when Goose exits.
 */
export function startSse(handler: SnapshotHandler, closed?: ClosedHandler): void {
  onSnapshot = handler;
  onClosed = closed || null;
  openStream(false);
}

/**
 * Opens the stream, or with `retry` a retry of it while polling: a retry's
 * first snapshot makes it the live stream and ends polling, its `closed`
 * ends the run, and an error before its first snapshot closes it quietly.
 */
function openStream(retry: boolean): void {
  let source: EventSource;
  try {
    if (typeof EventSource === "undefined") throw new Error("no EventSource");
    if (!retry) setConnection("connecting");
    source = new EventSource(eventsUrl());
  } catch {
    if (!retry) startPollFallback();
    return;
  }
  if (retry) retrySource = source;
  else eventSource = source;
  const current = (): boolean => source === eventSource || source === retrySource;

  let sawSnapshot = false;
  // After the first snapshot, EventSource auto-reconnects; if errors keep
  // stacking without a fresh snapshot, fall back to poll so the UI recovers.
  let sseErrorStreak = 0;
  const SSE_ERROR_FALLBACK_THRESHOLD = 3;

  source.addEventListener("snapshot", (ev: MessageEvent) => {
    if (!current()) return;
    try {
      const snap = checkSnapshot(JSON.parse(ev.data));
      if (!snap) return;
      if (source === retrySource) {
        retrySource = null;
        eventSource = source;
      }
      sawSnapshot = true;
      sseErrorStreak = 0;
      usingPoll = false;
      stopPoll();
      render(snap, "live");
    } catch (err: unknown) {
      setBanner("Bad snapshot event: " + String(err), "error");
    }
  });

  source.addEventListener("closed", (ev: MessageEvent) => {
    if (finished || !current()) return;
    finished = true;
    closeStreams();
    stopPoll();
    setConnection("closed");
    if (onClosed) {
      onClosed(typeof ev.data === "string" ? ev.data : "");
    } else {
      setBanner("Goose has exited.", "info");
    }
  });

  source.onerror = () => {
    if (finished || !current()) {
      return;
    }
    if (source === retrySource) {
      // Still polling: close it, or the browser reconnects it by itself.
      // eventSource is null while polling, so this closes only the retry.
      closeStreams();
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
          startPollFallback();
        })
        .catch(() => {
          startPollFallback();
        });
    } else {
      // Hard close (e.g. 503 at client cap, non-200 reconnect): EventSource
      // fires onerror once and does not reconnect. Fall back immediately.
      if (eventSource && eventSource.readyState === EventSource.CLOSED) {
        eventSource = null;
        startPollFallback();
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
        startPollFallback();
      }
    }
  };
}
