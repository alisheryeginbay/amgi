<script lang="ts">
  import Icon from "./Icon.svelte";
  import { fileName } from "../lib/markdown";

  let { src, alt, path }: { src: string | null; alt: string; path: string } = $props();
  let failed = $state(false);
  $effect(() => {
    src;
    failed = false;
  });
</script>

{#if src && !failed}
  <img {src} {alt} draggable="false" onerror={() => (failed = true)} />
{:else}
  <span class="missing" title={`Image not found: ${path}`}>
    <Icon name="photo" />
    {alt || fileName(path)}
  </span>
{/if}

<style>
  img {
    display: block;
    max-width: 100%;
    max-height: 360px;
    border-radius: 8px;
    object-fit: contain;
  }

  .missing {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--font-body);
    color: var(--secondary-label);
  }
</style>
