<script lang="ts">
  // Image updates on one host: what has a newer image in its registry, stack by stack, with a tick per service.
  // Nothing is pulled or restarted until the owner confirms, and then only the ticked services: their new
  // images are pulled and just those containers are recreated. Opened for one stack ("Update…" on its card) or
  // for the whole host ("Update all…"). Containers outside any stack can only have their image pulled: how they
  // were started isn't known, so they are left for the owner to recreate.
  import { untrack } from "svelte";
  import { clock } from "$lib/clock.svelte";
  import { drafts } from "$lib/drafts.svelte";
  import type { Device } from "$lib/api";
  import type { Container, DockerHost } from "$lib/docker/api";
  import { docker } from "$lib/docker/store.svelte";
  import { dockerIcons } from "$lib/docker/icons";
  import {
    checkFailureText,
    containerKey,
    hostUpdates,
    isTicked,
    serviceKey,
    shortDigest,
    updatePlan,
    updatesByKey,
    updatesDraftKey,
    updateOf,
    type StackUpdates,
    type UpdatesDraft,
  } from "$lib/docker/logic";
  import { dockerProblem } from "$lib/docker/problem";
  import { count, relTime } from "$lib/format";
  import ConfirmButton from "../ConfirmButton.svelte";
  import Icon from "../Icon.svelte";
  import Problem from "../Problem.svelte";

  let {
    host,
    device,
    stack,
    onclose,
    oncleanup,
  }: {
    host: DockerHost;
    device: Device | undefined;
    /** One stack's updates, or null for the whole host. */
    stack: string | null;
    onclose: () => void;
    /** Opens the cleanup, where the old images can be removed. */
    oncleanup: () => void;
  } = $props();

  const id = $derived(host.deviceId);
  const name = $derived(device?.name ?? "this host");
  const overview = $derived(docker.overviews.get(id));
  const check = $derived(docker.updates.get(id));
  const answers = $derived(updatesByKey(check));
  const all = $derived(overview ? hostUpdates(overview, answers) : null);
  const stacks = $derived<StackUpdates[]>(all ? (stack == null ? all.stacks : all.stacks.filter((s) => s.stack === stack)) : []);
  const others = $derived(stack == null ? (all?.others ?? []) : []);
  /** Images that couldn't be checked: all of the host's, or the ones this stack runs. */
  const failed = $derived.by(() => {
    if (!all) return [];
    if (stack == null) return all.failed;
    const mine = overview?.stacks.find((s) => s.name === stack)?.containers.map((c) => c.image) ?? [];
    return all.failed.filter((u) => mine.includes(u.image));
  });
  const checking = $derived(docker.checking.has(id));
  const checkError = $derived(docker.updateErrors.get(id));
  const updatingNow = $derived(docker.updating.get(id));
  const result = $derived(docker.updateResults.get(id));

  // The owner's ticks stay while another screen is shown (drafts.svelte.ts), until the update runs or the panel
  // is put away, here or by the Docker screen (draftsDroppedWith).
  const draftKey = untrack(() => updatesDraftKey(host.deviceId));
  let choices = $state<Record<string, boolean>>(drafts.get<UpdatesDraft>(draftKey)?.choices ?? {});
  $effect(() => drafts.keep<UpdatesDraft>(draftKey, { choices }, { choices: {} }));

  const plan = $derived(updatePlan(stacks, choices));
  const planCount = $derived(plan.reduce((n, s) => n + s.count, 0));

  function close() {
    drafts.clear(draftKey);
    docker.updateResults.delete(id);
    onclose();
  }

  function tick(stackName: string, service: string, on: boolean) {
    choices = { ...choices, [serviceKey(stackName, service)]: on };
  }

  function tickStack(s: StackUpdates, on: boolean) {
    const next = { ...choices };
    for (const x of s.services) next[serviceKey(s.stack, x.service)] = on;
    choices = next;
  }

  async function run() {
    const chosen = $state.snapshot(plan);
    choices = {};
    drafts.clear(draftKey);
    await docker.updateStacks(
      id,
      chosen.map((s) => ({ stack: s.stack, services: s.services.map((x) => x.service), images: [...new Set(s.services.map((x) => x.image))] })),
    );
  }

  function pulling(c: Container): boolean {
    return !!docker.action(containerKey(id, c.id))?.doing;
  }
</script>

