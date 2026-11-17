// CPU, RAM and disk readings for machines that have an SSH user. Reloaded when
// the back end says new readings are ready.
import { SvelteMap, SvelteSet } from "svelte/reactivity";
import { api, errorText, type MachineStatsEntry } from "./api";

class StatsStore {
  list = $state<MachineStatsEntry[]>([]);
  loaded = $state(false);
  error = $state<string | null>(null);
  byId = $derived(new Map(this.list.map((e) => [e.deviceId, e])));
  /** Machines being read right now because the owner asked. */
  refreshing = new SvelteSet<number>();
  /** Why a refresh the owner asked for could not even be tried, by machine. */
  refreshErrors = new SvelteMap<number, string>();
  #seq = 0;

  async load() {
    const seq = ++this.#seq;
    try {
      const list = await api.machineStats();
      if (seq !== this.#seq) return;
      this.list = list;
      this.error = null;
    } catch (e) {
      if (seq === this.#seq) this.error = errorText(e);
    } finally {
      this.loaded = true;
    }
  }

  get(id: number): MachineStatsEntry | undefined {
    return this.byId.get(id);
  }

  /** Read one machine now. A failed reading comes back inside the entry; only a refused request ends up in refreshErrors. */
  async refresh(id: number) {
    if (this.refreshing.has(id)) return;
    this.refreshing.add(id);
    this.refreshErrors.delete(id);
    try {
      const entry = await api.refreshStats(id);
      // Drop the answer of any reload that started before this one came back.
      this.#seq++;
      this.list = [...this.list.filter((e) => e.deviceId !== id), entry];
    } catch (e) {
      this.refreshErrors.set(id, errorText(e));
    } finally {
      this.refreshing.delete(id);
    }
  }

  /** After the owner has confirmed: forget the stored identity and read again. */
  async trustAgain(id: number) {
    this.refreshErrors.delete(id);
    try {
      await api.forgetHostKey(id);
    } catch (e) {
      this.refreshErrors.set(id, errorText(e));
      return;
    }
    await this.refresh(id);
  }
}

export const stats = new StatsStore();
