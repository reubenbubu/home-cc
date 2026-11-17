// A pretend terminal for the sample back end (mock/index.ts), so the Terminals screen can be tried and
// screenshotted in a browser. It prints a prompt, echoes what is typed and knows a few commands; nothing
// reaches a real machine. It follows the same interface as src-tauri/src/terminal.rs.
import { emit } from "@tauri-apps/api/event";
import type { Os } from "../api";

/** What the pretend terminal needs to know about a sample machine. */
export interface MockMachine {
  name: string;
  user: string | null;
  hostname: string;
  os: Os;
  /** Why logging in would fail right now, as the real back end would say it; null if it works. */
  problem: string | null;
}

const sleep = (msecs: number) => new Promise<void>((r) => setTimeout(r, msecs));

function base64(text: string): string {
  let raw = "";
  for (const byte of new TextEncoder().encode(text)) raw += String.fromCharCode(byte);
  return btoa(raw);
}

const ESC = "\x1b[";
const bold = (code: string, text: string) => `${ESC}1;${code}m${text}${ESC}0m`;
const colour = (code: string, text: string) => `${ESC}${code}m${text}${ESC}0m`;

class Fake {
  #line = "";
  #closed = false;
  readonly id: number;
  readonly machine: MockMachine;
  readonly command: string | null;
  cols: number;
  rows: number;
  readonly #ended: (id: number, reason: string | null, exitStatus: number | null) => void;

  constructor(
    id: number,
    machine: MockMachine,
    command: string | null,
    cols: number,
    rows: number,
    ended: (id: number, reason: string | null, exitStatus: number | null) => void,
  ) {
    this.id = id;
    this.machine = machine;
    this.command = command;
    this.cols = cols;
    this.rows = rows;
    this.#ended = ended;
  }

  #out(text: string) {
    if (!this.#closed) emit("terminal-output", { id: this.id, data: base64(text) });
  }

