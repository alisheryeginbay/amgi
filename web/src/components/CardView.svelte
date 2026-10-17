<script lang="ts">
  import { fade } from "svelte/transition";
  import Markdown from "./Markdown.svelte";
  import TagPills from "./TagPills.svelte";
  import type { CardKind } from "../lib/types";

  interface Card {
    deck: string;
    tags: string[];
    path: string;
    kind: CardKind;
    front: string;
    back: string;
  }

  /** A card as studied: the question, then the answer once `revealed`. A cloze shows its
   *  answer in place of the question while studying; `preview` shows both, one above the
   *  other, in smaller type. */
  let { card, revealed, preview = false }: { card: Card; revealed: boolean; preview?: boolean } = $props();
</script>

<article class:preview>
  <header>
    <span class="deck">{card.deck}</span>
    <TagPills tags={card.tags} />
  </header>

  {#if card.kind === "basic" || preview}
    <div class="front">
      <Markdown source={card.front} cardPath={card.path} emphasis={card.kind === "cloze"} />
    </div>
    {#if revealed}
      <div class="answer" in:fade={{ duration: preview ? 0 : 250 }}>
        <hr />
        <div class="back">
          <Markdown source={card.back} cardPath={card.path} emphasis={card.kind === "cloze"} />
        </div>
      </div>
    {/if}
  {:else}
    <div class="front">
      <Markdown source={revealed ? card.back : card.front} cardPath={card.path} emphasis />
    </div>
  {/if}
</article>

<style>
  article {
    max-width: 600px;
    margin: 0 auto;
  }

  header {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 20px;
    font-size: 11px;
  }

  .deck {
    flex: none;
    color: var(--tertiary-label);
  }

  .front {
    font-size: var(--font-title);
  }

  hr {
    margin: 28px 0;
    border: none;
    border-top: 1px solid var(--separator);
  }

  .back {
    font-size: var(--font-title2);
    color: var(--secondary-label);
  }

  .preview header {
    flex-wrap: wrap;
    margin-bottom: 14px;
  }

  .preview .front {
    font-size: var(--font-title2);
  }

  .preview hr {
    margin: 18px 0;
  }

  .preview .back {
    font-size: var(--font-title3);
  }

  /* Smaller images in the narrow preview. */
  .preview :global(img) {
    max-height: 240px;
  }
</style>
