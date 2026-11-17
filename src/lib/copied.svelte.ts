// The small "Copied" note shown where something was double-clicked to copy it (components/Copyable).
// Drawn once for the whole app by components/CopiedNote.svelte, so a table cell or a line cut short
// with "…" can't hide it.

class CopiedNote {
  /** Where to show it, in window pixels, and what it says. Null when nothing is shown. */
  shown = $state<{ x: number; y: number; ok: boolean; n: number } | null>(null);
  #timer: ReturnType<typeof setTimeout> | undefined;
  #n = 0;

  show(x: number, y: number, ok: boolean) {
    clearTimeout(this.#timer);
    this.shown = { x, y, ok, n: ++this.#n };
    this.#timer = setTimeout(() => (this.shown = null), 1400);
  }
}

export const copiedNote = new CopiedNote();
