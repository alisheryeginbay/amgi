<script lang="ts">
  import Browse from "./browse/Browse.svelte";
  import Dashboard from "./dashboard/Dashboard.svelte";
  import Study from "./study/Study.svelte";
  import { app } from "./lib/app.svelte";

  $effect(() => {
    if (app.environment.han) document.documentElement.dataset.han = app.environment.han;
    else delete document.documentElement.dataset.han;
  });

  // CJK font packs arrive as they finish downloading. One that arrives while CJK text
  // shows waits for the next card, pane or browsed list, so text never changes font while
  // it's being read. Linked before the page updates, the new text starts in the new font.
  const CJK = /[\p{Script=Han}\p{Script=Hangul}\p{Script=Hiragana}\p{Script=Katakana}]/u;
  const linked = new Set<string>();
  let shown = "";
  $effect.pre(() => {
    const view = `${app.pane}\n${app.study?.card?.id}\n${app.browse?.scope}`;
    const moved = view !== shown;
    shown = view;
    const waiting = app.fonts.filter((href) => !linked.has(href));
    if (!waiting.length) return;
    if (!moved && app.pane && CJK.test(document.getElementById("app")?.textContent ?? "")) return;
    for (const href of waiting) {
      const link = document.createElement("link");
      link.rel = "stylesheet";
      link.href = href;
      document.head.append(link);
      linked.add(href);
    }
  });
</script>

{#if app.pane === "study"}
  <Study />
{:else if app.pane === "dashboard"}
  <Dashboard />
{:else if app.pane === "browse"}
  <Browse />
{/if}
