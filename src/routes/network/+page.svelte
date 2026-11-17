<script lang="ts">
  import type { Device } from "$lib/api";
  import { devices } from "$lib/devices.svelte";
  import { count } from "$lib/format";
  import { scan } from "$lib/scan.svelte";
  import { status } from "$lib/status.svelte";
  import DeviceTable from "$lib/components/DeviceTable.svelte";
  import ScanStatus from "$lib/components/ScanStatus.svelte";

  /** Sorts by IP address, number by number; names that aren't addresses go last, by name. */
  function byAddress(a: Device, b: Device): number {
    const key = (d: Device) => {
      const parts = d.host.split(".").map(Number);
      return parts.length === 4 && parts.every((n) => Number.isInteger(n) && n >= 0 && n <= 255)
        ? parts.reduce((sum, n) => sum * 256 + n, 0)
        : Infinity;
    };
    return key(a) - key(b) || a.host.localeCompare(b.host);
  }

  const sorted = $derived([...devices.list].sort(byAddress));
  const fresh = $derived(sorted.filter((d) => !d.reviewed && d.kind !== "ignore"));
  const mesh = $derived(sorted.filter((d) => d.reviewed && d.kind === "mesh"));
  const others = $derived(sorted.filter((d) => d.reviewed && d.kind !== "mesh" && d.kind !== "ignore"));
  const ignored = $derived(sorted.filter((d) => d.kind === "ignore"));

  function upCount(list: Device[]): string {
    const up = list.filter((d) => status.health(d) === "up").length;
    return `${up} of ${list.length} up`;
  }
</script>

<header class="page-head">
  <h1>Network</h1>
  <div class="actions">
    <button class="btn" onclick={() => status.checkNow()} disabled={status.checking} title="Check every device now instead of waiting for the timer">
      {status.checking ? "Checking…" : "Check now"}
    </button>
    <button class="btn primary" onclick={() => scan.start()} disabled={scan.running}>
      {scan.running ? "Scanning…" : "Scan network"}
    </button>
  </div>
</header>

{#if status.checkError}
  <p class="alert gap" role="alert">The check didn't finish: {status.checkError}</p>
{/if}

<ScanStatus />

{#if devices.loaded && devices.list.length === 0}
  <section class="panel empty">
    <p>No devices yet. Press "Scan network" to find everything on your home network.</p>
  </section>
{:else}
  <div class="groups">
    <div class="side">
      {#if fresh.length > 0}
        <section class="panel group">
          <div class="group-head">
            <h2 class="section-title">New <span class="badge">{fresh.length}</span></h2>
            <span class="muted small">Found by a scan. Name each one if you like, then pick what it is.</span>
          </div>
          <DeviceTable list={fresh} variant="fresh" />
        </section>
      {/if}

      <section class="panel group">
        <div class="group-head">
          <h2 class="section-title">Mesh nodes</h2>
          {#if mesh.length > 0}<span class="muted small">{upCount(mesh)}</span>{/if}
        </div>
        {#if mesh.length === 0}
          <p class="muted">No mesh nodes yet. A scan picks out your Deco nodes by their maker; you can also set a device's type to "Mesh node".</p>
        {:else}
          <DeviceTable list={mesh} variant="known" />
        {/if}
      </section>
    </div>

    <div class="main">
      <section class="panel group">
        <div class="group-head">
          <h2 class="section-title">Devices</h2>
          {#if others.length > 0}<span class="muted small">{upCount(others)}</span>{/if}
        </div>
        {#if others.length === 0}
          <p class="muted">Devices you have added or accepted show here.</p>
        {:else}
          <DeviceTable list={others} variant="known" />
        {/if}
      </section>

      {#if ignored.length > 0}
        <details class="panel group">
          <summary>
            <span class="section-title">Ignored</span>
            <span class="muted small">{count(ignored.length, "device")}, not checked</span>
          </summary>
          <DeviceTable list={ignored} variant="ignored" />
        </details>
      {/if}
    </div>
  </div>
{/if}

<style>
  .gap { margin-bottom: var(--gap); }
  .groups, .side, .main { display: flex; flex-direction: column; gap: var(--gap); min-width: 0; }

  /* Ultrawide: the long device list on the left, the short groups beside it.
     Not at 2560, where splitting would cost each table its last columns. */
  @media (min-width: 3000px) {
    .groups { display: grid; grid-template-columns: minmax(0, 1.3fr) minmax(0, 1fr); align-items: start; }
    .main { grid-column: 1; grid-row: 1; }
    .side { grid-column: 2; grid-row: 1; }
  }

  .group { padding: 0.9rem 0.7rem 0.5rem; }
  .group-head, summary { display: flex; align-items: baseline; gap: 0.3rem 1rem; flex-wrap: wrap; padding: 0 0.55rem 0.35rem; }
  .group-head .section-title, summary .section-title { margin: 0; }
  .group > p { margin: 0; padding: 0.3rem 0.55rem 0.6rem; line-height: 1.5; }
  .badge {
    display: inline-block;
    min-width: 1.5rem;
    padding: 0.05rem 0.4rem;
    margin-left: 0.3rem;
    border-radius: 999px;
    background: var(--accent);
    color: var(--accent-ink);
    font-size: 0.85rem;
    letter-spacing: 0;
    text-align: center;
  }

  details.group { padding-bottom: 0.9rem; }
  details[open].group { padding-bottom: 0.5rem; }
  summary { cursor: pointer; padding-bottom: 0; border-radius: var(--radius-sm); }
  details[open] summary { padding-bottom: 0.35rem; }
  summary { list-style: none; }
  summary::-webkit-details-marker { display: none; }
  summary::before { content: "▸"; color: var(--muted); }
  details[open] summary::before { content: "▾"; }

  .empty p { margin: 0; color: var(--text-soft); }
</style>
