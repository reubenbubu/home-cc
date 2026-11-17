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
  MachineStats,
  MachineStatsEntry,
  Os,
  Sample,
  ScanProgress,
  ScanResult,
  Settings,
  StatsErrorKind,
  Subnet,
} from "../api";

/** How a sample device behaves when it is checked. */
type Pattern = "wired" | "wifi" | "flaky" | "recovered" | "noPing" | "self" | "off" | "gone" | "fresh";

interface MockDevice extends Device {
  pattern: Pattern;
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

export function installMock() {
  const start = nowSecs();
  /** "?awkward" in the address adds readings that are allowed but unusual, to check that the screens cope
   *  (tools/screenshots.mjs does). */
  const awkward = new URLSearchParams(window.location.search).has("awkward");
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
    dev("Main PC", "192.168.68.60", "pc", "windows", "self", { sshUser: "sam", mac: "a0:36:bc:5e:11:90", vendor: "ASUSTek Computer Inc.", hostname: "DESKTOP-MAIN", notes: "home-cc runs here" }),
    dev("NAS", "192.168.68.10", "server", "windows", "noPing", { sshUser: "admin", mac: "d8:bb:c1:7a:02:6e", vendor: "Micro-Star INTL Co., Ltd.", hostname: "NAS" }),
    dev("Homelab VM", "192.168.68.21", "vm", "linux", "wired", { sshUser: "sam", mac: "00:15:5d:44:0a:01", vendor: "Microsoft Corporation", hostname: "homelab" }),
    dev("Dev VM", "192.168.68.22", "vm", "linux", "wired", { sshUser: "sam", mac: "00:15:5d:44:0a:02", vendor: "Microsoft Corporation", hostname: "dev" }),
    dev("Proxmox", "192.168.68.30", "server", "proxmox", "wired", { sshUser: "root", mac: "b0:7b:25:1c:9e:40", vendor: "Dell Inc.", hostname: "pve" }),
    dev("Pi-hole 1", "192.168.68.31", "container", "linux", "wired", { sshUser: "root", mac: "bc:24:11:3a:55:01", vendor: "Proxmox Server Solutions GmbH", hostname: "pihole1" }),
    dev("Pi-hole 2", "192.168.68.32", "container", "linux", "wired", { mac: "bc:24:11:3a:55:02", vendor: "Proxmox Server Solutions GmbH", hostname: "pihole2" }),
    dev("Monitoring", "192.168.68.33", "container", "linux", "wired", { sshUser: "root", mac: "bc:24:11:3a:55:03", vendor: "Proxmox Server Solutions GmbH", hostname: "monitoring", notes: "Prometheus and Grafana" }),
    dev("Old laptop", "192.168.68.95", "pc", "linux", "gone", { sshUser: "sam", mac: "54:e1:ad:0f:be:77", vendor: "LCFC(HeFei) Electronics Technology co., ltd", hostname: "thinkpad", lastSeen: start - 30 * 3600 }),
    dev("Printer", "192.168.68.40", "other", "other", "wifi", { mac: "1c:2d:3e:4f:5a:6b", vendor: "Brother Industries, Ltd.", hostname: "BRW1C2D3E4F5A6B" }),
    dev("Living room TV", "192.168.68.41", "other", "other", "off", { mac: "8c:79:f5:21:d0:3a", vendor: "Samsung Electronics Co.,Ltd", hostname: "Samsung-TV", lastSeen: start - 18 * 60 }),
    dev("Sam's phone", "192.168.68.101", "other", "other", "flaky", { mac: "9a:3f:c2:71:0b:e4", hostname: "Pixel-8" }),
    dev("Tablet", "192.168.68.102", "other", "other", "wifi", { mac: "f2:7c:19:a4:66:d1" }),
    dev("192.168.68.117", "192.168.68.117", "other", "other", "fresh", { mac: "da:a1:19:5c:e0:27", reviewed: false, firstSeen: start - 150 }),
    dev("Galaxy-S24", "192.168.68.118", "other", "other", "fresh", { mac: "5e:0d:b3:92:4f:18", hostname: "Galaxy-S24", reviewed: false, firstSeen: start - 150 }),
    dev("Espressif Inc. device", "192.168.68.130", "other", "other", "fresh", { mac: "24:6f:28:4f:2a:1c", vendor: "Espressif Inc.", reviewed: false, firstSeen: start - 150 }),
    dev("Smart plug", "192.168.68.131", "ignore", "other", "wifi", { mac: "24:6f:28:9b:07:d3", vendor: "Espressif Inc.", hostname: "ESP_9B07D3" }),
    dev("Guest phone", "192.168.68.140", "ignore", "other", "gone", { mac: "b6:44:0e:c8:31:9f", lastSeen: start - 6 * DAY }),
  ];
  const idOf = (name: string) => devices.find((d) => d.name === name)?.id ?? -1;

