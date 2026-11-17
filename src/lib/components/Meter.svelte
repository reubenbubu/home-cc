<script lang="ts">
  // One labelled bar: CPU, RAM or a disk. A warning changes the bar's colour
  // and adds a line of text, so it doesn't rely on colour alone.
  // Goes inside a "meter-grid" (app.css), which lines up the bars of a card.
  import Icon from "./Icon.svelte";
  import { icons } from "$lib/icons";
  import { pct } from "$lib/format";

  let {
    label,
    hint,
    value,
    text,
    detail = "",
    warn = null,
    mono = false,
  }: {
    label: string;
    /** Longer text for the label's tooltip. */
    hint?: string;
    /** 0 to 100, or null when there is nothing to draw yet. */
    value: number | null;
    /** Shown after the bar instead of the percentage. */
    text?: string;
    detail?: string;
    warn?: string | null;
    mono?: boolean;
  } = $props();

  const width = $derived(value == null || !Number.isFinite(value) ? 0 : Math.min(100, Math.max(0, value)));
</script>

<div class="meter">
  <span class="label ellipsis" class:mono title={hint ?? label}>{label}</span>
  {#if value == null}
    <span class="track" aria-hidden="true"></span>
  {:else}
    <span
      class="track"
      role="meter"
      aria-label={hint ?? label}
      aria-valuemin="0"
      aria-valuemax="100"
      aria-valuenow={Math.round(width)}
      aria-valuetext="{pct(width)}{warn ? `, ${warn}` : ''}"
    >
      <span class="fill" class:warn={!!warn} style:width="{width}%"></span>
    </span>
  {/if}
  <span class="value mono">{text ?? pct(value)}</span>
  <span class="detail mono muted ellipsis" title={detail}>{detail}</span>
  {#if warn}
    <span class="warn-line"><Icon path={icons.warn} size={13} /> {warn}</span>
  {/if}
</div>

<style>
  .meter {
    display: grid;
    grid-template-columns: subgrid;
    grid-column: 1 / -1;
    align-items: center;
    row-gap: 0.15rem;
  }
  .label { color: var(--text-soft); }
  .label.mono { font-size: 0.92rem; }
  .value { text-align: right; font-size: 0.92rem; white-space: nowrap; }
  .detail { text-align: right; font-size: 0.85rem; }
  .warn-line {
    grid-column: 2 / -1;
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.85rem;
    color: var(--text-soft);
  }
  .warn-line :global(svg) { color: var(--warn); flex: none; }
</style>
