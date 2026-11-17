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
  type UpdateCheck,
} from "./api";
import { containerKey, keptSelection, shownHosts, stackKey, uncheckedImages, updatesByKey } from "./logic";

/** The open host is read again this often while the screen is open. */
export const REFRESH_SECS = 15;
/** The screens check a host's images for updates by themselves this often (UPDATES_EVERY in the back end, which
 *  has the last word), so registries such as Docker Hub aren't asked more than needed. */
export const UPDATES_EVERY_SECS = 6 * 3600;
/** After a check of the screens' own failed, they try again no sooner than this. */
const UPDATES_RETRY_SECS = 15 * 60;
/** Every host is read again this often while the Overview screen is open. */
export const OVERVIEW_SECS = 60;
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
  /** Each host's last check for image updates. */
  updates = new SvelteMap<number, UpdateCheck>();
  /** Why the last check of a host couldn't be done at all (per image problems are in the check itself). */
  updateErrors = new SvelteMap<number, string>();
  /** Hosts being checked for updates right now. */
  checking = new SvelteSet<number>();
  /** Stack updates under way, per host: the stack being updated now. */
  updating = new SvelteMap<number, string>();
  /** How the last update of each host's stacks went, for its panel. */
  updateResults = new SvelteMap<number, { done: string[]; failed: string[] }>();
  /** Checks under way, so one the owner asks for waits for the one running. */
  #checks = new Map<number, Promise<void>>();
  /** Images (by updateKey) asked about by themselves (a container recreated since the check), so each is asked once. */
  #askedImages = new Map<number, Set<string>>();
  /** When the screens' own full check of each host last started (Unix seconds). */
  #autoAt = new Map<number, number>();

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
      if (view.overview && !view.host.problem) this.#updatesDue(id);
    } catch (e) {
      this.readErrors.set(id, errorText(e));
    } finally {
      this.reading.delete(id);
    }
  }

  /** After a host was read: check its images when the screens' own check is due (the back end decides), or ask
   *  about the images of containers that have no answer yet, once each. Never while it is being updated. */
  #updatesDue(id: number) {
    if (this.checking.has(id) || this.updating.has(id)) return;
    const now = Date.now() / 1000;
    const check = this.updates.get(id);
    if (!check || check.checkedAt == null || now - check.checkedAt >= UPDATES_EVERY_SECS) {
      if (this.updateErrors.has(id) && now - (this.#autoAt.get(id) ?? 0) < UPDATES_RETRY_SECS) return;
      this.#autoAt.set(id, now);
      void this.checkUpdates(id, true);
      return;
    }
    const o = this.overviews.get(id);
    if (!o) return;
    const asked = this.#askedImages.get(id) ?? new Set<string>();
    const fresh = uncheckedImages(o, updatesByKey(check), asked);
    if (fresh.images.length === 0) return;
    for (const k of fresh.keys) asked.add(k);
    this.#askedImages.set(id, asked);
    void this.checkUpdates(id, true, fresh.images);
  }

  /** Check a host's images for newer ones. `auto` for the screens' own checks, which the back end runs at most
   *  every few hours (answering with the last check otherwise) and skips while a check is running; the owner's
   *  wait for it. `images`: only these. */
  async checkUpdates(id: number, auto: boolean, images: string[] | null = null): Promise<void> {
    const running = this.#checks.get(id);
    if (running) {
      if (auto) return;
      await running;
    }
    const job = (async () => {
      this.checking.add(id);
      try {
        const check = await dockerApi.checkUpdates(id, auto, images);
        if (check) this.updates.set(id, check);
        this.updateErrors.delete(id);
      } catch (e) {
        this.updateErrors.set(id, errorText(e));
      } finally {
        this.checking.delete(id);
        this.#checks.delete(id);
      }
    })();
    this.#checks.set(id, job);
    await job;
  }

  /** Update the ticked services of some stacks, one stack after the other, as the stack actions run (each card
   *  shows its progress and what Docker said). Then check those images again. */
  async updateStacks(hostId: number, plan: { stack: string; services: string[]; images: string[] }[]) {
    if (this.updating.has(hostId) || plan.length === 0) return;
    const result = { done: [] as string[], failed: [] as string[] };
    this.updateResults.delete(hostId);
    for (const p of plan) {
      this.updating.set(hostId, p.stack);
      const ok = await this.#run(hostId, stackKey(hostId, p.stack), "Updating", () => dockerApi.updateStack(hostId, p.stack, p.services));
      (ok ? result.done : result.failed).push(p.stack);
    }
    this.updating.delete(hostId);
    this.updateResults.set(hostId, result);
    await this.checkUpdates(hostId, false, [...new Set(plan.flatMap((p) => p.images))]);
  }

  /** For a container outside any stack: pull its image only, then check it again. */
  async pullImage(hostId: number, c: Container) {
    const ok = await this.#run(hostId, containerKey(hostId, c.id), "Pulling the new image", () => dockerApi.pullImage(hostId, c.image));
    if (ok) await this.checkUpdates(hostId, false, [c.image]);
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

  /** For the Overview screen: every host with a Docker tab, read now and then every OVERVIEW_SECS while the
   *  window is in view. Looks for Docker on machines due a look first, as the Docker screen does. Reads are
   *  `auto`, so a host whose login was turned away waits for the owner. Returns a function that stops. */
  watchAll(): () => void {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const readAll = (onlyUnread: boolean) =>
      Promise.all(
        this.shown.filter((h) => !onlyUnread || !this.overviews.has(h.deviceId)).map((h) => this.read(h.deviceId, true)),
      );
    const round = async (first: boolean) => {
      if (!document.hidden) {
        if (first || !this.loaded) await this.loadHosts();
        // Hosts already known are read straight away. Looking for Docker on the other machines can take a
        // while; what it finds is read as soon as that is done.
        const found = first ? this.probeDue().then(() => (stopped ? undefined : readAll(true))) : undefined;
        if (!stopped) await Promise.all([readAll(false), found]);
      }
      if (!stopped) timer = setTimeout(() => round(false), OVERVIEW_SECS * 1000);
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

  /** Returns whether the job worked. */
  async #run(hostId: number, key: string, doing: string, job: () => Promise<{ output: string }>): Promise<boolean> {
    if (this.actions.get(key)?.doing) return false;
    this.actions.set(key, { doing, output: null, error: null });
    let ok = false;
    try {
      const { output } = await job();
      this.actions.set(key, { doing: null, output: output || null, error: null });
      ok = true;
    } catch (e) {
      this.actions.set(key, { doing: null, output: null, error: errorText(e) });
    }
    await this.read(hostId, false);
    void this.#statsIfFine(hostId);
    return ok;
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
