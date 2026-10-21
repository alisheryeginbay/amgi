<script lang="ts">
  import Markdown from "./Markdown.svelte";

  /** A note's fields as a table of names and values, under the card previewed beside
   *  the browse table. Empty fields are left out. */
  let { names, values, cardPath }: { names: string[]; values: string[]; cardPath: string } = $props();

  const filled = $derived(names.map((name, i) => ({ name, value: values[i] ?? "" })).filter((f) => f.value.trim()));
</script>

<table>
  <thead>
    <tr><th>Field</th><th>Value</th></tr>
  </thead>
  <tbody>
    {#each filled as field (field.name)}
      <tr>
        <th scope="row">{field.name}</th>
        <td><Markdown source={field.value} {cardPath} inlineSounds /></td>
      </tr>
    {/each}
  </tbody>
</table>

<style>
  table {
    width: 100%;
    max-width: 600px;
    margin: 0 auto;
    border-collapse: collapse;
    font-size: var(--font-callout);
  }

  thead th {
    padding: 0 8px 4px 0;
    font-size: 11px;
    font-weight: 500;
    text-align: left;
    color: var(--secondary-label);
    border-bottom: 1px solid var(--separator);
  }

  tbody th {
    width: 1%;
    padding: 6px 12px 6px 0;
    font-weight: 400;
    text-align: left;
    vertical-align: top;
    white-space: nowrap;
    color: var(--secondary-label);
  }

  td {
    padding: 6px 0;
    vertical-align: top;
  }

  tbody tr + tr > * {
    border-top: 1px solid var(--separator);
  }

  td :global(img) {
    max-height: 160px;
  }
</style>
