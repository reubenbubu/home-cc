// The Pi-hole screen's data (and the Overview's): which devices are Pi-holes, each one's latest reading, read
// again every 30 seconds while a screen that shows them is open and the window is in view, and what the owner
// does to them.
import { SvelteMap, SvelteSet } from "svelte/reactivity";
import { errorText } from "$lib/api";
import { OneAtATime } from "$lib/running";
import { piholeApi, type PiholeReading, type PiholeSettings, type PiholeStatus, type Probe } from "./api";
import { nextPollSecs } from "./logic";

/** The last reading that worked, kept on screen (marked as old) while later ones fail. */
export interface GoodReading {
  reading: PiholeReading;
  fetchedAt: number;
}

class PiholeStore {
  /** Devices marked as Pi-holes, by device id order. */
  hosts = $state<PiholeSettings[]>([]);
  loaded = $state(false);
  loadError = $state<string | null>(null);
  /** The latest answer for each Pi-hole, which may hold an error. */
  statuses = new SvelteMap<number, PiholeStatus>();
  lastGood = new SvelteMap<number, GoodReading>();
  /** Why a reading could not even be asked for. */
  readErrors = new SvelteMap<number, string>();
  /** Pi-holes being read right now. */
  reading = new SvelteSet<number>();
  trusting = new SvelteSet<number>();
  trustErrors = new SvelteMap<number, string>();
  /** What is being done to blocking right now, by device id. */
  switching = new SvelteMap<number, string>();
  /** What came of the owner's last pause or resume, by device id. */
  switchErrors = new SvelteMap<number, string>();
  /** Looking for Pi-holes on the saved machines. */
  probing = $state(false);
  probes = $state<Probe[] | null>(null);
  probeError = $state<string | null>(null);
  #reads = new OneAtATime<number>();

  get ids(): Set<number> {
    return new Set(this.hosts.map((h) => h.deviceId));
  }

  settings(id: number): PiholeSettings | undefined {
    return this.hosts.find((h) => h.deviceId === id);
  }

  async load() {
    try {
      const list = await piholeApi.hosts();
      this.hosts = list;
      this.loadError = null;
      // A Pi-hole taken off the list (or its device removed) leaves nothing behind.
      const ids = new Set(list.map((h) => h.deviceId));
      for (const map of [this.statuses, this.lastGood, this.readErrors, this.trustErrors, this.switchErrors]) {
        for (const id of [...map.keys()]) if (!ids.has(id)) map.delete(id);
      }
    } catch (e) {
      this.loadError = errorText(e);
    } finally {
      this.loaded = true;
    }
  }

