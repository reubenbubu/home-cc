// Words and rules for the Proxmox screen. Kept apart from the store and the
// components so they can be tested without a browser (proxmox.test.mjs).
import type { GuestRef, PowerAction, ProxmoxGuest } from "./api";

/** What the owner is told to run on the Proxmox host to read its certificate's fingerprint. */
export const CERT_COMMAND = "openssl x509 -noout -fingerprint -sha256 -in /etc/pve/local/pve-ssl.pem";
/** The same for a certificate the owner uploaded or got from Let's Encrypt, which Proxmox then uses instead. */
export const CUSTOM_CERT_COMMAND = "openssl x509 -noout -fingerprint -sha256 -in /etc/pve/local/pveproxy-ssl.pem";

export const DEFAULT_TOKEN_ID = "root@pam!homecc";

export function guestRef(g: ProxmoxGuest): GuestRef {
  return { node: g.node, kind: g.kind, vmid: g.vmid };
}

/** A stable key for one guest, for maps and {#each}. */
export function guestKey(g: GuestRef): string {
  return `${g.node}/${g.kind}/${g.vmid}`;
}

export function kindLabel(g: { kind: GuestRef["kind"] }): string {
  return g.kind === "qemu" ? "VM" : "Container";
}

/** "VM 110" or "Container 101". */
export function guestId(g: { kind: GuestRef["kind"]; vmid: number }): string {
  return `${kindLabel(g)} ${g.vmid}`;
}

/** Its name, or "VM 110" when it has none. */
export function guestName(g: ProxmoxGuest): string {
  return g.name.trim() || guestId(g);
}

/** "Running", "Stopped", "Paused", "Template". */
export function stateLabel(g: ProxmoxGuest): string {
  if (g.template) return "Template";
  const s = g.status.trim();
  return s ? s[0].toUpperCase() + s.slice(1) : "Unknown";
}

/** What Proxmox is busy with, in words, such as "Backup running". Null when it isn't. */
export function lockLabel(lock: string | null): string | null {
  if (!lock) return null;
  const words: Record<string, string> = {
    backup: "Backup running",
    snapshot: "Snapshot being taken",
    "snapshot-delete": "Snapshot being deleted",
    rollback: "Rolling back",
    migrate: "Moving to another node",
    clone: "Being copied",
    create: "Being created",
    disk: "Disk being changed",
    suspending: "Hibernating",
    suspended: "Hibernated",
  };
  return words[lock] ?? `Busy (${lock})`;
}

/** Hibernated (Hibernate in Proxmox, or "qm suspend --todisk"): stopped, with its memory saved to disk. Proxmox
 *  locks it as "suspended", and starting it is how it is resumed. */
export function isHibernated(g: ProxmoxGuest): boolean {
  return g.lock === "suspended";
}

/** Paused, or put to sleep by its own system: halted where it was, but still in memory. Resume carries on. */
export function isPaused(g: ProxmoxGuest): boolean {
  return g.status === "paused" || g.status === "suspended";
}

/** What Proxmox is busy with that holds back `action`, in words, or null when nothing does. A hibernated VM's
 *  lock doesn't hold back Start: starting is what resumes it. */
export function lockBlocks(g: ProxmoxGuest, action: PowerAction): string | null {
  if (!g.lock) return null;
  if (action === "start" && isHibernated(g)) return null;
  return lockLabel(g.lock);
}

/** The power buttons that make sense for a guest as it is, in the order they are shown. */
export function powerActions(g: ProxmoxGuest): PowerAction[] {
  if (g.template) return [];
  switch (g.status) {
    case "running":
      return ["shutdown", "reboot", "stop"];
    // Proxmox won't shut down a paused VM ("VM is paused - cannot shutdown"): it has to be resumed first.
    case "paused":
    case "suspended":
      return ["resume", "stop"];
    case "stopped":
      return ["start"];
    default:
      // Not known: offer what can't do harm without asking, and the forced one behind its confirmation.
      return ["start", "stop"];
  }
}

