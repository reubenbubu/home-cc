<script lang="ts">
  // One Pi-hole on the Pi-hole screen: whether it blocks (and for how long a pause has left), today's numbers,
  // pausing and resuming, its certificate and connection settings, and anything that went wrong.
  import { untrack } from "svelte";
  import type { Device } from "$lib/api";
  import { clock } from "$lib/clock.svelte";
  import { drafts } from "$lib/drafts.svelte";
  import { relTime } from "$lib/format";
  import { icons } from "$lib/icons";
  import { blockingView, listTrouble, number, percent } from "$lib/pihole/logic";
  import { piholeProblem } from "$lib/pihole/problem";
  import { pihole } from "$lib/pihole/store.svelte";
  import Address from "../Address.svelte";
  import Icon from "../Icon.svelte";
  import Problem from "../Problem.svelte";
  import PauseControl from "./PauseControl.svelte";
  import PiholeCert from "./PiholeCert.svelte";
  import PiholeSetup from "./PiholeSetup.svelte";

  let { device }: { device: Device } = $props();

  const id = $derived(device.id);
  const settings = $derived(pihole.settings(id));
  const status = $derived(pihole.statuses.get(id) ?? null);
  const good = $derived(pihole.lastGood.get(id) ?? null);
  const readError = $derived(pihole.readErrors.get(id) ?? null);
  const trustError = $derived(pihole.trustErrors.get(id) ?? null);
  const switchError = $derived(pihole.switchErrors.get(id) ?? null);
  const reading = $derived(pihole.reading.has(id));
  const kind = $derived(status?.errorKind ?? null);
  const askCert = $derived((kind === "newCert" || kind === "certChanged") && !!status?.offeredFingerprint);
  /** Numbers are on screen, but the latest try failed. */
  const stale = $derived(!!good && (!!readError || (!!status && !status.reading)));

  // A countdown that moves every second while a pause runs (the shared clock ticks every few seconds).
  let now = $state(Date.now() / 1000);
  const counting = $derived(good?.reading.blocking.timerSecs != null);
  $effect(() => {
    if (!counting) return;
    const t = setInterval(() => (now = Date.now() / 1000), 1000);
    return () => clearInterval(t);
  });
  const view = $derived(good ? blockingView(good.reading.blocking, good.fetchedAt, Math.max(now, clock.now)) : null);
  const switching = $derived(pihole.switching.get(id) ?? null);
  /** The pause choices are open (PauseControl brings them back after another screen was shown). */
  let pauseOpen = $state(false);

  // What is open here comes back after another screen was shown (drafts.svelte.ts).
  const setupKey = untrack(() => `pihole-setup-open:${device.id}`);
  let setupOpen = $state(drafts.get<boolean>(setupKey) ?? false);
  $effect(() => drafts.remember(setupKey, setupOpen));
  const showSetup = $derived(setupOpen || kind === "noPassword");

  /** A problem on this Pi-hole, with what helps find the cause. Never the password. */
  function problem(what: string, error: string) {
    return piholeProblem(device, what, error, settings, status, {
      "Last good reading": good ? relTime(good.fetchedAt, clock.now) : "none yet",
    });
  }

  /** The dot by the name: how blocking is, or that the Pi-hole can't be read (and whether that waits for the owner). */
  const dot = $derived(
    view && !stale ? view.tone : readError || (status?.error && kind !== "noPassword" && !askCert) ? "bad" : status?.error ? "warn" : "idle",
  );
  const dotText = $derived(view && !stale ? view.label : status?.error || readError ? "Can't be read" : "Not read yet");

  const failedLists = $derived((good?.reading.lists ?? []).filter((l) => l.enabled && listTrouble(l.status)));
  const version = $derived(good?.reading.version ?? null);
</script>

