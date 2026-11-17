// What was typed into a form and not saved yet, and which panel was open, kept while the owner looks at
// another screen. Each screen is drawn afresh when it is opened again, so a form fills itself from here
// first, and from what is saved only when there is nothing here.
//
// Kept in this window's memory only, because a draft can hold a password or a token secret: never written
// to disk or browser storage, never sent to the back end, never logged. Gone when the app closes. A form
// clears its draft when it is saved, and when the owner cancels or closes it.
import { differs } from "./drafts";

class Drafts {
  #held = new Map<string, unknown>();

  /** A copy of the draft, so the form can change it freely. */
  get<T>(key: string): T | undefined {
    const v = this.#held.get(key);
    return v === undefined ? undefined : (structuredClone(v) as T);
  }

  has(key: string): boolean {
    return this.#held.has(key);
  }

  /** Keeps the value as it is now (not the live form, which changes after this). */
  set<T>(key: string, value: T): void {
    this.#held.set(key, $state.snapshot(value));
  }

  /** Keeps the value while it is different from what is saved, and forgets it once it is the same again. */
  keep<T>(key: string, value: T, saved: T | undefined): void {
    const now = $state.snapshot(value) as T;
    if (differs(now, saved === undefined ? undefined : ($state.snapshot(saved) as T))) this.#held.set(key, now);
    else this.#held.delete(key);
  }

  /** For what is open or chosen on a screen rather than typed: kept while there is one, forgotten when it
   *  is closed (null or false). */
  remember<T>(key: string, value: T | null | false): void {
    if (value === null || value === false || value === undefined) this.#held.delete(key);
    else this.#held.set(key, $state.snapshot(value));
  }

  clear(...keys: string[]): void {
    for (const k of keys) this.#held.delete(k);
  }
}

export const drafts = new Drafts();
