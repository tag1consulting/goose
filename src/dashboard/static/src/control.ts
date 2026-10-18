// The control panel: Start, Stop, Quit and the users target, sent as
// authenticated POSTs, plus the phase badge whose phase gates them and the
// run settings summary.

import { forgetStoredToken, getToken } from "./connection";
import { optionalElement, requireElement } from "./dom";
import {
  addDescribedBy,
  FollowedField,
  removeDescribedBy,
  setReadOnly,
} from "./follow";
import { formatInt } from "./format";
import { settingsSummary } from "./run-settings";
import { announce, setBanner } from "./status";

type ControlStatusKind = "info" | "ok" | "error";

interface ControlResultBody {
  ok?: boolean;
  command?: string;
  error?: string;
  message?: string;
  phase?: string;
  active_users?: number;
  target_users?: number | null;
  /** The setting at fault in a refusal. */
  field?: string | null;
}

/** Where each Controller listens, or null when it is off. */
interface HealthControllers {
  telnet: string | null;
  websocket: string | null;
}

interface HealthBody {
  control_enabled?: boolean;
  controllers?: HealthControllers;
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
const ctrlSummary = optionalElement("ctrl-summary", HTMLElement);
const ctrlTargetHelp = optionalElement("ctrl-target-help", HTMLElement);
const ctrlTargetReason = optionalElement("ctrl-target-reason", HTMLElement);

/** Help under the Target row: which settings a mid run change uses. */
const TARGET_HELP =
  "Mid run changes ramp at the increase rate when adding users and the decrease rate (else the increase rate) when removing them, 1 per second when neither is set; increase and decrease times are not used.";
/** Help under the Target row in Idle with a test plan configured. */
const TARGET_HELP_PLAN = "Applying a user count replaces the test plan with a users ramp.";

// Control panel state
let controlEnabled = false;
// The Target field: follows the server unless being edited.
const targetField = ctrlTarget
  ? new FollowedField(ctrlTarget, "Target", (text) => {
      setControlStatus(text, "info");
    })
  : null;
let step = 10;
let inFlight = false;
let lastSnap: DashboardSnapshot | null = null;
let currentPhase = "idle";
// Where the run is, from the latest snapshot's settings.
let currentStage = "idle";
let seenPhase: string | null = null;
// A cancel is in progress: the server refuses Users until idle.
let stopping = false;
let controlTokenMissing = false;
// The Controllers health reported, or null before it answers or when it did
// not say (a failed probe).
let controllers: HealthControllers | null = null;

/** True while control is on and the page has no token. */
export function isControlTokenMissing(): boolean {
  return controlTokenMissing;
}

/** True once health says control is on. */
export function isControlEnabled(): boolean {
  return controlEnabled;
}

/**
 * Shows the phase. Only snapshots announce a change: a control response can
 * run ahead of a snapshot taken just before the command, and announcing both
 * would flap the phase.
 */
export function setPhase(
  phase: string | undefined,
  fromSnapshot = true
): void {
  const p = (phase || "idle").toLowerCase();
  const known: Record<string, boolean> = {
    idle: true,
    increase: true,
    maintain: true,
    decrease: true,
    shutdown: true,
  };
  const cls = known[p] ? p : "idle";
  // The first phase the page sees is not a change.
  if (fromSnapshot) {
    if (seenPhase !== null && seenPhase !== cls) {
      announce(cls === "idle" ? "Test finished." : "Phase: " + cls + ".");
    }
    seenPhase = cls;
  }
  currentPhase = cls;
  phaseBadge.className = "phase-badge phase-" + cls;
  phaseBadge.textContent = p;
}

function setControlStatus(text: string, kind?: ControlStatusKind): void {
  if (!ctrlStatus) return;
  ctrlStatus.textContent = text || "";
  ctrlStatus.className = "control-status" + (kind ? " " + kind : "");
}

// Mark the Target field invalid and described by the status line, or clear
// both: the status line also carries other results, so it describes the field
// only while the field is invalid. Its other descriptions stay.
function setTargetInvalid(invalid: boolean): void {
  if (!ctrlTarget) return;
  if (invalid) {
    ctrlTarget.setAttribute("aria-invalid", "true");
    addDescribedBy(ctrlTarget, "ctrl-status");
  } else {
    ctrlTarget.removeAttribute("aria-invalid");
    removeDescribedBy(ctrlTarget, "ctrl-status");
  }
}

/**
 * The server's value for the Target field, as text: in Idle the configured
 * users the next Start runs (empty with a test plan); while running the
 * count the current step ramps toward. A 0 target (a cancel's ramp to zero)
 * falls back to the peak, then to the active users; `null` when there is
 * nothing to show.
 */
function serverTarget(snap: DashboardSnapshot): string | null {
  if (snap.phase === "idle") {
    return snap.settings.users == null ? "" : String(snap.settings.users);
  }
  for (const users of [snap.target_users, snap.maximum_users, snap.active_users]) {
    if (users >= 1) return String(users);
  }
  return null;
}

/** Why the Target row cannot change users now, or "" when it can. */
function targetReason(hasToken: boolean): string {
  if (!hasToken) return "Enter the control token to change settings";
  if (currentStage === "test_plan") {
    return "The test plan sets users; edit the plan, or stop and switch mode";
  }
  if (currentStage === "stopping") return "Users change again once the run is idle";
  return "";
}

/** Show `text` as the field's reason, or remove it. */
function setReason(el: HTMLElement | null, input: HTMLInputElement | null, text: string): void {
  if (!el) return;
  el.textContent = text;
  el.hidden = text === "";
  if (!input) return;
  if (text === "") {
    removeDescribedBy(input, el.id);
  } else {
    addDescribedBy(input, el.id);
  }
}

function readStep(): number {
  let n = parseInt(ctrlStep && ctrlStep.value ? ctrlStep.value : "", 10);
  if (!isFinite(n) || n < 1) n = 10;
  step = n;
  return step;
}

// aria-disabled, not disabled: a disabled button drops keyboard focus.
function setAvailable(button: HTMLButtonElement | null, on: boolean): void {
  if (button) button.setAttribute("aria-disabled", on ? "false" : "true");
}

function unavailable(button: HTMLButtonElement | null): boolean {
  return !button || button.getAttribute("aria-disabled") === "true";
}

function updateControlEnablement(): void {
  if (!controlPanel || !controlEnabled) return;

  const hasToken = !!getToken();
  const phase = currentPhase || "idle";
  const canStart = hasToken && !inFlight && phase === "idle";
  const canStop =
    hasToken && !inFlight && (phase === "increase" || phase === "maintain");
  const reason = targetReason(hasToken);
  const canUsers =
    reason === "" &&
    !inFlight &&
    !stopping &&
    (phase === "idle" ||
      phase === "increase" ||
      phase === "maintain" ||
      phase === "decrease");

  setAvailable(ctrlStart, canStart);
  setAvailable(ctrlStop, canStop);
  // Quit only shuts Goose down from idle, so it shows only then. A focused
  // Quit that hides hands focus to Start, the first button in its row.
  if (ctrlQuit) {
    const focused = document.activeElement === ctrlQuit;
    ctrlQuit.hidden = phase !== "idle";
    setAvailable(ctrlQuit, hasToken && !inFlight);
    if (focused && ctrlQuit.hidden && ctrlStart) ctrlStart.focus();
  }
  setAvailable(ctrlApply, canUsers);
  setAvailable(ctrlMinus, canUsers);
  setAvailable(ctrlPlus, canUsers);
  // Read only, never disabled, so focus stays and the reason is read.
  if (ctrlTarget) setReadOnly(ctrlTarget, reason !== "" || inFlight);
  if (ctrlStep) setReadOnly(ctrlStep, reason !== "" || inFlight);
  setReason(ctrlTargetReason, ctrlTarget, reason);
  // The help says how a change applies, so it goes while the reason says
  // none can; out of aria-describedby too, where hidden text is still read.
  if (ctrlTargetHelp) {
    ctrlTargetHelp.hidden = reason !== "";
    if (ctrlTarget) {
      if (reason === "") {
        addDescribedBy(ctrlTarget, ctrlTargetHelp.id);
      } else {
        removeDescribedBy(ctrlTarget, ctrlTargetHelp.id);
      }
    }
  }

  if (!hasToken) {
    controlPanel.classList.add("disabled");
  } else {
    controlPanel.classList.remove("disabled");
  }
}

/** Notes for a stage the Target row cannot change users in. */
function stageReason(stage: string): string {
  if (stage === "test_plan") return "Test plan running";
  if (stage === "stopping") return "Run stopping";
  return "";
}

export function updateControlFromSnapshot(snap: DashboardSnapshot): void {
  if (!controlEnabled || !controlPanel) return;
  lastSnap = snap;
  stopping = snap.stopping === true;
  const settings = snap.settings;
  const before = currentStage;
  currentStage = settings.stage;

  if (ctrlActive) {
    ctrlActive.textContent = formatInt(snap.active_users);
  }
  if (ctrlSummary) {
    ctrlSummary.textContent = settingsSummary(settings, true);
  }

  if (targetField) {
    // An edit the run can no longer take is discarded, said once.
    const reason = stageReason(currentStage);
    if (reason !== "" && reason !== stageReason(before)) {
      targetField.discard(reason);
    }
    targetField.fromServer(serverTarget(snap));
    if (!targetField.pending()) setTargetInvalid(false);
  }
  if (ctrlTargetHelp) {
    ctrlTargetHelp.textContent =
      snap.phase === "idle" && settings.test_plan != null ? TARGET_HELP_PLAN : TARGET_HELP;
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
  // Whatever happens, a users value this tab sent is settled by the reply.
  const settle = (data: ControlResultBody | null): void => {
    if (!targetField || typeof appliedUsers !== "number") return;
    if (data && data.ok) {
      targetField.accepted(String(appliedUsers));
    } else {
      targetField.refused(String(appliedUsers));
    }
  };
  return handleControlBody(res).then((data) => {
    settle(data);
    return data;
  });
}

function handleControlBody(res: Response): Promise<ControlResultBody | null> {
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
          setPhase(data.phase, false);
        }
        // Lock the user controls now rather than at the next snapshot.
        if (data.command === "stop") {
          stopping = true;
        }
        updateControlEnablement();
        return data;
      }
      setControlStatus(data.message || "Control rejected", "error");
      if (data.field === "users") setTargetInvalid(true);
      if (data.phase) {
        setPhase(data.phase, false);
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
  // The status line is about to stop describing an invalid Target.
  setTargetInvalid(false);
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
  if (unavailable(ctrlStart)) return;
  runControl("/api/v1/control/start");
}

function onStopClick(): void {
  if (unavailable(ctrlStop)) return;
  runControl("/api/v1/control/stop");
}

function onQuitClick(): void {
  if (unavailable(ctrlQuit)) return;
  runControl("/api/v1/control/quit");
}

/** Send `users` to `/users`, the field showing it until the reply. */
function sendUsers(users: number): void {
  if (targetField) targetField.sending(String(users));
  runControl("/api/v1/control/users", { users: users }, users);
}

function onApplyClick(): void {
  if (unavailable(ctrlApply)) return;
  const n = parseInt(targetField ? targetField.text() : "", 10);
  if (!isFinite(n) || n < 1) {
    setControlStatus("Target must be an integer ≥ 1", "error");
    setTargetInvalid(true);
    return;
  }
  sendUsers(n);
}

function onStepClick(delta: number): void {
  if (unavailable(delta < 0 ? ctrlMinus : ctrlPlus)) return;
  readStep();
  // From what the field shows: the server's value, or the user's edit.
  let base = parseInt(targetField ? targetField.text() : "", 10);
  if (!isFinite(base)) {
    base = lastSnap ? lastSnap.active_users : 1;
  }
  sendUsers(Math.max(1, base + delta * step));
}

/**
 * The sentence naming the Controllers' addresses after `lead`, or one saying
 * none is running; empty when health did not say. The addresses are bind
 * addresses as configured, such as 0.0.0.0:5116.
 */
function controllersText(lead: string): string {
  if (!controllers) return "";
  const parts: string[] = [];
  if (controllers.telnet) {
    parts.push("the telnet controller listening on " + controllers.telnet);
  }
  if (controllers.websocket) {
    parts.push("the WebSocket controller listening on " + controllers.websocket);
  }
  if (parts.length === 0) return "No telnet or WebSocket controller is running.";
  return lead + parts.join(" or ") + ".";
}

function applyControlChrome(): void {
  if (subtitleEl) {
    subtitleEl.textContent = controlEnabled
      ? "Live dashboard · control"
      : "Live dashboard";
  }
  if (footerNoteEl) {
    const sentences = controlEnabled
      ? [
          "Start, stop, and adjust users from this panel (authenticated). Stop begins a cancel ramp (decrease) before idle, and users cannot be changed until then.",
          controllersText("Advanced control remains on "),
        ]
      : [
          controllersText("Control this test via "),
          "This dashboard is read-only.",
        ];
    footerNoteEl.textContent = sentences.filter((s) => s !== "").join(" ");
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
      controllers = (health && health.controllers) || null;
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
    ctrlTarget.addEventListener("input", () => {
      setTargetInvalid(false);
    });
  }
  return enabled;
}
