// CPU, RAM and disk readings for machines that have an SSH user, and for this
// PC, which is read without SSH. Reloaded when the back end says new readings
// are ready.
import { SvelteMap, SvelteSet } from "svelte/reactivity";
import { api, errorText, type Device, type MachineStatsEntry } from "./api";
import { OneAtATime } from "./running";
import { status } from "./status.svelte";

class StatsStore {
  list = $state<MachineStatsEntry[]>([]);
  loaded = $state(false);
  error = $state<string | null>(null);
  byId = $derived(new Map(this.list.map((e) => [e.deviceId, e])));
  /** Machines being read right now because the owner asked. */
  refreshing = new SvelteSet<number>();
  /** Why a refresh the owner asked for could not even be tried, by machine. */
  refreshErrors = new SvelteMap<number, string>();
  /** Machines whose identity the owner has just approved: it is being stored, and the machine read with it. */
  trusting = new SvelteSet<number>();
  /** Why an identity the owner approved was not stored, by machine. */
  trustErrors = new SvelteMap<number, string>();
  #seq = 0;
  /** The readings asked for from here that are still going, one per machine at most. */
  #reads = new OneAtATime<number>();

  async load() {
    const seq = ++this.#seq;
    try {
      const list = await api.machineStats();
      if (seq !== this.#seq) return;
      this.list = list;
      this.error = null;
      // Why an approval was refused is old news once a later reading works, or the machine is gone.
      for (const id of [...this.trustErrors.keys()]) {
        const entry = list.find((e) => e.deviceId === id);
        if (!entry || !entry.error) this.trustErrors.delete(id);
      }
    } catch (e) {
      if (seq === this.#seq) this.error = errorText(e);
    } finally {
      this.loaded = true;
    }
  }

  get(id: number): MachineStatsEntry | undefined {
    return this.byId.get(id);
  }

  /** True for this PC: its stats are read here, without SSH, so it needs no SSH user. The back end says so
   *  in each entry; before the first one comes in, the screens' own idea of "this PC" stands in. */
  local(d: Device): boolean {
    if (d.kind === "ignore") return false;
    return this.get(d.id)?.local ?? status.isSelf(d);
  }

  /** Read one machine now. A failed reading comes back inside the entry; only a refused request ends up in
   *  refreshErrors. While a reading of that machine is going no second one is started; the promise then
   *  ends with the one that is going. */
  refresh(id: number): Promise<void> {
    return this.#reads.run(id, async () => {
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
    });
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
      try {
        await api.trustHostKey(id, fingerprint);
      } catch (e) {
        const why = errorText(e);
        this.trustErrors.set(id, why);
        await this.load();
        // Said in any case, also when the machine reads fine by now (load drops it then); the next
        // reading that works clears it.
        this.trustErrors.set(id, why);
        return;
      }
      // A reading that was already going started without the approval and comes back still asking for
      // it. Let it finish, then read again: that reading is the one that uses what was just approved.
      await this.#reads.settled(id);
      await this.refresh(id);
    } finally {
      this.trusting.delete(id);
    }
  }
}

export const stats = new StatsStore();
