<script lang="ts">
  import { onDestroy } from "svelte";
  import CardView from "../components/CardView.svelte";
  import TagPills from "../components/TagPills.svelte";
  import { app, commands, post } from "../lib/app.svelte";
  import { renderInline } from "../lib/markdown";
  import type { BrowseCard } from "../lib/types";
  import { COLUMNS, type ColumnId, type Sort, deckTitle, sorted } from "./columns";
  import { parseQuery } from "./query";

  // The table renders only the rows in view, so ten cards cost the same as ten thousand.
  const ROW_HEIGHT = 24;
  const HEADER_HEIGHT = 28;
  const INSET = 10;
  const OVERSCAN = 8;

  // Sort order and column widths are kept between launches, like the AppKit table's.
  function load<T>(key: string, fallback: T): T {
    try {
      const saved = localStorage.getItem(key);
      return saved ? { ...fallback, ...JSON.parse(saved) } : fallback;
    } catch {
      return fallback;
    }
  }
  function save(key: string, value: unknown) {
    try {
      localStorage.setItem(key, JSON.stringify(value));
    } catch {
      // Not kept; the table still works.
    }
  }

  let sort = $state<Sort | null>(load<{ value: Sort | null }>("browse.sort", { value: null }).value);
  let widths = $state<Record<ColumnId, number>>(
    load("browse.widths", Object.fromEntries(COLUMNS.map((c) => [c.id, c.width])) as Record<ColumnId, number>),
  );

  const collator = $derived(new Intl.Collator(app.environment.locale, { numeric: true, sensitivity: "base" }));
  const cards = $derived.by(() => {
    const browse = app.browse;
    if (!browse) return [];
    return browse.query ? browse.cards.filter(parseQuery(browse.query)) : browse.cards;
  });
  // The window subtitle counts them.
  $effect(() => post({ type: "count", count: cards.length }));
  const query = $derived(app.search.trim());
  const rows = $derived.by(() => {
    const matches = parseQuery(query);
    return sorted(query ? cards.filter(matches) : cards, sort, collator);
  });
  const rowIndex = $derived(new Map(rows.map((card, i) => [card.id, i])));

  // MARK: Layout

  let scroller = $state<HTMLDivElement>();
  let viewWidth = $state(0);
  let viewHeight = $state(0);
  let scrollTop = $state(0);

  /** Extra width is shared by every column in proportion, as with AppKit's uniform resizing. */
  const scale = $derived.by(() => {
    const total = COLUMNS.reduce((sum, c) => sum + widths[c.id], 0);
    return Math.max((viewWidth - 2 * INSET) / total, 0);
  });
  const shown = $derived(COLUMNS.map((c) => Math.max(c.minWidth, widths[c.id] * scale)));
  const template = $derived(shown.map((w) => `${w}px`).join(" "));
  const tableWidth = $derived(shown.reduce((a, b) => a + b, 0));

  const first = $derived(Math.max(0, Math.floor((scrollTop - HEADER_HEIGHT) / ROW_HEIGHT) - OVERSCAN));
  const last = $derived(Math.min(rows.length, Math.ceil((scrollTop + viewHeight) / ROW_HEIGHT) + OVERSCAN));
  const visible = $derived(rows.slice(first, last));

  // MARK: Selection

  let selection = $state.raw(new Set<string>());
  let anchor: string | null = null;
  let cursor: string | null = null;
  /** The row right-clicked outside the selection, outlined while its menu is open. */
  let contextRow = $state<string | null>(null);
  let focused = $state(false);

  // Another deck or tag starts with nothing selected.
  let scope = "";
  $effect(() => {
    const next = app.browse?.scope ?? "";
    if (next !== scope) {
      scope = next;
      select(new Set());
      scroller?.scrollTo({ top: 0 });
    }
  });

  // Rows hidden by a search are not selected.
  $effect(() => {
    const kept = [...selection].filter((id) => rowIndex.has(id));
    if (kept.length !== selection.size) select(new Set(kept));
  });

  function select(ids: Set<string>) {
    selection = ids;
    post({ type: "select", ids: [...ids] });
  }

  function range(from: string, to: string): Set<string> {
    const [a, b] = [rowIndex.get(from) ?? 0, rowIndex.get(to) ?? 0].sort((x, y) => x - y);
    return new Set(rows.slice(a, b + 1).map((c) => c.id));
  }

  function onRowDown(event: MouseEvent, card: BrowseCard) {
    if (event.button !== 0) return;
    // Focus stays on the table: a row scrolled out of view is removed, and focus would
    // go with it, so the arrow keys would stop working.
    event.preventDefault();
    scroller?.focus({ preventScroll: true });
    contextRow = null;
    if (event.shiftKey && anchor && rowIndex.has(anchor)) {
      select(range(anchor, card.id));
    } else if (event.metaKey) {
      const next = new Set(selection);
      if (next.has(card.id)) next.delete(card.id);
      else next.add(card.id);
      select(next);
      anchor = card.id;
    } else {
      select(new Set([card.id]));
      anchor = card.id;
    }
    cursor = card.id;
  }

  function onContextMenu(card: BrowseCard) {
    // Right-clicking a row outside the selection acts on that row alone.
    const targets = selection.has(card.id) ? selection : new Set([card.id]);
    contextRow = selection.has(card.id) ? null : card.id;
    post({ type: "contextCards", ids: [...targets] });
  }

  const closeMenu = () => (contextRow = null);
  commands.addEventListener("contextMenuClosed", closeMenu);
  onDestroy(() => commands.removeEventListener("contextMenuClosed", closeMenu));

  function open(card: BrowseCard) {
    post({ type: "open", path: card.path, line: card.line });
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "a" && event.metaKey) {
      select(new Set(rows.map((c) => c.id)));
    } else if ((event.key === "ArrowDown" || event.key === "ArrowUp") && rows.length) {
      const step = event.key === "ArrowDown" ? 1 : -1;
      const current = cursor !== null ? rowIndex.get(cursor) : undefined;
      const index = current === undefined ? (step > 0 ? 0 : rows.length - 1) : Math.min(Math.max(current + step, 0), rows.length - 1);
      const id = rows[index].id;
      if (event.shiftKey && anchor && rowIndex.has(anchor)) {
        select(range(anchor, id));
      } else {
        select(new Set([id]));
        anchor = id;
      }
      cursor = id;
      reveal(index);
    } else if (event.key === "Enter" && selection.size === 1) {
      const card = rows[rowIndex.get([...selection][0]) ?? -1];
      if (card) open(card);
    } else {
      return;
    }
    event.preventDefault();
  }

  /** Scroll so row `index` is in view. */
  function reveal(index: number) {
    if (!scroller) return;
    const top = HEADER_HEIGHT + index * ROW_HEIGHT;
    if (top < scroller.scrollTop + HEADER_HEIGHT) scroller.scrollTop = top - HEADER_HEIGHT;
    else if (top + ROW_HEIGHT > scroller.scrollTop + viewHeight) scroller.scrollTop = top + ROW_HEIGHT - viewHeight;
  }

  // MARK: Columns

  function toggleSort(column: ColumnId) {
    sort = sort?.column === column ? { column, ascending: !sort.ascending } : { column, ascending: true };
    save("browse.sort", { value: sort });
  }

  /** Dragging a column's right edge trades width with its neighbour. */
  function startResize(event: PointerEvent, index: number) {
    event.preventDefault();
    event.stopPropagation();
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    const [left, right] = [COLUMNS[index], COLUMNS[index + 1]];
    const start = { x: event.clientX, left: widths[left.id], right: widths[right.id] };
    const move = (e: PointerEvent) => {
      const total = start.left + start.right;
      const min = (c: (typeof COLUMNS)[number]) => c.minWidth / (scale || 1);
      const next = Math.min(Math.max(start.left + (e.clientX - start.x) / (scale || 1), min(left)), total - min(right));
      widths = { ...widths, [left.id]: next, [right.id]: total - next };
    };
    const end = () => {
      handle.removeEventListener("pointermove", move);
      save("browse.widths", widths);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", end, { once: true });
  }

  // MARK: Cells

  // Cells are refilled as rows scroll into view, and rendering Markdown is the slow part.
  const cache = new Map<string, string>();
  function line(source: string): string {
    let html = cache.get(source);
    if (html === undefined) {
      if (cache.size > 20_000) cache.clear();
      html = renderInline(source.replace(/\s*\n\s*/g, " "), true);
      cache.set(source, html);
    }
    return html;
  }

  const dueFormat = $derived(new Intl.DateTimeFormat(app.environment.locale, { day: "numeric", month: "short", year: "numeric" }));

  // MARK: Preview

  /** The card shown beside the table, when exactly one is selected. */
  const selected = $derived.by(() => {
    if (selection.size !== 1) return null;
    const index = rowIndex.get([...selection][0]);
    return index === undefined ? null : rows[index];
  });

  const PREVIEW_MIN = 240;
  const TABLE_MIN = 320;
  let layoutWidth = $state(0);
  let previewWidth = $state(load<{ value: number }>("browse.preview", { value: 360 }).value);
  const shownPreviewWidth = $derived(Math.max(PREVIEW_MIN, Math.min(previewWidth, layoutWidth - TABLE_MIN)));

  /** Dragging the divider resizes the preview; the table takes the rest. */
  function startPreviewResize(event: PointerEvent) {
    event.preventDefault();
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    const start = { x: event.clientX, width: shownPreviewWidth };
    const move = (e: PointerEvent) => {
      previewWidth = Math.max(PREVIEW_MIN, Math.min(start.width - (e.clientX - start.x), layoutWidth - TABLE_MIN));
    };
    const end = () => {
      handle.removeEventListener("pointermove", move);
      save("browse.preview", { value: previewWidth });
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", end, { once: true });
  }
</script>

<div class="browse" bind:clientWidth={layoutWidth}>
<div
  class="scroller"
  class:focused
  bind:this={scroller}
  bind:clientWidth={viewWidth}
  bind:clientHeight={viewHeight}
  onscroll={() => (scrollTop = scroller?.scrollTop ?? 0)}
  tabindex="0"
  role="grid"
  aria-multiselectable="true"
  aria-rowcount={rows.length}
  onfocusin={() => (focused = true)}
  onfocusout={(e) => (focused = scroller?.contains(e.relatedTarget as Node | null) ?? false)}
  {onkeydown}
>
  <div class="table" style:width="{tableWidth}px" style:height="{HEADER_HEIGHT + rows.length * ROW_HEIGHT}px">
    <div class="header" role="row" style:grid-template-columns={template}>
      {#each COLUMNS as column, i (column.id)}
        <div
          class="heading"
          class:numeric={column.numeric}
          role="columnheader"
          aria-sort={sort?.column === column.id ? (sort.ascending ? "ascending" : "descending") : "none"}
        >
          <button
            class="sort"
            class:sortable={column.sortable}
            tabindex="-1"
            disabled={!column.sortable}
            onclick={() => toggleSort(column.id)}
          >
            <span class="title">{column.title}</span>
            {#if sort?.column === column.id}
              <span class="arrow">{sort.ascending ? "▲" : "▼"}</span>
            {/if}
          </button>
          {#if i < COLUMNS.length - 1}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <span class="resize" onpointerdown={(e) => startResize(e, i)}></span>
          {/if}
        </div>
      {/each}
    </div>

    {#each visible as card, offset (card.id)}
      {@const index = first + offset}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        class="row"
        class:odd={index % 2 === 1}
        class:selected={selection.has(card.id)}
        class:context={contextRow === card.id}
        class:suspended={card.suspended}
        role="row"
        tabindex="-1"
        aria-selected={selection.has(card.id)}
        style:top="{HEADER_HEIGHT + index * ROW_HEIGHT}px"
        style:grid-template-columns={template}
        onmousedown={(e) => onRowDown(e, card)}
        ondblclick={() => open(card)}
        oncontextmenu={() => onContextMenu(card)}
      >
        <div class="cell">{@html line(card.front)}</div>
        <div class="cell secondary">{@html line(card.back)}</div>
        <div class="cell" title={card.deck}>{deckTitle(card.deck)}</div>
        <div class="cell"><TagPills tags={card.tags} /></div>
        {#if card.suspended}
          <div class="cell tertiary" title="Tagged #suspended, so it doesn't come up for study">Suspended</div>
        {:else if card.due === null}
          <div class="cell tertiary">New</div>
        {:else}
          <div class="cell">{dueFormat.format(new Date(card.due * 1000))}</div>
        {/if}
        <div class="cell numeric">{card.reps}</div>
        <div class="cell numeric">{card.lapses}</div>
      </div>
    {/each}
  </div>

  {#if query && rows.length === 0}
    <div class="no-results">
      <svg width="40" height="40" viewBox="0 0 20 20" aria-hidden="true">
        <circle cx="8.5" cy="8.5" r="5.5" fill="none" stroke="currentColor" stroke-width="1.4" />
        <path d="M12.6 12.6l4.4 4.4" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
      </svg>
      <h1>No Results for “{query}”</h1>
      <p>Check the spelling or try a new search.</p>
    </div>
  {/if}
</div>

{#if app.preview}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="divider" onpointerdown={startPreviewResize}></div>
  <aside class="preview" style:width="{shownPreviewWidth}px" aria-label="Selected card">
    {#if selected}
      <div class="card">
        <CardView card={selected} />
      </div>
      <footer class="tabular">
        {#if selected.suspended}
          <span>Suspended</span>
        {:else if selected.due === null}
          <span>New</span>
        {:else}
          <span>Due {dueFormat.format(new Date(selected.due * 1000))}</span>
        {/if}
        <span>{selected.reps === 1 ? "1 review" : `${selected.reps} reviews`}</span>
        <span>{selected.lapses === 1 ? "1 lapse" : `${selected.lapses} lapses`}</span>
      </footer>
    {:else}
      <p class="empty">{selection.size > 1 ? `${selection.size} cards selected` : "No card selected"}</p>
    {/if}
  </aside>
{/if}
</div>

<style>
  .browse {
    display: flex;
    height: 100%;
  }

  .scroller {
    position: relative;
    flex: 1;
    min-width: 0;
    height: 100%;
    overflow: auto;
    outline: none;
    font-variant-numeric: tabular-nums;
  }

  .table {
    position: relative;
    min-width: 100%;
    padding: 0 10px;
  }

  .header,
  .row {
    display: grid;
    position: absolute;
    left: 10px;
    right: 10px;
  }

  .header {
    position: sticky;
    top: 0;
    height: 28px;
    z-index: 1;
    margin: 0 -10px;
    padding: 0 10px;
    background: var(--text-background);
    box-shadow: 0 1px 0 var(--separator);
  }

  .heading {
    position: relative;
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    padding: 0 6px;
    font-size: 11px;
    font-weight: 500;
    color: var(--secondary-label);
  }

  .heading.numeric .sort {
    justify-content: flex-end;
  }

  .sort {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 4px;
    min-width: 0;
    height: 100%;
    padding: 0;
    border: none;
    background: none;
    color: inherit;
    font: inherit;
    text-align: inherit;
  }

  .sort:disabled {
    color: inherit;
  }

  .sort.sortable:hover {
    color: var(--label);
  }

  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .arrow {
    font-size: 7px;
  }

  .heading:not(:last-child)::after {
    content: "";
    position: absolute;
    right: 0;
    top: 7px;
    bottom: 7px;
    border-right: 1px solid var(--separator);
  }

  .resize {
    position: absolute;
    right: -4px;
    top: 0;
    bottom: 0;
    width: 8px;
    cursor: col-resize;
    z-index: 1;
  }

  .row {
    height: 24px;
    align-items: center;
    border-radius: 5px;
  }

  .row.odd {
    background: color-mix(in srgb, var(--label) 3.5%, transparent);
  }

  .row.selected {
    background: var(--unfocused-selected-row);
  }

  .focused .row.selected {
    background: var(--selected-row);
    color: var(--selected-text);
  }

  .focused .row.selected .secondary,
  .focused .row.selected .tertiary,
  .focused .row.selected :global(.pill) {
    color: inherit;
  }

  /* Suspended cards stay listed, but read as out of rotation. */
  .row.suspended:not(.selected) {
    color: var(--secondary-label);
  }

  .row.context {
    box-shadow: inset 0 0 0 2px var(--selected-row);
  }

  .cell {
    min-width: 0;
    padding: 0 6px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cell :global(code) {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  }

  .numeric {
    text-align: right;
  }

  .secondary {
    color: var(--secondary-label);
  }

  .tertiary {
    color: var(--tertiary-label);
  }

  .divider {
    flex: none;
    width: 1px;
    background: var(--separator);
    position: relative;
    cursor: col-resize;
  }

  /* A wider target than the line itself. */
  .divider::after {
    content: "";
    position: absolute;
    inset: 0 -4px;
  }

  .preview {
    flex: none;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
  }

  .preview .card {
    flex: 1;
    overflow-y: auto;
    padding: 24px 24px 16px;
  }

  .preview footer {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 14px;
    padding: 10px 24px;
    border-top: 1px solid var(--separator);
    font-size: var(--font-callout);
    color: var(--secondary-label);
  }

  .empty {
    margin: auto;
    color: var(--tertiary-label);
    font-size: var(--font-title3);
  }

  .no-results {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    color: var(--secondary-label);
    pointer-events: none;
  }

  .no-results h1 {
    margin: 8px 0 0;
    font-size: var(--font-title2);
    font-weight: 600;
    color: var(--label);
  }

  .no-results p {
    margin: 0;
  }
</style>
