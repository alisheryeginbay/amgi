<script lang="ts">
  import { untrack } from "svelte";
  import Markdown from "../components/Markdown.svelte";
  import { compare } from "../lib/diff";
  import { clozeGap, oneLine, renderInline } from "../lib/markdown";
  import type { StudyCard } from "../lib/types";

  /** A card to study, drawn on the sheet `Study` lays under it. Once `revealed`, a basic card's answer
   *  fades in below its question, which stays where it is, and a cloze's gap opens to its
   *  answer in the sentence. An `instant` reveal, from a key, shows the answer at once. */
  let {
    card,
    revealed,
    instant,
    onreveal,
  }: { card: StudyCard; revealed: boolean; instant: boolean; onreveal: () => void } = $props();

  const cloze = $derived(card.kind === "cloze");
  /** Null for a cloze answered with an image or sound, which swaps the whole card. */
  const gap = $derived(cloze ? clozeGap(card.front, card.back) : null);

  const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;

  /** What was typed, for a card that asks for its answer. */
  let typedText = $state("");
  /** The answer to type as plain text, without its images and sounds. */
  const expected = $derived.by(() => {
    if (card.typed == null) return "";
    const media = /!\[(?:[^\[\]]|\[[^\[\]]*\])*\]\((?:<[^>\n]*>|[^)\s]*)\)/g;
    const html = renderInline(oneLine(card.typed.replace(media, "")));
    return new DOMParser().parseFromString(html, "text/html").body.textContent ?? "";
  });
  const comparison = $derived(revealed && typedText.trim() ? compare(typedText, expected) : null);
  /** What the typing box asks for: the gap's hint, else the note field. */
  const placeholder = $derived(
    card.typedHint ?? (card.typedField ? `Type the ${card.typedField}` : cloze ? "…" : "Type the answer"),
  );

  /** A cloze whose gap is typed: the box stands in the gap until it's answered. It's
   *  made here, not in the template, because the gap is inside the card's rendered HTML. */
  $effect(() => {
    if (!gap || card.typed == null || revealed) return;
    const hint = live.querySelector(".gap .hint");
    if (!hint) return;
    const box = document.createElement("input");
    const [value, hintText] = untrack(() => [typedText, placeholder]);
    Object.assign(box, { type: "text", className: "typed-gap", value, placeholder: hintText });
    box.setAttribute("autocomplete", "off");
    box.setAttribute("autocapitalize", "off");
    box.setAttribute("spellcheck", "false");
    box.setAttribute("aria-label", "Your answer");
    // Wide enough for the hint, growing with the text.
    const fit = () => (box.style.width = `${Math.max(box.value.length, box.placeholder.length, 3) + 1.5}ch`);
    fit();
    box.addEventListener("input", () => {
      typedText = box.value;
      fit();
    });
    box.addEventListener("keydown", onTypedKey);
    const shown = [...hint.childNodes];
    hint.replaceChildren(box);
    box.focus({ preventScroll: true });
    return () => hint.replaceChildren(...shown);
  });

  /** Focus the box as the card appears, so typing goes straight in. */
  function focusOnMount(node: HTMLInputElement) {
    node.focus({ preventScroll: true });
  }

  function onTypedKey(event: KeyboardEvent) {
    if (event.key === "Enter" && !event.isComposing) {
      event.preventDefault();
      onreveal();
    }
  }

  // A cloze's gap widens from its hint to its answer, so the words around it slide
  // aside. Its width is measured before the answer shows, and again after.
  let live: HTMLElement;
  let wasRevealed = untrack(() => revealed);
  let gapWidth: number | null = null;

  $effect.pre(() => {
    if (revealed && !wasRevealed) gapWidth = live?.querySelector(".gap")?.getBoundingClientRect().width ?? null;
  });

  $effect(() => {
    const now = revealed;
    untrack(() => {
      if (now && !wasRevealed && gapWidth !== null && !reduceMotion && !instant) widenGap(gapWidth);
      wasRevealed = now;
      gapWidth = null;
    });
  });

  function widenGap(from: number) {
    const gap = live.querySelector<HTMLElement>(".gap");
    if (!gap) return;
    gap.classList.add("widening");
    const to = gap.getBoundingClientRect().width;
    const done = () => gap.classList.remove("widening");
    if (Math.abs(to - from) < 1) return done();
    const easing = getComputedStyle(gap).getPropertyValue("--ease-out");
    gap.animate([{ width: `${from}px` }, { width: `${to}px` }], { duration: 200, easing }).finished.then(done, done);
  }
</script>

