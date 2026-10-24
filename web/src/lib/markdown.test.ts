import assert from "node:assert/strict";
import { test } from "node:test";
import { clozeGap, GAP, highlight, imageSources, loadLanguage, oneLine, renderInline, splitFences } from "./markdown.ts";

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

test("splits fenced code blocks from text", () => {
  const source = "Some code:\n```Python extra\ndef f():\n\n    return [1]\n```\nafter\n~~~\n```\nunclosed";
  assert.deepEqual(splitFences(source), [
    { text: "Some code:" },
    { code: "def f():\n\n    return [1]", language: "python" },
    { text: "after" },
    { code: "```\nunclosed", language: "" },
  ]);
  assert.deepEqual(splitFences("inline ```code``` stays"), [{ text: "inline ```code``` stays" }]);
});

test("highlights once a language is loaded, and escapes until then", async () => {
  assert.equal(highlight("x <- '<b>'", "r"), "x &lt;- &#39;&lt;b&gt;&#39;");
  await loadLanguage("r");
  assert.equal(loadLanguage("r"), null);
  const html = highlight("x <- 1 # one\ny", "r");
  assert.match(html, /<span style="--shiki-light:#[0-9A-F]+;--shiki-dark:#[0-9A-F]+[^"]*"> # one<\/span>/i);
  assert.match(html, /<br>/);
  // Aliases load too; unknown languages stay plain.
  await loadLanguage("py");
  assert.match(highlight("def f(): pass", "py"), /--shiki-light/);
  assert.equal(loadLanguage("nonsense"), null);
  assert.equal(highlight("<b>", "nonsense"), "&lt;b&gt;");
});

test("puts code blocks on one line for table cells", () => {
  assert.equal(oneLine("Q\n```r\nx <- `a`\n\ny\n```\nend"), "Q `` x <- `a` y `` end");
});

test("marks a cloze's gap in code", async () => {
  await loadLanguage("r");
  const html = highlight('library("\uE001[…]\uE002")\nx', "r");
  assert.match(html, /class="gap">\[…\]<\/span>/);
  assert.doesNotMatch(html, /[\uE001\uE002]/);
  assert.equal(highlight("a \uE001<b>\uE002 c", ""), 'a <span class="gap">&lt;b&gt;</span> c');
  assert.equal(renderInline("`f(\uE001x\uE002)`"), "<code>f(<strong>x</strong>)</code>");
});

test("lists a side's images, leaving out sounds and paths outside the collection", () => {
  const source = "A tree\n![oak](oak.png)\n![](birdsong.mp3)\n![](../../outside.png)";
  assert.deepEqual(imageSources(source, "plants/trees.md"), ["amgi://collection/plants/oak.png"]);
});

test("reads paths starting `@/` from the collection root", () => {
  const source = "![](@/media/oak.png)\n![](@/../outside.png)";
  assert.deepEqual(imageSources(source, "cards/plants/trees.md"), ["amgi://collection/media/oak.png"]);
});

test("keeps a formula with a mistake on one line in table cells", () => {
  const cell = renderInline(oneLine("Area\n$$$\nx^2 + y\n$$$\nof a square"), true);
  assert.ok(!cell.includes("pre-line"));
  assert.ok(!cell.includes("\n"));
  assert.ok(cell.includes(`<span class="temml-error" style="color:#b22222">$$$ x^2 + y $$</span>`));
  // On the card itself, the error keeps Temml's message.
  assert.ok(renderInline("$\\frac{$").includes("ParseError"));
  // Temml throws on some mistakes even when asked not to.
  assert.equal(renderInline("$x^$"), `<span class="temml-error" style="color:#b22222">$x^$</span>`);
});
