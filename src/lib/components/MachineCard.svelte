<script lang="ts">
  // One machine on the Machines screen: status, system, and bars for CPU, RAM
  // and each disk, or a plain explanation of why there are none.
  import { api, KIND_LABELS, OS_LABELS, secretKeys, type Device, type Disk, type HostKey } from "$lib/api";
  import { clock } from "$lib/clock.svelte";
  import { ms, relTime, share, uptime, usedOfTotal } from "$lib/format";
  import { identityCommand, identityText } from "$lib/hostkey";
  import { icons } from "$lib/icons";
  import { keySetupAsk } from "$lib/keysetup.svelte";
  import { settings } from "$lib/settings.svelte";
  import { stats } from "$lib/stats.svelte";
  import { status } from "$lib/status.svelte";
  import { systemLabel, systemTitle } from "$lib/system";
  import { deviceDetails } from "$lib/problems.svelte";
  import { openTerminal } from "$lib/terminals.svelte";
  import type { Details, Problem as ProblemInfo } from "$lib/report";
  import Address from "./Address.svelte";
  import ConfirmButton from "./ConfirmButton.svelte";
  import Problem from "./Problem.svelte";
  import Icon from "./Icon.svelte";
  import Meter from "./Meter.svelte";
  import StatusDot from "./StatusDot.svelte";
  import type { Snippet } from "svelte";

  /** `handle`: the grip the screen puts at the start of the header, to move the card by. `tools`: more
   *  buttons from the screen, put before Edit (the group menu). */
  let {
    device,
    onedit,
    handle,
    tools,
  }: { device: Device; onedit: (d: Device) => void; handle?: Snippet; tools?: Snippet } = $props();

  const s = $derived(status.get(device));
  const entry = $derived(stats.get(device.id));
  const st = $derived(entry?.stats ?? null);
  const refreshing = $derived(stats.refreshing.has(device.id));
  const refreshError = $derived(stats.refreshErrors.get(device.id) ?? null);
  /** The identity the machine presented, when it is waiting for the owner's approval. */
  const offered = $derived(entry?.offeredKey ?? null);
  const trusting = $derived(stats.trusting.has(device.id));
  const trustError = $derived(stats.trustErrors.get(device.id) ?? null);
  const ram = $derived(share(st?.memUsedKb, st?.memTotalKb));
  const ramWarn = $derived(ram != null && ram > settings.value.ramWarnPct);
  /** This PC: read here without SSH, so no SSH user, login or identity is involved. */
  const local = $derived(stats.local(device));
  /** An appliance with no SSH: up or down is all there is to show, and nothing is missing. */
  const statusOnly = $derived(device.kind === "appliance" && !device.sshUser && !local);

  /** What a problem on this card hands to an agent, besides the message. */
  function machineProblem(area: ProblemInfo["area"], what: string, error: string): ProblemInfo {
    return {
      area,
      what,
      error,
      details: {
        ...deviceDetails(device),
        "Kind of error": entry?.errorKind,
        "Retrying by itself": entry?.error ? !entry.paused : null,
        "OS the machine reported": st?.osName,
        "Last good reading": entry?.collectedAt ? relTime(entry.collectedAt, clock.now) : "none yet",
        "Identity stored for it": entry && !local ? !!entry.pinnedFingerprint : null,
        "Read on this PC without SSH": local,
      },
    };
  }

  /** Read only when a prompt is made: whether the login could have used a stored password. */
  async function passwordDetails(): Promise<Details> {
    return { "Password stored for it": await api.hasSecret(secretKeys.sshPassword(device.id)) };
  }

  function diskLabel(d: Disk): string {
    // Windows volumes have a label worth showing; on Linux it is a device name, kept for the tooltip.
    return d.mount.endsWith(":") && d.label ? `${d.mount} ${d.label}` : d.mount;
  }
  function diskHint(d: Disk): string {
    return d.label && !d.mount.endsWith(":") ? `${d.mount} (${d.label})` : diskLabel(d);
  }
</script>

