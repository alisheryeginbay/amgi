<script lang="ts">
  import { DAY_SECONDS, date, number } from "./format";
  import { app } from "../lib/app.svelte";

  /** Cards due per day from today on. */
  let { forecast }: { forecast: number[] } = $props();

  const HEIGHT = 180;
  const AXIS_WIDTH = 32;
  const LABEL_HEIGHT = 20;
  const TOP = 8;

  let width = $state(0);
  let selected = $state<number | null>(null);

  /** Round tick values, about four of them, compact from 1,000 up. */
  const ticks = $derived.by(() => {
    const max = Math.max(...forecast, 1);
    const rough = max / 4;
    const magnitude = 10 ** Math.floor(Math.log10(rough));
    // Counts are whole cards, so ticks never step by less than one.
    const step = Math.max(1, [1, 2, 5, 10].map((m) => m * magnitude).find((s) => s >= rough) ?? magnitude * 10);
    const top = Math.max(Math.ceil(max / step) * step, 1);
    const values: number[] = [];
    for (let v = 0; v <= top + step / 2; v += step) values.push(Math.round(v));
    return { values, top };
  });
  const compact = $derived(new Intl.NumberFormat(app.environment.locale, { notation: "compact" }));

  const plotWidth = $derived(Math.max(width - AXIS_WIDTH, 0));
  const plotHeight = HEIGHT - LABEL_HEIGHT - TOP;
  const band = $derived(forecast.length ? plotWidth / forecast.length : 0);
  const barWidth = $derived(Math.min(band * 0.7, 24));
  const y = (value: number) => TOP + plotHeight - (value / ticks.top) * plotHeight;

  /** A column with a 4px rounded top and a square base. */
  function bar(index: number, value: number): string {
    const x = AXIS_WIDTH + band * index + (band - barWidth) / 2;
    const bottom = TOP + plotHeight;
    const height = bottom - y(value);
    if (height <= 0) return "";
    const r = Math.min(4, barWidth / 2, height);
    return `M${x},${bottom}V${bottom - height + r}Q${x},${bottom - height} ${x + r},${bottom - height}`
      + `H${x + barWidth - r}Q${x + barWidth},${bottom - height} ${x + barWidth},${bottom - height + r}V${bottom}Z`;
  }

  function dayDate(offset: number): Date {
    return new Date(Date.now() + offset * DAY_SECONDS * 1000);
  }

  function dayName(offset: number, short = false): string {
    if (offset === 0) return "Today";
    return short
      ? date(dayDate(offset), { weekday: "narrow" })
      : date(dayDate(offset), { weekday: "long", month: "short", day: "numeric" });
  }

  const tooltipX = $derived.by(() => {
    if (selected === null) return 0;
    const center = AXIS_WIDTH + band * (selected + 0.5);
    // Keep it inside the chart.
    return Math.min(Math.max(center, 60), Math.max(width - 60, 60));
  });
</script>

<div class="forecast" bind:clientWidth={width}>
  <svg {width} height={HEIGHT} role="img" aria-label="Cards due per day">
    {#each ticks.values as tick (tick)}
      <line class="grid" x1={AXIS_WIDTH} x2={width} y1={y(tick)} y2={y(tick)} />
      <text class="tick" x={AXIS_WIDTH - 6} y={y(tick)} dy="0.32em" text-anchor="end">{compact.format(tick)}</text>
    {/each}
    {#each forecast as count, day (day)}
      <path class="bar" class:dimmed={selected !== null && selected !== day} d={bar(day, count)} />
      {#if day % 2 === 0}
        <text class="tick" x={AXIS_WIDTH + band * (day + 0.5)} y={HEIGHT - 4} text-anchor="middle">{dayName(day, true)}</text>
      {/if}
      <!-- The whole band is the hit target, not only the painted bar. -->
      <rect
        class="hit"
        x={AXIS_WIDTH + band * day}
        y={TOP}
        width={band}
        height={plotHeight}
        role="presentation"
        onpointerenter={() => (selected = day)}
        onpointerleave={() => selected === day && (selected = null)}
      />
    {/each}
  </svg>

  {#if selected !== null}
    <div class="tooltip" style:left="{tooltipX}px">
      <strong class="tabular">{forecast[selected] === 1 ? "1 card" : `${number(forecast[selected])} cards`}</strong>
      <span>{dayName(selected)}</span>
    </div>
  {/if}
</div>

<style>
  .forecast {
    --bar: #2a78d6;
    position: relative;
    width: 100%;
  }

  @media (prefers-color-scheme: dark) {
    .forecast {
      --bar: #3987e5;
    }
  }

  svg {
    display: block;
    overflow: visible;
  }

  .grid {
    stroke: var(--separator);
    stroke-width: 1;
    shape-rendering: crispEdges;
  }

  .tick {
    font-size: var(--font-caption);
    fill: var(--secondary-label);
    font-variant-numeric: tabular-nums;
  }

  .bar {
    fill: var(--bar);
    transition: opacity 0.15s;
  }

  .dimmed {
    opacity: 0.4;
  }

  .hit {
    fill: transparent;
  }

  .tooltip {
    position: absolute;
    top: -6px;
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 4px 8px;
    border-radius: 6px;
    transform: translate(-50%, -100%);
    background: color-mix(in srgb, var(--text-background) 88%, transparent);
    -webkit-backdrop-filter: blur(20px);
    backdrop-filter: blur(20px);
    box-shadow: 0 0 0 0.5px var(--separator), 0 2px 8px rgb(0 0 0 / 0.12);
    font-size: var(--font-caption);
    white-space: nowrap;
    pointer-events: none;
  }

  .tooltip span {
    color: var(--secondary-label);
  }
</style>
