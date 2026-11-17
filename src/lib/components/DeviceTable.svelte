<script lang="ts">
  // One group of devices on the Network screen. "fresh" rows are ones a scan
  // found and the owner hasn't looked at; "known" rows show live status;
  // "ignored" rows can be brought back or removed.
  // Columns have fixed widths and the least important ones drop out as the
  // table gets narrower, so it never scrolls sideways.
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import { errorText, KIND_LABELS, type Device, type DeviceInput, type DeviceKind } from "$lib/api";
  import { clock } from "$lib/clock.svelte";
  import { devices, toInput } from "$lib/devices.svelte";
  import { ms, relTime } from "$lib/format";
  import { icons } from "$lib/icons";
  import { status } from "$lib/status.svelte";
  import ConfirmButton from "./ConfirmButton.svelte";
  import Icon from "./Icon.svelte";
  import Sparkline from "./Sparkline.svelte";
  import StatusDot from "./StatusDot.svelte";

  let { list, variant }: { list: Device[]; variant: "fresh" | "known" | "ignored" } = $props();

  const kinds = Object.entries(KIND_LABELS) as [DeviceKind, string][];
  const ACCEPT_AS: DeviceKind[] = ["pc", "server", "vm", "mesh", "other"];

  let error = $state<string | null>(null);
  const saving = new SvelteSet<number>();
  /** Names typed into "fresh" rows but not saved yet. */
  const drafts = new SvelteMap<number, string>();
  let renaming = $state<number | null>(null);
  let renameText = $state("");

  async function save(d: Device, patch: Partial<DeviceInput>) {
    saving.add(d.id);
    error = null;
    try {
      await devices.update(d.id, toInput(d, patch));
      drafts.delete(d.id);
    } catch (e) {
      error = `Couldn't save "${d.name}": ${errorText(e)}`;
    } finally {
      saving.delete(d.id);
    }
  }

  function accept(d: Device, kind: DeviceKind) {
    return save(d, { name: drafts.get(d.id)?.trim() || d.name, kind });
  }

  /** What the scan guessed, shown as the default choice. */
  function suggested(d: Device): DeviceKind {
    return ACCEPT_AS.includes(d.kind) ? d.kind : "other";
  }

  function startRename(d: Device) {
    renaming = d.id;
    renameText = d.name;
  }

  function finishRename(d: Device) {
    if (renaming !== d.id) return;
    renaming = null;
    const name = renameText.trim();
    if (name && name !== d.name) save(d, { name });
  }

  function renameKey(e: KeyboardEvent, d: Device) {
    if (e.key === "Enter") finishRename(d);
    else if (e.key === "Escape") renaming = null;
  }

  async function remove(d: Device) {
    error = null;
    try {
      await devices.remove(d.id);
    } catch (e) {
      error = `Couldn't remove "${d.name}": ${errorText(e)}`;
    }
  }

  function focus(node: HTMLInputElement) {
    node.focus();
    node.select();
  }

  /** Phones and tablets often make up a new hardware address for each network, which hides the maker. */
  function isPrivateMac(mac: string | null): boolean {
    return mac != null && /^[0-9a-f][26ae]/i.test(mac);
  }

  function seen(d: Device): string {
    return relTime(status.get(d)?.lastSeen ?? d.lastSeen, clock.now);
  }
</script>

