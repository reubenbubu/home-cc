// Typed wrappers around the Rust commands in src-tauri/src/commands.rs.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type DeviceKind = "pc" | "server" | "vm" | "container" | "mesh" | "other" | "ignore";
export type Os = "windows" | "linux" | "proxmox" | "other";

export interface Device {
  id: number;
  name: string;
  host: string;
  kind: DeviceKind;
  os: Os;
  sshUser: string | null;
  sshPort: number;
  notes: string;
  /** Hardware address, lower case with colons, when known. */
  mac: string | null;
  /** Maker, worked out from the hardware address. */
  vendor: string | null;
  /** Name the device gave itself on the network, if one was found. */
  hostname: string | null;
  /** Who added it: the owner by hand, or a network scan. */
  source: "manual" | "scan";
  /** False for a device a scan found that the owner hasn't looked at yet. Saving it makes it true. */
  reviewed: boolean;
  /** Unix seconds. */
  firstSeen: number;
  /** Unix seconds of the last answer to a scan or ping. */
  lastSeen: number | null;
}

export interface DeviceInput {
  name: string;
  host: string;
  kind: DeviceKind;
  os: Os;
  sshUser?: string | null;
  sshPort?: number | null;
  notes?: string;
  /** Not given (or null): left as it is. An empty string clears it. */
  mac?: string | null;
}

export interface Settings {
  /** Seconds between pings of every known device (5 to 3600). */
  pingIntervalSecs: number;
  /** Seconds between stats readings from each machine (10 to 3600). */
  statsIntervalSecs: number;
  /** IP range to scan, such as "192.168.68.0/22". Empty means work it out. */
  scanRange: string;
  /** A disk fuller than this (percent) is shown as a warning. */
  diskWarnPct: number;
  /** RAM use above this (percent) is shown as a warning. */
  ramWarnPct: number;
  /** Private key file for SSH. Empty means the usual ones in the .ssh folder. */
  sshKeyPath: string;
}

/** A local network the app could scan. */
export interface Subnet {
  cidr: string;
  /** Name of the network adapter. */
  interface: string;
  ownIp: string;
  /** True for the one the app would pick by itself. */
  best: boolean;
}

/** Sent as the "scan-progress" event while a scan runs. */
export interface ScanProgress {
  /** sweep: pinging every address. confirm: second look at slow or silent devices. names: looking up names. */
  phase: "sweep" | "confirm" | "names";
  done: number;
  total: number;
  /** Devices found so far. */
  found: number;
}

/** One device that answered a scan. */
export interface Found {
  ip: string;
  mac: string | null;
  vendor: string | null;
  hostname: string | null;
  looksLikeMesh: boolean;
  /** Ping time in ms. Devices that are there but don't answer ping (many Windows PCs) have none. */
  rttMs: number | null;
  /** True for the PC home-cc is running on. */
  isSelf: boolean;
}

export interface ScanResult {
  /** The range that was scanned. */
  range: string;
  found: Found[];
  /** New devices added to the inventory, and known ones refreshed. */
  added: number;
  updated: number;
  /** True if the scan was stopped early. */
  cancelled: boolean;
}

export interface Sample {
  /** Unix seconds. */
  ts: number;
  up: boolean;
  rttMs: number | null;
}

/** Latest check of one device, plus recent history (oldest first, at most 120). */
export interface DeviceStatus {
  deviceId: number;
  up: boolean;
  /** None when the device is up but doesn't answer ping. */
  rttMs: number | null;
  /** Unix seconds of the latest check. */
  checkedAt: number;
  /** Unix seconds of the last time it was there. */
  lastSeen: number | null;
  /** True when this device is the PC home-cc runs on (it is always up and has no response time). */
  isSelf: boolean;
  history: Sample[];
}

export interface Disk {
  /** "/" or "/home" on Linux, "C:" on Windows. */
  mount: string;
  /** Device name on Linux, volume label on Windows. May be empty. */
  label: string;
  totalBytes: number;
  usedBytes: number;
}

export interface MachineStats {
  osName: string;
  uptimeSecs: number | null;
  /** 0 to 100. The first reading from a Linux machine has none. */
  cpuPct: number | null;
  /** One-minute load average (Linux only). */
  load1: number | null;
  cpuCount: number | null;
  memTotalKb: number | null;
  memUsedKb: number | null;
  disks: Disk[];
  /** True when a Windows account wasn't allowed the full query; some values may be missing. */
  reduced: boolean;
}

