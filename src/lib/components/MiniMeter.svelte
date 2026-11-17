<script lang="ts">
  // Small version of Meter for the Overview cards: label and number above a short bar.
  import Icon from "./Icon.svelte";
  import { icons } from "$lib/icons";
  import { pct } from "$lib/format";

  let {
    label,
    value,
    text,
    warn = null,
  }: { label: string; value: number | null; text?: string; warn?: string | null } = $props();

  const width = $derived(value == null || !Number.isFinite(value) ? 0 : Math.min(100, Math.max(0, value)));
</script>

<div class="mini" title={warn ?? undefined}>
  <div class="top">
    <span class="muted">{label}</span>
    <span class="mono">{text ?? pct(value)}</span>
    {#if warn}<span class="flag"><Icon path={icons.warn} size={12} /><span class="sr-only">{warn}</span></span>{/if}
  </div>
  <span class="track" aria-hidden="true"><span class="fill" class:warn={!!warn} style:width="{width}%"></span></span>
</div>

<style>
  .mini { display: flex; flex-direction: column; gap: 0.3rem; min-width: 0; }
  .top { display: flex; align-items: baseline; gap: 0.35rem; font-size: 0.85rem; white-space: nowrap; }
  .flag { color: var(--warn); align-self: center; display: inline-flex; }
  .track { height: 0.3rem; }
</style>
