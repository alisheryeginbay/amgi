import assert from "node:assert/strict";
import { test } from "node:test";
import { dailyAverage, reviewTime, studyTime } from "./time.ts";

test("study time is rounded to its size", () => {
  assert.equal(studyTime(0), "0 s");
  assert.equal(studyTime(45), "45 s");
  assert.equal(studyTime(60), "1 min");
  assert.equal(studyTime(23 * 60 + 29), "23 min");
  assert.equal(studyTime(3570), "1 h");
  assert.equal(studyTime(3900), "1 h 5 min");
});

test("review time keeps a decimal under ten seconds", () => {
  assert.equal(reviewTime(8.42), "8.4 s");
  assert.equal(reviewTime(9.96), "10 s");
  assert.equal(reviewTime(75), "1 min");
});

test("daily average skips days without time", () => {
  assert.equal(dailyAverage([0, 600, 0, 1200]), 900);
  assert.equal(dailyAverage([0, 0]), null);
});
