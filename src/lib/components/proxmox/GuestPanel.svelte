<script lang="ts">
  // One VM or container: start, resume, shut down, restart and force stop, its console, and its snapshots
  // (take, roll back, delete). Everything that stops it, rolls it back or deletes asks first (spec section 7).
  import { untrack } from "svelte";
  import { errorText, proxmoxApi, type Device, type GuestAddresses, type ProxmoxGuest, type SnapshotList } from "$lib/api";
  import { clock } from "$lib/clock.svelte";
  import { devices } from "$lib/devices.svelte";
  import { drafts } from "$lib/drafts.svelte";
  import { icons } from "$lib/icons";
  import { openTerminal } from "$lib/terminals.svelte";
  import { relTime, share, uptime, usedOfTotal } from "$lib/format";
  import {
    actionWords,
    consoleCommand,
    consoleTitle,
    devicesAt,
    guestId,
    guestKey,
    guestName,
    guestRef,
    isHibernated,
    isPaused,
    lockBlocks,
    lockLabel,
    powerActions,
    rollbackPlan,
    snapshotNameProblem,
    stateLabel,
    suggestedSnapshotName,
  } from "$lib/proxmox";
  import { proxmox } from "$lib/proxmox.svelte";
  import type { Problem as ProblemInfo } from "$lib/report";
  import Address from "../Address.svelte";
  import ConfirmButton from "../ConfirmButton.svelte";
  import Icon from "../Icon.svelte";
  import Meter from "../Meter.svelte";
  import Problem from "../Problem.svelte";

  let {
    device,
    guest,
    problem,
    onclose,
  }: {
    /** The Proxmox host. */
    device: Device;
    guest: ProxmoxGuest;
    problem: (what: string, error: string) => ProblemInfo;
    onclose: () => void;
  } = $props();

  const ref = $derived(guestRef(guest));
  const key = $derived(guestKey(ref));
  const busy = $derived(proxmox.busy.get(key) ?? null);
  const outcome = $derived(proxmox.outcomes.get(key) ?? null);
  const lock = $derived(lockLabel(guest.lock));
  const actions = $derived(powerActions(guest));
  const hibernated = $derived(isHibernated(guest));
  const ram = $derived(share(guest.memUsed, guest.memTotal));
  const disk = $derived(share(guest.diskUsed, guest.diskTotal));

  let snaps = $state<SnapshotList | null>(null);
  let snapsFor = $state<string | null>(null);
  let snapsError = $state<string | null>(null);
  let loadingSnaps = $state(false);
  let snapName = $state("");
  let snapNote = $state("");
  let withRam = $state(false);
  let triedName = $state(false);
  /** The name offered when the form was filled, to tell whether anything was typed. */
  let suggested = $state("");
  const nameProblem = $derived(snapshotNameProblem(snapName));

  // A snapshot being written is kept while another screen is shown (drafts.svelte.ts), for each guest.
  type SnapshotForm = { name: string; note: string; withRam: boolean };
  const draftKey = $derived(`proxmox-snapshot:${device.id}:${key}`);
  $effect(() => {
    if (snapsFor !== key) return;
    drafts.keep<SnapshotForm>(draftKey, { name: snapName, note: snapNote, withRam }, { name: suggested, note: "", withRam: false });
  });

  function close() {
    drafts.clear(draftKey);
    onclose();
  }

  async function loadSnapshots(forKey: string) {
    loadingSnaps = true;
    try {
      const list = await proxmoxApi.snapshots(device.id, ref);
      if (forKey !== key) return;
      snaps = list;
      snapsError = null;
    } catch (e) {
      if (forKey === key) snapsError = errorText(e);
    } finally {
      if (forKey === key) loadingSnaps = false;
    }
  }

  // A different guest: start again with its own snapshots and a fresh name, or what was being written for it.
  $effect(() => {
    if (snapsFor === key) return;
    snapsFor = key;
    snaps = null;
    snapsError = null;
    const draft = drafts.get<SnapshotForm>(untrack(() => draftKey));
    suggested = suggestedSnapshotName(new Date());
    snapName = draft?.name ?? suggested;
    snapNote = draft?.note ?? "";
    withRam = draft?.withRam ?? false;
    triedName = false;
    if (!guest.template) loadSnapshots(key);
  });

  async function snapshotJob(busyText: string, done: string, job: () => Promise<{ finished: boolean }>, pending?: string) {
    const forKey = key;
    const ok = await proxmox.act(device.id, ref, busyText, done, job, pending);
    if (forKey === key) await loadSnapshots(forKey);
    return ok;
  }

  async function take(e: SubmitEvent) {
    e.preventDefault();
    triedName = true;
    if (nameProblem) return;
    const name = snapName.trim();
    const forKey = key;
    const takenDraft = draftKey;
    const ok = await snapshotJob("Taking snapshot…", `Snapshot "${name}" taken.`, () =>
      proxmoxApi.takeSnapshot(device.id, ref, name, snapNote, withRam && guest.kind === "qemu"),
    );
    if (ok) drafts.clear(takenDraft);
    // Another guest may be open by now: its own form stays as it is.
    if (ok && forKey === key) {
      suggested = suggestedSnapshotName(new Date());
      snapName = suggested;
      snapNote = "";
      triedName = false;
    }
  }

  /** Proxmox leaves a guest it stopped to roll back stopped; plan.startAfter starts it again (rollbackPlan). */
  function rollback(name: string, plan: ReturnType<typeof rollbackPlan>) {
    const pending = plan.startAfter
      ? `Proxmox is still rolling back. If ${guestName(guest)} isn't running once it's done, start it here.`
      : undefined;
    return snapshotJob("Rolling back…", plan.done, () => proxmoxApi.rollback(device.id, ref, name, plan.startAfter), pending);
  }

  /** "Taken 9 days ago, 08/11/2026 06:42", or just the date and time once it is too long ago for "ago". */
  function takenText(at: number | null): string {
    if (at == null) return "Time not known";
    const when = new Date(at * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
    const rel = relTime(at, clock.now);
    return /ago|now|yesterday/.test(rel) ? `Taken ${rel}, ${when}` : `Taken ${when}`;
  }

  // --- Console: a terminal on the Proxmox host over SSH that runs pct enter (a container) or qm terminal (a
  // VM's serial console), or, for a VM whose address is a machine in home-cc, a terminal on that machine.
  const running = $derived(guest.status === "running" && !guest.template);
  /** The Proxmox host's own SSH login, which the console needs; the API token is separate. */
  const sshUser = $derived(device.sshUser?.trim() || null);
  const command = $derived(consoleCommand(guest));

  let addrs = $state<GuestAddresses | null>(null);
  let addrsFor = $state<string | null>(null);
  let addrsError = $state<string | null>(null);
  const atAddresses = $derived(devicesAt(addrs?.addresses ?? [], devices.list));

  async function loadAddresses(forKey: string) {
    try {
      const found = await proxmoxApi.guestAddresses(device.id, ref);
      if (forKey !== addrsFor) return;
      addrs = found;
      addrsError = null;
    } catch (e) {
      if (forKey === addrsFor) addrsError = errorText(e);
    }
  }

  // Asked once for each guest while it runs (and again when it starts).
  $effect(() => {
    const want = running ? key : null;
    if (want === addrsFor) return;
    addrsFor = want;
    addrs = null;
    addrsError = null;
    if (want) loadAddresses(want);
  });

  function openConsole() {
    if (command && sshUser) void openTerminal({ deviceId: device.id, title: consoleTitle(guest), command });
  }

  const newestFirst = $derived(snaps ? [...snaps.snapshots].reverse() : []);
  const canSnapshot = $derived(!guest.template);
</script>

<aside class="panel side" aria-label="{guestName(guest)} details">
  <header>
    <div class="title">
      <h2 class="ellipsis" title={guestName(guest)}>{guestName(guest)}</h2>
      <span class="muted small">{guestId(guest)} on {guest.node}</span>
    </div>
    <button class="btn small" onclick={close}>Close</button>
  </header>

  <div class="state">
    <span class="dot {guest.template ? 'idle' : guest.status === 'running' ? 'ok' : isPaused(guest) || hibernated ? 'warn' : 'bad'}" aria-hidden="true"></span>
    <strong>{stateLabel(guest)}</strong>
    {#if guest.uptimeSecs != null}<span class="muted">up {uptime(guest.uptimeSecs)}</span>{/if}
    {#if lock}<span class="lock">{lock}</span>{/if}
  </div>

  {#if guest.status === "running"}
    <div class="meter-grid">
      <Meter label="CPU" value={guest.cpuPct} detail={guest.cpuCount ? `${guest.cpuCount} cores` : ""} />
      <Meter label="RAM" value={ram} detail={guest.memUsed != null && guest.memTotal != null ? usedOfTotal(guest.memUsed, guest.memTotal) : ""} />
      {#if guest.kind === "lxc"}
        <Meter label="Disk" value={disk} detail={guest.diskUsed != null && guest.diskTotal != null ? usedOfTotal(guest.diskUsed, guest.diskTotal) : ""} />
      {/if}
    </div>
  {/if}

  {#if guest.template}
    <p class="muted">A template can't be started and has no snapshots. Clone it in Proxmox to make a VM from it.</p>
  {:else}
    <section class="power" aria-label="Power">
      <div class="buttons">
        <!-- Drawn afresh for another guest, so a first click given for one never carries over to the next.
             While Proxmox is busy with the guest it refuses all of these (an API token can't skip its lock),
             except starting a hibernated VM, which is how it is resumed. -->
        {#key key}
        {#each actions as action (action)}
          {@const words = actionWords(guest, action)}
          {@const held = lockBlocks(guest, action)}
          {@const title = held ? `Wait until Proxmox is done: ${held}` : undefined}
          {#if words.confirm}
            <ConfirmButton
              label={words.label}
              confirmLabel={words.confirm}
              disabled={!!busy || !!held}
              {title}
              onconfirm={() => proxmox.power(device.id, guest, action)}
            />
          {:else}
            <button class="btn small" disabled={!!busy || !!held} {title} onclick={() => proxmox.power(device.id, guest, action)}>
              {words.label}
            </button>
          {/if}
        {/each}
        {/key}
      </div>
      {#if actions.includes("shutdown")}
        <p class="muted small">
          Shut down asks the system inside to close down properly. Force stop is like pulling the plug: anything not
          saved inside is lost.
        </p>
      {:else if actions.includes("resume")}
        <p class="muted small">
          Paused: it is halted where it was, still in memory. Resume carries on from there. Force stop is like pulling
          the plug: anything not saved inside is lost.
        </p>
      {:else if hibernated}
        <p class="muted small">Hibernated: its memory is saved to disk. Resume starts it again where it left off.</p>
      {/if}
      {#if busy}<p class="muted small" role="status">{busy}</p>{/if}
    </section>
  {/if}

  {#if outcome && !busy}
    {#if outcome.ok}
      <p class="done small" role="status">{outcome.text}</p>
    {:else}
      <Problem problem={problem(`Changing ${guestId(guest)} ("${guestName(guest)}")`, outcome.text)} />
    {/if}
  {/if}

  {#if running}
    <section class="console" aria-label="Console">
      <h3 class="section-title">Console</h3>
      {#if addrsError}
        <Problem tone="warn" problem={problem(`Reading the address of ${guestId(guest)} ("${guestName(guest)}")`, addrsError)} />
      {:else if addrs?.unknownWhy}
        <p class="muted small">{addrs.unknownWhy}</p>
      {:else if atAddresses.length}
        <ul class="addrs" aria-label="Addresses">
          {#each atAddresses as at (at.ip)}
            <li>
              <Address host={at.ip} known={at.device?.hostname ?? null} />
              {#if at.device}
                <span class="muted small">
                  · {at.device.name} in home-cc{#if !at.device.sshUser?.trim()}, with no SSH user:
                    <a href="/machines?edit={at.device.id}">add one</a>{guest.kind === "qemu" ? " to open a terminal on it" : ""}{/if}
                </span>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
      <div class="buttons">
        <button
          class="btn small"
          disabled={!sshUser || !command}
          title={sshUser ? `A terminal on ${device.name} that runs ${command}` : `Needs an SSH user for ${device.name}`}
          onclick={openConsole}
        >
          <Icon path={icons.terminal} size={15} />
          {guest.kind === "lxc" ? "Open console" : "Serial console"}
        </button>
        {#if guest.kind === "qemu"}
          {#each atAddresses as at (at.ip)}
            {#if at.device?.sshUser?.trim()}
              {@const target = at.device}
              <button class="btn small" title="A terminal on {target.name} over its own SSH login" onclick={() => openTerminal({ deviceId: target.id, title: target.name })}>
                <Icon path={icons.terminal} size={15} /> Terminal on {target.name}
              </button>
            {/if}
          {/each}
        {/if}
      </div>
      {#if !sshUser}
        <p class="muted small">
          The console is a terminal on {device.name} over SSH, which needs an SSH user for it (the API token is only for
          Proxmox's web API). <a href="/machines?edit={device.id}">Add an SSH user to {device.name}</a>
        </p>
      {:else}
        {#if guest.kind === "qemu"}
          <p class="muted small">The serial console needs a serial port set up on the VM. Ctrl+O leaves it.</p>
        {/if}
        {#if sshUser !== "root"}
          <p class="muted small">
            {guest.kind === "lxc" ? "pct" : "qm"} needs root on {device.name}, and home-cc logs in there as {sshUser}, so it
            may be refused.
          </p>
        {/if}
      {/if}
    </section>
  {/if}

  {#if canSnapshot}
    <section class="snaps" aria-label="Snapshots">
      <h3 class="section-title">Snapshots</h3>
      <form class="take" onsubmit={take}>
        <label>Name
          <input
            class="mono"
            bind:value={snapName}
            aria-invalid={triedName && !!nameProblem}
            autocomplete="off"
            spellcheck="false"
            maxlength="40"
          />
        </label>
        {#if triedName && nameProblem}<p class="bad small">{nameProblem}</p>{/if}
        <label>Note (optional) <input bind:value={snapNote} placeholder="Before the update" /></label>
        {#if guest.kind === "qemu" && guest.status === "running"}
          <label class="check">
            <input type="checkbox" bind:checked={withRam} /> Include RAM, so rolling back returns it running
          </label>
        {/if}
        <div>
          <button class="btn primary small" type="submit" disabled={!!busy}>Take snapshot</button>
        </div>
      </form>

      {#if snapsError}
        <Problem problem={problem(`Listing the snapshots of ${guestId(guest)} ("${guestName(guest)}")`, snapsError)} />
      {:else if !snaps}
        <p class="muted small">{loadingSnaps ? "Loading snapshots…" : ""}</p>
      {:else if snaps.snapshots.length === 0}
        <p class="muted small">No snapshots yet.</p>
      {:else}
        <ul class="list">
          {#each newestFirst as s (s.name)}
            {@const plan = rollbackPlan(guest, s)}
            <li>
              <div class="snap-head">
                <span class="mono name ellipsis" title={s.name}>{s.name}</span>
                {#if snaps.currentParent === s.name}<span class="here small" title="The present state comes from this snapshot">now based on this</span>{/if}
                {#if s.withRam}<span class="muted small">with RAM</span>{/if}
              </div>
              <div class="muted small">{takenText(s.takenAt)}</div>
              {#if s.description}<p class="note small">{s.description}</p>{/if}
              <div class="snap-actions">
                {#key s.name}
                  <ConfirmButton
                    label="Roll back"
                    confirmLabel={plan.confirm}
                    disabled={!!busy}
                    onconfirm={() => rollback(s.name, plan)}
                  />
                  <ConfirmButton
                    label="Delete"
                    confirmLabel="Yes, delete"
                    disabled={!!busy}
                    onconfirm={() =>
                      snapshotJob("Deleting snapshot…", `Snapshot "${s.name}" deleted.`, () => proxmoxApi.deleteSnapshot(device.id, ref, s.name))}
                  />
                {/key}
              </div>
            </li>
          {/each}
        </ul>
        <p class="muted small">
          Rolling back puts {guestName(guest)} back as it was when the snapshot was taken: everything changed since is
          lost. Deleting a snapshot leaves {guestName(guest)} as it is now.
        </p>
        {#if guest.status === "running" || isPaused(guest)}
          <p class="muted small">
            {guestName(guest)} is running. To roll back to a snapshot without RAM, Proxmox stops it, and home-cc starts
            it again afterwards.{snaps.snapshots.some((s) => s.withRam) ? " A snapshot with RAM brings it back running as it was then." : ""}
          </p>
        {:else if snaps.snapshots.some((s) => s.withRam)}
          <p class="muted small">A snapshot with RAM brings {guestName(guest)} back running as it was then.</p>
        {/if}
      {/if}
    </section>
  {/if}
</aside>

<style>
  .side { display: flex; flex-direction: column; gap: 0.9rem; min-width: 0; }
  header { display: flex; align-items: flex-start; justify-content: space-between; gap: 0.8rem; }
  .title { display: flex; flex-direction: column; gap: 0.1rem; min-width: 0; }
  h2 { margin: 0; font-size: 1.2rem; font-weight: 600; }
  .state { display: flex; align-items: center; gap: 0.5rem 0.7rem; flex-wrap: wrap; }
  .lock { color: var(--warn); }
  p { margin: 0; line-height: 1.5; }
  .power { display: flex; flex-direction: column; gap: 0.5rem; }
  .buttons { display: flex; gap: 0.4rem; flex-wrap: wrap; }
  .done { color: var(--ok); }
  .bad { color: var(--bad); margin-top: -0.5rem; }

  .console { display: flex; flex-direction: column; gap: 0.55rem; border-top: 1px solid var(--border); padding-top: 0.9rem; }
  .console .section-title { margin: 0; }
  .console .btn { display: inline-flex; align-items: center; gap: 0.4rem; }
  .addrs { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.3rem; }
  .addrs li { display: flex; align-items: baseline; flex-wrap: wrap; gap: 0.2rem 0.7rem; min-width: 0; }

  .snaps { display: flex; flex-direction: column; gap: 0.8rem; border-top: 1px solid var(--border); padding-top: 0.9rem; }
  .snaps .section-title { margin: 0; }
  .take { display: flex; flex-direction: column; gap: 0.7rem; }
  .check { flex-direction: row; align-items: center; gap: 0.5rem; cursor: pointer; }
  .check input { width: 1.15rem; min-height: 0; height: 1.15rem; padding: 0; accent-color: var(--accent); flex: none; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.5rem; }
  .list li { display: flex; flex-direction: column; gap: 0.25rem; padding: 0.6rem 0.7rem; border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--bg-sunk); }
  .snap-head { display: flex; align-items: baseline; gap: 0.6rem; min-width: 0; }
  .name { font-weight: 500; }
  .here { color: var(--accent); white-space: nowrap; }
  .note { color: var(--text-soft); white-space: pre-line; overflow-wrap: anywhere; }
  .snap-actions { display: flex; gap: 0.4rem; flex-wrap: wrap; margin-top: 0.25rem; }
</style>
