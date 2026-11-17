<script lang="ts">
  // Which machines are Docker hosts. home-cc looks for Docker on every Linux machine with an SSH user; the
  // owner can also say a machine is one (always shown) or isn't (never looked at again).
  import { devices, isMachine } from "$lib/devices.svelte";
  import type { HostMode } from "$lib/docker/api";
  import { docker } from "$lib/docker/store.svelte";
  import { dockerIcons } from "$lib/docker/icons";
  import { dockerProblem } from "$lib/docker/problem";
  import { clock } from "$lib/clock.svelte";
  import { count, relTime } from "$lib/format";
  import Icon from "../Icon.svelte";
  import Problem from "../Problem.svelte";

  let { onclose }: { onclose: () => void } = $props();

  const MODES: { value: HostMode; label: string }[] = [
    { value: "auto", label: "Show if Docker is found" },
    { value: "on", label: "Always show" },
    { value: "off", label: "Never show" },
  ];

  const rows = $derived(
    docker.hosts
      .map((h) => ({ h, d: devices.byId.get(h.deviceId) }))
      .sort((a, b) => (a.d?.name ?? "").localeCompare(b.d?.name ?? "", undefined, { numeric: true })),
  );
  /** Machines that can't be Docker hosts here, so the owner knows why they are missing. */
  const windows = $derived(devices.list.filter((d) => d.kind !== "ignore" && d.os === "windows" && d.sshUser).length);
  const noUser = $derived(devices.list.filter((d) => isMachine(d) && d.os !== "windows" && !d.sshUser?.trim()).length);
  const missing = $derived(
    [
      windows ? `${count(windows, "Windows machine")} (Docker is managed on Linux machines only)` : "",
      noUser ? `${count(noUser, "machine")} without an SSH user` : "",
    ]
      .filter(Boolean)
      .join(", and "),
  );
</script>

<section class="panel hosts" aria-label="Docker hosts">
  <header>
    <h2>Docker hosts</h2>
    <button class="icon-btn" aria-label="Close Docker hosts" title="Close" onclick={onclose}><Icon path={dockerIcons.close} size={16} /></button>
  </header>
  <p class="muted small">
    home-cc looks for Docker on each Linux machine that has an SSH user, over the same login as its stats. A machine
    without Docker is looked at again after a day.
  </p>

  <ul>
    {#each rows as { h, d } (h.deviceId)}
      {@const probing = docker.probing.has(h.deviceId)}
      {@const readError = docker.readErrors.get(h.deviceId)}
      <li>
        <div class="who">
          <strong class="ellipsis">{d?.name ?? `Device ${h.deviceId}`}</strong>
          <span class="mono small muted">{d?.host}</span>
        </div>
        <div class="what small">
          {#if probing}
            Looking for Docker…
          {:else if h.found === true}
            <span class="ok">Docker {h.version ?? ""}</span>{#if h.compose === "none"}<span class="muted">, no Docker Compose</span>{/if}
          {:else if h.found === false}
            <span class="muted">No Docker found{h.checkedAt ? `, ${relTime(h.checkedAt, clock.now)}` : ""}</span>
          {:else}
            <span class="muted">Not looked at yet</span>
          {/if}
        </div>
        <div class="acts">
          <select aria-label="Show {d?.name} on the Docker screen" value={h.mode} onchange={(e) => docker.setMode(h.deviceId, e.currentTarget.value as HostMode)}>
            {#each MODES as m}<option value={m.value}>{m.label}</option>{/each}
          </select>
          <button class="btn small" disabled={probing || h.mode === "off"} onclick={() => docker.probe(h.deviceId)}>Look again</button>
        </div>
        {#if h.problem && h.mode !== "off" && h.problem.kind !== "notInstalled"}
          <Problem class="wide" tone="warn" problem={dockerProblem(d, `Looking for Docker on "${d?.name}"`, h.problem.message, h)} />
        {/if}
        {#if readError}
          <Problem class="wide" problem={dockerProblem(d, `Looking for Docker on "${d?.name}"`, readError, h)} />
        {/if}
      </li>
    {:else}
      <li class="muted">No Linux machine has an SSH user yet. Add one on the Machines screen.</li>
    {/each}
  </ul>
  {#if missing}
    <p class="muted small">Not listed: {missing}. SSH users are set on the <a href="/machines">Machines</a> screen.</p>
  {/if}
</section>

<style>
  .hosts { display: flex; flex-direction: column; gap: 0.7rem; min-width: 0; padding: 0.85rem 1rem 1rem; }
  header { display: flex; align-items: center; justify-content: space-between; gap: 0.75rem; }
  h2 { margin: 0; font-size: 1.15rem; font-weight: 600; }
  p { margin: 0; line-height: 1.5; }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  li {
    display: grid;
    grid-template-columns: minmax(8rem, 1fr) minmax(8rem, 1fr) auto;
    align-items: center;
    gap: 0.4rem 1rem;
    padding: 0.55rem 0;
    border-top: 1px solid var(--border);
  }
  .who { display: flex; flex-direction: column; min-width: 0; }
  .ok { color: var(--ok); }
  .acts { display: flex; gap: 0.4rem; align-items: center; }
  .acts select { width: auto; min-height: 2.25rem; }
  li :global(.wide) { grid-column: 1 / -1; }
  @container side (max-width: 38rem) {
    li { grid-template-columns: minmax(0, 1fr) auto; }
    .acts { grid-column: 1 / -1; flex-wrap: wrap; }
  }
</style>