  #setSettings(s: PiholeSettings) {
    const i = this.hosts.findIndex((h) => h.deviceId === s.deviceId);
    this.hosts = i < 0 ? [...this.hosts, s].sort((a, b) => a.deviceId - b.deviceId) : this.hosts.map((h, j) => (j === i ? s : h));
  }

  /** Read one Pi-hole now; a second call while a reading is going waits for that one. */
  refresh(id: number): Promise<void> {
    return this.#reads.run(id, async () => {
      this.reading.add(id);
      try {
        const status = await piholeApi.status(id);
        this.statuses.set(id, status);
        this.readErrors.delete(id);
        if (status.reading && status.fetchedAt != null) {
          this.lastGood.set(id, { reading: status.reading, fetchedAt: status.fetchedAt });
          this.trustErrors.delete(id);
        }
        const s = this.settings(id);
        if (s && s.pinnedFingerprint !== status.pinnedFingerprint) this.#setSettings({ ...s, pinnedFingerprint: status.pinnedFingerprint });
      } catch (e) {
        this.readErrors.set(id, errorText(e));
      } finally {
        this.reading.delete(id);
      }
    });
  }

  refreshAll(): Promise<void> {
    return Promise.all(this.hosts.map((h) => this.refresh(h.deviceId))).then(() => {});
  }

  /** Seconds until the next round: sooner when a pause is about to end, so it is read again just after Pi-hole
   *  switched blocking back on. Only Pi-holes whose latest reading worked count (see nextPollSecs). */
  #nextWait(): number {
    const current: { timerSecs: number | null; fetchedAt: number | null }[] = [];
    for (const id of this.ids) {
      const s = this.statuses.get(id);
      if (!s?.reading || this.readErrors.has(id)) continue;
      current.push({ timerSecs: s.reading.blocking.timerSecs, fetchedAt: s.fetchedAt });
    }
    return nextPollSecs(current, Date.now() / 1000);
  }

  /** Every Pi-hole, read now and then every POLL_SECS (sooner when a pause is about to end) while the window
   *  is in view. Returns a function that stops. */
  watchAll(): () => void {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const round = async () => {
      if (!document.hidden) {
        await this.load();
        if (!stopped) await this.refreshAll();
      }
      if (stopped) return;
      timer = setTimeout(round, this.#nextWait() * 1000);
    };
    void round();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }

  /** Marks a device as a Pi-hole. The owner picked it. */
  async add(id: number): Promise<void> {
    this.#setSettings(await piholeApi.add(id));
    this.probes = this.probes?.filter((p) => p.deviceId !== id) ?? null;
    void this.refresh(id);
  }

  async remove(id: number): Promise<void> {
    await piholeApi.remove(id);
    this.hosts = this.hosts.filter((h) => h.deviceId !== id);
    await this.load();
  }

  /** Saves HTTPS or HTTP, the port and the password if one was typed, then reads the Pi-hole again. */
  async save(id: number, https: boolean, port: number, password: string): Promise<void> {
    this.#setSettings(await piholeApi.saveSettings(id, https, port, password ? password : null));
    await this.#reads.settled(id);
    await this.refresh(id);
  }

  async removePassword(id: number): Promise<void> {
    this.#setSettings(await piholeApi.removePassword(id));
    await this.#reads.settled(id);
    await this.refresh(id);
  }

  /** Approves exactly the certificate the owner was shown, then reads the Pi-hole with it. */
  async trust(id: number, fingerprint: string) {
    if (this.trusting.has(id)) return;
    this.trusting.add(id);
    this.trustErrors.delete(id);
    try {
      try {
        await piholeApi.trustCertificate(id, fingerprint);
      } catch (e) {
        this.trustErrors.set(id, errorText(e));
      }
      // A reading already going started without the approval; the one after it uses it.
      await this.#reads.settled(id);
      await this.refresh(id);
    } finally {
      this.trusting.delete(id);
    }
  }

  /** Switches blocking on, or off for `secs` seconds (null: until switched on), on each Pi-hole given, then
   *  reads them again. The screen has asked the owner before switching it off. */
  async setBlocking(ids: number[], blocking: boolean, secs: number | null): Promise<boolean> {
    const doing = blocking ? "Turning blocking on…" : "Pausing…";
    const todo = ids.filter((id) => !this.switching.has(id));
    for (const id of todo) {
      this.switching.set(id, doing);
      this.switchErrors.delete(id);
    }
    const results = await Promise.all(
      todo.map(async (id) => {
        try {
          await piholeApi.setBlocking(id, blocking, blocking ? null : secs);
          return true;
        } catch (e) {
          this.switchErrors.set(id, errorText(e));
          return false;
        } finally {
          this.switching.delete(id);
        }
      }),
    );
    await Promise.all(todo.map(async (id) => (await this.#reads.settled(id), this.refresh(id))));
    return results.every(Boolean);
  }

  /** Asks each of these machines whether it answers like a Pi-hole. */
  async probe(ids: number[]) {
    if (this.probing) return;
    this.probing = true;
    this.probeError = null;
    try {
      const found: Probe[] = [];
      // The back end takes at most 64 at a time.
      for (let i = 0; i < ids.length; i += 64) found.push(...(await piholeApi.probe(ids.slice(i, i + 64))));
      this.probes = found;
    } catch (e) {
      this.probeError = errorText(e);
    } finally {
      this.probing = false;
    }
  }
}

export const pihole = new PiholeStore();