/** A machine's SSH identity: what `ssh-keygen -lf` prints on that machine. */
export interface HostKey {
  /** For example "ssh-ed25519". */
  algorithm: string;
  /** For example "SHA256:…". */
  fingerprint: string;
}

/** Why a stats reading failed.
 *  - newHost: first connection, and getting in needs the stored password. The password is NOT sent
 *    until the owner approves the identity in offeredKey (trustHostKey).
 *  - hostKeyChanged: the machine's identity is not the one stored (expectedFingerprint); offeredKey is
 *    what it presents now. Nothing is sent to it until the owner approves (trustHostKey).
 *  - auth: no key or password was accepted.
 *  - unreachable: could not connect at all. */
export type StatsErrorKind = "newHost" | "hostKeyChanged" | "auth" | "unreachable" | "other";

/** Stats for one machine that has an SSH user. Either stats or error is set once it has been tried. */
export interface MachineStatsEntry {
  deviceId: number;
  stats: MachineStats | null;
  /** Unix seconds of the reading in stats. */
  collectedAt: number | null;
  /** Readable message from the latest try, if it failed. Older stats are kept alongside. */
  error: string | null;
  errorKind: StatsErrorKind | null;
  /** With newHost and hostKeyChanged: the identity the machine presented, for the owner to check. */
  offeredKey: HostKey | null;
  /** With hostKeyChanged: the fingerprint that was stored. */
  expectedFingerprint: string | null;
  /** The identity currently stored for this machine, if any. Show it somewhere unobtrusive so the
   *  owner can compare it with the machine's own. */
  pinnedFingerprint: string | null;
  /** True after a login failure (auth): the app has stopped retrying by itself, so it doesn't keep
   *  knocking with a wrong password. It tries again when the machine or a secret is edited, or on
   *  refreshStats. */
  paused: boolean;
}

/** Keychain entry names used with setSecret / hasSecret / deleteSecret. */
export const secretKeys = {
  /** Password for one machine. Removed with the machine. */
  sshPassword: (deviceId: number) => `ssh-password:${deviceId}`,
  /** Passphrase for the SSH private key. */
  sshKeyPassphrase: "ssh-key-passphrase",
  /** Proxmox: the API token's secret for one Proxmox host. Removed with the device. */
  proxmoxToken: (deviceId: number) => `proxmox-token:${deviceId}`,
};

export interface AppInfo {
  version: string;
  os: string;
  /** Processor family, such as "x86_64". */
  arch: string;
  /** False for a development build. */
  release: boolean;
}

/** Something the back end ran into by itself, in a timed round. */
export interface LogLine {
  /** Unix seconds. */
  at: number;
  text: string;
}

export const KIND_LABELS: Record<DeviceKind, string> = {
  pc: "PC",
  server: "Server",
  vm: "VM",
  container: "Container",
  mesh: "Mesh node",
  other: "Other",
  ignore: "Ignore",
};

export const OS_LABELS: Record<Os, string> = {
  windows: "Windows",
  linux: "Linux",
  proxmox: "Proxmox VE",
  other: "Other",
};

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  /** Problems the back end ran into by itself, oldest first (the last 50), for a problem report. */
  recentLog: () => invoke<LogLine[]>("recent_log"),
  listDevices: () => invoke<Device[]>("list_devices"),
  addDevice: (input: DeviceInput) => invoke<Device>("add_device", { input }),
  updateDevice: (id: number, input: DeviceInput) => invoke<Device>("update_device", { id, input }),
  removeDevice: (id: number) => invoke<void>("remove_device", { id }),
  setSecret: (key: string, value: string) => invoke<void>("set_secret", { key, value }),
  hasSecret: (key: string) => invoke<boolean>("has_secret", { key }),
  deleteSecret: (key: string) => invoke<void>("delete_secret", { key }),

  getSettings: () => invoke<Settings>("get_settings"),
  /** Returns the settings as saved. Rejects with a readable message if a value is out of range. */
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),

  /** Local networks worth scanning, best first. One entry per adapter, so the same cidr can appear
   *  twice (cable and Wi-Fi on one network). Can change while the app runs; reload it when the
   *  Settings screen opens and after a scan. */
  listSubnets: () => invoke<Subnet[]>("list_subnets"),
  /** Scans the range from settings (or the best local network), saves what it finds into the
   *  inventory and resolves when done; a /22 takes about 25 seconds. "scan-progress" events arrive
   *  meanwhile. Rejects if a scan is already running. */
  startScan: () => invoke<ScanResult>("start_scan"),
  /** Stops a running scan; startScan then resolves with what was found so far. */
  cancelScan: () => invoke<void>("cancel_scan"),
  /** True while a scan is running (for a screen opened mid-scan). */
  scanRunning: () => invoke<boolean>("scan_running"),

  /** Latest check and recent history of every device that has been checked. Ignored devices are not checked. */
  deviceStatuses: () => invoke<DeviceStatus[]>("device_statuses"),
  /** Check every device now instead of waiting for the timer. Resolves when the round is done.
   *  Rejects with a readable message while a scan is running (devices are checked when it ends),
   *  or when this PC can't send pings at all. */
  checkNow: () => invoke<void>("check_now"),
  /** A standing problem that stops status checks from working at all (for example a Linux PC that
   *  doesn't allow programs to send pings), with what to do about it. Null when checks work.
   *  Reload it on every "status-changed" event and show it prominently. */
  checkProblem: () => invoke<string | null>("check_problem"),

  /** One entry per machine that has an SSH user. */
  machineStats: () => invoke<MachineStatsEntry[]>("machine_stats"),
  /** Read one machine's stats now (also ends a pause). Resolves with the new entry (which may hold an error). */
  refreshStats: (deviceId: number) => invoke<MachineStatsEntry>("refresh_stats", { deviceId }),
  /** Approve the identity a machine presented (newHost or hostKeyChanged): pass the fingerprint from
   *  entry.offeredKey that the owner was shown and confirmed. From then on only that identity is
   *  accepted. Rejects if the machine has presented a different one since; reload and ask again.
   *  Follow it with refreshStats. */
  trustHostKey: (deviceId: number, fingerprint: string) => invoke<void>("trust_host_key", { deviceId, fingerprint }),
};

