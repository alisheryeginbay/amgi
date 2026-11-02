<script lang="ts">
  import Forecast from "./Forecast.svelte";
  import Heatmap from "./Heatmap.svelte";
  import { app } from "../lib/app.svelte";
  import { number } from "./format";
  import { dailyAverage, reviewTime, studyTime } from "./time";

  const d = $derived(app.dashboard);
  /** Whether any review was ever timed, for showing time at all. */
  const timed = $derived(d ? d.secsPerReview !== null || d.activitySecs.some((s) => s > 0) : false);
  const average = $derived(d ? dailyAverage(d.activitySecs.slice(-30)) : null);

  const HEATMAP_KEY = "dashboard.heatmapByTime";
  let byTime = $state(read());
  function read(): boolean {
    try {
      return localStorage.getItem(HEATMAP_KEY) === "1";
    } catch {
      return false;
    }
  }
  function shadeByTime(on: boolean) {
    byTime = on;
    try {
      localStorage.setItem(HEATMAP_KEY, on ? "1" : "0");
    } catch {
      // Only a remembered preference.
    }
  }
  const PERIODS = ["Today", "Yesterday", "Last week", "Last month", "Last year"];
  const percent = $derived(
    new Intl.NumberFormat(app.environment.locale, { style: "percent", minimumFractionDigits: 1, maximumFractionDigits: 1 }),
  );
</script>

{#if d}
  <main>
    <div class="content">
      <div class="tiles">
        {@render tile(number(d.due), "Due")}
        {@render tile(number(d.newToday), "New today")}
        {@render tile(number(d.reviewedToday), "Reviewed today")}
        {@render tile(d.streak === 1 ? "1 day" : `${number(d.streak)} days`, "Streak")}
        {#if timed}
          {@render tile(studyTime(d.studySecsToday), "Studied today")}
          {@render tile(
            d.secsLeft === null || d.due + d.newToday === 0 ? "–" : `≈ ${studyTime(Math.max(d.secsLeft, 60))}`,
            "Left today",
            "About how long the cards still due and new today will take, at your pace of the last 30 days",
          )}
          {@render tile(average === null ? "–" : studyTime(average), "Daily average", "Time per day studied, over the last 30 days")}
          {@render tile(
            d.secsPerReview === null ? "–" : reviewTime(d.secsPerReview),
            "Per review",
            "Time spent on a card before rating it, over the last 30 days",
          )}
        {/if}
      </div>

      <section class="panel">
        <div class="heading">
          <h2>Activity</h2>
          {#if timed}
            <div class="segmented" role="group" aria-label="Shade days by">
              <button class:on={!byTime} aria-pressed={!byTime} onclick={() => shadeByTime(false)}>Reviews</button>
              <button class:on={byTime} aria-pressed={byTime} onclick={() => shadeByTime(true)}>Time</button>
            </div>
          {/if}
        </div>
        <Heatmap activity={d.activity} secs={d.activitySecs} start={d.activityStart} byTime={timed && byTime} />
      </section>

      <div class="row">
        <section class="panel">
          <h2>Due in the next two weeks</h2>
          <Forecast forecast={d.forecast} secsPerCard={d.secsPerDueCard} />
        </section>
        <section class="panel cards">
          <h2>Cards</h2>
          <dl>
            {@render count("New", d.newCards, "Never studied")}
            {@render count("Young", d.youngCards, "Next review less than 21 days after the last")}
            {@render count("Mature", d.matureCards, "Next review 21 days or more after the last")}
            {#if d.suspendedCards > 0}
              {@render count("Suspended", d.suspendedCards, "Tagged #suspended, so they don't come up")}
            {/if}
          </dl>
        </section>
      </div>

      <section class="panel">
        <h2 title="Reviews not rated Again, of cards scheduled a day or more ahead">Retention</h2>
        <table>
          <thead>
            <tr>
              <th></th>
              <th title="Next review was less than 21 days after the one before">Young</th>
              <th title="Next review was 21 days or more after the one before">Mature</th>
              <th>Total</th>
              <th>Reviews</th>
            </tr>
          </thead>
          <tbody>
            {#each d.retention as r, i (i)}
              <tr>
                <th scope="row">{PERIODS[i]}</th>
                {@render recall(r.youngPassed, r.youngFailed)}
                {@render recall(r.maturePassed, r.matureFailed)}
                {@render recall(r.youngPassed + r.maturePassed, r.youngFailed + r.matureFailed)}
                <td class="muted">{number(r.youngPassed + r.youngFailed + r.maturePassed + r.matureFailed)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </section>
    </div>
  </main>
{/if}

{#snippet tile(value: string, label: string, help?: string)}
  <div class="tile" title={help}>
    <div class="value">{value}</div>
    <div class="label">{label}</div>
  </div>
{/snippet}

{#snippet recall(passed: number, failed: number)}
  {@const reviews = passed + failed}
  <td title={reviews ? `${number(passed)} of ${number(reviews)} remembered` : undefined} class:muted={!reviews}>
    {reviews ? percent.format(passed / reviews) : "–"}
  </td>
{/snippet}

{#snippet count(label: string, value: number, help: string)}
  <div title={help}>
    <dt>{label}</dt>
    <dd class="tabular muted">{number(value)}</dd>
  </div>
{/snippet}

<style>
  main {
    height: 100%;
    overflow-y: auto;
    padding: 32px;
  }

  .content {
    display: flex;
    flex-direction: column;
    gap: 20px;
    max-width: 860px;
    margin: 0 auto;
  }

  .tiles {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 12px;
  }

  .tile,
  .panel {
    padding: 16px;
    border-radius: 12px;
    background: var(--tile);
  }

  /* Figures and headings in the headline serif; labels stay in the system font. */
  .value {
    font-family: var(--font-headline);
    font-size: 28px;
    font-weight: 500;
    line-height: 1.15;
  }

  .label {
    margin-top: 2px;
    font-size: var(--font-callout);
    color: var(--secondary-label);
  }

  .heading {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }

  .segmented {
    display: flex;
    padding: 2px;
    border-radius: 7px;
    background: var(--quaternary-label);
  }

  .segmented button {
    padding: 2px 10px;
    border: none;
    border-radius: 5px;
    background: none;
    color: var(--secondary-label);
    font: inherit;
    font-size: var(--font-caption);
  }

  .segmented button.on {
    background: var(--text-background);
    color: var(--label);
    box-shadow: 0 0.5px 1.5px rgb(0 0 0 / 0.2);
  }

  h2 {
    margin: 0 0 14px;
    font-family: var(--font-headline);
    font-size: var(--font-title3);
    font-weight: 600;
  }

  .row {
    display: flex;
    align-items: flex-start;
    gap: 20px;
  }

  .row > .panel:first-child {
    flex: 1;
    min-width: 0;
  }

  .cards {
    flex: none;
    width: 220px;
  }

  dl {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin: 0;
  }

  dl > div {
    display: flex;
    justify-content: space-between;
  }

  dd {
    margin: 0;
  }

  .muted {
    color: var(--secondary-label);
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-variant-numeric: tabular-nums;
  }

  th,
  td {
    padding: 5px 0 5px 12px;
    text-align: right;
    font-weight: normal;
  }

  thead th {
    font-size: var(--font-callout);
    color: var(--secondary-label);
  }

  tbody th {
    padding-left: 0;
    text-align: left;
  }

  tbody tr + tr > * {
    border-top: 1px solid var(--separator);
  }
</style>
