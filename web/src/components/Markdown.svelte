<script lang="ts">
  import CardImage from "./CardImage.svelte";
  import SoundButton from "./SoundButton.svelte";
  import { parseBlocks } from "../lib/markdown";

  /** Card Markdown that keeps its line breaks, with images and sounds on rows of their
   *  own. With `emphasis`, bold runs (cloze gaps and answers) are tinted. */
  let { source, cardPath, emphasis = false }: { source: string; cardPath: string; emphasis?: boolean } = $props();
  const blocks = $derived(parseBlocks(source, cardPath));
</script>

<div class="markdown" class:emphasis>
  {#each blocks as block, i (i)}
    {#if block.type === "text"}
      <!-- Escaped when rendered: card text never holds HTML. -->
      <div class="text">{@html block.html}</div>
    {:else if block.type === "image"}
      <CardImage src={block.src} alt={block.alt} path={block.path} />
    {:else}
      <div class="sounds">
        {#each block.sounds as sound, j (j)}
          <SoundButton src={sound.src} path={sound.path} />
        {/each}
      </div>
    {/if}
  {/each}
</div>

<style>
  .markdown {
    display: flex;
    flex-direction: column;
    gap: 16px;
    align-items: flex-start;
  }

  .text {
    /* Full width, so display math centers on the card. */
    align-self: stretch;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    line-height: 1.45;
    -webkit-user-select: text;
    user-select: text;
    cursor: text;
  }

  /* Math sets its own spacing, a touch larger to match the text's weight. */
  .text :global(math) {
    white-space: normal;
    font-size: 1.1em;
  }

  /* Each group of a long formula is its own <math>, placed in the line like a word, so
     the line can break between them. Temml's flex layout is for Chromium. */
  .text :global(math:not(.tml-display)) {
    display: inline;
  }

  .text :global(math > mrow) {
    padding: 0;
  }

  .text :global(math.tml-display) {
    margin: 0.4em 0;
  }

  .emphasis :global(strong) {
    color: var(--accent);
  }

  .sounds {
    display: flex;
    gap: 8px;
  }
</style>
