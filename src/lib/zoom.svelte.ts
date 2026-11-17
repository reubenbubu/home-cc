// How big the whole app is drawn. Ctrl with + / - / 0, Ctrl with the mouse
// wheel, and the buttons in Settings all change it. It is remembered in the
// window's own storage, so it is back as it was when the app starts again.
//
// In the app this is the window's own zoom, the same as zooming a web page:
// everything grows, and the screens rearrange themselves for the smaller room
// they now have. The window's built-in zoom keys (Tauri's zoomHotkeysEnabled)
// stay off: they would zoom a second time on top of these, use other steps on
// Linux, and not be remembered.
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { addWheel, clampZoom, parseZoom, zoomIn, zoomKeyAction, zoomOut, zoomShows, ZOOM_DEFAULT, type ZoomAction, type ZoomView } from "./zoom";
import { errorText } from "./api";

const STORE_KEY = "home-cc.zoom";

/** Wheel turns further apart than this don't add up to a step. */
const WHEEL_PAUSE_MS = 400;

/** As the app starts: how long to wait for the window to show the remembered size once it has said yes,
 *  and how long the app is kept out of sight at most, however the window answers. */
const SETTLE_MS = 600;
const HIDE_AT_MOST_MS = 2000;

/** Why the size couldn't be changed: what was being done, and the cause. */
export interface ZoomProblem {
  what: string;
  error: string;
}

function remembered(): number {
  try {
    return parseZoom(localStorage.getItem(STORE_KEY));
  } catch {
    return ZOOM_DEFAULT;
  }
}

function remember(percent: number) {
  try {
    if (percent === ZOOM_DEFAULT) localStorage.removeItem(STORE_KEY);
    else localStorage.setItem(STORE_KEY, String(percent));
  } catch {
    // Not remembered for next time, but the size still changes now.
  }
}

function native(): boolean {
  return "__TAURI_INTERNALS__" in window && !("__HOME_CC_MOCK__" in window);
}

/** In a browser with the sample back end there is no window to zoom, so the page is scaled instead. That is
 *  close enough to see the size change, but the screens that rearrange by window width don't (to see those
 *  as zoomed, tools/screenshots.mjs has --zoom). --page-zoom keeps the app's height to the window. */
function pageZoom(percent: number) {
  const root = document.documentElement.style;
  if (percent === ZOOM_DEFAULT) {
    root.removeProperty("zoom");
    root.removeProperty("--page-zoom");
  } else {
    root.setProperty("zoom", String(percent / 100));
    root.setProperty("--page-zoom", String(percent / 100));
  }
}

function view(): ZoomView {
  return { ratio: window.devicePixelRatio, width: window.innerWidth };
}

function wait(ms: number): Promise<void> {
  return new Promise((done) => setTimeout(done, ms));
}

class ZoomStore {
  /** The size now, in percent. */
  percent = $state(ZOOM_DEFAULT);
  /** Why the last change couldn't be made, if it couldn't. Shown on every screen (+layout.svelte). */
  problem = $state<ZoomProblem | null>(null);
  /** False while the app starts and the window is still being brought to the remembered size, so the
   *  first screen isn't seen at 100% and then jumps. */
  ready = $state(true);
  #seq = 0;
  #wheel = 0;
  #wheelAt = 0;

  /** Puts back the size from last time. Called as the app starts, before the first screen is drawn. */
  restore() {
    const percent = remembered();
    if (percent === ZOOM_DEFAULT) return;
    // In a browser the page is scaled straight away, before anything is drawn.
    if (!native()) {
      this.set(percent, true);
      return;
    }
    // The window zooms a moment after it is asked to, which is after the first screen would be drawn. Until
    // it has, the app is kept out of sight; if the window doesn't seem to answer, it is shown anyway.
    this.ready = false;
    const before = view();
    let resized = () => {};
    const shows = new Promise<void>((done) => {
      resized = () => {
        if (zoomShows(before, view(), percent)) done();
      };
      window.addEventListener("resize", resized);
    });
    const restored = this.set(percent, true).then((ok) => (ok ? Promise.race([shows, wait(SETTLE_MS)]) : undefined));
    Promise.race([restored, wait(HIDE_AT_MOST_MS)]).then(() => {
      window.removeEventListener("resize", resized);
      this.ready = true;
    });
  }

  /** Changes the size and remembers it. Never rejects: a problem is kept in `problem`, and it resolves to
   *  whether the change was made. `restoring` is the size from last time being put back as the app starts. */
  async set(percent: number, restoring = false): Promise<boolean> {
    const next = clampZoom(percent);
    const before = this.percent;
    const seq = ++this.#seq;
    this.percent = next;
    this.problem = null;
    if (!restoring) remember(next);
    try {
      if (native()) await getCurrentWebview().setZoom(next / 100);
      else pageZoom(next);
      return true;
    } catch (e) {
      // Only the newest change speaks; an older one that failed late must not undo it.
      if (seq !== this.#seq) return false;
      this.percent = before;
      // A size from last time that couldn't be put back stays remembered, so the next start tries again.
      if (!restoring) remember(before);
      this.problem = {
        what: restoring ? `Couldn't put the app back to the size you left it at (${next}%)` : `Couldn't change the size of the app to ${next}%`,
        error: errorText(e),
      };
      return false;
    }
  }

  /** Puts away the message about a size that couldn't be changed. */
  dismiss() {
    this.problem = null;
  }

  act(action: ZoomAction) {
    this.set(action === "in" ? zoomIn(this.percent) : action === "out" ? zoomOut(this.percent) : ZOOM_DEFAULT);
  }

  /** Listens for the zoom keys and Ctrl with the wheel anywhere in the window. Returns a function that stops. */
  listen(): () => void {
    const onKey = (e: KeyboardEvent) => {
      const action = zoomKeyAction(e);
      if (!action) return;
      e.preventDefault();
      // Held down, the key repeats: one step for each repeat, as in a web browser.
      this.act(action);
    };
    const onWheel = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      // Otherwise the page would scroll, or the browser zoom by itself.
      e.preventDefault();
      const lines = e.deltaMode === WheelEvent.DOM_DELTA_LINE ? 40 : e.deltaMode === WheelEvent.DOM_DELTA_PAGE ? 800 : 1;
      if (e.timeStamp - this.#wheelAt > WHEEL_PAUSE_MS) this.#wheel = 0;
      this.#wheelAt = e.timeStamp;
      const { total, action } = addWheel(this.#wheel, e.deltaY * lines);
      this.#wheel = total;
      if (action) this.act(action);
    };
    // Caught on the way in, so a box or a terminal that keeps the key for itself doesn't stop the zoom.
    window.addEventListener("keydown", onKey, { capture: true });
    window.addEventListener("wheel", onWheel, { capture: true, passive: false });
    return () => {
      window.removeEventListener("keydown", onKey, { capture: true });
      window.removeEventListener("wheel", onWheel, { capture: true });
    };
  }
}

export const zoom = new ZoomStore();
