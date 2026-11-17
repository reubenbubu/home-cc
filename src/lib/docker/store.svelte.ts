// What the Docker screen knows: the Docker hosts, each host's stacks and
// containers, and the actions under way. Kept while the app runs, so going to
// another screen and back shows the last reading straight away.
import { SvelteMap, SvelteSet } from "svelte/reactivity";
import { errorText } from "$lib/api";
import {
  dockerApi,
  type Container,
  type ContainerAction,
  type ContainerStats,
  type DockerHost,
  type HostMode,
  type Overview,
  type StackAction,
} from "./api";
import { containerKey, keptSelection, shownHosts, stackKey } from "./logic";

/** The open host is read again this often while the screen is open. */
export const REFRESH_SECS = 15;
/** Looks for Docker going on at once. */
const PROBES_AT_ONCE = 3;

/** An action under way or just finished, on a container or a stack (keys from containerKey and stackKey). */
export interface Action {
  /** What is being done, as the button said it: "Restarting". */
  doing: string | null;
  /** What docker said the last time, if anything. */
  output: string | null;
  error: string | null;
}

class DockerStore {
  hosts = $state<DockerHost[]>([]);
  loaded = $state(false);
  error = $state<string | null>(null);
  /** The host the screen shows. */
  selected = $state<number | null>(null);
  /** The last good reading of each host. Kept when a later one fails, next to the problem. */
  overviews = new SvelteMap<number, Overview>();
  /** CPU and memory by container id, per host. */
  stats = new SvelteMap<number, Map<string, ContainerStats>>();
  statsErrors = new SvelteMap<number, string>();
  /** Hosts being read right now. */
  reading = new SvelteSet<number>();
  /** Why a reading the screen asked for couldn't even be tried. */
  readErrors = new SvelteMap<number, string>();
  /** Machines being looked at for Docker. */
  probing = new SvelteSet<number>();
  actions = new SvelteMap<string, Action>();

  shown = $derived(shownHosts(this.hosts));
  byId = $derived(new Map(this.hosts.map((h) => [h.deviceId, h])));
  #seq = 0;

  async loadHosts() {
    const seq = ++this.#seq;
    try {
      const hosts = await dockerApi.hosts();
      if (seq !== this.#seq) return;
      this.hosts = hosts;
      this.error = null;
      this.selected = keptSelection(this.selected, this.shown);
    } catch (e) {
      if (seq === this.#seq) this.error = errorText(e);
    } finally {
      this.loaded = true;
    }
  }

  #put(host: DockerHost) {
    this.#seq++;
    const i = this.hosts.findIndex((h) => h.deviceId === host.deviceId);
    this.hosts = i < 0 ? [...this.hosts, host] : this.hosts.map((h, j) => (j === i ? host : h));
    // A host set to "Never show" (or found to have no Docker) loses its tab; the screen
    // moves on rather than keep showing, and reading, it.
    const next = keptSelection(this.selected, this.shown);
    if (next !== this.selected) {
      this.selected = next;
      if (next != null) this.select(next);
    }
  }

  /** CPU and memory, only once the list was read and nothing is wrong: they would be one more login otherwise. */
  async #statsIfFine(id: number) {
    if (this.overviews.has(id) && !this.byId.get(id)?.problem) await this.readStats(id);
  }

  /** Look for Docker on the machines that need it, a few at a time. */
  async probeDue() {
    const due = this.hosts.filter((h) => h.needsCheck && !this.probing.has(h.deviceId)).map((h) => h.deviceId);
    const next = async (): Promise<void> => {
      const id = due.shift();
      if (id == null) return;
      await this.probe(id);
      return next();
    };
    await Promise.all(Array.from({ length: PROBES_AT_ONCE }, next));
  }

  async probe(id: number) {
    if (this.probing.has(id)) return;
    this.probing.add(id);
    try {
      this.#put(await dockerApi.probe(id));
    } catch (e) {
      this.readErrors.set(id, errorText(e));
    } finally {
      this.probing.delete(id);
    }
  }

