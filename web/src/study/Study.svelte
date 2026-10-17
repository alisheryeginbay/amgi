<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import { cubicOut } from "svelte/easing";
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

  const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;

  /** The card on screen, as a list keyed by deal: a card leaving keeps what it showed. */
  const dealtCards = $derived(card ? [{ deal, card, revealed }] : []);

  /** The next card settles into place, under the one leaving. */
  function dealIn(_node: Element) {
    return {
      delay: reduceMotion ? 0 : 60,
      duration: reduceMotion ? 150 : 460,
      easing: cubicOut,
      css: (t: number, u: number) =>
        reduceMotion ? `opacity: ${t}` : `opacity: ${t}; transform: translateY(${u * 14}px) scale(${0.97 + 0.03 * t})`,
    };
  }

  /** How far an answered card drifts aside as it leaves: left for Again, right for Easy. */
  const DRIFT = [-24, -10, 10, 24];

  /** The card answered lifts off the pile and fades. */
  function dealOut(node: HTMLElement, { answered }: { answered: boolean }) {
    node.style.zIndex = "1";
    node.style.pointerEvents = "none";
    const rating = app.study?.answered;
    const drift = answered && rating ? DRIFT[rating - 1] : 0;
    return {
      duration: reduceMotion ? 150 : 320,
      easing: cubicOut,
      css: (t: number, u: number) =>
        reduceMotion
          ? `opacity: ${t}`
          : `opacity: ${t}; transform: translate(${drift * u}px, ${-12 * u}px) scale(${1 + 0.02 * u})`,
    };
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
      <div class="stage" in:dealIn out:dealOut={{ answered: dealt.revealed }}>
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
    display: grid;
    width: min(560px, 100%);
    margin: auto;
  }

  .stage {
    grid-area: 1 / 1;
    min-width: 0;
  }
</style>
