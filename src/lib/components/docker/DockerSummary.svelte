<script lang="ts">
  // The Overview screen's Containers panel: one line per Docker host (how many containers run, in how many
  // stacks), the containers that need a look, and why a host couldn't be read. Each host opens on the Docker
  // screen. Reads every host while the Overview is open (docker.watchAll), so it fills in without the Docker
  // screen having been opened first.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { devices } from "$lib/devices.svelte";
  import { docker } from "$lib/docker/store.svelte";
  import { summarise } from "$lib/docker/logic";
  import { relTime } from "$lib/format";
  import { clock } from "$lib/clock.svelte";
  import { icons } from "$lib/icons";
  import Icon from "../Icon.svelte";
  import StatusDot from "../StatusDot.svelte";

  onMount(() => docker.watchAll());

  /** Hosts with a Docker tab, by name. */
  const hosts = $derived(
    docker.shown
      .map((h) => ({ host: h, name: devices.byId.get(h.deviceId)?.name ?? `Device ${h.deviceId}` }))
      .sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true })),
  );
  const looking = $derived(docker.probing.size > 0);

  function open(id: number) {
    docker.selected = id;
    void goto("/docker");
  }
</script>

{#if !docker.loaded}
  <p class="muted">Looking for Docker…</p>
{:else if docker.error}
  <p class="muted">Couldn't read the Docker hosts: {docker.error} <a href="/docker">Docker screen</a></p>
{:else if hosts.length === 0}
  <p class="muted">
    {looking ? "Looking for Docker on your Linux machines…" : "No Docker hosts found yet."}
    home-cc looks for Docker on every Linux machine that has an SSH user. <a href="/docker">Docker hosts</a>
  </p>
{:else}
  <ul class="hosts">
    {#each hosts as { host, name } (host.deviceId)}
      {@const o = docker.overviews.get(host.deviceId)}
      {@const s = o ? summarise(o) : null}
      {@const problem = host.problem ?? null}
      <li>
        <button class="host" onclick={() => open(host.deviceId)} title="Open {name} on the Docker screen">
          <StatusDot health={problem ? "down" : s ? (s.attention.length ? "unknown" : "up") : "unknown"} />
          <span class="name ellipsis">{name}</span>
          {#if s}
            <span class="count mono">{s.running} <span class="of">of {s.containers}</span></span>
          {/if}
        </button>
        {#if s}
          <div class="sub muted small">
            {s.running === 1 ? "1 running" : `${s.running} running`}{s.stacks ? ` · ${s.stacks === 1 ? "1 stack" : `${s.stacks} stacks`}` : ""}
            · read {relTime(o?.readAt, clock.now)}
          </div>
        {:else if !problem}
          <div class="sub muted small">{docker.reading.has(host.deviceId) ? "Reading…" : "Not read yet"}</div>
        {/if}
        {#if problem}
          <p class="problem small"><Icon path={icons.warn} size={13} /><span>{problem.message}</span></p>
        {/if}
        {#if s && s.attention.length}
          <ul class="attention small">
            {#each s.attention.slice(0, 5) as a (a.name)}
              <li><Icon path={icons.warn} size={12} /><span class="mono ellipsis">{a.name}</span><span class="muted">{a.why}</span></li>
            {/each}
            {#if s.attention.length > 5}<li class="muted">and {s.attention.length - 5} more</li>{/if}
          </ul>
        {/if}
      </li>
    {/each}
  </ul>
{/if}

<style>
  .hosts { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.75rem; }
  .host {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    width: 100%;
    min-width: 0;
    padding: 0;
    border: none;
    background: none;
    color: var(--text);
    font-weight: 600;
    text-align: left;
    cursor: pointer;
  }
  .host:hover .name { text-decoration: underline; }
  .host:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 3px; }
  .name { min-width: 0; flex: 1; }
  .count { flex: none; font-weight: 500; }
  .of { color: var(--muted); }
  .sub { margin: 0.15rem 0 0 1.35rem; }
  .problem { display: flex; gap: 0.35rem; margin: 0.25rem 0 0 1.35rem; color: var(--text-soft); overflow-wrap: anywhere; }
  .problem :global(svg) { flex: none; margin-top: 0.15rem; color: var(--warn); }
  .attention { list-style: none; margin: 0.3rem 0 0 1.35rem; padding: 0; display: flex; flex-direction: column; gap: 0.2rem; }
  .attention li { display: flex; align-items: baseline; gap: 0.4rem; min-width: 0; }
  .attention :global(svg) { flex: none; color: var(--warn); align-self: center; }
  .attention .mono { min-width: 0; flex: 0 1 auto; }
  .attention .muted { flex: none; }
</style>
