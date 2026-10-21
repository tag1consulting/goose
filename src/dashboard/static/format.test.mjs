// Response times under 10 ms are shown to the hundredth of a millisecond,
// truncated, and whole milliseconds from 10 ms (#585). Run with `npm test`;
// `pretest` builds src/format.ts into test-build/.

import { test } from "node:test";
import assert from "node:assert/strict";
import { formatMs } from "./test-build/format.mjs";

test("formatMs shows hundredths under 10 ms and whole milliseconds above", () => {
  // Each as the snapshot's microseconds make it.
  const cases = [
    [40, "0.04"],
    [400, "0.4"],
    [2350, "2.35"],
    [9990, "9.99"],
    [10000, "10"],
    [37999, "37"],
  ];
  for (const [us, shown] of cases) {
    assert.equal(formatMs(us / 1000), shown, us + " µs");
  }
});

test("formatMs truncates, never rounds up, and shows whole milliseconds plainly", () => {
  assert.equal(formatMs(429 / 1000), "0.42");
  assert.equal(formatMs(3), "3");
  assert.equal(formatMs(0), "0");
  assert.equal(formatMs(1400), "1,400");
  assert.equal(formatMs(undefined), "—");
  assert.equal(formatMs(NaN), "—");
});
