import type { BrowseCard, NoteTypeInfo } from "../lib/types";

// The browse search. Words must all match; see the README for the syntax.

/** Forgotten this many times or more, as in Anki. */
const LEECH_LAPSES = 8;
const DAY_SECS = 86_400;

/** Case- and accent-insensitive, like Finder's search. */
export function fold(s: string): string {
  return s.normalize("NFD").replace(/\p{M}/gu, "").toLocaleLowerCase();
}

type Test = (card: BrowseCard) => boolean;

const STATES: Record<string, (now: number) => Test> = {
  new: () => (c) => c.due === null,
  due: (now) => (c) => !c.suspended && c.due !== null && c.due <= now,
  suspended: () => (c) => c.suspended,
  leech: () => (c) => c.lapses >= LEECH_LAPSES,
  cloze: () => (c) => c.kind === "cloze",
  basic: () => (c) => c.kind === "basic",
};

type Compare = (a: number, b: number) => boolean;
const COMPARES: Record<string, Compare> = {
  "<": (a, b) => a < b,
  "<=": (a, b) => a <= b,
  ">": (a, b) => a > b,
  ">=": (a, b) => a >= b,
  "=": (a, b) => a === b,
  ":": (a, b) => a === b,
};

/** Numbers a card can be compared on. `due` is in days from now; new cards have none. */
const NUMBERS: Record<string, (card: BrowseCard, now: number) => number | null> = {
  reviews: (c) => c.reps,
  lapses: (c) => c.lapses,
  due: (c, now) => (c.due === null ? null : (c.due - now) / DAY_SECS),
};

interface Token {
  negated: boolean;
  /** Text before the first `:`, `<`, `>` or `=`, if the token has one and it wasn't quoted. */
  key: string | null;
  operator: string;
  value: string;
  /** The token as typed, without the leading `-`, for when it isn't a known filter. */
  raw: string;
}

/** Splits on spaces outside quotes. `a:"to go"` stays one token; an unclosed quote runs to the end. */
function tokenize(query: string): Token[] {
  const tokens: Token[] = [];
  let i = 0;
  while (i < query.length) {
    if (/\s/.test(query[i])) {
      i++;
      continue;
    }
    let negated = false;
    if (query[i] === "-" && i + 1 < query.length && !/\s/.test(query[i + 1])) {
      negated = true;
      i++;
    }
    const start = i;
    // A tag is never a filter, so tags like `lang::korean` keep their colons.
    const tag = query[i] === "#";
    let key: string | null = null;
    let operator = "";
    let value = "";
    while (i < query.length && !/\s/.test(query[i])) {
      if (query[i] === '"') {
        const end = query.indexOf('"', i + 1);
        const stop = end === -1 ? query.length : end;
        value += query.slice(i + 1, stop);
        i = stop + 1;
        continue;
      }
      if (key === null && !tag) {
        const op = /^(<=|>=|[<>=:])/.exec(query.slice(i));
        if (op && value) {
          key = value;
          value = "";
          operator = op[1];
          i += op[1].length;
          // `lapses:>3` reads like `lapses>3`.
          const after = operator === ":" ? /^(<=|>=|[<>=])/.exec(query.slice(i)) : null;
          if (after) {
            operator = after[1];
            i += after[1].length;
          }
          continue;
        }
      }
      value += query[i];
      i++;
    }
    tokens.push({ negated, key, operator, value, raw: query.slice(start, Math.min(i, query.length)) });
  }
  return tokens;
}

/** A plain `"quoted"` word has its quotes removed; anything else is searched as typed. */
function text(token: Token): string {
  return token.key === null ? token.value : token.raw;
}

function textTest(word: string, fields: (c: BrowseCard) => string[]): Test {
  const needle = fold(word);
  return (c) => fields(c).some((f) => fold(f).includes(needle));
}

/** Matches a deck and its subdecks, by its full path or any run of its folders: `korean` finds `lang/korean/verbs`. */
function deckTest(path: string): Test {
  const want = fold(path).split("/").filter(Boolean);
  return (c) => {
    const parts = fold(c.deck).split("/");
    for (let i = 0; i + want.length <= parts.length; i++) {
      if (want.every((w, j) => parts[i + j] === w)) return true;
    }
    return false;
  };
}

/** `#verbs` matches that tag exactly; `#lang*` any tag starting with `lang`. */
function tagTest(tag: string): Test {
  const want = fold(tag);
  if (want.endsWith("*")) {
    const prefix = want.slice(0, -1);
    return (c) => c.tags.some((t) => fold(t).startsWith(prefix));
  }
  return (c) => c.tags.some((t) => fold(t) === want);
}

/** A field name as typed in a search: folded, without spaces, so `wordmeaning:` finds `Word Meaning`. */
function fieldKey(name: string): string {
  return fold(name).replace(/\s+/g, "");
}

/** Notes with a field named `key` whose text has `word`. */
function fieldTest(key: string, word: string, noteTypes: NoteTypeInfo[]): Test {
  const index = new Map(noteTypes.map((t) => [t.name, t.fields.findIndex((f) => fieldKey(f) === key)]));
  const needle = fold(word);
  return (c) => {
    const i = c.noteType === null ? -1 : (index.get(c.noteType) ?? -1);
    return i >= 0 && fold(c.fields[i] ?? "").includes(needle);
  };
}

/** What one token tests; null if it matches everything, like a filter still being typed (`deck:`). */
function compile(token: Token, now: number, noteTypes: NoteTypeInfo[]): Test | null {
  const { key, operator, value } = token;
  if (key === null && token.raw.startsWith("#")) {
    return value.length > 1 ? tagTest(value.slice(1)) : null;
  }
  if (key !== null) {
    const name = key.toLowerCase();
    const number = NUMBERS[name];
    if (number) {
      if (value === "") return null;
      const n = Number(value);
      if (Number.isFinite(n)) {
        const compare = COMPARES[operator];
        return (c) => {
          const v = number(c, now);
          return v !== null && compare(v, n);
        };
      }
    } else if (operator === ":") {
      switch (name) {
        case "deck":
          return value ? deckTest(value) : null;
        case "q":
          return value ? textTest(value, (c) => [c.front]) : null;
        case "a":
          return value ? textTest(value, (c) => [c.back]) : null;
        case "is": {
          if (!value) return null;
          const state = STATES[value.toLowerCase()];
          if (state) return state(now);
          break;
        }
        default: {
          const field = fieldKey(key);
          if (noteTypes.some((t) => t.fields.some((f) => fieldKey(f) === field))) {
            return value ? fieldTest(field, value, noteTypes) : null;
          }
        }
      }
    }
  }
  const word = text(token);
  // A note's fields count too, with those no card shows.
  return word ? textTest(word, (c) => [c.front, c.back, ...c.tags, ...c.fields]) : null;
}

/**
 * Turns a search into a test for cards. Never fails: anything that isn't a known
 * filter is searched as text, so `12:30` and links still work.
 */
export function parseQuery(query: string, now = Date.now() / 1000, noteTypes: NoteTypeInfo[] = []): Test {
  const tests = tokenize(query).flatMap((token) => {
    const test = compile(token, now, noteTypes);
    if (!test) return [];
    return [token.negated ? (c: BrowseCard) => !test(c) : test];
  });
  return (card) => tests.every((t) => t(card));
}
