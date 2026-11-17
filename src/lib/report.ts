// Turns a problem shown on a screen into a prompt the owner can paste into a
// coding agent such as Claude Code. Three kinds: "I have a bug" asks it to find
// what went wrong in home-cc's own code; "Investigate" and "Remediate" send it
// to the machine or service the problem is about, to look into it or to fix it
// there (offered only when there is such a machine: see promptKinds). Pure
// functions only, so it can be tested without the app;
// components/AgentPrompt.svelte gathers the context.

export const REPO_URL = "https://github.com/reubenbubu/home-cc";

/** Which part of the app a problem came from. Decides where the agent is told to start looking. */
export type Area =
  | "stats"
  | "trust"
  | "scan"
  | "status"
  | "devices"
  | "secrets"
  | "settings"
  | "live"
  | "zoom"
  | "screen"
  | "keys"
  // Terminals
  | "terminal"
  // Docker (spec 5.5)
  | "docker"
  // Proxmox (spec 5.6)
  | "proxmox"
  // Pi-hole
  | "pihole";

/** Source files to start from for each area. Paths are from the root of the repository. */
export const AREA_FILES: Record<Area, string[]> = {
  stats: [
    "src-tauri/src/ssh.rs (connecting and logging in)",
    "src-tauri/src/stats.rs and src-tauri/src/stats_*.{sh,ps1} (what is run on the machine and how it is read)",
    "src-tauri/src/localstats.rs (this PC's own stats, read without SSH)",
    "src-tauri/src/monitor.rs (the timed stats rounds)",
    "src/lib/stats.svelte.ts and src/lib/components/MachineCard.svelte (the screen)",
  ],
  trust: [
    "src-tauri/src/monitor.rs (trust_host_key)",
    "src-tauri/src/hostkeys.rs and src-tauri/src/ssh.rs (machine identities)",
    "src/lib/stats.svelte.ts (the screen side)",
  ],
  scan: [
    "src-tauri/src/net/scan.rs (the scan)",
    "src-tauri/src/net/ifaces.rs and src-tauri/src/net/lan.rs (working out the network to scan)",
    "src-tauri/src/net/ping.rs, src-tauri/src/net/winping.rs, src-tauri/src/net/neigh.rs and src-tauri/src/net/names.rs",
    "src/lib/scan.svelte.ts (the screen side)",
  ],
  status: [
    "src-tauri/src/net/status.rs (checking devices)",
    "src-tauri/src/net/ping.rs and src-tauri/src/net/winping.rs (pinging)",
    "src-tauri/src/monitor.rs and src-tauri/src/history.rs",
    "src/lib/status.svelte.ts (the screen side)",
  ],
  devices: [
    "src-tauri/src/inventory.rs (saved devices, SQLite)",
    "src-tauri/src/commands.rs",
    "src/lib/devices.svelte.ts and the screen that showed the problem",
  ],
  secrets: [
    "src-tauri/src/secrets.rs (passwords in the system keychain)",
    "src-tauri/src/commands.rs (set_secret, has_secret, delete_secret)",
  ],
  settings: [
    "src-tauri/src/settings.rs (saving and checking settings)",
    "src/lib/settings.svelte.ts and src/routes/settings/+page.svelte",
  ],
  zoom: [
    "src/lib/zoom.svelte.ts and src/lib/zoom.ts (zooming the app and the terminals' text, remembering the sizes)",
    "src-tauri/capabilities/default.json (the permission to zoom the window)",
  ],
  live: [
    "src/lib/live.svelte.ts (listening for back-end events)",
    "src-tauri/src/monitor.rs (what sends the events)",
  ],
  keys: [
    "src-tauri/src/keysetup.rs (this PC's key, making one, and adding it to a machine: the scripts run there)",
    "src-tauri/src/ssh.rs (logging in, and which key files are offered)",
    "src/lib/components/KeySetup.svelte and src/lib/keysetup.ts (the screen)",
  ],
  // Terminals
  terminal: [
    "src-tauri/src/terminal.rs (the terminal over SSH, and copy and paste)",
    "src-tauri/src/ssh.rs and src-tauri/src/monitor.rs (connecting and logging in: login_for)",
    "src/lib/terminals.svelte.ts and src/lib/terminal.ts (the tabs) and src/routes/terminals/+page.svelte (the screen)",
  ],
  screen: [
    "the screen named under App, in src/routes/ (its +page.svelte) and the components it uses in src/lib/components/",
    "src/lib/api.ts (the shapes the back end sends)",
  ],
  // --- Docker (spec 5.5) ---
  docker: [
    "src-tauri/src/docker/mod.rs (connections to Docker hosts and every action)",
    "src-tauri/src/docker/scripts.rs (what is run on the host) and src-tauri/src/docker/parse.rs (how docker's output is read)",
    "src-tauri/src/docker/logs.rs (following a log) and src-tauri/src/docker/store.rs (Docker hosts and stacks remembered)",
    "src-tauri/src/docker/updates.rs (checking images for updates: image names, registry answers, the comparison)",
    "src-tauri/src/ssh.rs and src-tauri/src/monitor.rs (prepared_login: logging in)",
    "src/routes/docker/+page.svelte, src/lib/docker/ and src/lib/components/docker/ (the screen)",
  ],
  // --- Proxmox ---
  proxmox: [
    "src-tauri/src/proxmox/client.rs (the HTTPS calls to the Proxmox API) and src-tauri/src/proxmox/pin.rs (the approved certificate)",
    "src-tauri/src/proxmox/mod.rs (what each action asks Proxmox, and how failures are worded) and src-tauri/src/proxmox/model.rs (reading Proxmox's answers)",
    "src-tauri/src/proxmox/store.rs (token ID, port and certificate per device)",
    "src/lib/proxmox.svelte.ts, src/lib/proxmox.ts and src/routes/proxmox/ with src/lib/components/proxmox/ (the screen)",
  ],
  // --- Pi-hole ---
  pihole: [
    "src-tauri/src/pihole/client.rs (one HTTPS or HTTP call to the Pi-hole; the certificate check is src-tauri/src/proxmox/pin.rs)",
    "src-tauri/src/pihole/auth.rs (logging in to Pi-hole's API, keeping and renewing the session, and how login failures are worded)",
    "src-tauri/src/pihole/mod.rs (what a reading and each action ask Pi-hole) and src-tauri/src/pihole/model.rs (reading Pi-hole's answers)",
    "src-tauri/src/pihole/store.rs (which devices are Pi-holes, with their scheme, port and certificate)",
    "src/lib/pihole/ (store, comparison and wording) and src/routes/pihole/ with src/lib/components/pihole/ (the screen)",
  ],
};

