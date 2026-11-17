<script lang="ts">
  // Terminal tabs to the machines, over SSH. The tabs themselves live in terminals.svelte.ts, so they keep
  // running while another screen is shown; this screen shows them and the bar to open more.
  import { tick } from "svelte";
  import type { Action } from "svelte/action";
  import { devices } from "$lib/devices.svelte";
  import { deviceDetails } from "$lib/problems.svelte";
  import type { Problem as ProblemInfo } from "$lib/report";
  import { terminals, type Tab } from "$lib/terminals.svelte";
  import Address from "$lib/components/Address.svelte";
  import Problem from "$lib/components/Problem.svelte";

  const byName = (a: { name: string }, b: { name: string }) => a.name.localeCompare(b.name, undefined, { numeric: true });
  /** Machines home-cc can log in to: they have an SSH user. */
  const reachable = $derived(devices.list.filter((d) => d.kind !== "ignore" && d.sshUser?.trim()).sort(byName));
  let chosen = $state<number | null>(null);
  const picked = $derived(reachable.find((d) => d.id === chosen) ?? reachable[0] ?? null);

  function openChosen() {
    if (picked) terminals.open({ deviceId: picked.id, title: picked.name });
  }

  /** Lends a tab's terminal to this screen while it is shown, and fits it to the space it gets. */
  const host: Action<HTMLElement, number> = (node, key) => {
    terminals.attach(key, node);
    const resized = new ResizeObserver(() => terminals.fit(key));
    resized.observe(node);
    return {
      destroy() {
        resized.disconnect();
        terminals.detach(key);
      },
    };
  };

  // The tab shown takes the keyboard, also after coming back to this screen.
  $effect(() => {
    const key = terminals.active;
    if (key == null) return;
    tick().then(() => {
      terminals.fit(key);
      terminals.focus(key);
    });
  });

  function where(tab: Tab): string {
    const d = devices.byId.get(tab.deviceId);
    const at = d ? `${d.sshUser}@${d.host}` : "";
    return [at, tab.command, tab.programTitle].filter(Boolean).join(" · ");
  }

  /** The tab shown, and the machine it is on, for the line beside the tabs. */
  const shownTab = $derived(terminals.tabs.find((t) => t.key === terminals.active) ?? null);
  const shownDevice = $derived(shownTab ? devices.byId.get(shownTab.deviceId) : undefined);

  function stateText(tab: Tab): string {
    return { connecting: "Connecting", open: "Connected", ended: "Ended", failed: "Couldn't connect" }[tab.state];
  }

  function openProblem(tab: Tab): ProblemInfo {
    const d = devices.byId.get(tab.deviceId);
    return {
      area: "terminal",
      what: `Opening a terminal on "${d?.name ?? tab.title}"${tab.command ? " to run a command" : ""}`,
      error: tab.error ?? "",
      details: { ...(d ? deviceDetails(d) : { Device: `id ${tab.deviceId}, no longer saved` }), Command: tab.command },
    };
  }

  /** A terminal that ended for a reason worth handing on: the connection was lost, or the program was
   *  stopped or failed. */
  function endProblem(tab: Tab): ProblemInfo {
    const d = devices.byId.get(tab.deviceId);
    return {
      area: "terminal",
      what: `A terminal on "${d?.name ?? tab.title}"${tab.command ? " running a command" : ""}, which ended`,
      error: tab.endReason ?? "",
      details: {
        ...(d ? deviceDetails(d) : { Device: `id ${tab.deviceId}, no longer saved` }),
        Command: tab.command,
        "Exit code": tab.exitStatus == null ? null : String(tab.exitStatus),
      },
    };
  }

  /** Typing, a paste or a new size that didn't reach the machine. */
  function sendProblem(tab: Tab): ProblemInfo {
    const d = devices.byId.get(tab.deviceId);
    return {
      area: "terminal",
      what: `Sending what was typed or pasted to the terminal on "${d?.name ?? tab.title}"`,
      error: tab.sendError ?? "",
      details: { ...(d ? deviceDetails(d) : { Device: `id ${tab.deviceId}, no longer saved` }), Command: tab.command },
    };
  }

  /** Arrow keys move between tabs, as in other tab bars. */
  function tabKeys(e: KeyboardEvent, i: number) {
    const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (!step || terminals.tabs.length < 2) return;
    e.preventDefault();
    const next = terminals.tabs[(i + step + terminals.tabs.length) % terminals.tabs.length];
    terminals.active = next.key;
    document.getElementById(`tab-${next.key}`)?.focus();
  }
</script>

