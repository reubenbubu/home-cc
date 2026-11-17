// Typed wrappers around the Pi-hole commands (src-tauri/src/pihole/commands.rs).
// Kept apart from ../api.ts so the Pi-hole module stays in its own files.
import { invoke } from "@tauri-apps/api/core";

/** Why a Pi-hole reading failed.
 *  - noPassword: none stored. newCert / certChanged: the certificate needs the owner's approval; nothing was sent.
 *  - auth: the password wasn't accepted. tooManySessions: every API session the Pi-hole allows is in use.
 *  - rateLimited: Pi-hole holds back logins after too many tries. v5: an older Pi-hole, not supported yet.
 *  - unreachable: no answer. */
export type PiholeErrorKind =
  | "noPassword"
  | "newCert"
  | "certChanged"
  | "auth"
  | "tooManySessions"
  | "rateLimited"
  | "v5"
  | "unreachable"
  | "other";

export interface PiholeSummary {
  /** Queries in the last 24 hours. */
  queries: number;
  blocked: number;
  percentBlocked: number;
  clientsActive: number | null;
  clientsTotal: number | null;
  /** Domains on the blocklists ("gravity"). */
  domainsOnLists: number | null;
  /** Unix seconds of the last blocklist update. */
  gravityUpdated: number | null;
}

export type BlockingState = "enabled" | "disabled" | "failed" | "unknown";

export interface Blocking {
  state: BlockingState;
  /** Seconds until Pi-hole switches back by itself; null when it stays until changed. */
  timerSecs: number | null;
}

export type ListKind = "block" | "allow";

export interface AdList {
  address: string;
  kind: ListKind;
  enabled: boolean;
  comment: string | null;
  domains: number | null;
  /** Pi-hole's number for the list's state: 1 updated, 2 unchanged, 3 download failed (old copy used), 4 failed with no copy. */
  status: number | null;
}

export interface PiholeVersion {
  core: string | null;
  web: string | null;
  ftl: string | null;
  docker: string | null;
}

export interface PiholeReading {
  summary: PiholeSummary;
  blocking: Blocking;
  lists: AdList[] | null;
  listsError: string | null;
  version: PiholeVersion | null;
}

/** One Pi-hole. Either reading or error is set. */
export interface PiholeStatus {
  deviceId: number;
  reading: PiholeReading | null;
  fetchedAt: number | null;
  error: string | null;
  errorKind: PiholeErrorKind | null;
  offeredFingerprint: string | null;
  expectedFingerprint: string | null;
  pinnedFingerprint: string | null;
  https: boolean;
  port: number;
}

export interface PiholeSettings {
  deviceId: number;
  /** False: plain HTTP, chosen by the owner. */
  https: boolean;
  port: number;
  pinnedFingerprint: string | null;
  hasPassword: boolean;
}

export interface Probe {
  deviceId: number;
  /** "v6", "v5", or null when nothing like Pi-hole answered. */
  found: "v6" | "v5" | null;
}

export const piholeApi = {
  /** Every device marked as a Pi-hole. */
  hosts: () => invoke<PiholeSettings[]>("pihole_hosts"),
  /** Marks a device as a Pi-hole (HTTPS on 443 to begin with). Ask the owner first. */
  add: (deviceId: number) => invoke<PiholeSettings>("pihole_add", { deviceId }),
  /** Logs out and forgets the Pi-hole's settings and password. Ask the owner first. */
  remove: (deviceId: number) => invoke<void>("pihole_remove", { deviceId }),
  /** The password, when given, goes to the keychain and is never sent back. */
  saveSettings: (deviceId: number, https: boolean, port: number, password: string | null) =>
    invoke<PiholeSettings>("pihole_save_settings", { deviceId, https, port, password }),
  removePassword: (deviceId: number) => invoke<PiholeSettings>("pihole_remove_password", { deviceId }),
  status: (deviceId: number) => invoke<PiholeStatus>("pihole_status", { deviceId }),
  trustCertificate: (deviceId: number, fingerprint: string) => invoke<void>("pihole_trust_certificate", { deviceId, fingerprint }),
  /** On, or off for timerSecs seconds (null: until switched on). Ask the owner before switching it off. */
  setBlocking: (deviceId: number, blocking: boolean, timerSecs: number | null) =>
    invoke<Blocking>("pihole_set_blocking", { deviceId, blocking, timerSecs }),
  /** Which of these devices answer like a Pi-hole (HTTP on port 80; nothing secret is sent). */
  probe: (deviceIds: number[]) => invoke<Probe[]>("pihole_probe", { deviceIds }),
};
