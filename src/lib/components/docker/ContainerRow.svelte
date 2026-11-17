<script lang="ts">
  // One container in a stack: what it is, how it is doing, and its controls. Stopping and removing ask first,
  // in the row itself.
  import type { Container, ContainerStats } from "$lib/docker/api";
  import { docker } from "$lib/docker/store.svelte";
  import { dockerIcons } from "$lib/docker/icons";
  import { devices } from "$lib/devices.svelte";
  import { containerKey, execCommand, isRunning, portShown, portText, publishedPorts, stateText, stateTone } from "$lib/docker/logic";
  import { bytes } from "$lib/format";
  import { icons } from "$lib/icons";
  import { openTerminal } from "$lib/terminals.svelte";
  import Address from "../Address.svelte";
  import Icon from "../Icon.svelte";

  let {
    hostId,
    container: c,
    stats,
    now,
    logsOpen = false,
    onlogs,
  }: {
    hostId: number;
    container: Container;
    stats?: ContainerStats;
    now: number;
    /** True while this container's log is open. */
    logsOpen?: boolean;
    onlogs: (c: Container) => void;
  } = $props();

  let asking = $state<"stop" | "remove" | null>(null);

  const action = $derived(docker.action(containerKey(hostId, c.id)));
  const busy = $derived(!!action?.doing);
  const running = $derived(isRunning(c));
  const tone = $derived(stateTone(c));
  const label = $derived(c.service ?? c.name);
  const ports = $derived(publishedPorts(c));
  const cpu = $derived(stats?.cpuPct != null ? `${stats.cpuPct < 10 ? stats.cpuPct.toFixed(1) : Math.round(stats.cpuPct)}%` : "–");
  const mem = $derived(stats?.memUsedBytes != null ? bytes(stats.memUsedBytes) : "–");
  const memTitle = $derived(
    stats?.memUsedBytes != null && stats.memLimitBytes ? `${bytes(stats.memUsedBytes)} of ${bytes(stats.memLimitBytes)} it may use` : "Memory",
  );

  /** A shell inside the container, in a terminal tab over the host's SSH login. Null for a container whose
   *  name and id aren't ones Docker gives (never expected), so nothing odd is run. */
  const shell = $derived(execCommand(c));

  const shellTitle = $derived(
    !shell
      ? "home-cc can't open a terminal here: Docker gave this container a name it doesn't expect"
      : c.state !== "running"
        ? "A terminal can only be opened in a container while it runs"
        : "Open a terminal inside this container",
  );

  function openShell() {
    if (!shell) return;
    const host = devices.byId.get(hostId)?.name ?? "its host";
    void openTerminal({ deviceId: hostId, title: `${c.name} on ${host}`, command: shell });
  }

  function confirm() {
    const what = asking;
    asking = null;
    if (what) void docker.containerAction(hostId, c, what);
  }
</script>

