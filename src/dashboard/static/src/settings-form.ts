// The run settings form under the summary line: Host, Ramp up, Ramp down
// and Run time, each following the server unless being edited. Unapplied
// edits are pending: they go out with Apply settings, or with Start in Idle,
// as one settings body the server checks as a whole. Which fields can change
// comes from `settings.stage`, never from `phase`.

import { optionalElement } from "./dom";
import {
  addDescribedBy,
  FollowedField,
  removeDescribedBy,
  setReadOnly,
} from "./follow";
import { settingsHostsText, spanText } from "./run-settings";

/** The settings body of `/api/v1/control/settings` and a Start. */
export interface SettingsBody {
  host?: string | null;
  users?: number;
  increase_rate?: number | null;
  increase_time_secs?: number | null;
  decrease_rate?: number | null;
  decrease_time_secs?: number | null;
  run_time_secs?: number | null;
}

/** A ramp's unit: users per second, or a total time in seconds, minutes or hours. */
type Unit = "rate" | "s" | "min" | "h";

const SECS: Record<string, number> = { s: 1, min: 60, h: 3600 };

/** The open or closed state of the form, remembered per viewer. */
const OPEN_KEY = "goose-dashboard-settings-open";

/** `secs` as a number and the largest unit that divides it evenly. */
export function timeParts(secs: number): [string, Unit] {
  if (secs > 0 && secs % 3600 === 0) return [String(secs / 3600), "h"];
  if (secs > 0 && secs % 60 === 0) return [String(secs / 60), "min"];
  return [String(secs), "s"];
}

/** `n` of `unit` in seconds, whole when it is within rounding of a whole number. */
function seconds(n: number, unit: string): number {
  const secs = n * (SECS[unit] || 1);
  return Math.abs(secs - Math.round(secs)) < 1e-6 ? Math.round(secs) : secs;
}

/** The number a field shows, or null when it is empty or not a number. */
function numberOf(text: string): number | null {
  if (text.trim() === "") return null;
  const n = Number(text);
  return isFinite(n) ? n : NaN;
}

/** Elements of one form row: its number, its unit select, and its texts. */
interface RowElements {
  number: HTMLInputElement;
  unit: HTMLSelectElement;
  reason: HTMLElement | null;
  help: HTMLElement | null;
}

/** A number and its unit, followed as two fields and counted as one edit. */
class Pair {
  readonly number: FollowedField;
  readonly unit: FollowedField;

  constructor(
    readonly els: RowElements,
    readonly label: string,
    private readonly note: (text: string) => void
  ) {
    const quiet = (): void => {};
    this.number = new FollowedField(els.number, label, note);
    this.unit = new FollowedField(els.unit, label + " unit", quiet);
  }

  pending(): boolean {
    return this.number.pending() || this.unit.pending();
  }

  fromServer(number: string, unit: Unit): void {
    this.unit.fromServer(unit);
    this.number.fromServer(number);
  }

  /** Discard an edit, saying why once. */
  discard(reason: string): void {
    if (!this.pending()) return;
    this.revert();
    this.note(reason + "; unapplied " + this.label + " edit discarded");
  }

  revert(): void {
    this.number.revert();
    this.unit.revert();
  }

  sending(): void {
    this.number.sending(this.number.text());
    this.unit.sending(this.unit.text());
  }

  settled(ok: boolean, number: string, unit: string): void {
    if (ok) {
      this.number.accepted(number);
      this.unit.accepted(unit);
    } else {
      this.number.refused(number);
      this.unit.refused(unit);
    }
  }
}

/** What each part of the form can change in a stage. */
interface Editable {
  host: boolean;
  up: boolean;
  upUnit: boolean;
  down: boolean;
  downUnit: boolean;
  run: boolean;
}

const NONE: Editable = {
  host: false,
  up: false,
  upUnit: false,
  down: false,
  downUnit: false,
  run: false,
};

/** What can change in `stage`: everything in Idle; on a running users ramp
 * the rates of a ramp set by rate, and the run time. */
function editableIn(stage: string, settings: SnapshotSettings): Editable {
  if (stage === "idle") {
    return { host: true, up: true, upUnit: true, down: true, downUnit: true, run: true };
  }
  if (stage === "ramp_up" || stage === "hold" || stage === "hold_until_stopped") {
    return {
      ...NONE,
      up: settings.increase_time_secs == null,
      down: settings.decrease_time_secs == null,
      run: true,
    };
  }
  return NONE;
}

/** Why a stage can no longer take an edit, as the status line says. */
function stageChange(stage: string): string {
  switch (stage) {
    case "ramp_up":
    case "hold":
    case "hold_until_stopped":
      return "Run started elsewhere";
    case "ramping_down":
      return "Ramp down started";
    case "test_plan":
      return "Test plan running";
    case "stopping":
      return "Run stopping";
    default:
      return "Run stopped";
  }
}

