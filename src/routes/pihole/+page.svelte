<script lang="ts">
  // Pi-hole: every machine the owner marked as a Pi-hole, side by side, read every 30 seconds while this screen
  // is open; pausing and resuming blocking on one or all of them; and whether they block from the same lists.
  import { onMount } from "svelte";
  import type { Device } from "$lib/api";
  import { devices } from "$lib/devices.svelte";
  import { drafts } from "$lib/drafts.svelte";
  import { piholeProblem } from "$lib/pihole/problem";
  import { pihole } from "$lib/pihole/store.svelte";
  import Problem from "$lib/components/Problem.svelte";
  import AddPihole from "$lib/components/pihole/AddPihole.svelte";
  import ListCompare from "$lib/components/pihole/ListCompare.svelte";
  import PauseControl from "$lib/components/pihole/PauseControl.svelte";
  import PiholeCard from "$lib/components/pihole/PiholeCard.svelte";

  onMount(() => pihole.watchAll());

  const byName = (a: Device, b: Device) => a.name.localeCompare(b.name, undefined, { numeric: true });
  /** The Pi-holes, with their devices, by name. */
  const shown = $derived(
    pihole.hosts
      .map((h) => devices.byId.get(h.deviceId))
      .filter((d): d is Device => !!d)
      .sort(byName),
  );
  /** Pi-holes whose latest reading worked: the ones "Pause all" and "Resume all" act on. */
  const readable = $derived(shown.filter((d) => pihole.statuses.get(d.id)?.reading));
  const blocking = $derived(readable.filter((d) => pihole.statuses.get(d.id)?.reading?.blocking.state === "enabled"));
  const notBlocking = $derived(readable.filter((d) => pihole.statuses.get(d.id)?.reading?.blocking.state !== "enabled"));
  const allWord = $derived(readable.length === 2 ? "both" : "all");
  const resuming = $derived(notBlocking.some((d) => pihole.switching.has(d.id)));

  // Whether the Add panel is open comes back after another screen was shown.
  let addOpen = $state(drafts.get<boolean>("pihole-add-open") ?? false);
  $effect(() => drafts.remember("pihole-add-open", addOpen));
  const showAdd = $derived(addOpen || (pihole.loaded && shown.length === 0));
</script>

<header class="page-head">
  <h1>Pi-hole</h1>
  <div class="actions">
    {#if readable.length > 1}
      {#if notBlocking.length > 0}
        <button class="btn primary" disabled={resuming} onclick={() => pihole.setBlocking(notBlocking.map((d) => d.id), true, null)}>
          {resuming ? "Turning blocking on…" : `Resume ${notBlocking.length === readable.length ? allWord : notBlocking.length === 1 ? notBlocking[0].name : `${notBlocking.length} Pi-holes`}`}
        </button>
      {/if}
      {#if blocking.length === readable.length}
        <PauseControl
          ids={blocking.map((d) => d.id)}
          target={readable.length === 2 ? "both Pi-holes" : `all ${readable.length} Pi-holes`}
          label="Pause {allWord}…"
          draftKey="pihole-pause:all"
          small={false}
        />
      {/if}
    {/if}
    {#if shown.length > 0 && !showAdd}
      <button class="btn" onclick={() => (addOpen = true)}>Add a Pi-hole</button>
    {/if}
  </div>
</header>

{#if pihole.loadError}
  <Problem problem={piholeProblem(undefined, "Loading the list of Pi-holes", pihole.loadError)} />
{/if}

{#if !pihole.loaded || !devices.loaded}
  <p class="muted">Loading…</p>
{:else}
  <div class="page">
    {#if showAdd}
      <AddPihole onclose={shown.length > 0 ? () => (addOpen = false) : undefined} />
    {/if}
    {#if shown.length > 0}
      <div class="cards">
        {#each shown as d (d.id)}
          <PiholeCard device={d} />
        {/each}
      </div>
      <ListCompare hosts={shown.map((d) => ({ id: d.id, name: d.name, lists: pihole.lastGood.get(d.id)?.reading.lists ?? null }))} />
    {/if}
  </div>
{/if}

<style>
  .page { display: flex; flex-direction: column; gap: var(--gap); }
  .cards { display: grid; gap: var(--gap); grid-template-columns: repeat(auto-fill, minmax(min(100%, 34rem), 1fr)); align-items: start; }
  /* The pause choices are wider than a button: they go under the heading rather than squeezing it. */
  .actions { align-items: flex-start; }
</style>
