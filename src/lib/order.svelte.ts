// The order the owner has dragged the Machines cards into. It is a choice for this PC's screen, not
// something about the machines, so it is kept in the window's own storage rather than in the back end.
// That storage can be missing or refuse (a locked-down profile): then the cards are in name order and
// a new order lasts until the app is closed.
import { arrange, parseOrder, remember } from "./arrange";

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
