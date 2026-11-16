<script lang="ts">
  import { devices } from "$lib/devices.svelte";
  import { KIND_LABELS, OS_LABELS } from "$lib/api";

  const counts = $derived({
    total: devices.list.length,
    machines: devices.list.filter((d) => d.kind === "pc" || d.kind === "server").length,
    vms: devices.list.filter((d) => d.kind === "vm" || d.kind === "container").length,
    mesh: devices.list.filter((d) => d.kind === "mesh").length,
  });

  const machines = $derived(devices.list.filter((d) => d.kind !== "mesh"));
  const meshNodes = $derived(devices.list.filter((d) => d.kind === "mesh"));
</script>

<header class="head">
  <h1>Overview</h1>
  <div class="actions">
    <button class="btn" disabled title="Arrives with the network scanner">Scan network</button>
    <a class="btn primary link-btn" href="/machines?add=1">Add machine</a>
  </div>
</header>

<section class="stats" aria-label="Summary">
  <div class="panel stat"><div class="muted">Devices</div><div class="value mono">{counts.total}</div></div>
  <div class="panel stat"><div class="muted">PCs and servers</div><div class="value mono">{counts.machines}</div></div>
  <div class="panel stat"><div class="muted">VMs and containers</div><div class="value mono">{counts.vms}</div></div>
  <div class="panel stat"><div class="muted">Mesh nodes</div><div class="value mono">{counts.mesh}</div></div>
</section>

{#if devices.error}
  <p class="panel error" role="alert">Couldn't load devices: {devices.error}</p>
{/if}

<div class="layout">
  <section>
    <h2 class="section-title">Machines</h2>
    {#if devices.loaded && machines.length === 0}
      <div class="panel empty">
        <p>No machines yet. Add your PCs, VMs and Proxmox host to see them here.</p>
        <a class="btn primary link-btn" href="/machines?add=1">Add your first machine</a>
      </div>
    {:else}
      <div class="auto-grid">
        {#each machines as m (m.id)}
          <article class="panel card">
            <div class="card-top">
              <div class="card-name">
                <span class="dot idle" title="Not checked yet"></span>
                <span>{m.name}</span>
              </div>
              <span class="mono muted small">{m.host}</span>
            </div>
            <div class="muted small">{OS_LABELS[m.os]} · {KIND_LABELS[m.kind]}</div>
            <div class="muted small">Live status arrives in the next step.</div>
          </article>
        {/each}
      </div>
    {/if}
  </section>

  <aside class="side">
    <section class="panel">
      <h2 class="section-title">Deco mesh</h2>
      {#if meshNodes.length === 0}
        <p class="muted">The network scanner will find your 4 Deco nodes. You can also add them by hand as "Mesh node".</p>
      {:else}
        <div class="mesh">
          {#each meshNodes as n (n.id)}
            <div class="mesh-node">
              <div class="card-name"><span class="dot idle"></span>{n.name}</div>
              <div class="mono muted small">{n.host}</div>
            </div>
          {/each}
        </div>
      {/if}
    </section>
    <section class="panel">
      <h2 class="section-title">Containers</h2>
      <p class="muted">Docker containers from each host will show here once the Docker module is in.</p>
    </section>
  </aside>
</div>

<style>
  .head { display: flex; align-items: center; justify-content: space-between; gap: 1rem; flex-wrap: wrap; margin-bottom: 1.25rem; }
  h1 { margin: 0; font-size: 1.55rem; font-weight: 600; }
  .actions { display: flex; gap: 0.6rem; }
  .link-btn { display: inline-flex; align-items: center; text-decoration: none; }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }

  .stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(12rem, 1fr)); gap: var(--gap); margin-bottom: 1.4rem; }
  .stat .value { margin-top: 0.4rem; font-size: 1.85rem; }

  .layout { display: grid; grid-template-columns: minmax(0, 1fr); gap: 1.4rem; }
  .side { display: grid; grid-template-columns: repeat(auto-fit, minmax(22rem, 1fr)); gap: var(--gap); align-content: start; }

  /* Ultrawide and other very wide windows: put the side panels next to the machines. */
  @media (min-width: 2400px) {
    .layout { grid-template-columns: minmax(0, 1fr) 30rem; }
    .side { grid-template-columns: minmax(0, 1fr); }
  }

  .card { display: flex; flex-direction: column; gap: 0.6rem; padding: 1rem; }
  .card-top { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem; }
  .card-name { display: flex; align-items: center; gap: 0.6rem; font-weight: 600; min-width: 0; }
  .small { font-size: 0.85rem; }

  .mesh { display: grid; grid-template-columns: repeat(auto-fill, minmax(10rem, 1fr)); gap: 0.6rem; }
  .mesh-node { padding: 0.75rem; border: 1px solid var(--track); border-radius: var(--radius-sm); display: flex; flex-direction: column; gap: 0.4rem; }

  .empty { display: flex; flex-direction: column; align-items: flex-start; gap: 0.75rem; }
  .empty p { margin: 0; color: var(--text-soft); }
  .error { color: var(--bad); }
  p { margin: 0; line-height: 1.5; }
</style>
