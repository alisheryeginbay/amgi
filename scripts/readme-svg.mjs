// Builds media/amgi.svg for the README: "Amgi" is written by hand, fades, then 암기
// is written, fades, and the loop starts over. Each word comes from the tegaki CLI
// (https://github.com/gkurt/tegaki); this script puts both on one timeline.
//
//   node scripts/readme-svg.mjs

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const OUT = join(dirname(fileURLToPath(import.meta.url)), "..", "media", "amgi.svg");
const TEGAKI = "tegaki@1.1.0"; // pinned: the script parses this version's output
const LIGHT = "#1a1a1a";
const DARK = "#e6edf3";

const words = [
  { text: "Amgi", args: ["--font", "caveat", "--size", "100"] },
  { text: "암기", args: ["--font", "nanum-pen-script", "--size", "130"] },
];

function tegaki(text, args) {
  return execFileSync(
    "npx",
    ["-y", TEGAKI, text, "--mode", "loop", "--color", LIGHT, "-o", "-", ...args],
    { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] },
  );
}

function parse(svg) {
  const [, x, y, w, h] = svg.match(/viewBox="([\d.-]+) ([\d.-]+) ([\d.-]+) ([\d.-]+)"/).map(Number);
  const style = svg.match(/<style>([\s\S]*?)<\/style>/)[1];
  const body = svg.slice(svg.indexOf("</style>") + 8, svg.lastIndexOf("</svg>"));
  const duration = Number(style.match(/animation: \S+ ([\d.]+)s/)[1]);
  const anims = [...style.matchAll(/@keyframes (\S+) \{ (.*?) \} \} /g)].map((m) => ({
    name: m[1],
    frames: [...m[2].concat(" }").matchAll(/([\d.%,]+) \{ ([^}]*?) \}/g)].flatMap((f) =>
      f[1].split(",").map((at) => ({ at: parseFloat(at), decl: f[2].trim() })),
    ),
  }));
  return { x, y, w, h, duration, anims, body };
}

const parts = words.map((word) => parse(tegaki(word.text, word.args)));
const total = parts.reduce((sum, p) => sum + p.duration, 0);
const width = Math.max(...parts.map((p) => p.w));
const height = Math.max(...parts.map((p) => p.h));
// Keep enough precision that tegaki's 0.001% steps never collapse onto one keyframe.
const pct = (n) => `${Number(n.toFixed(5))}%`;

let offset = 0;
const css = [];
const groups = [];
parts.forEach((part, i) => {
  const prefix = `w${i}-`;
  const start = (offset / total) * 100;
  const scale = part.duration / total;
  for (const { name, frames } of part.anims) {
    // Outside its own slot a word holds its first frame (before) or last frame (after);
    // tegaki makes both the hidden state.
    const out = frames.map((f) => `${pct(start + f.at * scale)} { ${f.decl} }`);
    if (start > 0) out.unshift(`0% { ${frames[0].decl.replace(/;?\s*animation-timing-function:[^;]*/, "")} }`);
    if (start + 100 * scale < 99.999) out.push(`100% { ${frames.at(-1).decl} }`);
    const id = prefix + name;
    css.push(`@keyframes ${id} { ${out.join(" ")} } .${id} { animation: ${id} ${total}s linear infinite }`);
  }
  // Center each word on the shared canvas.
  const dx = (width - part.w) / 2 - part.x;
  const dy = (height - part.h) / 2 - part.y;
  const body = part.body
    .replace(/\btk-/g, prefix + "tk-")
    .trim();
  groups.push(`<g transform="translate(${dx.toFixed(2)} ${dy.toFixed(2)})">\n${body}\n</g>`);
  offset += part.duration;
});

css.push(
  `@media (prefers-color-scheme: dark) { [stroke="${LIGHT}"] { stroke: ${DARK} } [fill="${LIGHT}"] { fill: ${DARK} } }`,
);

const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width.toFixed(2)} ${height.toFixed(2)}" width="${width.toFixed(2)}" height="${height.toFixed(2)}" role="img" aria-label="Amgi, 암기">
<style>${css.join("\n")}</style>
${groups.join("\n")}
</svg>
`;

mkdirSync(dirname(OUT), { recursive: true });
writeFileSync(OUT, svg);
console.log(`wrote ${OUT} (${(svg.length / 1024).toFixed(1)} KB, ${total.toFixed(2)}s loop)`);