/** The server's number and unit for a ramp set by `rate` or `secs`. */
function rampServer(rate: number | null, secs: number | null): [string, Unit] {
  if (secs != null) return timeParts(secs);
  if (rate != null) return [String(rate), "rate"];
  return ["", "rate"];
}

/** A settings field the server named, and the input that shows it. */
const FIELD_INPUTS: Record<string, string> = {
  host: "set-host",
  increase_rate: "set-up",
  increase_time: "set-up",
  decrease_rate: "set-down",
  decrease_time: "set-down",
  run_time: "set-run",
};

/** The form, or null on a page without it. */
export class SettingsForm {
  private readonly host: FollowedField;
  private readonly up: Pair;
  private readonly down: Pair;
  private readonly run: Pair;
  private settings: SnapshotSettings | null = null;
  private stage = "idle";
  private editable: Editable = NONE;
  private opened = false;
  /** The values in flight, so the reply settles exactly what was sent. */
  private sent: { host?: string; up?: [string, string]; down?: [string, string]; run?: [string, string] } = {};

  private constructor(
    private readonly details: HTMLDetailsElement,
    private readonly summary: HTMLElement,
    private readonly hostInput: HTMLInputElement,
    private readonly hostHelp: HTMLElement | null,
    private readonly hostReason: HTMLElement | null,
    upEls: RowElements,
    downEls: RowElements,
    runEls: RowElements,
    private readonly held: HTMLElement | null,
    onChange: () => void,
    note: (text: string) => void
  ) {
    this.host = new FollowedField(hostInput, "Host", note);
    this.up = new Pair(upEls, "Ramp up", note);
    this.down = new Pair(downEls, "Ramp down", note);
    this.run = new Pair(runEls, "Run time", note);
    for (const el of [hostInput, upEls.number, upEls.unit, downEls.number, downEls.unit, runEls.number, runEls.unit]) {
      el.addEventListener(el instanceof HTMLSelectElement ? "change" : "input", () => {
        el.removeAttribute("aria-invalid");
        this.refresh();
        onChange();
      });
    }
    details.addEventListener("toggle", () => {
      try {
        window.localStorage.setItem(OPEN_KEY, details.open ? "open" : "closed");
      } catch (_) {
        // Storage blocked: the state is not remembered.
      }
    });
  }

  /** The form on this page, or null. */
  static find(onChange: () => void, note: (text: string) => void): SettingsForm | null {
    const details = optionalElement("ctrl-settings", HTMLDetailsElement);
    const summary = optionalElement("ctrl-settings-label", HTMLElement);
    const host = optionalElement("set-host", HTMLInputElement);
    const row = (id: string): RowElements | null => {
      const number = optionalElement(id, HTMLInputElement);
      const unit = optionalElement(id + "-unit", HTMLSelectElement);
      if (!number || !unit) return null;
      return {
        number,
        unit,
        reason: optionalElement(id + "-reason", HTMLElement),
        help: optionalElement(id + "-help", HTMLElement),
      };
    };
    const up = row("set-up");
    const down = row("set-down");
    const run = row("set-run");
    if (!details || !summary || !host || !up || !down || !run) return null;
    return new SettingsForm(
      details,
      summary,
      host,
      optionalElement("set-host-help", HTMLElement),
      optionalElement("set-host-reason", HTMLElement),
      up,
      down,
      run,
      optionalElement("set-run-held", HTMLElement),
      onChange,
      note
    );
  }

  /** How many settings have unapplied edits. */
  pendingCount(): number {
    let n = 0;
    if (this.host.pending()) n++;
    for (const pair of [this.up, this.down, this.run]) if (pair.pending()) n++;
    return n;
  }

  /** The pending edits as a settings body: an emptied field as `null`, an
   * untouched one left out. While running a ramp sends only its rate. */
  body(): SettingsBody {
    const body: SettingsBody = {};
    if (this.host.pending()) {
      const text = this.host.text().trim();
      body.host = text === "" ? null : text;
    }
    const idle = this.stage === "idle";
    const ramp = (pair: Pair, rate: "increase_rate" | "decrease_rate", time: "increase_time_secs" | "decrease_time_secs"): void => {
      if (!pair.pending()) return;
      const n = numberOf(pair.number.text());
      const unit = pair.unit.text();
      if (!idle) {
        body[rate] = n;
        return;
      }
      if (n === null) {
        body[rate] = null;
        body[time] = null;
      } else if (unit === "rate") {
        body[rate] = n;
        body[time] = null;
      } else {
        body[time] = seconds(n, unit);
        body[rate] = null;
      }
    };
    ramp(this.up, "increase_rate", "increase_time_secs");
    ramp(this.down, "decrease_rate", "decrease_time_secs");
    if (this.run.pending()) {
      const n = numberOf(this.run.number.text());
      body.run_time_secs = n === null ? null : seconds(n, this.run.unit.text());
    }
    return body;
  }

