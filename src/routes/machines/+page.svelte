<script lang="ts">
  import { tick, untrack } from "svelte";
  import { page } from "$app/state";
  import { devices, isMachine } from "$lib/devices.svelte";
  import { stats } from "$lib/stats.svelte";
  import {
    api,
    errorText,
    KIND_LABELS,
    OS_LABELS,
    secretKeys,
    type Device,
    type DeviceInput,
    type DeviceKind,
    type Os,
  } from "$lib/api";
  import { icons } from "$lib/icons";
  import { isIpv4, isIpv6 } from "$lib/lookups";
  import { macToSend, repointed } from "$lib/net";
  import Address from "$lib/components/Address.svelte";
  import ConfirmButton from "$lib/components/ConfirmButton.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import MachineCard from "$lib/components/MachineCard.svelte";
  import Problem from "$lib/components/Problem.svelte";
  import { deviceDetails } from "$lib/problems.svelte";
  import type { Problem as ProblemInfo } from "$lib/report";

  const blank = (): DeviceInput => ({ name: "", host: "", kind: "vm", os: "linux", sshUser: "", sshPort: 22, notes: "", mac: "" });

  let editing = $state<number | "new" | null>(null);
  let form = $state<DeviceInput>(blank());
  /** The address, SSH port and hardware address as they are saved, to tell what the form changes. */
  let stored = $state<{ host: string; sshPort: number; mac: string | null }>({ host: "", sshPort: 22, mac: null });
  let formError = $state<ProblemInfo | null>(null);
  let listError = $state<ProblemInfo | null>(null);

  /** A problem with the form. Says which machine and what was typed, but never the password. */
  function formProblem(area: ProblemInfo["area"], what: string, message: string): ProblemInfo {
    const d = typeof editing === "number" ? devices.byId.get(editing) : undefined;
    return {
      area,
      what,
      error: message,
      details: {
        ...(d ? deviceDetails(d) : { Device: "a new machine" }),
        "Typed in the form": JSON.stringify({ ...form, password: undefined }),
        "A password was typed": !!password,
      },
    };
  }
  let saving = $state(false);
  /** Typed into the form; goes to the keychain on save and is never read back. */
  let password = $state("");
  let hasPassword = $state(false);
  let showAll = $state(false);
  let formEl = $state<HTMLFormElement>();

  const byName = (a: Device, b: Device) => a.name.localeCompare(b.name, undefined, { numeric: true });
  const machines = $derived(devices.list.filter((d) => d.kind !== "ignore" && isMachine(d)).sort(byName));
  const rest = $derived(devices.list.filter((d) => d.kind === "ignore" || !isMachine(d)).sort(byName));
  const rows = $derived(showAll ? [...machines, ...rest] : machines);

  async function open(id: number | "new", input: DeviceInput) {
    form = input;
    typedHost = input.host.trim();
    stored = { host: input.host, sshPort: input.sshPort ?? 22, mac: input.mac || null };
    formError = null;
    password = "";
    hasPassword = false;
    editing = id;
    if (typeof id === "number") {
      api.hasSecret(secretKeys.sshPassword(id)).then(
        (stored) => {
          if (editing === id) hasPassword = stored;
        },
        (e) => {
          if (editing === id) formError = formProblem("secrets", "Opening a machine to edit it", `Couldn't check for a stored password: ${errorText(e)}`);
        },
      );
    }
    // Below 1500px the form sits above the cards, which may be scrolled out of view.
    await tick();
    formEl?.scrollIntoView({ block: "nearest" });
  }

  function startAdd() {
    open("new", blank());
  }

  function startEdit(d: Device) {
    open(d.id, { name: d.name, host: d.host, kind: d.kind, os: d.os, sshUser: d.sshUser ?? "", sshPort: d.sshPort, notes: d.notes, mac: d.mac ?? "" });
  }

  // Links from other screens: /machines?add=1 and /machines?edit=<id>.
  let handled: string | null = null;
  $effect(() => {
    const query = page.url.search;
    if (query === handled) return;
    const params = page.url.searchParams;
    if (params.has("add")) {
      handled = query;
      untrack(startAdd);
    } else if (params.has("edit")) {
      const d = devices.byId.get(Number(params.get("edit")));
      if (d) untrack(() => startEdit(d));
      if (d || devices.loaded) handled = query;
    } else {
      handled = query;
    }
  });

  async function save(e: SubmitEvent) {
    e.preventDefault();
    saving = true;
    formError = null;
    try {
      // The hardware address: nothing when the box wasn't touched (kept), "" when it was emptied (removed).
      const input = { ...form, sshPort: Number(form.sshPort) || 22, mac: macToSend(stored.mac, form.mac ?? "") };
      let saved: Device;
      if (typeof editing === "number") {
        const id = editing;
        if (password && !repointed(stored, input.host, input.sshPort)) {
          // Still the same machine: the new password goes in first. Saving the machine makes home-cc try
          // its login again straight away, and that try must not knock once more with the old password.
          try {
            await api.setSecret(secretKeys.sshPassword(id), password);
          } catch (err) {
            formError = formProblem("secrets", "Saving a machine with a new password", `Nothing was saved, because the password couldn't be stored: ${errorText(err)}`);
            return;
          }
          hasPassword = true;
          password = "";
          try {
            saved = await devices.update(id, input);
          } catch (err) {
            formError = formProblem("devices", "Saving a machine with a new password", `The new password was stored, but the rest wasn't saved: ${errorText(err)}`);
            return;
          }
        } else {
          // Pointed at another address or port: the machine is saved first. Storing a password starts a
          // login at once, and with the old address still saved it would be offered to the machine this
          // entry is moving away from. Saved first, home-cc forgets the identity it had approved, and
          // sends no password before the owner has approved what answers at the new place.
          saved = await devices.update(id, input);
          if (password && !(await storePassword(saved))) return;
        }
      } else if (editing === "new") {
        saved = await devices.add(input);
        if (password && !(await storePassword(saved))) return;
      } else {
        return;
      }
      editing = null;
      password = "";
      // Saving a machine or its password makes the back end read it again by itself, and the result
      // arrives as a "stats-changed" event; this only clears what the old entry showed.
      stats.load();
    } catch (err) {
      formError = formProblem("devices", "Saving a machine", errorText(err));
    } finally {
      saving = false;
    }
  }

  /** Stores the typed password for a machine that has just been saved. If that fails the form stays
   *  open on the machine, as it is now saved, so the password can be tried again. */
  async function storePassword(saved: Device): Promise<boolean> {
    try {
      await api.setSecret(secretKeys.sshPassword(saved.id), password);
      return true;
    } catch (err) {
      editing = saved.id;
      stored = { host: saved.host, sshPort: saved.sshPort, mac: saved.mac };
      form.mac = saved.mac ?? "";
      formError = formProblem("secrets", "Storing the password for a machine just saved", `Saved, but the password couldn't be stored: ${errorText(err)}`);
      return false;
    }
  }

  async function removePassword(id: number) {
    try {
      await api.deleteSecret(secretKeys.sshPassword(id));
      hasPassword = false;
    } catch (err) {
      formError = formProblem("secrets", "Removing a machine's stored password", `Couldn't remove the password: ${errorText(err)}`);
    }
  }

  async function remove(d: Device) {
    listError = null;
    try {
      await devices.remove(d.id);
      if (editing === d.id) editing = null;
    } catch (err) {
      listError = { area: "devices", what: `Removing "${d.name}" on the Machines screen`, error: `Couldn't remove "${d.name}": ${errorText(err)}`, details: deviceDetails(d) };
    }
  }

  /** The SSH identity stored for the machine in the form, for the owner to compare with the machine's own. */
  const pinned = $derived(typeof editing === "number" ? (stats.get(editing)?.pinnedFingerprint ?? null) : null);
  /** The address was changed and the hardware address left alone: the back end then forgets that one too. */
  const macGoes = $derived(
    typeof editing === "number" &&
      stored.mac !== null &&
      form.host.trim() !== stored.host.trim() &&
      macToSend(stored.mac, form.mac ?? "") === null,
  );

  /** The address typed in the form once typing has paused, to show the name it resolves to. */
  let typedHost = $state("");
  $effect(() => {
    const host = form.host.trim();
    const timer = setTimeout(() => (typedHost = host), 400);
    return () => clearTimeout(timer);
  });

  const kinds = Object.entries(KIND_LABELS) as [DeviceKind, string][];
  const oses = Object.entries(OS_LABELS) as [Os, string][];
