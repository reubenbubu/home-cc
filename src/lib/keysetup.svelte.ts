// "Set up key login" on a machine card: opens the Machines form on that machine with its key login part
// open (components/KeySetup.svelte picks the request up).

class KeySetupAsk {
  /** The machine whose key login part should open, until it has. */
  deviceId = $state<number | null>(null);

  ask(deviceId: number) {
    this.deviceId = deviceId;
  }

  /** True once for the machine that was asked for; the request is then done. */
  take(deviceId: number): boolean {
    if (this.deviceId !== deviceId) return false;
    this.deviceId = null;
    return true;
  }
}

export const keySetupAsk = new KeySetupAsk();

/** Where the key login part of the Machines form keeps whether it was open, and a passphrase typed for a new
 *  key, while another screen is shown (drafts.svelte.ts). Cleared with the rest of the form. */
export function keySetupDraft(deviceId: number | "new"): string {
  return `key-setup:${deviceId}`;
}
