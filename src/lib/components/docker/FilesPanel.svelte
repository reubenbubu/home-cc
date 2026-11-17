<script lang="ts">
  // A stack's files: its compose files, and the Dockerfiles and .env files in its folder. Edit, save (after a
  // confirm, with a copy of the old file kept next to it on the host), then bring the stack up with the change.
  import { onMount, untrack } from "svelte";
  import { errorText, type Device } from "$lib/api";
  import { fileOnReturn, type FileDraft } from "$lib/drafts";
  import { drafts } from "$lib/drafts.svelte";
  import { dockerApi, type DockerHost, type SavedFile, type Stack } from "$lib/docker/api";
  import { docker } from "$lib/docker/store.svelte";
  import { dockerIcons } from "$lib/docker/icons";
  import { baseName, stackKey } from "$lib/docker/logic";
  import { dockerProblem } from "$lib/docker/problem";
  import ConfirmButton from "../ConfirmButton.svelte";
  import Icon from "../Icon.svelte";
  import Problem from "../Problem.svelte";

  let { host, device, stack, onclose }: { host: DockerHost; device: Device | undefined; stack: Stack; onclose: () => void } = $props();

  let files = $state<string[]>([]);
  let path = $state<string | null>(null);
  /** The file as it was read from the host. */
  let original = $state("");
  let text = $state("");
  let loading = $state(false);
  let saving = $state(false);
  let error = $state<{ what: string; message: string } | null>(null);
  let saved = $state<SavedFile | null>(null);
  /** The file was changed on the host while the owner was on another screen; their text was kept. */
  let hostChanged = $state(false);
  /** The text came back from before the owner left the screen. */
  let restored = $state(false);

  // The file open and the owner's unsaved text stay while another screen is shown (drafts.svelte.ts). Only
  // Close (or a save) lets them go.
  const draftKey = untrack(() => `docker-file:${host.deviceId}:${stack.name}`);
  $effect(() => {
    if (path != null) drafts.set<FileDraft>(draftKey, { path, original, text });
  });

  function close() {
    drafts.clear(draftKey);
    onclose();
  }

  const changed = $derived(path != null && text !== original);
  const name = $derived(device?.name ?? "the host");
  const action = $derived(docker.action(stackKey(host.deviceId, stack.name)));
  const isCompose = $derived(path != null && (stack.configFiles.includes(path) || /compose[^/]*\.ya?ml$/.test(path)));

  async function listFiles() {
    loading = true;
    error = null;
    try {
      files = await dockerApi.stackFiles(host.deviceId, stack.name);
      const draft = drafts.get<FileDraft>(draftKey);
      if (draft && files.includes(draft.path)) await open(draft.path, draft);
      else if (files.length && (path == null || !files.includes(path))) await open(files[0]);
    } catch (e) {
      error = { what: `Listing the files of the "${stack.name}" stack`, message: errorText(e) };
    } finally {
      loading = false;
    }
  }

  /** Reads a file from the host. With a draft, the owner's text from before stays (see fileOnReturn). */
  async function open(next: string, draft?: FileDraft) {
    loading = true;
    error = null;
    saved = null;
    try {
      const body = await dockerApi.readFile(host.deviceId, stack.name, next);
      const shown = draft ? fileOnReturn(draft, body) : { original: body, text: body, hostChanged: false };
      path = next;
      original = shown.original;
      text = shown.text;
      hostChanged = shown.hostChanged;
      restored = shown.text !== shown.original;
    } catch (e) {
      error = { what: `Opening ${next}`, message: errorText(e) };
    } finally {
      loading = false;
    }
  }

  async function save() {
    if (path == null) return;
    saving = true;
    error = null;
    saved = null;
    try {
      saved = await dockerApi.saveFile(host.deviceId, stack.name, path, original, text);
      original = text;
      hostChanged = false;
      restored = false;
    } catch (e) {
      error = { what: `Saving ${path}`, message: errorText(e) };
    } finally {
      saving = false;
    }
  }

  /** Tab puts in two spaces (compose files are YAML, where tabs aren't allowed) instead of leaving the box. */
  function keydown(e: KeyboardEvent) {
    if (e.key !== "Tab" || e.shiftKey || e.ctrlKey || e.altKey || e.metaKey) return;
    const box = e.currentTarget as HTMLTextAreaElement;
    e.preventDefault();
    const { selectionStart: from, selectionEnd: to } = box;
    text = text.slice(0, from) + "  " + text.slice(to);
    queueMicrotask(() => box.setSelectionRange(from + 2, from + 2));
  }

  onMount(listFiles);
</script>

