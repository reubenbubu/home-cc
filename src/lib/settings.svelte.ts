// App settings and the list of local networks, shared by every screen.
import { api, errorText, type Settings, type Subnet } from "./api";
import { rangeSpanText, scanRanges } from "./net";

/** Same as the back end's defaults; shown until the saved ones have loaded. */
export const DEFAULT_SETTINGS: Settings = {
  pingIntervalSecs: 30,
  statsIntervalSecs: 60,
  scanRange: "",
  diskWarnPct: 85,
  ramWarnPct: 90,
  sshKeyPath: "",
};

class SettingsStore {
  value = $state<Settings>({ ...DEFAULT_SETTINGS });
  subnets = $state<Subnet[]>([]);
  loaded = $state(false);
  #valueError = $state<string | null>(null);
  #subnetsError = $state<string | null>(null);
  /** Why the saved settings or the list of networks couldn't be read. */
  error = $derived(this.#valueError ?? this.#subnetsError);
  #subnetsSeq = 0;
  /** The ranges to pick from: one per network, however many adapters are on it. */
  ranges = $derived(scanRanges(this.subnets));
  /** The network a scan uses when no range is set. */
  bestSubnet = $derived(this.subnets.find((s) => s.best) ?? this.subnets[0] ?? null);
  /** The range a scan will cover, in words. */
  scanRangeLabel = $derived(
    this.value.scanRange || (this.bestSubnet ? `automatic: ${this.bestSubnet.cidr}` : "automatic"),
  );
  /** The addresses that range covers, such as "192.168.68.1 to 192.168.71.254 (1,022 addresses)"; "" when
   *  it isn't known yet. */
  scanRangeSpan = $derived(rangeSpanText(this.value.scanRange || this.bestSubnet?.cidr || ""));

  async load() {
    // Loaded separately so a problem listing networks doesn't hide the saved settings.
    const [value] = await Promise.allSettled([api.getSettings(), this.loadSubnets()]);
    if (value.status === "fulfilled") this.value = value.value;
    this.#valueError = value.status === "rejected" ? errorText(value.reason) : null;
    this.loaded = true;
  }

  /** This PC's networks can change while the app is open (cable to Wi-Fi, a new address), so they are
   *  read again when the Settings screen opens and after every scan. Never rejects. */
  async loadSubnets() {
    // Reloads can overlap: the newest wins.
    const seq = ++this.#subnetsSeq;
    try {
      const subnets = await api.listSubnets();
      if (seq !== this.#subnetsSeq) return;
      this.subnets = subnets;
      this.#subnetsError = null;
    } catch (e) {
      if (seq === this.#subnetsSeq) this.#subnetsError = errorText(e);
    }
  }

  /** Rejects with the back end's message when a value is refused. */
  async save(next: Settings): Promise<Settings> {
    this.value = await api.saveSettings(next);
    return this.value;
  }
}

export const settings = new SettingsStore();
