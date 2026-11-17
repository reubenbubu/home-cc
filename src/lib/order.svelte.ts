// The order the owner has dragged the Machines cards into. It is a choice for this PC's screen, not
// something about the machines, so it is kept in the window's own storage rather than in the back end.
// That storage can be missing or refuse (a locked-down profile): then the cards are in name order and
// a new order lasts until the app is closed.
import { arrange, parseNames, parseOrder, remember } from "./arrange";

class CardOrder {
  saved = $state<number[]>([]);
  #key: string;

  constructor(key: string) {
    this.#key = key;
    try {
      this.saved = parseOrder(localStorage.getItem(key));
    } catch {
      // No storage: start in name order.
    }
  }

  /** The machines (given in name order) in the order the owner chose. */
  arrange<T extends { id: number }>(items: T[]): T[] {
    return arrange(items, this.saved);
  }

  /** Saves the cards as they are now on screen. `existing` is every device there is, so places of
   *  machines that are only hidden for now are kept. */
  set(shown: number[], existing: Iterable<number>) {
    this.saved = remember(shown, this.saved, existing);
    this.#write();
  }

  /** Forgets the places of machines that no longer exist. */
  keepOnly(existing: Iterable<number>) {
    const exists = new Set(existing);
    const next = this.saved.filter((id) => exists.has(id));
    if (next.length === this.saved.length) return;
    this.saved = next;
    this.#write();
  }

  /** Back to name order. */
  reset() {
    this.saved = [];
    this.#write();
  }

  #write() {
    try {
      if (this.saved.length) localStorage.setItem(this.#key, JSON.stringify(this.saved));
      else localStorage.removeItem(this.#key);
    } catch {
      // Not stored: the order still holds until the app is closed.
    }
  }
}

export const machineOrder = new CardOrder("home-cc.machine-order");

/** The name the machines in no group go by here ("Other machines" on screen). No group can have it: an
 *  empty group name means no group. */
export const NO_GROUP = "";

/** Which groups of cards the owner has collapsed on the Machines screen, by group name. Like the order, a
 *  choice for this PC's screen, kept in the window's storage; without it they all start expanded. */
class CollapsedGroups {
  names = $state<string[]>([]);
  #key = "home-cc.collapsed-groups";

  constructor() {
    try {
      this.names = parseNames(localStorage.getItem(this.#key));
    } catch {
      // No storage: every group starts expanded.
    }
  }

  has(group: string): boolean {
    return this.names.includes(group);
  }

  set(group: string, collapsed: boolean) {
    if (collapsed === this.has(group)) return;
    this.names = collapsed ? [...this.names, group] : this.names.filter((n) => n !== group);
    this.#write();
  }

  /** Forgets groups that no longer exist, so a new group of the same name starts expanded. */
  keepOnly(existing: Iterable<string>) {
    const keep = new Set(existing);
    keep.add(NO_GROUP);
    const next = this.names.filter((n) => keep.has(n));
    if (next.length === this.names.length) return;
    this.names = next;
    this.#write();
  }

  #write() {
    try {
      if (this.names.length) localStorage.setItem(this.#key, JSON.stringify(this.names));
      else localStorage.removeItem(this.#key);
    } catch {
      // Not stored: it holds until the app is closed.
    }
  }
}

export const collapsedGroups = new CollapsedGroups();
