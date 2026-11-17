<script lang="ts">
  // One Proxmox node: CPU, RAM and its system disk, then each storage it has.
  import type { ProxmoxNode, ProxmoxStorage } from "$lib/api";
  import { share, uptime, usedOfTotal } from "$lib/format";
  import { settings } from "$lib/settings.svelte";
  import Meter from "../Meter.svelte";

  let { node, stale = false }: { node: ProxmoxNode; stale?: boolean } = $props();

  const online = $derived(node.status === "online");
  const ram = $derived(share(node.memUsed, node.memTotal));
  const disk = $derived(share(node.diskUsed, node.diskTotal));

  function storageHint(s: ProxmoxStorage): string {
    const parts = [s.name];
    if (s.kind) parts.push(`type ${s.kind}`);
    if (s.content.length) parts.push(`holds ${s.content.join(", ")}`);
    if (s.shared) parts.push("shared");
    return parts.join(" · ");
  }

  const diskWarn = (pct: number | null) =>
    pct != null && pct > settings.value.diskWarnPct ? `Nearly full: above your ${settings.value.diskWarnPct}% warning level` : null;
</script>

<article class="panel node" class:stale>
  <header>
    <span
      class="dot {online ? 'ok' : node.status === 'offline' ? 'bad' : 'idle'}"
      role="img"
      aria-label={online ? "Online" : node.status === "offline" ? "Offline" : "State not known"}
    ></span>
    <h3 class="ellipsis" title={node.name}>{node.name}</h3>
    <span class="muted small sub">
      {#if !online}<span class="down">{node.status === "offline" ? "offline" : "state not known"}</span>{/if}
      {#if node.uptimeSecs != null}<span>up {uptime(node.uptimeSecs)}</span>{/if}
    </span>
  </header>

  <div class="meter-grid">
    <Meter label="CPU" value={online ? node.cpuPct : null} text={online ? undefined : "–"} detail={node.cpuCount ? `${node.cpuCount} cores` : ""} />
    <Meter
      label="RAM"
      value={ram}
      text={ram == null ? "–" : undefined}
      detail={node.memUsed != null && node.memTotal != null ? usedOfTotal(node.memUsed, node.memTotal) : ""}
      warn={ram != null && ram > settings.value.ramWarnPct ? `Above your ${settings.value.ramWarnPct}% warning level` : null}
    />
    <Meter
      label="System disk"
      value={disk}
      text={disk == null ? "–" : undefined}
      detail={node.diskUsed != null && node.diskTotal != null ? usedOfTotal(node.diskUsed, node.diskTotal) : ""}
      warn={diskWarn(disk)}
    />
  </div>

  {#if node.storages.length > 0}
    <h4 class="muted small">Storage</h4>
    <div class="meter-grid">
      {#each node.storages as s (s.name)}
        {@const used = share(s.used, s.total)}
        <Meter
          label={s.name}
          hint={storageHint(s)}
          mono
          value={s.active ? used : null}
          text={s.active && used != null ? undefined : "–"}
          detail={!s.active ? "not available" : s.used != null && s.total != null ? usedOfTotal(s.used, s.total) : ""}
          warn={s.active ? diskWarn(used) : null}
        />
      {/each}
    </div>
  {/if}
</article>

<style>
  .node { display: flex; flex-direction: column; gap: 0.7rem; padding: 0.9rem 1rem 1rem; min-width: 0; }
  header { display: flex; align-items: center; gap: 0.6rem; min-width: 0; }
  h3 { margin: 0; font-size: 1.08rem; font-weight: 600; }
  .sub { display: flex; gap: 0.8rem; margin-left: auto; white-space: nowrap; }
  .down { color: var(--bad); font-weight: 600; }
  h4 { margin: 0.2rem 0 -0.2rem; font-weight: 500; text-transform: uppercase; letter-spacing: 0.06em; }
  /* A reading from before the latest try failed: still shown, but clearly old. */
  .stale .meter-grid { opacity: 0.6; }
</style>
