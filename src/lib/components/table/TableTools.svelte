<script lang="ts" generics="T">
  // The bar above a table: a filter box, a quick choice for the columns that have one, the sort and the
  // grouping, and how many rows are shown while some are filtered out. The sort is named here as well as
  // on its column's header, so it is never hidden when a narrow window drops that column or the headers.
  // When the filter hides every row, it says so under the bar, with a way to show them all again.
  import { parseSortChoice } from "$lib/table";
  import type { TableView } from "$lib/table.svelte";
  import { icons } from "$lib/icons";
  import Icon from "../Icon.svelte";

  let {
    view,
    label,
    noun = "rows",
  }: {
    view: TableView<T>;
    /** What the table is, for the controls' names: "devices", "mesh nodes". */
    label: string;
    /** What a row is, many of them: "devices". */
    noun?: string;
  } = $props();

  let box = $state<HTMLInputElement>();

  /** Columns with a quick choice: those whose rows have more than one value, or that have one chosen. A
   *  value chosen that no row has any more (the one device that was down is up again) stays in the menu,
   *  with none, so the menu still says what is hiding the rows. */
  const choosing = $derived(
    view.columns
      .filter((c) => c.choose)
      .map((col) => {
        const choices = view.choicesOf(col);
        const chosen = view.only[col.id];
        if (chosen !== undefined && !choices.some((c) => c.key === chosen)) {
          choices.push({ key: chosen, label: chosen || col.none || "None", count: 0 });
        }
        return { col, choices };
      })
      .filter(({ col, choices }) => choices.length > 1 || view.only[col.id] !== undefined),
  );

  const sortValue = $derived(view.sort ? `${view.sort.col}:${view.sort.dir}` : "");

  /** What hides the rows, in words: the typed filter and each choice. */
  const hiding = $derived.by(() => {
    const parts: string[] = [];
    if (view.filter.trim()) parts.push(`"${view.filter.trim()}"`);
    for (const { col, choices } of choosing) {
      const key = view.only[col.id];
      if (key !== undefined) parts.push(`${col.label}: ${choices.find((c) => c.key === key)?.label ?? (key || col.none || "None")}`);
    }
    return parts.join(" with ");
  });

  function clearText() {
    view.filter = "";
    box?.focus();
  }

  function keys(e: KeyboardEvent) {
    if (e.key === "Escape" && view.filter) {
      e.preventDefault();
      view.filter = "";
    }
  }
</script>

<div class="tools">
  <span class="find">
    <input
      bind:this={box}
      type="search"
      bind:value={view.filter}
      onkeydown={keys}
      placeholder="Filter"
      aria-label="Filter {label}"
      autocomplete="off"
      spellcheck="false"
    />
    {#if view.filter}
      <button type="button" class="clear" aria-label="Clear what is typed" title="Clear what is typed" onclick={clearText}>
        <Icon path={icons.clear} size={14} />
      </button>
    {/if}
  </span>
  {#each choosing as { col, choices } (col.id)}
    <select
      aria-label="Show only {label} with this {col.label}"
      value={view.only[col.id] ?? "\u0000all"}
      onchange={(e) => view.choose(col.id, e.currentTarget.value === "\u0000all" ? undefined : e.currentTarget.value)}
    >
      <option value={"\u0000all"}>Any {col.label.toLowerCase()}</option>
      {#each choices as c (c.key)}<option value={c.key}>{col.label}: {c.label} ({c.count})</option>{/each}
    </select>
  {/each}
  <select aria-label="Sort {label}" value={sortValue} onchange={(e) => view.setSort(parseSortChoice(e.currentTarget.value))}>
    <option value="">Sort: usual ({view.usual})</option>
    {#each view.sortable as o (o.value)}<option value={o.value}>Sort: {o.label}</option>{/each}
  </select>
  {#if view.groupable.length > 0}
    <select aria-label="Group {label}" value={view.group ?? ""} onchange={(e) => view.setGroup(e.currentTarget.value || null)}>
      <option value="">No grouping</option>
      {#each view.groupable as c (c.id)}<option value={c.id}>Group by {c.label.toLowerCase()}</option>{/each}
    </select>
  {/if}
  <span class="count muted small" role="status">{view.filtering ? `${view.shown} of ${view.total} shown` : ""}</span>
</div>
{#if view.filtering && view.shown === 0 && view.total > 0}
  <p class="nothing">
    <span>No {noun} match {hiding}.</span>
    <button type="button" class="btn small" onclick={() => view.clearFilter()}>Clear the filter</button>
  </p>
{/if}

<style>
  .tools { display: flex; flex-wrap: wrap; align-items: center; gap: 0.45rem 0.5rem; padding: 0 0.55rem 0.5rem; min-width: 0; }
  .find { position: relative; display: flex; flex: 1 1 13rem; min-width: min(100%, 11rem); max-width: 26rem; }
  input { min-height: 2.25rem; padding-right: 2.2rem; }
  input::-webkit-search-cancel-button { display: none; }
  .clear {
    position: absolute;
    top: 50%;
    right: 0.2rem;
    display: inline-grid;
    place-items: center;
    width: 1.9rem;
    height: 1.9rem;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--muted);
    cursor: pointer;
    transform: translateY(-50%);
  }
  .clear:hover { color: var(--text); background: var(--panel-raised); }
  .clear:focus-visible { outline: 2px solid var(--accent); outline-offset: 0; }
  select { width: auto; max-width: 100%; min-height: 2.25rem; padding: 0 0.5rem; flex: 0 1 auto; min-width: 0; text-overflow: ellipsis; }
  .count { white-space: nowrap; }
  .nothing {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem 0.8rem;
    margin: 0;
    padding: 0.4rem 0.55rem 0.7rem;
    color: var(--text-soft);
    overflow-wrap: anywhere;
  }
</style>
