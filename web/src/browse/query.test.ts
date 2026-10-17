import assert from "node:assert/strict";
import { test } from "node:test";
import type { BrowseCard } from "../lib/types";
import { parseQuery } from "./query.ts";

const NOW = 1_800_000_000;
const DAY = 86_400;

function card(id: string, fields: Partial<BrowseCard>): BrowseCard {
  return {
    id, deck: "geography", tags: [], path: "geography.md", line: 1, kind: "basic",
    front: "", back: "", due: null, reps: 0, lapses: 0, suspended: false, ...fields,
  };
}

const CARDS = [
  card("paris", { front: "Capital of France?", back: "Paris", tags: ["europe", "capitals"], due: NOW - DAY, reps: 4 }),
  card("cafe", { front: "What does café mean?", back: "coffee", deck: "lang/french/words", tags: ["food"], due: NOW + 3 * DAY, reps: 2 }),
  card("water", { front: "Formula for water", back: "H2O", deck: "science", kind: "cloze" }),
  card("leech", { front: "Boiling point at 12:30 altitude", back: "less than 100", deck: "science/physics", lapses: 9, reps: 12, due: NOW - 60, suspended: true, tags: ["suspended"] }),
];

function search(query: string): string[] {
  const matches = parseQuery(query, NOW);
  return CARDS.filter(matches).map((c) => c.id);
}

test("an empty search matches everything", () => {
  assert.deepEqual(search(""), ["paris", "cafe", "water", "leech"]);
  assert.deepEqual(search("   "), ["paris", "cafe", "water", "leech"]);
});

test("words match question, answer and tags, ignoring case and accents", () => {
  assert.deepEqual(search("CAFE"), ["cafe"]);
  assert.deepEqual(search("paris"), ["paris"]);
  assert.deepEqual(search("capitals"), ["paris"]);
});

test("every word must match", () => {
  assert.deepEqual(search("capital france"), ["paris"]);
  assert.deepEqual(search("capital water"), []);
});

test("quotes keep a phrase together", () => {
  assert.deepEqual(search('"for water"'), ["water"]);
  assert.deepEqual(search('"water for"'), []);
  assert.deepEqual(search('"unclosed for wat'), []);
  assert.deepEqual(search('"formula for wat'), ["water"]);
});

test("a leading minus leaves cards out", () => {
  assert.deepEqual(search("-paris"), ["cafe", "water", "leech"]);
  assert.deepEqual(search("-is:new -is:suspended"), ["paris", "cafe"]);
  assert.deepEqual(search("- paris"), []);
});

test("#tag matches whole tags, with * for a prefix", () => {
  assert.deepEqual(search("#capital"), []);
  assert.deepEqual(search("#Capitals"), ["paris"]);
  assert.deepEqual(search("#cap*"), ["paris"]);
  assert.deepEqual(search("-#suspended"), ["paris", "cafe", "water"]);
  assert.deepEqual(search("#"), ["paris", "cafe", "water", "leech"]);
});

test("deck: matches a deck and its subdecks by any run of folders", () => {
  assert.deepEqual(search("deck:science"), ["water", "leech"]);
  assert.deepEqual(search("deck:physics"), ["leech"]);
  assert.deepEqual(search("deck:french"), ["cafe"]);
  assert.deepEqual(search("deck:lang/french"), ["cafe"]);
  assert.deepEqual(search('deck:"french/words"'), ["cafe"]);
  assert.deepEqual(search("deck:fren"), []);
  assert.deepEqual(search("deck:lang/words"), []);
});

test("q: and a: search one side", () => {
  assert.deepEqual(search("q:paris"), []);
  assert.deepEqual(search("a:paris"), ["paris"]);
  assert.deepEqual(search("Q:capital"), ["paris"]);
  assert.deepEqual(search('a:"less than"'), ["leech"]);
});

test("is: filters by state and kind", () => {
  assert.deepEqual(search("is:new"), ["water"]);
  assert.deepEqual(search("is:due"), ["paris"]);
  assert.deepEqual(search("is:suspended"), ["leech"]);
  assert.deepEqual(search("is:leech"), ["leech"]);
  assert.deepEqual(search("is:cloze"), ["water"]);
  assert.deepEqual(search("is:basic"), ["paris", "cafe", "leech"]);
});

test("numbers compare with <, <=, >, >=, = or :", () => {
  assert.deepEqual(search("lapses>3"), ["leech"]);
  assert.deepEqual(search("lapses:>=9"), ["leech"]);
  assert.deepEqual(search("reviews:4"), ["paris"]);
  assert.deepEqual(search("reviews=2"), ["cafe"]);
  assert.deepEqual(search("reviews<=2"), ["cafe", "water"]);
});

test("due compares days from now, and never matches new cards", () => {
  assert.deepEqual(search("due<0"), ["paris", "leech"]);
  assert.deepEqual(search("due<7"), ["paris", "cafe", "leech"]);
  assert.deepEqual(search("due>1"), ["cafe"]);
  assert.deepEqual(search("due>-2"), ["paris", "cafe", "leech"]);
});

test("filters still being typed match everything", () => {
  for (const query of ["deck:", "is:", "q:", "lapses>", "due<"]) {
    assert.deepEqual(search(query), ["paris", "cafe", "water", "leech"], query);
  }
});

test("anything else is searched as text", () => {
  assert.deepEqual(search("12:30"), ["leech"]);
  assert.deepEqual(search("is:nothing"), []);
  assert.deepEqual(search("lapses>many"), []);
  assert.deepEqual(search("H2O"), ["water"]);
});