<article class="panel card" aria-label="Pi-hole {device.name}">
  <header class="top">
    <div class="name-line">
      <span class="dot {dot}" role="img" aria-label={dotText} title={dotText}></span>
      <h2 class="name ellipsis" title={device.name}>{device.name}</h2>
      <span class="tools">
        <button class="icon-btn" class:spin={reading} aria-label="Read {device.name} now" title="Read now" disabled={reading} onclick={() => pihole.refresh(id)}>
          <Icon path={icons.refresh} size={16} />
        </button>
        {#if !showSetup}
          <button class="btn small" onclick={() => (setupOpen = true)}>Connection settings</button>
        {/if}
      </span>
    </div>
    <div class="muted small where">
      <Address host={device.host} known={device.hostname} />
      <span>· {settings?.https === false ? "HTTP" : "HTTPS"}{settings && settings.port !== (settings.https ? 443 : 80) ? ` port ${settings.port}` : ""}</span>
      {#if good}<span>· {stale ? "last good reading" : "updated"} {relTime(good.fetchedAt, clock.now)}</span>{/if}
    </div>
  </header>

  {#if readError}
    <Problem problem={problem(`Reading Pi-hole "${device.name}"`, readError)} />
  {/if}
  {#if trustError}
    <Problem problem={problem(`Approving the certificate of "${device.name}"`, trustError)} message="Not approved: {trustError}" />
  {/if}
  {#if status && askCert}
    <PiholeCert {device} {status} />
  {:else if status?.error}
    <Problem problem={problem(`Reading Pi-hole "${device.name}" through its API`, status.error)} tone={kind === "noPassword" ? "warn" : undefined} />
  {/if}
  {#if switchError}
    <Problem problem={problem(`Pausing or resuming blocking on "${device.name}"`, switchError)} />
  {/if}
  <!-- Under what went wrong, which it is often the fix for. -->
  {#if showSetup}
    <PiholeSetup {device} {problem} onclose={kind === "noPassword" ? undefined : () => (setupOpen = false)} />
  {/if}

  {#if !status && !readError}
    <p class="muted">Connecting to {device.name}…</p>
  {/if}

  {#if good && view}
    {@const r = good.reading}
    {@const s = r.summary}
    <div class="blocking {view.tone}" class:stale>
      <div class="state">
        <strong>{view.label}</strong>
        {#if view.detail}<span class="detail">{view.detail}</span>{/if}
      </div>
      <div class="acts">
        {#if switching}
          <span class="muted small">{switching}</span>
        {:else if r.blocking.state !== "enabled"}
          <button class="btn small primary" onclick={() => pihole.setBlocking([id], true, null)} disabled={stale}>Resume blocking</button>
        {:else if !pauseOpen}
          <button class="btn small" onclick={() => (pauseOpen = true)} disabled={stale}>Pause blocking…</button>
        {/if}
      </div>
    </div>
    {#if r.blocking.state === "enabled" && !switching}
      <PauseControl ids={[id]} target={device.name} draftKey="pihole-pause:{id}" bind:open={pauseOpen} button={false} />
    {/if}

    <dl class="numbers" class:stale>
      <div class="big">
        <dt>Queries today</dt>
        <dd class="mono">{number(s.queries)}</dd>
      </div>
      <div class="big">
        <dt>Blocked today</dt>
        <dd class="mono">{number(s.blocked)} <span class="pct">{percent(s.percentBlocked)}</span></dd>
      </div>
      <div>
        <dt>Clients</dt>
        <dd class="mono">{number(s.clientsActive)}{#if s.clientsTotal != null}&nbsp;<span class="of">of {number(s.clientsTotal)} seen</span>{/if}</dd>
      </div>
      <div>
        <dt>Domains on blocklists</dt>
        <dd class="mono">{number(s.domainsOnLists)}</dd>
      </div>
      <div>
        <dt>Blocklists updated</dt>
        <dd>{s.gravityUpdated ? relTime(s.gravityUpdated, clock.now) : "not known"}</dd>
      </div>
      <div>
        <dt>Version</dt>
        <dd title={version ? `Core ${version.core ?? "?"}, FTL ${version.ftl ?? "?"}, web interface ${version.web ?? "?"}${version.docker ? `, Docker image ${version.docker}` : ""}` : undefined}>
          {#if version?.core}{version.core}{#if version.ftl}&nbsp;<span class="muted small">FTL {version.ftl}</span>{/if}{:else}not known{/if}
        </dd>
      </div>
    </dl>
    {#if r.listsError}
      <p class="muted small">The lists couldn't be read: {r.listsError}</p>
    {:else if failedLists.length}
      <p class="small trouble">
        <Icon path={icons.warn} size={13} />
        <span>{failedLists.length === 1 ? "1 list" : `${failedLists.length} lists`} couldn't be downloaded at the last update: <span class="mono">{failedLists.map((l) => l.address).join(", ")}</span></span>
      </p>
    {/if}
  {/if}
</article>

<style>
  .card { display: flex; flex-direction: column; gap: 0.8rem; min-width: 0; }
  .top { display: flex; flex-direction: column; gap: 0.25rem; min-width: 0; }
  .name-line { display: flex; align-items: center; gap: 0.6rem; min-width: 0; }
  .name { margin: 0; font-size: 1.1rem; font-weight: 600; flex: 1; }
  .tools { display: flex; align-items: center; gap: 0.4rem; flex: none; }
  .where { display: flex; flex-wrap: wrap; gap: 0 0.35rem; align-items: baseline; min-width: 0; }
  .dot.idle { border: 2px solid var(--idle); }

  .blocking {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem 1rem;
    flex-wrap: wrap;
    padding: 0.6rem 0.8rem;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
  }
  .blocking.ok { border-color: color-mix(in srgb, var(--ok) 45%, transparent); background: color-mix(in srgb, var(--ok) 7%, transparent); }
  .blocking.warn { border-color: var(--warn); background: color-mix(in srgb, var(--warn) 9%, transparent); }
  .blocking.bad { border-color: var(--bad); background: color-mix(in srgb, var(--bad) 9%, transparent); }
  .state { display: flex; align-items: baseline; gap: 0.3rem 0.7rem; flex-wrap: wrap; min-width: 0; }
  .ok strong { color: var(--ok); }
  .warn strong { color: var(--warn); }
  .bad strong { color: var(--bad); }
  .detail { color: var(--text-soft); font-variant-numeric: tabular-nums; }
  .acts { display: flex; gap: 0.4rem; flex-wrap: wrap; }

  .numbers { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 11rem), 1fr)); gap: 0.7rem 1rem; margin: 0; }
  .numbers > div { min-width: 0; }
  .numbers dt { color: var(--muted); font-size: 0.85rem; }
  .numbers dd { margin: 0.15rem 0 0; overflow-wrap: anywhere; }
  .numbers .big dd { font-size: 1.45rem; }
  .pct { font-size: 1rem; color: var(--text-soft); }
  .of { color: var(--muted); font-size: 0.9em; }
  .stale { opacity: 0.6; }
  .trouble { display: flex; gap: 0.4rem; margin: 0; color: var(--text-soft); }
  .trouble :global(svg) { color: var(--warn); flex: none; margin-top: 0.15rem; }
  .trouble .mono { overflow-wrap: anywhere; }

  .spin :global(svg) { animation: spin 0.9s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) {
    .spin :global(svg) { animation: none; }
  }
</style>
