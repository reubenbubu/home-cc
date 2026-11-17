<script lang="ts">
  // One group of devices on the Network screen. "fresh" rows are ones a scan
  // found and the owner hasn't looked at; "known" rows show live status;
  // "ignored" rows can be brought back or removed.
  // Columns have fixed widths and the least important ones drop out as the
  // table gets narrower, so it never scrolls sideways.
  import { untrack } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import { errorText, KIND_LABELS, type Device, type DeviceInput, type DeviceKind } from "$lib/api";
  import { clock } from "$lib/clock.svelte";
  import { devices, toInput } from "$lib/devices.svelte";
  import { drafts as kept } from "$lib/drafts.svelte";
  import { ms, relTime } from "$lib/format";
  import { icons } from "$lib/icons";
  import { names } from "$lib/names.svelte";
  import { status } from "$lib/status.svelte";
  import { deviceDetails } from "$lib/problems.svelte";
  import type { Problem as ProblemInfo } from "$lib/report";
  import { STATUS_ORDER, type Column } from "$lib/table";
  import { TableView } from "$lib/table.svelte";
  import Address from "./Address.svelte";
  import ConfirmButton from "./ConfirmButton.svelte";
  import Problem from "./Problem.svelte";
  import Icon from "./Icon.svelte";
  import Sparkline from "./Sparkline.svelte";
  import StatusDot from "./StatusDot.svelte";
  import GroupRow from "./table/GroupRow.svelte";
  import SortButton from "./table/SortButton.svelte";
  import TableTools from "./table/TableTools.svelte";

  let {
    list,
    variant,
    id,
    label,
  }: {
    list: Device[];
    variant: "fresh" | "known" | "ignored";
    /** Names this table's sort, grouping and filter where they are kept: one for each table. */
    id: string;
    /** What the table lists, for the names of its controls: "mesh nodes". */
    label: string;
  } = $props();

  const kinds = Object.entries(KIND_LABELS) as [DeviceKind, string][];
  const ACCEPT_AS: DeviceKind[] = ["pc", "server", "vm", "mesh", "appliance", "other"];

  let error = $state<ProblemInfo | null>(null);
  const saving = new SvelteSet<number>();
  // Names typed into "fresh" rows, and a rename not finished yet, stay while another screen is shown
  // (drafts.svelte.ts). One of these tables for each group, so each keeps its own.
  const namesKey = `device-names:${untrack(() => variant)}`;
  const renameDraftKey = `device-rename:${untrack(() => variant)}`;
  /** Names typed into "fresh" rows but not saved yet. */
  const drafts = new SvelteMap<number, string>(kept.get<[number, string][]>(namesKey) ?? []);
  const rename = kept.get<{ id: number; text: string }>(renameDraftKey);
  let renaming = $state<number | null>(rename?.id ?? null);
  let renameText = $state(rename?.text ?? "");
  $effect(() => kept.keep<[number, string][]>(namesKey, [...drafts], []));
  $effect(() => kept.remember(renameDraftKey, renaming === null ? null : { id: renaming, text: renameText }));

  async function save(d: Device, patch: Partial<DeviceInput>) {
    saving.add(d.id);
    error = null;
    try {
      await devices.update(d.id, toInput(d, patch));
      drafts.delete(d.id);
    } catch (e) {
      error = {
        area: "devices",
        what: `Saving a change to "${d.name}" on the Network screen`,
        error: `Couldn't save "${d.name}": ${errorText(e)}`,
        details: { ...deviceDetails(d), "Change": JSON.stringify(patch) },
      };
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
      error = {
        area: "devices",
        what: `Removing "${d.name}" on the Network screen`,
        error: `Couldn't remove "${d.name}": ${errorText(e)}`,
        details: deviceDetails(d),
      };
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

  /** The Response column in words, as shown. */
  function responseText(d: Device): string {
    const s = status.get(d);
    if (!s) return "";
    if (!s.up) return "down";
    if (s.rttMs == null) return status.isSelf(d) ? "this PC" : "no reply";
    return ms(s.rttMs);
  }

  const HEALTH_WORDS = { up: "Up", down: "Down", unknown: "Not checked yet" } as const;

  // Sorting, grouping and filtering (table.ts). The filter looks at every column, the hostname shown beside
  // each address included. Grouping is offered for what many devices share (status, type, maker), not for
  // names or addresses.
  const columns = $derived.by(() => {
    const cols: Column<Device>[] = [];
    if (variant !== "ignored") {
      cols.push({
        id: "status",
        label: "Status",
        kind: "rank",
        order: STATUS_ORDER,
        value: (d) => (status.health(d) === "unknown" ? null : status.health(d)),
        text: (d) => HEALTH_WORDS[status.health(d)],
        none: HEALTH_WORDS.unknown,
        group: true,
        choose: true,
      });
    }
    cols.push(
      { id: "name", label: "Name", value: (d) => d.name },
      { id: "address", label: "Address", kind: "ip", value: (d) => d.host },
      { id: "hostname", label: "Hostname", value: (d) => names.text(d.host, d.hostname) },
    );
    if (variant !== "ignored") {
      cols.push({
        id: "response",
        label: "Response",
        kind: "number",
        value: (d) => (status.get(d)?.up ? status.get(d)?.rttMs : null),
        search: responseText,
      });
    }
    if (variant !== "fresh") {
      cols.push(
        {
          id: "seen",
          label: "Last seen",
          kind: "time",
          value: (d) => status.get(d)?.lastSeen ?? d.lastSeen,
          search: seen,
          words: ["longest ago first", "most recent first"],
        },
        { id: "type", label: "Type", value: (d) => KIND_LABELS[d.kind], group: true, choose: variant === "known" },
      );
    }
    cols.push(
      {
        id: "maker",
        label: "Maker",
        value: (d) => d.vendor,
        search: (d) => d.vendor || (isPrivateMac(d.mac) ? "private address" : null),
        group: true,
        none: "Maker not known",
      },
      { id: "mac", label: "Hardware address", value: (d) => d.mac },
    );
    return cols;
  });

  const view = new TableView<Device>({
    id: untrack(() => id),
    columns: () => columns,
    rows: () => list,
    key: (d) => d.id,
    usual: "by address",
  });
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
  {#if error}<Problem problem={error} />{/if}
  <TableTools {view} {label} noun={label} />
  {#if view.shown > 0}
  <table class={variant} use:view.watch>
    <thead>
      <tr>
        {#if variant !== "ignored"}<th class="c-dot" aria-sort={view.ariaSort("status")}><SortButton {view} col="status" compact /></th>{/if}
        <th class="c-name" aria-sort={view.ariaSort("name")}><SortButton {view} col="name" /></th>
        <th class="c-ip" aria-sort={view.ariaSort("address")}><SortButton {view} col="address" /></th>
        <th class="c-host" aria-sort={view.ariaSort("hostname")}><SortButton {view} col="hostname" /></th>
        {#if variant !== "ignored"}<th class="c-rtt" aria-sort={view.ariaSort("response")}><SortButton {view} col="response" /></th>{/if}
        {#if variant !== "fresh"}<th class="c-seen" aria-sort={view.ariaSort("seen")}><SortButton {view} col="seen" /></th>{/if}
        {#if variant === "known"}<th class="c-spark">History</th>{/if}
        {#if variant !== "fresh"}<th class="c-type" aria-sort={view.ariaSort("type")}><SortButton {view} col="type" /></th>{/if}
        <th class="c-maker" aria-sort={view.ariaSort("maker")}><SortButton {view} col="maker" /></th>
        <th class="c-mac" aria-sort={view.ariaSort("mac")}><SortButton {view} col="mac" /></th>
        {#if variant === "fresh"}<th class="c-accept">Add as</th>{/if}
        {#if variant === "ignored"}<th class="c-remove"><span class="sr-only">Remove</span></th>{/if}
      </tr>
    </thead>
    {#each view.sections as section (section.key)}
    <tbody>
      {#if section.label !== null}<GroupRow {view} {section} />{/if}
      {#if section.open}
      {#each section.rows as d (d.id)}
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
            <span class="ip"><Address host={d.host} part="ip" /></span>
            <!-- Stands in for the Hardware address column when the table is too narrow for it. -->
            {#if d.mac}<span class="mac-line muted"><span class="sr-only">hardware address </span>{d.mac}</span>{/if}
          </td>
          <td class="c-host"><Address host={d.host} known={d.hostname} part="name" /></td>
          {#if variant !== "ignored"}<td class="c-rtt mono">{@render response(d)}</td>{/if}
          {#if variant !== "fresh"}<td class="c-seen muted">{seen(d)}</td>{/if}
          {#if variant === "known"}
            <td class="c-spark">{#if status.get(d)}<Sparkline history={status.get(d)?.history ?? []} />{/if}</td>
          {/if}
          {#if variant !== "fresh"}<td class="c-type">{@render typeSelect(d)}</td>{/if}
          <td class="c-maker">{@render maker(d)}</td>
          <td class="c-mac mono">{#if d.mac}{d.mac}{:else}<span class="muted">–</span>{/if}</td>

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
      {/if}
    </tbody>
    {/each}
  </table>
  {/if}
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
  .c-host { width: 12rem; }
  /* Room for "Response" and the arrow when the table is sorted by it. */
  .c-rtt { width: 7.6rem; }
  .c-seen { width: 7.5rem; }
  .c-spark { width: 11rem; }
  .c-type { width: 9.5rem; }
  .c-mac { width: 11rem; }
  .c-accept { width: 34rem; }
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
     its own column it moves under the address, where it can still be read and copied. The hostname is
     never dropped: an address is always shown with its name. */
  @container (max-width: 100rem) {
    .c-mac { display: none; }
    .mac-line { display: block; }
    td.c-ip { line-height: 1.3; }
  }
  @container (max-width: 82rem) {
    .c-maker { display: none; }
  }
  @container (max-width: 68rem) {
    .known .c-seen, .fresh .c-rtt { display: none; }
    .c-host { width: 10rem; }
  }
  @container (max-width: 56rem) {
    .ignored .c-seen { display: none; }
  }

  /* Too narrow for columns (a small window zoomed in): each device is a short block of lines instead.
     Status, name and response on the first line, the address and type under them, then any buttons. */
  @container (max-width: 48rem) {
    table, tbody { display: block; }
    thead { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
    tr {
      display: grid;
      grid-template-columns: 2.1rem minmax(0, 1fr) auto;
      align-items: center;
      padding-block: 0.3rem;
      border-bottom: 1px solid var(--border);
    }
    tr:last-child { border-bottom: 0; }
    td { display: block; height: auto; border-bottom: 0; padding-block: 0.15rem; }
    .box td { width: auto; }
    .c-seen, .c-spark, .c-maker, .c-host { display: none; }
    td.c-dot { grid-area: 1 / 1; }
    td.c-name { grid-area: 1 / 2; }
    td.c-rtt { grid-area: 1 / 3; text-align: right; }
    td.c-ip { grid-area: 2 / 2; }
    td.c-type { grid-area: 2 / 3; }
    td.c-accept { grid-area: 3 / 1 / 4 / -1; }
    .c-accept .buttons { flex-wrap: wrap; }
    .ignored tr { grid-template-columns: minmax(0, 1fr) auto; }
    .ignored td.c-name { grid-area: 1 / 1; }
    .ignored td.c-ip { grid-area: 2 / 1; }
    .ignored td.c-type { grid-area: 1 / 2; }
    .ignored td.c-remove { grid-area: 2 / 2; }
  }
</style>
