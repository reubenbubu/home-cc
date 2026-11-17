<script lang="ts">
  // The Overview's Pi-hole panel: one line per Pi-hole, whether it blocks and how much it blocked today. Reads
  // every Pi-hole while the Overview is open. Shows nothing at all when no machine is a Pi-hole.
  import { onMount } from "svelte";
  import type { Device } from "$lib/api";
  import { clock } from "$lib/clock.svelte";
  import { devices } from "$lib/devices.svelte";
  import { blockingView, percent, TROUBLE } from "$lib/pihole/logic";
  import { pihole } from "$lib/pihole/store.svelte";

  onMount(() => pihole.watchAll());

  const shown = $derived(
    pihole.hosts
      .map((h) => devices.byId.get(h.deviceId))
      .filter((d): d is Device => !!d)
      .sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true })),
  );
</script>

{#if shown.length > 0}
  <section class="panel" aria-label="Pi-hole">
    <h2 class="section-title"><a href="/pihole">Pi-hole</a></h2>
    <ul class="lines">
      {#each shown as d (d.id)}
        {@const s = pihole.statuses.get(d.id)}
        {@const good = pihole.lastGood.get(d.id)}
        {@const failed = pihole.readErrors.has(d.id) || !!s?.error}
        {@const v = good && !failed ? blockingView(good.reading.blocking, good.fetchedAt, clock.now) : null}
        <li>
          <a href="/pihole" class="line">
            <span class="dot {v ? v.tone : failed ? 'bad' : 'idle'}"></span>
            <span class="name ellipsis" title={d.name}>{d.name}</span>
            <span class="what small">
              {#if v && good}
                {v.label}{v.detail && v.paused ? ` · ${v.detail}` : ""}
                <span class="muted">· {percent(good.reading.summary.percentBlocked)} blocked today</span>
              {:else if failed}
                <span class="muted">{pihole.readErrors.has(d.id) ? "couldn't be read" : TROUBLE[s?.errorKind ?? "other"]}</span>
              {:else}
                <span class="muted">Reading…</span>
              {/if}
            </span>
          </a>
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  .section-title a { color: inherit; text-decoration: none; }
  .section-title a:hover { text-decoration: underline; }
  .lines { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.45rem; }
  .line { display: flex; align-items: baseline; gap: 0.6rem; color: var(--text); text-decoration: none; min-width: 0; flex-wrap: wrap; }
  .line:hover .name { text-decoration: underline; }
  .line .dot { align-self: center; }
  .name { font-weight: 600; flex: 0 1 auto; }
  .what { color: var(--text-soft); flex: 1 1 12rem; min-width: 0; }
  .dot.idle { border: 2px solid var(--idle); }
</style>
