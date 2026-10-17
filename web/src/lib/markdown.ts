import { Marked } from "marked";
import temml from "temml";

/** File extensions shown as sounds rather than images. Anki's `[sound:…]` also takes
 *  video files, of which only the audio is played. */
const SOUND_EXTENSIONS = new Set([
  "mp3", "m4a", "m4b", "aac", "wav", "aif", "aiff", "caf", "flac",
  "ogg", "oga", "opus", "mp4", "m4v", "mov", "webm",
]);

export function extension(path: string): string {
  const name = path.split(/[?#]/)[0].split("/").pop() ?? "";
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

export function isSound(path: string): boolean {
  return SOUND_EXTENSIONS.has(extension(path));
}

export type Block =
  | { type: "text"; html: string }
  /** `src` is null when the path leaves the collection. */
  | { type: "image"; alt: string; src: string | null; path: string }
  /** Sounds next to each other share a row. */
  | { type: "sounds"; sounds: { src: string | null; path: string }[] };

const escapes: Record<string, string> = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };
export const escapeHTML = (s: string) => s.replace(/[&<>"']/g, (c) => escapes[c]);

/** Index of the first `needle` in `src` not escaped with a backslash. */
function unescapedIndex(src: string, needle: string): number | undefined {
  for (let i = src.indexOf(needle); i >= 0; i = src.indexOf(needle, i + 1)) {
    let slashes = 0;
    while (src[i - 1 - slashes] === "\\") slashes++;
    if (slashes % 2 === 0) return i;
  }
  return undefined;
}

/** Length of `$…$` or `$$…$$` math at the start of `src`, or null. As in Pandoc, a single
 *  `$` opens only before a non-space and closes only after a non-space and before a
 *  non-digit, so prices like `$5 and $10` stay text. Kept in step with `math_len` in
 *  `core/src/parser.rs`, which decides what a cloze treats as math. */
export function mathLength(src: string): number | null {
  const display = src.startsWith("$$");
  const body = src.slice(display ? 2 : 1);
  if (!src.startsWith("$") || body === "" || (!display && /^\s/.test(body))) return null;
  for (let i = 0; i < body.length; i++) {
    const c = body[i];
    if (c === "\\") {
      i++;
    } else if (c === "$" && i > 0) {
      if (display) {
        if (body[i + 1] === "$") return i + 4;
      } else if (!/\s/.test(body[i - 1]) && !/\d/.test(body[i + 1] ?? "")) {
        return i + 2;
      }
    }
  }
  return null;
}

/** Inline Markdown only. Card text is never HTML: tags are shown as typed. Math becomes
 *  MathML, which WebKit lays out natively in the system's math font; with `compact`, display
 *  math stays on the line and nothing wraps, as in a table cell. */
function renderer(compact: boolean): Marked {
  const marked = new Marked({ gfm: true, async: false });
  marked.use({
    extensions: [
      // `**…**` is bold wherever it is, even inside a word next to punctuation, as in a
      // cloze gap `시**[…]**요`. Markdown's own rule would leave those stars showing.
      {
        name: "strongAnywhere",
        level: "inline",
        start: (src: string) => unescapedIndex(src, "**"),
        tokenizer(src: string) {
          const match = /^\*\*(?!\s)((?:\\.|[^\\])+?)(?<!\s)\*\*(?!\*)/.exec(src);
          if (!match) return undefined;
          return { type: "strong", raw: match[0], text: match[1], tokens: this.lexer.inlineTokens(match[1]) };
        },
      },
      {
        name: "math",
        level: "inline",
        start: (src: string) => unescapedIndex(src, "$"),
        tokenizer(src: string) {
          const length = mathLength(src);
          if (length === null) return undefined;
          const display = src.startsWith("$$");
          const delimiter = display ? 2 : 1;
          return { type: "math", raw: src.slice(0, length), tex: src.slice(delimiter, length - delimiter), display };
        },
        renderer(token) {
          const display = (token.display as boolean) && !compact;
          const html = temml.renderToString(token.tex as string, {
            displayMode: display,
            // Groups that end after a relation or operator, where TeX would break a line.
            wrap: display || compact ? "none" : "tex",
            // Mistakes show in red with the TeX as written, rather than failing the card.
            throwOnError: false,
          });
          return display || compact ? html : breakable(html);
        },
      },
    ],
    renderer: {
      html: ({ text }) => escapeHTML(text),
      // Images are taken out before rendering; any left (inside a link) show their alt text.
      image: ({ text }) => escapeHTML(text),
    },
  });
  return marked;
}

/** Inline MathML as one `<math>` per group Temml marked, with a line break allowed between
 *  them. WebKit never breaks inside a `<math>`, so a long formula would otherwise run off
 *  the card. */
function breakable(html: string): string {
  const match = /^<math([^>]*)>([\s\S]*)<\/math>$/.exec(html);
  if (!match) return html;
  const [, attributes, body] = match;
  const groups: string[] = [];
  let depth = 0;
  let start = 0;
  for (const tag of body.matchAll(/<(\/?)mrow\b[^>]*?(\/?)>/g)) {
    if (tag[2]) continue;
    if (tag[1]) {
      depth--;
      if (depth === 0) {
        groups.push(body.slice(start, tag.index + tag[0].length));
        start = tag.index + tag[0].length;
      }
    } else {
      if (depth === 0 && body.slice(start, tag.index).trim()) return html;
      depth++;
    }
  }
  if (groups.length < 2 || depth !== 0 || body.slice(start).trim()) return html;
  return groups.map((group) => `<math${attributes}>${group}</math>`).join("<wbr>");
}

const card = renderer(false);
const cell = renderer(true);

/** Card Markdown as HTML. `compact` keeps display math on the line, for table cells. */
export function renderInline(source: string, compact = false): string {
  return (compact ? cell : card).parseInline(source) as string;
}

/** `![alt](path)` or `![alt](<path>)`; alt text may hold balanced brackets. */
const MEDIA = /!\[((?:[^\[\]]|\[[^\[\]]*\])*)\]\((?:<([^>\n]*)>|([^)\s]*))\)/g;

/** A card's Markdown as rows: text, images, and rows of sounds. Media paths are
 *  relative to the deck file at `cardPath`. */
export function parseBlocks(source: string, cardPath: string): Block[] {
  const blocks: Block[] = [];
  const addText = (text: string) => {
    // Media sit on rows of their own, so space next to them is not needed.
    const trimmed = text.trim();
    if (trimmed) blocks.push({ type: "text", html: renderInline(trimmed) });
  };
  let rest = 0;
  for (const match of source.matchAll(MEDIA)) {
    let before = source.slice(rest, match.index);
    let end = match.index + match[0].length;
    // Drop emphasis wrapped around the media, as in a cloze answer `**![](x.png)**`.
    const starsBefore = before.length - before.replace(/\*+$/, "").length;
    const starsAfter = source.slice(end).match(/^\*+/)?.[0].length ?? 0;
    const stars = Math.min(starsBefore, starsAfter);
    before = before.slice(0, before.length - stars);
    end += stars;
    addText(before);

    const angled = match[2] !== undefined;
    const path = match[2] ?? match[3] ?? "";
    const src = resolveMedia(path, angled, cardPath);
    if (isSound(path)) {
      const last = blocks.at(-1);
      if (last?.type === "sounds") last.sounds.push({ src, path });
      else blocks.push({ type: "sounds", sounds: [{ src, path }] });
    } else {
      blocks.push({ type: "image", alt: match[1], src, path });
    }
    rest = end;
  }
  addText(source.slice(rest));
  return blocks;
}

/** Every sound linked in `source`, in order. */
export function soundSources(source: string, cardPath: string): string[] {
  return parseBlocks(source, cardPath).flatMap((block) =>
    block.type === "sounds" ? block.sounds.flatMap((s) => (s.src ? [s.src] : [])) : [],
  );
}

/** URL of a media link written in a card. Relative paths are relative to the deck
 *  file and must stay inside the collection, served from `amgi://collection/`. */
export function resolveMedia(path: string, angled: boolean, cardPath: string): string | null {
  if (/^[a-z][a-z0-9+.-]*:/i.test(path)) return path;
  // `<path>` is literal; a bare destination may be percent-encoded.
  let literal = path;
  if (!angled) {
    try {
      literal = decodeURIComponent(path);
    } catch {
      // Not valid percent-encoding, so it was meant literally.
    }
  }
  const parts = cardPath.split("/").slice(0, -1);
  for (const segment of literal.split("/")) {
    if (segment === "" || segment === ".") continue;
    if (segment === "..") {
      if (parts.length === 0) return null;
      parts.pop();
    } else {
      parts.push(segment);
    }
  }
  return "amgi://collection/" + parts.map(encodeURIComponent).join("/");
}

/** The file name a media link points at, for labels. */
export function fileName(path: string): string {
  const name = path.split("/").pop() ?? path;
  try {
    return decodeURIComponent(name);
  } catch {
    return name;
  }
}
