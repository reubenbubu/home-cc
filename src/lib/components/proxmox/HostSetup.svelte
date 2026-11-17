<script lang="ts">
  // The API token for one Proxmox host: its ID and port (saved in home-cc) and its secret (kept in the
  // system keychain and never shown again), with how to make one in Proxmox.
  import { errorText, type Device } from "$lib/api";
  import { icons } from "$lib/icons";
  import { DEFAULT_TOKEN_ID, tokenCommands } from "$lib/proxmox";
  import { proxmox } from "$lib/proxmox.svelte";
  import type { Problem as ProblemInfo } from "$lib/report";
  import ConfirmButton from "../ConfirmButton.svelte";
  import Icon from "../Icon.svelte";
  import Problem from "../Problem.svelte";

  let {
    device,
    problem,
    onclose,
  }: {
    device: Device;
    /** Builds a problem with this host's details. */
    problem: (what: string, error: string) => ProblemInfo;
    /** Given when the form can be closed (there is a token already). */
    onclose?: () => void;
  } = $props();

  const stored = $derived(proxmox.settings.get(device.id));
  let tokenId = $state("");
  let port = $state(8006);
  /** Typed here; goes to the keychain on save and is never read back. */
  let secret = $state("");
  let saving = $state(false);
  let error = $state<ProblemInfo | null>(null);
  let filledFor = $state<number | null>(null);

  // Fill the form once the stored settings are in, and again for another host.
  $effect(() => {
    const s = stored;
    if (filledFor === device.id) return;
    if (!s) {
      proxmox.loadSettings(device.id).catch((e) => {
        error = problem("Reading the Proxmox token settings", `Couldn't read the saved settings: ${errorText(e)}`);
      });
      return;
    }
    tokenId = s.tokenId ?? "";
    port = s.port;
    secret = "";
    filledFor = device.id;
  });

  const commands = $derived(tokenCommands(tokenId));

  async function save(e: SubmitEvent) {
    e.preventDefault();
    saving = true;
    error = null;
    try {
      await proxmox.saveSettings(device.id, tokenId.trim(), Number(port) || 8006, secret);
      secret = "";
      onclose?.();
    } catch (err) {
      error = problem("Saving the Proxmox API token", errorText(err));
    } finally {
      saving = false;
    }
  }

  async function removeSecret() {
    error = null;
    try {
      await proxmox.removeSecret(device.id);
    } catch (err) {
      error = problem("Removing the Proxmox token secret", `Couldn't remove the secret: ${errorText(err)}`);
    }
  }
</script>

<form class="panel setup" onsubmit={save}>
  <div class="head">
    <h2 class="section-title">API token for {device.name}</h2>
    {#if onclose}<button type="button" class="btn small" onclick={onclose}>Close</button>{/if}
  </div>

  <div class="cols">
    <div class="fields">
      <label>Token ID
        <input class="mono" bind:value={tokenId} placeholder={DEFAULT_TOKEN_ID} autocomplete="off" spellcheck="false" required />
      </label>
      <label>Secret
        <input
          class="mono"
          type="password"
          bind:value={secret}
          autocomplete="new-password"
          placeholder={stored?.hasSecret ? "Type a new one to replace it" : "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"}
        />
      </label>
      {#if stored?.hasSecret}
        <div class="stored">
          <span><Icon path={icons.check} size={14} /> A secret is stored.</span>
          <ConfirmButton label="Remove" confirmLabel="Remove secret" onconfirm={removeSecret} />
        </div>
      {/if}
      <label class="port">API port <input class="mono" type="number" min="1" max="65535" bind:value={port} /></label>
      <p class="hint">
        home-cc talks to {device.host} on this port (Proxmox uses 8006). The secret is kept in your system's keychain,
        never shown again, and only sent once you have approved the host's certificate.
      </p>
      {#if error}<Problem problem={error} />{/if}
      <div class="actions">
        <button class="btn primary" type="submit" disabled={saving}>{saving ? "Saving…" : "Save"}</button>
      </div>
    </div>

    <div class="howto">
      <h3>How to make a token</h3>
      <ol>
        <li>
          In Proxmox's web interface go to <strong>Datacenter › Permissions › API Tokens › Add</strong>. Pick the user
          (for example root@pam), name the token (for example homecc) and leave <strong>Privilege Separation</strong>
          ticked. Copy the secret: Proxmox shows it only once.
        </li>
        <li>
          Under <strong>Datacenter › Permissions › Add › API Token Permission</strong>, give the token the role
          <strong>PVEAuditor</strong> on path <span class="mono">/</span> (to see nodes, storage and guests), and again
          <strong>PVEVMAdmin</strong> (to start, stop and take snapshots).
        </li>
      </ol>
      <p>Or run these two lines in the Proxmox host's shell; the first prints the secret:</p>
      <pre class="mono cmd">{commands.create}
{commands.grant}</pre>
    </div>
  </div>
</form>

<style>
  .setup { display: flex; flex-direction: column; gap: 0.9rem; container-type: inline-size; }
  .head { display: flex; align-items: center; justify-content: space-between; gap: 1rem; }
  .head .section-title { margin: 0; }
  .cols { display: grid; gap: 1.2rem; }
  @container (min-width: 60rem) {
    .cols { grid-template-columns: minmax(0, 26rem) minmax(0, 1fr); }
  }
  .fields { display: flex; flex-direction: column; gap: 0.9rem; min-width: 0; }
  .port { max-width: 12rem; }
  .stored { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem; flex-wrap: wrap; margin-top: -0.4rem; color: var(--text-soft); }
  .stored > span { display: flex; align-items: center; gap: 0.4rem; }
  .stored :global(svg) { color: var(--ok); }
  .stored :global(.btn) { margin-left: auto; }
  .hint { margin: 0; font-size: 0.92rem; line-height: 1.5; color: var(--muted); }
  .actions { display: flex; gap: 0.6rem; }
  .howto { min-width: 0; line-height: 1.55; color: var(--text-soft); }
  .howto h3 { margin: 0 0 0.4rem; font-size: 1rem; font-weight: 600; color: var(--text); }
  .howto ol { margin: 0 0 0.6rem; padding-left: 1.3rem; display: flex; flex-direction: column; gap: 0.4rem; }
  .howto p { margin: 0 0 0.4rem; }
  .cmd {
    margin: 0;
    padding: 0.6rem 0.75rem;
    border-radius: var(--radius-sm);
    background: var(--bg-sunk);
    border: 1px solid var(--border);
    font-size: 0.9rem;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }
</style>
