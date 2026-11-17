<script lang="ts">
  // Key login for the machine in the Machines form: which key this PC uses (with its public half to copy),
  // making one if there is none, and "Set up key login", which adds the key to the machine like ssh-copy-id and
  // then checks that a login with the key alone works. A password is only sent to a machine whose identity the
  // owner approved; if it isn't approved yet the identity is shown here first, as on the machine cards.
  import { onMount, tick, untrack } from "svelte";
  import { api, errorText, secretKeys, type Device, type HostKey, type KeySetupOutcome, type LocalKey, type Os } from "$lib/api";
  import { identityCommand, identityText } from "$lib/hostkey";
  import { icons } from "$lib/icons";
  import { manualSteps, passwordSource, setupBlocker, whatHappens, type Typed } from "$lib/keysetup";
  import { keySetupAsk, keySetupDraft } from "$lib/keysetup.svelte";
  import { drafts } from "$lib/drafts.svelte";
  import { copyText, deviceDetails } from "$lib/problems.svelte";
  import type { Problem as ProblemInfo } from "$lib/report";
  import ConfirmButton from "./ConfirmButton.svelte";
  import Icon from "./Icon.svelte";
  import Problem from "./Problem.svelte";

  let {
    device,
    os,
    typed,
    password,
    hasPassword,
    onpasswordremoved,
  }: {
    /** The machine as saved, or null for one that isn't saved yet. */
    device: Device | null;
    /** The system chosen in the form. */
    os: Os;
    /** Address, user and port as they are in the form now, saved or not. */
    typed: Typed;
    /** The password typed in the form. Used for this once; it is only stored if the owner saves the form. */
    password: string;
    /** A password is stored for this machine. */
    hasPassword: boolean;
    onpasswordremoved: () => void;
  } = $props();

  // Open or not, and a passphrase typed for a new key, come back after another screen was shown (in this
  // window's memory only, see drafts.svelte.ts). The machine form clears this when it is saved or cancelled.
  type KeySetupForm = { open: boolean; passphrase: string };
  const draftKey = untrack(() => keySetupDraft(device?.id ?? "new"));
  const draft = drafts.get<KeySetupForm>(draftKey);
  let open = $state(draft?.open ?? false);
  let el = $state<HTMLElement>();
  let local = $state<LocalKey | null>(null);
  let loading = $state(false);
  let loadError = $state<ProblemInfo | null>(null);
  let passphrase = $state(draft?.passphrase ?? "");
  $effect(() => drafts.keep<KeySetupForm>(draftKey, { open, passphrase }, { open: false, passphrase: "" }));
  // Open when the screen was left: its key is read again.
  onMount(() => {
    if (open && !local && !loading) void loadKey();
  });
  let creating = $state(false);
  let createError = $state<ProblemInfo | null>(null);
  let working = $state(false);
  let outcome = $state<KeySetupOutcome | null>(null);
  let setupError = $state<ProblemInfo | null>(null);
  let approving = $state(false);
  let approveError = $state<string | null>(null);
  let removed = $state(false);
  let removeError = $state<ProblemInfo | null>(null);
  let copied = $state<"idle" | "copied" | "failed">("idle");

  const user = $derived((device?.sshUser ?? typed.user).trim() || "the SSH user");
  const blocker = $derived(setupBlocker(device, typed, !!local?.publicKey));
  const source = $derived(passwordSource(!!password, hasPassword));
  const steps = $derived(local?.publicKey ? manualSteps(local.publicKey, os) : []);

  /** A problem in this part of the form. Says which machine and key, never a password. */
  function problem(what: string, error: string): ProblemInfo {
    return {
      area: "keys",
      what,
      error,
      details: {
        ...(device ? deviceDetails(device) : { Device: "a new machine" }),
        "This PC's key": local?.path ?? "none found",
        "Key chosen in Settings": local ? local.fromSettings : null,
        "Key fingerprint": local?.fingerprint,
        "A password was typed": !!password,
        "Password stored for it": hasPassword,
      },
    };
  }

  // "Set up key login" on this machine's card.
  $effect(() => {
    const id = device?.id;
    if (id != null && keySetupAsk.deviceId === id) untrack(() => keySetupAsk.take(id) && show());
  });

  async function show() {
    open = true;
    if (!local && !loading) await loadKey();
    await tick();
    el?.scrollIntoView({ block: "nearest" });
  }

  function toggle() {
    if (open) open = false;
    else show();
  }

  async function loadKey() {
    loading = true;
    loadError = null;
    try {
      local = await api.localSshKey();
    } catch (e) {
      loadError = problem("Looking for this PC's SSH key", `Couldn't look for this PC's SSH key: ${errorText(e)}`);
    } finally {
      loading = false;
    }
  }

  async function create() {
    if (creating) return;
    creating = true;
    createError = null;
    try {
      local = await api.createSshKey(passphrase || null);
      passphrase = "";
    } catch (e) {
      createError = problem("Making an SSH key for this PC", `Couldn't make the key: ${errorText(e)}`);
    } finally {
      creating = false;
    }
  }

  async function setUp() {
    if (!device || working) return;
    const d = device;
    working = true;
    setupError = null;
    approveError = null;
    removeError = null;
    outcome = null;
    removed = false;
    try {
      outcome = await api.setupKeyLogin(d.id, password || null);
    } catch (e) {
      setupError = problem(`Setting up key login for "${d.name}"`, errorText(e));
    } finally {
      working = false;
    }
  }

  /** The owner checked the identity on screen: store exactly that one, then carry on with the setup. */
  async function approve(key: HostKey) {
    if (!device) return;
    approving = true;
    approveError = null;
    try {
      await api.approveKeySetupIdentity(device.id, key.fingerprint);
    } catch (e) {
      approveError = errorText(e);
      return;
    } finally {
      approving = false;
    }
    await setUp();
  }

  async function removePassword() {
    if (!device) return;
    removeError = null;
    try {
      await api.deleteSecret(secretKeys.sshPassword(device.id));
      removed = true;
      onpasswordremoved();
    } catch (e) {
      removeError = problem(`Removing the stored password of "${device.name}" after setting up key login`, `Couldn't remove the password: ${errorText(e)}`);
    }
  }

  async function copy() {
    if (!local?.publicKey) return;
    copied = (await copyText(local.publicKey)) ? "copied" : "failed";
  }

  const selectAll = (e: FocusEvent) => (e.currentTarget as HTMLTextAreaElement).select();
