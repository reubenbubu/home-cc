<script lang="ts">
  // Docker (spec 5.5): the compose stacks and containers on each Docker host, with their logs, files and a
  // cleanup. On a wide screen the log (or files, or cleanup) sits beside the stacks; on a narrower one above them.
  import { onDestroy, onMount, untrack } from "svelte";
  import { clock } from "$lib/clock.svelte";
  import { devices } from "$lib/devices.svelte";
  import { drafts } from "$lib/drafts.svelte";
  import type { Container, ImageUpdate, Stack } from "$lib/docker/api";
  import {
    draftsDroppedWith,
    hostUpdates,
    isLoginProblem,
    stackOrder,
    uncheckedText,
    updatesByKey,
    updatesDraftKey,
    updatesText,
  } from "$lib/docker/logic";
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
  import UpdatesPanel from "$lib/components/docker/UpdatesPanel.svelte";

  type Panel =
    | { kind: "logs"; hostId: number; container: Container }
    | { kind: "files"; hostId: number; stack: Stack }
    | { kind: "cleanup"; hostId: number }
    | { kind: "updates"; hostId: number; stack: string | null }
    | { kind: "hosts" };

  // The log, files, cleanup or hosts panel that was open is open again after another screen was shown.
  let panel = $state<Panel | null>(drafts.get<Panel>("docker-panel") ?? null);
  $effect(() => drafts.remember("docker-panel", panel));

  /** Every change of the side panel comes through here, so a panel put away lets its draft go. An update panel
   *  put away (by its Close, the toolbar, its Clean up link or another host's tab) lets its last result go too,
   *  and one opened afresh starts without one (an update that finished while it was put away included), so an
   *  old "Updated …" never shows as if it had just happened. */
  function show(next: Panel | null) {
    const dropped = draftsDroppedWith(panel, next);
    if (panel?.kind === "updates" && dropped.includes(updatesDraftKey(panel.hostId))) docker.updateResults.delete(panel.hostId);
    if (next?.kind === "updates" && !(panel?.kind === "updates" && panel.hostId === next.hostId) && !docker.updating.has(next.hostId)) {
      docker.updateResults.delete(next.hostId);
    }
    drafts.clear(...dropped);
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

  // The side panel follows the host tab. Cleanup and updates are about a whole host, so they move to the new
  // one (updates as the whole host's, a stack being of the old host); a log or a compose file belongs to the
  // host it came from, so it closes (a log stops following; unsaved compose edits stay kept for when its Files
  // is opened again). The hosts list is about every host and stays. The old host's cleanup or update ticks
  // go, as when the panel is put away.
  $effect(() => {
    const id = docker.selected;
    untrack(() => {
      if (!panel || panel.kind === "hosts" || panel.hostId === id) return;
      if (id != null && panel.kind === "cleanup") show({ kind: "cleanup", hostId: id });
      else if (id != null && panel.kind === "updates") show({ kind: "updates", hostId: id, stack: null });
      else show(null);
    });
  });

  /** Each host's last update check, by updateKey. */
  const answers = $derived(host ? updatesByKey(docker.updates.get(host.deviceId)) : new Map<string, ImageUpdate>());
  const updates = $derived(overview ? hostUpdates(overview, answers) : null);
  const check = $derived(host ? docker.updates.get(host.deviceId) : undefined);
  const checking = $derived(host ? docker.checking.has(host.deviceId) : false);
  const checkError = $derived(host ? docker.updateErrors.get(host.deviceId) : undefined);

  /** How many containers on a host have a newer image, for its tab. */
  function tabUpdates(id: number): number {
    const o = docker.overviews.get(id);
    return o ? hostUpdates(o, updatesByKey(docker.updates.get(id))).count : 0;
  }

  function toggleUpdates(stack: string | null) {
    if (!host) return;
    const open = panel?.kind === "updates" && panel.hostId === host.deviceId && panel.stack === stack;
    show(open ? null : { kind: "updates", hostId: host.deviceId, stack });
  }

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
          {#if tabUpdates(h.deviceId)}
            {@const n = tabUpdates(h.deviceId)}
            <span class="badge" title="{n} {n === 1 ? 'container has' : 'containers have'} a newer image">{n}<span class="sr-only">{n === 1 ? " update" : " updates"}</span></span>
          {/if}
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

        {#if overview && !host.problem}
          <!-- Image updates: what the last check found, when it ran, and the buttons to check again and to update. -->
          <div class="update-line small">
            {#if checking}
              <span class="checking" role="status"><span class="spinner" aria-hidden="true"></span>Checking for newer images…</span>
            {:else if updates && check}
              {#if updates.count}
                <strong class="found">{updatesText(updates)}</strong>
              {:else}
                <!-- "Up to date" only when every image has an answer and none failed. -->
                <span>{updates.failed.length || !updates.complete ? "No updates found" : "All images up to date"}</span>
              {/if}
              {#if !updates.complete}
                <span class="muted" title={updates.unchecked.join("\n")}>{uncheckedText(updates.unchecked.length)}</span>
              {/if}
              {#if updates.failed.length}
                <button class="linkish" onclick={() => toggleUpdates(null)}>{updates.failed.length === 1 ? "1 image" : `${updates.failed.length} images`} couldn't be checked</button>
              {/if}
            {:else}
              <span class="muted">Not checked for updates yet</span>
            {/if}
            {#if check?.checkedAt && !checking}<span class="muted">checked {relTime(check.checkedAt, clock.now)}</span>{/if}
            <span class="buttons">
              <button class="btn small" disabled={checking || docker.updating.has(host.deviceId)} onclick={() => host && docker.checkUpdates(host.deviceId, false)}>
                Check for updates
              </button>
              {#if updates?.count}
                <button
                  class="btn small primary"
                  class:on={panel?.kind === "updates" && panel.stack == null}
                  aria-pressed={panel?.kind === "updates" && panel.stack == null}
                  onclick={() => toggleUpdates(null)}
                >
                  {updates.stacks.length ? "Update all…" : "Pull new images…"}
                </button>
              {/if}
            </span>
          </div>
          {#if checkError}
            <Problem
              tone="warn"
              problem={dockerProblem(device, `Checking the images on "${device?.name}" for updates`, checkError, host)}
              message="Couldn't check for updates: {checkError}"
            />
          {/if}
        {/if}

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
            <!-- Keyed by host too: each card's list keeps its own sort and filter (TableView). -->
            {#each stackOrder(overview.stacks) as s (`${host.deviceId}:${s.name}`)}
              <StackCard
                {host}
                {device}
                stack={s}
                {stats}
                now={clock.now}
                logsFor={panel?.kind === "logs" ? panel.container.id : null}
                filesFor={panel?.kind === "files" && panel.hostId === host.deviceId ? panel.stack.name : null}
                updatesFor={panel?.kind === "updates" && panel.stack === s.name}
                {answers}
                onlogs={toggleLogs}
                onfiles={toggleFiles}
                onupdates={(st) => toggleUpdates(st?.name ?? null)}
              />
            {/each}
            {#if overview.others.length}
              {#key host.deviceId}
              <StackCard
                {host}
                {device}
                others={overview.others}
                {stats}
                now={clock.now}
                logsFor={panel?.kind === "logs" ? panel.container.id : null}
                filesFor={null}
                updatesFor={panel?.kind === "updates" && panel.stack == null}
                {answers}
                onlogs={toggleLogs}
                onfiles={toggleFiles}
                onupdates={() => toggleUpdates(null)}
              />
              {/key}
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
        {:else if panelHost && panel.kind === "updates"}
          {#key panel.hostId}
            <UpdatesPanel
              host={panelHost}
              device={panelDevice}
              stack={panel.stack}
              onclose={() => show(null)}
              oncleanup={() => panelHost && show({ kind: "cleanup", hostId: panelHost.deviceId })}
            />
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
  /* A filled button that is pressed keeps its colours and gets a ring. */
  .btn.primary.on { color: var(--accent-ink); outline: 2px solid var(--accent); outline-offset: 2px; }
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
  .badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 1.35rem;
    height: 1.35rem;
    padding: 0 0.35rem;
    border-radius: 999px;
    background: var(--accent);
    color: var(--accent-ink);
    font-size: 0.75rem;
    font-weight: 700;
  }

  .update-line { display: flex; flex-wrap: wrap; align-items: center; gap: 0.35rem 1rem; min-width: 0; }
  .update-line .found { color: var(--accent); }
  .update-line .buttons { display: inline-flex; flex-wrap: wrap; gap: 0.4rem; }
  .update-line .checking { display: inline-flex; align-items: center; gap: 0.5rem; color: var(--accent); }
  .linkish { border: none; background: none; padding: 0; color: var(--warn); cursor: pointer; font: inherit; text-decoration: underline; }
  .spinner {
    display: inline-block;
    width: 0.85rem;
    height: 0.85rem;
    border: 2px solid color-mix(in srgb, var(--accent) 30%, transparent);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.9s linear infinite;
  }
  @media (prefers-reduced-motion: reduce) { .spinner { animation: none; } }

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
