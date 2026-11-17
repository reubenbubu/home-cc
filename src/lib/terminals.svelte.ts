// The terminal tabs. They live here, not on the Terminals screen, so they keep running (and keep their
// scrollback) while another screen is shown: each tab's terminal is drawn into an element of its own,
// which the screen borrows while it is open (attach / detach).
//
// Other screens open a tab with openTerminal({ deviceId, title, command }), for example
// openTerminal({ deviceId: 7, title: "web on Homelab VM", command: "docker exec -it web sh" }).
import { goto } from "$app/navigation";
import { tick } from "svelte";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal, type ITheme } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { errorText, terminalApi, terminalEvents } from "./api";
import {
  clipboardKey,
  decodeBase64,
  Drawn,
  EarlyOutput,
  endedText,
  InOrder,
  isClosedError,
  listenAll,
  pasteProblem,
  rightClickAction,
  tabTitle,
} from "./terminal";

export type TabState = "connecting" | "open" | "ended" | "failed";

export interface Tab {
  key: number;
  deviceId: number;
  title: string;
  /** Run in place of the login shell, if set. */
  command: string | null;
  state: TabState;
  /** Why the terminal could not be opened (failed). */
  error: string | null;
  /** What the tab says once the terminal ended (ended). */
  ended: string | null;
  /** Why it ended, if not normally: the connection was lost, a signal, an exit code (ended). */
  endReason: string | null;
  /** The program's exit code, when it ended with one. */
  exitStatus: number | null;
  /** Typing, a paste or a new size that didn't reach the machine. */
  sendError: string | null;
  /** The title the program running in it set, if any (shells often show the user and folder). */
  programTitle: string;
  /** True once a terminal has run in it, so there is something to show. */
  shown: boolean;
}

/** A tab's terminal on screen, and its link to the back end. Not reactive: xterm keeps its own state. */
interface Live {
  term: Terminal;
  fit: FitAddon;
  /** The element the terminal is drawn into. Kept while the screen is away, so nothing is redrawn. */
  el: HTMLDivElement;
  opened: boolean;
  /** The back end's id while a terminal is running in this tab. */
  id: number | null;
  drawn: Drawn;
  input: InOrder | null;
  /** Typed while connecting: sent as soon as the terminal runs. */
  typedEarly: string;
  /** Notices the terminal's letters changing size. */
  watch: ResizeObserver | null;
}

/** Output nobody claims within this long belongs to a tab that no longer exists. */
const UNCLAIMED_MS = 30_000;

function cssVar(name: string, fallback: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
}

/** The app's colours (app.css), with a palette for programs that use colour picked to sit well on them. */
function theme(): ITheme {
  return {
    background: cssVar("--bg-sunk", "#0a0d11"),
    foreground: cssVar("--text", "#e6edf3"),
    cursor: cssVar("--accent", "#2dd4bf"),
    cursorAccent: cssVar("--bg-sunk", "#0a0d11"),
    selectionBackground: "rgba(45, 212, 191, 0.32)",
    scrollbarSliderBackground: "rgba(139, 148, 158, 0.25)",
    scrollbarSliderHoverBackground: "rgba(139, 148, 158, 0.4)",
    scrollbarSliderActiveBackground: "rgba(139, 148, 158, 0.55)",
    black: "#484f58",
    red: "#ff7b72",
    green: "#3fb950",
    yellow: "#d29922",
    blue: "#58a6ff",
    magenta: "#bc8cff",
    cyan: "#39c5cf",
    white: "#b1bac4",
    brightBlack: "#6e7681",
    brightRed: "#ffa198",
    brightGreen: "#56d364",
    brightYellow: "#e3b341",
    brightBlue: "#79c0ff",
    brightMagenta: "#d2a8ff",
    brightCyan: "#56d4dd",
    brightWhite: "#f0f6fc",
  };
}

const FONT = '"JetBrains Mono", Consolas, "DejaVu Sans Mono", monospace';

/** A little larger than the app's text, which follows the display scaling. */
function fontSize(): number {
  const base = parseFloat(getComputedStyle(document.documentElement).fontSize) || 13;
  return Math.round(base * 1.08 * 2) / 2;
}

/** After the screen has had a chance to show a new tab, so it is measured at its real size. */
function settle(): Promise<void> {
  return tick().then(
    () => new Promise<void>((done) => {
      const timer = setTimeout(done, 150);
      requestAnimationFrame(() => {
        clearTimeout(timer);
        done();
      });
    }),
  );
}