<div class="row" class:open={logsOpen} role="row">
  <div class="who" role="cell">
    <span class="dot {tone}" role="img" aria-label={stateText(c, now)}></span>
    <div class="names">
      <span class="name ellipsis" title={c.name}>{label}</span>
      <span class="image muted small mono ellipsis" title={c.image}>{c.image}</span>
    </div>
  </div>
  <div class="info">
    <span class="state small" class:bad={tone === "bad"} class:warn={tone === "warn"} role="cell">
      {busy ? `${action?.doing}…` : stateText(c, now)}
    </span>
    <span class="ports mono small" role="cell" title={c.ports.map(portText).join(", ")}>
      {#each ports as p}
        {@const shown = portShown(p)}
        <!-- Published on one address only: that address and its hostname under the port, each on a line. -->
        <span class="port" class:bound={!!shown.ip}>{shown.text}{#if shown.ip}<span class="at"><span class="on">on</span><Address host={shown.ip} stack /></span>{/if}</span>
      {:else}<span class="muted">–</span>{/each}
    </span>
    <span class="num mono small" role="cell" title="CPU, as a share of one core">{running ? cpu : "–"}</span>
    <span class="num mono small" role="cell" title={memTitle}>{running ? mem : "–"}</span>
  </div>
  <div class="acts" role="cell">
    {#if asking}
      <span class="ask small">{asking === "stop" ? `Stop ${label}?` : running ? `Stop and remove ${label}?` : `Remove ${label}?`}</span>
      <button class="btn small danger" onclick={confirm}>{asking === "stop" ? "Yes, stop" : "Yes, remove"}</button>
      <button class="btn small" onclick={() => (asking = null)}>Cancel</button>
    {:else}
      <button class="btn small" class:on={logsOpen} aria-pressed={logsOpen} aria-label="Logs of {c.name}" onclick={() => onlogs(c)}>
        <Icon path={dockerIcons.logs} size={15} /><span class="txt">Logs</span>
      </button>
      {#if running}
        <button
          class="icon-btn"
          title={shellTitle}
          aria-label="Open a terminal in {c.name}"
          disabled={!shell || c.state !== "running"}
          onclick={openShell}
        >
          <Icon path={icons.terminal} size={16} />
        </button>
        <button class="icon-btn" title="Restart" aria-label="Restart {c.name}" disabled={busy} onclick={() => docker.containerAction(hostId, c, "restart")}>
          <Icon path={dockerIcons.restart} size={16} />
        </button>
        <button class="icon-btn" title="Stop" aria-label="Stop {c.name}" disabled={busy} onclick={() => (asking = "stop")}>
          <Icon path={dockerIcons.stop} size={16} />
        </button>
      {:else}
        <!-- Keeps the buttons in the same places as in the rows of running containers. -->
        <span class="slot" aria-hidden="true"></span>
        <span class="slot" aria-hidden="true"></span>
        <button class="icon-btn" title="Start" aria-label="Start {c.name}" disabled={busy} onclick={() => docker.containerAction(hostId, c, "start")}>
          <Icon path={dockerIcons.start} size={16} />
        </button>
      {/if}
      <button class="icon-btn" title="Remove" aria-label="Remove {c.name}" disabled={busy} onclick={() => (asking = "remove")}>
        <Icon path={dockerIcons.remove} size={16} />
      </button>
    {/if}
  </div>
</div>

<style>
  /* Columns are shared with the header row in StackCard.svelte. */
  .row {
    display: grid;
    grid-template-columns: var(--cols);
    align-items: center;
    gap: 0.25rem 0.9rem;
    padding: 0.35rem 0.5rem;
    margin: 0 -0.5rem;
    border-radius: var(--radius-sm);
    min-width: 0;
  }
  .row:hover, .row.open { background: var(--panel-raised); }
  .info { display: contents; }
  .who { display: flex; align-items: center; gap: 0.6rem; min-width: 0; }
  .names { display: flex; flex-direction: column; min-width: 0; }
  .name { font-weight: 500; }
  .image { font-size: 0.8rem; }
  .state { min-width: 0; color: var(--text-soft); }
  .state.bad { color: var(--bad); }
  .state.warn { color: var(--warn); }
  .ports { display: flex; flex-wrap: wrap; gap: 0 0.6rem; min-width: 0; color: var(--text-soft); }
  .port { white-space: nowrap; }
  .port.bound { display: flex; flex-direction: column; min-width: 0; max-width: 100%; }
  .at { display: flex; align-items: baseline; gap: 0.4em; min-width: 0; }
  .on { color: var(--muted); font-family: var(--font); }
  .num { text-align: right; white-space: nowrap; color: var(--text-soft); }
  .acts { display: flex; align-items: center; justify-content: flex-end; gap: 0.15rem; flex-wrap: wrap; }
  .acts .btn { display: inline-flex; align-items: center; gap: 0.35rem; margin-right: 0.2rem; }
  .acts .btn.on { border-color: var(--accent); color: var(--accent); }
  .ask { color: var(--text); margin-right: 0.3rem; }
  .slot { width: 2.25rem; flex: none; }

  /* A narrow card, too narrow for the columns (see StackCard.svelte): name and controls on top, the rest on a
     second line. */
  @container stack (max-width: 49rem) {
    .row { grid-template-columns: minmax(0, 1fr) auto; }
    .info { display: flex; flex-wrap: wrap; gap: 0.15rem 1rem; grid-column: 1 / -1; padding-left: 1.2rem; }
    .num { text-align: left; }
    .num::before { color: var(--muted); }
    .info .num:nth-of-type(3)::before { content: "CPU "; }
    .info .num:nth-of-type(4)::before { content: "RAM "; }
  }
  @container stack (max-width: 30rem) {
    .acts .txt { display: none; }
  }
</style>
