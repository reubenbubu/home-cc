<script lang="ts">
  // Progress of a running scan, or the range and the result of the last one.
  // The compact form (Overview) shows nothing until there is something to say.
  import { clock } from "$lib/clock.svelte";
  import { count, relTime } from "$lib/format";
  import { PHASE_TEXT, scan } from "$lib/scan.svelte";
  import { settings } from "$lib/settings.svelte";
  import Problem from "./Problem.svelte";

  let { compact = false }: { compact?: boolean } = $props();

  const p = $derived(scan.progress);
  const share = $derived(p && p.total > 0 ? Math.min(100, Math.max(0, (p.done / p.total) * 100)) : 0);
  const visible = $derived(!compact || scan.running || scan.result !== null || scan.error !== null);
</script>

{#if visible}
  <section class="panel scan" aria-label="Network scan">
    {#if scan.running}
      <div class="row">
        <div class="what">
          <strong>{scan.stopping ? "Stopping…" : p ? PHASE_TEXT[p.phase] : "Starting the scan…"}</strong>
          <span class="muted">step {scan.step} of 3 · range {settings.scanRangeLabel}</span>
        </div>
        <div class="numbers mono" aria-live="off">
          {#if p}
            <span>{p.done} of {p.total} {p.phase === "sweep" ? "addresses" : "devices"}</span>
            <span class="found">{count(p.found, "device")} found</span>
          {/if}
        </div>
        <button class="btn small" onclick={() => scan.stop()} disabled={scan.stopping}>Stop</button>
      </div>
      <span
        class="track"
        role="progressbar"
        aria-label="Scan progress"
        aria-valuemin="0"
        aria-valuemax="100"
        aria-valuenow={Math.round(share)}
      >
        <span class="fill" style:width="{share}%"></span>
      </span>
    {:else}
      <div class="row">
        {#if !compact}
          <div class="what">
            <span class="muted">Range to scan</span>
            <span class="mono">{settings.scanRangeLabel}</span>
            <a href="/settings">Change</a>
          </div>
        {/if}
        {#if scan.result}
          {@const r = scan.result}
          <div class="result" role="status">
            <strong>{count(r.found.length, "device")} found, {r.added} new</strong>
            <span class="muted">
              {r.cancelled ? "scan stopped early · " : ""}<span class="mono">{r.range}</span> · {relTime(scan.finishedAt, clock.now)}
            </span>
            {#if compact && r.added > 0}<a href="/network">Review on Network</a>{/if}
          </div>
        {:else if !compact}
          <div class="result muted">No scan yet in this session. A scan of a home network takes about half a minute.</div>
        {/if}
      </div>
    {/if}
    {#if scan.error}
      <Problem
        problem={{ area: "scan", what: "Scanning the network", error: scan.error, details: { "Range to scan": settings.scanRangeLabel } }}
        message="The scan didn't finish: {scan.error}"
      />
    {/if}
  </section>
{/if}

<style>
  .scan { display: flex; flex-direction: column; gap: 0.7rem; padding-block: 0.8rem; margin-bottom: var(--gap); }
  .row { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem 1.5rem; flex-wrap: wrap; min-height: 2.25rem; }
  .what, .result { display: flex; align-items: baseline; gap: 0.3rem 0.75rem; flex-wrap: wrap; }
  .numbers { display: flex; gap: 1.25rem; margin-left: auto; font-size: 0.92rem; white-space: nowrap; }
  .found { color: var(--accent); }
</style>
