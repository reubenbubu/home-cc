<script lang="ts" generics="T">
  // The heading of one group of rows: the value they share and how many there are. A click collapses or
  // opens the group. It is the first row of the group's own <tbody>.
  //
  // It spans every column the table shows, and when the table's own styles turn its rows into blocks (a
  // narrow window), it becomes a block too: both are worked out here, so a table needs nothing extra.
  // `grid` is for a table made of other elements than <table> (role="table", rows in role="rowgroup"):
  // there it is a block of its own, as wide as the list.
  import type { Section, TableView } from "$lib/table.svelte";
  import { icons } from "$lib/icons";
  import Icon from "../Icon.svelte";

  let { view, section, grid = false }: { view: TableView<T>; section: Section<T>; grid?: boolean } = $props();

  function fit(th: HTMLTableCellElement) {
    const table = th.closest("table");
    const row = th.parentElement;
    const update = () => {
      const cells = [...(table?.tHead?.rows[0]?.cells ?? [])];
      th.colSpan = Math.max(1, cells.filter((c) => getComputedStyle(c).display !== "none").length);
      const body = th.closest("tbody");
      row?.classList.toggle("flat", !!body && getComputedStyle(body).display !== "table-row-group");
    };
    update();
    const watch = new ResizeObserver(update);
    if (table) watch.observe(table);
    return { destroy: () => watch.disconnect() };
  }
</script>

{#snippet toggle()}
  <button type="button" class="toggle" aria-expanded={section.open} onclick={() => view.toggle(section.key)}>
    <span class="chevron" class:open={section.open}><Icon path={icons.chevron} size={14} /></span>
    <span class="label">{section.label}</span>
    <span class="count"><span class="sr-only">, </span>{section.count}<span class="sr-only"> {section.count === 1 ? "row" : "rows"}</span></span>
  </button>
{/snippet}

{#if grid}
  <div class="group-row grid" role="row">
    <div class="cell" role="rowheader">{@render toggle()}</div>
  </div>
{:else}
  <tr class="group-row">
    <th class="cell" scope="rowgroup" use:fit>{@render toggle()}</th>
  </tr>
{/if}

<style>
  .cell {
    padding: 0.55rem 0.55rem 0.25rem;
    border-bottom: 1px solid var(--border);
    text-align: left;
    font-weight: 600;
    white-space: nowrap;
  }
  /* Set by fit() above, without waiting for a redraw. */
  .group-row:global(.flat), .group-row:global(.flat) th { display: block; }
  .grid .cell { padding: 0.45rem 0 0.2rem; }
  .toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.45rem;
    max-width: 100%;
    min-height: 2.1rem;
    margin-left: -0.35rem;
    padding: 0 0.6rem 0 0.25rem;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }
  .toggle:hover { background: var(--panel-raised); }
  .toggle:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .label { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .chevron { display: inline-grid; flex: none; color: var(--muted); transition: transform 0.15s; }
  .chevron.open { transform: rotate(90deg); }
  .count {
    flex: none;
    min-width: 1.5rem;
    padding: 0 0.4rem;
    border-radius: 999px;
    background: var(--panel-raised);
    color: var(--muted);
    font-size: 0.85rem;
    font-weight: 500;
    text-align: center;
  }
  @media (prefers-reduced-motion: reduce) {
    .chevron { transition: none; }
  }
</style>
