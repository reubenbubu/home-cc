// Asking the back end for the names of addresses, for names.svelte.ts. Every
// address asked for during one moment (one pass of drawing the screen) goes
// in one request, and each answer is kept for a while. Kept apart from the
// store so it can be tested without a browser (lookups.test.mjs).
import type { ResolvedName } from "./api";

/** Most addresses in one request: the back end refuses more. */
export const BATCH = 256;
/** How long an answer is used before the address is asked about again, in milliseconds. The old answer
 *  stays on screen meanwhile. The back end remembers answers just as long, so asking again is cheap. */
export const KEEP_MS = { name: 10 * 60_000, none: 2 * 60_000, error: 30_000 };

/** Only these are looked up: anything else is already a name, or can't be one. */
export function isIpv4(text: string): boolean {
  const parts = text.trim().split(".");
  return parts.length === 4 && parts.every((p) => /^\d{1,3}$/.test(p) && Number(p) <= 255);
}

/** What the back end said about one address. */
export interface Answer {
  /** Null: no name was found (or, with error, the lookup failed). */
  name: string | null;
  /** When it arrived (milliseconds, as now() gives them). */
  at: number;
  /** Why the lookup failed, if it did. */
  error?: string;
}

export interface LookupOptions {
  /** Asks the back end; one answer per address. */
  send: (addresses: string[]) => Promise<ResolvedName[]>;
  /** Where answers are kept. A reactive map makes the screens follow them. */
  answers?: Map<string, Answer>;
  /** Runs the request once the current moment is over. */
  schedule?: (run: () => void) => void;
  now?: () => number;
}

export class Lookups {
  readonly answers: Map<string, Answer>;
  #send: LookupOptions["send"];
  #schedule: NonNullable<LookupOptions["schedule"]>;
  #now: NonNullable<LookupOptions["now"]>;
  /** Waiting for the next request, and in a request that hasn't answered yet. Never asked for twice. */
  #queued = new Set<string>();
  #asking = new Set<string>();
  #scheduled = false;

  constructor(options: LookupOptions) {
    this.#send = options.send;
    this.answers = options.answers ?? new Map();
    this.#schedule = options.schedule ?? ((run) => queueMicrotask(run));
    this.#now = options.now ?? (() => Date.now());
  }

  /** The answer for an address so far (undefined while there is none), asking for it when there is none
   *  or it is old. Safe to call while the screen is being drawn: nothing it changes is shown until the
   *  request has answered. */
  get(address: string): Answer | undefined {
    const key = address.trim();
    const answer = this.answers.get(key);
    if (!answer || this.#isOld(answer)) this.#want(key);
    return answer;
  }

  #isOld(a: Answer): boolean {
    const keep = a.error !== undefined ? KEEP_MS.error : a.name ? KEEP_MS.name : KEEP_MS.none;
    return this.#now() - a.at >= keep;
  }

  #want(address: string) {
    if (!isIpv4(address) || this.#queued.has(address) || this.#asking.has(address)) return;
    this.#queued.add(address);
    if (this.#scheduled) return;
    this.#scheduled = true;
    this.#schedule(() => this.#flush());
  }

  #flush() {
    this.#scheduled = false;
    const all = [...this.#queued];
    this.#queued.clear();
    for (let i = 0; i < all.length; i += BATCH) this.#ask(all.slice(i, i + BATCH));
  }

  async #ask(addresses: string[]) {
    for (const a of addresses) this.#asking.add(a);
    try {
      const got = await this.#send(addresses);
      const at = this.#now();
      const byAddress = new Map(got.map((r) => [r.address, r.name]));
      for (const a of addresses) this.answers.set(a, { name: byAddress.get(a) ?? null, at });
    } catch (e) {
      const at = this.#now();
      const error = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
      // A name found before is still the best guess; otherwise say the lookup failed.
      for (const a of addresses) {
        const before = this.answers.get(a);
        this.answers.set(a, before?.name ? { ...before, at } : { name: null, at, error });
      }
    } finally {
      for (const a of addresses) this.#asking.delete(a);
    }
  }
}
