<script lang="ts">
  // A container's log, live: the last lines first, then new ones as they come. Search shows only the lines
  // with the text, and the whole log held here can be saved to a file. Following stops when the panel closes.
  import { onDestroy, onMount, tick, untrack } from "svelte";
  import { drafts } from "$lib/drafts.svelte";
  import { errorText, type Device } from "$lib/api";
  import { dockerApi, dockerEvents, type Container, type DockerHost } from "$lib/docker/api";
  import { dockerIcons } from "$lib/docker/icons";
  import { highlight, keepLast, logFileName, logLine, matches, type LogLine } from "$lib/docker/logic";
  import { dockerProblem } from "$lib/docker/problem";
  import Icon from "../Icon.svelte";
  import Problem from "../Problem.svelte";

  let { host, device, container, onclose }: { host: DockerHost; device: Device | undefined; container: Container; onclose: () => void } =
    $props();

  /** Lines held in the panel; older ones are dropped. */
  const KEEP = 20000;
  /** Lines drawn at once. More would make the window slow; search reaches the rest. */
  const DRAW = 1500;
  const TAILS = [
    { value: 200, label: "Last 200 lines" },
    { value: 1000, label: "Last 1,000 lines" },
    { value: 5000, label: "Last 5,000 lines" },
    { value: 0, label: "The whole log" },
  ];

  interface Held extends LogLine {
    raw: string;
    n: number;
  }

  let lines = $state.raw<Held[]>([]);
  let dropped = $state(0);
  // The search and how much of the log to show stay while another screen is shown (drafts.svelte.ts).
  type LogsView = { query: string; tail: number };
  const draftKey = untrack(() => `docker-logs:${host.deviceId}:${container.id}`);
  const view = drafts.get<LogsView>(draftKey);
  let query = $state(view?.query ?? "");
  let tail = $state(view?.tail ?? 200);
  $effect(() => drafts.keep<LogsView>(draftKey, { query, tail }, { query: "", tail: 200 }));

  function close() {
    drafts.clear(draftKey);
    onclose();
  }
  let phase = $state<"starting" | "following" | "ended">("starting");
  let ended = $state<string | null>(null);
  let error = $state<string | null>(null);
  let saved = $state<string | null>(null);
  let saveError = $state<string | null>(null);
  /** Keep the newest line in view. Off while the owner scrolls back. */
  let stick = $state(true);
  let box = $state<HTMLDivElement>();

  let streamId: number | null = null;
  /** Lines (and ends) of streams whose id isn't known yet: events can come before the start call returns.
   *  The panel is drawn afresh for another container (see the {#key} around it). */
  const early = new Map<number, string[]>();
  const earlyEnd = new Map<number, string | null>();
  let count = 0;
  let generation = 0;
  const unlisten: (() => void)[] = [];

  const shown = $derived(query.trim() ? lines.filter((l) => matches(l.text, query)) : lines);
  const drawn = $derived(shown.length > DRAW ? shown.slice(shown.length - DRAW) : shown);
  const timeFormat = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });

  function add(raw: string[]) {
    const fresh = raw.map((r) => ({ ...logLine(r), raw: r, n: ++count }));
    const all = lines.concat(fresh);
    const kept = keepLast(all, KEEP);
    dropped += all.length - kept.length;
    lines = kept;
  }

  async function start() {
    const mine = ++generation;
    stop();
    lines = [];
    dropped = 0;
    count = 0;
    ended = null;
    error = null;
    phase = "starting";
    stick = true;
    try {
      const id = await dockerApi.logsStart(host.deviceId, container.id, tail || null);
      if (mine !== generation) {
        void dockerApi.logsStop(id);
        return;
      }
      streamId = id;
      phase = "following";
      const waiting = early.get(id);
      early.clear();
      if (waiting) add(waiting);
      if (earlyEnd.has(id)) {
        streamId = null;
        phase = "ended";
        ended = earlyEnd.get(id) ?? null;
      }
      earlyEnd.clear();
    } catch (e) {
      if (mine === generation) {
        error = errorText(e);
        phase = "ended";
      }
    }
  }

  function stop() {
    if (streamId != null) void dockerApi.logsStop(streamId).catch(() => {});
    streamId = null;
  }

  onMount(() => {
    dockerEvents
      .onLogs((e) => {
        if (e.id === streamId) add(e.lines);
        else if (streamId == null) early.set(e.id, [...(early.get(e.id) ?? []), ...e.lines]);
      })
      .then((u) => unlisten.push(u), (e) => (error = errorText(e)));
    dockerEvents
      .onLogsEnd((e) => {
        if (e.id !== streamId) {
          if (streamId == null) earlyEnd.set(e.id, e.reason);
          return;
        }
        streamId = null;
        phase = "ended";
        ended = e.reason;
      })
      .then((u) => unlisten.push(u), (e) => (error = errorText(e)));
    void start();
  });

  onDestroy(() => {
    generation++;
    stop();
    for (const u of unlisten) u();
  });

  $effect(() => {
    void drawn;
    if (!stick) return;
    void tick().then(() => box?.scrollTo({ top: box.scrollHeight }));
  });

  function scrolled() {
    if (!box) return;
    stick = box.scrollHeight - box.scrollTop - box.clientHeight < 30;
  }

  async function download() {
    saveError = null;
    saved = null;
    try {
      const name = logFileName(device?.name ?? "docker", container.name, new Date());
      saved = await dockerApi.saveLog(name, lines.map((l) => l.raw).join("\n") + "\n");
    } catch (e) {
      saveError = errorText(e);
    }
  }

  async function showSaved() {
    if (!saved) return;
    try {
      const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
      await revealItemInDir(saved);
    } catch (e) {
      saveError = `Couldn't open the folder: ${errorText(e)}`;
    }
  }
