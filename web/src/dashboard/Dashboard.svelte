<script lang="ts">
  import Forecast from "./Forecast.svelte";
  import Heatmap from "./Heatmap.svelte";
  import { app } from "../lib/app.svelte";
  import { number } from "./format";

  const d = $derived(app.dashboard);
  const percent = $derived(new Intl.NumberFormat(app.environment.locale, { style: "percent", maximumFractionDigits: 0 }));
</script>

{#if d}
  <main>
    <div class="content">
      <div class="tiles">
        {@render tile(number(d.due), "Due")}
        {@render tile(number(d.newToday), "New today")}
        {@render tile(number(d.reviewedToday), "Reviewed today")}
        {@render tile(d.streak === 1 ? "1 day" : `${number(d.streak)} days`, "Streak")}
      </div>

      <section class="panel">
        <h2>Activity</h2>
        <Heatmap activity={d.activity} start={d.activityStart} />
      </section>

      <div class="row">
        <section class="panel">
          <h2>Due in the next two weeks</h2>
          <Forecast forecast={d.forecast} />
        </section>
        <section class="panel cards">
          <h2>Cards</h2>
          <dl>
            {@render count("New", d.newCards, "Never studied")}
            {@render count("Young", d.youngCards, "Next review less than 21 days after the last")}
            {@render count("Mature", d.matureCards, "Next review 21 days or more after the last")}
          </dl>
          <hr />
          <dl>
            <div title="Reviews in the last 30 days not rated Again, counting cards last seen a day or more before">
              <dt>Retention</dt>
              <dd class="tabular">{d.retention == null ? "–" : percent.format(d.retention)}</dd>
            </div>
          </dl>
        </section>
      </div>
    </div>
  </main>
{/if}

{#snippet tile(value: string, label: string)}
  <div class="tile">
    <div class="value">{value}</div>
    <div class="label">{label}</div>
  </div>
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

  .value {
    font-size: var(--font-title);
    font-weight: 600;
  }

  .label {
    margin-top: 2px;
    font-size: var(--font-callout);
    color: var(--secondary-label);
  }

  h2 {
    margin: 0 0 14px;
    font-size: var(--font-body);
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

  hr {
    margin: 10px 0;
    border: none;
    border-top: 1px solid var(--separator);
  }
</style>