<section class="panel updates" aria-label="Image updates on {name}">
  <header>
    <h2>{stack == null ? `Updates on ${name}` : `Update ${stack}`}</h2>
    <button class="icon-btn" aria-label="Close the updates" title="Close" onclick={close}><Icon path={dockerIcons.close} size={16} /></button>
  </header>
  <p class="muted small when">
    {#if checking}
      <span class="spinner" aria-hidden="true"></span> Checking the registries for newer images…
    {:else}
      {check?.checkedAt ? `Checked ${relTime(check.checkedAt, clock.now)}.` : check ? "Checked in part." : "Not checked yet."}
    {/if}
    <button class="btn small" disabled={checking || !!updatingNow} onclick={() => docker.checkUpdates(id, false)}>Check again</button>
  </p>

  {#if checkError}
    <Problem problem={dockerProblem(device, `Checking the images on "${name}" for updates`, checkError, host)} message="Couldn't check for updates: {checkError}" />
  {/if}

  {#if result}
    <div class="alert ask" role="status">
      {#if result.done.length}
        <p>
          Updated {result.done.join(", ")}. The old images stay on {name} as unused images;
          <button class="linkish" onclick={oncleanup}>Clean up…</button> can remove them.
        </p>
      {/if}
      {#if result.failed.length}<p>Not updated: {result.failed.join(", ")}. What Docker said is on {result.failed.length === 1 ? "its card" : "their cards"}.</p>{/if}
    </div>
  {/if}

  {#if stacks.length}
    <div class="groups" class:stale={!!updatingNow}>
      {#each stacks as s (s.stack)}
        {@const ticked = s.services.filter((x) => isTicked(s.stack, x, choices)).length}
        <fieldset class="group">
          <legend>
            <label class="all">
              <input
                type="checkbox"
                checked={ticked === s.services.length}
                indeterminate={ticked > 0 && ticked < s.services.length}
                disabled={!!updatingNow}
                onchange={(e) => tickStack(s, e.currentTarget.checked)}
              />
              <span><strong>{s.stack}</strong> <span class="muted">({count(s.services.length, "service")})</span></span>
            </label>
            {#if updatingNow === s.stack}<span class="doing small" role="status"><span class="spinner" aria-hidden="true"></span>Updating…</span>{/if}
          </legend>
          <ul>
            {#each s.services as x (x.service)}
              <li>
                <label class="item">
                  <input
                    type="checkbox"
                    checked={isTicked(s.stack, x, choices)}
                    disabled={!!updatingNow}
                    onchange={(e) => tick(s.stack, x.service, e.currentTarget.checked)}
                  />
                  <span class="what">
                    <span class="label">{x.service}</span>
                    <span class="detail mono small ellipsis" title={x.image}>{x.image}</span>
                    <span class="detail mono small" title="{x.update.current ?? ''} → {x.update.latest ?? ''}">
                      {shortDigest(x.update.current)} → {shortDigest(x.update.latest)}{x.update.downloaded ? " (pulled already)" : ""}
                    </span>
                    <span class="detail small muted">
                      {x.stopped ? `Stopped now: updating starts ${x.containers.join(", ")}` : `Restarts ${x.containers.join(", ")}`}
                    </span>
                  </span>
                </label>
              </li>
            {/each}
          </ul>
        </fieldset>
      {/each}
    </div>
    <p class="note muted small">
      Pulls the new images, then recreates only the ticked containers; the rest of each stack keeps running. The old
      images stay on the host until a cleanup removes them.
    </p>
    <footer>
      <ConfirmButton
        label={planCount ? `Update ${count(planCount, "container")} in ${count(plan.length, "stack")}` : "Nothing ticked"}
        confirmLabel="Yes, pull and restart {planCount === 1 ? 'it' : 'them'}"
        disabled={planCount === 0 || !!updatingNow}
        onconfirm={run}
      />
      {#if updatingNow}<span class="muted small" role="status">Updating {updatingNow}…</span>{/if}
    </footer>
  {:else if check && !checking && !others.length}
    <p class="muted">{stack == null ? `Everything on ${name} that could be checked is up to date.` : `${stack} is up to date.`}</p>
  {/if}

  {#if others.length}
    <section class="others" aria-label="Containers outside any stack">
      <h3>Containers outside any stack</h3>
      <p class="note muted small">
        home-cc doesn't know the options these were started with, so it won't recreate them. It can pull the new image;
        to use it, recreate the container yourself with the same options.
      </p>
      <ul>
        {#each others as { container: c } (c.id)}
          {@const u = updateOf(answers, c)}
          {@const pullError = docker.action(containerKey(id, c.id))?.error}
          <li>
            <div class="other">
            <span class="what">
              <span class="label">{c.name}</span>
              <span class="detail mono small ellipsis" title={c.image}>{c.image}</span>
              <span class="detail mono small">{shortDigest(u?.current)} → {shortDigest(u?.latest)}</span>
            </span>
            {#if u?.downloaded}
              <span class="done small">New image pulled. Recreate {c.name} by hand to use it.</span>
            {:else if pulling(c)}
              <span class="doing small" role="status"><span class="spinner" aria-hidden="true"></span>Pulling…</span>
            {:else}
              <span class="pull"><ConfirmButton label="Pull image" confirmLabel="Yes, pull it" onconfirm={() => docker.pullImage(id, c)} /></span>
            {/if}
            </div>
            {#if pullError}
              <Problem problem={dockerProblem(device, `Pulling the image of "${c.name}" on "${name}"`, pullError, host)} message="Couldn't pull {c.image}: {pullError}" />
            {/if}
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  {#if failed.length}
    <Problem
      tone="warn"
      problem={dockerProblem(
        device,
        `Checking images for updates on "${name}"`,
        failed.map((u) => `${u.image}: ${checkFailureText(u)}`).join("\n"),
        host,
      )}
    >
      <p>{count(failed.length, "image")} couldn't be checked:</p>
      <ul class="fails small">
        {#each failed as u (u.image)}<li><span class="mono">{u.image}</span>: {checkFailureText(u)}</li>{/each}
      </ul>
    </Problem>
  {/if}
</section>

<style>
  .updates { display: flex; flex-direction: column; gap: 0.75rem; min-width: 0; padding: 0.85rem 1rem 1rem; }
  header { display: flex; align-items: center; justify-content: space-between; gap: 0.75rem; }
  h2 { margin: 0; font-size: 1.15rem; font-weight: 600; overflow-wrap: anywhere; }
  h3 { margin: 0; font-size: 1rem; font-weight: 600; }
  p { margin: 0; line-height: 1.5; }
  .when { display: flex; align-items: center; flex-wrap: wrap; gap: 0.4rem 0.6rem; }
  .groups { display: flex; flex-direction: column; gap: 0.6rem; }
  .stale { opacity: 0.7; }
  .group { border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 0.5rem 0.75rem 0.6rem; margin: 0; min-width: 0; }
  legend { padding: 0 0.3rem; display: flex; align-items: center; gap: 0.75rem; flex-wrap: wrap; }
  label.all, label.item { flex-direction: row; align-items: center; gap: 0.55rem; cursor: pointer; }
  label.item { display: flex; align-items: flex-start; padding: 0.35rem 0; min-width: 0; }
  label.item input { margin-top: 0.2rem; }
  input[type="checkbox"] { width: 1.05rem; height: 1.05rem; min-height: 0; flex: none; padding: 0; accent-color: var(--accent); }
  ul { list-style: none; margin: 0; padding: 0; }
  li + li { border-top: 1px solid var(--border); }
  .what { display: flex; flex-direction: column; min-width: 0; flex: 1; }
  .label { font-weight: 500; color: var(--text); overflow-wrap: anywhere; }
  .detail { color: var(--text-soft); overflow-wrap: anywhere; }
  .note { margin: 0; }
  footer { display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; }
  .others { display: flex; flex-direction: column; gap: 0.4rem; min-width: 0; }
  .other { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 0.4rem 0.75rem; padding: 0.4rem 0; }
  .pull { display: inline-flex; gap: 0.4rem; flex-wrap: wrap; }
  .done { color: var(--accent); }
  .doing { display: inline-flex; align-items: center; gap: 0.45rem; color: var(--accent); }
  .fails { margin: 0.3rem 0 0; padding-left: 1.2rem; list-style: disc; overflow-wrap: anywhere; }
  .fails li + li { border: none; }
  .linkish { border: none; background: none; padding: 0; color: var(--accent); cursor: pointer; font: inherit; text-decoration: underline; }
  .spinner {
    display: inline-block;
    width: 0.85rem;
    height: 0.85rem;
    border: 2px solid color-mix(in srgb, var(--accent) 30%, transparent);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.9s linear infinite;
    flex: none;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .spinner { animation: none; } }
</style>
