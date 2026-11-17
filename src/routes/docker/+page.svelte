<script lang="ts">
  // Docker (spec 5.5): the compose stacks and containers on each Docker host, with their logs, files and a
  // cleanup. On a wide screen the log (or files, or cleanup) sits beside the stacks; on a narrower one above them.
  import { onDestroy, onMount, untrack } from "svelte";
  import { clock } from "$lib/clock.svelte";
  import { devices } from "$lib/devices.svelte";
  import { drafts } from "$lib/drafts.svelte";
  import type { Container, Stack } from "$lib/docker/api";
  import { draftsDroppedWith, isLoginProblem, stackOrder } from "$lib/docker/logic";
  import { dockerProblem } from "$lib/docker/problem";
  import { docker, REFRESH_SECS } from "$lib/docker/store.svelte";
  import { count, relTime } from "$lib/format";
  import { icons } from "$lib/icons";
  import Address from "$lib/components/Address.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import Problem from "$lib/components/Problem.svelte";
  import CleanupPanel from "$lib/components/docker/CleanupPanel.svelte";
  import FilesPanel from "$lib/components/docker/FilesPanel.svelte";
  import HostsPanel from "$lib/components/docker/HostsPanel.svelte";
  import LogsPanel from "$lib/components/docker/LogsPanel.svelte";
  import StackCard from "$lib/components/docker/StackCard.svelte";

  type Panel =
    | { kind: "logs"; hostId: number; container: Container }
    | { kind: "files"; hostId: number; stack: Stack }
    | { kind: "cleanup"; hostId: number }
    | { kind: "hosts" };

  // The log, files, cleanup or hosts panel that was open is open again after another screen was shown.
  let panel = $state<Panel | null>(drafts.get<Panel>("docker-panel") ?? null);
  $effect(() => drafts.remember("docker-panel", panel));

  /** Every change of the side panel comes through here, so a panel put away lets its draft go. */
  function show(next: Panel | null) {
    drafts.clear(...draftsDroppedWith(panel, next));
    panel = next;
  }

  const host = $derived(docker.selected != null ? docker.byId.get(docker.selected) : undefined);
  const device = $derived(host ? devices.byId.get(host.deviceId) : undefined);
  const overview = $derived(host ? docker.overviews.get(host.deviceId) : undefined);
  const stats = $derived(host ? docker.stats.get(host.deviceId) : undefined);
  const reading = $derived(host ? docker.reading.has(host.deviceId) : false);
  const readError = $derived(host ? docker.readErrors.get(host.deviceId) : undefined);
  const statsError = $derived(host ? docker.statsErrors.get(host.deviceId) : undefined);
  const probingCount = $derived(docker.probing.size);
  const all = $derived(overview ? [...overview.stacks.flatMap((s) => s.containers), ...overview.others] : []);
  const running = $derived(all.filter((c) => c.state === "running").length);

  const panelHost = $derived(panel && panel.kind !== "hosts" ? docker.byId.get(panel.hostId) : undefined);
  const panelDevice = $derived(panelHost ? devices.byId.get(panelHost.deviceId) : undefined);
  /** The open log's container as last read, so its state stays current. */
  const logsContainer = $derived.by(() => {
    if (panel?.kind !== "logs") return null;
    const id = panel.container.id;
    const o = docker.overviews.get(panel.hostId);
    const fresh = o && [...o.stacks.flatMap((s) => s.containers), ...o.others].find((c) => c.id === id);
    return fresh ?? panel.container;
  });

  // The side panel follows the host tab. Cleanup is about a whole host, so it moves to the new one and
  // reads its preview afresh; a log or a compose file belongs to the host it came from, so it closes (a log
  // stops following; unsaved compose edits stay kept for when its Files is opened again). The hosts list is
  // about every host and stays. The old host's cleanup ticks go, as when the cleanup is put away.
  $effect(() => {
    const id = docker.selected;
    untrack(() => {
      if (!panel || panel.kind === "hosts" || panel.hostId === id) return;
      show(panel.kind === "cleanup" && id != null ? { kind: "cleanup", hostId: id } : null);
    });
  });

  function hostName(id: number): string {
    return devices.byId.get(id)?.name ?? `Device ${id}`;
  }

  function hostTone(id: number): string {
    const h = docker.byId.get(id);
    if (!h) return "idle";
    if (h.problem) return isLoginProblem(h.problem.kind) || h.problem.kind === "unreachable" ? "bad" : "warn";
    return docker.overviews.has(id) ? "ok" : "idle";
  }

  function toggleLogs(c: Container) {
    if (!host) return;
    show(panel?.kind === "logs" && panel.container.id === c.id ? null : { kind: "logs", hostId: host.deviceId, container: c });
  }

  function toggleFiles(s: Stack) {
    if (!host) return;
    show(panel?.kind === "files" && panel.stack.name === s.name && panel.hostId === host.deviceId ? null : { kind: "files", hostId: host.deviceId, stack: s });
  }

  let stopFollowing: (() => void) | null = null;
  onMount(async () => {
    await docker.loadHosts();
    stopFollowing = docker.follow();
    await docker.probeDue();
    // A host found just now: show it without waiting for the timer.
    if (docker.selected != null && !docker.overviews.has(docker.selected)) docker.select(docker.selected);
  });
  onDestroy(() => stopFollowing?.());
