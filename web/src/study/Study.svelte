<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import { fade } from "svelte/transition";
  import Markdown from "../components/Markdown.svelte";
  import TagPills from "../components/TagPills.svelte";
  import { app, commands, post } from "../lib/app.svelte";
  import { player } from "../lib/audio.svelte";
  import { soundSources } from "../lib/markdown";
  import type { StudyCard } from "../lib/types";

  const card = $derived(app.study?.card ?? null);
  const revealed = $derived(app.study?.revealed ?? false);

  /** Sounds on the side showing, and those the answer adds to the question. */
  function sounds(card: StudyCard, revealed: boolean) {
    const front = soundSources(card.front, card.path);
    if (!revealed) return { shown: front, added: front };
    const back = soundSources(card.back, card.path);
    // A cloze's answer is the whole card again; a basic one shows both sides.
    const shown = card.kind === "cloze" ? back : [...front, ...back];
    return { shown, added: back.filter((src) => !front.includes(src)) };
  }

  /** What the side showing adds; a replay falls back to every sound shown. */
  function playSounds(replay = false) {
    if (!card) return player.stop();
    const { shown, added } = sounds(card, revealed);
    const list = replay && added.length === 0 ? shown : added;
    if (list.length) player.play(list);
    else player.stop();
  }

  // Play a card's sounds when it appears, then the answer's when it is revealed. New
  // intervals for the same card, or turning autoplay on, play nothing.
  let played = "";
  $effect(() => {
    const key = card ? `${card.id} ${revealed}` : "";
    if (key === played) return;
    played = key;
    untrack(() => (app.study?.autoplay && key ? playSounds() : player.stop()));
  });

  const replay = () => playSounds(true);
  const forwarded = (event: Event) => handleKey((event as CustomEvent<string>).detail);
  commands.addEventListener("replay", replay);
  commands.addEventListener("key", forwarded);
  onDestroy(() => {
    commands.removeEventListener("replay", replay);
    commands.removeEventListener("key", forwarded);
    player.stop();
  });

  /** Space shows the answer, 1–4 rate the card, R replays its sounds. The app forwards
   *  these keys while focus is elsewhere, so they work without the page taking focus. */
  function handleKey(key: string): boolean {
    if (!card) return false;
    key = key.toLowerCase();
    if (key === " " && !revealed) {
      post({ type: "reveal" });
    } else if (["1", "2", "3", "4"].includes(key) && revealed) {
      post({ type: "answer", rating: Number(key) as 1 | 2 | 3 | 4 });
    } else if (key === "r") {
      replay();
    } else {
      return false;
    }
    return true;
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.metaKey || event.ctrlKey || event.altKey || event.repeat) return;
    if (handleKey(event.key)) event.preventDefault();
  }
</script>

<svelte:window {onkeydown} />

{#if card}
  {#key card.id}
    <main in:fade={{ duration: 200 }}>
      <article>
        <header>
          <span class="deck">{card.deck}</span>
          <TagPills tags={card.tags} />
        </header>

        {#if card.kind === "basic"}
          <div class="front"><Markdown source={card.front} cardPath={card.path} /></div>
          {#if revealed}
            <div class="answer" in:fade={{ duration: 250 }}>
              <hr />
              <div class="back"><Markdown source={card.back} cardPath={card.path} /></div>
            </div>
          {/if}
        {:else}
          <div class="front">
            <Markdown source={revealed ? card.back : card.front} cardPath={card.path} emphasis />
          </div>
        {/if}
      </article>
    </main>
  {/key}
{/if}

<style>
  main {
    height: 100%;
    overflow-y: auto;
    padding: 72px 48px 32px;
  }

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
</style>