// --- Proxmox (spec 5.6; back end in src-tauri/src/proxmox/) ---

/** Why a Proxmox reading failed.
 *  - noToken: no token ID, or no secret stored for it.
 *  - newCert: first contact. offeredFingerprint is the host's certificate; nothing (not even the token)
 *    was sent. Approve it with proxmoxApi.trustCertificate once the owner has checked it.
 *  - certChanged: the certificate isn't the approved one (expectedFingerprint); nothing was sent.
 *  - auth: the token wasn't accepted. permission: the token lacks a permission.
 *  - unreachable: no connection. */
export type ProxmoxErrorKind = "noToken" | "newCert" | "certChanged" | "auth" | "permission" | "unreachable" | "other";

export type GuestKind = "qemu" | "lxc";

export interface ProxmoxStorage {
  name: string;
  /** Proxmox's type name, such as "dir", "lvmthin", "zfspool", "cifs". */
  kind: string | null;
  /** What it holds, such as "images", "rootdir", "iso", "backup". */
  content: string[];
  shared: boolean;
  /** False when Proxmox can't reach it or it is switched off. */
  active: boolean;
  /** Bytes. */
  used: number | null;
  total: number | null;
}

export interface ProxmoxNode {
  name: string;
  /** "online", "offline" or "unknown". */
  status: string;
  /** 0 to 100. */
  cpuPct: number | null;
  cpuCount: number | null;
  /** Bytes. */
  memUsed: number | null;
  memTotal: number | null;
  /** The node's own system disk, in bytes. */
  diskUsed: number | null;
  diskTotal: number | null;
  uptimeSecs: number | null;
  storages: ProxmoxStorage[];
}

/** A VM (qemu) or container (lxc). */
export interface ProxmoxGuest {
  vmid: number;
  kind: GuestKind;
  /** Empty if it has none. */
  name: string;
  node: string;
  /** "running", "stopped" or "paused" (Proxmox may give others, such as "suspended" or "io-error"). */
  status: string;
  /** A template can't be started. */
  template: boolean;
  /** Set while Proxmox is busy with it, such as "backup" or "snapshot". */
  lock: string | null;
  /** 0 to 100, of the processors given to it. */
  cpuPct: number | null;
  cpuCount: number | null;
  /** Bytes. */
  memUsed: number | null;
  memTotal: number | null;
  /** Containers only; Proxmox doesn't know what a VM's disk holds. */
  diskUsed: number | null;
  diskTotal: number | null;
  uptimeSecs: number | null;
  tags: string[];
}

export interface ProxmoxOverview {
  nodes: ProxmoxNode[];
  guests: ProxmoxGuest[];
}

