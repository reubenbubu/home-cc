<script lang="ts">
  // Proxmox (spec 5.6): every Proxmox host the owner has added (a device whose System is Proxmox VE), one
  // at a time when there are several.
  import { devices } from "$lib/devices.svelte";
  import { drafts } from "$lib/drafts.svelte";
  import ProxmoxHost from "$lib/components/proxmox/ProxmoxHost.svelte";

  const byName = (a: { name: string }, b: { name: string }) => a.name.localeCompare(b.name, undefined, { numeric: true });
  const hosts = $derived(devices.list.filter((d) => d.os === "proxmox" && d.kind !== "ignore").sort(byName));

  // The host picked stays picked after another screen was shown.
  let chosen = $state<number | null>(drafts.get<number>("proxmox-host") ?? null);
  $effect(() => drafts.remember("proxmox-host", chosen));
  const host = $derived(hosts.find((h) => h.id === chosen) ?? hosts[0] ?? null);
</script>

<header class="page-head">
  <h1>Proxmox</h1>
  {#if hosts.length > 1}
    <div class="tabs" role="tablist" aria-label="Proxmox hosts">
      {#each hosts as h (h.id)}
        <button
          class="btn small"
          class:current={h.id === host?.id}
          role="tab"
          aria-selected={h.id === host?.id}
          onclick={() => (chosen = h.id)}
        >
          {h.name}
        </button>
      {/each}
    </div>
  {/if}
</header>

{#if !devices.loaded}
  <p class="muted">Loading…</p>
{:else if !host}
  <section class="panel empty">
    <h2 class="section-title">No Proxmox host yet</h2>
    <p>
      Add your Proxmox PC on the Machines screen (or pick it from a network scan) and set its System to
      <strong>Proxmox VE</strong>. Then come back here to add its API token.
    </p>
    <p><a class="btn link-btn" href="/machines?add=1">Add a machine</a></p>
  </section>
{:else}
  {#key host.id}
    <ProxmoxHost device={host} />
  {/key}
{/if}

<style>
  .tabs { display: flex; gap: 0.4rem; flex-wrap: wrap; }
  .tabs .current { background: var(--panel-raised); border-color: var(--accent); }
  .empty { max-width: 46rem; display: flex; flex-direction: column; gap: 0.7rem; }
  .empty .section-title { margin: 0; }
  .empty p { margin: 0; line-height: 1.55; color: var(--text-soft); }
</style>
