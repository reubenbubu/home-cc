// Up or down for every device, with recent history. Reloaded when the back end
// says a round of checks has finished.
import { api, errorText, type Device, type DeviceStatus } from "./api";
import { isThisPc, ownAddresses } from "./net";
import { scan } from "./scan.svelte";
import { settings } from "./settings.svelte";

export type Health = "up" | "down" | "unknown";

/** Unix seconds of the newest check in a list, or 0 for none. */
function newest(list: DeviceStatus[]): number {
  return list.reduce((max, s) => Math.max(max, s.checkedAt), 0);
}

class StatusStore {
  list = $state<DeviceStatus[]>([]);
  loaded = $state(false);
  error = $state<string | null>(null);
  /** Set while status checks can't work at all (for example this PC isn't allowed to send pings):
   *  what is wrong and what to do about it, in the back end's words. */
  problem = $state<string | null>(null);
  /** True while a "Check now" round is running. */
  checking = $state(false);
  /** Why the last "Check now" was refused. Dropped once a later round of checks has come in. */
  checkError = $state<string | null>(null);
  byId = $derived(new Map(this.list.map((s) => [s.deviceId, s])));
  /** True once the back end has said which device is this PC. */
  #anySelf = $derived(this.list.some((s) => s.isSelf));
  #own = $derived(ownAddresses(settings.subnets, scan.result?.found));
  #seq = 0;
  /** The newest check on screen when "Check now" was refused. */
  #refusedAfter = 0;

  async load() {
    const seq = ++this.#seq;
    // Asked for separately, so a problem reading one doesn't hide the other.
    const [list, problem] = await Promise.allSettled([api.deviceStatuses(), api.checkProblem()]);
    this.loaded = true;
    if (seq !== this.#seq) return;
    if (list.status === "fulfilled") {
      this.list = list.value;
      if (this.checkError !== null && newest(list.value) > this.#refusedAfter) this.checkError = null;
    }
    if (problem.status === "fulfilled") this.problem = problem.value || null;
    const failed = list.status === "rejected" ? list : problem.status === "rejected" ? problem : null;
    this.error = failed ? errorText(failed.reason) : null;
  }

  /** Ignored devices are never checked, so a reading left over from before is not shown. */
  get(d: Device): DeviceStatus | undefined {
    return d.kind === "ignore" ? undefined : this.byId.get(d.id);
  }

  health(d: Device): Health {
    const s = this.get(d);
    return s ? (s.up ? "up" : "down") : "unknown";
  }

  /** True for the PC home-cc runs on. It is never checked over the network (the back end just reports it
   *  as there, with no response time), so it must not be described as a device that doesn't answer.
   *  The back end marks it in each status; the guess from this PC's addresses only fills in before that. */
  isSelf(d: Device): boolean {
    return isThisPc(d.host, this.get(d)?.isSelf, this.#anySelf, this.#own);
  }

  /** Check every device now. The back end refuses while a scan is running, or when checks can't work. */
  async checkNow() {
    if (this.checking) return;
    this.checking = true;
    this.checkError = null;
    let refused: string | null = null;
    try {
      await api.checkNow();
    } catch (e) {
      refused = errorText(e);
    }
    // Also after a refusal: the reason may be a standing problem that has just been noticed.
    await this.load();
    if (refused !== null) {
      this.#refusedAfter = newest(this.list);
      this.checkError = refused;
    }
    this.checking = false;
  }
}

export const status = new StatusStore();