  /** The new host when the pending edits would change it, else null. */
  hostChange(): string | null {
    if (!this.host.pending() || !this.settings) return null;
    const text = this.host.text().trim();
    if (text === (this.settings.host || "")) return null;
    if (text !== "") return text;
    return this.settings.hosts.length ? "each scenario's own host" : "no host";
  }

  /** The pending edits are being sent. */
  sending(): void {
    this.sent = {};
    if (this.host.pending()) {
      this.sent.host = this.host.text();
      this.host.sending(this.sent.host);
    }
    const pairs: ["up" | "down" | "run", Pair][] = [
      ["up", this.up],
      ["down", this.down],
      ["run", this.run],
    ];
    for (const [key, pair] of pairs) {
      if (!pair.pending()) continue;
      this.sent[key] = [pair.number.text(), pair.unit.text()];
      pair.sending();
    }
  }

  /** The reply to what was sent: accepted with the server's settings, or not. */
  settled(ok: boolean, settings: SnapshotSettings | null): void {
    const sent = this.sent;
    this.sent = {};
    if (sent.host !== undefined) {
      if (ok) this.host.accepted(sent.host);
      else this.host.refused(sent.host);
    }
    const pairs: [[string, string] | undefined, Pair][] = [
      [sent.up, this.up],
      [sent.down, this.down],
      [sent.run, this.run],
    ];
    for (const [values, pair] of pairs) {
      if (values) pair.settled(ok, values[0], values[1]);
    }
    // The reply's settings for what was not sent, such as a time a rate
    // cleared; what was sent waits for a snapshot to report it, so one from
    // before the reply cannot put the old value back.
    if (ok && settings) {
      this.settings = settings;
      this.follow(settings, sent);
    }
    this.refresh();
  }

  /** Drop every pending edit; the fields follow the server again. */
  discardAll(): void {
    this.host.revert();
    for (const pair of [this.up, this.down, this.run]) pair.revert();
    this.refresh();
  }

  /** The disclosure's label, counting `n` unapplied edits in all. */
  setPendingLabel(n: number): void {
    this.summary.textContent = n ? "Edit settings (" + n + " unapplied)" : "Edit settings";
  }

  /** Open the form, so a field the server named can take focus. */
  open(): void {
    this.details.open = true;
  }

  /** Mark the input showing `field` invalid and move focus to it; false when
   * the form shows no such field. */
  focusField(field: string): boolean {
    const id = FIELD_INPUTS[field];
    if (!id) return false;
    const input = document.getElementById(id);
    if (!input) return false;
    this.open();
    input.setAttribute("aria-invalid", "true");
    input.focus();
    return true;
  }

  /** A snapshot: follow its settings, and discard edits the stage can no
   * longer take. */
  fromSnapshot(settings: SnapshotSettings): void {
    const before = this.editable;
    const stageBefore = this.stage;
    this.stage = settings.stage;
    if (!this.opened) {
      this.opened = true;
      let stored: string | null = null;
      try {
        stored = window.localStorage.getItem(OPEN_KEY);
      } catch (_) {
        stored = null;
      }
      this.details.open = stored === null ? settings.stage === "idle" : stored === "open";
    }
    const now = editableIn(settings.stage, settings);
    if (stageBefore !== settings.stage) {
      const reason = stageChange(settings.stage);
      if (before.host && !now.host) this.host.discard(reason);
      const pairs: [Pair, boolean, boolean, boolean, boolean][] = [
        [this.up, before.up, now.up, now.upUnit, this.up.unit.pending()],
        [this.down, before.down, now.down, now.downUnit, this.down.unit.pending()],
        [this.run, before.run, now.run, true, false],
      ];
      for (const [pair, was, is, unit, unitEdit] of pairs) {
        if ((was && !is) || (unitEdit && !unit)) pair.discard(reason);
      }
    }
    this.follow(settings);
    this.refresh();
  }

  /** Follow `settings`, the configured part of a snapshot or a reply,
   * but for the fields in `skip`. */
  private follow(settings: SnapshotSettings, skip: { host?: unknown; up?: unknown; down?: unknown; run?: unknown } = {}): void {
    this.settings = settings;
    if (skip.host === undefined) this.host.fromServer(settings.host || "");
    const [up, upUnit] = rampServer(settings.increase_rate, settings.increase_time_secs);
    if (skip.up === undefined) this.up.fromServer(up, upUnit);
    const [down, downUnit] = rampServer(settings.decrease_rate, settings.decrease_time_secs);
    if (skip.down === undefined) this.down.fromServer(down, downUnit);
    const [run, runUnit] =
      settings.run_time_secs == null ? ["", this.run.unit.text() as Unit] : timeParts(settings.run_time_secs);
    if (skip.run === undefined) this.run.fromServer(run, runUnit === "rate" ? "s" : runUnit);
  }