{#snippet response(d: Device)}
  {@const s = status.get(d)}
  {#if !s}
    <span class="muted">–</span>
  {:else if !s.up}
    <span class="down">down</span>
  {:else if s.rttMs == null && status.isSelf(d)}
    <span class="muted" title="home-cc runs on this PC, so there is nothing to time">this PC</span>
  {:else if s.rttMs == null}
    <span class="muted" title="It is there, but it doesn't answer ping (common for Windows PCs)">no reply</span>
  {:else}
    {ms(s.rttMs)}
  {/if}
{/snippet}

{#snippet maker(d: Device)}
  {#if d.vendor}
    <span title={d.vendor}>{d.vendor}</span>
  {:else if isPrivateMac(d.mac)}
    <span class="muted" title="This device uses a made-up hardware address, as most phones and tablets do, so its maker can't be told">
      private address
    </span>
  {:else}
    <span class="muted">–</span>
  {/if}
{/snippet}

{#snippet typeSelect(d: Device)}
  <select
    class="type"
    aria-label="Type of {d.name}"
    value={d.kind}
    disabled={saving.has(d.id)}
    onchange={(e) => save(d, { kind: e.currentTarget.value as DeviceKind })}
  >
    {#each kinds as [value, label]}<option {value}>{label}</option>{/each}
  </select>
{/snippet}

<div class="box">
  {#if error}<p class="alert" role="alert">{error}</p>{/if}
  <table class={variant}>
    <thead>
      <tr>
        {#if variant !== "ignored"}<th class="c-dot"><span class="sr-only">Status</span></th>{/if}
        <th class="c-name">Name</th>
        <th class="c-ip">Address</th>
        {#if variant !== "ignored"}<th class="c-rtt">Response</th>{/if}
        {#if variant !== "fresh"}<th class="c-seen">Last seen</th>{/if}
        {#if variant === "known"}<th class="c-spark">History</th>{/if}
        {#if variant !== "fresh"}<th class="c-type">Type</th>{/if}
        <th class="c-maker">Maker</th>
        <th class="c-mac">Hardware address</th>
        <th class="c-host">Calls itself</th>
        {#if variant === "fresh"}<th class="c-accept">Add as</th>{/if}
        {#if variant === "ignored"}<th class="c-remove"><span class="sr-only">Remove</span></th>{/if}
      </tr>
    </thead>
    <tbody>
      {#each list as d (d.id)}
        <tr class:busy={saving.has(d.id)}>
          {#if variant !== "ignored"}<td class="c-dot"><StatusDot health={status.health(d)} /></td>{/if}

          <td class="c-name">
            {#if variant === "fresh"}
              <input
                class="name-input"
                aria-label="Name for {d.name}"
                value={drafts.get(d.id) ?? d.name}
                oninput={(e) => drafts.set(d.id, e.currentTarget.value)}
                onkeydown={(e) => e.key === "Enter" && accept(d, suggested(d))}
              />
            {:else if renaming === d.id}
              <input
                class="name-input"
                aria-label="New name for {d.name}"
                bind:value={renameText}
                use:focus
                onkeydown={(e) => renameKey(e, d)}
                onblur={() => finishRename(d)}
              />
            {:else}
              <button class="name-btn" title="Rename" onclick={() => startRename(d)}>
                <span class="ellipsis">{d.name}</span>
                <Icon path={icons.edit} size={13} />
              </button>
            {/if}
          </td>

          <td class="c-ip mono" title={d.mac ? `${d.host} · hardware address ${d.mac}` : d.host}>
            <span class="ip">{d.host}</span>
            <!-- Stands in for the Hardware address column when the table is too narrow for it. -->
            {#if d.mac}<span class="mac-line muted"><span class="sr-only">hardware address </span>{d.mac}</span>{/if}
          </td>
          {#if variant !== "ignored"}<td class="c-rtt mono">{@render response(d)}</td>{/if}
          {#if variant !== "fresh"}<td class="c-seen muted">{seen(d)}</td>{/if}
          {#if variant === "known"}
            <td class="c-spark">{#if status.get(d)}<Sparkline history={status.get(d)?.history ?? []} />{/if}</td>
          {/if}
          {#if variant !== "fresh"}<td class="c-type">{@render typeSelect(d)}</td>{/if}
          <td class="c-maker">{@render maker(d)}</td>
          <td class="c-mac mono">{#if d.mac}{d.mac}{:else}<span class="muted">–</span>{/if}</td>
          <td class="c-host mono" title={d.hostname ?? undefined}>
            {#if d.hostname}{d.hostname}{:else}<span class="muted">–</span>{/if}
          </td>

          {#if variant === "fresh"}
            <td class="c-accept">
              <div class="buttons">
                {#each ACCEPT_AS as kind}
                  <button
                    class="btn small"
                    class:suggested={kind === suggested(d)}
                    disabled={saving.has(d.id)}
                    aria-label="Add {d.name} as {KIND_LABELS[kind]}"
                    onclick={() => accept(d, kind)}
                  >
                    {KIND_LABELS[kind]}
                  </button>
                {/each}
                <button
                  class="btn small quiet"
                  disabled={saving.has(d.id)}
                  aria-label="Ignore {d.name}"
                  title="Keep it out of the lists and stop checking it"
                  onclick={() => accept(d, "ignore")}
                >
                  Ignore
                </button>
              </div>
            </td>
          {/if}
          {#if variant === "ignored"}
            <td class="c-remove">
              <div class="buttons">
                <ConfirmButton label="Remove" confirmLabel="Confirm remove" onconfirm={() => remove(d)} />
              </div>
            </td>
          {/if}
        </tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .box { container-type: inline-size; display: flex; flex-direction: column; gap: 0.6rem; }
  table { width: 100%; border-collapse: collapse; table-layout: fixed; }
  th, td { padding: 0.3rem 0.55rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-align: left; }
  th { font-weight: 500; color: var(--muted); font-size: 0.85rem; padding-block: 0.55rem; border-bottom: 1px solid var(--border); }
  td { height: 2.75rem; border-bottom: 1px solid var(--border); }
  tr:last-child td { border-bottom: 0; }
  tr.busy { opacity: 0.6; }
  td.mono { font-size: 0.92rem; }

  .c-dot { width: 2.1rem; text-overflow: clip; }
  .c-ip { width: 9.5rem; }
  .c-rtt { width: 6.2rem; }
  .c-seen { width: 7.5rem; }
  .c-spark { width: 11rem; }
  .c-type { width: 9.5rem; }
  .c-mac { width: 11rem; }
  .c-accept { width: 30.5rem; }
  .c-remove { width: 15rem; }
  td.c-name, td.c-type, td.c-accept, td.c-remove { overflow: visible; }
  .down { color: var(--bad); font-weight: 600; }

  .name-btn {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    max-width: 100%;
    min-height: 2.25rem;
    padding: 0 0.4rem;
    margin-left: -0.4rem;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: none;
    font-weight: 600;
    cursor: text;
  }
  .name-btn :global(svg) { flex: none; color: var(--muted); opacity: 0; }
  .name-btn:hover, .name-btn:focus-visible { border-color: var(--border-strong); outline: none; }
  .name-btn:focus-visible { border-color: var(--accent); }
  .name-btn:hover :global(svg), .name-btn:focus-visible :global(svg) { opacity: 1; }
  .name-input { min-height: 2.25rem; padding: 0 0.4rem; margin-left: -0.4rem; width: calc(100% + 0.4rem); font-weight: 600; }
  .type { min-height: 2.25rem; padding: 0 0.4rem; }

  .buttons { display: flex; gap: 0.3rem; }
  .c-remove .buttons { justify-content: flex-end; }
  .btn.suggested { border-color: var(--accent); color: var(--accent); }
  .btn.quiet { margin-left: auto; color: var(--muted); }

  .ip, .mac-line { display: block; overflow: hidden; text-overflow: ellipsis; }
  /* Sized so that the 17 characters of an address fit the Address column as it is. */
  .mac-line { display: none; font-size: 0.8rem; line-height: 1.25; user-select: text; }

  /* Narrower tables drop the least important columns first. The hardware address is never lost: without
     its own column it moves under the address, where it can still be read and copied. */
  @container (max-width: 100rem) {
    .c-mac { display: none; }
    .mac-line { display: block; }
    td.c-ip { line-height: 1.3; }
  }
  @container (max-width: 82rem) {
    .known .c-host, .fresh .c-host { display: none; }
  }
  @container (max-width: 68rem) {
    .c-maker { display: none; }
  }
  @container (max-width: 56rem) {
    .known .c-seen, .fresh .c-rtt, .ignored .c-host { display: none; }
  }
</style>
