// The banner and the connection indicator, which the connection, the control
// panel and the snapshot render all write.

import { requireElement } from "./dom";

export type ConnectionMode =
  | "live"
  | "poll"
  | "disconnected"
  | "connecting"
  | "closed";

export type BannerKind = "info" | "warn" | "error";

const bannerEl = requireElement("banner", HTMLElement);
const connectionEl = requireElement("connection", HTMLElement);
const connectionLabel = requireElement("connection-label", HTMLElement);
const announceEl = requireElement("announce", HTMLElement);

// True for the rest of the task that wrote the live region last.
let joining = false;

/**
 * Says a change once through the page's one polite live region. Messages
 * written in one task are joined: a screen reader speaks only the last write
 * of a task, so a second write would drop the first.
 */
export function announce(text: string): void {
  const next = joining ? announceEl.textContent + " " + text : text;
  if (announceEl.textContent !== next) announceEl.textContent = next;
  if (!joining) {
    joining = true;
    queueMicrotask(() => {
      joining = false;
    });
  }
}

let lastMode: ConnectionMode = "connecting";
let lost = false;

export function setConnection(mode: ConnectionMode): void {
  let cls = "disconnected";
  let label = "disconnected";
  if (mode === "live") {
    cls = "live";
    label = "SSE";
  } else if (mode === "poll") {
    cls = "poll";
    label = "poll";
  } else if (mode === "connecting") {
    cls = "disconnected";
    label = "connecting";
  } else if (mode === "closed") {
    cls = "finished";
    label = "finished";
  }
  // Only the edges: a snapshot repeats "live" every second.
  const connected = mode === "live" || mode === "poll";
  if (mode === "disconnected" && (lastMode === "live" || lastMode === "poll")) {
    lost = true;
    announce("Connection lost.");
  } else if (connected && lost) {
    lost = false;
    announce("Connection restored.");
  }
  lastMode = mode;
  connectionEl.className = "connection " + cls;
  connectionLabel.textContent = label;
}

/** The pill alone: polling has started, but no poll has answered yet. */
export function showPolling(): void {
  connectionEl.className = "connection poll";
  connectionLabel.textContent = "poll";
}

export function setBanner(text: string, kind?: BannerKind): void {
  // #banner is a live region: rewriting the same text would announce it again.
  const cls = text ? "banner " + (kind || "info") : "banner hidden";
  if (bannerEl.textContent === text && bannerEl.className === cls) return;
  bannerEl.textContent = text;
  bannerEl.className = cls;
}
