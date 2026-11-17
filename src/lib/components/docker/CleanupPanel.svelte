<script lang="ts">
  // Cleanup with a preview: everything that could go, ticked or not, with the space it takes. Nothing is removed
  // until the owner confirms, and only what is ticked. Named volumes start unticked: they may hold a stack's data.
  import { onMount, untrack } from "svelte";
  import { drafts } from "$lib/drafts.svelte";
  import { errorText, type Device } from "$lib/api";
  import { dockerApi, type CleanupItem, type CleanupPick, type CleanupPreview, type CleanupResult, type DockerHost } from "$lib/docker/api";
  import { docker } from "$lib/docker/store.svelte";
  import { dockerIcons } from "$lib/docker/icons";
  import { cleanupDraftKey, defaultPick, isUnnamedVolume, offeredKeys, pickTotals, restorePick, type PickDraft } from "$lib/docker/logic";
  import { dockerProblem } from "$lib/docker/problem";
  import { bytes, count } from "$lib/format";
  import ConfirmButton from "../ConfirmButton.svelte";
  import Icon from "../Icon.svelte";
  import Problem from "../Problem.svelte";

  let { host, device, onclose }: { host: DockerHost; device: Device | undefined; onclose: () => void } = $props();

  let preview = $state<CleanupPreview | null>(null);
  let pick = $state<CleanupPick>({ containers: [], images: [], volumes: [], buildCache: false });
  let loading = $state(false);
  let error = $state<string | null>(null);
  let result = $state<CleanupResult | null>(null);
  let running = $state(false);

  const totals = $derived(preview ? pickTotals(preview, pick) : { count: 0, bytes: 0 });
  const name = $derived(device?.name ?? "this host");

  // What the owner ticked or unticked stays while another screen is shown (drafts.svelte.ts), until the
  // cleanup runs or the panel is closed, here or by the Docker screen (draftsDroppedWith).
  const draftKey = untrack(() => cleanupDraftKey(host.deviceId));
  $effect(() => {
    if (preview) drafts.keep<PickDraft>(draftKey, { pick, offered: offeredKeys(preview) }, { pick: defaultPick(preview), offered: offeredKeys(preview) });
  });

  function close() {
    drafts.clear(draftKey);
    onclose();
  }

  async function load() {
    loading = true;
    error = null;
    try {
      preview = await dockerApi.cleanupPreview(host.deviceId);
      const draft = drafts.get<PickDraft>(draftKey);
      pick = draft ? restorePick(draft, preview) : defaultPick(preview);
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  async function run() {
    running = true;
    error = null;
    result = null;
    try {
      result = await dockerApi.cleanup(host.deviceId, $state.snapshot(pick));
      drafts.clear(draftKey);
    } catch (e) {
      error = errorText(e);
    } finally {
      running = false;
    }
    void docker.read(host.deviceId, false);
    await load();
  }

  onMount(load);

  type Group = "containers" | "images" | "volumes";

  function toggle(group: Group, key: string, on: boolean) {
    const list = pick[group].filter((k) => k !== key);
    pick[group] = on ? [...list, key] : list;
  }

  function toggleAll(group: Group, items: CleanupItem[], on: boolean) {
    pick[group] = on ? items.map((i) => i.key) : [];
  }

  function groupBytes(items: CleanupItem[]): number {
    return items.reduce((n, i) => n + (i.sizeBytes ?? 0), 0);
  }
</script>

{#snippet group(id: Group, title: string, items: CleanupItem[], note: string)}
  {@const picked = items.filter((i) => pick[id].includes(i.key)).length}
  <fieldset class="group">
    <legend>
      <label class="all">
        <input
          type="checkbox"
          checked={items.length > 0 && picked === items.length}
          indeterminate={picked > 0 && picked < items.length}
          disabled={items.length === 0 || running}
          onchange={(e) => toggleAll(id, items, e.currentTarget.checked)}
        />
        <span><strong>{title}</strong> <span class="muted">({items.length}{items.length ? `, ${bytes(groupBytes(items))}` : ""})</span></span>
      </label>
    </legend>
    {#if note}<p class="note muted small">{note}</p>{/if}
    {#if items.length}
      <ul>
        {#each items as item (item.key)}
          <li>
            <label class="item">
              <input
                type="checkbox"
                checked={pick[id].includes(item.key)}
                disabled={running}
                onchange={(e) => toggle(id, item.key, e.currentTarget.checked)}
              />
              <span class="what">
                <span class="label ellipsis mono" title={item.key}>{item.label}</span>
                {#if item.detail}<span class="detail muted small ellipsis" class:named={id === "volumes" && !isUnnamedVolume(item.detail)}>{item.detail}</span>{/if}
              </span>
              <span class="size mono small">{item.sizeBytes != null ? bytes(item.sizeBytes) : "–"}</span>
            </label>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="muted small none">Nothing to remove.</p>
    {/if}
  </fieldset>
{/snippet}

<section class="panel cleanup" aria-label="Clean up Docker on {name}">
  <header>
    <h2>Clean up {name}</h2>
    <button class="icon-btn" aria-label="Close the cleanup" title="Close" onclick={close}><Icon path={dockerIcons.close} size={16} /></button>
  </header>
  <p class="muted small">
    What Docker keeps that nothing uses. Untick anything you want to keep; nothing is removed until you confirm. Docker
    itself refuses to remove anything that came into use since this list was made.
  </p>

  {#if error}
    <Problem problem={dockerProblem(device, `Cleaning up Docker on "${name}"`, error, host)} />
  {/if}
  {#if result}
    <div class="alert ask" role="status">
      <p>Removed {count(result.removed, "thing")}, freeing about {bytes(result.freedBytes)}.</p>
      {#if result.failures.length}
        <p>Not removed:</p>
        <ul class="fails small">{#each result.failures as f}<li>{f}</li>{/each}</ul>
      {/if}
    </div>
  {/if}

  {#if loading && !preview}
    <p class="muted">Looking at what Docker keeps on {name}… this can take a minute on a busy host.</p>
  {:else if preview}
    <div class="groups" class:stale={loading}>
      {@render group("containers", "Stopped containers", preview.containers, "")}
      {@render group("images", "Unused images", preview.images, "Not used by any container, running or stopped. They can be pulled again.")}
      {@render group(
        "volumes",
        "Unused volumes",
        preview.volumes,
        "Named volumes may hold a stack's data, such as a database, even while the stack is down. Only unnamed ones are ticked; tick a named one only if you are sure.",
      )}
      <fieldset class="group">
        <legend>
          <label class="all">
            <input type="checkbox" bind:checked={pick.buildCache} disabled={preview.buildCacheCount === 0 || running} />
            <span>
              <strong>Build cache</strong>
              <span class="muted">({preview.buildCacheCount}{preview.buildCacheCount ? `, ${bytes(preview.buildCacheBytes)}` : ""})</span>
            </span>
          </label>
        </legend>
        <p class="note muted small">Speeds up building images again. Safe to remove; the next build just takes longer.</p>
      </fieldset>
    </div>

    <footer>
      <ConfirmButton
        label={totals.count ? `Remove ${count(totals.count, "thing")} (about ${bytes(totals.bytes)})` : "Nothing ticked"}
        confirmLabel="Yes, remove them from {name}"
        disabled={totals.count === 0 || running || loading}
        onconfirm={run}
      />
      <button class="btn small" onclick={load} disabled={loading || running}>Look again</button>
      {#if running}<span class="muted small" role="status">Removing…</span>{/if}
    </footer>
  {/if}
</section>

<style>
  .cleanup { display: flex; flex-direction: column; gap: 0.75rem; min-width: 0; padding: 0.85rem 1rem 1rem; }
  header { display: flex; align-items: center; justify-content: space-between; gap: 0.75rem; }
  h2 { margin: 0; font-size: 1.15rem; font-weight: 600; }
  p { margin: 0; line-height: 1.5; }
  .groups { display: flex; flex-direction: column; gap: 0.6rem; }
  .stale { opacity: 0.6; }
  .group { border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 0.5rem 0.75rem 0.6rem; margin: 0; min-width: 0; }
  legend { padding: 0 0.3rem; }
  label.all, label.item { flex-direction: row; align-items: center; gap: 0.55rem; cursor: pointer; }
  input[type="checkbox"] { width: 1.05rem; height: 1.05rem; min-height: 0; flex: none; padding: 0; accent-color: var(--accent); }
  .note { margin: 0.1rem 0 0.4rem; }
  ul { list-style: none; margin: 0; padding: 0; max-height: 15rem; overflow-y: auto; }
  li + li { border-top: 1px solid var(--border); }
  label.item { display: flex; padding: 0.3rem 0; min-width: 0; }
  .what { display: flex; flex-direction: column; min-width: 0; flex: 1; }
  .label { font-size: 0.9rem; color: var(--text); }
  .detail.named { color: var(--warn); }
  .size { flex: none; color: var(--text-soft); }
  .none { margin: 0.2rem 0 0; }
  .fails { margin: 0.2rem 0 0; padding-left: 1.2rem; list-style: disc; max-height: none; }
  .fails li + li { border: none; }
  footer { display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; }
</style>
