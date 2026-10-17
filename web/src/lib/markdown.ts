import { Marked } from "marked";

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

// Inline Markdown only. Card text is never HTML: tags are shown as typed.
const inline = new Marked({ gfm: true, async: false });
inline.use({
  // `**…**` is bold wherever it is, even inside a word next to punctuation, as in a
  // cloze gap `시**[…]**요`. Markdown's own rule would leave those stars showing.
  extensions: [
    {
      name: "strongAnywhere",
      level: "inline",
      start: (src: string) => {
        const i = src.indexOf("**");
        return i < 0 ? undefined : i;
      },
      tokenizer(src: string) {
        const match = /^\*\*(?!\s)((?:\\.|[^\\])+?)(?<!\s)\*\*(?!\*)/.exec(src);
        if (!match) return undefined;
        return { type: "strong", raw: match[0], text: match[1], tokens: this.lexer.inlineTokens(match[1]) };
      },
    },
  ],
  renderer: {
    html: ({ text }) => escapeHTML(text),
    // Images are taken out before rendering; any left (inside a link) show their alt text.
    image: ({ text }) => escapeHTML(text),
  },
});

export function renderInline(source: string): string {
  return inline.parseInline(source) as string;
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
