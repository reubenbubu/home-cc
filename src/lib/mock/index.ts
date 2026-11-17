// A stand-in for the Rust back end, with sample data shaped like the owner's
// network, so the screens can be worked on and screenshotted in a browser.
// Only loaded outside Tauri, in dev or in a "mock" build (see routes/+layout.ts).
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  Device,
  DeviceInput,
  DeviceKind,
  DeviceStatus,
  Disk,
  Found,
  HostKey,
  KeySetupOutcome,
  LocalKey,
  MachineStats,
  MachineStatsEntry,
  Os,
  OsSource,
  Sample,
  ScanProgress,
  ScanResult,
  Settings,
  StatsErrorKind,
  Subnet,
} from "../api";
import { foldGroup } from "../arrange";
import { mayReplace, systemAfterSave } from "../system";
import { terminalCommands } from "./terminal";
import { dockerMock } from "./docker";
// Proxmox
import { proxmoxMock } from "./proxmox";
// Pi-hole
import { piholeMock } from "./pihole";

/** How a sample device behaves when it is checked. */
type Pattern = "wired" | "wifi" | "flaky" | "recovered" | "noPing" | "self" | "off" | "gone" | "fresh";

/** How a sample machine lets home-cc log in: with the key file, or only with a password (so not at all
 *  while none is stored for it). */
type Access = "key" | "password";

interface MockDevice extends Device {
  pattern: Pattern;
  access?: Access;
}

const GB = 1024 ** 3;
const HISTORY = 120;
const nowSecs = () => Math.floor(Date.now() / 1000);
const sleep = (msecs: number) => new Promise<void>((r) => setTimeout(r, msecs));

