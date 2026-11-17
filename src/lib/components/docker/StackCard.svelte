<script lang="ts">
  // One compose stack: its state, the actions on the whole stack, and its containers. Also used, without a
  // stack, for the containers that aren't in one.
  import type { Device } from "$lib/api";
  import type { Container, ContainerStats, DockerHost, ImageUpdate, Stack } from "$lib/docker/api";
  import { docker } from "$lib/docker/store.svelte";
  import { dockerIcons } from "$lib/docker/icons";
  import {
    containerKey,
    isRunning,
    portText,
    publishedPorts,
    stackKey,
    stackSummary,
    STATE_ORDER,
    stateRank,
    stateWord,
    updateMark,
    updateOf,
  } from "$lib/docker/logic";
  import { names } from "$lib/names.svelte";
  import type { Column } from "$lib/table";
  import { TableView } from "$lib/table.svelte";
  import { untrack } from "svelte";
  import { count } from "$lib/format";
  import { dockerProblem } from "$lib/docker/problem";
  import ConfirmButton from "../ConfirmButton.svelte";
  import Icon from "../Icon.svelte";
  import Problem from "../Problem.svelte";
  import GroupRow from "../table/GroupRow.svelte";
  import SortButton from "../table/SortButton.svelte";
  import TableTools from "../table/TableTools.svelte";
  import ContainerRow from "./ContainerRow.svelte";

  let {
    host,
    device,
    stack,
    others,
    stats,
    now,
    logsFor,
    filesFor,
    updatesFor = false,
    answers,
    onlogs,
    onfiles,
    onupdates,
  }: {
    host: DockerHost;
    device: Device | undefined;
    /** The stack shown. Without one, `others` are shown as "Other containers". */
    stack?: Stack;
    others?: Container[];
    stats: Map<string, ContainerStats> | undefined;
    now: number;
    /** The container whose log is open, if any. */
    logsFor: string | null;
    /** The stack whose files are open, if any. */
    filesFor: string | null;
    /** True while this card's updates are open in the side panel. */
    updatesFor?: boolean;
    /** The host's last update check, by updateKey. */
    answers: Map<string, ImageUpdate>;
    onlogs: (c: Container) => void;
    onfiles: (stack: Stack) => void;
    /** Opens the updates of this stack, or of the containers outside any stack (null). */
    onupdates: (stack: Stack | null) => void;
  } = $props();

  const containers = $derived(stack?.containers ?? others ?? []);
  /** Containers with a newer image in their registry. */
  const updateCount = $derived(containers.filter((c) => updateOf(answers, c)?.state === "available").length);
  const summary = $derived(stack ? stackSummary(stack) : null);
  const key = $derived(stack ? stackKey(host.deviceId, stack.name) : "");
  const action = $derived(stack ? docker.action(key) : undefined);
  const busy = $derived(!!action?.doing);
  const down = $derived(!!stack && stack.containers.length === 0);
  const noCompose = $derived(host.compose === "none");
  /** Container actions that went wrong, shown under the list. */
  const failed = $derived(
    containers
      .map((c) => ({ c, a: docker.action(containerKey(host.deviceId, c.id)) }))
      .filter((x): x is { c: Container; a: NonNullable<typeof x.a> } => !!x.a?.error),
  );

  // Sorting, grouping and filtering (table.ts), each stack's list on its own. Grouped by state only: the
  // names, ports and readings are each a container's own. The filter finds a container by its service,
  // name or image, its state, or a port (with the address and hostname it is published on); not by its
  // CPU or memory, which would find most containers for a typed "1".
  const columns: Column<Container>[] = [
    {
      id: "name",
      label: "Container",
      value: (c) => c.service ?? c.name,
      search: (c) => [c.service, c.name, c.image, updateMark(updateOf(answers, c))?.text],
    },
    {
      id: "state",
      label: "State",
      kind: "rank",
      order: STATE_ORDER,
      value: stateRank,
      text: stateWord,
      search: (c) => [stateWord(c), c.health],
      words: ["trouble first", "stopped first"],
      group: true,
      choose: true,
    },
    {
      id: "ports",
      label: "Ports",
      kind: "number",
      value: (c) => publishedPorts(c)[0]?.hostPort ?? null,
      search: (c) =>
        c.ports.flatMap((p) => (p.hostIp?.trim() ? [portText(p), p.hostIp, names.text(p.hostIp.trim())] : [portText(p)])),
    },
    { id: "cpu", label: "CPU", kind: "number", value: (c) => (isRunning(c) ? stats?.get(c.id)?.cpuPct : null), search: () => null },
    { id: "memory", label: "Memory", kind: "number", value: (c) => (isRunning(c) ? stats?.get(c.id)?.memUsedBytes : null), search: () => null },
  ];

  const view = new TableView<Container>({
    id: untrack(() => `docker-containers:${host.deviceId}:${stack ? `stack:${stack.name}` : "others"}`),
    columns: () => columns,
    rows: () => containers,
    key: (c) => c.id,
    usual: "by service",
  });
  /** The bar above the list, for a stack of more than one container (or while it hides some). */
  const tools = $derived(containers.length > 1 || view.filtering);
