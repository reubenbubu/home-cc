<script lang="ts">
  // Every VM and container on a Proxmox host, with its state and what it uses, a quick start, resume or shut
  // down (which asks first), and "Manage" for everything else (restart, force stop, snapshots).
  import { untrack } from "svelte";
  import type { ProxmoxGuest } from "$lib/api";
  import { drafts } from "$lib/drafts.svelte";
  import { bytes, pct, uptime, usedOfTotal } from "$lib/format";
  import {
    actionWords,
    guestKey,
    guestName,
    isHibernated,
    isPaused,
    kindLabel,
    lockBlocks,
    lockLabel,
    stateLabel,
  } from "$lib/proxmox";
  import { proxmox } from "$lib/proxmox.svelte";
  import { STATUS_ORDER, type Column } from "$lib/table";
  import { TableView } from "$lib/table.svelte";
  import ConfirmButton from "../ConfirmButton.svelte";
  import GroupRow from "../table/GroupRow.svelte";
  import SortButton from "../table/SortButton.svelte";
  import TableTools from "../table/TableTools.svelte";

  let {
    deviceId,
    guests,
    selected,
    onselect,
    stale = false,
  }: {
    deviceId: number;
    guests: ProxmoxGuest[];
    /** guestKey of the guest open in the side panel. */
    selected: string | null;
    onselect: (g: ProxmoxGuest) => void;
    stale?: boolean;
  } = $props();

  // Still ticked after another screen was shown.
  const templatesKey = untrack(() => `proxmox-templates:${deviceId}`);
  let showTemplates = $state(drafts.get<boolean>(templatesKey) ?? false);
  $effect(() => drafts.remember(templatesKey, showTemplates));
  const templates = $derived(guests.filter((g) => g.template).length);
  const shown = $derived(showTemplates ? guests : guests.filter((g) => !g.template));
  const multiNode = $derived(new Set(guests.map((g) => g.node)).size > 1);

  /** Running, then paused or hibernated, then stopped, as the dots show it; templates aren't any of these. */
  function stateRank(g: ProxmoxGuest): string | null {
    const tone = dotClass(g);
    return tone === "bad" ? "down" : tone === "warn" ? "warning" : tone === "ok" ? "up" : null;
  }
  const used = (u: number | null, total: number | null) => u ?? total;

  // Sorting, grouping and filtering (table.ts). Grouping is offered for state, type and node, not for names
  // or numbers. RAM and disk sort by what is used (the disk by its size when that is all Proxmox says).
  const columns = $derived.by(() => {
    const cols: Column<ProxmoxGuest>[] = [
      {
        id: "state",
        label: "State",
        kind: "rank",
        order: STATUS_ORDER,
        value: stateRank,
        text: stateLabel,
        search: (g) => [stateLabel(g), lockLabel(g.lock)],
        none: "Template",
        words: ["stopped first", "running first"],
        group: true,
        choose: true,
      },
      { id: "id", label: "ID", kind: "number", value: (g) => g.vmid },
      { id: "name", label: "Name", value: guestName, search: (g) => [guestName(g), ...g.tags] },
      { id: "type", label: "Type", value: kindLabel, group: true, choose: true },
    ];
    if (multiNode) cols.push({ id: "node", label: "Node", value: (g) => g.node, group: true, choose: true });
    cols.push(
      // Readings, left out of the filter: a typed "4" would find every guest using 4 of something.
      { id: "cpu", label: "CPU", kind: "number", value: (g) => g.cpuPct, search: () => null },
      // A stopped guest uses no memory, so it goes last; the disk of a VM is only known by its size.
      { id: "ram", label: "RAM", kind: "number", value: (g) => g.memUsed, search: () => null },
      { id: "disk", label: "Disk", kind: "number", value: (g) => used(g.diskUsed, g.diskTotal), search: () => null },
      { id: "up", label: "Up for", kind: "number", value: (g) => g.uptimeSecs || null, search: () => null },
    );
    return cols;
  });

  const view = new TableView<ProxmoxGuest>({
    id: untrack(() => `proxmox-guests:${deviceId}`),
    columns: () => columns,
    rows: () => shown,
    key: guestKey,
    usual: "as Proxmox lists them",
  });

  function dotClass(g: ProxmoxGuest): string {
    if (g.template) return "idle";
    if (g.status === "running") return "ok";
    if (isPaused(g) || isHibernated(g)) return "warn";
    return "bad";
  }

  /** The one button worth having in the list: start a stopped guest (or resume a hibernated one), resume a
   *  paused one, shut down a running one. */
  function quick(g: ProxmoxGuest): "start" | "resume" | "shutdown" | null {
    if (g.template) return null;
    if (g.status === "stopped") return "start";
    if (isPaused(g)) return "resume";
    if (g.status === "running") return "shutdown";
    return null;
  }
