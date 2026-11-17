// Problems shown this session, and the context gathered for a prompt to an
// agent (see report.ts). Screens show problems with components/Problem.svelte,
// which records them here.
import { page } from "$app/state";
import { api, errorText, KIND_LABELS, OS_LABELS, type Device } from "./api";
import { names } from "./names.svelte";
import { agentPrompt, type Context, type Details, type Problem } from "./report";
import { settings } from "./settings.svelte";
import { status } from "./status.svelte";

/** Git commit the screens were built from (set in vite.config.js), or "" if not known. */
declare const __HOME_CC_COMMIT__: string;
const COMMIT = typeof __HOME_CC_COMMIT__ === "string" ? __HOME_CC_COMMIT__ : "";

const KEEP = 30;

interface Seen {
  at: number;
  what: string;
  error: string;
}

const sameProblem = (a: { what: string; error: string }, b: { what: string; error: string }) =>
  a.what === b.what && a.error === b.error;

class Problems {
  /** Oldest first. A problem shown again moves to the end instead of being listed twice. */
  #seen: Seen[] = [];

  record(p: Problem) {
    const at = Math.floor(Date.now() / 1000);
    this.#seen = this.#seen.filter((s) => !sameProblem(s, p));
    this.#seen.push({ at, what: p.what, error: p.error });
    if (this.#seen.length > KEEP) this.#seen.shift();
  }

  /** Every recorded problem but this one, oldest first. */
  others(p: Problem): Seen[] {
    return this.#seen.filter((s) => !sameProblem(s, p));
  }
}

export const problems = new Problems();

/** What helps tell one device's problem from another's. Notes are left out: they are the owner's own words. */
export function deviceDetails(d: Device): Details {
  return {
    Device: d.name,
    Address: d.host,
    Type: KIND_LABELS[d.kind],
    "OS set in home-cc": OS_LABELS[d.os],
    "SSH user": d.sshUser,
    "SSH port": d.sshUser ? d.sshPort : null,
    "Resolved hostname": names.describe(d.host, d.hostname),
    Maker: d.vendor,
  };
}

/** The settings, minus anything that isn't useful for finding a problem. None of them are secret. */
function settingsDetails(): Details {
  const s = settings.value;
  return {
    "Seconds between pings": s.pingIntervalSecs,
    "Seconds between stats readings": s.statsIntervalSecs,
    "Range to scan": s.scanRange || "worked out by the app",
    "SSH key file": s.sshKeyPath || "the usual ones in the .ssh folder",
  };
}

/** Gathers the context and writes the prompt. Never rejects: a part that can't be read is named instead. */
export async function buildPrompt(p: Problem, extra?: () => Promise<Details>): Promise<string> {
  const missing: string[] = [];
  const [info, log, more] = await Promise.allSettled([
    api.appInfo(),
    api.recentLog(),
    extra ? extra() : Promise.resolve({} as Details),
  ]);
  if (info.status === "rejected") missing.push(`The app's version and system: ${errorText(info.reason)}`);
  if (log.status === "rejected") missing.push(`The back-end log: ${errorText(log.reason)}`);
  if (more.status === "rejected") missing.push(`Some details: ${errorText(more.reason)}`);
  if (settings.error) missing.push(`The settings below may not be the saved ones: ${settings.error}`);

  let screen: string | undefined;
  try {
    screen = page.url.pathname;
  } catch {
    // Not inside the app's router (as in a test); the screen is then simply left out.
  }

  const ctx: Context = {
    ...(info.status === "fulfilled" ? info.value : {}),
    commit: COMMIT || undefined,
    screen,
    now: Math.floor(Date.now() / 1000),
    checkProblem: p.area === "status" && p.error === status.problem ? null : status.problem,
    settings: settings.loaded ? settingsDetails() : undefined,
    recent: problems.others(p),
    backendLog: log.status === "fulfilled" ? log.value : undefined,
    missing,
  };
  const details = { ...p.details, ...(more.status === "fulfilled" ? more.value : {}) };
  return agentPrompt({ ...p, details }, ctx);
}

/** Puts text on the clipboard. Resolves false if neither way worked, so it can be shown for copying by hand. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    // Older way, for a webview that doesn't offer the clipboard to this page.
    const area = document.createElement("textarea");
    area.value = text;
    area.setAttribute("readonly", "");
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    try {
      return document.execCommand("copy");
    } catch {
      return false;
    } finally {
      area.remove();
    }
  }
}
