// Typed wrappers around the Rust commands in src-tauri/src/commands.rs.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** "appliance": something with its own address that isn't a PC, such as a printer, TV or camera.
 *  "handheld": a phone, tablet or handheld games console; not a machine, only checked that it is there. */
export type DeviceKind = "pc" | "server" | "vm" | "container" | "mesh" | "appliance" | "handheld" | "other" | "ignore";
export type Os = "windows" | "linux" | "proxmox" | "other";
/** Where a System came from, strongest first: the owner's choice, this PC itself, an SSH login, a working
 *  Proxmox API, a guess from the ports that answer on the network. */
export type OsSource = "manual" | "this-pc" | "ssh" | "proxmox" | "network";

export interface Device {
  id: number;
  name: string;
  host: string;
  kind: DeviceKind;
  os: Os;
  /** Where `os` came from; null while it isn't known yet (`os` is then "other"). */
  osSource: OsSource | null;
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
  /** The group the owner put it in on the Machines screen, or none. A scan never changes it. */
  group: string | null;
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
  /** Group name. Not given (or null): left as it is. An empty string takes the machine out of its group. */
  group?: string | null;
  /** The System the form showed when it opened. The same as `os`: the owner didn't change it, and the stored
   *  System stays (home-cc may have found it since). Not given: `os` is compared with the stored one. */
  osWas?: Os | null;
  /** "Let home-cc work it out": the System is forgotten, with the owner's choice of it, and found again. */
  osAuto?: boolean;
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
  /** The SSH user a machine added by hand starts with (only fills in the form). Empty means none. */
  defaultSshUser: string;
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
  /** True for this PC (the one home-cc runs on): read here, without SSH and without an SSH user. None of
   *  the SSH states (newHost, hostKeyChanged, auth, paused) ever apply to it. */
  local: boolean;
}

// --- Terminals (src-tauri/src/terminal.rs) ---
// The fixed interface shared with the Docker and Proxmox screens. To open a tab from another screen, use
// openTerminal in terminals.svelte.ts rather than these.

/** The "terminal-output" event: bytes the machine printed, base64-encoded (decode with terminal.ts). */
export interface TerminalOutput {
  id: number;
  data: string;
}

/** The "terminal-exit" event. The terminal is gone. */
export interface TerminalExit {
  id: number;
  /** Why, for the owner. Null for a normal end: `exit`, a program that finished fine, or a closed tab. */
  reason: string | null;
  /** The program's exit code, when the machine sent one. */
  exitStatus: number | null;
}

export const terminalApi = {
  /** Connects and starts the login shell, or `command` in its place (for example "docker exec -it web sh"),
   *  on a terminal of this size. Resolves with the terminal's id once it runs; its first output may arrive
   *  before that. Rejects with a readable message, also when the machine's identity needs approving first. */
  open: (deviceId: number, command: string | null, cols: number, rows: number) =>
    invoke<number>("terminal_open", { deviceId, command, cols, rows }),
  /** What was typed or pasted. Rejects once the terminal has closed. */
  write: (id: number, data: string) => invoke<void>("terminal_write", { id, data }),
  /** Bytes the terminal itself reports, as xterm.js's onBinary gives them: each character is one byte (0 to
   *  255). Mouse clicks in the oldest mouse format, which vim uses with xterm.js, come this way. */
  writeBinary: (id: number, data: string) => invoke<void>("terminal_write_binary", { id, data }),
  resize: (id: number, cols: number, rows: number) => invoke<void>("terminal_resize", { id, cols, rows }),
  /** The screen has drawn this many more bytes of output. The back end stops reading while too much waits. */
  ack: (id: number, bytes: number) => invoke<void>("terminal_ack", { id, bytes }),
  /** Closes the terminal and its connection. Closing one that already closed is fine. */
  close: (id: number) => invoke<void>("terminal_close", { id }),
  /** Closes every terminal, for example ones left over from before the window was reloaded. */
  closeAll: () => invoke<void>("terminal_close_all"),
  /** The text on the clipboard ("" if none). The back end reads it: the web view may not be allowed to. */
  clipboardRead: () => invoke<string>("clipboard_read_text"),
  clipboardWrite: (text: string) => invoke<void>("clipboard_write_text", { text }),
};

export const terminalEvents = {
  onOutput: (fn: (e: TerminalOutput) => void): Promise<UnlistenFn> =>
    listen<TerminalOutput>("terminal-output", (e) => fn(e.payload)),
  onExit: (fn: (e: TerminalExit) => void): Promise<UnlistenFn> => listen<TerminalExit>("terminal-exit", (e) => fn(e.payload)),
};
// --- End of terminals ---

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

/** The name an address resolves to (see resolveNames). */
export interface ResolvedName {
  /** As it was asked for. */
  address: string;
  /** Tidied for showing ("nas.lan" gives "nas"). Null when none was found, and for anything that isn't an
   *  IPv4 address. */
  name: string | null;
  /** No answer yet for this IPv4 address: its lookup didn't get its turn, most likely because a scan was
   *  busy. Not "no name": ask again soon. */
  retrySoon?: boolean;
}

