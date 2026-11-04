// How a change typed into the Control card would split between a Gaggle's
// running workers: the users each would run and the throttle each would
// take. The snapshot carries the split of the current values, worked out
// in Rust by the manager's own rules; these follow the same rules for a
// value the user is still typing. The fixture split-fixture.json, written
// by a Rust test from the manager's rules on 1,000 random cases, checks
// that they agree (split.test.mjs).

/** `workers` workers with `value` each. */
export interface SplitGroup {
  value: number;
  workers: number;
}

/**
 * Each running worker's users after a change to `users` in all, from
 * `counts`, each worker's users now in worker order. More users are dealt
 * one at a time to the worker running fewest, the first among equals;
 * fewer stop the users started last, which for users dealt that way are on
 * the worker running most, the last among equals.
 */
export function usersSplit(users: number, counts: number[]): number[] {
  const out = counts.slice();
  if (out.length === 0) return out;
  const total = counts.reduce((sum, n) => sum + n, 0);
  if (users === total) return out;
  // In closed form, not one user at a time, so a large typed number costs no
  // more than a small one: every worker is raised to (or lowered to) one
  // level, and the users left over go one each to the first workers at it
  // (or come one each from the last).
  const up = users > total;
  const change = Math.abs(users - total);
  const cost = (level: number): number =>
    counts.reduce((sum, n) => sum + Math.max(0, up ? level - n : n - level), 0);
  let lo = up ? Math.min(...counts) : 0;
  let hi = up ? lo + change : Math.max(...counts);
  if (up) {
    while (lo < hi) {
      const mid = Math.ceil((lo + hi) / 2);
      if (cost(mid) <= change) lo = mid;
      else hi = mid - 1;
    }
  } else {
    while (lo < hi) {
      const mid = Math.floor((lo + hi) / 2);
      if (cost(mid) <= change) hi = mid;
      else lo = mid + 1;
    }
  }
  const level = lo;
  let rest = change - cost(level);
  for (let i = 0; i < out.length; i++) out[i] = up ? Math.max(out[i], level) : Math.min(out[i], level);
  if (up) {
    for (let i = 0; i < out.length && rest > 0; i++) {
      if (out[i] === level) {
        out[i] += 1;
        rest -= 1;
      }
    }
  } else {
    for (let i = out.length - 1; i >= 0 && rest > 0 && level > 0; i--) {
      if (out[i] === level) {
        out[i] -= 1;
        rest -= 1;
      }
    }
  }
  return out;
}

/**
 * `throttle` split in proportion to `users`: each share its proportion
 * rounded down, at least 1 for a worker with users, the rest one at a time
 * by largest remainder, then the first workers; a share raised to 1 that
 * passes the total is taken back from the largest.
 */
function throttleShares(throttle: number, users: number[]): number[] {
  const total = users.reduce((sum, n) => sum + n, 0);
  if (throttle === 0 || total === 0) return users.map(() => 0);
  const shares: number[] = [];
  const remainders: [number, number][] = [];
  users.forEach((u, worker) => {
    if (u === 0) {
      shares.push(0);
      return;
    }
    // Exact in a double: a throttle and users within Goose's bounds keep
    // the product below 2^53.
    const exact = throttle * u;
    shares.push(Math.max(1, Math.floor(exact / total)));
    remainders.push([exact % total, worker]);
  });
  remainders.sort((a, b) => b[0] - a[0] || a[1] - b[1]);
  let sum = shares.reduce((s, n) => s + n, 0);
  let at = 0;
  while (sum < throttle && remainders.length > 0) {
    shares[remainders[at % remainders.length][1]] += 1;
    sum += 1;
    at += 1;
  }
  while (sum > throttle) {
    let largest = 0;
    for (let i = 1; i < shares.length; i++) if (shares[i] > shares[largest]) largest = i;
    if (shares[largest] <= 1) break;
    shares[largest] -= 1;
    sum -= 1;
  }
  return shares;
}

/**
 * Each running worker's throttle share of `throttle`, by its share of the
 * users in `counts`: a worker with no users keeps 1, and with no users
 * anywhere the throttle is split evenly.
 */
export function throttleSplit(throttle: number, counts: number[]): number[] {
  const workers = counts.length;
  if (workers === 0) return [];
  if (counts.every((n) => n === 0)) {
    const base = Math.floor(throttle / workers);
    return counts.map((_, i) => Math.max(1, base + (i < throttle % workers ? 1 : 0)));
  }
  const idle = counts.filter((n) => n === 0).length;
  const busy = counts.filter((n) => n > 0);
  const shares = throttleShares(Math.max(0, throttle - idle), busy);
  let next = 0;
  return counts.map((n) => (n === 0 ? 1 : Math.max(1, shares[next++] || 1)));
}

/** `values` as groups of equal values, largest first. */
export function groups(values: number[]): SplitGroup[] {
  const counts = new Map<number, number>();
  for (const value of values) counts.set(value, (counts.get(value) || 0) + 1);
  return [...counts.entries()]
    .sort((a, b) => b[0] - a[0])
    .map(([value, workers]) => ({ value, workers }));
}

function plural(n: number, noun: string): string {
  return n.toLocaleString("en-US") + " " + noun + (n === 1 ? "" : "s");
}

/**
 * The preview sentence for `split`: `split: 20 users on 7 workers, 19 on
 * 40`. `unit` is what a value counts: `users` or `req/s`. Empty with no
 * workers.
 */
export function splitText(split: SplitGroup[], unit: "users" | "req/s"): string {
  if (split.length === 0) return "";
  const first = split[0];
  const value = (n: number): string =>
    unit === "users" ? plural(n, "user") : n.toLocaleString("en-US") + " req/s";
  const parts = split.map((group, i) =>
    i === 0
      ? value(group.value) + " on " + plural(group.workers, "worker")
      : group.value.toLocaleString("en-US") + " on " + group.workers.toLocaleString("en-US")
  );
  if (split.length === 1 && first.workers > 1) {
    return "split: " + value(first.value) + " on each of " + plural(first.workers, "worker");
  }
  return "split: " + parts.join(", ");
}
