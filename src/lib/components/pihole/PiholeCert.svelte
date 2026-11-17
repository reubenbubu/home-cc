<script lang="ts">
  // Asks the owner to check a Pi-hole's certificate: on first contact over HTTPS, or when it isn't the one they
  // approved. Nothing, not even the password, is sent to the Pi-hole until they approve.
  import type { Device } from "$lib/api";
  import type { PiholeStatus } from "$lib/pihole/api";
  import { pihole } from "$lib/pihole/store.svelte";
  import Address from "../Address.svelte";
  import ConfirmButton from "../ConfirmButton.svelte";

  let { device, status }: { device: Device; status: PiholeStatus } = $props();

  /** Prints the certificate Pi-hole v6 makes for itself, in the form shown here. */
  const COMMAND = "sudo openssl x509 -in /etc/pihole/tls.pem -noout -fingerprint -sha256";

  const offered = $derived(status.offeredFingerprint ?? "");
  const changed = $derived(status.errorKind === "certChanged");
  const busy = $derived(pihole.trusting.has(device.id) || pihole.reading.has(device.id));
</script>

<div class="alert {changed ? 'warn' : 'ask'} cert" role={changed ? "alert" : "status"}>
  {#if changed}
    <p><strong>{device.name} shows a different certificate from the one you approved.</strong></p>
    <p>
      That is expected if you reinstalled Pi-hole, gave it a new certificate, or pointed this device at another
      address. If you didn't, something else may be answering at <Address host={device.host} known={device.hostname} />.
      home-cc has sent it nothing, and won't until you approve the new certificate.
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
      Pi-hole makes its own certificate, so home-cc can't tell by itself that this really is your Pi-hole. The password
      has not been sent, and won't be until you approve. The Pi-hole shows this certificate (SHA-256):
    </p>
    <p class="mono print">{offered}</p>
  {/if}
  <p class="muted small how">
    To check, run <span class="mono cmd">{COMMAND}</span> on the Pi-hole (in its container's shell if it runs in
    Docker, or with <span class="mono cmd">docker exec</span>). It prints "sha256 Fingerprint=" and the same text as
    {changed ? '"Now"' : "above"}. If you gave Pi-hole a certificate of your own, check that one instead.
  </p>
  <div class="confirm">
    <!-- Drawn afresh for each certificate, so a half-given approval never carries over to one not yet read. -->
    {#key offered}
      <ConfirmButton
        label={changed ? "Approve the new certificate" : "Approve this certificate"}
        confirmLabel="Yes, it matches"
        disabled={busy || !offered}
        onconfirm={() => pihole.trust(device.id, offered)}
      />
    {/key}
  </div>
</div>

<style>
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
