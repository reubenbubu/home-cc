// State of the network scan. Lives outside the screens so a scan started on
// one screen is still shown, with its result, after moving to another.
import { api, errorText, type ScanProgress, type ScanResult } from "./api";
import { settings } from "./settings.svelte";

export const PHASE_TEXT: Record<ScanProgress["phase"], string> = {
  sweep: "Checking every address",
  confirm: "Taking a second look at quiet devices",
  names: "Looking up device names",
};

const PHASES: ScanProgress["phase"][] = ["sweep", "confirm", "names"];

class ScanStore {
  running = $state(false);
  /** True after Stop was pressed, until the scan winds down. */
  stopping = $state(false);
  progress = $state<ScanProgress | null>(null);
  result = $state<ScanResult | null>(null);
  /** Unix seconds when the last scan finished. */
  finishedAt = $state<number | null>(null);
  error = $state<string | null>(null);
  /** 1 to 3, for "Step 2 of 3". */
  step = $derived(this.progress ? Math.max(1, PHASES.indexOf(this.progress.phase) + 1) : 1);
  /** True when this window started the scan and is waiting for its result. */
  #own = false;

  /** At start: pick up a scan that was already running (for example after the screens reloaded). */
  async init() {
    try {
      const running = await api.scanRunning();
      if (!this.#own) this.running = running;
    } catch (e) {
      this.error = errorText(e);
    }
  }

  onProgress(p: ScanProgress) {
    // A late event after the scan has ended must not bring the progress bar back.
    if (this.running) this.progress = p;
  }

  /** The inventory changed. If a scan we didn't start was running, see whether it is over. */
  async onInventoryChanged() {
    if (!this.running || this.#own) return;
    try {
      const running = await api.scanRunning();
      if (this.#own) return;
      this.running = running;
      if (!running) {
        this.progress = null;
        settings.loadSubnets();
      }
    } catch {
      // The next event asks again.
    }
  }

  async start() {
    if (this.running) return;
    this.running = true;
    this.#own = true;
    this.stopping = false;
    this.progress = null;
    this.error = null;
    try {
      this.result = await api.startScan();
      this.finishedAt = Math.floor(Date.now() / 1000);
    } catch (e) {
      this.error = errorText(e);
    } finally {
      this.#own = false;
      this.running = false;
      this.stopping = false;
      this.progress = null;
      // This PC's networks may have changed since they were last read (see settings.loadSubnets).
      settings.loadSubnets();
    }
  }

  async stop() {
    if (!this.running || this.stopping) return;
    this.stopping = true;
    try {
      await api.cancelScan();
    } catch (e) {
      this.stopping = false;
      this.error = errorText(e);
    }
  }
}

export const scan = new ScanStore();