export type Details = Record<string, string | number | boolean | null | undefined>;

/** A problem as a screen shows it. */
export interface Problem {
  area: Area;
  /** What the owner was doing or looking at, such as 'Reading stats from "nas"'. */
  what: string;
  /** The message on screen, word for word. */
  error: string;
  /** Facts that help find the cause: the device, the kind of error, and so on. Never secrets. */
  details?: Details;
  /** A stack trace or other long text, shown as it is. */
  trace?: string;
  /** What the problem is about when that differs from its area: "home-cc" for home-cc itself although the
   *  area is about machines (a list it couldn't load from its own back end, a setting or password it couldn't
   *  store), so there is nothing on a machine to investigate or fix; "this PC" for this PC rather than the
   *  machine named in the details (this PC's own SSH key, say). */
  about?: "home-cc" | "this PC";
}

/** Which prompt: look into it on the machine, fix it on the machine, or find the bug in home-cc. */
export type PromptKind = "investigate" | "remediate" | "bug";

/** The words on each prompt's button, and what it asks for (its tooltip). */
export const PROMPT_KINDS: Record<PromptKind, { label: string; asks: string }> = {
  investigate: {
    label: "Investigate",
    asks: "Copies a prompt asking an agent to look into this on the machine itself and report back, changing nothing",
  },
  remediate: {
    label: "Remediate",
    asks: "Copies a prompt asking an agent to fix this on the machine itself, saying first what it will change and asking before anything destructive",
  },
  bug: {
    label: "I have a bug",
    asks: "Copies a prompt asking an agent to find out why home-cc showed this, and to fix home-cc's code if the fault is there",
  },
};

/** What a problem in each area is about, outside home-cc: a saved machine or a service on it ("machine"),
 *  this PC and the network around it ("this PC", for pinging and scanning), or nothing but home-cc itself
 *  (null: its saved devices, settings, keychain, zoom, live updates, screens). The one place that decides
 *  which problems get "Investigate" and "Remediate" (with reachOf, and a problem's own `about` where it differs
 *  from its area). */
