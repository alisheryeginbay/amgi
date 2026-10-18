<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import Flashcard from "./Flashcard.svelte";
  import { app, commands, post } from "../lib/app.svelte";
  import { player } from "../lib/audio.svelte";
  import { soundSources } from "../lib/markdown";
  import type { StudyCard } from "../lib/types";

  const card = $derived(app.study?.card ?? null);
  const revealed = $derived(app.study?.revealed ?? false);

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

  /** The card on screen, as a list keyed by deal: a card leaving keeps what it showed. */
  const dealtCards = $derived(card ? [{ deal, card, revealed }] : []);

  // Cards cross-fade: the next one fades in once the last has gone, so their text never
  // shows through each other.
  const OUT = 100;

  function dealIn(_node: HTMLElement) {
    return { delay: OUT, duration: 140, css: (t: number) => `opacity: ${t}` };
  }

  /** Takes a leaving card out of the pile's layout, pinned where it stood, so the pile
   *  takes the next card's size and place at once instead of when this one is gone. */
  function pinInPlace(node: HTMLElement) {
    const pile = node.parentElement!;
    const scroller = pile.parentElement!;
    // Where it stood before the next card joined the pile.
    const next = [...pile.children].filter((c): c is HTMLElement => c !== node && !c.classList.contains("leaving"));
    for (const c of next) c.style.display = "none";
    const at = node.getBoundingClientRect();
    for (const c of next) c.style.display = "";
    node.style.width = `${node.offsetWidth}px`;
    node.classList.add("leaving");
    // The next card starts from its top, even after a long card was scrolled.
    scroller.scrollTop = 0;
    const origin = pile.getBoundingClientRect();
    node.style.top = `${at.top - origin.top}px`;
    node.style.left = `${at.left - origin.left}px`;
  }

  function dealOut(node: HTMLElement) {
    pinInPlace(node);
    return { duration: OUT, css: (t: number) => `opacity: ${t}` };
  }

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

<main>
  <div class="pile">
    {#each dealtCards as dealt (dealt.deal)}
      <div class="stage" in:dealIn out:dealOut>
        <Flashcard card={dealt.card} revealed={dealt.revealed} onreveal={() => post({ type: "reveal" })} />
      </div>
    {/each}
  </div>
</main>

<style>
  /* Room above for the toolbar and below for the answer buttons floating there. */
  main {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow-y: auto;
    padding: 64px 40px 104px;
  }

  /* Auto margins center the card, and keep a tall one's top in view. Cards coming
     and going share one cell. */
  .pile {
    position: relative;
    display: grid;
    width: min(560px, 100%);
    margin: auto;
  }

  .stage {
    grid-area: 1 / 1;
    min-width: 0;
  }

  /* An answered card leaving lies over the pile without sizing it. */
  .stage:global(.leaving) {
    position: absolute;
    z-index: 1;
    pointer-events: none;
  }
</style>
