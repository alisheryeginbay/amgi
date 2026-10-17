<script lang="ts">
  import { onMount } from "svelte";
  import { DAY_SECONDS, date, number, weekdayIndex } from "./format";
  import { app } from "../lib/app.svelte";

  /** A year of reviews as a grid of days: one column per week, like GitHub's
   *  contribution graph. Shade is the day's quartile among days with reviews. */
  let { activity, start }: { activity: number[]; start: number } = $props();

  interface Day {
    index: number;
    date: Date;
    count: number;
  }

  const days = $derived(
    activity.map((count, index): Day => ({ index, date: new Date((start + index * DAY_SECONDS) * 1000), count })),
  );

  /** Days grouped into week columns, padded with null before the first day and after today. */
  const weeks = $derived.by(() => {
    if (days.length === 0) return [];
    const padded: (Day | null)[] = [...Array(weekdayIndex(days[0].date)).fill(null), ...days];
    const columns: (Day | null)[][] = [];
    for (let i = 0; i < padded.length; i += 7) {
      const week = padded.slice(i, i + 7);
      columns.push([...week, ...Array(7 - week.length).fill(null)]);
    }
    return columns;
  });

  /** Quartile boundaries of the days with reviews, so one huge day does not wash out the rest. */
  const bounds = $derived.by(() => {
    const active = activity.filter((n) => n > 0).sort((a, b) => a - b);
    return active.length ? [0.25, 0.5, 0.75].map((q) => active[Math.floor((active.length - 1) * q)]) : [];
  });
  const level = (count: number) => (count === 0 ? 0 : 1 + bounds.filter((b) => count > b).length);

  /** Short month names above the first column of each month, skipping any that would crowd the last. */
  const months = $derived.by(() => {
    let lastMonth = -1;
    let lastLabel = -3;
    return weeks.map((week, column) => {
      const day = week.find((d) => d !== null);
      if (!day) return "";
      const month = day.date.getMonth();
      const show = month !== lastMonth && column - lastLabel >= 3;
      lastMonth = month;
      if (!show) return "";
      lastLabel = column;
      return date(day.date, { month: "short" });
    });
  });

  /** Every other weekday name, like GitHub's Mon / Wed / Fri. */
  const weekdays = $derived(
    Array.from({ length: 7 }, (_, row) => {
      if (row % 2 === 0) return "";
      const weekday = (app.environment.firstWeekday + row) % 7;
      // 4 January 1970 was a Sunday.
      return date(new Date(1970, 0, 4 + weekday), { weekday: "short" });
    }),
  );

  const summary = $derived.by(() => {
    const total = activity.reduce((a, b) => a + b, 0);
    const studied = activity.filter((n) => n > 0).length;
    return `${number(total)} ${total === 1 ? "review" : "reviews"} on ${number(studied)} ${studied === 1 ? "day" : "days"} in the last year`;
  });

  let scroller: HTMLDivElement;
  let container: HTMLDivElement;
  // Start at today, the right end.
  onMount(() => {
    scroller.scrollLeft = scroller.scrollWidth;
  });

  let hovered = $state<{ day: Day; x: number; y: number } | null>(null);
  const TOOLTIP_WIDTH = 220;

  function hover(event: PointerEvent, day: Day) {
    const cell = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const box = container.getBoundingClientRect();
    hovered = { day, x: cell.left + cell.width / 2 - box.left, y: cell.top - box.top };
  }

  const tooltip = $derived.by(() => {
    if (!hovered) return null;
    const half = TOOLTIP_WIDTH / 2;
    // Keep the bubble inside the heatmap; the arrow still points at the cell.
    const x = Math.min(Math.max(hovered.x, half), Math.max(container.clientWidth - half, half));
    const { count } = hovered.day;
    return {
      x,
      y: hovered.y,
      arrow: hovered.x - x,
      amount: count === 0 ? "No" : number(count),
      text: ` ${count === 1 ? "card" : "cards"} reviewed on ${date(hovered.day.date, {
        weekday: "long",
        month: "long",
        day: "numeric",
        year: "numeric",
      })}`,
    };
  });
</script>