export const AREA_REACH: Record<Area, "machine" | "this PC" | null> = {
  stats: "machine",
  trust: "machine",
  keys: "machine",
  terminal: "machine",
  docker: "machine",
  proxmox: "machine",
  pihole: "machine",
  scan: "this PC",
  status: "this PC",
  devices: null,
  secrets: null,
  settings: null,
  live: null,
  zoom: null,
  screen: null,
};

/** Where an agent sent to investigate or fix a problem is to go, and how it gets there. */
export type Reach =
  | {
      on: "machine";
      name: string;
      address: string;
      /** The name the address resolves to, when there is one. */
      hostname?: string;
      /** Set when home-cc logs in to it over SSH. */
      ssh?: { user: string; port: number };
      /** The service's own web address, when home-cc reaches it that way (Proxmox, Pi-hole). */
      api?: { what: string; url: string };
    }
  | { on: "this PC"; name?: string };

const textOf = (v: Details[string]) => (typeof v === "string" && v.trim() ? v.trim() : undefined);

/** Where the problem is, outside home-cc, or null when it is about home-cc itself. A machine's problem
 *  needs the machine's address among its details (deviceDetails), or there is nowhere to send an agent. A
 *  problem's `about` comes before its area. */
export function reachOf(p: Problem): Reach | null {
  const area = p.about === "home-cc" ? null : p.about ?? AREA_REACH[p.area];
  if (area === "this PC") return { on: "this PC" };
  if (area !== "machine") return null;
  const d = p.details ?? {};
  const address = textOf(d["Address"]);
  if (!address) return null;
  const name = textOf(d["Device"]) ?? address;
  // This PC's own stats are read without SSH: the agent works here, not on another machine.
  if (d["Read on this PC without SSH"] === true) return { on: "this PC", name };
  const reach: Reach = { on: "machine", name, address };
  const hostname = textOf(d["Resolved hostname"]);
  // Only an actual name, not "none found" or "still being looked up".
  if (hostname && !/\s/.test(hostname) && hostname !== address) reach.hostname = hostname;
  const user = textOf(d["SSH user"]);
  if (user) reach.ssh = { user, port: typeof d["SSH port"] === "number" ? d["SSH port"] : 22 };
  // An IPv6 address goes in brackets in a web address, or its last part reads as the port.
  const inUrl = address.includes(":") ? `[${address}]` : address;
  if (p.area === "proxmox") {
    const port = typeof d["API port"] === "number" ? d["API port"] : 8006;
    reach.api = { what: "Proxmox's web page and API", url: `https://${inUrl}:${port}` };
  } else if (p.area === "pihole" && typeof d["Port"] === "number") {
    const scheme = d["Connection"] === "plain HTTP" ? "http" : "https";
    reach.api = { what: "Pi-hole's web page and API", url: `${scheme}://${inUrl}:${d["Port"]}/admin` };
  }
  return reach;
}

/** The prompts offered under a problem, in the order shown: all three when it is about a machine or
 *  service, only "I have a bug" when it is about home-cc itself. */
export function promptKinds(p: Problem): PromptKind[] {
  return reachOf(p) ? ["investigate", "remediate", "bug"] : ["bug"];
}

export interface LogLine {
  /** Unix seconds. */
  at: number;
  text: string;
}

/** Everything around the problem. Each part may be missing if it couldn't be read. */
export interface Context {
  version?: string;
  os?: string;
  arch?: string;
  release?: boolean;
  /** Git commit the app was built from, if known. */
  commit?: string;
  /** Path of the screen, such as "/machines". */
  screen?: string;
  /** When the report was made (Unix seconds). */
  now: number;
  /** The standing problem with status checks, if there is one. */
  checkProblem?: string | null;
  settings?: Details;
  /** Other problems shown this session, oldest first. Should not include this one. */
  recent?: { at: number; what: string; error: string }[];
  /** What the back end noted by itself, oldest first. */
  backendLog?: LogLine[];
  /** Parts of the context that couldn't be read, with why. */
  missing?: string[];
}

/** At most this many lines of each list, the newest ones, so the prompt stays readable. */
export const MAX_LINES = 15;

