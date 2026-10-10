// The control panel: Start, Stop, Quit and the users target, sent as
// authenticated POSTs, plus the phase badge whose phase gates them.

import { forgetStoredToken, getToken } from "./connection";
import { optionalElement, requireElement } from "./dom";
import { formatInt } from "./format";
import { setBanner } from "./status";

type ControlStatusKind = "info" | "ok" | "error";

interface ControlResultBody {
  ok?: boolean;
  command?: string;
  error?: string;
  message?: string;
  phase?: string;
  active_users?: number;
  target_users?: number | null;
}

interface HealthBody {
  control_enabled?: boolean;
}

const phaseBadge = requireElement("phase-badge", HTMLElement);
const subtitleEl = optionalElement("subtitle", HTMLElement);
const footerNoteEl = optionalElement("footer-note", HTMLElement);
const controlPanel = optionalElement("control-panel", HTMLElement);
const ctrlStart = optionalElement("ctrl-start", HTMLButtonElement);
const ctrlStop = optionalElement("ctrl-stop", HTMLButtonElement);
const ctrlQuit = optionalElement("ctrl-quit", HTMLButtonElement);
const ctrlActive = optionalElement("ctrl-active", HTMLElement);
const ctrlTarget = optionalElement("ctrl-target", HTMLInputElement);
const ctrlApply = optionalElement("ctrl-apply", HTMLButtonElement);
const ctrlMinus = optionalElement("ctrl-minus", HTMLButtonElement);
const ctrlPlus = optionalElement("ctrl-plus", HTMLButtonElement);
const ctrlStep = optionalElement("ctrl-step", HTMLInputElement);
const ctrlStatus = optionalElement("ctrl-status", HTMLElement);

// Control panel state (normative lastTarget/dirty algorithm)
let controlEnabled = false;
let lastTarget: number | null = null;
let dirty = false;
let step = 10;
let inFlight = false;
let lastSnap: DashboardSnapshot | null = null;
let lastDisplayTarget: number | null = null;
let currentPhase = "idle";
// A cancel is in progress: the server refuses Users until idle.
let stopping = false;
let controlTokenMissing = false;

/** True while control is on and the page has no token. */
export function isControlTokenMissing(): boolean {
  return controlTokenMissing;
}

export function setPhase(phase: string | undefined): void {
  const p = (phase || "idle").toLowerCase();
  const known: Record<string, boolean> = {
    idle: true,
    increase: true,
    maintain: true,
    decrease: true,
    shutdown: true,
  };
  const cls = known[p] ? p : "idle";
  currentPhase = cls;
  phaseBadge.className = "phase-badge phase-" + cls;
  phaseBadge.textContent = p;
}

function setControlStatus(text: string, kind?: ControlStatusKind): void {
  if (!ctrlStatus) return;
  ctrlStatus.textContent = text || "";
  ctrlStatus.className = "control-status" + (kind ? " " + kind : "");
}

function displayTargetFromSnap(snap: DashboardSnapshot | null): number | null {
  if (lastTarget != null) return lastTarget;
  if (!snap) return null;
  // Prefer plan/control target over peak HWM (maximum_users) so the control
  // field matches the KPI "active / target" second number. Ignore 0 (stop /
  // cancel ramp) — control input and server only accept users >= 1.
  if (snap.target_users >= 1) {
    return snap.target_users;
  }
  if (snap.maximum_users >= 1) {
    return snap.maximum_users;
  }
  if (snap.active_users >= 1) {
    return snap.active_users;
  }
  return null;
}

function readStep(): number {
  let n = parseInt(ctrlStep && ctrlStep.value ? ctrlStep.value : "", 10);
  if (!isFinite(n) || n < 1) n = 10;
  step = n;
  return step;
}