const LABELS: Record<PowerAction, string> = {
  start: "Start",
  resume: "Resume",
  shutdown: "Shut down",
  reboot: "Restart",
  stop: "Force stop",
};

/** Shown on the guest while Proxmox works on it. */
const BUSY: Record<PowerAction, string> = {
  start: "Starting…",
  resume: "Resuming…",
  shutdown: "Shutting down…",
  reboot: "Restarting…",
  stop: "Stopping…",
};

/** Shown once Proxmox is done. */
const DONE: Record<PowerAction, string> = {
  start: "Started.",
  resume: "Resumed.",
  shutdown: "Shut down.",
  reboot: "Restarted.",
  stop: "Stopped.",
};

/** The second click's label for an action that stops the guest, which asks first (spec section 7). Null for
 *  one that only starts it. */
const CONFIRM: Record<PowerAction, string | null> = {
  start: null,
  resume: null,
  shutdown: "Yes, shut down",
  reboot: "Yes, restart",
  stop: "Yes, force stop",
};

/** The words for one power action on this guest: its button, its second click (null when it needs none), and
 *  what shows while Proxmox works on it and once it is done. Starting a hibernated VM is called resuming it. */
export function actionWords(
  g: ProxmoxGuest,
  action: PowerAction,
): { label: string; confirm: string | null; busy: string; done: string } {
  const as: PowerAction = action === "start" && isHibernated(g) ? "resume" : action;
  return { label: LABELS[as], confirm: CONFIRM[as], busy: BUSY[as], done: DONE[as] };
}

/** What rolling `g` back to `snap` does to whether it runs, and so what the owner is asked and told:
 *  - The snapshot has RAM: Proxmox brings it back running as it was then, even if it is stopped now.
 *  - It is running and the snapshot has no RAM: Proxmox stops it to roll back and leaves it stopped, so
 *    home-cc starts it again afterwards (startAfter). */
export function rollbackPlan(
  g: ProxmoxGuest,
  snap: { name: string; withRam: boolean },
): { startAfter: boolean; confirm: string; done: string } {
  const on = g.status === "running" || isPaused(g);
  if (snap.withRam) {
    return {
      startAfter: false,
      confirm: on ? "Yes, roll back" : "Yes, roll back and start",
      done: `Rolled back to "${snap.name}", running as it was then.`,
    };
  }
  if (on) {
    return { startAfter: true, confirm: "Yes, roll back and restart", done: `Rolled back to "${snap.name}" and started again.` };
  }
  return { startAfter: false, confirm: "Yes, roll back", done: `Rolled back to "${snap.name}".` };
}

/** Why a snapshot name won't do, or null when it is fine. The same rules as Proxmox's. */
export function snapshotNameProblem(name: string): string | null {
  const n = name.trim();
  if (!n) return "Give the snapshot a name.";
  if (!/^[A-Za-z][A-Za-z0-9_-]{1,39}$/.test(n)) {
    return "Start with a letter and use only letters, digits, - and _ (no spaces), 2 to 40 characters.";
  }
  if (n.toLowerCase() === "current") return "\"current\" is the name Proxmox uses for the present state. Pick another.";
  return null;
}

/** A name for a new snapshot, from the time: "snap_20261117_1403". */
export function suggestedSnapshotName(at: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `snap_${at.getFullYear()}${p(at.getMonth() + 1)}${p(at.getDate())}_${p(at.getHours())}${p(at.getMinutes())}`;
}

/** The two commands that make a token for home-cc on the Proxmox host and give it what it needs. */
export function tokenCommands(tokenId: string): { create: string; grant: string } {
  const id = /^[^\s@!=]+@[^\s@!=]+![^\s@!=]+$/.test(tokenId.trim()) ? tokenId.trim() : DEFAULT_TOKEN_ID;
  const [user, name] = id.split("!");
  return {
    create: `pveum user token add ${user} ${name} --privsep 1`,
    grant: `pveum acl modify / --tokens '${id}' --roles PVEAuditor,PVEVMAdmin`,
  };
}
