// The Proxmox screen's data: each host's latest reading, read again every few
// seconds while the screen is open, and the actions the owner starts.
import { SvelteMap, SvelteSet } from "svelte/reactivity";
import {
  api,
  errorText,
  proxmoxApi,
  secretKeys,
  type GuestRef,
  type PowerAction,
  type ProxmoxOverview,
  type ProxmoxSettings,
  type ProxmoxStatus,
} from "./api";
import { ACTION_BUSY, guestKey } from "./proxmox";
import { OneAtATime } from "./running";

/** Seconds between readings while the screen is open. */
export const POLL_SECS = 5;

/** The last reading that worked, kept on screen (marked as old) while later ones fail. */
export interface GoodReading {
  overview: ProxmoxOverview;
  fetchedAt: number;
}

class ProxmoxStore {
  /** The latest answer for each host, which may hold an error. */
  statuses = new SvelteMap<number, ProxmoxStatus>();
  lastGood = new SvelteMap<number, GoodReading>();
  /** Why a reading could not even be asked for (the host isn't a Proxmox host any more, say). */
  readErrors = new SvelteMap<number, string>();
  /** Hosts being read right now. */
  reading = new SvelteSet<number>();
  settings = new SvelteMap<number, ProxmoxSettings>();
  /** Hosts whose certificate the owner has just approved. */
  trusting = new SvelteSet<number>();
  trustErrors = new SvelteMap<number, string>();
  /** What is being done to a guest right now, by guestKey: shown on its buttons. */
  busy = new SvelteMap<string, string>();
  /** What came of the owner's last action on a guest, by guestKey. */
  outcomes = new SvelteMap<string, { ok: boolean; text: string }>();
  #reads = new OneAtATime<number>();

  /** Read one host now; a second call while a reading is going waits for that one. */
  refresh(deviceId: number): Promise<void> {
    return this.#reads.run(deviceId, async () => {
      this.reading.add(deviceId);
      try {
        const status = await proxmoxApi.status(deviceId);
        this.statuses.set(deviceId, status);
        this.readErrors.delete(deviceId);
        if (status.overview && status.fetchedAt != null) {
          this.lastGood.set(deviceId, { overview: status.overview, fetchedAt: status.fetchedAt });
          this.trustErrors.delete(deviceId);
        }
      } catch (e) {
        this.readErrors.set(deviceId, errorText(e));
      } finally {
        this.reading.delete(deviceId);
      }
    });
  }

  /** Reads the host now and then every POLL_SECS seconds, until the returned function is called. A slow
   *  host is never asked twice at once: the next reading is timed from the end of the last one. */
  watch(deviceId: number): () => void {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const round = async () => {
      await this.refresh(deviceId);
      if (!stopped) timer = setTimeout(round, POLL_SECS * 1000);
    };
    round();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }

  async loadSettings(deviceId: number): Promise<ProxmoxSettings> {
    const s = await proxmoxApi.settings(deviceId);
    this.settings.set(deviceId, s);
    return s;
  }

  /** Saves the token ID and port, then the secret if one was typed, then reads the host again. */
  async saveSettings(deviceId: number, tokenId: string, port: number, secret: string): Promise<void> {
    let saved = await proxmoxApi.saveSettings(deviceId, tokenId, port);
    this.settings.set(deviceId, saved);
    if (secret.trim()) {
      try {
        await api.setSecret(secretKeys.proxmoxToken(deviceId), secret.trim());
      } catch (e) {
        throw `The token ID was saved, but the secret couldn't be stored in the keychain: ${errorText(e)}`;
      }
      saved = { ...saved, hasSecret: true };
      this.settings.set(deviceId, saved);
    }
    await this.refresh(deviceId);
  }

  async removeSecret(deviceId: number): Promise<void> {
    await api.deleteSecret(secretKeys.proxmoxToken(deviceId));
    const s = this.settings.get(deviceId);
    if (s) this.settings.set(deviceId, { ...s, hasSecret: false });
    await this.refresh(deviceId);
  }

  /** Approves exactly the certificate the owner was shown, then reads the host with it. */
  async trust(deviceId: number, fingerprint: string) {
    if (this.trusting.has(deviceId)) return;
    this.trusting.add(deviceId);
    this.trustErrors.delete(deviceId);
    try {
      try {
        await proxmoxApi.trustCertificate(deviceId, fingerprint);
      } catch (e) {
        this.trustErrors.set(deviceId, errorText(e));
        // Show what the host presents now, for the owner to judge.
        await this.#reads.settled(deviceId);
        await this.refresh(deviceId);
        return;
      }
      // A reading already going started without the approval; the one after it uses it.
      await this.#reads.settled(deviceId);
      await this.refresh(deviceId);
      const s = this.settings.get(deviceId);
      if (s) this.settings.set(deviceId, { ...s, pinnedFingerprint: fingerprint });
    } finally {
      this.trusting.delete(deviceId);
    }
  }

  /** Runs a change on one guest, showing `busyText` on it meanwhile, then reads the host again. */
  async act(deviceId: number, guest: GuestRef, busyText: string, done: string, job: () => Promise<{ finished: boolean }>) {
    const key = guestKey(guest);
    if (this.busy.has(key)) return false;
    this.busy.set(key, busyText);
    this.outcomes.delete(key);
    try {
      const outcome = await job();
      this.outcomes.set(key, {
        ok: true,
        text: outcome.finished ? done : "Proxmox is still working on it. The list shows the result when it's done.",
      });
      return true;
    } catch (e) {
      this.outcomes.set(key, { ok: false, text: errorText(e) });
      return false;
    } finally {
      this.busy.delete(key);
      this.refresh(deviceId);
    }
  }

  power(deviceId: number, guest: GuestRef, action: PowerAction) {
    const done: Record<PowerAction, string> = {
      start: "Started.",
      shutdown: "Shut down.",
      reboot: "Restarted.",
      stop: "Stopped.",
    };
    return this.act(deviceId, guest, ACTION_BUSY[action], done[action], () => proxmoxApi.power(deviceId, guest, action));
  }
}

export const proxmox = new ProxmoxStore();