/** Local time like "2026-11-17 14:03:09", so lines from the app and the log can be lined up. */
export function stamp(unixSecs: number): string {
  const d = new Date(unixSecs * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

/** A fenced block that can't be closed early by backticks inside the text. */
export function fenced(text: string): string {
  const longest = Math.max(2, ...[...text.matchAll(/`+/g)].map((m) => m[0].length));
  const fence = "`".repeat(longest + 1);
  return `${fence}text\n${text}\n${fence}`;
}

function detailLines(details: Details | undefined): string[] {
  return Object.entries(details ?? {})
    .filter(([, v]) => v !== undefined && v !== null && v !== "")
    .map(([k, v]) => `- ${k}: ${typeof v === "boolean" ? (v ? "yes" : "no") : v}`);
}

function newest<T>(list: T[] | undefined): { shown: T[]; dropped: number } {
  const all = list ?? [];
  const shown = all.slice(-MAX_LINES);
  return { shown, dropped: all.length - shown.length };
}

const ABOUT_HOME_CC = `I use home-cc (${REPO_URL}), a desktop app for managing my home network, built with Tauri 2, Rust and Svelte.`;

/** "I have a bug": find out why home-cc showed it, starting from home-cc's own code. */
function bugIntro(): string[] {
  return [
    `${ABOUT_HOME_CC} It showed the problem below. Please find out why it happened.`,
    "",
    "- Start from the source of home-cc. If you are not in a checkout of it, clone the repository first, " +
      "and check out the commit named under App if one is given, so you read the code I am running.",
    "- Decide whether this is a bug or limitation in home-cc, or something to change on my machines or network. " +
      "Say which, and how sure you are.",
    "- If it is a bug, fix it with the smallest change that makes sense, add or update a test, and tell me what changed.",
    "- If it is on my side, give me the exact steps to fix it, saying which machine to run each command on.",
    "- If you can't tell from what is here, tell me what to check or run and what to send back.",
  ];
}

function whereName(reach: Reach, area: Area): string {
  if (reach.on === "machine") return `my machine "${reach.name}"`;
  if (reach.name) return `this PC ("${reach.name}"), the one home-cc runs on`;
  return AREA_REACH[area] === "this PC" ? "this PC, the one home-cc runs on, or the network around it" : "this PC, the one home-cc runs on";
}

/** "Investigate" and "Remediate": go to the machine and look, or fix it there. */
function machineIntro(kind: "investigate" | "remediate", reach: Reach, area: Area): string[] {
  const go = "- Go to the machine named under Where it is, the way it says there, and find the cause on it.";
  const ifHomeCc =
    "the source of home-cc is at the address above, and the files to start from are listed at the end.";
  if (kind === "investigate") {
    return [
      `${ABOUT_HOME_CC} It showed the problem below, about ${whereName(reach, area)}. ` +
        "Please look into it there and tell me what is causing it. Don't change anything.",
      "",
      go,
      "- Read whatever helps (logs, services and containers, disk space, settings), but change nothing: " +
        "no restarts, installs, edits or deletes. If something can only be found out by changing something, ask me first.",
      "- Tell me what is causing it and how sure you are, what you looked at to find out, and what would fix it: " +
        "the exact steps, saying which machine to run each command on.",
      `- If what home-cc reports doesn't match what you find on the machine, the fault may be in home-cc instead. Say so: ${ifHomeCc}`,
      "- If you can't reach the machine, tell me what to run there and what to send back.",
    ];
  }
  return [
    `${ABOUT_HOME_CC} It showed the problem below, about ${whereName(reach, area)}. Please fix it there.`,
    "",
    go,
    "- Before you change anything, tell me what you will change and why.",
    "- Ask me, and wait for my yes, before anything destructive or hard to undo: deleting or overwriting files or data, " +
      "removing containers, images or volumes, stopping or restarting services, containers or VMs, rebooting, " +
      "or changing the firewall, the network settings or how I log in.",
    "- Make the smallest change that fixes it, and keep a copy of any file before you edit it.",
    "- Afterwards, check that it is fixed (home-cc should stop showing the problem on its next reading), " +
      "and tell me exactly what you changed, so I can undo it.",
    `- If the cause is in home-cc rather than on the machine, say so and change nothing on the machine: ${ifHomeCc}`,
    "- If you can't reach the machine, tell me what to run there and what to send back.",
  ];
}

/** Which machine to reach and how, for "Investigate" and "Remediate". Never a password or token. */
function reachLines(reach: Reach, area: Area): string[] {
  if (reach.on === "this PC") {
    const lines = [
      `- ${reach.name ? `This PC ("${reach.name}")` : "This PC"}: the one home-cc runs on (see Running on, under App), ` +
        "probably the one you are working on now. Work here; no SSH is needed.",
    ];
    if (AREA_REACH[area] === "this PC") lines.push("- The devices home-cc checks and scans are on the network around it.");
    return lines;
  }
  const lines = [`- Machine: "${reach.name}" at ${reach.address}${reach.hostname ? ` (hostname ${reach.hostname})` : ""}`];
  if (reach.ssh) {
    lines.push(
      `- Over SSH, as home-cc does: \`ssh -p ${reach.ssh.port} ${reach.ssh.user}@${reach.address}\`. ` +
        "home-cc logs in from this PC, with its SSH key (the SSH key file under Settings) or a password stored for it. " +
        "If a password is asked for, ask me to log in myself or to add your key; never ask me to paste one here.",
    );
    if (area === "docker") lines.push("- Docker: run docker and docker compose there, over that SSH login, as home-cc does.");
  } else {
    lines.push("- home-cc has no SSH login for it. If you need to log in to it, ask me how.");
  }
  if (reach.api) {
    lines.push(`- ${reach.api.what}: ${reach.api.url} (the token or password home-cc uses is not included)`);
  }
  return lines;
}

/** The prompt, as Markdown. "Investigate" and "Remediate" fall back to "I have a bug" for a problem
 *  about home-cc itself (promptKinds doesn't offer them there). */
export function agentPrompt(problem: Problem, ctx: Context, kind: PromptKind = "bug"): string {
  const reach = kind === "bug" ? null : reachOf(problem);
  const out: string[] = [];
  out.push(
    ...(reach && kind !== "bug" ? machineIntro(kind, reach, problem.area) : bugIntro()),
    "",
    "## What happened",
    "",
    `${problem.what}. home-cc showed this message:`,
    "",
    fenced(problem.error.trim()),
  );

  if (reach) out.push("", "## Where it is", "", ...reachLines(reach, problem.area));

  const details = detailLines(problem.details);
  if (details.length) out.push("", "## Details", "", ...details);
  if (problem.trace?.trim()) out.push("", "## Stack trace", "", fenced(problem.trace.trim()));

  out.push("", "## App", "");
  const version = [ctx.version ? `home-cc ${ctx.version}` : "home-cc, version not known"];
  if (ctx.release === false) version.push("development build");
  out.push(`- ${version.join(", ")}`);
  out.push(`- Built from commit: ${ctx.commit || "not known"}`);
  if (ctx.os) out.push(`- Running on: ${ctx.os}${ctx.arch ? ` (${ctx.arch})` : ""}`);
  if (ctx.screen) out.push(`- Screen: ${ctx.screen}`);
  out.push(`- Time: ${stamp(ctx.now)} (local time on this PC)`);

  if (ctx.checkProblem) {
    out.push("", "## Standing problem", "", "Status checks were not working at the time:", "", fenced(ctx.checkProblem.trim()));
  }

  const settings = detailLines(ctx.settings);
  if (settings.length) out.push("", "## Settings", "", ...settings);

  const recent = newest(ctx.recent);
  if (recent.shown.length) {
    out.push("", "## Other problems shown recently (oldest first)", "");
    if (recent.dropped) out.push(`(${recent.dropped} older ones left out)`);
    for (const r of recent.shown) out.push(`- ${stamp(r.at)}: ${r.what}: ${r.error.replace(/\s+/g, " ").trim()}`);
  }

  const log = newest(ctx.backendLog);
  if (log.shown.length) {
    out.push("", "## Back-end log (oldest first)", "");
    if (log.dropped) out.push(`(${log.dropped} older lines left out)`);
    out.push(fenced(log.shown.map((l) => `${stamp(l.at)} ${l.text}`).join("\n")));
  }

  if (ctx.missing?.length) {
    out.push("", "## Not included", "", ...ctx.missing.map((m) => `- ${m}`));
  }

  out.push(
    "",
    reach ? "## home-cc's code, if the cause turns out to be there" : "## Where to start looking",
    "",
    ...AREA_FILES[problem.area].map((f) => `- ${f}`),
  );
  out.push(
    "",
    "No passwords or other secrets are included. Ask me if you need something that isn't here.",
  );
  return out.join("\n") + "\n";
}
