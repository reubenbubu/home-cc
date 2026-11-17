<script lang="ts">
  import { KIND_LABELS, OS_LABELS, type Device, type MachineStats, type StatsErrorKind } from "$lib/api";
  import { clock } from "$lib/clock.svelte";
  import { devices, isMachine } from "$lib/devices.svelte";
  import { ms, pct, relTime, share } from "$lib/format";
  import { icons } from "$lib/icons";
  import { scan } from "$lib/scan.svelte";
  import { settings } from "$lib/settings.svelte";
  import { stats } from "$lib/stats.svelte";
  import { status } from "$lib/status.svelte";
  import CheckProblem from "$lib/components/CheckProblem.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import MiniMeter from "$lib/components/MiniMeter.svelte";
  import ScanStatus from "$lib/components/ScanStatus.svelte";
  import Sparkline from "$lib/components/Sparkline.svelte";
  import StatusDot from "$lib/components/StatusDot.svelte";

  const byName = (a: Device, b: Device) => a.name.localeCompare(b.name, undefined, { numeric: true });
  const shown = $derived(devices.list.filter((d) => d.kind !== "ignore"));
  const machines = $derived(shown.filter(isMachine).sort(byName));
  const meshNodes = $derived(shown.filter((d) => d.kind === "mesh").sort(byName));
  const others = $derived(shown.filter((d) => d.kind === "other" && d.reviewed).sort(byName));
  const fresh = $derived(shown.filter((d) => !d.reviewed));

  const upOf = (list: Device[]) => list.filter((d) => status.health(d) === "up").length;
  const downOf = (list: Device[]) => list.filter((d) => status.health(d) === "down").length;

  function fullestDisk(st: MachineStats): { mount: string; used: number } | null {
    let best: { mount: string; used: number } | null = null;
    for (const d of st.disks) {
      const used = share(d.usedBytes, d.totalBytes);
      if (used != null && (!best || used > best.used)) best = { mount: d.mount, used };
    }
    return best;
  }

  /** A machine card's one line about a failed reading. The first two wait for the owner on Machines. */
  const NOTE: Record<StatsErrorKind, string> = {
    newHost: "Needs your approval before its password is used",
    hostKeyChanged: "Identity changed: needs your approval",
    auth: "Couldn't log in to read stats",
    unreachable: "Couldn't read stats",
    other: "Couldn't read stats",
  };

  interface Issue {
    key: string;
    text: string;
    href: string;
  }

  /** Everything the owner should look at, most pressing first. */
  const issues = $derived.by(() => {
    const down: Issue[] = [];
    const warn: Issue[] = [];
    for (const d of [...machines, ...meshNodes]) {
      const href = d.kind === "mesh" ? "/network" : "/machines";
      if (status.health(d) === "down") {
        const seen = status.get(d)?.lastSeen ?? d.lastSeen;
        down.push({ key: `down-${d.id}`, text: `${d.name} is down (last seen ${relTime(seen, clock.now)})`, href });
      }
      const entry = stats.get(d.id);
      if (!d.sshUser || !entry) continue;
      if (entry.errorKind === "hostKeyChanged") {
        warn.push({ key: `key-${d.id}`, text: `${d.name}: its identity has changed and needs your approval`, href });
      } else if (entry.errorKind === "newHost") {
        warn.push({ key: `new-${d.id}`, text: `${d.name}: first connection, its identity needs your approval`, href });
      } else if (entry.errorKind === "auth") {
        warn.push({ key: `auth-${d.id}`, text: `${d.name}: couldn't log in to read stats`, href });
      }
      if (!entry.stats || entry.error) continue;
      const ram = share(entry.stats.memUsedKb, entry.stats.memTotalKb);
      if (ram != null && ram > settings.value.ramWarnPct) {
        warn.push({ key: `ram-${d.id}`, text: `${d.name}: RAM is ${pct(ram)} used`, href });
      }
      // Keyed by position: two disks of one machine can report the same mount point.
      for (const [i, disk] of entry.stats.disks.entries()) {
        const used = share(disk.usedBytes, disk.totalBytes);
        if (used != null && used > settings.value.diskWarnPct) {
          warn.push({ key: `disk-${d.id}-${i}`, text: `${d.name}: disk ${disk.mount} is ${pct(used)} full`, href });
        }
      }
    }
    return [...down, ...warn];
  });
</script>

