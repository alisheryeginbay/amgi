<script lang="ts">
  import { app } from "../lib/app.svelte";

  let { tags }: { tags: string[] } = $props();
</script>

<span class="pills">
  <!-- By position: front matter can repeat a tag. -->
  {#each tags as tag, i (i)}
    {@const icon = app.icons[tag.toLowerCase()]}
    <span class="pill">{icon ? `${icon} ${tag}` : tag}</span>
  {/each}
</span>

<style>
  .pills {
    display: inline-flex;
    gap: 4px;
    min-width: 0;
    overflow: hidden;
  }

  .pill {
    flex: none;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--fill);
    color: var(--secondary-label);
    white-space: nowrap;
  }
</style>