  #prompt() {
    const m = this.machine;
    if (this.command) this.#out("/ # ");
    else if (m.os === "windows") this.#out(`${m.user}@${m.hostname} C:\\Users\\${m.user}>`);
    else this.#out(`${bold("32", `${m.user}@${m.hostname}`)}:${bold("34", "~")}$ `);
  }

  start() {
    const m = this.machine;
    if (this.command) {
      this.#out(colour("2", `(sample) ${this.command}`) + "\r\n");
    } else if (m.os === "windows") {
      this.#out("Microsoft Windows [Version 10.0.26100.2605]\r\n(c) Microsoft Corporation. All rights reserved.\r\n\r\n");
    } else {
      this.#out(`Welcome to ${m.name}.\r\n\r\n`);
    }
    this.#out(
      colour("33", "This is the sample back end: nothing here reaches a real machine.") +
        "\r\nTry ls, docker ps, colours, clear or exit.\r\n\r\n",
    );
    this.#prompt();
  }

  type(data: string) {
    // Arrow keys, function keys and the like: a real shell would act on them; this one ignores them.
    if (data.startsWith("\x1b")) return;
    for (const ch of data) {
      if (ch === "\r") {
        this.#out("\r\n");
        const line = this.#line.trim();
        this.#line = "";
        if (this.#run(line)) this.#prompt();
      } else if (ch === "\x7f" || ch === "\b") {
        if (this.#line) {
          this.#line = this.#line.slice(0, -1);
          this.#out("\b \b");
        }
      } else if (ch === "\x03") {
        this.#line = "";
        this.#out("^C\r\n");
        this.#prompt();
      } else if (ch === "\x04" && !this.#line) {
        this.#run("exit");
      } else if (ch >= " ") {
        this.#line += ch;
        this.#out(ch);
      }
    }
  }

  /** Runs one line. False when the session ended. */
  #run(line: string): boolean {
    const [cmd] = line.split(/\s+/);
    switch (cmd) {
      case "":
        return true;
      case "exit":
      case "logout":
        this.close(null, 0);
        return false;
      case "clear":
      case "cls":
        this.#out(`${ESC}2J${ESC}H`);
        return true;
      case "ls":
      case "dir":
        this.#out(
          [
            bold("34", "compose"),
            bold("34", "backups"),
            bold("34", "scripts"),
            "notes.txt",
            bold("32", "update.sh"),
            colour("36", "media -> /mnt/media"),
          ].join("  ") + "\r\n",
        );
        return true;
      case "docker":
        this.#out(
          [
            "CONTAINER ID   IMAGE                          STATUS        PORTS                  NAMES",
            "3f2a9c1d7b44   jellyfin/jellyfin:10.10        Up 4 days     0.0.0.0:8096->8096     jellyfin",
            "a81e0f5c2d93   ghcr.io/home-assistant:2025.1  Up 4 days                            homeassistant",
            "c07d4b8e1f22   nginx:1.27-alpine              Up 2 hours    0.0.0.0:443->443       proxy",
          ].join("\r\n") + "\r\n",
        );
        return true;
      case "colours":
      case "colors": {
        let row = "";
        for (let i = 0; i < 8; i++) row += `${ESC}${40 + i}m  ${ESC}0m`;
        row += "\r\n";
        for (let i = 0; i < 8; i++) row += `${ESC}${100 + i}m  ${ESC}0m`;
        this.#out(`${row}\r\n${bold("31", "red")} ${bold("32", "green")} ${bold("33", "yellow")} ${bold("34", "blue")} ${bold("35", "magenta")} ${bold("36", "cyan")}\r\n`);
        return true;
      }
      case "size":
        this.#out(`${this.cols} columns, ${this.rows} rows\r\n`);
        return true;
      default:
        this.#out(`${cmd}: command not found\r\n`);
        return true;
    }
  }

  close(reason: string | null, exitStatus: number | null) {
    if (this.#closed) return;
    this.#closed = true;
    // As from the real back end: the end comes after the last of the output.
    setTimeout(() => this.#ended(this.id, reason, exitStatus), 0);
  }
}

/** The terminal commands of the sample back end. `lookup` describes a sample machine. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export function terminalCommands(lookup: (deviceId: number) => MockMachine): Record<string, (args: any) => unknown> {
  let nextId = 1;
  let clipboard = "echo pasted from the clipboard";
  const open = new Map<number, Fake>();
  const ended = (id: number, reason: string | null, exitStatus: number | null) => {
    open.delete(id);
    emit("terminal-exit", { id, reason, exitStatus });
  };
  const get = (id: number) => {
    const fake = open.get(id);
    if (!fake) throw "This terminal has already closed.";
    return fake;
  };

  return {
    terminal_open: async ({ deviceId, command, cols, rows }: { deviceId: number; command: string | null; cols: number; rows: number }) => {
      const m = lookup(deviceId);
      if (!m.user) throw `${m.name} has no SSH user, so home-cc can't log in to it. Add one with Edit on the Machines screen.`;
      await sleep(350);
      if (m.problem) throw m.problem;
      const id = nextId++;
      const fake = new Fake(id, m, command?.trim() || null, cols, rows, ended);
      open.set(id, fake);
      // The first output can come before the answer, as it can from the real back end.
      fake.start();
      return id;
    },
    terminal_write: ({ id, data }: { id: number; data: string }) => get(id).type(data),
    terminal_resize: ({ id, cols, rows }: { id: number; cols: number; rows: number }) => {
      const fake = get(id);
      fake.cols = cols;
      fake.rows = rows;
    },
    terminal_ack: () => {},
    terminal_close: ({ id }: { id: number }) => open.get(id)?.close(null, null),
    terminal_close_all: () => {
      for (const fake of open.values()) fake.close(null, null);
    },
    clipboard_read_text: () => clipboard,
    clipboard_write_text: ({ text }: { text: string }) => {
      clipboard = text;
    },
  };
}
