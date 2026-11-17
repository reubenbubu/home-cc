<script lang="ts">
  // How home-cc reaches one Pi-hole: HTTPS (the certificate is checked) or plain HTTP, the port, and the
  // password, which goes to the system keychain and is never shown again. Also takes the device off the
  // Pi-hole screen.
  import { untrack } from "svelte";
  import { errorText, type Device } from "$lib/api";
  import { differs } from "$lib/drafts";
  import { drafts } from "$lib/drafts.svelte";
  import { icons } from "$lib/icons";
  import { portFor } from "$lib/pihole/logic";
  import { pihole } from "$lib/pihole/store.svelte";
  import type { Problem as ProblemInfo } from "$lib/report";
  import Address from "../Address.svelte";
  import ConfirmButton from "../ConfirmButton.svelte";
  import DraftNote from "../DraftNote.svelte";
  import Icon from "../Icon.svelte";
  import Problem from "../Problem.svelte";

  let {
    device,
    problem,
    onclose,
  }: {
    device: Device;
    /** Builds a problem with this Pi-hole's details. */
    problem: (what: string, error: string) => ProblemInfo;
    /** Given when the form can be closed (a password is stored). */
    onclose?: () => void;
  } = $props();

  type Form = { https: boolean; port: number; password: string };

  const stored = $derived(pihole.settings(device.id));
  // What was typed before the owner went to another screen comes back first (drafts.svelte.ts).
  const draftKey = untrack(() => `pihole-setup:${device.id}`);
  const draft = drafts.get<Form>(draftKey);
  const start = untrack(() => pihole.settings(device.id));
  let https = $state(draft?.https ?? start?.https ?? true);
  let port = $state(draft?.port ?? start?.port ?? 443);
  /** Typed here; goes to the keychain on save and is never read back. Until then it is only in this window's
   *  memory, also while another screen is shown. */
  let password = $state(draft?.password ?? "");
  let saving = $state(false);
  let error = $state<ProblemInfo | null>(null);
  /** The form was filled from a draft rather than from what is saved. */
  let restored = $state(!!draft);

  const savedForm = $derived<Form | undefined>(stored ? { https: stored.https, port: stored.port, password: "" } : undefined);
  const unsaved = $derived(restored && differs<Form>({ https, port, password }, savedForm));

  // Kept while it differs from what is saved.
  $effect(() => {
    drafts.keep<Form>(draftKey, { https, port, password }, savedForm);
  });

  function choose(useHttps: boolean) {
    https = useHttps;
    port = portFor(useHttps, Number(port) || 0);
  }

  function discard() {
    drafts.clear(draftKey);
    restored = false;
    password = "";
    if (stored) {
      https = stored.https;
      port = stored.port;
    }
  }

  function close() {
    drafts.clear(draftKey);
    onclose?.();
  }

  async function save(e: SubmitEvent) {
    e.preventDefault();
    saving = true;
    error = null;
    try {
      const p = Number(port) || (https ? 443 : 80);
      await pihole.save(device.id, https, p, password);
      port = p;
      password = "";
      restored = false;
      drafts.clear(draftKey);
      onclose?.();
    } catch (err) {
      error = problem("Saving the Pi-hole connection settings", errorText(err));
    } finally {
      saving = false;
    }
  }

  async function removePassword() {
    error = null;
    try {
      await pihole.removePassword(device.id);
    } catch (err) {
      error = problem("Removing the stored Pi-hole password", `Couldn't remove the password: ${errorText(err)}`);
    }
  }

  async function removeHost() {
    error = null;
    try {
      drafts.clear(draftKey);
      await pihole.remove(device.id);
    } catch (err) {
      error = problem("Taking a device off the Pi-hole screen", errorText(err));
    }
  }
</script>

