<script lang="ts">
  // Asks the owner to check a Proxmox host's certificate: on first contact, or when it isn't the one they
  // approved. Nothing, not even the API token, is sent to the host until they approve.
  import type { Device, ProxmoxStatus } from "$lib/api";
  import { CERT_COMMAND, CUSTOM_CERT_COMMAND } from "$lib/proxmox";
  import { proxmox } from "$lib/proxmox.svelte";
  import ConfirmButton from "../ConfirmButton.svelte";

  let { device, status }: { device: Device; status: ProxmoxStatus } = $props();

  const offered = $derived(status.offeredFingerprint ?? "");
  const changed = $derived(status.errorKind === "certChanged");
  const busy = $derived(proxmox.trusting.has(device.id) || proxmox.reading.has(device.id));
</script>

<div class="alert {changed ? 'warn' : 'ask'} cert" role={changed ? "alert" : "status"}>
  {#if changed}
    <p><strong>{device.name} shows a different certificate from the one you approved.</strong></p>
    <p>
      That is expected if you reinstalled Proxmox, renewed or replaced its certificate, or pointed this device at
      another address. If you didn't, something else may be answering at {device.host}. home-cc has sent it nothing,
      and won't until you approve the new certificate.
    </p>
    <dl class="prints">
      <dt>Was</dt>
      <dd class="mono">{status.expectedFingerprint ?? "not known"}</dd>
      <dt>Now</dt>
      <dd class="mono">{offered}</dd>
    </dl>
  {:else}
    <p><strong>First connection to {device.name}: check its certificate.</strong></p>
    <p>
      Proxmox makes its own certificate, so home-cc can't tell by itself that this really is your Proxmox host. The
      API token has not been sent, and won't be until you approve. The host shows this certificate (SHA-256):
    </p>
    <p class="mono print">{offered}</p>
  {/if}
  <p class="muted small how">
    To check, run <span class="mono cmd">{CERT_COMMAND}</span> in the Proxmox host's shell. If you uploaded your own
    certificate or use Let's Encrypt, use <span class="mono cmd">{CUSTOM_CERT_COMMAND}</span> instead. It prints
    "sha256 Fingerprint=" and the same text as {changed ? '"Now"' : "above"}. In the web interface the same is under the
    node's System › Certificates.
  </p>
  <div class="confirm">
    <!-- Drawn afresh for each certificate, so a half-given approval never carries over to one not yet read. -->
    {#key offered}
      <ConfirmButton
        label={changed ? "Approve the new certificate" : "Approve this certificate"}
        confirmLabel="Yes, it matches"
        disabled={busy || !offered}
        onconfirm={() => proxmox.trust(device.id, offered)}
      />
    {/key}
  </div>
</div>

<style>
  .cert { max-width: 80rem; }
  .cert p { margin: 0; line-height: 1.5; }
  .cert p + p { margin-top: 0.35rem; }
  .prints { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 0.2rem 0.7rem; margin: 0.5rem 0; }
  .prints dt { color: var(--muted); }
  .prints dd { margin: 0; }
  .prints dd, .print, .cmd { font-size: 0.92rem; overflow-wrap: anywhere; user-select: text; }
  .print { margin: 0.3rem 0 0.4rem !important; }
  .cmd { color: var(--text-soft); font-size: 0.9em; }
  .how { margin-top: 0.5rem !important; }
  .confirm { display: flex; gap: 0.4rem; flex-wrap: wrap; margin-top: 0.7rem; }
</style>
