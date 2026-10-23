<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import { cubicOut } from "svelte/easing";
  import Flashcard from "./Flashcard.svelte";
  import { app, commands, post } from "../lib/app.svelte";
  import { player } from "../lib/audio.svelte";
  import { imageSources, soundSources } from "../lib/markdown";
  import type { StudyCard, StudyState } from "../lib/types";

  // A new card waits for its images, so it shows whole instead of growing as they
  // arrive. The last card stays up meanwhile, but no longer than this.
  const IMAGE_WAIT = 100;

  let shown = $state.raw<StudyState | null>(untrack(() => app.study));
  /** Whether a new card is waiting for its images; keys wait with it. */
  let pending = false;
  let ticket = 0;
  /** The images being loaded, held so they stay cached until the card shows. */
  let preloaded: HTMLImageElement[] = [];

  function preload(src: string) {
    const image = new Image();
    image.src = src;
    preloaded.push(image);
    return image.decode().catch(() => {});
  }

  $effect(() => {
    const next = app.study;
    const mine = ++ticket;
    untrack(() => {
      const card = next?.card;
      // The card on screen changing, such as revealed, has its images already.
      const sources =
        card && card.id !== shown?.card?.id
          ? [...new Set([...imageSources(card.front, card.path), ...imageSources(card.back, card.path)])]
          : [];
      if (sources.length === 0) {
        pending = false;
        shown = next;
        return;
      }
      pending = true;
      preloaded = [];
      const wait = new Promise((resolve) => setTimeout(resolve, IMAGE_WAIT));
      Promise.race([Promise.all(sources.map(preload)), wait]).then(() => {
        if (mine !== ticket) return;
        pending = false;
        shown = next;
      });
    });
  });

  const card = $derived(shown?.card ?? null);
  const revealed = $derived(shown?.revealed ?? false);
  const instant = $derived(shown?.instant ?? false);

  /** Today's share studied: answers given, out of those and the cards still to study. */
  const reviewed = $derived(shown?.reviewed ?? 0);
  const total = $derived(reviewed + (shown?.remaining ?? 0));
  const progress = $derived(total > 0 ? reviewed / total : 0);
  const showProgress = $derived(shown?.progress ?? true);

  // Each card is dealt anew, even the same card again, rather than fading back to its
  // question in place. New intervals for the card on screen keep it where it is.
  let dealt = 0;
  let lastId = "";
  let lastRevealed = false;
  const deal = $derived.by(() => {
    const id = card?.id ?? "";
    if (id !== lastId || (lastRevealed && !revealed)) dealt++;
    lastId = id;
    lastRevealed = revealed;
    return dealt;
  });

  // The next card replaces the last at once, so their text never shows through each
  // other, and fades in quickly: even the same card again visibly changes.
  function dealIn(_node: HTMLElement) {
    return { duration: 120, easing: cubicOut, css: (t: number) => `opacity: ${t}` };
  }

  // The next card starts from its top, even after a long card was scrolled.
  let scroller: HTMLElement;
  $effect(() => {
    deal;
    untrack(() => scroller && (scroller.scrollTop = 0));
  });

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
    untrack(() => (shown?.autoplay && key ? playSounds() : player.stop()));
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
    if (!card || pending) return false;
    key = key.toLowerCase();
    if (key === " " && !revealed) {
      post({ type: "reveal", via: "key" });
    } else if (["1", "2", "3", "4"].includes(key) && revealed) {
      post({ type: "answer", rating: Number(key) as 1 | 2 | 3 | 4, via: "key" });
    } else if (key === "r") {
      replay();
    } else {
      return false;
    }
    return true;
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.metaKey || event.ctrlKey || event.altKey || event.repeat) return;
    // Keys typed into a typed answer are its text.
    if (event.target instanceof HTMLInputElement) return;
    if (handleKey(event.key)) event.preventDefault();
  }
</script>

<svelte:window {onkeydown} />

<main bind:this={scroller}>
  {#if card}
    <div class="pile">
      {#if showProgress}
        <div
          class="progress"
          role="progressbar"
          aria-label="Studied today"
          aria-valuemin={0}
          aria-valuemax={total}
          aria-valuenow={reviewed}
          title="{reviewed} of {total} studied today"
        >
          <div class="bar" style:transform="scaleX({progress})"></div>
        </div>
      {/if}
      <!-- The sheet stays put while what's on it is dealt anew, so it never blinks. -->
      <div class="sheet">
        {#key deal}
          <div class="stage" in:dealIn>
            <Flashcard {card} {revealed} {instant} onreveal={() => post({ type: "reveal", via: "pointer" })} />
          </div>
        {/key}
      </div>
    </div>
  {/if}
</main>

<style>
  /* Room above for the toolbar and below for the answer buttons floating there. */
  main {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow-y: auto;
    /* A scrollbar showing for a long answer doesn't nudge the card sideways. */
    scrollbar-gutter: stable both-edges;
    padding: 64px 40px 104px;
  }

  /* The card sits at the top, centered across, and grows downward. */
  .pile {
    display: grid;
    width: min(560px, 100%);
    margin: 0 auto;
  }

  /* The card's sheet, lifted off the window; rounded to sit with the glass answer
     buttons below it. */
  .sheet {
    min-width: 0;
    border-radius: 14px;
    background: var(--card-background);
    box-shadow: var(--card-shadow);
  }

  /* Today's progress, a thin track above the card. */
  .progress {
    height: 4px;
    margin-bottom: 16px;
    border-radius: 2px;
    background: var(--fill-strong);
    overflow: hidden;
  }

  .bar {
    height: 100%;
    background: var(--accent);
    transform-origin: left;
    transition: transform 0.3s var(--ease-out);
  }

  @media (prefers-reduced-motion: reduce) {
    .bar {
      transition: none;
    }
  }
</style>