{#snippet howToCheck(key: HostKey, same: string)}
  <p class="muted small">
    To check, run <span class="mono cmd">{identityCommand(key.algorithm, device.os)}</span> on the machine itself. It
    prints a line with the same SHA256 text as {same}.
  </p>
{/snippet}

<!-- Approves exactly the identity on screen. Drawn afresh when that changes, so a half-given confirmation
     never carries over to an identity the owner hasn't read. Held back while the machine is being read:
     that reading may come back with another identity, or none to approve. -->
{#snippet approve(key: HostKey, label: string)}
  <div class="confirm">
    {#key key.fingerprint}
      <ConfirmButton
        {label}
        confirmLabel="Yes, it matches"
        disabled={refreshing || trusting}
        onconfirm={() => stats.trust(device.id, key.fingerprint)}
      />
    {/key}
  </div>
{/snippet}

<article class="panel card">
  <header>
    {@render handle?.()}
    <StatusDot health={status.health(device)} />
    <h3 class="ellipsis" title={device.name}>{device.name}</h3>
    {#if s?.up && s.rttMs != null}
      <span class="mono muted small">{ms(s.rttMs)}</span>
    {:else if s?.up && status.isSelf(device)}
      <span class="muted small self" title="home-cc runs on this PC">this PC</span>
    {/if}
    {#if s && !s.up}<span class="down small">down</span>{/if}
    <span class="tools">
      {#if device.sshUser || local}
        <button
          class="icon-btn"
          class:spin={refreshing}
          aria-label="Read stats for {device.name} now"
          title={local ? "Read this PC's stats now" : "Read stats now"}
          disabled={refreshing || trusting}
          onclick={() => stats.refresh(device.id)}
        >
          <Icon path={icons.refresh} size={16} />
        </button>
        <!-- Terminals -->
        <button
          class="icon-btn"
          aria-label="Open a terminal on {device.name}"
          title="Open a terminal"
          onclick={() => openTerminal({ deviceId: device.id, title: device.name })}
        >
          <Icon path={icons.terminal} size={16} />
        </button>
      {/if}
      {@render tools?.()}
      <button class="icon-btn" aria-label="Edit {device.name}" title="Edit" onclick={() => onedit(device)}>
        <Icon path={icons.edit} size={16} />
      </button>
    </span>
  </header>

  <div class="sub muted small">
    <Address host={device.host} known={device.hostname} />
    <!-- The System's name, or the one the machine gave; where home-cc's System came from on hover. -->
    <span class="ellipsis" title="{st?.osName ? `${st.osName}. ` : ''}{systemTitle(device, OS_LABELS)}">{st?.osName || (statusOnly && device.os === "other" ? KIND_LABELS.appliance : systemLabel(device, OS_LABELS))}</span>
    {#if st?.uptimeSecs != null}<span class="nowrap">up {uptime(st.uptimeSecs)}</span>{/if}
  </div>

  {#if refreshError}
    <Problem
      problem={machineProblem("stats", `Reading stats from "${device.name}" on request`, refreshError)}
      message="Couldn't read stats: {refreshError}"
      extra={local ? undefined : passwordDetails}
    />
  {/if}
  <!-- Not inside the approval block: after a refusal the machine may have nothing left to approve. -->
  {#if trustError}
    <Problem
      problem={machineProblem("trust", `Approving the identity of "${device.name}"`, trustError)}
      message="Not approved: {trustError}"
    />
  {/if}

  {#if statusOnly}
    <div class="state">
      <p class="muted" title="Only checked for being up. One you can log in to over SSH (a NAS box, say) can show stats too: add an SSH user with Edit.">
        {s ? `${s.up ? "Up" : "Down"}, last seen ${relTime(s.lastSeen ?? device.lastSeen, clock.now)}` : "Waiting for the first check…"}
      </p>
    </div>
  {:else if !device.sshUser && !local}
    <div class="state">
      <p class="muted">Add an SSH user to see stats.</p>
      <button class="btn small" onclick={() => onedit(device)}>Add SSH user</button>
    </div>
  {:else if entry?.errorKind === "hostKeyChanged" && offered}
    <div class="alert warn" role="alert">
      <p><strong>This machine's identity has changed.</strong></p>
      <p>
        That is normal if you reinstalled or replaced it. If you didn't, something else may be answering at this
        address. Nothing is sent to it until you approve the new identity.
      </p>
      <dl class="prints">
        <dt>Was</dt>
        <dd class="mono">{entry.expectedFingerprint ?? "not known"}</dd>
        <dt>Now</dt>
        <dd class="mono">{identityText(offered)}</dd>
      </dl>
      {@render howToCheck(offered, '"Now"')}
      {@render approve(offered, "Approve the new identity")}
    </div>
  {:else if entry?.errorKind === "newHost" && offered}
    <div class="alert ask" role="status">
      <p><strong>Before the stored password is used for {device.name}, check its identity.</strong></p>
      <p>The machine shows this identity:</p>
      <p class="mono print">{identityText(offered)}</p>
      <p>
        Check it matches, then approve to use the stored password. The password has not been sent, and won't be
        until you approve.
      </p>
      {@render howToCheck(offered, "the identity above")}
      {@render approve(offered, "Approve and use the password")}
    </div>
  {:else if entry?.error && local}
    <Problem problem={machineProblem("stats", `Reading this PC's own stats ("${device.name}"), without SSH`, entry.error)} />
  {:else if entry?.error}
    <Problem
      problem={machineProblem("stats", `Reading stats from "${device.name}" over SSH`, entry.error)}
      extra={passwordDetails}
    />
    {#if entry.errorKind === "auth"}
      <!-- Opens the form on this machine with its key login part open (components/KeySetup.svelte). -->
      <div class="state">
        <p class="muted small">A key login needs no password, and home-cc can set it up for you.</p>
        <button class="btn small" onclick={() => { keySetupAsk.ask(device.id); onedit(device); }}>Set up key login</button>
      </div>
    {/if}
    {#if entry.paused}
      <p class="muted small paused">
        home-cc has stopped trying this machine by itself, so it doesn't keep knocking with a login that fails. Fix the
        login with Edit, or press the refresh button above to try again.
      </p>
    {/if}
  {:else if !st}
    <div class="state">
      <p class="muted">{refreshing ? "Reading…" : "Waiting for the first reading…"}</p>
    </div>
  {/if}

  {#if (device.sshUser || local) && st}
    <div class="meter-grid" class:stale={!!entry?.error}>
      {#if st.cpuPct != null}
        <Meter label="CPU" value={st.cpuPct} detail={st.cpuCount ? `${st.cpuCount} cores` : ""} />
      {:else if st.load1 != null}
        <Meter
          label="CPU"
          hint="Load average over the last minute. A percentage follows with the next reading."
          value={null}
          text="–"
          detail="load {st.load1.toFixed(2)}"
        />
      {:else}
        <Meter label="CPU" value={null} text="–" />
      {/if}

      {#if st.memTotalKb != null && st.memUsedKb != null}
        <Meter
          label="RAM"
          value={ram}
          detail={usedOfTotal(st.memUsedKb * 1024, st.memTotalKb * 1024)}
          warn={ramWarn ? `Above your ${settings.value.ramWarnPct}% warning level` : null}
        />
      {:else}
        <Meter label="RAM" value={null} text="–" />
      {/if}

      <!-- No key: the mount point is the machine's own text, and two disks can report the same one. -->
      {#each st.disks as d}
        {@const used = share(d.usedBytes, d.totalBytes)}
        <Meter
          label={diskLabel(d)}
          hint={diskHint(d)}
          mono
          value={used}
          detail={usedOfTotal(d.usedBytes, d.totalBytes)}
          warn={used != null && used > settings.value.diskWarnPct ? `Nearly full: above your ${settings.value.diskWarnPct}% warning level` : null}
        />
      {/each}
    </div>
    <footer class="muted small">
      <span>
        {entry?.error ? "Last good reading" : "Updated"} {relTime(entry?.collectedAt, clock.now)}
        {#if local}<span title="home-cc runs on this PC, so it reads the stats here, without SSH"> · read locally</span>{/if}
      </span>
      {#if st.reduced}
        <span title="Sign in with an administrator account on that PC to get every value">
          Limited readings: this Windows account can't see everything.
        </span>
      {/if}
    </footer>
  {/if}
</article>

<style>
  .card { display: flex; flex-direction: column; gap: 0.7rem; padding: 0.9rem 1rem 1rem; min-width: 0; }
  header { display: flex; align-items: center; gap: 0.6rem; min-width: 0; }
  h3 { margin: 0; font-size: 1.08rem; font-weight: 600; }
  .tools { display: flex; margin-left: auto; margin-right: -0.4rem; }
  .down { color: var(--bad); font-weight: 600; }
  .self { flex: none; white-space: nowrap; }
  .sub { display: flex; gap: 0.25rem 0.9rem; min-width: 0; margin-top: -0.35rem; }
  .nowrap { white-space: nowrap; flex: none; }
  /* The system's name gives way before the hostname does. */
  .sub .ellipsis { flex: 1 1 0; }

  .state { display: flex; align-items: center; justify-content: space-between; gap: 0.75rem; flex-wrap: wrap; min-height: 2.25rem; }
  p { margin: 0; line-height: 1.5; }
  .alert p + p { margin-top: 0.3rem; }
  .confirm { display: flex; gap: 0.4rem; flex-wrap: wrap; margin-top: 0.6rem; }
  /* Identities are there to be compared and copied: always selectable, and wrapped rather than cut. */
  .prints { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 0.2rem 0.7rem; margin: 0.5rem 0; }
  .prints dt { color: var(--muted); }
  .prints dd { margin: 0; }
  .prints dd, .print, .cmd { font-size: 0.92rem; overflow-wrap: anywhere; user-select: text; }
  .alert .print { margin: 0.2rem 0 0.4rem; }
  .cmd { color: var(--text-soft); font-size: 0.92em; }
  .paused { margin-top: -0.2rem; }

  /* Readings from before the latest try failed: still shown, but clearly old. */
  .stale { opacity: 0.6; }
  footer { display: flex; flex-wrap: wrap; gap: 0.2rem 1rem; justify-content: space-between; margin-top: auto; }

  .spin :global(svg) { animation: spin 0.9s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) {
    .spin :global(svg) { animation: none; }
  }
</style>
