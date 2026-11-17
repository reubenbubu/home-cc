// Down machines the owner has silenced on the Overview (the old laptop that is known to be off): left out of
// Needs attention and the Down counts until they are up again, or the silence is undone. Like the card
// order (order.svelte.ts) it is a choice for this PC's screen, kept in the window's own storage. That
// storage can be missing or refuse: then a silence lasts until the app is closed. The rules are in
// overview.ts.
import type { Device } from "./api";
import { keepSilences, parseSilences, stillSilenced, type Seen, type Silence } from "./overview";
import { status } from "./status.svelte";

const KEY = "home-cc.silenced";

/** A device as a silence is checked against it. A scan that finds it counts as seen too. */
function seen(d: Device): Seen {
  const times = [status.get(d)?.lastSeen, d.lastSeen].filter((t): t is number => t != null);
  return { id: d.id, health: status.health(d), lastSeen: times.length ? Math.max(...times) : null };
}

class Silences {
  list = $state<Silence[]>([]);

  constructor() {
    try {
      this.list = parseSilences(localStorage.getItem(KEY));
    } catch {
      // No storage: nothing is silenced.
    }
  }

  /** True while the owner's silence of this device holds. */
  has(d: Device): boolean {
    const s = this.list.find((x) => x.id === d.id);
    return !!s && stillSilenced(s, seen(d));
  }

  /** Silences a device that is down now. */
  silence(d: Device) {
    if (status.health(d) !== "down") return;
    const at = Math.floor(Date.now() / 1000);
    this.list = [...this.list.filter((s) => s.id !== d.id), { id: d.id, at }];
    this.#write();
  }

  undo(d: Device) {
    const next = this.list.filter((s) => s.id !== d.id);
    if (next.length === this.list.length) return;
    this.list = next;
    this.#write();
  }

  /** Forgets the silences of machines that are up again or no longer exist. `devices` is every device there is. */
  keepOnly(devices: Device[]) {
    const next = keepSilences(this.list, devices.map(seen));
    if (next.length === this.list.length) return;
    this.list = next;
    this.#write();
  }

  #write() {
    try {
      if (this.list.length) localStorage.setItem(KEY, JSON.stringify(this.list));
      else localStorage.removeItem(KEY);
    } catch {
      // Not stored: it holds until the app is closed.
    }
  }
}

export const silences = new Silences();
