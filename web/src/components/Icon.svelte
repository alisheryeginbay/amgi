<script lang="ts">
  // A few SF Symbols–like glyphs, drawn on a 20-unit grid in the current text color.
  type Name = "speaker" | "speaker-playing" | "speaker-slash" | "photo" | "sparkle" | "retry" | "clock" | "warning";
  let { name, size = 16 }: { name: Name; size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 20 20" aria-hidden="true">
  {#if name === "sparkle"}
    <path d="M10 2.5c.6 4.2 2.3 5.9 6.5 6.5-4.2.6-5.9 2.3-6.5 6.5-.6-4.2-2.3-5.9-6.5-6.5 4.2-.6 5.9-2.3 6.5-6.5z" fill="currentColor" />
    <path d="M15.5 13c.2 1.4.8 2 2.2 2.2-1.4.2-2 .8-2.2 2.2-.2-1.4-.8-2-2.2-2.2 1.4-.2 2-.8 2.2-2.2z" fill="currentColor" />
  {:else if name === "retry"}
    <path d="M4.5 10a5.5 5.5 0 1 0 1.8-4.1" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
    <path d="M5.6 2.8v3.6h3.6" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" />
  {:else if name === "clock"}
    <circle cx="10" cy="10" r="6.75" fill="none" stroke="currentColor" stroke-width="1.6" />
    <path d="M10 6.2V10l2.6 1.7" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
  {:else if name === "warning"}
    <path d="M8.7 3.4a1.5 1.5 0 0 1 2.6 0l6.3 11.3a1.5 1.5 0 0 1-1.3 2.2H3.7a1.5 1.5 0 0 1-1.3-2.2z" fill="currentColor" />
    <!-- Dark on the yellow it's drawn in, like the system's caution sign. -->
    <path d="M10 7.5v4" stroke="#1d1d1f" stroke-width="1.7" stroke-linecap="round" />
    <circle cx="10" cy="14.2" r="1" fill="#1d1d1f" />
  {:else if name === "photo"}
    <rect x="2.5" y="4" width="15" height="12" rx="2.5" fill="none" stroke="currentColor" stroke-width="1.5" />
    <circle cx="7" cy="8.5" r="1.5" fill="currentColor" />
    <path d="M3.5 14.5l4.2-4 3 2.8 2.3-2 3.5 3.2" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" />
  {:else}
    <path d="M3 7.5h2.8L10 4v12l-4.2-3.5H3z" fill="currentColor" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" />
    {#if name === "speaker-slash"}
      <path d="M13 7.5l4 5M17 7.5l-4 5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
    {:else}
      <path d="M12.6 7.4a3.6 3.6 0 0 1 0 5.2" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      <path
        d="M14.8 5.2a6.7 6.7 0 0 1 0 9.6"
        fill="none"
        stroke="currentColor"
        stroke-width="1.5"
        stroke-linecap="round"
        opacity={name === "speaker-playing" ? 1 : 0.45}
      />
    {/if}
  {/if}
</svg>