function updateControlEnablement(): void {
  if (!controlPanel || !controlEnabled) return;

  const hasToken = !!getToken();
  const phase = currentPhase || "idle";
  const canStart = hasToken && !inFlight && phase === "idle";
  const canStop =
    hasToken && !inFlight && (phase === "increase" || phase === "maintain");
  const canUsers =
    hasToken &&
    !inFlight &&
    !stopping &&
    (phase === "idle" ||
      phase === "increase" ||
      phase === "maintain" ||
      phase === "decrease");

  if (ctrlStart) ctrlStart.disabled = !canStart;
  if (ctrlStop) ctrlStop.disabled = !canStop;
  // Quit only shuts Goose down from idle, so it shows only then.
  if (ctrlQuit) {
    ctrlQuit.hidden = phase !== "idle";
    ctrlQuit.disabled = !hasToken || inFlight;
  }
  if (ctrlApply) ctrlApply.disabled = !canUsers;
  if (ctrlMinus) ctrlMinus.disabled = !canUsers;
  if (ctrlPlus) ctrlPlus.disabled = !canUsers;
  if (ctrlTarget) ctrlTarget.disabled = !hasToken || inFlight;
  if (ctrlStep) ctrlStep.disabled = !hasToken || inFlight;

  if (!hasToken) {
    controlPanel.classList.add("disabled");
  } else {
    controlPanel.classList.remove("disabled");
  }
}

export function updateControlFromSnapshot(snap: DashboardSnapshot): void {
  if (!controlEnabled || !controlPanel) return;
  lastSnap = snap;
  stopping = snap.stopping === true;

  if (ctrlActive) {
    ctrlActive.textContent = formatInt(snap.active_users);
  }

  if (!dirty && ctrlTarget) {
    const dt = displayTargetFromSnap(snap);
    if (dt != null) {
      ctrlTarget.value = String(dt);
      lastDisplayTarget = dt;
    }
  }

  updateControlEnablement();
}

// Control POSTs: Bearer only — never append ?token= (do not use withToken).
function postControl(
  path: string,
  body?: { users: number } | undefined
): Promise<Response> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  const token = getToken();
  if (token) {
    headers["Authorization"] = "Bearer " + token;
  }
  return fetch(path, {
    method: "POST",
    headers: headers,
    body: body === undefined ? "{}" : JSON.stringify(body),
  });
}
function handleControlResponse(
  res: Response,
  appliedUsers?: number
): Promise<ControlResultBody | null> {
  if (res.status === 401) {
    forgetStoredToken();
    setBanner(
      "Open this dashboard as http://host:port/?token=… (token required for control).",
      "error"
    );
    setControlStatus("Unauthorized — reopen with ?token=", "error");
    return Promise.resolve(null);
  }
  if (res.status === 503) {
    // Prefer server message: timeout vs busy vs unavailable. Never blind-retry
    // on timeout — the action may still be applying on the load generator.
    return res.json().then(
      (body: ControlResultBody) => {
        const err = body && body.error ? String(body.error) : "";
        const msg =
          body && body.message
            ? String(body.message)
            : "Control unavailable";
        if (err === "timeout") {
          setControlStatus(
            "Timed out — check phase/users before retrying",
            "error"
          );
        } else if (err === "busy") {
          setControlStatus("Control busy — wait and retry", "error");
        } else {
          setControlStatus(msg, "error");
        }
        return null;
      },
      () => {
        setControlStatus("Control unavailable", "error");
        return null;
      }
    );
  }
  return res.json().then(
    (data: ControlResultBody) => {
      if (!data) {
        setControlStatus("Unexpected control response", "error");
        return null;
      }
      if (data.ok) {
        setControlStatus(
          data.command === "quit"
            ? "Goose is shutting down."
            : data.message || "OK",
          "ok"
        );
        if (data.phase) {
          setPhase(data.phase);
        }
        // Lock the user controls now rather than at the next snapshot.
        if (data.command === "stop") {
          stopping = true;
        }
        if (
          typeof appliedUsers === "number" &&
          isFinite(appliedUsers) &&
          appliedUsers >= 1
        ) {
          lastTarget = appliedUsers;
          dirty = false;
          if (ctrlTarget) {
            ctrlTarget.value = String(appliedUsers);
            lastDisplayTarget = appliedUsers;
          }
        } else if (
          data.target_users != null &&
          typeof data.target_users === "number" &&
          data.target_users >= 1
        ) {
          lastTarget = data.target_users;
          dirty = false;
          if (ctrlTarget) {
            ctrlTarget.value = String(data.target_users);
            lastDisplayTarget = data.target_users;
          }
        }
        updateControlEnablement();
        return data;
      }
      setControlStatus(data.message || "Control rejected", "error");
      if (data.phase) {
        setPhase(data.phase);
      }
      updateControlEnablement();
      return data;
    },
    () => {
      setControlStatus("HTTP " + res.status, "error");
      return null;
    }
  );
}