</script>

<section class="panel logs" aria-label="Log of {container.name}">
  <header>
    <div class="title">
      <h2 class="ellipsis" title={container.name}>{container.name}</h2>
      <span class="muted small">
        {#if phase === "starting"}Starting…{:else if phase === "following"}<span class="live">Live</span>{:else}Not following{/if}
      </span>
    </div>
    <button class="icon-btn" aria-label="Close the log" title="Close" onclick={close}><Icon path={dockerIcons.close} size={16} /></button>
  </header>

  <div class="tools">
    <input type="search" placeholder="Search the log" aria-label="Search the log" bind:value={query} />
    <select aria-label="How much of the log to show" bind:value={tail} onchange={() => start()}>
      {#each TAILS as t}<option value={t.value}>{t.label}</option>{/each}
    </select>
    <button class="btn small" onclick={download} disabled={lines.length === 0}>
      <Icon path={dockerIcons.download} size={15} /> Save to a file
    </button>
  </div>

  <p class="count muted small">
    {#if query.trim()}
      {shown.length.toLocaleString()} of {lines.length.toLocaleString()} lines hold “{query.trim()}”
    {:else}
      {lines.length.toLocaleString()} lines
    {/if}
    {#if shown.length > DRAW}· showing the newest {DRAW.toLocaleString()}{/if}
    {#if dropped > 0}· {dropped.toLocaleString()} older lines dropped from view{/if}
  </p>

  {#if error}
    <Problem problem={dockerProblem(device, `Following the log of "${container.name}" on "${device?.name}"`, error, host)} />
    <div><button class="btn small" onclick={() => start()}>Try again</button></div>
  {/if}
  {#if saveError}
    <Problem problem={{ ...dockerProblem(device, `Saving the log of "${container.name}"`, saveError, host), about: "home-cc" }} />
  {/if}
  {#if saved}
    <p class="saved small" role="status">
      Saved to <span class="mono">{saved}</span>
      <button class="btn small" onclick={showSaved}>Show in folder</button>
    </p>
  {/if}

  <div class="lines mono" bind:this={box} onscroll={scrolled} role="log" aria-live="off" tabindex="-1">
    {#each drawn as l (l.n)}
      <div class="line">
        <span class="at">{l.at != null ? timeFormat.format(l.at) : ""}</span>
        <span class="text">{#each highlight(l.text, query) as part}{#if part.hit}<mark>{part.text}</mark>{:else}{part.text}{/if}{/each}</span>
      </div>
    {:else}
      <p class="empty muted">{phase === "starting" ? "Fetching the log…" : query.trim() ? "No lines hold that text." : "Nothing in the log yet."}</p>
    {/each}
  </div>

  {#if phase === "ended" && !error}
    <p class="ended small" role="status">
      {ended ?? "Stopped following."}
      <button class="btn small" onclick={() => start()}>Follow again</button>
    </p>
  {:else if !stick}
    <button class="btn small jump" onclick={() => (stick = true)}>Jump to the newest lines</button>
  {/if}
</section>

<style>
  .logs { display: flex; flex-direction: column; gap: 0.6rem; min-width: 0; padding: 0.85rem 1rem 1rem; }
  header { display: flex; align-items: center; justify-content: space-between; gap: 0.75rem; }
  .title { display: flex; align-items: baseline; gap: 0.75rem; min-width: 0; }
  h2 { margin: 0; font-size: 1.15rem; font-weight: 600; }
  .live { color: var(--ok); }
  .tools { display: flex; gap: 0.5rem; flex-wrap: wrap; }
  .tools input { flex: 1 1 14rem; min-width: 0; width: auto; }
  .tools select { flex: 0 1 12rem; width: auto; }
  .tools .btn { display: inline-flex; align-items: center; gap: 0.4rem; }
  p { margin: 0; }
  .saved { display: flex; align-items: center; gap: 0.6rem; flex-wrap: wrap; overflow-wrap: anywhere; }
  .lines {
    height: clamp(16rem, calc(100vh - 19rem), 70rem);
    overflow: auto;
    padding: 0.5rem 0.6rem;
    background: var(--bg-sunk);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    font-size: 0.86rem;
    line-height: 1.45;
    user-select: text;
  }
  .line { display: grid; grid-template-columns: 5.2rem minmax(0, 1fr); gap: 0.6rem; }
  .at { color: var(--idle); white-space: nowrap; }
  .text { white-space: pre-wrap; overflow-wrap: anywhere; color: var(--text-soft); }
  mark { background: color-mix(in srgb, var(--warn) 45%, transparent); color: var(--text); border-radius: 2px; }
  .empty { padding: 0.5rem; }
  .ended { display: flex; align-items: center; gap: 0.75rem; flex-wrap: wrap; color: var(--text-soft); }
  .jump { align-self: flex-start; }
</style>
