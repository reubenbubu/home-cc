<script lang="ts">
  // One Proxmox host on the Proxmox screen: its token, its certificate, its nodes and its guests. Read
  // again every few seconds while it is shown.
  import { tick, untrack } from "svelte";
  import type { Device, ProxmoxGuest } from "$lib/api";
  import { clock } from "$lib/clock.svelte";
  import { relTime } from "$lib/format";
  import { icons } from "$lib/icons";
  import { guestKey } from "$lib/proxmox";
  import { proxmox } from "$lib/proxmox.svelte";
  import { deviceDetails } from "$lib/problems.svelte";
  import type { Problem as ProblemInfo } from "$lib/report";
  import Icon from "../Icon.svelte";
  import Problem from "../Problem.svelte";
  import CertCheck from "./CertCheck.svelte";
  import GuestPanel from "./GuestPanel.svelte";
  import GuestTable from "./GuestTable.svelte";
  import HostSetup from "./HostSetup.svelte";
  import NodeCard from "./NodeCard.svelte";

  let { device }: { device: Device } = $props();

  // Read now and every few seconds, for as long as this host is on screen.
  $effect(() => {
    const id = device.id;
    return untrack(() => proxmox.watch(id));
  });

  const status = $derived(proxmox.statuses.get(device.id) ?? null);
  const good = $derived(proxmox.lastGood.get(device.id) ?? null);
  const readError = $derived(proxmox.readErrors.get(device.id) ?? null);
  const trustError = $derived(proxmox.trustErrors.get(device.id) ?? null);
  const reading = $derived(proxmox.reading.has(device.id));
  const kind = $derived(status?.errorKind ?? null);
  const askCert = $derived((kind === "newCert" || kind === "certChanged") && !!status?.offeredFingerprint);
  /** Readings are on screen, but the latest try failed: Proxmox's answer was an error, or no answer could be
   *  asked for (readError, in which case status still holds the last answer). */
  const stale = $derived(!!good && (!!readError || (!!status && !status.overview)));
  /** A token ID is saved but its secret isn't in the keychain (lost with a keychain reset, say): the form shows,
   *  and this says why. Without a token ID the form says enough by itself. */
  const missingSecret = $derived(kind === "noToken" && !!proxmox.settings.get(device.id)?.tokenId);

  let setupOpen = $state(false);
  const showSetup = $derived(setupOpen || kind === "noToken");

  let selectedKey = $state<string | null>(null);
  const selected = $derived(
    selectedKey && good ? (good.overview.guests.find((g) => guestKey(g) === selectedKey) ?? null) : null,
  );
  let panelEl = $state<HTMLElement>();

  async function select(g: ProxmoxGuest) {
    selectedKey = guestKey(g);
    // Below 1500px the panel sits above the list, which may be scrolled out of view.
    await tick();
    panelEl?.scrollIntoView({ block: "nearest" });
  }

  /** A problem on this host, with what helps find the cause. Never the secret. */
  function problem(what: string, error: string): ProblemInfo {
    const s = proxmox.settings.get(device.id);
    return {
      area: "proxmox",
      what,
      error,
      details: {
        ...deviceDetails(device),
        "Kind of error": status?.errorKind,
        "API port": s?.port,
        "Token ID": s?.tokenId,
        "Token secret stored": s?.hasSecret,
        "Certificate approved": status ? !!status.pinnedFingerprint : null,
        "Last good reading": good ? relTime(good.fetchedAt, clock.now) : "none yet",
      },
    };
  }
</script>

