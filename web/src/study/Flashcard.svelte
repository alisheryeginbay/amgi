<script lang="ts">
  import { untrack } from "svelte";
  import Markdown from "../components/Markdown.svelte";
  import { clozeGap } from "../lib/markdown";
  import type { StudyCard } from "../lib/types";

  /** A card to study, set as text on the page. Once `revealed`, a basic card's answer
   *  fades in below its question, which stays where it is, and a cloze's gap opens to its
   *  answer in the sentence. Clicking the card reveals it. */
  let { card, revealed, onreveal }: { card: StudyCard; revealed: boolean; onreveal: () => void } = $props();

  const cloze = $derived(card.kind === "cloze");
  /** Null for a cloze answered with an image or sound, which swaps the whole card. */
  const gap = $derived(cloze ? clozeGap(card.front, card.back) : null);

  const EASE = "cubic-bezier(0.2, 0.7, 0.2, 1)";
  const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;

  function onclick() {
    // Not when the click ends a text selection.
    if (!revealed && (window.getSelection()?.isCollapsed ?? true)) onreveal();
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
      if (now && !wasRevealed && gapWidth !== null && !reduceMotion) widenGap(gapWidth);
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
    gap.animate([{ width: `${from}px` }, { width: `${to}px` }], { duration: 450, easing: EASE }).finished.then(done, done);
  }
</script>

{#snippet content(shown: boolean)}
  <div class="body" class:shown>
    {#if gap}
      <Markdown source={gap.source} {gap} cardPath={card.path} emphasis />
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
      </div>
      <!-- Takes its room before it shows, so the question never moves. -->
      <div class="answer" inert={!shown}>
        <hr />
        <Markdown source={card.back} cardPath={card.path} inlineSounds />
      </div>
    {/if}
  </div>
{/snippet}

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions: Space reveals, handled by the page. -->
<div class="card" class:revealed {onclick}>

  <!-- An unseen copy of the card answered holds its size, so nothing moves the card
       while the answer opens. -->
  <div class="layers">
    <div class="layer" bind:this={live}>{@render content(revealed)}</div>
    <div class="layer sizer" aria-hidden="true" inert>{@render content(true)}</div>
  </div>
</div>

<style>
  .card {
    --ease: cubic-bezier(0.2, 0.7, 0.2, 1);
    display: flex;
    flex-direction: column;
    min-height: 340px;
  }

  .card:not(.revealed) {
    cursor: pointer;
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

  /* Short text sits in the middle of the card; long text grows it. */
  .body {
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: center;
    padding: 36px 20px 10px;
    font-size: 26px;
    text-align: center;
  }

  .body :global(.markdown) {
    align-items: center;
  }

  /* Basic: the question large, the answer a size down beneath a short rule. */
  .question {
    font-size: 30px;
  }

  .answer {
    font-size: 21px;
    opacity: 0;
    transition: opacity 0.14s ease-out;
  }

  .shown .answer {
    opacity: 1;
  }

  hr {
    width: 28px;
    margin: 28px auto 22px;
    border: none;
    border-top: 1px solid var(--separator);
  }

  /* Cloze: the gap shows its hint, then its answer. */
  .body:not(.shown) :global(.gap .fill),
  .shown :global(.gap .hint) {
    display: none;
  }

  .shown :global(.gap .fill) {
    animation: -global-gap-fill 0.45s var(--ease) both;
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
      opacity 0.3s ease-out,
      visibility 0.3s;
  }

  .shown .side:not(.answer-side),
  .body:not(.shown) .answer-side {
    opacity: 0;
    visibility: hidden;
  }

  @media (prefers-reduced-motion: reduce) {
    .shown :global(.gap .fill) {
      animation: none;
    }
  }
</style>