<div class="heatmap" bind:this={container} role="img" aria-label={`Activity: ${summary}`}>
  <div class="grid-row">
    <div class="weekdays">
      {#each weekdays as label, row (row)}
        <span>{label}</span>
      {/each}
    </div>
    <div class="scroller" bind:this={scroller} onscroll={() => (hovered = null)}>
      <div class="months">
        {#each months as label, column (column)}
          <span>{label}</span>
        {/each}
      </div>
      <div class="weeks">
        {#each weeks as week, column (column)}
          <div class="week">
            {#each week as day, row (row)}
              {#if day}
                <div
                  class="cell level-{level(day.count)}"
                  role="presentation"
                  class:hovered={hovered?.day.index === day.index}
                  onpointerenter={(event) => hover(event, day)}
                  onpointerleave={() => hovered?.day.index === day.index && (hovered = null)}
                ></div>
              {:else}
                <div class="cell empty"></div>
              {/if}
            {/each}
          </div>
        {/each}
      </div>
    </div>
  </div>

  <div class="footer">
    <span>{summary}</span>
    <span class="legend">
      Less
      {#each [0, 1, 2, 3, 4] as l (l)}
        <span class="cell level-{l}"></span>
      {/each}
      More
    </span>
  </div>

  {#if tooltip}
    <div class="tooltip" style:left="{tooltip.x}px" style:top="{tooltip.y - 3}px" style:width="{TOOLTIP_WIDTH}px">
      <div class="bubble"><strong>{tooltip.amount}</strong>{tooltip.text}</div>
      <div class="arrow" style:transform="translateX({tooltip.arrow}px)"></div>
    </div>
  {/if}
</div>

<style>
  .heatmap {
    --cell: 11px;
    --gap: 3px;
    /* One-hue ordinal ramp, validated separately for each appearance: every step
       clears 2:1 against the window background and adjacent steps differ in lightness. */
    --level-1: #86b6ef;
    --level-2: #3987e5;
    --level-3: #1c5cab;
    --level-4: #0d366b;
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  @media (prefers-color-scheme: dark) {
    .heatmap {
      --level-1: #184f95;
      --level-2: #256abf;
      --level-3: #5598e7;
      --level-4: #9ec5f4;
    }
  }

  .grid-row {
    display: flex;
    align-items: flex-end;
    gap: var(--gap);
  }

  .weekdays {
    display: flex;
    flex-direction: column;
    gap: var(--gap);
    flex: none;
    width: 26px;
  }

  .weekdays span,
  .months span {
    font-size: var(--font-caption);
    color: var(--secondary-label);
    line-height: var(--cell);
    white-space: nowrap;
  }

  .weekdays span {
    height: var(--cell);
  }

  .scroller {
    overflow-x: auto;
    scrollbar-width: none;
  }

  .scroller::-webkit-scrollbar {
    display: none;
  }

  .months {
    display: flex;
    gap: var(--gap);
    margin-bottom: 4px;
  }

  .months span {
    flex: none;
    width: var(--cell);
    overflow: visible;
  }

  .weeks {
    display: flex;
    gap: var(--gap);
  }

  .week {
    display: flex;
    flex-direction: column;
    gap: var(--gap);
  }

  .cell {
    flex: none;
    display: inline-block;
    width: var(--cell);
    height: var(--cell);
    border-radius: 2.5px;
  }

  .cell.empty {
    visibility: hidden;
  }

  .level-0 {
    background: var(--quaternary-label);
  }
  .level-1 {
    background: var(--level-1);
  }
  .level-2 {
    background: var(--level-2);
  }
  .level-3 {
    background: var(--level-3);
  }
  .level-4 {
    background: var(--level-4);
  }

  .hovered {
    box-shadow: inset 0 0 0 1.5px var(--label);
  }

  .footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: var(--font-caption);
    color: var(--secondary-label);
  }

  .legend {
    display: flex;
    align-items: center;
    gap: var(--gap);
  }

  .tooltip {
    --tooltip: #2b2b2b;
    position: absolute;
    display: flex;
    flex-direction: column;
    align-items: center;
    transform: translate(-50%, -100%);
    pointer-events: none;
    filter: drop-shadow(0 2px 4px rgb(0 0 0 / 0.2));
    z-index: 1;
  }

  @media (prefers-color-scheme: dark) {
    .tooltip {
      --tooltip: #4a4a4a;
    }
  }

  .bubble {
    padding: 6px 10px;
    border-radius: 6px;
    background: var(--tooltip);
    color: white;
    font-size: var(--font-callout);
    text-align: center;
  }

  .arrow {
    width: 12px;
    height: 6px;
    background: var(--tooltip);
    clip-path: polygon(0 0, 100% 0, 50% 100%);
  }
</style>