<form class="panel setup" onsubmit={save} aria-label="Connection settings for {device.name}">
  <div class="head">
    <h3 class="title">Connection settings for {device.name}</h3>
    {#if onclose}<button type="button" class="btn small" onclick={close}>Close</button>{/if}
  </div>

  <fieldset class="scheme">
    <legend>Connect with</legend>
    <label class="choice">
      <input type="radio" name="scheme-{device.id}" checked={https} onchange={() => choose(true)} />
      <span><strong>HTTPS</strong> (recommended): the connection is encrypted, and home-cc checks the Pi-hole's certificate.</span>
    </label>
    <label class="choice">
      <input type="radio" name="scheme-{device.id}" checked={!https} onchange={() => choose(false)} />
      <span><strong>HTTP</strong>: only if this Pi-hole doesn't offer HTTPS.</span>
    </label>
  </fieldset>
  {#if !https}
    <p class="alert warn http" role="alert">
      Over HTTP the password, and the session Pi-hole gives home-cc for it, cross your network unencrypted. Anything on
      the network that listens in could read them and take over this Pi-hole. Pi-hole v6 offers HTTPS on port 443:
      use that unless this one really can't.
    </p>
  {/if}

  <div class="row">
    <label class="port">Port <input class="mono" type="number" min="1" max="65535" bind:value={port} /></label>
    <label class="pw">Password
      <input
        class="mono"
        type="password"
        bind:value={password}
        autocomplete="new-password"
        placeholder={stored?.hasPassword ? "Type a new one to replace it" : "App password or web interface password"}
      />
    </label>
  </div>
  {#if stored?.hasPassword}
    <div class="stored">
      <span><Icon path={icons.check} size={14} /> A password is stored.</span>
      <ConfirmButton label="Remove" confirmLabel="Remove password" onconfirm={removePassword} />
    </div>
  {/if}
  <p class="hint">
    home-cc talks to <Address host={device.host} known={device.hostname} /> on this port. The password is kept in your
    system's keychain, never shown again{https ? ", and only sent once you have approved the Pi-hole's certificate" : ""}.
    Best is an app password: in the Pi-hole's web interface open <strong>Settings › Web interface / API</strong>, switch
    the page to <strong>Expert</strong> (top right) and use <strong>Configure app password</strong>; copy the password it
    shows and press <strong>Enable new app password</strong>. It also works when two-factor login is on.
  </p>
  {#if error}<Problem problem={error} />{/if}
  {#if unsaved}<DraftNote ondiscard={discard} />{/if}
  <div class="actions">
    <button class="btn primary" type="submit" disabled={saving}>{saving ? "Saving…" : "Save"}</button>
    <span class="grow"></span>
    <ConfirmButton
      label="Remove from the Pi-hole screen"
      confirmLabel="Yes, remove it"
      title="home-cc logs out and forgets the password. Nothing on the Pi-hole changes."
      onconfirm={removeHost}
    />
  </div>
</form>

<style>
  .setup { display: flex; flex-direction: column; gap: 0.8rem; background: var(--panel-raised); }
  .head { display: flex; align-items: center; justify-content: space-between; gap: 0.6rem; flex-wrap: wrap; }
  .title { margin: 0; font-size: 1rem; font-weight: 600; }
  .scheme { border: 0; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.4rem; min-width: 0; }
  .scheme legend { padding: 0; margin-bottom: 0.35rem; color: var(--text-soft); }
  .choice { flex-direction: row; gap: 0.55rem; align-items: baseline; line-height: 1.45; color: var(--text-soft); cursor: pointer; }
  .choice input { flex: none; width: auto; min-height: 0; margin: 0; }
  .http { margin: 0; line-height: 1.5; }
  .row { display: flex; gap: 0.8rem; flex-wrap: wrap; }
  .port { flex: 0 1 8rem; min-width: 6rem; }
  .pw { flex: 1 1 14rem; min-width: 0; }
  .stored { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem; flex-wrap: wrap; margin-top: -0.3rem; color: var(--text-soft); }
  .stored > span { display: flex; align-items: center; gap: 0.4rem; }
  .stored :global(svg) { color: var(--ok); }
  .hint { margin: 0; font-size: 0.92rem; line-height: 1.5; color: var(--muted); }
  .actions { display: flex; gap: 0.6rem; flex-wrap: wrap; align-items: center; }
  .grow { flex: 1; }
</style>
