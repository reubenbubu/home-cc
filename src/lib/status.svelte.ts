// Up or down for every device, with recent history. Reloaded when the back end
// says a round of checks has finished.
import { api, errorText, type Device, type DeviceStatus } from "./api";

export type Health = "up" | "down" | "unknown";

class StatusStore {
  list = $state<DeviceStatus[]>([]);
  loaded = $state(false);
  error = $state<string | null>(null);
  /** True while a "Check now" round is running. */
  checking = $state(false);
  checkError = $state<string | null>(null);
  byId = $derived(new Map(this.list.map((s) => [s.deviceId, s])));
  #seq = 0;

  async load() {
    const seq = ++this.#seq;
    try {
      const list = await api.deviceStatuses();
      if (seq !== this.#seq) return;
      this.list = list;
      this.error = null;
    } catch (e) {
      if (seq === this.#seq) this.error = errorText(e);
    } finally {
      this.loaded = true;
    }
  }

  /** Ignored devices are never checked, so a reading left over from before is not shown. */
  get(d: Device): DeviceStatus | undefined {
    return d.kind === "ignore" ? undefined : this.byId.get(d.id);
  }

  health(d: Device): Health {
    const s = this.get(d);
    return s ? (s.up ? "up" : "down") : "unknown";
  }

  async checkNow() {
    if (this.checking) return;
    this.checking = true;
    this.checkError = null;
    try {
      await api.checkNow();
      await this.load();
    } catch (e) {
      this.checkError = errorText(e);
    } finally {
      this.checking = false;
    }
  }
}

export const status = new StatusStore();
