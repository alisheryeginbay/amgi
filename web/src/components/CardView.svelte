<script lang="ts">
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

  /** A card's two sides, one above the other, as previewed beside the browse table. */
  let { card }: { card: Card } = $props();
  const cloze = $derived(card.kind === "cloze");
</script>

<article>
  <header>
    <span class="deck">{card.deck}</span>
    <TagPills tags={card.tags} />
  </header>

  <div class="front">
    <Markdown source={card.front} cardPath={card.path} emphasis={cloze} />
  </div>
  <hr />
  <div class="back">
    <Markdown source={card.back} cardPath={card.path} emphasis={cloze} />
  </div>
</article>

<style>
  article {
    max-width: 600px;
    margin: 0 auto;
  }

  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    margin-bottom: 14px;
    font-size: 11px;
  }

  .deck {
    flex: none;
    color: var(--tertiary-label);
  }

  .front {
    font-size: var(--font-title2);
  }

  hr {
    margin: 18px 0;
    border: none;
    border-top: 1px solid var(--separator);
  }

  .back {
    font-size: var(--font-title3);
    color: var(--secondary-label);
  }

  /* Smaller images in the narrow preview. */
  article :global(img) {
    max-height: 240px;
  }
</style>
