// How big the whole app is drawn: the steps between 50% and 200%, and which
// keys and wheel turns move between them. Kept apart from the store
// (zoom.svelte.ts) so it can be tested with plain Node.

/** The sizes to step through, in percent: the same ones a web browser uses. */
export const ZOOM_STEPS = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200];
export const ZOOM_MIN = ZOOM_STEPS[0];
export const ZOOM_MAX = ZOOM_STEPS[ZOOM_STEPS.length - 1];
export const ZOOM_DEFAULT = 100;

/** A whole percentage from 50 to 200. Anything that isn't a number means the normal size. */
export function clampZoom(percent: number): number {
  if (typeof percent !== "number" || !Number.isFinite(percent)) return ZOOM_DEFAULT;
  return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(percent)));
}

/** The next step up. From a size between two steps, the step just above it. */
export function zoomIn(percent: number): number {
  const now = clampZoom(percent);
  return ZOOM_STEPS.find((s) => s > now) ?? ZOOM_MAX;
}

/** The next step down. From a size between two steps, the step just below it. */
export function zoomOut(percent: number): number {
  const now = clampZoom(percent);
  return [...ZOOM_STEPS].reverse().find((s) => s < now) ?? ZOOM_MIN;
}

/** The size remembered from last time; a missing or damaged value means the normal size. */
export function parseZoom(text: string | null | undefined): number {
  if (text == null || text.trim() === "") return ZOOM_DEFAULT;
  return clampZoom(Number(text));
}

/** What the window looks like from inside the page: its pixel ratio and its width in page pixels. */
export interface ZoomView {
  ratio: number;
  width: number;
}

/** Whether the window has started drawing at a new size, from how it looked before the change. Zooming to
 *  `percent` raises the pixel ratio and narrows the page by that much; Windows' WebView2 changes both, other
 *  windows at least the width. A window that only changed size (being put back where it was) changes
 *  neither in step, so it doesn't count. */
export function zoomShows(before: ZoomView, now: ZoomView, percent: number): boolean {
  const factor = percent / 100;
  if (factor === 1) return true;
  if (Math.abs(now.ratio - before.ratio * factor) < 0.01) return true;
  return Math.abs(now.width * factor - before.width) <= 2 * factor;
}

export type ZoomAction = "in" | "out" | "reset";

/** The parts of a key press that matter here. */
export interface ZoomKey {
  key: string;
  ctrlKey: boolean;
  metaKey?: boolean;
  altKey?: boolean;
}

/** What a key press asks for: Ctrl with = or + to zoom in, - to zoom out and 0 for the normal size (the
 *  number pad's keys too). Not with Alt, as AltGr on many keyboards is Ctrl and Alt together. */
export function zoomKeyAction(e: ZoomKey): ZoomAction | null {
  if (!(e.ctrlKey || e.metaKey) || e.altKey) return null;
  switch (e.key) {
    case "=":
    case "+":
      return "in";
    case "-":
      return "out";
    case "0":
      return "reset";
    default:
      return null;
  }
}

/** How far the wheel has to turn for one step: about one notch of a mouse wheel. A touchpad sends many
 *  small turns, which add up to a step instead of each making one. */
export const WHEEL_STEP = 100;

/** Adds one turn of the wheel (with Ctrl held) to what has built up so far. Returns what is left over and
 *  the step to take, if any: "in" when the wheel turns away from the owner, as in a web browser. */
export function addWheel(total: number, deltaY: number): { total: number; action: ZoomAction | null } {
  // A different direction starts again from nothing.
  const sum = Math.sign(total) === -Math.sign(deltaY) ? deltaY : total + deltaY;
  // A mouse that reports less than a full step per notch still steps once on each notch.
  if (Math.abs(sum) >= WHEEL_STEP || (total === 0 && Math.abs(deltaY) >= WHEEL_STEP / 2)) {
    return { total: 0, action: sum < 0 ? "in" : "out" };
  }
  return { total: sum, action: null };
}