/** Which SSH key this PC logs in with (localSshKey). */
export interface LocalKey {
  /** Full path of the private key file. Null when this PC has no key. */
  path: string | null;
  /** What to call it: the file name, or the path chosen in Settings. */
  name: string | null;
  /** True if the key file is the one chosen in Settings. */
  fromSettings: boolean;
  /** The line that goes in authorized_keys on a machine. Null if it couldn't be read. */
  publicKey: string | null;
  /** For example "ssh-ed25519". */
  algorithm: string | null;
  /** "SHA256:…", as ssh-keygen -lf prints it. */
  fingerprint: string | null;
  /** Why the public key couldn't be read, or why the key file chosen in Settings is missing. */
  problem: string | null;
  /** Worth knowing but not a problem, such as a .pub file that didn't match its key. */
  note: string | null;
  /** Other key files in the .ssh folder that logins offer as well. */
  others: string[];
  /** Where a new key would be written. Only set when this PC has no key and one can be made. */
  newKeyPath: string | null;
}

/** What came of setupKeyLogin. A failure is a rejection with a readable message.
 *  - approve: nothing was sent. Getting in needs the password and the machine's identity isn't approved yet;
 *    show offeredKey (and expectedFingerprint for a changed identity), approve it with
 *    approveKeySetupIdentity, then call setupKeyLogin again.
 *  - done: the key is on the machine (added now, or there already) and a login with keys only was tried. */
export type KeySetupOutcome =
  | { outcome: "approve"; offeredKey: HostKey; expectedFingerprint: string | null }
  | {
      outcome: "done";
      /** The file on the machine that holds the key. */
      file: string;
      alreadyThere: boolean;
      /** True if home-cc then logged in without a password. */
      keyLoginWorks: boolean;
      windows: boolean;
      /** What was done and how the check went, for the owner. */
      message: string;
      /** With keyLoginWorks false: what to check on the machine. */
      advice: string | null;
    };

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
  appliance: "Appliance",
  handheld: "Handheld",
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
  /** Puts one machine in a group ("" for none) and changes nothing else about it. */
  setDeviceGroup: (id: number, group: string) => invoke<Device>("set_device_group", { id, group }),
  /** Renames a group on every device in it; an empty `to` takes them all out of it. Gives how many changed. */
  renameGroup: (from: string, to: string) => invoke<number>("rename_group", { from, to }),
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

  /** The name each address resolves to (reverse DNS, mDNS and NetBIOS), one answer per address in the same
   *  order. At most 256 addresses at a time. Only IPv4 addresses are looked up. Takes up to about 2.5 seconds;
   *  answers are remembered (a name for 10 minutes, "no name" for 2), so asking again is cheap. Screens use
   *  the store in names.svelte.ts, through components/Address.svelte, rather than calling this directly. */
  resolveNames: (addresses: string[]) => invoke<ResolvedName[]>("resolve_names", { addresses }),

  /** One entry per machine that has an SSH user. */
  machineStats: () => invoke<MachineStatsEntry[]>("machine_stats"),
  /** Read one machine's stats now (also ends a pause). Resolves with the new entry (which may hold an error). */
  refreshStats: (deviceId: number) => invoke<MachineStatsEntry>("refresh_stats", { deviceId }),
  /** Approve the identity a machine presented (newHost or hostKeyChanged): pass the fingerprint from
   *  entry.offeredKey that the owner was shown and confirmed. From then on only that identity is
   *  accepted. Rejects if the machine has presented a different one since; reload and ask again.
   *  Follow it with refreshStats. */
  trustHostKey: (deviceId: number, fingerprint: string) => invoke<void>("trust_host_key", { deviceId, fingerprint }),

  /** Which key this PC logs in with (the one chosen in Settings, or the usual ones in .ssh), with its public half. */
  localSshKey: () => invoke<LocalKey>("local_ssh_key"),
  /** Make an ed25519 key in the .ssh folder, for a PC that has none (never overwrites a file). A passphrase, if
   *  given, locks it and is stored in the keychain as secretKeys.sshKeyPassphrase. Resolves with the new key. */
  createSshKey: (passphrase: string | null) => invoke<LocalKey>("create_ssh_key", { passphrase }),
  /** Add this PC's public key to the machine, logging in once (with the password if no key gets in), then try a
   *  login with keys only. `password` is the one typed in the form, used for this once and not stored; without
   *  it the stored one is used. A password is only sent to an approved identity: see KeySetupOutcome. */
  setupKeyLogin: (deviceId: number, password: string | null) =>
    invoke<KeySetupOutcome>("setup_key_login", { deviceId, password }),
  /** Approve the identity setupKeyLogin showed (outcome "approve"). Rejects if the machine has shown another one
   *  since, or was pointed elsewhere; then set up again and check the new one. */
  approveKeySetupIdentity: (deviceId: number, fingerprint: string) =>
    invoke<void>("approve_key_setup_identity", { deviceId, fingerprint }),
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

/** A running guest's addresses as Proxmox knows them: a VM's from its QEMU guest agent, a container's
 *  from Proxmox itself. */
export interface GuestAddresses {
  /** IPv4, without loopback and link-local ones. */
  addresses: string[];
  /** Why Proxmox can't tell, when that is normal (no guest agent running in the VM). */
  unknownWhy: string | null;
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
  /** A running guest's IPv4 addresses. Rejects for a real failure, not for a VM without the guest agent. */
  guestAddresses: (deviceId: number, guest: GuestRef) =>
    invoke<GuestAddresses>("proxmox_guest_addresses", { deviceId, guest }),
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