class Terminals {
  tabs = $state<Tab[]>([]);
  /** The key of the tab shown. */
  active = $state<number | null>(null);
  /** A copy or paste that didn't work. */
  clipboardError = $state<string | null>(null);

  #live = new Map<number, Live>();
  /** Tab key for each running terminal's id. */
  #tabOf = new Map<number, number>();
  /** Terminals this screen closed: anything still on its way from them is dropped. */
  #closed = new Set<number>();
  #early = new EarlyOutput();
  #nextKey = 1;
  #started = false;
  /** Settles once terminals left over from before are closed and the screen hears what terminals print. */
  #ready: Promise<void> = Promise.resolve();
  /** Why the screen can't hear what terminals print, while it can't. No terminal can work then. */
  #eventsError: string | null = null;
  /** Setting up the listeners, while that is under way, so two tabs trying again don't do it twice. */
  #listenTry: Promise<void> | null = null;

  /** Called once when the app loads (+layout.svelte), and again harmlessly by open(). Terminals left
   *  behind by an earlier load of the window (a reload) have no tab any more, so they are closed now
   *  rather than staying connected; then the screen listens for output and ends. */
  start() {
    if (this.#started) return;
    this.#started = true;
    const leftOver = terminalApi.closeAll().catch(() => {});
    this.#ready = Promise.all([leftOver, this.#listen()]).then(() => {});
    setInterval(() => {
      for (const { id, running } of this.#early.unclaimed(UNCLAIMED_MS)) {
        if (running) terminalApi.close(id).catch(() => {});
      }
    }, UNCLAIMED_MS / 3);
  }

  /** Listen for output and ends. If that fails, #eventsError says why and the next connect tries again. */
  #listen(): Promise<void> {
    this.#listenTry ??= listenAll([
      () => terminalEvents.onOutput(({ id, data }) => this.#output(id, decodeBase64(data))),
      () => terminalEvents.onExit(({ id, reason, exitStatus }) => this.#exit(id, reason, exitStatus)),
    ])
      .then(
        () => {
          this.#eventsError = null;
        },
        (e) => {
          this.#eventsError = `The screen can't receive what terminals print: ${errorText(e)}`;
        },
      )
      .finally(() => {
        this.#listenTry = null;
      });
    return this.#listenTry;
  }

