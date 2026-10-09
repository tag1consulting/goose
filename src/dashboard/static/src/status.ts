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
  connectionEl.className = "connection " + cls;
  connectionLabel.textContent = label;
}

export function setBanner(text: string, kind?: BannerKind): void {
  if (!text) {
    bannerEl.textContent = "";
    bannerEl.className = "banner hidden";
    return;
  }
  bannerEl.textContent = text;
  bannerEl.className = "banner " + (kind || "info");
}