<div class="term-page">
  <header class="page-head">
    <h1>Terminals</h1>
    <form
      class="actions opener"
      onsubmit={(e) => {
        e.preventDefault();
        openChosen();
      }}
    >
      <label class="pick">
        <span class="sr-only">Machine</span>
        <select bind:value={() => picked?.id ?? null, (v) => (chosen = v)} disabled={reachable.length === 0}>
          {#each reachable as d (d.id)}
            <option value={d.id}>{d.name} ({d.sshUser}@{d.host})</option>
          {:else}
            <option value={null}>No machine has an SSH user yet</option>
          {/each}
        </select>
      </label>
      <!-- The option's text can't hold the address's hostname as a field of its own: it is shown here. -->
      {#if picked}<span class="picked-at small"><Address host={picked.host} known={picked.hostname} /></span>{/if}
      <button class="btn primary" type="submit" disabled={!picked}>Open terminal</button>
    </form>
  </header>

  {#if terminals.clipboardError}
    <Problem
      class="clip"
      problem={{ area: "terminal", what: "Copying or pasting in a terminal tab", error: terminals.clipboardError }}
    />
  {/if}

  {#if terminals.tabs.length === 0}
    <section class="panel empty">
      <h2 class="section-title">No terminals open</h2>
      {#if devices.loaded && reachable.length === 0}
        <p>
          A terminal needs a machine that home-cc can log in to. Add an SSH user to one on the
          <a href="/machines">Machines</a> screen, then come back here.
        </p>
      {:else}
        <p>Pick a machine above and press <strong>Open terminal</strong>, or use the terminal button on a machine's card.</p>
        <p class="muted">
          You can have several open at once, to different machines. Copy with Ctrl+Shift+C, or Ctrl+C while text is
          selected; paste with Ctrl+Shift+V. Right-click copies the selection, or pastes when nothing is selected.
        </p>
      {/if}
    </section>
  {:else}
    <div class="tabbar">
    <div class="tabs" role="tablist" aria-label="Open terminals">
      {#each terminals.tabs as tab, i (tab.key)}
        <div class="tab" class:active={tab.key === terminals.active} title={where(tab)}>
          <button
            id="tab-{tab.key}"
            class="tab-name"
            role="tab"
            aria-selected={tab.key === terminals.active}
            aria-controls="pane-{tab.key}"
            tabindex={tab.key === terminals.active ? 0 : -1}
            onclick={() => (terminals.active = tab.key)}
            onkeydown={(e) => tabKeys(e, i)}
          >
            <span
              class="dot"
              class:ok={tab.state === "open"}
              class:idle={tab.state === "connecting" || tab.state === "ended"}
              class:bad={tab.state === "failed"}
              class:pulse={tab.state === "connecting"}
              role="img"
              aria-label={stateText(tab)}
            ></span>
            <span class="ellipsis">{tab.title}</span>
          </button>
          <button class="icon-btn close" aria-label="Close {tab.title}" title="Close (ends the connection)" onclick={() => terminals.close(tab.key)}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
              <path d="M6 6l12 12M18 6L6 18" />
            </svg>
          </button>
        </div>
      {/each}
    </div>
    {#if shownTab && shownDevice}
      <p class="where small muted" title={where(shownTab)}>
        <span class="user">{shownDevice.sshUser}@</span><Address host={shownDevice.host} known={shownDevice.hostname} />
        {#if shownTab.command}<span class="command mono ellipsis">{shownTab.command}</span>{/if}
      </p>
    {/if}
    </div>

    {#each terminals.tabs as tab (tab.key)}
      <div
        id="pane-{tab.key}"
        class="pane"
        role="tabpanel"
        aria-labelledby="tab-{tab.key}"
        hidden={tab.key !== terminals.active}
      >
        {#if tab.state === "failed"}
          <div class="banner">
            <Problem problem={openProblem(tab)} message="Couldn't open the terminal: {tab.error}" />
            <div class="banner-actions">
              <button class="btn small" onclick={() => terminals.reconnect(tab.key)}>Try again</button>
              {#if tab.error?.includes("Machines screen")}<a class="btn small link-btn" href="/machines">Go to Machines</a>{/if}
            </div>
          </div>
        {:else if tab.state === "ended"}
          {#snippet again()}
            <div class="banner-actions">
              <button class="btn small" onclick={() => terminals.reconnect(tab.key)}>Reconnect</button>
              <button class="btn small" onclick={() => terminals.close(tab.key)}>Close tab</button>
            </div>
          {/snippet}
          {#if tab.endReason}
            <!-- The connection was lost, or the program was stopped or ended with an error code. -->
            <div class="banner">
              <Problem problem={endProblem(tab)} message={tab.ended ?? ""} tone={tab.exitStatus != null ? "warn" : undefined} />
              {@render again()}
            </div>
          {:else}
            <div class="banner ended" role="status">
              <p>{tab.ended}</p>
              {@render again()}
            </div>
          {/if}
        {:else if tab.state === "connecting"}
          <p class="connecting muted" role="status">Connecting to {tab.title}…</p>
        {/if}
        {#if tab.sendError}
          <div class="banner">
            <Problem problem={sendProblem(tab)} message="Not sent to {tab.title}: {tab.sendError}" />
            <div class="banner-actions">
              <button class="btn small" onclick={() => terminals.dismissSendError(tab.key)}>Dismiss</button>
            </div>
          </div>
        {/if}
        <!-- Kept in place (it is measured, and takes the keyboard while connecting) but not seen until
             something has run in it. -->
        <div class="screen" class:quiet={!tab.shown} use:host={tab.key}></div>
      </div>
    {/each}
  {/if}
</div>

<style>
  /* The terminal takes all the height the window gives it. */
  .term-page { display: flex; flex-direction: column; height: 100%; min-height: 24rem; }
  .page-head { margin-bottom: 0.9rem; }
  .opener { flex: 1 1 22rem; justify-content: flex-end; min-width: 0; flex-wrap: nowrap; }
  .pick { flex: 0 1 26rem; min-width: 0; }
  .opener .btn { flex: none; }
  .term-page :global(.clip) { margin-bottom: 0.75rem; }

  .empty { max-width: 46rem; display: flex; flex-direction: column; gap: 0.6rem; }
  .empty p { margin: 0; line-height: 1.55; }

  /* The tabs, with the shown tab's login and command beside them; that goes above them when there is no room. */
  .tabbar { display: flex; flex-wrap: wrap-reverse; align-items: flex-end; gap: 0 1rem; border-bottom: 1px solid var(--border); min-width: 0; }
  .tabs { display: flex; gap: 0.25rem; flex-wrap: wrap; min-width: 0; }
  .where { display: flex; align-items: baseline; gap: 0 0.6rem; min-width: 0; max-width: 100%; margin: 0 0 0.55rem auto; }
  .where .user { margin-right: -0.6rem; flex: none; }
  .where .command { min-width: 0; color: var(--text-soft); }
  .picked-at { display: flex; align-self: center; min-width: 0; flex: 0 1 auto; color: var(--text-soft); }
  .tab {
    display: flex;
    align-items: center;
    min-width: 0;
    max-width: 18rem;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: var(--radius-sm) var(--radius-sm) 0 0;
    color: var(--muted);
  }
  .tab:hover { color: var(--text); background: var(--panel); }
  .tab.active { color: var(--text); background: var(--panel); border-color: var(--border); margin-bottom: -1px; }
  .tab.active .tab-name { box-shadow: inset 0 2px 0 var(--accent); }
  .tab-name {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
    min-height: 2.5rem;
    padding: 0 0.3rem 0 0.8rem;
    border: none;
    background: none;
    color: inherit;
    cursor: pointer;
    font-weight: 500;
    border-radius: var(--radius-sm) 0 0 0;
  }
  .tab-name:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .close { width: 1.9rem; height: 1.9rem; margin-right: 0.25rem; }
  .pulse { animation: pulse 1.2s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: 0.35; } }
  @media (prefers-reduced-motion: reduce) { .pulse { animation: none; } }

  .pane {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-sunk);
    border: 1px solid var(--border);
    border-top: none;
    border-radius: 0 0 var(--radius) var(--radius);
  }
  .pane[hidden] { display: none; }
  .banner { display: flex; flex-direction: column; gap: 0.5rem; padding: 0.7rem 0.8rem; border-bottom: 1px solid var(--border); }
  .banner.ended { flex-direction: row; align-items: center; justify-content: space-between; flex-wrap: wrap; background: var(--panel); }
  .banner p { margin: 0; line-height: 1.5; overflow-wrap: anywhere; }
  .banner-actions { display: flex; gap: 0.5rem; flex-wrap: wrap; }
  .connecting { margin: 0; padding: 0.5rem 0.8rem; }

  /* xterm.js draws into this; the padding keeps letters off the border. */
  .screen { flex: 1; min-height: 0; min-width: 0; padding: 0.4rem 0 0.2rem 0.6rem; overflow: hidden; }
  /* Not visibility: hidden, which would stop the terminal taking the keyboard, so keys typed while it
     connects would go nowhere. */
  .screen.quiet { opacity: 0; pointer-events: none; }
  .screen :global(.terminal-host) { width: 100%; height: 100%; }
  .screen :global(.xterm) { height: 100%; }
  /* Every character as typed: no joined-up arrows for "->" or "!=", as a terminal should be. */
  .screen :global(.xterm-rows) { font-variant-ligatures: none; font-feature-settings: "liga" 0, "calt" 0; }
  /* Below the last whole row: the terminal's own colour, not xterm.js's black. */
  .screen :global(.xterm .xterm-viewport) { background-color: var(--bg-sunk); scrollbar-width: thin; }
</style>