</script>

{#snippet howToCheck(key: HostKey, same: string)}
  <p class="small muted">
    To check, run <span class="mono cmd">{identityCommand(key.algorithm, os)}</span> on the machine itself. It prints a
    line with the same SHA256 text as {same}.
  </p>
{/snippet}

<!-- Drawn afresh for each identity, so a half-given confirmation never carries over to one the owner hasn't read. -->
{#snippet approveButton(key: HostKey, label: string)}
  <div class="confirm">
    {#key key.fingerprint}
      <ConfirmButton {label} confirmLabel="Yes, it matches" disabled={approving || working} onconfirm={() => approve(key)} />
    {/key}
  </div>
{/snippet}

<section class="keys" bind:this={el} aria-labelledby="key-login-title">
  <div class="head">
    <h3 id="key-login-title">Key login</h3>
    <button type="button" class="btn small" aria-expanded={open} onclick={toggle}>{open ? "Hide" : "Show key login"}</button>
  </div>

  {#if !open}
    <p class="hint">Log in with this PC's SSH key instead of a password. home-cc can add the key to the machine for you.</p>
  {:else if loading}
    <p class="hint">Looking for this PC's key…</p>
  {:else if loadError}
    <Problem problem={loadError} />
    <div><button type="button" class="btn small" onclick={loadKey}>Look again</button></div>
  {:else if local}
    {#if local.publicKey}
      <div class="field">
        <span class="label">This PC's key</span>
        <span class="mono path" title={local.path}>{local.path}</span>
        <span class="hint">
          {local.fromSettings ? "The key file chosen in Settings." : "From your .ssh folder."}
          <span class="mono print">{local.algorithm} {local.fingerprint}</span>
        </span>
      </div>
      <div class="field">
        <span class="label" id="public-key-label">Its public key, the line that goes on the machine</span>
        <textarea class="mono pub" readonly rows="3" value={local.publicKey} aria-labelledby="public-key-label" onfocus={selectAll}></textarea>
        <div class="row">
          <button type="button" class="btn small with-icon" onclick={copy}><Icon path={icons.copy} size={14} /> Copy public key</button>
          <span class="said" role="status">
            {#if copied === "copied"}Copied.{:else if copied === "failed"}Couldn't copy it. Select the text above and copy it yourself.{/if}
          </span>
        </div>
      </div>
      {#if local.note}<p class="hint">{local.note}</p>{/if}
      {#if local.others.length > 0}
        <p class="hint">Logins also offer {local.others.join(", ")} from the same folder.</p>
      {/if}
    {:else if local.newKeyPath}
      <p class="text">
        This PC has no SSH key yet. home-cc can make one: <span class="mono path">{local.newKeyPath}</span>, with its
        public half next to it in a .pub file. Nothing that is there already is changed.
      </p>
      <label>Passphrase for the new key (optional)
        <input
          type="password"
          bind:value={passphrase}
          autocomplete="new-password"
          placeholder="Leave empty for none"
          onkeydown={(e) => {
            // Enter here makes the key; it must not save the machine form around it.
            if (e.key === "Enter") {
              e.preventDefault();
              create();
            }
          }}
        />
      </label>
      <p class="hint under">A passphrase locks the key file. home-cc keeps it in your system's keychain, like the one in Settings.</p>
      {#if createError}<Problem problem={createError} />{/if}
      <div><button type="button" class="btn" disabled={creating} onclick={create}>{creating ? "Making the key…" : "Make a key"}</button></div>
    {:else if local.problem}
      <Problem problem={problem("Reading this PC's SSH key", local.problem)} />
      <div><button type="button" class="btn small" onclick={loadKey}>Look again</button></div>
    {/if}

    {#if local.publicKey}
      <h4>Add it to {device?.name ?? "this machine"}</h4>
      <p class="hint">{whatHappens(os, user, source)}</p>
      {#if blocker}<p class="hint blocker">{blocker}</p>{/if}
      <div>
        <button type="button" class="btn primary" disabled={!!blocker || working || approving} onclick={setUp}>
          {working ? "Setting up…" : "Set up key login"}
        </button>
      </div>

      {#if approveError}
        <Problem problem={problem(`Approving the identity of "${device?.name}" to set up key login`, approveError)} message="Not approved: {approveError}" />
      {/if}
      {#if outcome?.outcome === "approve" && outcome.expectedFingerprint}
        {@const key = outcome.offeredKey}
        <div class="alert warn" role="alert">
          <p><strong>This machine's identity has changed.</strong></p>
          <p>
            That is normal if you reinstalled or replaced it. If you didn't, something else may be answering at this
            address. Nothing is sent to it until you approve the new identity.
          </p>
          <dl class="prints">
            <dt>Was</dt>
            <dd class="mono">{outcome.expectedFingerprint}</dd>
            <dt>Now</dt>
            <dd class="mono">{identityText(key)}</dd>
          </dl>
          {@render howToCheck(key, '"Now"')}
          {@render approveButton(key, "Approve the new identity")}
        </div>
      {:else if outcome?.outcome === "approve"}
        {@const key = outcome.offeredKey}
        <div class="alert ask" role="status">
          <p><strong>Before the password is used for {device?.name}, check its identity.</strong></p>
          <p>The machine shows this identity:</p>
          <p class="mono print">{identityText(key)}</p>
          <p>
            Check it matches, then approve to use the password. The password has not been sent, and won't be until you
            approve.
          </p>
          {@render howToCheck(key, "the identity above")}
          {@render approveButton(key, "Approve and use the password")}
        </div>
      {:else if outcome?.outcome === "done" && outcome.keyLoginWorks}
        <div class="alert ok" role="status">
          <p class="done"><Icon path={icons.check} size={16} /> <span>{outcome.message}</span></p>
          {#if removed}
            <p>The stored password was removed.</p>
          {:else if hasPassword}
            <p>The stored password isn't needed for this any more.</p>
            <div class="confirm">
              <ConfirmButton label="Remove the stored password" confirmLabel="Yes, remove it" onconfirm={removePassword} />
            </div>
          {/if}
        </div>
      {:else if outcome?.outcome === "done"}
        {@const done = outcome}
        <Problem tone="warn" problem={problem(`Setting up key login for "${device?.name}"`, [done.message, done.advice].filter(Boolean).join(" "))}>
          <p>{done.message}</p>
          {#if done.advice}<p class="advice">{done.advice}</p>{/if}
        </Problem>
      {/if}
      {#if setupError}<Problem problem={setupError} />{/if}
      {#if removeError}<Problem problem={removeError} />{/if}

      <details class="manual">
        <summary>Do it by hand</summary>
        <p class="hint">If home-cc can't log in to add the key (password logins turned off, say), run this on the machine.</p>
        {#each steps as step, i}
          <div class="field">
            <span class="label" id="manual-step-{i}">{step.where}:</span>
            <textarea class="mono cmd-box" readonly rows="3" value={step.command} aria-labelledby="manual-step-{i}" onfocus={selectAll}></textarea>
          </div>
        {/each}
      </details>
    {/if}
  {/if}
</section>

<style>
  .keys { display: flex; flex-direction: column; gap: 0.75rem; padding-top: 0.9rem; border-top: 1px solid var(--border); min-width: 0; }
  .head { display: flex; align-items: center; justify-content: space-between; gap: 0.75rem; }
  h3 { margin: 0; font-size: 1rem; font-weight: 600; }
  h4 { margin: 0.2rem 0 0; font-size: 0.95rem; font-weight: 600; }
  p { margin: 0; line-height: 1.5; }
  .text { line-height: 1.5; }
  .hint { font-size: 0.92rem; color: var(--muted); }
  .hint.under { margin-top: -0.45rem; }
  .blocker { color: var(--warn); }
  .field { display: flex; flex-direction: column; gap: 0.3rem; min-width: 0; }
  .label { color: var(--text-soft); font-size: 0.92rem; }
  .path { color: var(--text-soft); font-size: 0.92rem; overflow-wrap: anywhere; user-select: text; }
  .print { font-size: 0.85rem; overflow-wrap: anywhere; user-select: text; }
  textarea {
    width: 100%;
    padding: 0.45rem 0.6rem;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    background: var(--bg-sunk);
    font-size: 0.85rem;
    line-height: 1.45;
    resize: vertical;
    word-break: break-all;
    user-select: text;
  }
  .row { display: flex; align-items: center; gap: 0.4rem 0.8rem; flex-wrap: wrap; }
  .with-icon { display: inline-flex; align-items: center; gap: 0.4rem; }
  .said { color: var(--muted); font-size: 0.88rem; }
  .said:empty { display: none; }
  .alert p + p { margin-top: 0.3rem; }
  .alert.ok { border-color: var(--ok); background: color-mix(in srgb, var(--ok) 8%, transparent); }
  .done { display: flex; gap: 0.45rem; align-items: flex-start; }
  .done :global(svg) { flex: none; margin-top: 0.2rem; color: var(--ok); }
  .advice { margin-top: 0.35rem; }
  .confirm { display: flex; gap: 0.4rem; flex-wrap: wrap; margin-top: 0.6rem; }
  .prints { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 0.2rem 0.7rem; margin: 0.5rem 0; }
  .prints dt { color: var(--muted); }
  .prints dd { margin: 0; }
  .prints dd, .alert .print, .cmd { font-size: 0.92rem; overflow-wrap: anywhere; user-select: text; }
  .alert .print { margin: 0.2rem 0 0.4rem; }
  .cmd { color: var(--text-soft); font-size: 0.92em; }
  .manual { display: flex; flex-direction: column; gap: 0.6rem; }
  .manual summary { cursor: pointer; color: var(--accent); font-size: 0.92rem; width: fit-content; }
  .manual[open] summary { margin-bottom: 0.5rem; }
  .manual .field + .field { margin-top: 0.6rem; }
  .manual > .hint { margin-bottom: 0.6rem; }
</style>