  const settings: Settings = {
    pingIntervalSecs: 30,
    statsIntervalSecs: 60,
    scanRange: "",
    diskWarnPct: 85,
    ramWarnPct: 90,
    sshKeyPath: "",
  };
  const subnets: Subnet[] = [
    { cidr: "192.168.68.0/22", interface: "Ethernet", ownIp: "192.168.68.60", best: true },
    // On the same network by Wi-Fi as well as by cable: the same range comes back twice.
    { cidr: "192.168.68.0/22", interface: "Wi-Fi", ownIp: "192.168.68.61", best: false },
    { cidr: "172.28.160.0/20", interface: "vEthernet (WSL)", ownIp: "172.28.160.1", best: false },
  ];
  const secrets = new Set<string>([`ssh-password:${idOf("NAS")}`]);

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
    return { deviceId: d.id, up: last.up, rttMs: last.rttMs, checkedAt: last.ts, lastSeen: d.lastSeen, history };
  }

  function checkAll() {
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
      osName: "Windows 11",
      uptimeSecs: 2 * DAY + 5 * 3600,
      cpuPct: null,
      load1: null,
      cpuCount: 16,
      memTotalKb: kb(31.9),
      memUsedKb: kb(14.6),
      disks: [disk("C:", "", 930.5, 512.2), disk("D:", "", 1862.9, 1204.7)],
      reduced: true,
    }],
    [idOf("NAS"), {
      osName: "Microsoft Windows 11 Pro",
      uptimeSecs: 21 * DAY + 7 * 3600,
      cpuPct: 18,
      load1: null,
      cpuCount: 12,
      memTotalKb: kb(63.9),
      memUsedKb: kb(41.3),
      disks: [disk("C:", "System", 465.1, 201.4), disk("D:", "Media", 14901.2, 13872.6), disk("E:", "Backups", 7450.6, 4310.9)],
      reduced: false,
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
  ]);

  if (awkward) {
    // Two filesystems mounted on the same folder, both nearly full.
    goodStats.get(idOf("Homelab VM"))?.disks.push(disk("/mnt/data", "/dev/sdc1", 500, 471.5), disk("/mnt/data", "/dev/sdd1", 200, 188.2));
  }

  const entries = new Map<number, MachineStatsEntry>();
  const entry = (
    name: string,
    age: number | null,
    error: string | null = null,
    errorKind: StatsErrorKind | null = null,
  ) => {
    const deviceId = idOf(name);
    const stats = age == null ? null : (goodStats.get(deviceId) ?? null);
    entries.set(deviceId, { deviceId, stats, collectedAt: age == null ? null : start - age, error, errorKind });
  };
  entry("Main PC", 21);
  entry("NAS", 20);
  entry("Homelab VM", 19);
  entry("Dev VM", null);
  entry(
    "Proxmox",
    2 * 3600 + 400,
    "This machine's identity is not the one home-cc remembers. Check that it is really your machine before trusting it again.",
    "hostKeyChanged",
  );
  entry(
    "Pi-hole 1",
    null,
    "Couldn't log in as root. Tried the key file id_ed25519 and the ssh-agent; no password is stored for this machine.",
    "auth",
  );
  entry("Monitoring", 18);
  entry("Old laptop", 30 * 3600, "Couldn't reach 192.168.68.95: the connection timed out. Is the machine switched on?", "unreachable");
  /** Machines whose changed identity the owner has accepted. */
  const trusted = new Set<number>();

  function readStats(d: MockDevice): MachineStatsEntry {
    const prev = entries.get(d.id);
    const stats = goodStats.get(d.id);
    const blocked =
      (prev?.errorKind === "hostKeyChanged" && !trusted.has(d.id)) ||
      (prev?.errorKind === "auth" && !secrets.has(`ssh-password:${d.id}`)) ||
      d.pattern === "gone";
    let next: MachineStatsEntry;
    if (blocked && prev) {
      next = prev;
    } else if (stats) {
      const wobble = (n: number | null) => (n == null ? null : Math.max(1, Math.min(99, Math.round(n + (Math.random() - 0.5) * 8))));
      // A Linux machine has no CPU percentage on its first reading, then gets one.
      const cpuPct = stats.cpuPct ?? (prev?.stats && !stats.reduced && d.os !== "windows" ? 6 : null);
      const fresh = { ...stats, cpuPct: wobble(cpuPct), uptimeSecs: stats.uptimeSecs == null ? null : stats.uptimeSecs + (nowSecs() - start) };
      next = { deviceId: d.id, stats: fresh, collectedAt: nowSecs(), error: null, errorKind: null };
    } else {
      next = {
        deviceId: d.id,
        stats: null,
        collectedAt: null,
        error: `Couldn't reach ${d.host}: nothing answered on port ${d.sshPort}. Is SSH turned on there?`,
        errorKind: "unreachable",
      };
    }
    entries.set(d.id, next);
    return next;
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
        isSelf: d.name === "Main PC",
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
      checkAll();
      emit("status-changed");
    }, settings.pingIntervalSecs * 1000);
    statsTimer = setInterval(() => {
      for (const d of devices) {
        // The machine waiting for its first reading stays that way until it is refreshed by hand.
        const waiting = entries.get(d.id)?.stats == null && entries.get(d.id)?.error == null;
        if (d.sshUser && d.kind !== "ignore" && !waiting) readStats(d);
      }
      emit("stats-changed");
    }, settings.statsIntervalSecs * 1000);
  }
  startTimers();

  function checkInput(input: DeviceInput) {
    if (!input.name?.trim()) throw "Name can't be empty";
    if (!input.host?.trim()) throw "Address can't be empty";
    if (/\s/.test(input.host.trim())) throw "Address can't contain spaces";
    const mac = input.mac?.trim();
    if (mac && mac.replace(/[^0-9a-f]/gi, "").length !== 12) throw "Hardware address should look like aa:bb:cc:dd:ee:ff";
  }

  function normalMac(mac: string | null | undefined): string | null {
    const hex = (mac ?? "").replace(/[^0-9a-f]/gi, "").toLowerCase();
    return hex.length === 12 ? (hex.match(/../g) ?? []).join(":") : null;
  }

  const publicDevice = ({ pattern: _pattern, ...d }: MockDevice): Device => d;
  const find = (id: number): MockDevice => {
    const d = devices.find((x) => x.id === id);
    if (!d) throw `Not found: device ${id}`;
    return d;
  };

  // The arguments arrive untyped, as they do in Rust; each command says what it expects.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const commands: Record<string, (args: any) => unknown> = {
    app_info: () => ({ version: "0.1.0", os: "windows" }),
    list_devices: () => devices.map(publicDevice),
    add_device: ({ input }: { input: DeviceInput }) => {
      checkInput(input);
      const d = dev(input.name.trim(), input.host.trim(), input.kind, input.os, "wired", {
        sshUser: input.sshUser?.trim() || null,
        sshPort: input.sshPort ?? 22,
        notes: input.notes ?? "",
        mac: normalMac(input.mac),
        source: "manual",
        firstSeen: nowSecs(),
        lastSeen: null,
      });
      devices.push(d);
      if (d.sshUser) entries.set(d.id, { deviceId: d.id, stats: null, collectedAt: null, error: null, errorKind: null });
      return publicDevice(d);
    },
    update_device: ({ id, input }: { id: number; input: DeviceInput }) => {
      checkInput(input);
      const d = find(id);
      Object.assign(d, {
        name: input.name.trim(),
        host: input.host.trim(),
        kind: input.kind,
        os: input.os,
        sshUser: input.sshUser?.trim() || null,
        sshPort: input.sshPort ?? 22,
        notes: input.notes ?? "",
        mac: normalMac(input.mac) ?? d.mac,
        reviewed: true,
      });
      if (!d.sshUser) entries.delete(d.id);
      else if (!entries.has(d.id)) entries.set(d.id, { deviceId: d.id, stats: null, collectedAt: null, error: null, errorKind: null });
      return publicDevice(d);
    },
    remove_device: ({ id }: { id: number }) => {
      find(id);
      devices = devices.filter((d) => d.id !== id);
      histories.delete(id);
      entries.delete(id);
      secrets.delete(`ssh-password:${id}`);
    },
    set_secret: ({ key }: { key: string }) => void secrets.add(key),
    has_secret: ({ key }: { key: string }) => secrets.has(key),
    delete_secret: ({ key }: { key: string }) => void secrets.delete(key),

    get_settings: () => ({ ...settings }),
    save_settings: ({ settings: next }: { settings: Settings }) => {
      if (!(next.pingIntervalSecs >= 5 && next.pingIntervalSecs <= 3600)) throw "Ping timer must be between 5 seconds and 1 hour";
      if (!(next.statsIntervalSecs >= 10 && next.statsIntervalSecs <= 3600)) throw "Stats timer must be between 10 seconds and 1 hour";
      const okPct = (n: number) => n >= 1 && n <= 100;
      if (!okPct(next.diskWarnPct) || !okPct(next.ramWarnPct)) throw "Warning levels must be between 1 and 100 percent";
      Object.assign(settings, next, { scanRange: next.scanRange.trim(), sshKeyPath: next.sshKeyPath.trim() });
      startTimers();
      return { ...settings };
    },

    list_subnets: () => subnets,
    start_scan: () => startScan(),
    cancel_scan: () => void (scanCancelled = true),
    scan_running: () => scanTimer != null,

    device_statuses: () => devices.map(statusOf).filter((s) => s != null),
    check_now: async () => {
      await sleep(900);
      checkAll();
      emit("status-changed");
    },

    machine_stats: () => devices.filter((d) => d.sshUser && d.kind !== "ignore").map((d) => entries.get(d.id)).filter((e) => e != null),
    refresh_stats: async ({ deviceId }: { deviceId: number }) => {
      const d = find(deviceId);
      if (!d.sshUser) throw "This machine has no SSH user yet";
      await sleep(1100);
      return readStats(d);
    },
    forget_host_key: ({ deviceId }: { deviceId: number }) => void trusted.add(find(deviceId).id),
  };

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
