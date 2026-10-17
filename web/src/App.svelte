<script lang="ts">
  import Browse from "./browse/Browse.svelte";
  import Dashboard from "./dashboard/Dashboard.svelte";
  import Study from "./study/Study.svelte";
  import { app } from "./lib/app.svelte";
</script>

{#if app.pane === "study"}
  <Study />
{:else if app.pane === "dashboard"}
  <Dashboard />
{:else if app.pane === "browse"}
  <Browse />
{/if}
