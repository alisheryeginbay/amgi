import assert from "node:assert/strict";
import { test } from "node:test";
import { compare } from "./diff.ts";

const show = (pieces: { text: string; kind: string }[]) =>
  pieces.map((p) => (p.kind === "same" ? p.text : p.kind === "extra" ? `(${p.text})` : `[${p.text}]`)).join("");

test("a right answer is correct, whatever its spaces", () => {
  const result = compare("  학교 ", "학교");
  assert.equal(result.correct, true);
  assert.equal(show(result.typed), "학교");
});

test("mistakes are marked on both sides", () => {
  const result = compare("hollo wrld", "hello world");
  assert.equal(result.correct, false);
  assert.equal(show(result.typed), "h(o)llo wrld");
  assert.equal(show(result.answer), "h[e]llo w[o]rld");
});

test("case and accents count", () => {
  assert.equal(compare("Cafe", "café").correct, false);
  assert.equal(show(compare("", "가다").answer), "[가다]");
});
