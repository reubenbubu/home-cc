// App settings and the list of local networks, shared by every screen.
import { api, errorText, type Settings, type Subnet } from "./api";

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
  error = $state<string | null>(null);
  /** The network a scan uses when no range is set. */
  bestSubnet = $derived(this.subnets.find((s) => s.best) ?? this.subnets[0] ?? null);
  /** The range a scan will cover, in words. */
  scanRangeLabel = $derived(
    this.value.scanRange || (this.bestSubnet ? `automatic: ${this.bestSubnet.cidr}` : "automatic"),
  );

  async load() {
    // Loaded separately so a problem listing networks doesn't hide the saved settings.
    const [value, subnets] = await Promise.allSettled([api.getSettings(), api.listSubnets()]);
    if (value.status === "fulfilled") this.value = value.value;
    if (subnets.status === "fulfilled") this.subnets = subnets.value;
    const failed = value.status === "rejected" ? value : subnets.status === "rejected" ? subnets : null;
    this.error = failed ? errorText(failed.reason) : null;
    this.loaded = true;
  }

  /** Rejects with the back end's message when a value is refused. */
  async save(next: Settings): Promise<Settings> {
    this.value = await api.saveSettings(next);
    return this.value;
  }
}

export const settings = new SettingsStore();
