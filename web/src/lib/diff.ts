/** A run of characters from a typed answer compared with the right one. */
export interface Piece {
  text: string;
  /** In both; typed but not in the answer; or in the answer but not typed. */
  kind: "same" | "extra" | "missing";
}

export interface Comparison {
  correct: boolean;
  /** What was typed, with the characters that don't belong marked `extra`. */
  typed: Piece[];
  /** The answer, with the characters left out marked `missing`. */
  answer: Piece[];
}

/** Spaces collapsed and the ends trimmed, so they never count as mistakes. */
function tidy(s: string): string {
  return s.normalize("NFC").replace(/\s+/g, " ").trim();
}

/** Characters next to each other of the same kind, as one piece. */
function join(chars: { char: string; kind: Piece["kind"] }[]): Piece[] {
  const pieces: Piece[] = [];
  for (const { char, kind } of chars) {
    const last = pieces.at(-1);
    if (last?.kind === kind) last.text += char;
    else pieces.push({ text: char, kind });
  }
  return pieces;
}

/** Compare a typed answer with the right one, character by character, keeping the
 *  longest run of characters they share in order. Case and accents count, as in Anki. */
export function compare(typed: string, answer: string): Comparison {
  const a = Array.from(tidy(typed));
  const b = Array.from(tidy(answer));
  // lengths[i][j]: the longest common subsequence of a[i..] and b[j..].
  const lengths = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      lengths[i][j] = a[i] === b[j] ? lengths[i + 1][j + 1] + 1 : Math.max(lengths[i + 1][j], lengths[i][j + 1]);
    }
  }
  const mine: { char: string; kind: Piece["kind"] }[] = [];
  const theirs: { char: string; kind: Piece["kind"] }[] = [];
  let [i, j] = [0, 0];
  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && a[i] === b[j]) {
      mine.push({ char: a[i++], kind: "same" });
      theirs.push({ char: b[j++], kind: "same" });
    } else if (j < b.length && (i === a.length || lengths[i][j + 1] >= lengths[i + 1][j])) {
      theirs.push({ char: b[j++], kind: "missing" });
    } else {
      mine.push({ char: a[i++], kind: "extra" });
    }
  }
  return { correct: a.join("") === b.join(""), typed: join(mine), answer: join(theirs) };
}
