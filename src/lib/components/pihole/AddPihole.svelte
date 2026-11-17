<script lang="ts">
  // Picking which machines are Pi-holes. home-cc suggests machines whose name says Pi-hole, and those that
  // answer like one when asked, but adds one only when the owner presses its Add button.
  import { untrack } from "svelte";
  import { errorText, KIND_LABELS, type Device } from "$lib/api";
  import { devices } from "$lib/devices.svelte";
  import { drafts } from "$lib/drafts.svelte";
  import { suggestedByName } from "$lib/pihole/logic";
  import { piholeProblem } from "$lib/pihole/problem";
  import { pihole } from "$lib/pihole/store.svelte";
  import type { Problem as ProblemInfo } from "$lib/report";
  import Address from "../Address.svelte";
  import Problem from "../Problem.svelte";

  let { onclose }: { onclose?: () => void } = $props();

  const byName = (a: Device, b: Device) => a.name.localeCompare(b.name, undefined, { numeric: true });
  const candidates = $derived(devices.list.filter((d) => d.kind !== "ignore" && !pihole.ids.has(d.id)).sort(byName));
  const byNameHits = $derived(suggestedByName(candidates, new Set()));
  /** Machines that answered like a Pi-hole, and aren't suggested by name already. */
  const answered = $derived(
    (pihole.probes ?? [])
      .filter((p) => p.found && !pihole.ids.has(p.deviceId) && !byNameHits.some((d) => d.id === p.deviceId))
      .map((p) => ({ probe: p, device: devices.byId.get(p.deviceId) }))
      .filter((x): x is { probe: typeof x.probe; device: Device } => !!x.device),
  );
  const foundOf = (id: number) => pihole.probes?.find((p) => p.deviceId === id)?.found ?? null;

  // The machine picked in the list comes back after another screen was shown.
  const pickKey = "pihole-add-pick";
  let picked = $state<number | null>(untrack(() => drafts.get<number>(pickKey) ?? null));
  $effect(() => drafts.remember(pickKey, picked));
  let adding = $state<number | null>(null);
  let error = $state<ProblemInfo | null>(null);

  async function add(d: Device) {
    adding = d.id;
    error = null;
    try {
      await pihole.add(d.id);
      if (picked === d.id) picked = null;
    } catch (e) {
      error = piholeProblem(d, `Adding "${d.name}" as a Pi-hole`, errorText(e));
    } finally {
      adding = null;
    }
  }

  function look() {
    void pihole.probe(candidates.map((d) => d.id));
  }
</script>

{#snippet row(d: Device, why: string, v5 = false)}
  <li>
    <div class="who">
      <strong class="ellipsis" title={d.name}>{d.name}</strong>
      <span class="muted small where"><Address host={d.host} known={d.hostname} /> · {KIND_LABELS[d.kind]}</span>
      <span class="small why">{why}</span>
    </div>
    {#if v5}
      <span class="muted small">Pi-hole v5 isn't supported yet: update it to v6 first.</span>
    {:else}
      <button class="btn small primary" disabled={adding != null} onclick={() => add(d)} aria-label="Add {d.name} as a Pi-hole">
        {adding === d.id ? "Adding…" : "Add as Pi-hole"}
      </button>
    {/if}
  </li>
{/snippet}

<section class="panel add" aria-label="Add a Pi-hole">
  <div class="head">
    <h2 class="section-title">Add a Pi-hole</h2>
    {#if onclose}<button class="btn small" onclick={onclose}>Close</button>{/if}
  </div>
  <p class="intro">
    Any machine home-cc knows can be a Pi-hole: a PC, a VM or a container with its own address. Add the machine first
    on the Machines screen if it isn't listed. home-cc then reads Pi-hole's numbers through its own API, with a password
    you give it.
  </p>

  {#if byNameHits.length || answered.length}
    <h3 class="sub">Suggested</h3>
    <ul class="list">
      {#each byNameHits as d (d.id)}
        {@const f = foundOf(d.id)}
        {@render row(d, f === "v6" ? "Its name says Pi-hole, and it answers like Pi-hole v6" : f === "v5" ? "Its name says Pi-hole; it runs Pi-hole v5" : "Its name says Pi-hole", f === "v5")}
      {/each}
      {#each answered as { probe, device } (device.id)}
        {@render row(device, probe.found === "v6" ? "Answers like Pi-hole v6" : "Answers like Pi-hole v5", probe.found === "v5")}
      {/each}
    </ul>
  {/if}

  <div class="look">
    <button class="btn small" onclick={look} disabled={pihole.probing || candidates.length === 0}>
      {pihole.probing ? "Looking…" : "Look for Pi-holes on my machines"}
    </button>
    <span class="muted small">
      {#if pihole.probes && !pihole.probing}
        Asked {pihole.probes.length === 1 ? "1 machine" : `${pihole.probes.length} machines`}; {pihole.probes.filter((p) => p.found).length === 0 ? "none answered like a Pi-hole." : "the ones that answered are suggested above."}
      {:else}
        Asks each machine, over plain HTTP on port 80, whether Pi-hole's login page is there. Nothing secret is sent.
      {/if}
    </span>
  </div>
  {#if pihole.probeError}
    <Problem problem={piholeProblem(undefined, "Looking for Pi-holes on the saved machines", pihole.probeError)} />
  {/if}

  <h3 class="sub">Or pick a machine</h3>
  <div class="pick">
    <label class="choose">Machine
      <select bind:value={picked}>
        <option value={null}>Choose…</option>
        {#each candidates as d (d.id)}
          <option value={d.id}>{d.name}</option>
        {/each}
      </select>
    </label>
    {#if picked != null && devices.byId.get(picked)}
      {@const d = devices.byId.get(picked)!}
      <span class="muted small picked"><Address host={d.host} known={d.hostname} /> · {KIND_LABELS[d.kind]}</span>
      <button class="btn primary" disabled={adding != null} onclick={() => add(d)}>{adding === d.id ? "Adding…" : `Add ${d.name} as a Pi-hole`}</button>
    {/if}
  </div>
  {#if error}<Problem problem={error} />{/if}
</section>

<style>
  .add { display: flex; flex-direction: column; gap: 0.75rem; min-width: 0; max-width: 72rem; }
  .head { display: flex; align-items: center; justify-content: space-between; gap: 0.6rem; flex-wrap: wrap; }
  .head .section-title { margin: 0; }
  p { margin: 0; line-height: 1.5; }
  .intro { color: var(--text-soft); }
  .sub { margin: 0.3rem 0 0; font-size: 1rem; font-weight: 600; }
  .list { list-style: none; margin: 0; padding: 0; display: grid; gap: 0.5rem; grid-template-columns: repeat(auto-fill, minmax(min(100%, 30rem), 1fr)); }
  .list li { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem 0.8rem; flex-wrap: wrap; padding: 0.6rem 0.75rem; border: 1px solid var(--border); border-radius: var(--radius-sm); min-width: 0; }
  .who { display: flex; flex-direction: column; gap: 0.15rem; min-width: 0; flex: 1 1 14rem; }
  .where { display: flex; flex-wrap: wrap; gap: 0 0.3rem; }
  .why { color: var(--accent); }
  .look { display: flex; align-items: center; gap: 0.4rem 0.8rem; flex-wrap: wrap; }
  .pick { display: flex; align-items: flex-end; gap: 0.6rem; flex-wrap: wrap; }
  .choose { flex: 0 1 26rem; min-width: 0; }
  .picked { align-self: center; display: flex; flex-wrap: wrap; gap: 0 0.3rem; }
</style>