function runControl(
  path: string,
  body?: { users: number },
  appliedUsers?: number
): void {
  if (inFlight || !getToken()) return;
  inFlight = true;
  updateControlEnablement();
  setControlStatus("Sending…", "info");
  postControl(path, body)
    .then((res) => handleControlResponse(res, appliedUsers))
    .catch((err: unknown) => {
      setControlStatus("Request failed: " + String(err), "error");
    })
    .then(() => {
      inFlight = false;
      updateControlEnablement();
    });
}

function onStartClick(): void {
  runControl("/api/v1/control/start");
}

function onStopClick(): void {
  runControl("/api/v1/control/stop");
}

function onQuitClick(): void {
  runControl("/api/v1/control/quit");
}

function onApplyClick(): void {
  const n = parseInt(ctrlTarget && ctrlTarget.value ? ctrlTarget.value : "", 10);
  if (!isFinite(n) || n < 1) {
    setControlStatus("Target must be an integer ≥ 1", "error");
    return;
  }
  runControl("/api/v1/control/users", { users: n }, n);
}

function onStepClick(delta: number): void {
  readStep();
  let base: number;
  if (dirty) {
    base = parseInt(
      ctrlTarget && ctrlTarget.value ? ctrlTarget.value : "",
      10
    );
  } else {
    const fromSnap = displayTargetFromSnap(lastSnap);
    base = fromSnap == null ? NaN : fromSnap;
  }
  if (!isFinite(base)) {
    base = lastSnap ? lastSnap.active_users : 1;
  }
  const next = Math.max(1, base + delta * step);
  if (ctrlTarget) ctrlTarget.value = String(next);
  dirty = true;
  runControl("/api/v1/control/users", { users: next }, next);
}

function applyControlChrome(): void {
  if (subtitleEl) {
    subtitleEl.textContent = controlEnabled
      ? "Live dashboard · control"
      : "Live dashboard";
  }
  if (footerNoteEl) {
    footerNoteEl.textContent = controlEnabled
      ? "Start, stop, and adjust users from this panel (authenticated). Stop begins a cancel ramp (decrease) before idle, and users cannot be changed until then. Advanced control remains on Controllers."
      : "Control this test via telnet :5116 or WebSocket :5117. This dashboard is read-only.";
  }
}

/** Set up the panel; resolves to whether control is on. */
export function initControlPanel(): Promise<boolean> {
  if (!controlPanel) return Promise.resolve(false);

  const enabled = fetch("/api/v1/health")
    .then((res) => {
      if (!res.ok) throw new Error("HTTP " + res.status);
      return res.json() as Promise<HealthBody>;
    })
    .then((health) => {
      controlEnabled = !!(health && health.control_enabled);
      applyControlChrome();
      if (!controlEnabled) {
        controlPanel.classList.add("hidden");
        return false;
      }
      controlPanel.classList.remove("hidden");
      if (!getToken()) {
        controlTokenMissing = true;
        setBanner(
          "Open this dashboard as http://host:port/?token=… (token required for control).",
          "error"
        );
      }
      updateControlEnablement();
      return true;
    })
    .catch(() => {
      // Health probe failed — leave panel hidden (observe-only fallback).
      controlEnabled = false;
      applyControlChrome();
      controlPanel.classList.add("hidden");
      return false;
    });

  if (ctrlStart) ctrlStart.addEventListener("click", onStartClick);
  if (ctrlStop) ctrlStop.addEventListener("click", onStopClick);
  if (ctrlQuit) ctrlQuit.addEventListener("click", onQuitClick);
  if (ctrlApply) ctrlApply.addEventListener("click", onApplyClick);
  if (ctrlMinus)
    ctrlMinus.addEventListener("click", () => {
      onStepClick(-1);
    });
  if (ctrlPlus)
    ctrlPlus.addEventListener("click", () => {
      onStepClick(1);
    });
  if (ctrlStep) {
    ctrlStep.addEventListener("change", readStep);
    ctrlStep.addEventListener("input", readStep);
  }
  if (ctrlTarget) {
    ctrlTarget.addEventListener("focus", () => {
      dirty = true;
    });
    ctrlTarget.addEventListener("input", () => {
      dirty = true;
    });
    ctrlTarget.addEventListener("blur", () => {
      if (!dirty) return;
      const n = parseInt(ctrlTarget.value, 10);
      if (
        lastDisplayTarget != null &&
        isFinite(n) &&
        n === lastDisplayTarget
      ) {
        dirty = false;
      }
    });
  }
  return enabled;
}
