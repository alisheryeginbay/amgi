<script lang="ts">
  import { untrack } from "svelte";
  import Markdown from "../components/Markdown.svelte";
  import TagPills from "../components/TagPills.svelte";
  import { clozeGap } from "../lib/markdown";
  import type { StudyCard } from "../lib/types";

  /** A card to study. Once `revealed`, a basic card's question shrinks and rises to make
   *  room for the answer, and a cloze's gap opens to its answer in the sentence. Clicking
   *  the card reveals it. */
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
        <Markdown source={card.front} cardPath={card.path} />
      </div>
      <div class="reveal" inert={!shown}>
        <div class="inner">
          <hr />
          <div class="answer">
            <Markdown source={card.back} cardPath={card.path} />
          </div>
        </div>
      </div>
    {/if}
  </div>
{/snippet}

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions: Space reveals, handled by the page. -->
<div class="card" class:revealed {onclick}>
  <header>
    <span class="deck">{card.deck}</span>
    <TagPills tags={card.tags} />
  </header>

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
    padding: 18px 24px 36px;
    border-radius: 22px;
    background: var(--card-background);
    box-shadow: var(--card-shadow);
  }

  .card:not(.revealed) {
    cursor: pointer;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    min-height: 18px;
    font-size: 11px;
  }

  .deck {
    flex: none;
    color: var(--tertiary-label);
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
    padding: 28px 20px 10px;
    font-size: 26px;
    letter-spacing: -0.01em;
    text-align: center;
  }

  .body :global(.markdown) {
    align-items: center;
  }

  /* Basic: the question settles small above the answer as it opens beneath. */
  .question {
    transition:
      font-size 0.45s var(--ease),
      color 0.45s var(--ease);
  }

  .shown .question {
    font-size: var(--font-title3);
    letter-spacing: 0;
    color: var(--secondary-label);
  }

  .reveal {
    display: grid;
    grid-template-rows: 0fr;
    transition: grid-template-rows 0.45s var(--ease);
  }

  .shown .reveal {
    grid-template-rows: 1fr;
  }

  .inner {
    min-height: 0;
    overflow: hidden;
  }

  hr {
    margin: 22px 0;
    border: none;
    border-top: 1px solid var(--separator);
    transform: scaleX(0);
    transition: transform 0.55s var(--ease) 0.08s;
  }

  .shown hr {
    transform: none;
  }

  .answer {
    opacity: 0;
    transform: translateY(8px);
    filter: blur(6px);
    transition:
      opacity 0.4s ease-out 0.12s,
      transform 0.5s var(--ease) 0.12s,
      filter 0.4s ease-out 0.12s;
  }

  .shown .answer {
    opacity: 1;
    transform: none;
    filter: none;
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
    .question,
    .reveal,
    hr {
      transition: none;
    }

    hr,
    .answer {
      transform: none;
      filter: none;
    }

    .answer {
      transition: opacity 0.2s;
    }

    .shown :global(.gap .fill) {
      animation: none;
    }
  }
</style>