{#snippet compared(shown: boolean)}
  {#if shown && comparison}
    <div class="comparison" class:correct={comparison.correct}>
      {#if comparison.correct}
        <span class="mark" aria-label="Right">✓</span> {typedText.trim()}
      {:else}
        <!-- prettier-ignore -->
        <div class="yours"><span class="mark" aria-label="Typed">✗</span> {#each comparison.typed as piece, i (i)}<span class={piece.kind}>{piece.text}</span>{/each}</div>
        <!-- prettier-ignore -->
        <div class="right"><span class="mark" aria-hidden="true"></span> {#each comparison.answer as piece, i (i)}<span class={piece.kind}>{piece.text}</span>{/each}</div>
      {/if}
    </div>
  {/if}
{/snippet}

{#snippet content(shown: boolean)}
  <div class="body" class:shown>
    {#if gap}
      <Markdown source={gap.source} {gap} cardPath={card.path} emphasis />
      {#if card.typed != null}
        <div class="gap-comparison">{@render compared(shown)}</div>
      {/if}
    {:else if cloze}
      <div class="swap">
        <div class="side" inert={shown}>
          <Markdown source={card.front} cardPath={card.path} emphasis />
        </div>
        <div class="side answer-side" inert={!shown}>
          <Markdown source={card.back} cardPath={card.path} emphasis />
        </div>
      </div>
    {:else}
      <div class="question">
        <Markdown source={card.front} cardPath={card.path} inlineSounds />
        {#if card.typed != null}
          {#if !shown}
            <input
              class="typed"
              type="text"
              bind:value={typedText}
              {placeholder}
              autocomplete="off"
              autocapitalize="off"
              spellcheck="false"
              aria-label={card.typedField ?? "Your answer"}
              use:focusOnMount
              onkeydown={onTypedKey}
            />
          {:else}
            {@render compared(shown)}
          {/if}
        {/if}
      </div>
      <!-- Takes no room until it shows, so the card's size gives nothing away. -->
      <div class="answer" inert={!shown}>
        <Markdown source={card.back} cardPath={card.path} inlineSounds />
      </div>
    {/if}
  </div>
{/snippet}

<div class="card" class:revealed class:instant>
  <div class="layers">
    <div class="layer" bind:this={live}>{@render content(revealed)}</div>
    {#if cloze}
      <!-- An unseen copy of the cloze answered holds its size, so the sentence doesn't
           move while its gap opens. -->
      <div class="layer sizer" aria-hidden="true" inert>{@render content(true)}</div>
    {/if}
  </div>
</div>

<style>
  .card {
    display: flex;
    flex-direction: column;
    min-height: 180px;
  }

  .layers {
    flex: 1;
    display: grid;
  }

  .layer {
    grid-area: 1 / 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .sizer {
    visibility: hidden;
  }

  /* Text starts at the top of the card; long text grows it downward. */
  .body {
    flex: 1;
    display: flex;
    flex-direction: column;
    padding: 36px 36px;
    font-size: 22px;
    text-align: center;
  }

  .body :global(.markdown) {
    align-items: center;
  }

  /* Basic: the question, then the answer below it. */
  .question,
  .answer {
    text-align: left;
  }

  .question :global(.markdown),
  .answer :global(.markdown) {
    align-items: flex-start;
  }


  /* A typed answer: a box under the question, then what was typed against the answer. */
  .typed {
    display: block;
    width: 100%;
    margin: 18px 0 6px;
    padding: 6px 10px;
    border: 1px solid var(--separator);
    border-radius: 8px;
    background: var(--text-background);
    color: var(--label);
    font: inherit;
    font-size: 18px;
    outline: none;
  }

  .typed:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 25%, transparent);
  }

  .typed::placeholder {
    color: var(--tertiary-label);
  }

  /* A typed gap: a box in the sentence, with what was typed under it once answered. */
  .body :global(.typed-gap) {
    max-width: 100%;
    padding: 0 0.2em;
    border: none;
    border-bottom: 2px solid var(--accent);
    border-radius: 0;
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    color: var(--accent);
    font: inherit;
    font-weight: 600;
    text-align: center;
    outline: none;
  }

  .body :global(.typed-gap::placeholder) {
    color: var(--tertiary-label);
    font-weight: 400;
  }

  .gap-comparison {
    display: flex;
    justify-content: center;
    text-align: left;
  }

  .comparison {
    margin: 16px 0 6px;
    font-size: 18px;
    line-height: 1.5;
    white-space: pre-wrap;
  }

  .comparison .mark {
    display: inline-block;
    width: 1.2em;
    font-family: system-ui;
    color: var(--red);
  }

  .comparison.correct .mark {
    color: var(--green);
  }

  .comparison .extra {
    color: var(--red);
    text-decoration: line-through;
  }

  .comparison .missing {
    color: var(--green);
    background: color-mix(in srgb, var(--green) 14%, transparent);
    border-radius: 3px;
  }

  /* The answer is ruled off by a dashed line across the whole card, edge to edge. */
  .answer {
    display: none;
    margin: 28px -36px 0;
    padding: 28px 36px 0;
    border-top: 1px dashed var(--separator);
  }

  .shown .answer {
    display: block;
    transition: opacity 0.14s var(--ease-out);

    @starting-style {
      opacity: 0;
    }
  }

  /* Cloze: the gap shows its hint, then its answer. */
  .body:not(.shown) :global(.gap .fill),
  .shown :global(.gap .hint) {
    display: none;
  }

  .shown :global(.gap .fill) {
    animation: -global-gap-fill 0.2s var(--ease-out) both;
  }

  /* While widening, the gap is one box, its answer fading out past the edge. */
  .body :global(.gap.widening) {
    display: inline-block;
    max-width: 100%;
    white-space: nowrap;
    mask-image: linear-gradient(to right, #000 calc(100% - 0.5em), transparent);
  }

  @keyframes -global-gap-fill {
    from {
      opacity: 0;
      filter: blur(5px);
    }
  }

  /* Cloze answered with media: one side fades to the other. */
  .swap {
    display: grid;
  }

  .side {
    grid-area: 1 / 1;
    transition:
      opacity 0.2s var(--ease-out),
      visibility 0.2s;
  }

  .shown .side:not(.answer-side),
  .body:not(.shown) .answer-side {
    opacity: 0;
    visibility: hidden;
  }

  /* Shown by a key: at once. */
  .instant .answer,
  .instant .side {
    transition: none;
  }

  .instant :global(.gap .fill) {
    animation: none;
  }

  @media (prefers-reduced-motion: reduce) {
    .shown :global(.gap .fill) {
      animation: none;
    }
  }
</style>
