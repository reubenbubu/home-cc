<script lang="ts" generics="T">
  // A column's header as a button that sorts the table by it: one way, the other way, then back to the
  // table's own order. Goes inside the table's own <th>, which takes aria-sort={view.ariaSort(col)}.
  import { dirWords } from "$lib/table";
  import type { TableView } from "$lib/table.svelte";
  import { icons } from "$lib/icons";
  import Icon from "../Icon.svelte";

  let {
    view,
    col,
    compact = false,
  }: {
    view: TableView<T>;
    /** The column's id. */
    col: string;
    /** Only the arrow shows (the name is still read out), for a narrow column such as the status dot. */
    compact?: boolean;
  } = $props();

  const column = $derived(view.columns.find((c) => c.id === col));
  const dir = $derived(view.sort?.col === col ? view.sort.dir : null);
  const title = $derived.by(() => {
    if (!column) return "";
    const [up, down] = dirWords(column);
    if (dir === "asc") return `${column.label}: sorted ${up}. Click for ${down}.`;
    if (dir === "desc") return `${column.label}: sorted ${down}. Click for the usual order (${view.usual}).`;
    return `Sort by ${column.label}, ${up}`;
  });
</script>

<button type="button" class="sort" class:on={dir !== null} class:compact onclick={() => view.cycle(col)} {title}>
  {#if compact}<span class="sr-only">{column?.label}</span>{:else}<span class="label">{column?.label}</span>{/if}
  <span class="arrow"><Icon path={dir === "asc" ? icons.sortUp : dir === "desc" ? icons.sortDown : icons.sortBoth} size={12} /></span>
</button>

<style>
  .sort {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    max-width: 100%;
    min-height: 1.9rem;
    margin: -0.3rem -0.3rem;
    padding: 0 0.3rem;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: none;
    color: inherit;
    font: inherit;
    font-weight: inherit;
    text-align: left;
    cursor: pointer;
  }
  .label { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* Takes no room until it is wanted, so a header's name isn't cut short for it. */
  .arrow { display: none; flex: none; opacity: 0.6; }
  .sort:hover { color: var(--text); }
  .sort:hover .arrow, .sort:focus-visible .arrow, .compact .arrow, .on .arrow { display: inline-grid; }
  .sort:focus-visible { outline: 2px solid var(--accent); outline-offset: 0; }
  .on { color: var(--text); }
  .on .arrow, .on.compact .arrow { opacity: 1; color: var(--accent); }
</style>