  async setMode(id: number, mode: HostMode) {
    try {
      this.#put(await dockerApi.setHostMode(id, mode));
      this.readErrors.delete(id);
      if (mode === "on" && !this.overviews.has(id)) void this.read(id, false);
    } catch (e) {
      this.readErrors.set(id, errorText(e));
    }
  }

  /** Read a host's containers. `auto` for the screen's timer: a host whose login was turned away waits for the
   *  owner instead. */
  async read(id: number, auto: boolean) {
    if (this.reading.has(id)) return;
    this.reading.add(id);
    if (!auto) this.readErrors.delete(id);
    try {
      const view = await dockerApi.overview(id, auto);
      this.#put(view.host);
      if (view.overview) this.overviews.set(id, view.overview);
      this.readErrors.delete(id);
    } catch (e) {
      this.readErrors.set(id, errorText(e));
    } finally {
      this.reading.delete(id);
    }
  }

  async readStats(id: number) {
    try {
      const list = await dockerApi.stats(id);
      this.stats.set(id, new Map(list.map((s) => [s.id, s])));
      this.statsErrors.delete(id);
    } catch (e) {
      this.statsErrors.set(id, errorText(e));
    }
  }

  /** The open host, read now and then every REFRESH_SECS while the window is in view. Returns a function that stops. */
  follow(): () => void {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const round = async (auto: boolean) => {
      const id = this.selected;
      if (id != null && !document.hidden) {
        await this.read(id, auto);
        if (!stopped) await this.#statsIfFine(id);
      }
      if (!stopped) timer = setTimeout(() => round(true), REFRESH_SECS * 1000);
    };
    void round(true);
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }

  /** Show another host, and read it straight away. A host whose login was turned away waits for Try again. */
  select(id: number) {
    this.selected = id;
    void this.read(id, true).then(() => this.#statsIfFine(id));
  }

  /** The owner's Refresh: read the host now, trying a refused login again too. */
  refresh(id: number) {
    void this.read(id, false).then(() => this.#statsIfFine(id));
  }

  action(key: string): Action | undefined {
    return this.actions.get(key);
  }

  async #run(hostId: number, key: string, doing: string, job: () => Promise<{ output: string }>) {
    if (this.actions.get(key)?.doing) return;
    this.actions.set(key, { doing, output: null, error: null });
    try {
      const { output } = await job();
      this.actions.set(key, { doing: null, output: output || null, error: null });
    } catch (e) {
      this.actions.set(key, { doing: null, output: null, error: errorText(e) });
    }
    await this.read(hostId, false);
    void this.#statsIfFine(hostId);
  }

  containerAction(hostId: number, c: Container, action: ContainerAction) {
    const doing = { start: "Starting", stop: "Stopping", restart: "Restarting", remove: "Removing" }[action];
    return this.#run(hostId, containerKey(hostId, c.id), doing, () => dockerApi.containerAction(hostId, c.id, action));
  }

  stackAction(hostId: number, stack: string, action: StackAction) {
    const doing = {
      up: "Starting",
      down: "Taking down",
      pull: "Pulling new images and restarting",
      restart: "Restarting",
      build: "Building and starting",
    }[action];
    return this.#run(hostId, stackKey(hostId, stack), doing, () => dockerApi.stackAction(hostId, stack, action));
  }

  clearAction(key: string) {
    this.actions.delete(key);
  }

  async forgetStack(hostId: number, stack: string) {
    try {
      await dockerApi.forgetStack(hostId, stack);
      this.actions.delete(stackKey(hostId, stack));
    } catch (e) {
      this.actions.set(stackKey(hostId, stack), { doing: null, output: null, error: errorText(e) });
    }
    await this.read(hostId, false);
  }
}

export const docker = new DockerStore();