  /** Typing, a paste, a new size or a note of what was drawn didn't reach the machine. Shown on the tab.
   *  A terminal that has just closed is no news: the tab says the session ended. */
  #sendFailed(key: number, e: unknown) {
    const text = errorText(e);
    if (isClosedError(text)) return;
    const tab = this.tab(key);
    if (tab) tab.sendError = text;
  }

  /** The owner has seen the problem with sending. */
  dismissSendError(key: number) {
    const tab = this.tab(key);
    if (tab) tab.sendError = null;
  }

  tab(key: number): Tab | undefined {
    return this.tabs.find((t) => t.key === key);
  }

  /** A new tab, shown at once, that connects by itself. Returns its key. */
  open(o: { deviceId: number; title: string; command?: string | null }): number {
    this.start();
    const key = this.#nextKey++;
    this.tabs.push({
      key,
      deviceId: o.deviceId,
      title: tabTitle(o.title, this.tabs.map((t) => t.title)),
      command: o.command?.trim() || null,
      state: "connecting",
      error: null,
      ended: null,
      endReason: null,
      exitStatus: null,
      sendError: null,
      programTitle: "",
      shown: false,
    });
    this.#live.set(key, this.#create(key));
    this.active = key;
    void this.#connect(key);
    return key;
  }

  /** Connect a tab whose terminal ended or never started, keeping what it showed. */
  reconnect(key: number) {
    const live = this.#live.get(key);
    if (!live || live.id != null || this.tab(key)?.state === "connecting") return;
    live.term.write("\r\n");
    void this.#connect(key);
  }

  /** Close the tab and its connection. */
  close(key: number) {
    const live = this.#live.get(key);
    if (live) {
      if (live.id != null) {
        this.#closed.add(live.id);
        this.#tabOf.delete(live.id);
        terminalApi.close(live.id).catch(() => {});
      }
      live.input?.stop();
      live.watch?.disconnect();
      live.term.dispose();
      live.el.remove();
      this.#live.delete(key);
    }
    const i = this.tabs.findIndex((t) => t.key === key);
    if (i < 0) return;
    this.tabs.splice(i, 1);
    if (this.active === key) this.active = this.tabs[Math.min(i, this.tabs.length - 1)]?.key ?? null;
  }

  /** Put a tab's terminal into `host` on the screen. */
  attach(key: number, host: HTMLElement) {
    const live = this.#live.get(key);
    if (!live) return;
    host.appendChild(live.el);
    if (!live.opened) {
      live.term.open(live.el);
      live.opened = true;
      // The terminal measures its letters when it opens; if the font arrives later, it measures again.
      document.fonts?.load(`${live.term.options.fontSize}px "JetBrains Mono"`).then(() => {
        live.term.options.fontFamily = FONT;
      }, () => {});
      // Letters of a new size change the size of the drawing, though not of its place on the screen:
      // fit again then, or the bottom rows end up out of sight.
      const drawing = live.el.querySelector(".xterm-screen");
      if (drawing) {
        live.watch = new ResizeObserver(() => this.fit(key));
        live.watch.observe(drawing);
      }
    }
    this.fit(key);
  }

  /** Take a tab's terminal off the screen, keeping it running. */
  detach(key: number) {
    this.#live.get(key)?.el.remove();
  }

  /** Make the terminal fill its place on the screen. The program is told the new size. */
  fit(key: number) {
    const live = this.#live.get(key);
    // Hidden tabs have no size; they are fitted when shown.
    if (!live?.opened || !live.el.isConnected || live.el.clientWidth === 0 || live.el.clientHeight === 0) return;
    try {
      live.fit.fit();
    } catch {
      // Not measurable yet; the next resize fits it.
    }
  }

  focus(key: number) {
    this.#live.get(key)?.term.focus();
  }

  #create(key: number): Live {
    const el = document.createElement("div");
    el.className = "terminal-host";
    const term = new Terminal({
      fontFamily: FONT,
      fontSize: fontSize(),
      cursorBlink: true,
      cursorInactiveStyle: "outline",
      scrollback: 5000,
      theme: theme(),
      // While a program follows the mouse, Shift+drag still selects text (xterm.js does this by itself).
      allowProposedApi: false,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    const live: Live = { term, fit, el, opened: false, id: null, drawn: new Drawn(), input: null, typedEarly: "", watch: null };

    term.onData((data) => {
      if (live.input) live.input.push(data);
      else if (this.tab(key)?.state === "connecting" && live.typedEarly.length < 64 * 1024) live.typedEarly += data;
    });
    // Mouse clicks and the wheel in the oldest mouse format (vim uses it with xterm.js) come here, as one
    // character per byte. Not kept while connecting: a click then means nothing to the program.
    term.onBinary((data) => live.input?.push(data, "binary"));
    term.onResize(({ cols, rows }) => {
      if (live.id != null) terminalApi.resize(live.id, cols, rows).catch((e) => this.#sendFailed(key, e));
    });
    term.onTitleChange((title) => {
      const tab = this.tab(key);
      if (tab) tab.programTitle = title.slice(0, 200);
    });
    term.attachCustomKeyEventHandler((e) => {
      const action = clipboardKey(e, term.hasSelection());
      if (!action) return true;
      // Also stops the web view pasting by itself (it treats Ctrl+Shift+V as paste).
      e.preventDefault();
      if (e.type === "keydown") void (action === "copy" ? this.#copy(live) : this.#paste(live));
      return false;
    });
    el.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      const action = rightClickAction(term.hasSelection(), term.modes.mouseTrackingMode !== "none", e.shiftKey);
      if (action) void (action === "copy" ? this.#copy(live) : this.#paste(live));
    });
    return live;
  }

  async #copy(live: Live) {
    const text = live.term.getSelection();
    live.term.clearSelection();
    if (!text) return;
    try {
      await terminalApi.clipboardWrite(text);
      this.clipboardError = null;
    } catch (e) {
      this.clipboardError = `Couldn't copy: ${errorText(e)}`;
    }
  }

  async #paste(live: Live) {
    if (!live.input) return;
    try {
      const text = await terminalApi.clipboardRead();
      const tooBig = pasteProblem(text);
      this.clipboardError = tooBig && `Couldn't paste: ${tooBig}`;
      // paste() marks it as pasted for programs that ask (vim, the shell), and sends Enter as a terminal does.
      if (text && !tooBig && live.input) live.term.paste(text);
    } catch (e) {
      this.clipboardError = `Couldn't paste: ${errorText(e)}`;
    }
    live.term.focus();
  }

  async #connect(key: number) {
    const tab = this.tab(key);
    const live = this.#live.get(key);
    if (!tab || !live) return;
    tab.state = "connecting";
    tab.error = null;
    tab.ended = null;
    tab.endReason = null;
    tab.exitStatus = null;
    tab.sendError = null;
    await Promise.all([settle(), this.#ready]);
    // Listening failed when the app loaded, or for an earlier tab: try again (this is what "Try again" does).
    if (this.#eventsError) await this.#listen();
    if (this.#eventsError) {
      live.typedEarly = "";
      const still = this.tab(key);
      if (still) {
        still.state = "failed";
        still.error = this.#eventsError;
      }
      return;
    }
    if (this.#live.get(key) !== live) return;
    this.fit(key);
    const { cols, rows } = live.term;
    let id: number;
    try {
      id = await terminalApi.open(tab.deviceId, tab.command, cols, rows);
    } catch (e) {
      live.typedEarly = "";
      const still = this.tab(key);
      if (still) {
        still.state = "failed";
        still.error = errorText(e);
      }
      return;
    }
    // Closed while it was connecting.
    if (this.#live.get(key) !== live) {
      this.#closed.add(id);
      terminalApi.close(id).catch(() => {});
      return;
    }
    live.id = id;
    live.drawn = new Drawn();
    this.#tabOf.set(id, key);
    live.input = new InOrder(
      (data, kind) => (kind === "binary" ? terminalApi.writeBinary(id, data) : terminalApi.write(id, data)),
      (e) => this.#sendFailed(key, e),
    );
    live.input.push(live.typedEarly);
    live.typedEarly = "";
    const current = this.tab(key);
    if (current) {
      current.state = "open";
      current.shown = true;
    }
    // The tab may have changed size while connecting.
    if (live.term.cols !== cols || live.term.rows !== rows) {
      terminalApi.resize(id, live.term.cols, live.term.rows).catch((e) => this.#sendFailed(key, e));
    }
    const early = this.#early.take(id);
    for (const chunk of early?.chunks ?? []) this.#draw(key, live, id, chunk);
    if (early?.exit) this.#ended(key, early.exit.reason, early.exit.exitStatus);
    // Once the screen shows the terminal (shown, above), so the keyboard can go to it.
    await tick();
    if (this.active === key && this.#live.get(key) === live) live.term.focus();
  }

  #draw(key: number, live: Live, id: number, bytes: Uint8Array) {
    live.term.write(bytes, () => {
      const report = live.drawn.add(bytes.length);
      if (report && live.id === id) terminalApi.ack(id, report).catch((e) => this.#sendFailed(key, e));
    });
  }

  #output(id: number, bytes: Uint8Array) {
    if (this.#closed.has(id)) return;
    const key = this.#tabOf.get(id);
    const live = key == null ? undefined : this.#live.get(key);
    if (key == null || !live || live.id !== id) {
      this.#early.output(id, bytes);
      return;
    }
    this.#draw(key, live, id, bytes);
  }

  #exit(id: number, reason: string | null, exitStatus: number | null) {
    if (this.#closed.delete(id)) return;
    const key = this.#tabOf.get(id);
    if (key == null) {
      this.#early.exit(id, reason, exitStatus);
      return;
    }
    this.#ended(key, reason, exitStatus);
  }

  #ended(key: number, reason: string | null, exitStatus: number | null) {
    const live = this.#live.get(key);
    if (live) {
      if (live.id != null) this.#tabOf.delete(live.id);
      live.id = null;
      live.input?.stop();
      live.input = null;
    }
    const tab = this.tab(key);
    if (tab) {
      tab.state = "ended";
      tab.ended = endedText(reason);
      tab.endReason = reason;
      tab.exitStatus = exitStatus;
    }
  }
}

export const terminals = new Terminals();

/** Open a terminal tab on a machine and show the Terminals screen. `command` runs in place of the login
 *  shell, with a terminal of its own, for example "docker exec -it web sh" or "pct enter 101". */
export async function openTerminal(o: { deviceId: number; title: string; command?: string | null }): Promise<number> {
  const key = terminals.open(o);
  await goto("/terminals");
  return key;
}
