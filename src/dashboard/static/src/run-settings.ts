// The run settings summary: one line saying what the next Start runs, or
// where the running test is, from the snapshot's `settings`. Its wording
// comes from `settings.stage`, never from `phase`, which cannot tell a
// Users decrease from the configured ramp down.

/** A time span in words: "10 min", "6 min 52 s", "1 h 1 min". */
export function spanText(secs: number): string {
  const total = Math.max(0, Math.round(secs));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const parts: string[] = [];
  if (h) parts.push(h + " h");
  if (m) parts.push(m + " min");
  if (s || parts.length === 0) parts.push(s + " s");
  return parts.join(" ");
}

/** A rate in users per second, as "10/s" or "0.5/s". */
export function rateText(rate: number): string {
  return String(Math.round(rate * 100) / 100) + "/s";
}

/** The hosts the next Start will load test, or "no host set". */
export function settingsHostsText(settings: SnapshotSettings): string {
  if (settings.host != null) return settings.host;
  return settings.hosts.length ? settings.hosts.join(", ") : "no host set";
}

/** A parsed `--test-plan` string: its steps, peak users and total seconds. */
export interface PlanSummary {
  steps: number;
  peak: number;
  secs: number;
}

/** Read a canonical test plan, as `settings.test_plan` holds it. */
export function planSummary(plan: string): PlanSummary | null {
  let peak = 0;
  let secs = 0;
  const steps = plan.split(";");
  for (const step of steps) {
    const parts = step.split(",");
    const users = parseInt(parts[0], 10);
    const span = parseInt(parts[1], 10);
    if (parts.length !== 2 || !isFinite(users) || !isFinite(span)) return null;
    peak = Math.max(peak, users);
    secs += span;
  }
  return { steps: steps.length, peak, secs };
}

function users(n: number | null): string {
  return n === 1 ? "1 user" : String(n == null ? 0 : n) + " users";
}

/** How the ramp up goes: "at 10/s", "over 5 min" or "at 1/s (default)". */
function rampUpText(settings: SnapshotSettings): string {
  if (settings.increase_time_secs != null) {
    return "over " + spanText(settings.increase_time_secs);
  }
  if (settings.increase_rate != null) return "at " + rateText(settings.increase_rate);
  return "at 1/s (default)";
}

/** Whether a ramp down is configured, by rate or by time. */
function hasRampDown(settings: SnapshotSettings): boolean {
  return settings.decrease_time_secs != null || settings.decrease_rate != null;
}

/** How the ramp down goes after `lead`: "at 10/s" or "over 1 min". */
function rampDownText(settings: SnapshotSettings, lead: string): string {
  if (settings.decrease_time_secs != null) {
    return lead + "over " + spanText(settings.decrease_time_secs);
  }
  return lead + "at " + rateText(settings.decrease_rate || 0);
}

/**
 * What follows the ramp up: the hold and what ends it, as in "hold 10 min ·
 * then 10/s down".
 */
function tailText(settings: SnapshotSettings): string {
  const down = hasRampDown(settings);
  if (settings.run_time_secs != null) {
    const hold = "hold " + spanText(settings.run_time_secs);
    if (!down) return hold + " · then all stop";
    if (settings.decrease_time_secs != null) {
      return hold + " · then down over " + spanText(settings.decrease_time_secs);
    }
    return hold + " · then " + rateText(settings.decrease_rate || 0) + " down";
  }
  return down ? rampDownText(settings, "then straight down ") : "until stopped";
}

/**
 * The summary line for `settings`, with the hosts first when `withHost`. A
 * test plan reads "test plan: 3 steps, peak 100 users, 1 h 1 min total".
 */
export function settingsSummary(settings: SnapshotSettings, withHost: boolean): string {
  const parts: string[] = [];
  if (withHost) parts.push(settingsHostsText(settings));
  const stage = settings.stage;
  if (stage === "stopping") {
    parts.push("stopping");
  } else if (settings.test_plan != null) {
    const plan = planSummary(settings.test_plan);
    parts.push(
      plan
        ? "test plan: " +
            plan.steps +
            (plan.steps === 1 ? " step, peak " : " steps, peak ") +
            users(plan.peak) +
            ", " +
            spanText(plan.secs) +
            " total"
        : "test plan"
    );
  } else if (stage === "ramp_up") {
    parts.push("ramping to " + users(settings.users) + " " + rampUpText(settings));
    parts.push(tailText(settings));
  } else if (stage === "hold") {
    const ends = hasRampDown(settings) ? "ramp down" : "all stop";
    parts.push(users(settings.users));
    const left = settings.hold_ends_in_secs;
    parts.push(
      left === 0 || left == null
        ? ends + " after this ramp"
        : ends + " in " + spanText(left)
    );
  } else if (stage === "hold_until_stopped") {
    parts.push(users(settings.users));
    parts.push("holding until stopped");
  } else if (stage === "ramping_down") {
    parts.push(rampDownText(settings, "ramping down "));
  } else {
    parts.push(users(settings.users) + " " + rampUpText(settings));
    parts.push(tailText(settings));
  }
  return parts.join(" · ");
}

/** Whether two host lists say the same hosts. */
export function sameHosts(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((host, i) => host === b[i]);
}
