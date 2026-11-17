// What a form keeps while the owner looks at another screen (drafts.svelte.ts holds it): when a draft is
// worth keeping, and how a file being edited is shown again when the screen comes back. No Svelte here, so
// "npm test" can try it.

/** The same value, looking inside lists and objects. For the plain values forms hold: text, numbers, yes/no,
 *  nothing, and lists and objects of those. */
export function sameValues(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  if (Array.isArray(a) && Array.isArray(b)) {
    return a.length === b.length && a.every((v, i) => sameValues(v, b[i]));
  }
  const ra = a as Record<string, unknown>;
  const rb = b as Record<string, unknown>;
  const keys = new Set([...Object.keys(ra), ...Object.keys(rb)]);
  for (const k of keys) {
    // A field left out and a field that is undefined are the same thing in a form.
    if (!sameValues(ra[k], rb[k])) return false;
  }
  return true;
}

/** The form holds something that isn't saved. With nothing saved to compare to (not read yet), anything
 *  typed counts. */
export function differs<T>(value: T, saved: T | undefined): boolean {
  return saved === undefined || !sameValues(value, saved);
}

/** A file being edited when the owner left the screen. */
export interface FileDraft {
  path: string;
  /** The file as it was read from the host then. */
  original: string;
  /** The owner's text. */
  text: string;
}

/** The file shown again after the screen comes back, now that it has been read from the host afresh.
 *  The owner's text is never replaced. `original` becomes what is on the host now, so a save compares with
 *  that; `hostChanged` says the file there changed in the meantime, which the screen must say, as saving then
 *  replaces that change. */
export function fileOnReturn(draft: Pick<FileDraft, "original" | "text">, onHost: string): { original: string; text: string; hostChanged: boolean } {
  const edited = draft.text !== draft.original;
  if (!edited || draft.text === onHost) return { original: onHost, text: onHost, hostChanged: false };
  return { original: onHost, text: draft.text, hostChanged: onHost !== draft.original };
}

/** The file draft to use when a file is opened: the one held for that very file, if there is one, so every
 *  read of it (a retry after an error, a click on its tab) goes through fileOnReturn and keeps the owner's
 *  text. */
export function draftFor(held: FileDraft | undefined, path: string): FileDraft | undefined {
  return held?.path === path ? held : undefined;
}

/** Whether the file shown now may take the place of the draft held. Not when the draft holds unsaved text for
 *  another file that couldn't be shown again (its read failed, or it is no longer listed) while nothing has
 *  been typed into the file shown: that would lose the owner's text without a word. */
export function replacesFileDraft(held: FileDraft | undefined, now: FileDraft): boolean {
  if (!held || held.path === now.path) return true;
  return held.text === held.original || now.text !== now.original;
}
