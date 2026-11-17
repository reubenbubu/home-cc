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
    suspending: "Pausing",
    suspended: "Paused to disk",
  };
  return words[lock] ?? `Busy (${lock})`;
}

/** The power buttons that make sense for a guest as it is, in the order they are shown. */
export function powerActions(g: ProxmoxGuest): PowerAction[] {
  if (g.template) return [];
  switch (g.status) {
    case "running":
      return ["shutdown", "reboot", "stop"];
    case "paused":
      return ["shutdown", "stop"];
    case "stopped":
      return ["start"];
    default:
      // Not known: offer what can't do harm without asking, and the forced one behind its confirmation.
      return ["start", "stop"];
  }
}

export const ACTION_LABELS: Record<PowerAction, string> = {
  start: "Start",
  shutdown: "Shut down",
  reboot: "Restart",
  stop: "Force stop",
};

/** Shown on the button while Proxmox works on it. */
export const ACTION_BUSY: Record<PowerAction, string> = {
  start: "Starting…",
  shutdown: "Shutting down…",
  reboot: "Restarting…",
  stop: "Stopping…",
};

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