</script>

<article class="panel stack" aria-label={stack ? `Stack ${stack.name}` : "Other containers"}>
  <header>
    <div class="title">
      {#if summary}<span class="dot {summary.tone}" aria-hidden="true"></span>{/if}
      <h3 class="ellipsis" title={stack?.name}>{stack?.name ?? "Other containers"}</h3>
      {#if summary}
        <span class="muted small summary">{summary.text}</span>
      {:else}
        <span class="muted small summary">Not part of a compose stack</span>
      {/if}
      {#if updateCount}<span class="updates small">{count(updateCount, "update")}</span>{/if}
    </div>
    {#if !stack && updateCount}
      <div class="acts">
        <button class="btn small" class:on={updatesFor} aria-pressed={updatesFor} onclick={() => onupdates(null)}>Pull new images…</button>
      </div>
    {/if}
    {#if stack}
      <div class="acts">
        {#if busy}
          <span class="doing small" role="status"><span class="spinner" aria-hidden="true"></span>{action?.doing}…</span>
        {:else if !noCompose}
          {#if down}
            <button class="btn small primary" onclick={() => docker.stackAction(host.deviceId, stack.name, "up")}>Start</button>
          {:else}
            {#if updateCount}
              <button
                class="btn small primary"
                class:on={updatesFor}
                aria-pressed={updatesFor}
                title="Pull the newer images and recreate only the containers that use them; you confirm first"
                onclick={() => onupdates(stack)}
              >
                Update…
              </button>
            {/if}
            <button class="btn small" title="Fetch newer images, then recreate what changed" onclick={() => docker.stackAction(host.deviceId, stack.name, "pull")}>
              Pull and restart
            </button>
            <button class="btn small" onclick={() => docker.stackAction(host.deviceId, stack.name, "restart")}>Restart</button>
            {#if summary && summary.running < summary.total}
              <button class="btn small" onclick={() => docker.stackAction(host.deviceId, stack.name, "up")}>Start all</button>
            {/if}
            <ConfirmButton
              label="Take down"
              confirmLabel="Yes, take {stack.name} down"
              onconfirm={() => docker.stackAction(host.deviceId, stack.name, "down")}
            />
          {/if}
        {/if}
        {#if stack.workingDir || stack.configFiles.length}
          <button class="btn small" class:on={filesFor === stack.name} aria-pressed={filesFor === stack.name} onclick={() => onfiles(stack)}>
            <Icon path={dockerIcons.file} size={15} /> Files
          </button>
        {/if}
        {#if down && !busy}
          <ConfirmButton label="Forget" confirmLabel="Yes, forget it" onconfirm={() => docker.forgetStack(host.deviceId, stack.name)} />
        {/if}
      </div>
    {/if}
  </header>
  {#if stack?.workingDir}
    <p class="folder mono small muted ellipsis" title={stack.workingDir}>{stack.workingDir}</p>
  {/if}

  {#if action?.error}
    <Problem
      problem={dockerProblem(device, `Running a stack action on "${stack?.name}" on "${device?.name}"`, action.error, host)}
      message={action.error}
    />
  {:else if action?.output}
    <div class="output">
      <details>
        <summary class="small">What Docker said</summary>
        <pre class="mono small">{action.output}</pre>
      </details>
      <button class="x" aria-label="Hide what Docker said" title="Hide" onclick={() => docker.clearAction(key)}>×</button>
    </div>
  {/if}

  {#if down}
    <p class="muted small down">
      No containers. This stack was seen here before; Start brings it up again from its compose files{noCompose ? " (Docker Compose is needed for that)" : ""}.
    </p>
  {:else}
    {#if tools}<div class="tools"><TableTools {view} label={stack ? `containers of ${stack.name}` : "other containers"} noun="containers" /></div>{/if}
    {#if view.shown > 0}
    <div class="list" role="table" aria-label="Containers" use:view.watch>
      <div class="head muted small" role="row">
        <span role="columnheader" aria-sort={view.ariaSort("name")}><SortButton {view} col="name" /></span>
        <span role="columnheader" aria-sort={view.ariaSort("state")}><SortButton {view} col="state" /></span>
        <span role="columnheader" aria-sort={view.ariaSort("ports")}><SortButton {view} col="ports" /></span>
        <span role="columnheader" class="num" aria-sort={view.ariaSort("cpu")}><SortButton {view} col="cpu" /></span>
        <span role="columnheader" class="num" aria-sort={view.ariaSort("memory")}><SortButton {view} col="memory" /></span>
        <span role="columnheader"><span class="sr-only">Actions</span></span>
      </div>
      {#each view.sections as section (section.key)}
        <div role="rowgroup" class="rows">
          {#if section.label !== null}<GroupRow {view} {section} grid />{/if}
          {#if section.open}
            {#each section.rows as c (c.id)}
              <ContainerRow hostId={host.deviceId} container={c} stats={stats?.get(c.id)} {now} logsOpen={logsFor === c.id} update={updateOf(answers, c)} {onlogs} />
            {/each}
          {/if}
        </div>
      {/each}
    </div>
    {/if}
  {/if}

  {#each failed as { c, a } (c.id)}
    <Problem
      problem={dockerProblem(device, `Acting on the container "${c.name}" on "${device?.name}"`, a.error ?? "", host)}
      message="{c.service ?? c.name}: {a.error}"
    />
  {/each}
</article>

<style>
  .stack {
    container: stack / inline-size;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    padding: 0.85rem 1rem 0.8rem;
    min-width: 0;
    --cols: minmax(8rem, 1.6fr) minmax(7.5rem, 1.1fr) minmax(5rem, 1fr) 4rem 5.8rem 15.1rem;
  }
  header { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem 1rem; flex-wrap: wrap; min-width: 0; }
  .title { display: flex; flex-wrap: wrap; align-items: baseline; gap: 0.1rem 0.6rem; min-width: 0; }
  .title .dot { align-self: center; }
  h3 { margin: 0; font-size: 1.1rem; font-weight: 600; }
  .summary { white-space: nowrap; }
  .updates { white-space: nowrap; color: var(--accent); font-weight: 600; }
  .acts { display: flex; align-items: center; gap: 0.4rem; flex-wrap: wrap; }
  .acts .btn { display: inline-flex; align-items: center; gap: 0.35rem; }
  .acts .btn.on { border-color: var(--accent); color: var(--accent); }
  /* A filled button that is pressed keeps its colours and gets a ring. */
  .acts .btn.primary.on { color: var(--accent-ink); outline: 2px solid var(--accent); outline-offset: 2px; }
  .doing { display: inline-flex; align-items: center; gap: 0.5rem; color: var(--accent); }
  .folder { margin: -0.35rem 0 0; }
  .down { margin: 0; }
  p { margin: 0; }

  .list { display: flex; flex-direction: column; min-width: 0; }
  .head {
    display: grid;
    grid-template-columns: var(--cols);
    gap: 0.9rem;
    padding: 0 0 0.3rem;
    border-bottom: 1px solid var(--border);
    margin-bottom: 0.25rem;
  }
  .head .num { text-align: right; }
  .rows { display: flex; flex-direction: column; min-width: 0; }
  /* The bar lines up with the card's own text. */
  .tools { margin: 0 -0.55rem -0.35rem; }
  /* Past the state dot, as in the rows. */
  .head > :first-child { padding-left: calc(8px + 0.6rem); }

  .output { display: flex; align-items: flex-start; gap: 0.5rem; }
  .output details { flex: 1; min-width: 0; }
  .output summary { cursor: pointer; color: var(--muted); }
  .output pre {
    margin: 0.4rem 0 0;
    padding: 0.6rem 0.7rem;
    max-height: 14rem;
    overflow: auto;
    background: var(--bg-sunk);
    border-radius: var(--radius-sm);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .x { border: none; background: none; color: var(--muted); cursor: pointer; padding: 0 0.3rem; font-size: 1rem; }

  .spinner {
    width: 0.9rem;
    height: 0.9rem;
    border: 2px solid color-mix(in srgb, var(--accent) 30%, transparent);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .spinner { animation: none; } }

  /* Below what the columns need: the minimums in --cols (45.4rem) and five gaps of 0.9rem. Change it with them,
     here and in ContainerRow.svelte. */
  @container stack (max-width: 50rem) {
    .head { display: none; }
  }
</style>