</script>

<section class="docker">
  <div class="page-head">
    <h1>Docker</h1>
    <div class="actions">
      {#if host}
        <button class="btn" disabled={reading} onclick={() => host && docker.refresh(host.deviceId)}>
          <span class="icon" class:spin={reading}><Icon path={icons.refresh} size={16} /></span> Refresh
        </button>
        <button
          class="btn"
          class:on={panel?.kind === "cleanup"}
          onclick={() => show(panel?.kind === "cleanup" && panel.hostId === host?.deviceId ? null : host ? { kind: "cleanup", hostId: host.deviceId } : null)}
        >
          Clean up…
        </button>
      {/if}
      <button class="btn" class:on={panel?.kind === "hosts"} onclick={() => show(panel?.kind === "hosts" ? null : { kind: "hosts" })}>
        Docker hosts…
      </button>
    </div>
  </div>

  {#if docker.error}
    <Problem problem={dockerProblem(undefined, "Loading the Docker hosts", docker.error)} message="Couldn't load the Docker hosts: {docker.error}" />
  {/if}

  {#if docker.shown.length > 0}
    <div class="hosts" role="tablist" aria-label="Docker hosts">
      {#each docker.shown as h (h.deviceId)}
        <button
          role="tab"
          class="host-tab"
          class:active={h.deviceId === docker.selected}
          aria-selected={h.deviceId === docker.selected}
          onclick={() => docker.select(h.deviceId)}
        >
          <span class="dot {hostTone(h.deviceId)}" aria-hidden="true"></span>
          {hostName(h.deviceId)}
        </button>
      {/each}
      {#if probingCount}<span class="muted small probing" role="status">Looking for Docker on {count(probingCount, "machine")}…</span>{/if}
    </div>
  {/if}

  <div class="layout" class:with-side={!!panel}>
    <div class="main">
      {#if !docker.loaded}
        <p class="muted">Loading…</p>
      {:else if docker.shown.length === 0}
        <div class="panel empty">
          {#if probingCount}
            <p>Looking for Docker on {count(probingCount, "machine")}…</p>
          {:else}
            <h2>No Docker hosts yet</h2>
            <p class="muted">
              home-cc looks for Docker on every Linux machine that has an SSH user. None of them has Docker, or they couldn't
              be reached. Open <strong>Docker hosts</strong> to see each one, or to mark a machine as a Docker host yourself.
            </p>
            <div><button class="btn primary" onclick={() => show({ kind: "hosts" })}>Docker hosts…</button></div>
          {/if}
        </div>
      {:else if host}
        <p class="summary muted small">
          {#if device}<Address host={device.host} known={device.hostname} />{/if}
          {#if host.version}<span>Docker {host.version}</span>{/if}
          {#if overview}
            <span>{count(overview.stacks.length, "stack")}, {count(all.length, "container")} ({running} running)</span>
            <span>Updated {relTime(overview.readAt, clock.now)}{reading ? ", reading…" : ""}</span>
          {:else if reading}
            <span>Reading…</span>
          {/if}
          <span title="The list is read again every {REFRESH_SECS} seconds while this screen is open">refreshes every {REFRESH_SECS} s</span>
        </p>

        {#if host.problem}
          <Problem
            tone={host.problem.kind === "unreachable" ? undefined : "warn"}
            problem={dockerProblem(device, `Reading Docker on "${device?.name}"`, host.problem.message, host)}
          >
            <p class="message">{host.problem.message}</p>
            <div class="fix">
              {#if isLoginProblem(host.problem.kind)}<a class="btn small link-btn" href="/machines">Open Machines</a>{/if}
              <button class="btn small" disabled={reading} onclick={() => host && docker.read(host.deviceId, false)}>Try again</button>
            </div>
            {#if overview}<p class="muted small">The list below is from the last time it worked.</p>{/if}
          </Problem>
        {/if}
        {#if readError}
          <Problem problem={dockerProblem(device, `Reading Docker on "${device?.name}"`, readError, host)} />
        {/if}
        {#if statsError && !host.problem}
          <Problem tone="warn" problem={dockerProblem(device, `Reading CPU and memory of the containers on "${device?.name}"`, statsError, host)} message="CPU and memory couldn't be read: {statsError}" />
        {/if}
        {#if host.compose === "none" && host.found}
          <p class="alert warn small">
            Docker Compose isn't installed on {device?.name}, so stacks can only be seen here, not started or stopped as a whole.
            Install the docker-compose-plugin package there, then press Look again under Docker hosts.
          </p>
        {/if}

        {#if overview}
          <div class="stacks" class:stale={!!host.problem}>
            {#each stackOrder(overview.stacks) as s (s.name)}
              <StackCard
                {host}
                {device}
                stack={s}
                {stats}
                now={clock.now}
                logsFor={panel?.kind === "logs" ? panel.container.id : null}
                filesFor={panel?.kind === "files" && panel.hostId === host.deviceId ? panel.stack.name : null}
                onlogs={toggleLogs}
                onfiles={toggleFiles}
              />
            {/each}
            {#if overview.others.length}
              <StackCard
                {host}
                {device}
                others={overview.others}
                {stats}
                now={clock.now}
                logsFor={panel?.kind === "logs" ? panel.container.id : null}
                filesFor={null}
                onlogs={toggleLogs}
                onfiles={toggleFiles}
              />
            {/if}
            {#if overview.stacks.length === 0 && overview.others.length === 0}
              <p class="panel muted">No containers on {device?.name} yet.</p>
            {/if}
          </div>
        {:else if !host.problem && !readError}
          <p class="muted">Reading the containers on {device?.name}…</p>
        {/if}
      {/if}
    </div>

    {#if panel}
      <aside class="side">
        {#if panel.kind === "hosts"}
          <HostsPanel onclose={() => show(null)} />
        {:else if panelHost && panel.kind === "logs" && logsContainer}
          {#key `${panel.hostId}:${panel.container.id}`}
            <LogsPanel host={panelHost} device={panelDevice} container={logsContainer} onclose={() => show(null)} />
          {/key}
        {:else if panelHost && panel.kind === "files"}
          {#key `${panel.hostId}:${panel.stack.name}`}
            <FilesPanel host={panelHost} device={panelDevice} stack={panel.stack} onclose={() => show(null)} />
          {/key}
        {:else if panelHost && panel.kind === "cleanup"}
          {#key panel.hostId}
            <CleanupPanel host={panelHost} device={panelDevice} onclose={() => show(null)} />
          {/key}
        {/if}
      </aside>
    {/if}
  </div>
</section>

<style>
  .docker { display: flex; flex-direction: column; gap: var(--gap); min-width: 0; }
  .page-head { margin-bottom: 0; }
  .actions .btn { display: inline-flex; align-items: center; gap: 0.45rem; }
  .btn.on { border-color: var(--accent); color: var(--accent); }
  .icon { display: inline-flex; }

  .hosts { display: flex; flex-wrap: wrap; align-items: center; gap: 0.4rem; }
  .host-tab {
    display: inline-flex;
    align-items: center;
    gap: 0.55rem;
    min-height: 2.5rem;
    padding: 0 1rem;
    border: 1px solid var(--border-strong);
    border-radius: 999px;
    background: var(--panel);
    color: var(--text-soft);
    cursor: pointer;
    font-weight: 500;
  }
  .host-tab:hover { background: var(--panel-raised); }
  .host-tab.active { border-color: var(--accent); color: var(--text); background: var(--panel-raised); }
  .host-tab:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .probing { margin-left: 0.5rem; }

  .layout { display: grid; grid-template-columns: minmax(0, 1fr); gap: var(--gap); align-items: start; }
  .main { display: flex; flex-direction: column; gap: var(--gap); min-width: 0; }
  .side { container: side / inline-size; min-width: 0; }
  /* Narrower windows: the open panel goes first, above the stacks. */
  .side { order: -1; }

  .summary { display: flex; flex-wrap: wrap; gap: 0.2rem 1.2rem; margin: 0; }
  .stacks {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 50rem), 1fr));
    gap: var(--gap);
    align-items: start;
  }
  .stale { opacity: 0.65; }
  .empty { display: flex; flex-direction: column; gap: 0.6rem; max-width: 48rem; }
  .empty h2 { margin: 0; font-size: 1.15rem; }
  .empty p { margin: 0; line-height: 1.5; }
  .message { white-space: pre-line; }
  .fix { display: flex; gap: 0.4rem; flex-wrap: wrap; margin-top: 0.5rem; }
  p.alert { margin: 0; }

  /* Wide windows: the panel beside the stacks, staying in view while the list scrolls. */
  @media (min-width: 1600px) {
    .layout.with-side { grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); }
    .side { order: 0; position: sticky; top: 0; }
  }
  @media (min-width: 2400px) {
    .layout.with-side { grid-template-columns: minmax(0, 1.5fr) minmax(0, 1fr); }
  }

  .spin :global(svg) { animation: spin 0.9s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .spin :global(svg) { animation: none; } }
</style>