</script>

<header class="page-head">
  <h1>Machines</h1>
  <button class="btn primary" onclick={startAdd}>Add machine</button>
</header>

<div class="layout" class:with-form={editing !== null}>
  <div class="main">
    {#if machines.length > 0}
      <section class="cards" aria-label="Machine stats">
        {#each machines as m (m.id)}
          <!-- A reading this card can't draw must not take the other machines down with it. -->
          <svelte:boundary onerror={(e) => console.error(`The card for "${m.name}" could not be shown:`, e)}>
            <MachineCard device={m} onedit={startEdit} />
            {#snippet failed(error, reset)}
              <article class="panel broken" role="alert">
                <h3 class="ellipsis" title={m.name}>{m.name}</h3>
                <p>This machine's readings can't be shown: something it sent isn't in a form home-cc understands.</p>
                <Problem
                  problem={{
                    area: "screen",
                    what: `Showing the card for "${m.name}" on the Machines screen, which failed while drawing`,
                    error: errorText(error),
                    details: deviceDetails(m),
                    trace: error instanceof Error ? error.stack : undefined,
                  }}
                  message="What went wrong: {errorText(error)}"
                />
                <div class="form-actions">
                  <button class="btn small" onclick={reset}>Try again</button>
                  <button class="btn small" onclick={() => startEdit(m)}>Edit</button>
                </div>
              </article>
            {/snippet}
          </svelte:boundary>
        {/each}
      </section>
    {/if}

    <section class="panel table-wrap">
      <div class="table-head">
        <h2 class="section-title">All machines</h2>
        {#if rest.length > 0}
          <label class="check"><input type="checkbox" bind:checked={showAll} /> Show other devices too ({rest.length})</label>
        {/if}
      </div>
      {#if listError}<Problem problem={listError} />{/if}
      {#if devices.loaded && rows.length === 0}
        <p class="muted pad">Nothing here yet. Use "Add machine" to add your first one, or scan the network and set a device's type to PC, Server or VM.</p>
      {:else}
        <div class="box">
          <table>
            <thead>
              <tr>
                <th class="c-name">Name</th>
                <th class="c-ip">Address</th>
                <th class="c-host">Hostname</th>
                <th class="c-type">Type</th>
                <th class="c-os">System</th>
                <th class="c-ssh">SSH</th>
                <th class="c-notes">Notes</th>
                <th class="c-actions"><span class="sr-only">Actions</span></th>
              </tr>
            </thead>
            <tbody>
              {#each rows as d (d.id)}
                <tr class:selected={editing === d.id}>
                  <td class="c-name name" title={d.name}>{d.name}</td>
                  <td class="c-ip mono muted" title={d.host}>
                    <Address host={d.host} part="ip" />
                    <!-- Stands in for the Hostname column when the table is too narrow for it. -->
                    <span class="host-line"><Address host={d.host} known={d.hostname} part="name" /></span>
                  </td>
                  <td class="c-host"><Address host={d.host} known={d.hostname} part="name" /></td>
                  <td class="c-type">{KIND_LABELS[d.kind]}</td>
                  <td class="c-os">{OS_LABELS[d.os]}</td>
                  <td class="c-ssh mono muted">{d.sshUser ? `${d.sshUser} · port ${d.sshPort}` : "–"}</td>
                  <td class="c-notes muted" title={d.notes}>{d.notes}</td>
                  <td class="c-actions"><div class="row-actions">
                    <button class="btn small" onclick={() => startEdit(d)}>Edit</button>
                    <ConfirmButton label="Remove" confirmLabel="Confirm remove" onconfirm={() => remove(d)} />
                  </div></td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </section>
  </div>

  {#if editing !== null}
    <form class="panel form" onsubmit={save} bind:this={formEl}>
      <h2 class="section-title">{editing === "new" ? "Add machine" : "Edit machine"}</h2>
      <label>Name <input bind:value={form.name} placeholder="Homelab VM" required /></label>
      <label>Address (IP or hostname) <input class="mono" bind:value={form.host} placeholder="192.168.68.21" required /></label>
      {#if isIpv4(form.host.trim()) || isIpv6(form.host.trim())}
        <!-- Only ever the name of the address in the field: while it is being typed, not the last one's. -->
        <p class="hint under resolved">Hostname
          {#if typedHost === form.host.trim()}<Address host={typedHost} part="name" />{:else}<span class="pending">looking…</span>{/if}
        </p>
      {/if}
      <div class="row">
        <label>Type
          <select bind:value={form.kind}>
            {#each kinds as [value, label]}<option {value}>{label}</option>{/each}
          </select>
        </label>
        <label>System
          <select bind:value={form.os}>
            {#each oses as [value, label]}<option {value}>{label}</option>{/each}
          </select>
        </label>
      </div>
      <label>Hardware address (optional)
        <input class="mono" bind:value={form.mac} placeholder="aa:bb:cc:dd:ee:ff" autocomplete="off" spellcheck="false" />
      </label>
      {#if macGoes}
        <p class="hint under">
          The address has changed, so this hardware address is forgotten when you save: it belongs to whatever was at
          the old address. The next scan fills in the right one.
        </p>
      {:else if typeof editing === "number" && stored.mac !== null && !(form.mac ?? "").trim()}
        <p class="hint under">Saving removes the hardware address. The next scan that finds this machine fills it in again.</p>
      {/if}
      <div class="row">
        <label>SSH user <input class="mono" bind:value={form.sshUser} placeholder="jim" autocomplete="off" spellcheck="false" /></label>
        <label>SSH port <input class="mono" type="number" min="1" max="65535" bind:value={form.sshPort} /></label>
      </div>
      <label>SSH password (optional)
        <input
          type="password"
          bind:value={password}
          autocomplete="new-password"
          placeholder={hasPassword ? "Type a new one to replace it" : "Only if you don't use a key"}
        />
      </label>
      {#if hasPassword && typeof editing === "number"}
        {@const id = editing}
        <div class="stored">
          <span><Icon path={icons.check} size={14} /> A password is stored.</span>
          <span class="stored-actions">
            <ConfirmButton label="Remove" confirmLabel="Remove password" onconfirm={() => removePassword(id)} />
          </span>
        </div>
      {/if}
      <p class="hint">
        To read stats, home-cc logs in with your key file (the one chosen in <a href="/settings">Settings</a>, or the usual
        ones in your .ssh folder), then the ssh-agent, then this password. The password is kept in your system's keychain
        and is never shown again.
      </p>
      {#if pinned}
        <div class="pinned">
          <span class="hint">Identity home-cc remembers for this machine</span>
          <span class="mono print" title="home-cc only logs in to a machine that shows this identity. Changing the address or the SSH port forgets it.">{pinned}</span>
        </div>
      {/if}
      {#if form.os === "windows"}
        <p class="hint">
          Windows: turn on OpenSSH Server in Windows settings first. For an administrator account the key goes in
          <span class="mono path">C:\ProgramData\ssh\administrators_authorized_keys</span>, not in the user's own .ssh folder.
        </p>
      {/if}
      <label>Notes <input bind:value={form.notes} /></label>
      {#if formError}<Problem problem={formError} />{/if}
      <div class="form-actions">
        <button class="btn primary" type="submit" disabled={saving}>{saving ? "Saving…" : "Save"}</button>
        <button class="btn" type="button" onclick={() => (editing = null)}>Cancel</button>
      </div>
    </form>
  {/if}
</div>

<style>
  .layout { display: flex; flex-direction: column; gap: var(--gap); }
  .main { display: flex; flex-direction: column; gap: var(--gap); min-width: 0; }
  /* In one column the form goes first, so opening it is never off screen. */
  .form { order: -1; }
  @media (min-width: 1500px) {
    .layout.with-form { display: grid; grid-template-columns: minmax(0, 1fr) 27rem; align-items: start; }
    .form { order: 0; position: sticky; top: 0; }
  }

  /* Spec section 6: 2 to 3 cards across at 1080p, 3 to 4 at 1440p, 5 to 6 on the ultrawide.
     Counted from the room the cards really have, so it still holds with the form open. */
  .main { container-type: inline-size; }
  .cards { display: grid; grid-template-columns: minmax(0, 1fr); gap: var(--gap); }
  @container (min-width: 62rem) {
    .cards { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  }
  @container (min-width: 100rem) {
    .cards { grid-template-columns: repeat(3, minmax(0, 1fr)); }
  }
  @container (min-width: 140rem) {
    .cards { grid-template-columns: repeat(4, minmax(0, 1fr)); }
  }
  @container (min-width: 190rem) {
    .cards { grid-template-columns: repeat(5, minmax(0, 1fr)); }
  }
  @container (min-width: 230rem) {
    .cards { grid-template-columns: repeat(6, minmax(0, 1fr)); }
  }

  .broken { display: flex; flex-direction: column; gap: 0.7rem; min-width: 0; border-color: var(--bad); }
  .broken h3 { margin: 0; font-size: 1.08rem; font-weight: 600; }
  .broken p { margin: 0; line-height: 1.5; }

  .table-wrap { padding: 0.9rem 0.7rem 0.5rem; }
  .table-head { display: flex; align-items: center; justify-content: space-between; gap: 0.4rem 1rem; flex-wrap: wrap; padding: 0 0.55rem 0.35rem; }
  .table-head .section-title { margin: 0; }
  .check { flex-direction: row; align-items: center; gap: 0.5rem; color: var(--muted); cursor: pointer; }
  .check input { width: 1.15rem; min-height: 0; height: 1.15rem; padding: 0; accent-color: var(--accent); }
  .pad { margin: 0; padding: 0.3rem 0.55rem 0.6rem; line-height: 1.5; }
  .table-wrap :global(.alert) { margin: 0 0.55rem 0.5rem; }

  .box { container-type: inline-size; }
  table { width: 100%; border-collapse: collapse; table-layout: fixed; }
  th, td { padding: 0.3rem 0.55rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-align: left; }
  th { font-weight: 500; color: var(--muted); font-size: 0.85rem; padding-block: 0.55rem; border-bottom: 1px solid var(--border); }
  td { height: 2.9rem; border-bottom: 1px solid var(--border); }
  tr:last-child td { border-bottom: 0; }
  tr.selected td { background: var(--panel-raised); }
  td.mono { font-size: 0.92rem; }
  .name { font-weight: 600; }
  .c-ip { width: 10rem; }
  .c-host { width: 11rem; }
  .c-type { width: 7.5rem; }
  .c-os { width: 8rem; }
  .c-ssh { width: 12rem; }
  .c-actions { width: 19.5rem; }
  td.c-actions { overflow: visible; }
  .row-actions { display: flex; gap: 0.4rem; justify-content: flex-end; }
  .host-line { display: none; font-size: 0.85rem; line-height: 1.3; }
  @container (max-width: 80rem) {
    .c-host { display: none; }
    .host-line { display: block; overflow: hidden; }
    td.c-ip { line-height: 1.3; }
  }
  @container (max-width: 72rem) {
    .c-notes { display: none; }
  }
  @container (max-width: 56rem) {
    .c-os { display: none; }
  }

  .form { display: flex; flex-direction: column; gap: 0.9rem; }
  .row { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 0.75rem; }
  .form-actions { display: flex; gap: 0.6rem; }
  .stored { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem; flex-wrap: wrap; margin-top: -0.4rem; color: var(--text-soft); }
  .stored > span:first-child { display: flex; align-items: center; gap: 0.4rem; }
  .stored :global(svg) { color: var(--ok); }
  .stored-actions { display: flex; gap: 0.4rem; }
  .hint { margin: 0; font-size: 0.92rem; line-height: 1.5; color: var(--muted); }
  /* A note about the field right above it. */
  .hint.under { margin-top: -0.55rem; }
  .resolved { display: flex; gap: 0.5rem; min-width: 0; }
  .resolved :global(.address) { color: var(--text-soft); }
  .resolved .pending { font-style: italic; }
  .pinned { display: flex; flex-direction: column; gap: 0.15rem; }
  /* Small enough for the whole fingerprint to stay on one line in the side-by-side form. */
  .print { color: var(--text-soft); font-size: 0.8rem; overflow-wrap: anywhere; user-select: text; }
  .path { color: var(--text-soft); font-size: 0.92em; overflow-wrap: anywhere; }
</style>
