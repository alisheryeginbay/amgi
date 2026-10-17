<script lang="ts">
  import CardImage from "./CardImage.svelte";
  import SoundButton from "./SoundButton.svelte";
  import { GAP, loadLanguage, parseBlocks } from "../lib/markdown";

  /** Card Markdown that keeps its line breaks, with images and sounds on rows of their
   *  own. With `emphasis`, bold runs (cloze gaps and answers) are tinted. A `gap`, as
   *  from `clozeGap`, fills the `GAP` in `source` with both its hint and its answer, for
   *  the card to show one or the other. */
  let {
    source,
    cardPath,
    emphasis = false,
    gap,
  }: { source: string; cardPath: string; emphasis?: boolean; gap?: { hint: string; answer: string } } = $props();
  // Bumped when a code block's grammar arrives, so the block is highlighted again.
  let grammars = $state(0);
  const blocks = $derived.by(() => {
    void grammars;
    return parseBlocks(source, cardPath);
  });
  $effect(() => {
    for (const block of blocks) {
      if (block.type === "code") loadLanguage(block.language)?.then(() => grammars++);
    }
  });

  const html = (text: string) =>
    gap
      ? text.replace(
          `<strong>${GAP}</strong>`,
          `<span class="gap"><span class="hint">${gap.hint}</span><span class="fill">${gap.answer}</span></span>`,
        )
      : text;
</script>

<div class="markdown" class:emphasis>
  {#each blocks as block, i (i)}
    {#if block.type === "text"}
      <!-- Escaped when rendered: card text never holds HTML. -->
      <div class="text">{@html html(block.html)}</div>
    {:else if block.type === "code"}
      <!-- Escaped by the highlighter. -->
      <pre class="code"><code data-language={block.language || undefined}>{@html block.html}</code></pre>
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

  .code {
    align-self: stretch;
    margin: 0;
    padding: 10px 12px;
    border-radius: 8px;
    background: var(--fill);
    overflow-x: auto;
    /* Code reads at a smaller size than the card's large text, about 14px on a study
       card, and never smaller than the 12px preview text. */
    font-size: max(13.5px, 0.6em);
    line-height: 1.5;
    text-align: left;
    -webkit-user-select: text;
    user-select: text;
    cursor: text;
  }

  /* Shiki's GitHub themes, following the system appearance. */
  .code :global(span) {
    color: var(--shiki-light);
    font-style: var(--shiki-light-font-style);
    font-weight: var(--shiki-light-font-weight);
  }

  @media (prefers-color-scheme: dark) {
    .code :global(span) {
      color: var(--shiki-dark);
      font-style: var(--shiki-dark-font-style);
      font-weight: var(--shiki-dark-font-weight);
    }
  }

  /* A cloze's gap inside code. */
  .code :global(.gap),
  .code :global(.gap span) {
    color: var(--accent);
    font-weight: 600;
  }

  .code :global(.gap) {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }

  .emphasis :global(strong) {
    color: var(--accent);
  }

  .sounds {
    display: flex;
    gap: 8px;
  }
</style>
