import assert from "node:assert/strict";
import { test } from "node:test";
import { clozeGap, GAP } from "./markdown.ts";

test("finds the gap a cloze's answer fills", () => {
  const gap = clozeGap("The capital of **[…]** is Paris.", "The capital of **France** is Paris.");
  assert.equal(gap?.source, `The capital of **${GAP}** is Paris.`);
  assert.equal(gap?.hint, "<strong>[…]</strong>");
  assert.equal(gap?.answer, "<strong>France</strong>");
});

test("finds a gap whose hint or answer shares characters with its edges", () => {
  const gap = clozeGap(String.raw`\[1\] is **[\[i\]]** and more.`, String.raw`\[1\] is **[first]** and more.`);
  assert.equal(gap?.source, `\\[1\\] is **${GAP}** and more.`);
  assert.equal(gap?.answer, "<strong>[first]</strong>");
});

test("leaves image answers to a whole-card swap", () => {
  assert.equal(clozeGap("![](a.png) is **[…]**", "![](a.png) is **![](b.png)**"), null);
});

test("leaves sides that differ elsewhere to a whole-card swap", () => {
  assert.equal(clozeGap("A **[…]** b", "Z **x** b"), null);
});
