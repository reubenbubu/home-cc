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
  /** Machines whose identity is being stored right now. */
  trusting = new SvelteSet<number>();
  /** Why an identity the owner approved was not stored, by machine. */
  trustErrors = new SvelteMap<number, string>();
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
    this.trustErrors.delete(id);
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

  /** After the owner has looked at an identity and confirmed it: store exactly that one, then read again.
   *  The back end refuses if the machine has shown a different identity since; the entry is then
   *  loaded again so the owner sees the one to judge, with the reason in trustErrors. */
  async trust(id: number, fingerprint: string) {
    if (this.trusting.has(id)) return;
    this.trusting.add(id);
    this.trustErrors.delete(id);
    this.refreshErrors.delete(id);
    try {
      await api.trustHostKey(id, fingerprint);
    } catch (e) {
      this.trustErrors.set(id, errorText(e));
      await this.load();
      return;
    } finally {
      this.trusting.delete(id);
    }
    await this.refresh(id);
  }
}

export const stats = new StatsStore();