{#snippet response(d: Device)}
  {@const s = status.get(d)}
  {#if s && !s.up}
    <span class="down small">down</span>
  {:else if s?.up && s.rttMs != null}
    <span class="mono muted small">{ms(s.rttMs)}</span>
  {:else if s?.up && status.isSelf(d)}
    <span class="muted small self" title="home-cc runs on this PC">this PC</span>
  {/if}
{/snippet}

<header class="page-head">
  <h1>Overview</h1>
  <div class="actions">
    <button class="btn" onclick={() => scan.start()} disabled={scan.running}>
      {scan.running ? "Scanning…" : "Scan network"}
    </button>
    <a class="btn primary link-btn" href="/machines?add=1">Add machine</a>
  </div>
</header>

<CheckProblem />
<ScanStatus compact />

<section class="stats" aria-label="Summary">
  <div class="panel stat">
    <div class="muted">Up</div>
    <div class="value mono">{upOf(shown)}</div>
    <div class="muted small">of {shown.length} devices</div>
  </div>
  <div class="panel stat">
    <div class="muted">Down</div>
    <div class="value mono" class:alarm={downOf(shown) > 0}>{downOf(shown)}</div>
    <div class="muted small">{downOf(machines) + downOf(meshNodes)} of them machines or mesh nodes</div>
  </div>
  <div class="panel stat">
    <div class="muted">Machines up</div>
    <div class="value mono">{upOf(machines)}<span class="of">of {machines.length}</span></div>
    <div class="muted small">PCs, servers, VMs, containers and appliances</div>
  </div>
  <div class="panel stat">
    <div class="muted">Mesh nodes up</div>
    <div class="value mono">{upOf(meshNodes)}<span class="of">of {meshNodes.length}</span></div>
    <div class="muted small">Deco</div>
  </div>
  <a class="panel stat link" href="/network">
    <div class="muted">New devices</div>
    <div class="value mono">{fresh.length}</div>
    <div class="muted small">{fresh.length > 0 ? "Review them on Network" : "Nothing to review"}</div>
  </a>
</section>

<div class="layout">
  <div class="main">
    <section>
      <h2 class="section-title">Machines</h2>
      {#if devices.loaded && machines.length === 0}
        <div class="panel empty">
          <p>No machines yet. Add your PCs, VMs and Proxmox host, or scan the network and pick them out.</p>
          <a class="btn primary link-btn" href="/machines?add=1">Add your first machine</a>
        </div>
      {:else}
        <div class="auto-grid">
          {#each machines as m (m.id)}
            {@const entry = stats.get(m.id)}
            {@const st = entry?.stats}
            {@const disk = st ? fullestDisk(st) : null}
            {@const ram = share(st?.memUsedKb, st?.memTotalKb)}
            <!-- An appliance's system is mostly not known ("Other"), and then not worth its place. -->
            {@const system = m.kind === "appliance" && m.os === "other" ? [] : [OS_LABELS[m.os]]}
            <article class="panel card">
              <div class="card-top">
                <div class="card-name">
                  <StatusDot health={status.health(m)} />
                  <span class="ellipsis" title={m.name}>{m.name}</span>
                </div>
                {@render response(m)}
              </div>
              <div class="muted small ellipsis"><span class="mono">{m.host}</span> · {[...system, KIND_LABELS[m.kind]].join(" · ")}</div>
              <Sparkline history={status.get(m)?.history ?? []} />
              {#if !m.sshUser && m.kind === "appliance"}
                <!-- Most appliances have no SSH, so there are no stats to ask for: just when it was last there. -->
                <div class="muted small">Last seen {relTime(status.get(m)?.lastSeen ?? m.lastSeen, clock.now)}</div>
              {:else if !m.sshUser}
                <a class="small quiet-link" href="/machines?edit={m.id}">Add an SSH user to see stats</a>
              {:else if entry?.error}
                <a class="small note" href="/machines" title={entry.error}>
                  <Icon path={icons.warn} size={13} />
                  {(entry.errorKind && NOTE[entry.errorKind]) || NOTE.other}
                </a>
              {:else if st}
                <div class="minis">
                  {#if st.cpuPct == null && st.load1 != null}
                    <MiniMeter label="Load" value={null} text={st.load1.toFixed(2)} />
                  {:else}
                    <MiniMeter label="CPU" value={st.cpuPct} />
                  {/if}
                  <MiniMeter
                    label="RAM"
                    value={ram}
                    warn={ram != null && ram > settings.value.ramWarnPct ? `RAM is above your ${settings.value.ramWarnPct}% warning level` : null}
                  />
                  <MiniMeter
                    label="Disk"
                    value={disk?.used ?? null}
                    warn={disk && disk.used > settings.value.diskWarnPct ? `Disk ${disk.mount} is nearly full` : null}
                  />
                </div>
              {:else}
                <div class="muted small">Waiting for the first reading…</div>
              {/if}
            </article>
          {/each}
        </div>
      {/if}
    </section>

    {#if others.length > 0}
      <section>
        <h2 class="section-title">Other devices</h2>
        <div class="chips">
          {#each others as d (d.id)}
            <div class="chip">
              <StatusDot health={status.health(d)} />
              <span class="ellipsis" title={d.name}>{d.name}</span>
              {@render response(d)}
            </div>
          {/each}
        </div>
      </section>
    {/if}
  </div>

  <aside class="side">
    <section class="panel">
      <h2 class="section-title">Needs attention</h2>
      {#if issues.length === 0}
        <p class="fine"><Icon path={icons.check} size={15} /> Nothing right now.</p>
      {:else}
        <ul class="issues">
          {#each issues as issue (issue.key)}
            <li><Icon path={icons.warn} size={14} /><a href={issue.href}>{issue.text}</a></li>
          {/each}
        </ul>
      {/if}
    </section>
    <section class="panel">
      <h2 class="section-title">Deco mesh</h2>
      {#if meshNodes.length === 0}
        <p class="muted">A network scan finds your 4 Deco nodes. You can also add them by hand as "Mesh node".</p>
      {:else}
        <div class="mesh">
          {#each meshNodes as n (n.id)}
            <div class="mesh-node">
              <div class="card-name"><StatusDot health={status.health(n)} /><span class="ellipsis" title={n.name}>{n.name}</span></div>
              <div class="card-top">
                <span class="mono muted small">{n.host}</span>
                {@render response(n)}
              </div>
              <Sparkline history={status.get(n)?.history ?? []} />
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
  .stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(12rem, 1fr)); gap: var(--gap); margin-bottom: 1.4rem; }
  .stat { display: block; color: inherit; text-decoration: none; }
  .stat.link:hover { background: var(--panel-raised); }
  .stat .value { margin: 0.3rem 0 0.15rem; font-size: 1.85rem; }
  .stat .value.alarm { color: var(--bad); }
  .stat .of { margin-left: 0.5rem; color: var(--muted); font-size: 1.1rem; }

  .layout { display: grid; grid-template-columns: minmax(0, 1fr); gap: 1.4rem; }
  .main { display: flex; flex-direction: column; gap: 1.4rem; min-width: 0; }
  .side { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 22rem), 1fr)); gap: var(--gap); align-content: start; }

  /* Ultrawide and other very wide windows: put the side panels next to the machines. */
  @media (min-width: 2400px) {
    .layout { grid-template-columns: minmax(0, 1fr) 34rem; }
    .side { grid-template-columns: minmax(0, 1fr); }
  }

  .card { display: flex; flex-direction: column; gap: 0.55rem; padding: 0.9rem 1rem; min-width: 0; }
  .card-top { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem; min-width: 0; }
  .card-name { display: flex; align-items: center; gap: 0.6rem; font-weight: 600; min-width: 0; }
  .down { color: var(--bad); font-weight: 600; flex: none; }
  .card-top .mono, .self { flex: none; white-space: nowrap; }
  .minis { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 0.9rem; }
  .note { display: flex; align-items: center; gap: 0.35rem; color: var(--text-soft); text-decoration: none; }
  .note :global(svg) { color: var(--warn); flex: none; }
  .note:hover, .quiet-link:hover { text-decoration: underline; }
  .quiet-link { color: var(--muted); text-decoration: none; }

  .chips { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 15rem), 1fr)); gap: 0.5rem; }
  .chip {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    min-width: 0;
    min-height: 2.5rem;
    padding: 0 0.8rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel);
  }
  .chip .ellipsis { flex: 1; }

  .issues { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.5rem; }
  .issues li { display: flex; gap: 0.5rem; line-height: 1.4; }
  .issues :global(svg) { color: var(--warn); flex: none; margin-top: 0.15rem; }
  .issues a { color: var(--text); text-decoration: none; overflow-wrap: anywhere; }
  .issues a:hover { text-decoration: underline; }
  .fine { display: flex; align-items: center; gap: 0.45rem; color: var(--text-soft); }
  .fine :global(svg) { color: var(--ok); }

  .mesh { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 14rem), 1fr)); gap: 0.6rem; }
  .mesh-node { padding: 0.75rem; border: 1px solid var(--track); border-radius: var(--radius-sm); display: flex; flex-direction: column; gap: 0.4rem; min-width: 0; }

  .empty { display: flex; flex-direction: column; align-items: flex-start; gap: 0.75rem; }
  .empty p { margin: 0; color: var(--text-soft); }
  p { margin: 0; line-height: 1.5; }
</style>