/** One Proxmox host. Either overview or error is set. */
export interface ProxmoxStatus {
  deviceId: number;
  overview: ProxmoxOverview | null;
  /** Unix seconds of the reading in overview. */
  fetchedAt: number | null;
  error: string | null;
  errorKind: ProxmoxErrorKind | null;
  /** With newCert and certChanged: SHA-256 of the certificate the host showed, "AB:CD:…". */
  offeredFingerprint: string | null;
  /** With certChanged: the approved one. */
  expectedFingerprint: string | null;
  /** The approved certificate, if any. */
  pinnedFingerprint: string | null;
}

export interface ProxmoxSettings {
  deviceId: number;
  /** Such as "root@pam!homecc". */
  tokenId: string | null;
  /** The API port, normally 8006. */
  port: number;
  pinnedFingerprint: string | null;
  /** Whether the token's secret is in the keychain (it is never sent back). */
  hasSecret: boolean;
}

/** Which VM or container an action is for. */
export interface GuestRef {
  node: string;
  kind: GuestKind;
  vmid: number;
}

/** start (also resumes a hibernated VM), resume (a paused one), shutdown (asks the guest's system), reboot,
 *  stop (pulls the plug). Shut down, restart and force stop are confirmed first. */
export type PowerAction = "start" | "resume" | "shutdown" | "reboot" | "stop";

export interface ProxmoxSnapshot {
  name: string;
  description: string;
  /** Unix seconds. */
  takenAt: number | null;
  parent: string | null;
  /** The VM's memory was saved too, so a rollback returns it running. */
  withRam: boolean;
}

export interface SnapshotList {
  /** Oldest first. */
  snapshots: ProxmoxSnapshot[];
  /** The snapshot the guest's present state comes from, if any. */
  currentParent: string | null;
}

/** finished is false if Proxmox was still at it when home-cc stopped waiting; it carries on there. */
export interface TaskOutcome {
  finished: boolean;
}

export const proxmoxApi = {
  /** Token ID, port and approved certificate of one Proxmox host. The secret goes through
   *  api.setSecret(secretKeys.proxmoxToken(id), …). */
  settings: (deviceId: number) => invoke<ProxmoxSettings>("proxmox_settings", { deviceId }),
  /** An empty tokenId removes it. Rejects with a readable message for a token ID that can't be right. */
  saveSettings: (deviceId: number, tokenId: string, port: number) =>
    invoke<ProxmoxSettings>("proxmox_save_settings", { deviceId, tokenId, port }),
  /** Every node, VM, container and storage. A failed reading comes back inside the answer. */
  status: (deviceId: number) => invoke<ProxmoxStatus>("proxmox_status", { deviceId }),
  /** Approve the certificate in status.offeredFingerprint, once the owner has checked it. Rejects if the
   *  host has shown a different one since; read the status again and ask again. */
  trustCertificate: (deviceId: number, fingerprint: string) =>
    invoke<void>("proxmox_trust_certificate", { deviceId, fingerprint }),
  power: (deviceId: number, guest: GuestRef, action: PowerAction) =>
    invoke<TaskOutcome>("proxmox_power", { deviceId, guest, action }),
  snapshots: (deviceId: number, guest: GuestRef) => invoke<SnapshotList>("proxmox_snapshots", { deviceId, guest }),
  /** withRam only applies to VMs. */
  takeSnapshot: (deviceId: number, guest: GuestRef, name: string, description: string, withRam: boolean) =>
    invoke<TaskOutcome>("proxmox_take_snapshot", { deviceId, guest, name, description, withRam }),
  /** Proxmox stops a running guest to roll it back to a snapshot without RAM, and leaves it stopped;
   *  startAfter starts it again once the rollback is done. */
  rollback: (deviceId: number, guest: GuestRef, name: string, startAfter: boolean) =>
    invoke<TaskOutcome>("proxmox_rollback", { deviceId, guest, name, startAfter }),
  deleteSnapshot: (deviceId: number, guest: GuestRef, name: string) =>
    invoke<TaskOutcome>("proxmox_delete_snapshot", { deviceId, guest, name }),
};

// --- end Proxmox ---

/** Events from the back end. Each returns a function that stops listening. */
export const events = {
  onScanProgress: (fn: (p: ScanProgress) => void): Promise<UnlistenFn> =>
    listen<ScanProgress>("scan-progress", (e) => fn(e.payload)),
  /** A round of checks finished, or the inventory changed (for example after a scan): reload devices and statuses. */
  onStatusChanged: (fn: () => void): Promise<UnlistenFn> => listen("status-changed", () => fn()),
  /** New machine stats are ready: reload them. */
  onStatsChanged: (fn: () => void): Promise<UnlistenFn> => listen("stats-changed", () => fn()),
};

/** Errors from Rust arrive as plain strings. */
export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}