<section class="panel files" aria-label="Files of the {stack.name} stack">
  <header>
    <div class="title">
      <h2 class="ellipsis">{stack.name}: files</h2>
      {#if stack.workingDir}<span class="muted small mono ellipsis" title={stack.workingDir}>{stack.workingDir}</span>{/if}
    </div>
    {#if changed}
      <ConfirmButton label="Close" confirmLabel="Close without saving" onconfirm={close} />
    {:else}
      <button class="icon-btn" aria-label="Close the files" title="Close" onclick={close}><Icon path={dockerIcons.close} size={16} /></button>
    {/if}
  </header>

  {#if files.length > 1}
    <div class="tabs" role="tablist" aria-label="Files">
      {#each files as f (f)}
        <button
          role="tab"
          class="tab mono small"
          class:active={f === path}
          aria-selected={f === path}
          title={f}
          disabled={loading || saving || (changed && f !== path)}
          onclick={() => open(f)}
        >
          {baseName(f)}
        </button>
      {/each}
    </div>
    {#if changed}<p class="muted small">Save or undo your changes to open another file.</p>{/if}
  {/if}

  {#if error}
    <Problem problem={dockerProblem(device, `${error.what} on "${name}"`, error.message, host)} />
  {/if}

  {#if path != null}
    <p class="path mono small muted ellipsis" title={path}>{path}</p>
    {#if hostChanged && changed}
      <p class="alert warn small" role="status">
        {baseName(path)} was changed on {name} while you were on another screen. Your text is kept here, so saving
        replaces that change. Undo changes shows the file as it is on {name} now.
      </p>
    {/if}
    <textarea
      class="mono"
      bind:value={text}
      onkeydown={keydown}
      spellcheck="false"
      autocomplete="off"
      aria-label="Contents of {baseName(path)}"
      disabled={loading || saving}
    ></textarea>
    <div class="bar">
      <ConfirmButton
        label={saving ? "Saving…" : "Save"}
        confirmLabel="Yes, save to {name}"
        disabled={!changed || saving || loading}
        onconfirm={save}
      />
      <button class="btn small" disabled={!changed || saving} onclick={() => (text = original)}>Undo changes</button>
      <span class="muted small">
        {changed ? (restored ? "Unsaved changes, kept from before." : "Not saved yet.") : "A copy of the old file is kept next to it on each save."}
      </span>
    </div>
  {:else if loading}
    <p class="muted">Opening…</p>
  {:else if !error}
    <p class="muted">No files found for this stack.</p>
  {/if}

  {#if saved}
    <div class="alert {saved.warning ? 'warn' : 'ask'}" role="status">
      <p>Saved. The old version is at <span class="mono">{saved.backup}</span>.</p>
      {#if saved.warning}
        <p>Docker Compose doesn't accept the stack's files as they are now:</p>
        <pre class="mono small">{saved.warning}</pre>
      {:else if isCompose || stack.configFiles.length}
        <p>Bring the stack up to use the change:</p>
      {/if}
      {#if host.compose !== "none"}
        <div class="bar">
          <button class="btn small primary" disabled={!!action?.doing} onclick={() => docker.stackAction(host.deviceId, stack.name, "up")}>Start the stack with it</button>
          <button class="btn small" disabled={!!action?.doing} onclick={() => docker.stackAction(host.deviceId, stack.name, "build")}>Rebuild images and start</button>
          {#if action?.doing}<span class="muted small" role="status">{action.doing}…</span>{/if}
        </div>
      {/if}
    </div>
  {/if}
  {#if action?.error && saved}
    <Problem problem={dockerProblem(device, `Starting the "${stack.name}" stack after saving a file`, action.error, host)} />
  {/if}
</section>

<style>
  .files { display: flex; flex-direction: column; gap: 0.6rem; min-width: 0; padding: 0.85rem 1rem 1rem; }
  header { display: flex; align-items: center; justify-content: space-between; gap: 0.75rem; }
  .title { display: flex; align-items: baseline; gap: 0.75rem; min-width: 0; }
  h2 { margin: 0; font-size: 1.15rem; font-weight: 600; flex: none; }
  p { margin: 0; line-height: 1.5; }
  .tabs { display: flex; flex-wrap: wrap; gap: 0.3rem; }
  .tab {
    min-height: 2.1rem;
    padding: 0 0.7rem;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    background: var(--panel);
    color: var(--text-soft);
    cursor: pointer;
  }
  .tab.active { border-color: var(--accent); color: var(--accent); background: var(--panel-raised); }
  .tab:disabled:not(.active) { opacity: 0.5; cursor: not-allowed; }
  textarea {
    height: clamp(16rem, calc(100vh - 22rem), 70rem);
    padding: 0.6rem 0.75rem;
    resize: vertical;
    font-size: 0.88rem;
    line-height: 1.5;
    tab-size: 2;
    /* Long lines wrap rather than scroll sideways. */
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    overflow-x: hidden;
    overflow-y: auto;
  }
  .bar { display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; }
  .alert .bar { margin-top: 0.5rem; }
  .alert pre { margin: 0.3rem 0 0; white-space: pre-wrap; overflow-wrap: anywhere; max-height: 12rem; overflow: auto; }
</style>
