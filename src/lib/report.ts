// Turns a problem shown on a screen into a prompt the owner can paste into a
// coding agent (such as Claude Code, opened in a checkout of home-cc) to find
// out what went wrong and fix it. Pure functions only, so it can be tested
// without the app; components/AgentPrompt.svelte gathers the context.

export const REPO_URL = "https://github.com/reubenbubu/home-cc";

/** Which part of the app a problem came from. Decides where the agent is told to start looking. */
export type Area = "stats" | "trust" | "scan" | "status" | "devices" | "secrets" | "settings" | "live" | "screen";

/** Source files to start from for each area. Paths are from the root of the repository. */
export const AREA_FILES: Record<Area, string[]> = {
  stats: [
    "src-tauri/src/ssh.rs (connecting and logging in)",
    "src-tauri/src/stats.rs and src-tauri/src/stats_*.{sh,ps1} (what is run on the machine and how it is read)",
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
  live: [
    "src/lib/live.svelte.ts (listening for back-end events)",
    "src-tauri/src/monitor.rs (what sends the events)",
  ],
  screen: [
    "the screen named under App, in src/routes/ (its +page.svelte) and the components it uses in src/lib/components/",
    "src/lib/api.ts (the shapes the back end sends)",
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

/** The prompt, as Markdown. */
export function agentPrompt(problem: Problem, ctx: Context): string {
  const out: string[] = [];
  out.push(
    `I use home-cc (${REPO_URL}), a desktop app for managing my home network, built with Tauri 2, Rust and Svelte. ` +
      "It showed the problem below. Please find out why it happened.",
    "",
    "- Start from the source of home-cc. If you are not in a checkout of it, clone the repository first, " +
      "and check out the commit named under App if one is given, so you read the code I am running.",
    "- Decide whether this is a bug or limitation in home-cc, or something to change on my machines or network. " +
      "Say which, and how sure you are.",
    "- If it is a bug, fix it with the smallest change that makes sense, add or update a test, and tell me what changed.",
    "- If it is on my side, give me the exact steps to fix it, saying which machine to run each command on.",
    "- If you can't tell from what is here, tell me what to check or run and what to send back.",
    "",
    "## What happened",
    "",
    `${problem.what}. home-cc showed this message:`,
    "",
    fenced(problem.error.trim()),
  );

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

  out.push("", "## Where to start looking", "", ...AREA_FILES[problem.area].map((f) => `- ${f}`));
  out.push(
    "",
    "No passwords or other secrets are included. Ask me if you need something that isn't here.",
  );
  return out.join("\n") + "\n";
}