<div class="host">
  <div class="bar">
    <span class="muted small">
      <strong class="name">{device.name}</strong>
      <span class="mono">· {device.host}</span>
      {#if good}
        <span>· {stale ? "Last good reading" : "Updated"} {relTime(good.fetchedAt, clock.now)}</span>
      {/if}
    </span>
    <span class="tools">
      <button
        class="icon-btn"
        class:spin={reading}
        aria-label="Read {device.name} now"
        title="Read now"
        disabled={reading}
        onclick={() => proxmox.refresh(device.id)}
      >
        <Icon path={icons.refresh} size={16} />
      </button>
      {#if !showSetup}
        <button class="btn small" onclick={() => (setupOpen = true)}>Connection settings</button>
      {/if}
    </span>
  </div>

  {#if missingSecret && status?.error}
    <Problem problem={problem(`Reading Proxmox host "${device.name}" through its API`, status.error)} tone="warn" />
  {/if}
  {#if showSetup}
    <HostSetup {device} {problem} onclose={kind === "noToken" ? undefined : () => (setupOpen = false)} />
  {/if}

  {#if readError}
    <Problem problem={problem(`Reading Proxmox host "${device.name}"`, readError)} />
  {/if}
  {#if trustError}
    <Problem problem={problem(`Approving the certificate of "${device.name}"`, trustError)} message="Not approved: {trustError}" />
  {/if}

  {#if status && askCert}
    <CertCheck {device} {status} />
  {:else if status?.error && kind !== "noToken"}
    <Problem problem={problem(`Reading Proxmox host "${device.name}" through its API`, status.error)} />
  {/if}

  {#if !status && !readError}
    <p class="muted">Connecting to {device.name}…</p>
  {/if}

  {#if good}
    {@const overview = good.overview}
    <div class="layout" class:with-panel={!!selected}>
      <div class="main">
        <div class="content">
          <section class="nodes" aria-label="Nodes">
            {#each overview.nodes as node (node.name)}
              <NodeCard {node} {stale} />
            {:else}
              <p class="panel muted note">
                Proxmox shows no nodes to this token, so CPU, RAM and storage can't be shown. Give the token the
                PVEAuditor role on "/" (see Connection settings).
              </p>
            {/each}
          </section>
          <GuestTable deviceId={device.id} guests={overview.guests} selected={selectedKey} onselect={select} {stale} />
        </div>
      </div>
      {#if selected}
        <div class="side" bind:this={panelEl}>
          <GuestPanel {device} guest={selected} {problem} onclose={() => (selectedKey = null)} />
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .host { display: flex; flex-direction: column; gap: var(--gap); }
  .bar { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem 1rem; flex-wrap: wrap; margin-top: -0.6rem; }
  .tools { display: flex; align-items: center; gap: 0.4rem; }
  .name { color: var(--text-soft); font-weight: 600; font-size: 1rem; }
  /* Long explanations stay readable on a wide screen. */
  .host > :global(.alert) { max-width: 92rem; }
  p { margin: 0; }
  .note { line-height: 1.5; }

  .layout { display: flex; flex-direction: column; gap: var(--gap); }
  .main { min-width: 0; container-type: inline-size; }
  /* In one column the panel goes first, so opening it is never off screen. */
  .side { order: -1; min-width: 0; }
  @media (min-width: 1500px) {
    .layout.with-panel { display: grid; grid-template-columns: minmax(0, 1fr) 30rem; align-items: start; }
    .side { order: 0; position: sticky; top: 0; }
  }

  .content { display: grid; gap: var(--gap); grid-template-columns: minmax(0, 1fr); align-items: start; }
  .nodes { display: grid; gap: var(--gap); grid-template-columns: repeat(auto-fill, minmax(min(100%, 26rem), 1fr)); }
  /* Wide: the nodes become a column of their own beside the list. */
  @container (min-width: 110rem) {
    .content { grid-template-columns: 30rem minmax(0, 1fr); }
    .nodes { grid-template-columns: minmax(0, 1fr); }
  }
  @container (min-width: 200rem) {
    .content { grid-template-columns: 36rem minmax(0, 1fr); }
  }

  .spin :global(svg) { animation: spin 0.9s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) {
    .spin :global(svg) { animation: none; }
  }
</style>
