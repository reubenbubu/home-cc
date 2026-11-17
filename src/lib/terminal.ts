// The parts of the terminal tabs that don't need a browser: decoding output, which keys copy and paste,
// keeping typing in order, telling the back end what has been drawn, and the words shown on a tab.
// The tabs themselves are in terminals.svelte.ts; tests are in terminal.test.mjs.

/** The bytes in a "terminal-output" event. They go to the terminal as they are: a character split across
 *  two events is put back together there. */
export function decodeBase64(text: string): Uint8Array {
  const raw = atob(text);
  const bytes = new Uint8Array(raw.length);
  for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);
  return bytes;
}

/** The parts of a key event that decide what it does. */
export interface KeyLike {
  key: string;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  metaKey: boolean;
}

/** What a key does instead of going to the program, as in Windows Terminal: Ctrl+Shift+C and Ctrl+Insert
 *  copy, Ctrl+Shift+V and Shift+Insert paste, and Ctrl+C copies while text is selected (and is the usual
 *  interrupt when nothing is). Ctrl+V and every other key go to the program: Ctrl+V means something in vim
 *  and in the shell. */
export function clipboardKey(e: KeyLike, hasSelection: boolean): "copy" | "paste" | null {
  if (e.altKey || e.metaKey) return null;
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  if (e.ctrlKey && e.shiftKey) {
    if (key === "c") return "copy";
    if (key === "v") return "paste";
    return null;
  }
  if (e.ctrlKey && key === "Insert") return "copy";
  if (e.shiftKey && !e.ctrlKey && key === "Insert") return "paste";
  if (e.ctrlKey && !e.shiftKey && key === "c" && hasSelection) return "copy";
  return null;
}

/** A right-click copies the selection if there is one, and pastes if not (as in Windows Terminal and
 *  PuTTY). While a program follows the mouse (htop, tmux with the mouse on) the click is the program's,
 *  unless Shift is held. */
export function rightClickAction(hasSelection: boolean, programWantsMouse: boolean, shiftKey: boolean): "copy" | "paste" | null {
  if (programWantsMouse && !shiftKey) return null;
  return hasSelection ? "copy" : "paste";
}

/** Counts output the screen has drawn, and says when to tell the back end, in steps so it isn't told after
 *  every few bytes. The back end stops reading while too much is waiting to be drawn (terminal.rs); the
 *  step must stay well below that. */
export class Drawn {
  #pending = 0;
  readonly step: number;
  constructor(step = 64 * 1024) {
    this.step = step;
  }

  /** Returns how many bytes to report now: 0 until a step has gathered. */
  add(bytes: number): number {
    this.#pending += bytes;
    if (this.#pending < this.step) return 0;
    const report = this.#pending;
    this.#pending = 0;
    return report;
  }
}

/** Sends typing one piece at a time, so it can't arrive out of order; what is typed meanwhile goes in the
 *  next piece, joined up. */
export class InOrder {
  #waiting = "";
  #busy = false;
  #stopped = false;
  readonly #send: (data: string) => Promise<unknown>;
  readonly #failed: (error: unknown) => void;
  constructor(send: (data: string) => Promise<unknown>, failed: (error: unknown) => void = () => {}) {
    this.#send = send;
    this.#failed = failed;
  }

  push(data: string) {
    if (this.#stopped || !data) return;
    this.#waiting += data;
    if (!this.#busy) void this.#next();
  }

  /** Drop anything still waiting and send no more. */
  stop() {
    this.#stopped = true;
    this.#waiting = "";
  }

  async #next() {
    this.#busy = true;
    while (this.#waiting && !this.#stopped) {
      const data = this.#waiting;
      this.#waiting = "";
      try {
        await this.#send(data);
      } catch (e) {
        this.#failed(e);
      }
    }
    this.#busy = false;
  }
}

/** What a terminal said before the screen knew which tab it belongs to: its output can arrive while the
 *  request that opened it is still on its way back. Kept for a short while, up to a limit. */
export interface Early {
  chunks: Uint8Array[];
  bytes: number;
  exit: { reason: string | null; exitStatus: number | null } | null;
  /** Milliseconds, as from Date.now(). */
  since: number;
}

export class EarlyOutput {
  #held = new Map<number, Early>();
  readonly maxBytes: number;
  constructor(maxBytes = 1024 * 1024) {
    this.maxBytes = maxBytes;
  }

  #entry(id: number, now: number): Early {
    let e = this.#held.get(id);
    if (!e) this.#held.set(id, (e = { chunks: [], bytes: 0, exit: null, since: now }));
    return e;
  }

  output(id: number, bytes: Uint8Array, now = Date.now()) {
    const e = this.#entry(id, now);
    if (e.bytes + bytes.length > this.maxBytes) return;
    e.chunks.push(bytes);
    e.bytes += bytes.length;
  }

  exit(id: number, reason: string | null, exitStatus: number | null, now = Date.now()) {
    this.#entry(id, now).exit = { reason, exitStatus };
  }

  /** Everything held for this terminal, which is no longer held. */
  take(id: number): Early | null {
    const e = this.#held.get(id) ?? null;
    this.#held.delete(id);
    return e;
  }

  /** Terminals nobody claimed within `maxAgeMs`: left over from a window that was reloaded, say. They are
   *  no longer held; the caller closes the ones still running. */
  unclaimed(maxAgeMs: number, now = Date.now()): { id: number; running: boolean }[] {
    const old: { id: number; running: boolean }[] = [];
    for (const [id, e] of this.#held) {
      if (now - e.since >= maxAgeMs) {
        old.push({ id, running: e.exit == null });
        this.#held.delete(id);
      }
    }
    return old;
  }
}

/** The name for a new tab: the given one, numbered if a tab already has it. */
export function tabTitle(title: string, taken: string[]): string {
  const base = title.trim() || "Terminal";
  if (!taken.includes(base)) return base;
  for (let n = 2; ; n++) {
    const next = `${base} (${n})`;
    if (!taken.includes(next)) return next;
  }
}

/** What a tab says once its terminal has ended. */
export function endedText(reason: string | null): string {
  return reason ? `Session ended. ${reason}` : "Session ended.";
}
