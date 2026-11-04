// The Control card's split previews for a typed value follow the manager's
// own rules: split-fixture.json holds 1,000 random cases worked out by
// them in Rust (`split_fixture_matches_the_rules` in src/gaggle/dashboard.rs).
// Run with `npm test`; `pretest` builds src/split.ts into test-build/.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { groups, splitText, throttleSplit, usersSplit } from "./test-build/split.mjs";

const CASES = JSON.parse(readFileSync(new URL("split-fixture.json", import.meta.url), "utf8"));

test("the fixture holds 1,000 cases", () => {
  assert.equal(CASES.length, 1000);
});

test("users split as the manager deals them, on every case", () => {
  for (const [at, c] of CASES.entries()) {
    assert.deepEqual(usersSplit(c.users, c.counts), c.users_after, "case " + at);
  }
});

test("the throttle splits as the manager splits it, on every case", () => {
  for (const [at, c] of CASES.entries()) {
    assert.deepEqual(throttleSplit(c.throttle, c.loads), c.shares, "case " + at);
  }
});

test("the preview sentences", () => {
  assert.equal(
    splitText(groups(usersSplit(900, new Array(47).fill(0))), "users"),
    "split: 20 users on 7 workers, 19 on 40"
  );
  assert.equal(
    splitText(groups(throttleSplit(3000, new Array(47).fill(19))), "req/s"),
    "split: 64 req/s on 39 workers, 63 on 8"
  );
  assert.equal(splitText([{ value: 1, workers: 1 }], "users"), "split: 1 user on 1 worker");
  assert.equal(splitText([{ value: 5, workers: 3 }], "req/s"), "split: 5 req/s on each of 3 workers");
  assert.equal(splitText([], "users"), "");
});
