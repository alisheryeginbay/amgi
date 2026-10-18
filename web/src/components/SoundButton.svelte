<script lang="ts">
  import Icon from "./Icon.svelte";
  import { player } from "../lib/audio.svelte";
  import { fileName } from "../lib/markdown";

  /** `compact` is a bare speaker glyph that sits in a line of text. */
  let { src, path, compact = false }: { src: string | null; path: string; compact?: boolean } = $props();

  let missing = $state(false);
  // A missing file would only show up once played; ask the app up front.
  $effect(() => {
    missing = !src;
    if (!src?.startsWith("amgi:")) return;
    let current = true;
    fetch(src, { method: "HEAD" })
      .then((response) => current && (missing = !response.ok))
      .catch(() => current && (missing = true));
    return () => (current = false);
  });

  const unplayable = $derived(missing || (src !== null && (player.unplayable.has(src) || !player.supports(src))));
  const playing = $derived(src !== null && player.current === src);
  const name = $derived(fileName(path));
  const help = $derived(missing ? `Sound not found: ${path}` : unplayable ? `Can't play ${name}` : `Play ${name}`);
</script>

<button
  class:compact
  class:playing
  class:unplayable
  disabled={missing}
  title={help}
  aria-label={help}
  onclick={() => src && player.play([src])}
>
  <Icon name={unplayable ? "speaker-slash" : playing ? "speaker-playing" : "speaker"} size={compact ? 15 : 18} />
</button>

<style>
  button {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    padding: 0;
    border: 0.5px solid var(--separator);
    border-radius: 50%;
    background: var(--fill);
    color: var(--label);
    transition: background 0.15s;
  }

  button:hover:not(:disabled) {
    background: var(--fill-strong);
  }

  button:active:not(:disabled) {
    transform: scale(0.96);
  }

  .unplayable {
    color: var(--secondary-label);
  }

  .compact {
    display: inline-grid;
    width: 1em;
    height: 1em;
    /* Hangs past the end of the line, so centred text stays centred. */
    margin-left: 0.3em;
    margin-right: -1.3em;
    vertical-align: -0.1em;
    border: none;
    background: none;
    color: var(--tertiary-label);
    transition: color 0.15s;
  }

  .compact:hover:not(:disabled),
  .compact.playing {
    background: none;
    color: var(--accent);
  }
</style>