  /** Read only states, reasons, help, placeholders and the disclosure's
   * label, from the stage and whether the page can send (`canSend`: a
   * token, and no request in flight). */
  update(canSend: boolean): void {
    this.canSend = canSend;
    this.refresh();
  }

  private canSend = false;

  private refresh(): void {
    const settings = this.settings;
    const editable = settings ? editableIn(this.stage, settings) : NONE;
    this.editable = editable;
    const on = this.canSend;
    setReadOnly(this.hostInput, !(on && editable.host));
    setReadOnly(this.up.els.number, !(on && editable.up));
    setReadOnly(this.up.els.unit, !(on && editable.upUnit));
    setReadOnly(this.down.els.number, !(on && editable.down));
    setReadOnly(this.down.els.unit, !(on && editable.downUnit));
    setReadOnly(this.run.els.number, !(on && editable.run));
    setReadOnly(this.run.els.unit, !(on && editable.run));

    if (!settings) return;

    // While running only a rate changes: the time units go.
    const running = this.stage !== "idle";
    for (const pair of [this.up, this.down]) {
      for (const option of Array.from(pair.els.unit.options)) {
        option.hidden = running && option.value !== "rate" && pair.unit.text() === "rate";
      }
    }

    const reasons = this.stageReasons(settings);
    setText(this.hostReason, this.hostInput, reasons.host);
    if (this.hostHelp) {
      setText(
        this.hostHelp,
        this.hostInput,
        settings.host == null && settings.hosts.length ? "Overrides every scenario's host" : ""
      );
    }
    this.hostInput.placeholder =
      settings.host == null ? "default: " + settingsHostsText(settings) : "";
    for (const [pair, reason] of [
      [this.up, reasons.up],
      [this.down, reasons.down],
      [this.run, reasons.run],
    ] as [Pair, string][]) {
      setText(pair.els.reason, pair.els.number, reason);
      setText(pair.els.reason, pair.els.unit, reason);
    }

    this.up.els.number.placeholder = "default: 1 user per second";
    this.down.els.number.placeholder = "default: all stop at once";
    this.run.els.number.placeholder = "default: none";

    // Run time help: what it is, and what empty means with the form's ramp down.
    if (this.run.els.help) {
      const downSet = this.down.number.text().trim() !== "";
      const parts = ["Time at full users after the ramp up, not total time."];
      parts.push(
        downSet
          ? "Empty: ramps straight down after the ramp up."
          : "Empty: holds until stopped."
      );
      if (running && editable.run) {
        parts.push("While running, emptying it holds until stopped, refused while a ramp down is set.");
      }
      setText(this.run.els.help, this.run.els.number, parts.join(" "));
    }
    if (this.held) {
      const total = settings.run_time_secs;
      const left = settings.hold_ends_in_secs;
      const text =
        this.stage === "hold" && total != null && left != null
          ? "held " + spanText(Math.max(0, total - left)) + " of " + spanText(total)
          : "";
      setText(this.held, this.run.els.number, text);
    }
  }

  /** The reason line of each group for the stage, "" when it can change. */
  private stageReasons(settings: SnapshotSettings): { host: string; up: string; down: string; run: string } {
    const stage = this.stage;
    if (stage === "idle") return { host: "", up: "", down: "", run: "" };
    if (stage === "ramp_up" || stage === "hold" || stage === "hold_until_stopped") {
      const byTime = (secs: number | null): string =>
        secs != null
          ? "Set by time: " + spanText(secs) + "; change it when idle"
          : "Users per second while running; a time changes only when idle";
      return {
        host: "Host changes only when idle",
        up: byTime(settings.increase_time_secs),
        down: byTime(settings.decrease_time_secs),
        run: "",
      };
    }
    const all =
      stage === "test_plan"
        ? "The test plan sets the ramp and how long it runs"
        : stage === "ramping_down"
          ? "Ramp down started; ramp settings change again when idle"
          : "Run stopping; settings change again when idle";
    return { host: "Host changes only when idle", up: all, down: all, run: all };
  }
}

/** Show `text` on `el`, hidden when empty, and in the input's description
 * only while shown. */
function setText(el: HTMLElement | null, input: HTMLElement, text: string): void {
  if (!el) return;
  el.textContent = text;
  el.hidden = text === "";
  if (text === "") {
    removeDescribedBy(input, el.id);
  } else {
    addDescribedBy(input, el.id);
  }
}