/** Small seeded generator, so the sample data looks the same on every load. */
function seeded(seed: number): () => number {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function sampleFor(pattern: Pattern, i: number, rand: () => number): { up: boolean; rttMs: number | null } {
  const wired = () => ({ up: true, rttMs: 0.25 + rand() * 0.45 });
  const wifi = () => ({ up: true, rttMs: rand() < 0.06 ? 25 + rand() * 45 : 2 + rand() * 9 });
  const down = { up: false, rttMs: null };
  switch (pattern) {
    case "wired":
      return wired();
    case "wifi":
    case "fresh":
      return wifi();
    case "flaky":
      return rand() < 0.06 || (i >= 38 && i < 42) ? down : wifi();
    case "recovered":
      return i >= 61 && i < 70 ? down : wired();
    case "noPing":
    // The PC the app runs on is never checked over the network: it is reported as there, with no time.
    case "self":
      return { up: true, rttMs: null };
    case "off":
      return i >= 84 ? down : wifi();
    case "gone":
      return down;
  }
}

function round2(n: number | null): number | null {
  return n == null ? null : Math.round(n * 100) / 100;
}

const BASE64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/** A made-up SSH identity, the same for the same seed. */
function sampleKey(seed: number, algorithm = "ssh-ed25519"): HostKey {
  const rand = seeded(seed);
  let text = "";
  for (let i = 0; i < 43; i++) text += BASE64[Math.floor(rand() * 64)] ?? "A";
  return { algorithm, fingerprint: `SHA256:${text}` };
}

function isIpv4(text: string): boolean {
  const parts = text.split(".");
  return parts.length === 4 && parts.every((p) => /^\d{1,3}$/.test(p) && Number(p) <= 255);
}

/** What the back end says while this PC isn't allowed to send pings ("?noping" in the address). */
const PING_PROBLEM =
  "This computer does not let apps send pings, so home-cc can't tell which devices are up. To allow it for good, " +
  "create the file /etc/sysctl.d/60-ping.conf holding the line net.ipv4.ping_group_range = 0 2147483647 and then " +
  "run this in a terminal: sudo sysctl --system";

export function installMock() {
  const start = nowSecs();
  /** "?awkward" in the address adds readings that are allowed but unusual, to check that the screens cope
   *  (tools/screenshots.mjs does). */
  const query = new URLSearchParams(window.location.search);
  const awkward = query.has("awkward");
  /** "?noping" in the address: this PC isn't allowed to send pings, so no status check works. */
  const noPing = query.has("noping");
  const DAY = 86400;
  let nextId = 1;

  function dev(
    name: string,
    host: string,
    kind: DeviceKind,
    os: Os,
    pattern: Pattern,
    more: Partial<MockDevice> = {},
  ): MockDevice {
    return {
      id: nextId++,
      name,
      host,
      kind,
      os,
      // What the back end makes of a database from before it knew where a System came from.
      osSource: os === "other" ? null : "manual",
      sshUser: null,
      sshPort: 22,
      notes: "",
      mac: null,
      vendor: null,
      hostname: null,
      source: "scan",
      reviewed: true,
      firstSeen: start - 40 * DAY,
      lastSeen: start - 8,
      group: null,
      pattern,
      ...more,
    };
  }

  const tpLink = "TP-Link Systems Inc.";
  let devices: MockDevice[] = [
    dev("Living room Deco", "192.168.68.1", "mesh", "other", "wired", { mac: "3c:52:a1:10:2f:01", vendor: tpLink, hostname: "deco-BE85" }),
    dev("Office Deco", "192.168.68.52", "mesh", "other", "wired", { mac: "3c:52:a1:10:2f:5d", vendor: tpLink, hostname: "deco-BE85" }),
    dev("Bedroom Deco", "192.168.68.53", "mesh", "other", "wifi", { mac: "3c:52:a1:10:30:12", vendor: tpLink, hostname: "deco-BE85" }),
    dev("Garage Deco", "192.168.68.54", "mesh", "other", "recovered", { mac: "3c:52:a1:10:31:c4", vendor: tpLink, hostname: "deco-BE85" }),
    // home-cc runs here, so its stats are read without SSH: no SSH user.
    dev("Main PC", "192.168.68.60", "pc", "windows", "self", { mac: "a0:36:bc:5e:11:90", vendor: "ASUSTek Computer Inc.", hostname: "DESKTOP-MAIN", notes: "home-cc runs here" }),
    dev("NAS", "192.168.68.10", "server", "windows", "noPing", { sshUser: "admin", mac: "d8:bb:c1:7a:02:6e", vendor: "Micro-Star INTL Co., Ltd.", hostname: "NAS" }),
    dev("Homelab VM", "192.168.68.21", "vm", "linux", "wired", { sshUser: "sam", mac: "00:15:5d:44:0a:01", vendor: "Microsoft Corporation", hostname: "homelab" }),
    dev("Dev VM", "192.168.68.22", "vm", "linux", "wired", { sshUser: "sam", mac: "00:15:5d:44:0a:02", vendor: "Microsoft Corporation", hostname: "dev" }),
    // "?pvenossh": no SSH user for the Proxmox host, so its consoles can't be opened (the API token is separate).
    dev("Proxmox", "192.168.68.30", "server", "proxmox", "wired", { sshUser: query.has("pvenossh") ? null : "root", mac: "b0:7b:25:1c:9e:40", vendor: "Dell Inc.", hostname: "pve" }),
    dev("Pi-hole 1", "192.168.68.31", "container", "linux", "wired", { sshUser: "root", mac: "bc:24:11:3a:55:01", vendor: "Proxmox Server Solutions GmbH", hostname: "pihole1" }),
    dev("Pi-hole 2", "192.168.68.32", "container", "linux", "wired", { mac: "bc:24:11:3a:55:02", vendor: "Proxmox Server Solutions GmbH", hostname: "pihole2" }),
    dev("Monitoring", "192.168.68.33", "container", "linux", "wired", { sshUser: "root", mac: "bc:24:11:3a:55:03", vendor: "Proxmox Server Solutions GmbH", hostname: "monitoring", notes: "Prometheus and Grafana" }),
    dev("Old laptop", "192.168.68.95", "pc", "linux", "gone", { sshUser: "sam", mac: "54:e1:ad:0f:be:77", vendor: "LCFC(HeFei) Electronics Technology co., ltd", hostname: "thinkpad", lastSeen: start - 30 * 3600 }),
    dev("Printer", "192.168.68.40", "appliance", "other", "wifi", { mac: "1c:2d:3e:4f:5a:6b", vendor: "Brother Industries, Ltd.", hostname: "BRW1C2D3E4F5A6B" }),
    dev("Living room TV", "192.168.68.41", "appliance", "other", "off", { mac: "8c:79:f5:21:d0:3a", vendor: "Samsung Electronics Co.,Ltd", hostname: "Samsung-TV", lastSeen: start - 18 * 60 }),
    dev("Sam's phone", "192.168.68.101", "other", "other", "flaky", { mac: "9a:3f:c2:71:0b:e4", hostname: "Pixel-8" }),
    dev("Tablet", "192.168.68.102", "other", "other", "wifi", { mac: "f2:7c:19:a4:66:d1" }),
    dev("192.168.68.117", "192.168.68.117", "other", "other", "fresh", { mac: "da:a1:19:5c:e0:27", reviewed: false, firstSeen: start - 150 }),
    dev("Galaxy-S24", "192.168.68.118", "other", "other", "fresh", { mac: "5e:0d:b3:92:4f:18", hostname: "Galaxy-S24", reviewed: false, firstSeen: start - 150 }),
    dev("Espressif Inc. device", "192.168.68.130", "other", "other", "fresh", { mac: "24:6f:28:4f:2a:1c", vendor: "Espressif Inc.", reviewed: false, firstSeen: start - 150 }),
    dev("Smart plug", "192.168.68.131", "ignore", "other", "wifi", { mac: "24:6f:28:9b:07:d3", vendor: "Espressif Inc.", hostname: "ESP_9B07D3" }),
    dev("Guest phone", "192.168.68.140", "ignore", "other", "gone", { mac: "b6:44:0e:c8:31:9f", lastSeen: start - 6 * DAY }),
    dev("Media PC", "192.168.68.70", "pc", "windows", "noPing", { sshUser: "sam", mac: "70:85:c2:3b:9d:14", vendor: "ASRock Incorporation", hostname: "MEDIA-PC", access: "password" }),
  ];
  for (const name of ["NAS", "Pi-hole 1"]) {
    const d = devices.find((x) => x.name === name);
    if (d) d.access = "password";
  }
  // Where each System came from. Dev VM and Old laptop were set by hand; the rest that have one were found:
  // this PC knows its own, the Proxmox host's API answered, two were only guessed from the network (Pi-hole 1,
  // whose login fails, and Media PC, waiting for its identity to be approved, which a reading then confirms).
  const sources: [string, OsSource][] = [
    ["Main PC", "this-pc"],
    ["NAS", "ssh"],
    ["Homelab VM", "ssh"],
    ["Proxmox", "proxmox"],
    ["Pi-hole 1", "network"],
    ["Pi-hole 2", "network"],
    ["Monitoring", "ssh"],
    ["Media PC", "network"],
  ];
  for (const [name, source] of sources) {
    const d = devices.find((x) => x.name === name);
    if (d) d.osSource = source;
  }
  // Groups on the Machines screen: the Proxmox host with its containers, and the NAS PC with the VMs it
  // runs. The rest are in no group. "?nogroups" in the address: no machine is in a group.
  if (!query.has("nogroups")) {
    const groups: [string, string[]][] = [
      ["Proxmox lab", ["Proxmox", "Pi-hole 1", "Pi-hole 2", "Monitoring"]],
      ["NAS PC", ["NAS", "Homelab VM", "Dev VM"]],
    ];
    for (const [group, names] of groups) {
      for (const d of devices) if (names.includes(d.name)) d.group = group;
    }
  }
  const idOf = (name: string) => devices.find((d) => d.name === name)?.id ?? -1;

  const settings: Settings = {
    pingIntervalSecs: 30,
    statsIntervalSecs: 60,
    scanRange: "",
    diskWarnPct: 85,
    ramWarnPct: 90,
    sshKeyPath: "",
    defaultSshUser: "",
  };
  const subnets: Subnet[] = [
    { cidr: "192.168.68.0/22", interface: "Ethernet", ownIp: "192.168.68.60", best: true },
    // On the same network by Wi-Fi as well as by cable: the same range comes back twice.
    { cidr: "192.168.68.0/22", interface: "Wi-Fi", ownIp: "192.168.68.61", best: false },
    { cidr: "172.28.160.0/20", interface: "vEthernet (WSL)", ownIp: "172.28.160.1", best: false },
  ];
  const secrets = new Set<string>([`ssh-password:${idOf("NAS")}`, `ssh-password:${idOf("Media PC")}`]);

  // Ping history: two minutes short of an hour, one check every 30 seconds.
  const histories = new Map<number, Sample[]>();
  const rands = new Map<number, () => number>();
  for (const d of devices) {
    if (d.kind === "ignore") continue;
    const rand = seeded(d.id * 7919);
    rands.set(d.id, rand);
    const from = d.pattern === "fresh" ? HISTORY - 5 : 0;
    const list: Sample[] = [];
    for (let i = from; i < HISTORY; i++) {
      const s = sampleFor(d.pattern, i, rand);
      list.push({ ts: start - 8 - (HISTORY - 1 - i) * 30, up: s.up, rttMs: round2(s.rttMs) });
    }
    histories.set(d.id, list);
  }

  function statusOf(d: MockDevice): DeviceStatus | null {
    const history = histories.get(d.id);
    const last = history?.[history.length - 1];
    if (!history || !last || d.kind === "ignore") return null;
    return { deviceId: d.id, up: last.up, rttMs: last.rttMs, checkedAt: last.ts, lastSeen: d.lastSeen, isSelf: d.pattern === "self", history };
  }

  function checkAll() {
    // Without pings there is nothing to record: the last readings just get older.
    if (noPing) return;
    const ts = nowSecs();
    for (const d of devices) {
      if (d.kind === "ignore") continue;
      const rand = rands.get(d.id) ?? seeded(d.id * 7919);
      rands.set(d.id, rand);
      const s = sampleFor(d.pattern, 1000, rand);
      const list = histories.get(d.id) ?? [];
      list.push({ ts, up: s.up, rttMs: round2(s.rttMs) });
      histories.set(d.id, list.slice(-HISTORY));
      if (s.up) d.lastSeen = ts;
    }
  }

  // Machine stats, one scenario per machine.
  const disk = (mount: string, label: string, totalGb: number, usedGb: number): Disk => ({
    mount,
    label,
    totalBytes: Math.round(totalGb * GB),
    usedBytes: Math.round(usedGb * GB),
  });
  const kb = (gb: number) => Math.round(gb * 1024 * 1024);
  const linux = (osName: string, more: Partial<MachineStats>): MachineStats => ({
    osName,
    uptimeSecs: 4 * DAY + 3 * 3600,
    cpuPct: 7,
    load1: 0.31,
    cpuCount: 4,
    memTotalKb: kb(8),
    memUsedKb: kb(3.1),
    disks: [],
    reduced: false,
    ...more,
  });
  const goodStats = new Map<number, MachineStats>([
    [idOf("Main PC"), {
      osName: "Microsoft Windows 11 Pro",
      uptimeSecs: 2 * DAY + 5 * 3600,
      cpuPct: 12,
      load1: null,
      cpuCount: 16,
      memTotalKb: kb(31.9),
      memUsedKb: kb(14.6),
      disks: [disk("C:", "", 930.5, 512.2), disk("D:", "", 1862.9, 1204.7)],
      reduced: false,
    }],
    // Its SSH account isn't an administrator there, so the readings are limited.
    [idOf("NAS"), {
      osName: "Windows 11",
      uptimeSecs: 21 * DAY + 7 * 3600,
      cpuPct: null,
      load1: null,
      cpuCount: 12,
      memTotalKb: kb(63.9),
      memUsedKb: kb(41.3),
      disks: [disk("C:", "System", 465.1, 201.4), disk("D:", "Media", 14901.2, 13872.6), disk("E:", "Backups", 7450.6, 4310.9)],
      reduced: true,
    }],
    [idOf("Homelab VM"), linux("Ubuntu 24.04.1 LTS", {
      cpuPct: 23,
      load1: 0.92,
      memTotalKb: kb(15.6),
      memUsedKb: kb(14.5),
      disks: [disk("/", "/dev/sda2", 96.8, 61.3), disk("/var/lib/docker", "/dev/sdb1", 250, 148.9)],
    })],
    [idOf("Dev VM"), linux("Debian GNU/Linux 12 (bookworm)", {
      uptimeSecs: 9 * 3600 + 12 * 60,
      cpuPct: null,
      load1: 0.08,
      memUsedKb: kb(1.9),
      disks: [disk("/", "/dev/sda1", 62.7, 23.4)],
    })],
    [idOf("Proxmox"), linux("Proxmox VE 8.3 (Debian GNU/Linux 12)", {
      uptimeSecs: 63 * DAY + 2 * 3600,
      cpuPct: 11,
      load1: 0.64,
      cpuCount: 8,
      memTotalKb: kb(31.2),
      memUsedKb: kb(19.8),
      disks: [disk("/", "/dev/mapper/pve-root", 93.9, 31.2), disk("/var/lib/vz", "/dev/mapper/pve-data", 812.3, 402.5)],
    })],
    [idOf("Pi-hole 1"), linux("Debian GNU/Linux 12 (bookworm)", {
      cpuPct: 2,
      load1: 0.02,
      cpuCount: 1,
      memTotalKb: kb(0.5),
      memUsedKb: kb(0.21),
      disks: [disk("/", "/dev/loop0", 7.8, 2.1)],
    })],
    [idOf("Monitoring"), linux("Debian GNU/Linux 12 (bookworm)", {
      uptimeSecs: 63 * DAY + 1 * 3600,
      cpuPct: null,
      load1: 0.42,
      cpuCount: 2,
      memTotalKb: kb(4),
      memUsedKb: kb(2.7),
      disks: [disk("/", "/dev/loop2", 31.4, 22.9)],
    })],
    [idOf("Old laptop"), linux("Fedora Linux 41 (Workstation Edition)", {
      uptimeSecs: 2 * 3600 + 40 * 60,
      cpuPct: 4,
      load1: 0.15,
      cpuCount: 8,
      memTotalKb: kb(15.4),
      memUsedKb: kb(5.2),
      disks: [disk("/", "/dev/nvme0n1p3", 237.9, 96.4)],
    })],
    [idOf("Media PC"), {
      osName: "Microsoft Windows 11 Home",
      uptimeSecs: 6 * 3600 + 25 * 60,
      cpuPct: 9,
      load1: null,
      cpuCount: 8,
      memTotalKb: kb(15.9),
      memUsedKb: kb(6.8),
      disks: [disk("C:", "", 476.3, 288.1)],
      reduced: false,
    }],
  ]);

  if (awkward) {
    // Two filesystems mounted on the same folder, both nearly full.
    goodStats.get(idOf("Homelab VM"))?.disks.push(disk("/mnt/data", "/dev/sdc1", 500, 471.5), disk("/mnt/data", "/dev/sdd1", 200, 188.2));
  }

  // SSH identities. Each machine presents one; home-cc stores the one it accepts.
  const serverKeys = new Map<number, HostKey>();
  const pinned = new Map<number, HostKey>();
  /** Machines whose stored identity the owner has approved: only these are sent the password. */
  const approved = new Set<number>();
  function serverKey(d: MockDevice): HostKey {
    let key = serverKeys.get(d.id);
    if (!key) serverKeys.set(d.id, (key = sampleKey(d.id * 104729)));
    return key;
  }
  const hasPassword = (d: MockDevice) => secrets.has(`ssh-password:${d.id}`);
  const say = {
    changed: (was: HostKey, now: HostKey) =>
      `This machine's identity has changed since home-cc first reached it (was ${was.fingerprint}, now ${now.fingerprint})`,
    newHost: "The stored password has not been sent: this machine's identity needs your approval first.",
    auth: (d: MockDevice) =>
      `Couldn't log in as ${d.sshUser}. Tried the key file id_ed25519 and the ssh-agent; ` +
      (hasPassword(d) ? "the stored password was refused too." : "no password is stored for this machine."),
  };

  const entries = new Map<number, MachineStatsEntry>();
  const blankEntry = (deviceId: number): MachineStatsEntry => ({
    deviceId,
    stats: null,
    collectedAt: null,
    error: null,
    errorKind: null,
    offeredKey: null,
    expectedFingerprint: null,
    pinnedFingerprint: null,
    paused: false,
    local: false,
  });
  /** The PC home-cc runs on: read here without SSH, with or without an SSH user. */
  const isLocal = (d: MockDevice) => d.pattern === "self" && d.kind !== "ignore";
  /** Machines with stats: those with an SSH user, and this PC. */
  const readsStats = (d: MockDevice) => d.kind !== "ignore" && (!!d.sshUser || isLocal(d));
  /** An entry as the screens get it: the stored identity is looked up at that moment. */
  const publicEntry = (e: MachineStatsEntry): MachineStatsEntry => ({ ...e, pinnedFingerprint: pinned.get(e.deviceId)?.fingerprint ?? null });
  const entry = (
    name: string,
    age: number | null,
    error: string | null = null,
    errorKind: StatsErrorKind | null = null,
    more: Partial<MachineStatsEntry> = {},
  ) => {
    const deviceId = idOf(name);
    const stats = age == null ? null : (goodStats.get(deviceId) ?? null);
    entries.set(deviceId, { ...blankEntry(deviceId), stats, collectedAt: age == null ? null : start - age, error, errorKind, ...more });
  };
  // Machines that have been reached before have their identity stored.
  for (const d of devices) {
    if (d.sshUser && d.name !== "Dev VM" && d.name !== "Media PC") {
      pinned.set(d.id, serverKey(d));
      if (d.access === "password") approved.add(d.id);
    }
  }
  entry("Main PC", 21, null, null, { local: true });
  entry("NAS", 20);
  entry("Homelab VM", 19);
  // Never read yet.
  entry("Dev VM", null);
  // Reinstalled since the last reading: it presents a new identity.
  const proxmox = devices.find((d) => d.name === "Proxmox");
  if (proxmox) {
    const was = serverKey(proxmox);
    const now = sampleKey(proxmox.id * 15485863);
    serverKeys.set(proxmox.id, now);
    entry("Proxmox", 2 * 3600 + 400, say.changed(was, now), "hostKeyChanged", { offeredKey: now, expectedFingerprint: was.fingerprint });
  }
  // No key is accepted and no password is stored: the login fails, and home-cc stops retrying by itself.
  const pihole = devices.find((d) => d.name === "Pi-hole 1");
  if (pihole) entry("Pi-hole 1", null, say.auth(pihole), "auth", { paused: true });
  entry("Monitoring", 18);
  entry("Old laptop", 30 * 3600, "Couldn't reach 192.168.68.95: the connection timed out. Is the machine switched on?", "unreachable");
  // First connection, and getting in needs the stored password: it waits for the owner's approval.
  const mediaPc = devices.find((d) => d.name === "Media PC");
  if (mediaPc) entry("Media PC", null, say.newHost, "newHost", { offeredKey: serverKey(mediaPc), paused: true });

  /** One try at reading a machine, by the same rules as the back end:
   *  - an identity that differs from the stored one stops everything until the owner approves it;
   *  - on a first connection the stored password is not sent before the owner approves the identity
   *    (a key file may be used, and the identity is then stored without asking);
   *  - a failed login pauses the machine, and so does waiting for a first approval.
   *  "known" is the identity stored when the try began; a try that takes a while doesn't see one
   *  approved in the meantime. */
  function readStats(d: MockDevice, known: HostKey | null = pinned.get(d.id) ?? null): MachineStatsEntry {
    const prev = entries.get(d.id);
    const stats = goodStats.get(d.id);
    const failed = (errorKind: StatsErrorKind, error: string, more: Partial<MachineStatsEntry> = {}): MachineStatsEntry => ({
      ...blankEntry(d.id),
      stats: prev?.stats ?? null,
      collectedAt: prev?.collectedAt ?? null,
      error,
      errorKind,
      ...more,
    });
    const key = serverKey(d);
    const needsPassword = d.access === "password";
    let next: MachineStatsEntry;
    if (isLocal(d) && stats) {
      // Read here: no identity, login or pause, and CPU from the full Windows query.
      next = { ...blankEntry(d.id), local: true, stats: { ...stats, uptimeSecs: stats.uptimeSecs == null ? null : stats.uptimeSecs + (nowSecs() - start) }, collectedAt: nowSecs() };
    } else if (d.pattern === "gone" && prev?.errorKind === "unreachable") {
      next = prev;
    } else if (!stats || d.pattern === "gone") {
      next = failed("unreachable", `Couldn't reach ${d.host}: nothing answered on port ${d.sshPort}. Is SSH turned on there?`);
    } else if (known && known.fingerprint !== key.fingerprint) {
      next = failed("hostKeyChanged", say.changed(known, key), { offeredKey: key, expectedFingerprint: known.fingerprint });
    } else if (needsPassword && hasPassword(d) && !approved.has(d.id)) {
      // The identity is remembered, but the password waits for the owner's approval, and the timer
      // doesn't try again by itself.
      if (!known && !pinned.has(d.id)) pinned.set(d.id, key);
      next = failed("newHost", say.newHost, { offeredKey: known ?? key, paused: true });
    } else {
      // The identity is remembered on first contact, before the login is tried. The first one stored
      // stays: an approval given while this try was under way is not overwritten.
      if (!known && !pinned.has(d.id)) pinned.set(d.id, key);
      if (needsPassword && !hasPassword(d)) {
        next = failed("auth", say.auth(d), { paused: true });
      } else {
        const wobble = (n: number | null) => (n == null ? null : Math.max(1, Math.min(99, Math.round(n + (Math.random() - 0.5) * 8))));
        // A Linux machine has no CPU percentage on its first reading, then gets one.
        const cpuPct = stats.cpuPct ?? (prev?.stats && !stats.reduced && d.os !== "windows" ? 6 : null);
        const fresh = { ...stats, cpuPct: wobble(cpuPct), uptimeSecs: stats.uptimeSecs == null ? null : stats.uptimeSecs + (nowSecs() - start) };
        next = { ...blankEntry(d.id), stats: fresh, collectedAt: nowSecs() };
        // What the reading shows fills in the System, unless the owner chose one (as the back end does).
        const found: Os = /windows/i.test(stats.osName) ? "windows" : /proxmox/i.test(stats.osName) ? "proxmox" : "linux";
        if (found !== d.os || d.osSource !== "ssh") {
          if (mayReplace(d, found, "ssh")) {
            d.os = found;
            d.osSource = "ssh";
            setTimeout(() => emit("status-changed"), 0);
          }
        }
      }
    }
    entries.set(d.id, next);
    return next;
  }

  /** Like the back end after a secret changes: paused machines get another try straight away. */
  function retryPaused(only: number | null) {
    const again = devices.filter((d) => entries.get(d.id)?.paused && (only == null || d.id === only));
    if (again.length === 0) return;
    setTimeout(() => {
      for (const d of again) if (readsStats(d) && entries.has(d.id)) readStats(d);
      emit("stats-changed");
    }, 600);
  }

  /** For "?awkward": set once an approval has been overtaken by a newer identity. */
  let raced = false;

  // Key login. This PC has an ed25519 key, unless "?nokey" is in the address (to show making one).
  const samplePublicKey = (seed: number, comment: string) => {
    const rand = seeded(seed);
    let text = "AAAAC3NzaC1lZDI1NTE5AAAAI";
    for (let i = 0; i < 43; i++) text += BASE64[Math.floor(rand() * 64)] ?? "A";
    return `ssh-ed25519 ${text} ${comment}`;
  };
  const keyPath = "C:\\Users\\sam\\.ssh\\id_ed25519";
  const noKey: LocalKey = {
    path: null,
    name: null,
    fromSettings: false,
    publicKey: null,
    algorithm: null,
    fingerprint: null,
    problem: null,
    note: null,
    others: [],
    newKeyPath: keyPath,
  };
  const madeKey = (): LocalKey => ({
    ...noKey,
    path: keyPath,
    name: "id_ed25519",
    publicKey: samplePublicKey(77, "sam@DESKTOP-MAIN"),
    algorithm: "ssh-ed25519",
    fingerprint: sampleKey(77).fingerprint,
    newKeyPath: null,
  });
  let localKey: LocalKey = query.has("nokey") ? noKey : madeKey();
  /** Machines that hold this PC's key already. */
  const keyed = new Set<number>();
  /** Identities a key setup showed, waiting for the owner's approval. */
  const keyOffers = new Map<number, HostKey>();

  /** A key setup by the same rules as the back end: a password (typed or stored) only goes to an approved
   *  identity, a changed identity stops everything, and a key login is tried at the end. The sample machines:
   *  the Linux VMs take the key and then key login works; Media PC wants its identity approved first; NAS takes
   *  the key but still refuses a key login (Windows administrator file); Pi-hole 1 has no password stored, and
   *  refuses root's password. */
  async function setupKeyLogin(d: MockDevice, password: string | null): Promise<KeySetupOutcome> {
    const user = d.sshUser;
    if (!user) throw "This machine has no SSH user. Add one and save, then try again.";
    if (!localKey.publicKey) throw "This PC has no SSH key yet. Make one first (above).";
    await sleep(1300);
    if (d.pattern === "gone") {
      throw `Couldn't connect to ${d.name} to set up key login. ${d.host} did not answer. Check that it is switched on and on the network.`;
    }
    const key = serverKey(d);
    const known = pinned.get(d.id) ?? null;
    if (known && known.fingerprint !== key.fingerprint) {
      keyOffers.set(d.id, key);
      return { outcome: "approve", offeredKey: key, expectedFingerprint: known.fingerprint };
    }
    // Remembered on first contact (not approved), as the stats timer does.
    if (!known) pinned.set(d.id, key);
    const windows = d.os === "windows";
    if (d.access === "password" && !keyed.has(d.id)) {
      if (!password && !hasPassword(d)) {
        throw (
          `home-cc couldn't get in to ${d.name} with a key, and no password is typed or stored for it. Type ${user}'s ` +
          "password in the box above, then press Set up key login again. It is used for this once and stored only if " +
          `you press Save. What home-cc saw: Could not log in to ${d.host} as ${user}. The key file id_ed25519 was not ` +
          "accepted. Check the user name, and that this PC's public key has been added to that machine, or store a password for it."
        );
      }
      if (!approved.has(d.id)) {
        keyOffers.set(d.id, key);
        return { outcome: "approve", offeredKey: key, expectedFingerprint: null };
      }
      if (user === "root") {
        throw (
          `Could not log in to ${d.host} as root. The key file id_ed25519 and ` +
          `${password ? "the password typed in the form" : "the stored password"} were not accepted. Check the user name ` +
          "and the password. Many Linux systems don't let root log in with a password over SSH (PermitRootLogin in " +
          "/etc/ssh/sshd_config). Use another user, or add the key by hand."
        );
      }
    }
    const file = windows
      ? "C:\\ProgramData\\ssh\\administrators_authorized_keys"
      : user === "root" ? "/root/.ssh/authorized_keys" : `/home/${user}/.ssh/authorized_keys`;
    const already = keyed.has(d.id);
    keyed.add(d.id);
    const what = already ? `This PC's key was already in ${file} on ${d.name}` : `home-cc added this PC's key to ${file} on ${d.name}`;
    if (d.name === "NAS") {
      return {
        outcome: "done",
        file,
        alreadyThere: already,
        keyLoginWorks: false,
        windows,
        message: `${what}, but logging in with the key alone still didn't work. What home-cc saw: Could not log in to ${d.host} as ${user}. The key file id_ed25519 was not accepted.`,
        advice:
          `${user} is an administrator, so the key has to be in C:\\ProgramData\\ssh\\administrators_authorized_keys ` +
          "(home-cc put it there), open to Administrators and SYSTEM only. Check C:\\ProgramData\\ssh\\sshd_config on NAS " +
          "(PubkeyAuthentication must not be no; after a change, run Restart-Service sshd in PowerShell as administrator), " +
          "and Windows' own log of the refusal: Event Viewer, Applications and Services Logs, OpenSSH, Operational.",
      };
    }
    // From now on the key gets in by itself.
    d.access = "key";
    retryPaused(d.id);
    return { outcome: "done", file, alreadyThere: already, keyLoginWorks: true, windows, message: `Key login works. ${what}, and then logged in with the key alone.`, advice: null };
  }

  // Scan simulation.
  let scanTimer: ReturnType<typeof setInterval> | null = null;
  let scanCancelled = false;
  let scansDone = 0;

  function addressCount(range: string): number {
    const prefix = Number(range.split("/")[1]);
    return Number.isInteger(prefix) && prefix >= 16 && prefix <= 30 ? 2 ** (32 - prefix) - 2 : 254;
  }

  function finishScan(range: string, cancelled: boolean): ScanResult {
    const ts = nowSecs();
    let added = 0;
    if (!cancelled && scansDone === 0) {
      const d = dev("Amazon Technologies Inc. device", "192.168.68.150", "other", "other", "fresh", {
        mac: "fc:65:de:83:b1:42",
        vendor: "Amazon Technologies Inc.",
        reviewed: false,
        firstSeen: ts,
        lastSeen: ts,
      });
      devices.push(d);
      added = 1;
    }
    if (!cancelled) scansDone++;
    const present = devices.filter((d) => d.pattern !== "gone" && d.pattern !== "off");
    const found: Found[] = present.slice(0, cancelled ? 9 : present.length).map((d) => {
      d.lastSeen = ts;
      const history = histories.get(d.id);
      return {
        ip: d.host,
        mac: d.mac,
        vendor: d.vendor,
        hostname: d.hostname,
        looksLikeMesh: d.vendor === tpLink,
        rttMs: d.pattern === "noPing" || d.pattern === "self" ? null : (history?.[history.length - 1]?.rttMs ?? 4.2),
        isSelf: d.pattern === "self",
      };
    });
    return { range, found, added, updated: found.length - added, cancelled };
  }

  function startScan(): Promise<ScanResult> {
    if (scanTimer) throw "A scan is already running";
    const range = settings.scanRange || subnets[0].cidr;
    const total = addressCount(range);
    const expect = devices.filter((d) => d.pattern !== "gone" && d.pattern !== "off").length + (scansDone === 0 ? 1 : 0);
    scanCancelled = false;
    const steps: ScanProgress[] = [];
    for (let i = 0; i <= 40; i++) {
      steps.push({ phase: "sweep", done: Math.round((total * i) / 40), total, found: Math.round((expect - 2) * Math.sqrt(i / 40)) });
    }
    for (let i = 0; i <= 6; i++) steps.push({ phase: "confirm", done: i * 2, total: 12, found: expect - 2 + Math.round(i / 3) });
    for (let i = 0; i <= 8; i++) steps.push({ phase: "names", done: Math.round((expect * i) / 8), total: expect, found: expect });

    return new Promise((resolve) => {
      let i = 0;
      const done = (cancelled: boolean) => {
        if (scanTimer) clearInterval(scanTimer);
        scanTimer = null;
        const result = finishScan(range, cancelled);
        // Checks wait while a scan runs; a round follows as soon as it ends.
        checkAll();
        emit("status-changed");
        resolve(result);
      };
      scanTimer = setInterval(() => {
        if (scanCancelled) return done(true);
        const step = steps[i++];
        if (!step) return done(false);
        emit("scan-progress", step);
      }, 130);
    });
  }

  // Timers, like the back end's: a round of checks, and a round of stats.
  let pingTimer: ReturnType<typeof setInterval>;
  let statsTimer: ReturnType<typeof setInterval>;
  function startTimers() {
    clearInterval(pingTimer);
    clearInterval(statsTimer);
    pingTimer = setInterval(() => {
      if (scanTimer) return;
      checkAll();
      emit("status-changed");
    }, settings.pingIntervalSecs * 1000);
    statsTimer = setInterval(() => {
      for (const d of devices) {
        // The machine waiting for its first reading stays that way until it is refreshed by hand.
        const e = entries.get(d.id);
        const waiting = e?.stats == null && e?.error == null;
        // After a failed login a machine is left alone until the owner does something about it.
        if (readsStats(d) && !waiting && !e?.paused) readStats(d);
      }
      emit("stats-changed");
    }, settings.statsIntervalSecs * 1000);
  }
  startTimers();

  function checkInput(input: DeviceInput) {
    if (!input.name?.trim()) throw "Name can't be empty";
    if (!input.host?.trim()) throw "Address can't be empty";
    if (/\s/.test(input.host.trim())) throw "Address can't contain spaces";
    if (/^[0-9.]+$/.test(input.host.trim()) && !isIpv4(input.host.trim())) {
      throw "That address isn't complete. An address made of numbers needs four of them, like 192.168.1.20";
    }
    const mac = input.mac?.trim();
    if (mac && mac.replace(/[^0-9a-f]/gi, "").length !== 12) throw "Hardware address should look like aa:bb:cc:dd:ee:ff";
    if (input.group != null) tidyGroup(input.group);
  }

  /** A group name as the back end stores it (inventory.rs tidy_group): trimmed, one space between words,
   *  at most 40 characters. Null for none. */
  function tidyGroup(name: string): string | null {
    const tidy = name.split(/\s+/).filter(Boolean).join(" ");
    if (!tidy) return null;
    // eslint-disable-next-line no-control-regex
    if (/[\u0000-\u001f\u007f-\u009f]/.test(tidy)) throw "A group name can't contain tabs or other invisible characters";
    if ([...tidy].length > 40) throw "A group name can be at most 40 characters long";
    return tidy;
  }

  /** The spelling of a group that already has this name in other letter case, as the back end does it:
   *  typing "nas pc" puts a machine in "NAS PC". Letter case is taken out with the back end's own rule
   *  (foldGroup), not a looser one, so the screens can't be shown working here and fail in the app. */
  function storedSpelling(name: string, exceptId: number | null, exceptGroup: string | null): string {
    const others = devices.filter((d) => d.id !== exceptId && d.group && d.group !== exceptGroup).map((d) => d.group as string);
    return others.find((g) => g === name) ?? others.find((g) => foldGroup(g) === foldGroup(name)) ?? name;
  }

  /** The group to store for a device: `typed` not given keeps `current`. */
  function groupFrom(typed: string | null | undefined, current: string | null, id: number | null): string | null {
    if (typed == null) return current;
    const tidy = tidyGroup(typed);
    return tidy === null ? null : storedSpelling(tidy, id, null);
  }

  function normalMac(mac: string | null | undefined): string | null {
    const hex = (mac ?? "").replace(/[^0-9a-f]/gi, "").toLowerCase();
    return hex.length === 12 ? (hex.match(/../g) ?? []).join(":") : null;
  }

  /** Strips one pair of quotes from around a path: Windows' "Copy as path" adds them. */
  function unquote(text: string): string {
    const t = text.trim();
    const quoted = t.length >= 2 && (t[0] === '"' || t[0] === "'") && t[t.length - 1] === t[0];
    return quoted ? t.slice(1, -1).trim() : t;
  }

  /** The machine a password entry belongs to, or null for a secret every machine shares. */
  function passwordOwner(key: string): number | null {
    const m = /^ssh-password:(\d+)$/.exec(key);
    return m ? Number(m[1]) : null;
  }

  /** Names the sample network gives addresses, tidied as the back end does it ("nas.lan" gives "nas").
   *  Devices give the name a scan found for them; one device never has a name. */
  const SAMPLE_NAMES: Record<string, string> = {
    // This PC by Wi-Fi, and its address on the WSL adapter.
    "192.168.68.61": "DESKTOP-MAIN",
    "172.28.160.1": "DESKTOP-MAIN",
    "192.168.68.102": "ipad",
    "192.168.68.130": "esp-4f2a1c",
    "192.168.68.140": "android-5c1e",
  };
  const NEVER_NAMED = "192.168.68.117";
  function sampleName(address: string): string | null {
    const a = address.trim();
    if (!isIpv4(a) || a === NEVER_NAMED) return null;
    return SAMPLE_NAMES[a] ?? devices.find((d) => d.host === a)?.hostname ?? `host-${a.split(".")[3]}`;
  }

  const publicDevice = ({ pattern: _pattern, access: _access, ...d }: MockDevice): Device => d;
  const find = (id: number): MockDevice => {
    const d = devices.find((x) => x.id === id);
    if (!d) throw `Not found: device ${id}`;
    return d;
  };

  // The arguments arrive untyped, as they do in Rust; each command says what it expects.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const commands: Record<string, (args: any) => unknown> = {
    app_info: () => ({ version: "0.1.0", os: noPing ? "linux" : "windows", arch: "x86_64", release: false }),
    recent_log: () => [{ at: nowSecs() - 90, text: "could not read ssh-password:3 from the keychain: sample error" }],
    list_devices: () => devices.map(publicDevice),
    add_device: ({ input }: { input: DeviceInput }) => {
      checkInput(input);
      const system = systemAfterSave(null, input);
      const d = dev(input.name.trim(), input.host.trim(), input.kind, system.os, "wired", {
        osSource: system.osSource,
        sshUser: input.sshUser?.trim() || null,
        sshPort: input.sshPort ?? 22,
        notes: input.notes ?? "",
        mac: normalMac(input.mac),
        source: "manual",
        firstSeen: nowSecs(),
        lastSeen: null,
        group: groupFrom(input.group, null, null),
      });
      devices.push(d);
      if (readsStats(d)) entries.set(d.id, blankEntry(d.id));
      return publicDevice(d);
    },
    update_device: ({ id, input }: { id: number; input: DeviceInput }) => {
      checkInput(input);
      const d = find(id);
      const host = input.host.trim();
      const sshPort = input.sshPort ?? 22;
      const moved = host !== d.host;
      // The hardware address: not given keeps it, empty removes it, anything else replaces it.
      let { mac, vendor, hostname } = d;
      if (input.mac == null) {
        // A new address with no hardware address given: the entry now describes whatever is there.
        if (moved) mac = vendor = hostname = null;
      } else {
        const next = normalMac(input.mac);
        if (next !== mac) vendor = null;
        mac = next;
      }
      // Pointed somewhere else: the stored identity belonged to the old place, so first-use rules apply again.
      if (moved || sshPort !== d.sshPort) pinned.delete(d.id);
      approved.delete(d.id);
      Object.assign(d, {
        name: input.name.trim(),
        host,
        kind: input.kind,
        ...systemAfterSave(d, input, moved),
        sshUser: input.sshUser?.trim() || null,
        sshPort,
        notes: input.notes ?? "",
        mac,
        vendor,
        hostname,
        reviewed: true,
        group: groupFrom(input.group, d.group, d.id),
      });
      const e = entries.get(d.id);
      if (!readsStats(d)) entries.delete(d.id);
      else if (!e) entries.set(d.id, blankEntry(d.id));
      // An edit ends a pause.
      else if (e.paused) entries.set(d.id, { ...e, paused: false });
      return publicDevice(d);
    },
    set_device_group: ({ id, group }: { id: number; group: string }) => {
      const d = find(id);
      d.group = groupFrom(group, d.group, d.id);
      return publicDevice(d);
    },
    rename_group: ({ from, to }: { from: string; to: string }) => {
      const old = tidyGroup(from);
      if (old === null) throw "Say which group to rename";
      const tidy = tidyGroup(to);
      const next = tidy === null ? null : storedSpelling(tidy, null, old);
      const members = devices.filter((d) => d.group === old);
      if (members.length === 0) throw `Not found: group "${old}"`;
      for (const d of members) d.group = next;
      return members.length;
    },
    remove_device: ({ id }: { id: number }) => {
      find(id);
      devices = devices.filter((d) => d.id !== id);
      histories.delete(id);
      entries.delete(id);
      pinned.delete(id);
      approved.delete(id);
      serverKeys.delete(id);
      secrets.delete(`ssh-password:${id}`);
    },
    set_secret: ({ key }: { key: string }) => {
      secrets.add(key);
      retryPaused(passwordOwner(key));
    },
    has_secret: ({ key }: { key: string }) => secrets.has(key),
    delete_secret: ({ key }: { key: string }) => {
      secrets.delete(key);
      retryPaused(passwordOwner(key));
    },

    get_settings: () => ({ ...settings }),
    save_settings: ({ settings: next }: { settings: Settings }) => {
      if (!(next.pingIntervalSecs >= 5 && next.pingIntervalSecs <= 3600)) throw "Ping timer must be between 5 seconds and 1 hour";
      if (!(next.statsIntervalSecs >= 10 && next.statsIntervalSecs <= 3600)) throw "Stats timer must be between 10 seconds and 1 hour";
      const okPct = (n: number) => n >= 1 && n <= 100;
      if (!okPct(next.diskWarnPct) || !okPct(next.ramWarnPct)) throw "Warning levels must be between 1 and 100 percent";
      Object.assign(settings, next, {
        scanRange: next.scanRange.trim(),
        sshKeyPath: unquote(next.sshKeyPath),
        defaultSshUser: next.defaultSshUser.trim(),
      });
      startTimers();
      return { ...settings };
    },

    list_subnets: () => subnets,
    start_scan: () => startScan(),
    cancel_scan: () => void (scanCancelled = true),
    scan_running: () => scanTimer != null,

    device_statuses: () => devices.map(statusOf).filter((s) => s != null),
    check_now: async () => {
      // Like the back end: refused while a scan runs, and while pings can't be sent at all.
      if (scanTimer) throw "A scan is running. Every device is checked when it ends.";
      if (noPing) throw PING_PROBLEM;
      await sleep(900);
      checkAll();
      emit("status-changed");
    },
    check_problem: () => (noPing ? PING_PROBLEM : null),
    resolve_names: async ({ addresses }: { addresses: string[] }) => {
      if (addresses.length > 256) throw `Names can be looked up for at most 256 addresses at a time, not ${addresses.length}.`;
      // Long enough for "looking…" to be seen.
      await sleep(250);
      return addresses.map((address) => ({ address, name: sampleName(address) }));
    },

    machine_stats: () =>
      devices
        .filter(readsStats)
        .map((d) => entries.get(d.id))
        .filter((e) => e != null)
        .map(publicEntry),
    refresh_stats: async ({ deviceId }: { deviceId: number }) => {
      const d = find(deviceId);
      if (!d.sshUser && !isLocal(d)) throw "Add an SSH user to this machine first";
      // The try starts now, with the identity stored at this moment, and takes a while.
      const known = pinned.get(d.id) ?? null;
      await sleep(1100);
      return publicEntry(readStats(d, known));
    },
    trust_host_key: ({ deviceId, fingerprint }: { deviceId: number; fingerprint: string }) => {
      const d = find(deviceId);
      if (awkward && !raced && entries.get(d.id)?.errorKind === "hostKeyChanged") {
        // Once: the machine showed yet another identity in a round the screen hasn't heard about.
        raced = true;
        serverKeys.set(d.id, sampleKey(d.id * 32452843, "ssh-rsa"));
        readStats(d);
      }
      const offered = entries.get(d.id)?.offeredKey;
      if (!offered) throw "This machine isn't waiting for its identity to be approved.";
      if (offered.fingerprint !== fingerprint) {
        throw "This machine has shown a different identity since you looked. Check the new one before you approve it.";
      }
      // Exactly the identity the owner was shown, replacing any stored before.
      pinned.set(d.id, offered);
      approved.add(d.id);
    },

    local_ssh_key: () => ({ ...localKey }),
    create_ssh_key: async ({ passphrase }: { passphrase: string | null }) => {
      if (localKey.publicKey) throw "This PC already has an SSH key, so home-cc didn't make another.";
      await sleep(700);
      localKey = madeKey();
      if (passphrase) secrets.add("ssh-key-passphrase");
      return { ...localKey };
    },
    setup_key_login: ({ deviceId, password }: { deviceId: number; password: string | null }) => setupKeyLogin(find(deviceId), password),
    approve_key_setup_identity: ({ deviceId, fingerprint }: { deviceId: number; fingerprint: string }) => {
      const d = find(deviceId);
      const offered = keyOffers.get(d.id);
      if (!offered || offered.fingerprint !== fingerprint) {
        throw "This machine has shown a different identity since, or its address changed. Try setting up key login again and check the identity it shows then.";
      }
      pinned.set(d.id, offered);
      approved.add(d.id);
      keyOffers.delete(d.id);
      // The card was waiting for this approval too: it reads the machine again.
      setTimeout(() => {
        if (d.sshUser && entries.has(d.id)) readStats(d);
        emit("stats-changed");
      }, 600);
    },
    // --- Terminals (mock/terminal.ts) ---
    // Logging in fails the way the machine's last stats reading did, with the back end's words.
    ...terminalCommands((deviceId) => {
      const d = find(deviceId);
      const e = entries.get(deviceId);
      const approve = `Approve it on the Machines screen, then open the terminal again.`;
      const problem =
        e?.errorKind === "newHost"
          ? `Logging in to ${d.name} needs its stored password, and home-cc only sends that once you have approved this machine's identity. ${approve}`
          : e?.errorKind === "hostKeyChanged"
            ? `${d.name}'s identity has changed since home-cc first reached it, so nothing was sent to it. Check the new identity on the Machines screen and approve it there if you expected the change, then open the terminal again.`
            : e?.errorKind === "auth" || e?.errorKind === "unreachable"
              ? (e.error ?? "Couldn't log in.")
              : null;
      return { name: d.name, user: d.sshUser, hostname: d.hostname?.toLowerCase() ?? "machine", os: d.os, problem };
    }),
    // --- Docker (spec 5.5), see docker.ts ---
    ...dockerMock({ devices: () => devices, idOf }),
  };

  // --- Proxmox: its sample host and commands live in mock/proxmox.ts ---
  Object.assign(commands, proxmoxMock({ find, secrets, query }, idOf("Proxmox")));
  // --- end Proxmox ---
  // --- Pi-hole: its sample Pi-holes and commands live in mock/pihole.ts ---
  Object.assign(commands, piholeMock({ find, devices: () => devices, secrets, query }, [idOf("Pi-hole 1"), idOf("Pi-hole 2")]));
  // --- end Pi-hole ---

  mockIPC(
    (cmd, payload) => {
      const run = commands[cmd];
      if (!run) throw `The sample back end has no command "${cmd}"`;
      return run(payload ?? {});
    },
    { shouldMockEvents: true },
  );
  // Lets the screens say that this is sample data.
  (window as unknown as { __HOME_CC_MOCK__?: boolean }).__HOME_CC_MOCK__ = true;
}