</script>

<section class="panel guests" class:stale aria-label="VMs and containers">
  <div class="head">
    <h2 class="section-title">VMs and containers</h2>
    {#if templates > 0}
      <label class="check"><input type="checkbox" bind:checked={showTemplates} /> Show templates ({templates})</label>
    {/if}
  </div>
  {#if shown.length === 0}
    <p class="muted pad">
      {guests.length === 0
        ? "Proxmox shows no VMs or containers to this token. If there should be some, give the token the PVEAuditor role on \"/\" (see Connection settings)."
        : "Only templates here. Tick \"Show templates\" to see them."}
    </p>
  {:else}
    <div class="box">
      <TableTools {view} label="VMs and containers" noun="VMs or containers" />
      {#if view.shown > 0}
      <table use:view.watch>
        <thead>
          <tr>
            <th class="c-state" aria-sort={view.ariaSort("state")}><SortButton {view} col="state" /></th>
            <th class="c-id" aria-sort={view.ariaSort("id")}><SortButton {view} col="id" /></th>
            <th class="c-name" aria-sort={view.ariaSort("name")}><SortButton {view} col="name" /></th>
            <th class="c-type" aria-sort={view.ariaSort("type")}><SortButton {view} col="type" /></th>
            {#if multiNode}<th class="c-node" aria-sort={view.ariaSort("node")}><SortButton {view} col="node" /></th>{/if}
            <th class="c-cpu" aria-sort={view.ariaSort("cpu")}><SortButton {view} col="cpu" /></th>
            <th class="c-ram" aria-sort={view.ariaSort("ram")}><SortButton {view} col="ram" /></th>
            <th class="c-disk" aria-sort={view.ariaSort("disk")}><SortButton {view} col="disk" /></th>
            <th class="c-up" aria-sort={view.ariaSort("up")}><SortButton {view} col="up" /></th>
            <th class="c-actions"><span class="sr-only">Actions</span></th>
          </tr>
        </thead>
        {#each view.sections as section (section.key)}
        <tbody>
          {#if section.label !== null}<GroupRow {view} {section} />{/if}
          {#if section.open}
          {#each section.rows as g (guestKey(g))}
            {@const key = guestKey(g)}
            {@const busy = proxmox.busy.get(key)}
            {@const action = quick(g)}
            {@const lock = lockLabel(g.lock)}
            <tr class:selected={selected === key}>
              <td class="c-state">
                <span class="state">
                  <span class="dot {dotClass(g)}" aria-hidden="true"></span>
                  <span class="ellipsis">{stateLabel(g)}</span>
                </span>
              </td>
              <td class="c-id mono">{g.vmid}</td>
              <td class="c-name">
                <span class="name ellipsis" title={guestName(g)}>{guestName(g)}</span>
                {#if lock}
                  <span
                    class="lock small"
                    title={isHibernated(g) ? "Its memory is saved to disk. Resume starts it again where it left off." : `Proxmox is busy with it: ${g.lock}`}
                  >{lock}</span>
                {/if}
                {#if g.tags.length}<span class="tags muted small ellipsis" title={g.tags.join(", ")}>{g.tags.join(" · ")}</span>{/if}
              </td>
              <td class="c-type">{kindLabel(g)}</td>
              {#if multiNode}<td class="c-node mono muted ellipsis" title={g.node}>{g.node}</td>{/if}
              <td class="c-cpu mono">
                {#if g.cpuPct != null}
                  {pct(g.cpuPct)}<span class="muted small" title="Share of the {g.cpuCount} cores given to it">{g.cpuCount ? ` of ${g.cpuCount}` : ""}</span>
                {:else}
                  <span class="muted" title={g.cpuCount ? `${g.cpuCount} cores given to it` : undefined}>–</span>
                {/if}
              </td>
              <td class="c-ram mono">
                {#if g.memUsed != null && g.memTotal != null}
                  {usedOfTotal(g.memUsed, g.memTotal)}
                {:else if g.memTotal != null}
                  <span class="muted">{bytes(g.memTotal)}</span>
                {:else}–{/if}
              </td>
              <td class="c-disk mono" title={g.kind === "qemu" ? "Proxmox only knows a VM's disk size, not how full it is" : undefined}>
                {#if g.diskUsed != null && g.diskTotal != null}
                  {usedOfTotal(g.diskUsed, g.diskTotal)}
                {:else if g.diskTotal != null}
                  <span class="muted">{bytes(g.diskTotal)}</span>
                {:else}–{/if}
              </td>
              <td class="c-up mono muted">{g.uptimeSecs != null ? uptime(g.uptimeSecs) : "–"}</td>
              <td class="c-actions">
                <div class="row-actions">
                  {#if busy}
                    <span class="muted small busy">{busy}</span>
                  {:else if action}
                    {@const words = actionWords(g, action)}
                    {@const held = lockBlocks(g, action)}
                    {@const title = held ? `Wait until Proxmox is done: ${held}` : undefined}
                    {#if words.confirm}
                      <ConfirmButton
                        label={words.label}
                        confirmLabel={words.confirm}
                        disabled={!!held}
                        {title}
                        onconfirm={() => proxmox.power(deviceId, g, action)}
                      />
                    {:else}
                      <button class="btn small" disabled={!!held} {title} onclick={() => proxmox.power(deviceId, g, action)}>
                        {words.label}
                      </button>
                    {/if}
                  {/if}
                  <button class="btn small" aria-label="Manage {guestName(g)}" onclick={() => onselect(g)}>Manage</button>
                </div>
              </td>
            </tr>
          {/each}
          {/if}
        </tbody>
        {/each}
      </table>
      {/if}
    </div>
  {/if}
</section>

<style>
  .guests { padding: 0.9rem 0.7rem 0.5rem; min-width: 0; }
  .head { display: flex; align-items: center; justify-content: space-between; gap: 0.4rem 1rem; flex-wrap: wrap; padding: 0 0.55rem 0.35rem; }
  .head .section-title { margin: 0; }
  .check { flex-direction: row; align-items: center; gap: 0.5rem; color: var(--muted); cursor: pointer; }
  .check input { width: 1.15rem; min-height: 0; height: 1.15rem; padding: 0; accent-color: var(--accent); }
  .pad { margin: 0; padding: 0.3rem 0.55rem 0.6rem; line-height: 1.5; }

  .box { container-type: inline-size; }
  table { width: 100%; border-collapse: collapse; table-layout: fixed; }
  th, td { padding: 0.3rem 0.55rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-align: left; }
  th { font-weight: 500; color: var(--muted); font-size: 0.85rem; padding-block: 0.55rem; border-bottom: 1px solid var(--border); }
  td { height: 2.9rem; border-bottom: 1px solid var(--border); }
  tr:last-child td { border-bottom: 0; }
  tr.selected td { background: var(--panel-raised); }
  td.mono { font-size: 0.92rem; }

  .c-state { width: 7.5rem; }
  .c-id { width: 4.5rem; }
  .c-type { width: 6.5rem; }
  .c-node { width: 7rem; }
  .c-cpu { width: 6.5rem; }
  .c-ram { width: 10.5rem; }
  .c-disk { width: 10.5rem; }
  .c-up { width: 7.5rem; }
  .c-actions { width: 13.5rem; }
  td.c-actions { overflow: visible; }

  .state { display: flex; align-items: center; gap: 0.5rem; min-width: 0; }
  td.c-name { display: table-cell; }
  .name { display: block; font-weight: 600; }
  .lock { display: inline-block; margin-top: 0.1rem; color: var(--warn); }
  .tags { display: block; }
  .lock + .tags { display: none; }
  /* Wraps while a shut down waits for its second click, which takes more room than the column has. */
  .row-actions { display: flex; flex-wrap: wrap; gap: 0.4rem; justify-content: flex-end; align-items: center; padding-block: 0.2rem; }
  .busy { white-space: nowrap; }

  /* Narrower: the least needed columns go first. */
  @container (max-width: 84rem) {
    .c-disk { display: none; }
  }
  @container (max-width: 74rem) {
    .c-up, .c-node { display: none; }
  }
  @container (max-width: 62rem) {
    .c-type { display: none; }
  }
  @container (max-width: 52rem) {
    .c-id { display: none; }
  }
  /* The smallest window zoomed right in: name, state and the buttons are what matter (Manage shows the rest). */
  @container (max-width: 44rem) {
    .c-cpu { display: none; }
  }
  @container (max-width: 36rem) {
    .c-ram { display: none; }
  }

  .stale tbody { opacity: 0.6; }
</style>
