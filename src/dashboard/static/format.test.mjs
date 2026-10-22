// Response times under 10 ms are shown to the hundredth of a millisecond,
// and whole milliseconds from 10 ms, rounded half up (#585). Run with `npm test`;
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
    [9994, "9.99"],
    [9995, "10"],
    [37999, "38"],
  ];
  for (const [us, shown] of cases) {
    assert.equal(formatMs(us / 1000), shown, us + " µs");
  }
});

test("formatMs rounds half up and shows whole milliseconds plainly", () => {
  assert.equal(formatMs(429 / 1000), "0.43");
  assert.equal(formatMs(2345 / 1000), "2.35");
  assert.equal(formatMs(37499 / 1000), "37");
  assert.equal(formatMs(3), "3");
  assert.equal(formatMs(0), "0");
  assert.equal(formatMs(1400), "1,400");
  assert.equal(formatMs(undefined), "—");
  assert.equal(formatMs(NaN), "—");
});
