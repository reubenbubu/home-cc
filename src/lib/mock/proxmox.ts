// The sample back end's Proxmox host, shaped like the owner's: one node running the Pi-holes and the
// monitoring container, a Home Assistant VM, the Homelab VM, a stopped test VM and a template.
//
// Extra words in the address show the states that wait for the owner:
//   ?pvecert     first contact: the certificate must be approved
//   ?pvechanged  the certificate isn't the approved one
//   ?pvenotoken  no API token yet
//   ?pvenosecret a token ID is saved, but its secret isn't in the keychain
//   ?pveperm     the token lacks permissions
//   ?pvestates   homeassistant is paused and win11-test is hibernated
//   ?pvenossh    the host has no SSH user, so consoles can't be opened (set in mock/index.ts)
import type {
  GuestRef,
  PowerAction,
  ProxmoxGuest,
  ProxmoxNode,
  ProxmoxSettings,
  ProxmoxSnapshot,
  ProxmoxStatus,
} from "../api";

const GB = 1024 ** 3;
const sleep = (msecs: number) => new Promise<void>((r) => setTimeout(r, msecs));
const nowSecs = () => Math.floor(Date.now() / 1000);

const PRINT_A = "3F:9A:61:0C:D2:7E:44:B8:15:C9:2A:E0:7B:56:D4:11:88:3C:F2:6A:09:BE:73:4D:E5:1F:A8:C2:60:97:3B:5E";
const PRINT_B = "B1:07:5C:E9:22:8F:D3:6A:41:CC:90:17:5E:A3:F8:2D:6B:04:E7:99:3A:C5:12:7F:80:DB:46:2E:B9:51:0D:F6";

interface Context {
  /** The device, or a thrown "Not found". */
  find: (id: number) => { id: number; name: string; os: string };
  secrets: Set<string>;
  query: URLSearchParams;
}

const tokenKey = (id: number) => `proxmox-token:${id}`;

export function proxmoxMock(ctx: Context, proxmoxId: number) {
  const { query, secrets } = ctx;
  const start = nowSecs();
  let tokenId: string | null = query.has("pvenotoken") ? null : "root@pam!homecc";
  let port = 8006;
  if (!query.has("pvenotoken") && !query.has("pvenosecret")) secrets.add(tokenKey(proxmoxId));
  /** The approved certificate, and the one the host shows. */
  let pinned: string | null = query.has("pvecert") ? null : PRINT_A;
  const shown = query.has("pvechanged") ? PRINT_B : PRINT_A;
  let offered: string | null = null;
  const noPermission = query.has("pveperm");

  const guest = (vmid: number, kind: "qemu" | "lxc", name: string, more: Partial<ProxmoxGuest>): ProxmoxGuest => ({
    vmid,
    kind,
    name,
    node: "pve",
    status: "running",
    template: false,
    lock: null,
    cpuPct: null,
    cpuCount: 1,
    memUsed: null,
    memTotal: null,
    diskUsed: null,
    diskTotal: null,
    uptimeSecs: null,
    tags: [],
    ...more,
  });

  /** Uptime is counted from this moment for each running guest; null when stopped. */
  const since = new Map<number, number | null>();
  const guests: ProxmoxGuest[] = [
    guest(101, "lxc", "pihole1", { cpuCount: 1, memTotal: 0.5 * GB, diskTotal: 4 * GB, tags: ["dns"] }),
    guest(102, "lxc", "pihole2", { cpuCount: 1, memTotal: 0.5 * GB, diskTotal: 4 * GB, tags: ["dns"] }),
    guest(103, "lxc", "monitoring", { cpuCount: 2, memTotal: 4 * GB, diskTotal: 32 * GB, tags: ["prometheus", "grafana"] }),
    guest(110, "qemu", "homeassistant", {
      status: query.has("pvestates") ? "paused" : "running",
      cpuCount: 2,
      memTotal: 4 * GB,
      diskTotal: 32 * GB,
      lock: query.has("awkward") ? "backup" : null,
    }),
    // The Homelab VM on the Machines screen (192.168.68.21) runs here.
    guest(111, "qemu", "homelab", { cpuCount: 4, memTotal: 8 * GB, diskTotal: 128 * GB, tags: ["docker"] }),
    guest(120, "qemu", "win11-test", {
      status: "stopped",
      cpuCount: 4,
      memTotal: 8 * GB,
      diskTotal: 64 * GB,
      lock: query.has("pvestates") ? "suspended" : null,
    }),
    guest(9000, "qemu", "debian-12-template", { status: "stopped", template: true, cpuCount: 2, memTotal: 2 * GB, diskTotal: 3 * GB }),
  ];
  const ages: Record<number, number> = { 101: 14 * 86400 + 3600, 102: 14 * 86400 + 3500, 103: 6 * 86400, 110: 2 * 86400 + 7200, 111: 9 * 86400 };
  /** What each guest reports as its address while it runs. win11-test has no guest agent. */
  const addresses: Record<number, string> = { 101: "192.168.68.31", 102: "192.168.68.32", 103: "192.168.68.33", 110: "192.168.68.35", 111: "192.168.68.21" };
  for (const g of guests) since.set(g.vmid, g.status !== "stopped" ? start - (ages[g.vmid] ?? 3600) : null);

  const snapshots = new Map<number, { list: ProxmoxSnapshot[]; current: string | null }>([
    [
      110,
      {
        list: [
          { name: "after_setup", description: "Fresh install with HACS", takenAt: start - 40 * 86400, parent: null, withRam: false },
          { name: "before_2026_11", description: "Before the November update", takenAt: start - 9 * 86400, parent: "after_setup", withRam: true },
        ],
        current: "before_2026_11",
      },
    ],
    [101, { list: [{ name: "clean", description: "", takenAt: start - 60 * 86400, parent: null, withRam: false }], current: "clean" }],
  ]);

  let tick = 0;
  /** A gentle wobble, the same on every load. */
  const wobble = (seed: number, size: number) => (Math.sin(tick * 0.9 + seed * 1.7) + 1) * 0.5 * size;

  function live(g: ProxmoxGuest): ProxmoxGuest {
    const from = since.get(g.vmid) ?? null;
    if (g.status !== "running" || from == null) {
      return { ...g, cpuPct: null, memUsed: null, diskUsed: null, uptimeSecs: null };
    }
    const base: Record<number, [number, number, number]> = {
      101: [0.6, 0.16, 1.2],
      102: [0.5, 0.15, 1.2],
      103: [6, 0.62, 11.5],
      110: [3, 0.55, 0],
    };
    const [cpu, mem, disk] = base[g.vmid] ?? [2, 0.4, 0];
    return {
      ...g,
      cpuPct: Math.round((cpu + wobble(g.vmid, cpu)) * 10) / 10,
      memUsed: Math.round((g.memTotal ?? 0) * (mem + wobble(g.vmid + 1, 0.04))),
      diskUsed: g.kind === "lxc" ? Math.round(disk * GB) : null,
      uptimeSecs: nowSecs() - from,
    };
  }

  function node(): ProxmoxNode {
    return {
      name: "pve",
      status: "online",
      cpuPct: Math.round((7 + wobble(1, 5)) * 10) / 10,
      cpuCount: 8,
      memUsed: Math.round((9.4 + wobble(2, 0.6)) * GB),
      memTotal: 31.2 * GB,
      diskUsed: 11.9 * GB,
      diskTotal: 93.9 * GB,
      uptimeSecs: nowSecs() - (start - 14 * 86400 - 5400),
      storages: [
        { name: "local", kind: "dir", content: ["iso", "vztmpl", "backup"], shared: false, active: true, used: 11.9 * GB, total: 93.9 * GB },
        { name: "local-lvm", kind: "lvmthin", content: ["rootdir", "images"], shared: false, active: true, used: 312 * GB, total: 348.8 * GB },
        { name: "nas-backup", kind: "cifs", content: ["backup"], shared: true, active: false, used: null, total: null },
      ],
    };
  }

  function check(deviceId: number) {
    const d = ctx.find(deviceId);
    if (d.os !== "proxmox") throw `"${d.name}" isn't set up as a Proxmox host. On the Machines screen, set its System to Proxmox VE.`;
  }

  function failure(): Omit<ProxmoxStatus, "deviceId" | "pinnedFingerprint"> | null {
    const empty = { overview: null, fetchedAt: null, offeredFingerprint: null, expectedFingerprint: null };
    if (!tokenId) return { ...empty, errorKind: "noToken", error: "Add an API token for this Proxmox host: its ID (such as root@pam!homecc) and its secret." };
    if (!secrets.has(tokenKey(proxmoxId))) {
      return { ...empty, errorKind: "noToken", error: "The API token's secret isn't stored. Paste it into the secret box and save." };
    }
    if (pinned !== shown) {
      offered = shown;
      return pinned
        ? {
            ...empty,
            errorKind: "certChanged",
            error: "This Proxmox host shows a different certificate from the one you approved, so home-cc sent it nothing. Check the new one before you approve it.",
            offeredFingerprint: shown,
            expectedFingerprint: pinned,
          }
        : {
            ...empty,
            errorKind: "newCert",
            error: "First connection to this Proxmox host. Check its certificate before home-cc sends it the API token.",
            offeredFingerprint: shown,
          };
    }
    if (noPermission) {
      return {
        ...empty,
        errorKind: "permission",
        error: permissionText("Permission check failed (/, Sys.Audit) (HTTP 403)"),
      };
    }
    return null;
  }

  function permissionText(why: string) {
    return (
      `Proxmox refused: ${why}. The token ${tokenId} is missing a permission. A token made with "Privilege Separation" ` +
      "ticked (the default) only gets the permissions given to the token itself, not its user's. Give it the PVEAuditor " +
      'role (to see nodes, storage and guests) and PVEVMAdmin (to start, stop and take snapshots) on "/": in Proxmox under ' +
      "Datacenter > Permissions > Add > API Token Permission, or run this in the Proxmox host's shell:\n" +
      `pveum acl modify / --tokens '${tokenId}' --roles PVEAuditor,PVEVMAdmin`
    );
  }

  /** What every action checks first, like the back end. */
  function ready(deviceId: number) {
    check(deviceId);
    const f = failure();
    if (f) throw f.error;
  }

  function find(ref: GuestRef): ProxmoxGuest {
    const g = guests.find((x) => x.vmid === ref.vmid && x.kind === ref.kind && x.node === ref.node);
    if (!g) throw `Configuration file 'nodes/${ref.node}/${ref.kind === "qemu" ? "qemu-server" : "lxc"}/${ref.vmid}.conf' does not exist (HTTP 500)`;
    return g;
  }

  function nameProblem(name: string): string | null {
    const n = name.trim();
    if (!/^[A-Za-z][A-Za-z0-9_-]{1,39}$/.test(n)) {
      return "A snapshot name must start with a letter and hold only letters, digits, - and _ (no spaces), 2 to 40 characters. For example: before_update";
    }
    if (n.toLowerCase() === "current") return '"current" is the name Proxmox uses for the present state. Pick another name.';
    return null;
  }

  const settings = (deviceId: number): ProxmoxSettings => ({
    deviceId,
    tokenId: deviceId === proxmoxId ? tokenId : null,
    port: deviceId === proxmoxId ? port : 8006,
    pinnedFingerprint: deviceId === proxmoxId ? pinned : null,
    hasSecret: secrets.has(tokenKey(deviceId)),
  });

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const commands: Record<string, (args: any) => unknown> = {
    proxmox_settings: ({ deviceId }: { deviceId: number }) => {
      ctx.find(deviceId);
      return settings(deviceId);
    },
    proxmox_save_settings: ({ deviceId, tokenId: id, port: p }: { deviceId: number; tokenId: string; port: number }) => {
      ctx.find(deviceId);
      const t = id.trim();
      if (t.includes("=")) throw "Put only the token ID here, and the secret in its own box. It looks like user@realm!name, for example root@pam!homecc: the user, the realm, then the token's own name.";
      if (t && !/^[^\s@!=]+@[^\s@!=]+![^\s@!=]+$/.test(t)) {
        throw "The token ID should look like user@realm!name, for example root@pam!homecc: the user, the realm, then the token's own name.";
      }
      if (!(p >= 1 && p <= 65535)) throw "The port must be between 1 and 65535. Proxmox uses 8006.";
      if (deviceId === proxmoxId) {
        tokenId = t || null;
        port = p;
      }
      return settings(deviceId);
    },
    proxmox_status: async ({ deviceId }: { deviceId: number }): Promise<ProxmoxStatus> => {
      check(deviceId);
      await sleep(250);
      tick++;
      const f = deviceId === proxmoxId ? failure() : null;
      const base = { deviceId, pinnedFingerprint: deviceId === proxmoxId ? pinned : null };
      if (deviceId !== proxmoxId) {
        return {
          ...base,
          overview: null,
          fetchedAt: null,
          error: "Couldn't connect to this host on port 8006: the connection was refused. The PC is there, but nothing is listening on that port: check the port (Proxmox uses 8006) and that this is the Proxmox PC.",
          errorKind: "unreachable",
          offeredFingerprint: null,
          expectedFingerprint: null,
        };
      }
      if (f) return { ...base, ...f };
      offered = null;
      return {
        ...base,
        overview: { nodes: [node()], guests: guests.map(live) },
        fetchedAt: nowSecs(),
        error: null,
        errorKind: null,
        offeredFingerprint: null,
        expectedFingerprint: null,
      };
    },
    proxmox_trust_certificate: ({ deviceId, fingerprint }: { deviceId: number; fingerprint: string }) => {
      check(deviceId);
      if (deviceId !== proxmoxId || !offered || offered !== fingerprint) {
        throw "This Proxmox host has shown a different certificate since you looked, or none. Check the one shown now before you approve it.";
      }
      pinned = offered;
      offered = null;
    },
    proxmox_power: async ({ deviceId, guest: ref, action }: { deviceId: number; guest: GuestRef; action: PowerAction }) => {
      ready(deviceId);
      const g = find(ref);
      if (g.template) throw `Proxmox tried to ${action} VM ${g.vmid}, but it failed: you can't start a vm if it's a template`;
      // Proxmox lets a hibernated VM ("suspended" lock) start: that is how it is resumed.
      if (g.lock && !(action === "start" && g.lock === "suspended")) throw `VM ${g.vmid} is locked (${g.lock}) (HTTP 500)`;
      if ((action === "shutdown" || action === "reboot") && g.status === "paused") throw `VM ${g.vmid} is paused - cannot ${action} (HTTP 500)`;
      await sleep(action === "shutdown" ? 2200 : 1400);
      if (action === "start") {
        g.status = "running";
        g.lock = null;
        since.set(g.vmid, nowSecs());
      } else if (action === "resume") {
        g.status = "running";
      } else if (action === "reboot") {
        since.set(g.vmid, nowSecs());
      } else {
        g.status = "stopped";
        since.set(g.vmid, null);
      }
      return { finished: true };
    },
    proxmox_guest_addresses: async ({ deviceId, guest: ref }: { deviceId: number; guest: GuestRef }) => {
      ready(deviceId);
      const g = find(ref);
      await sleep(200);
      if (g.status !== "running") {
        if (g.kind === "lxc") return { addresses: [], unknownWhy: null };
        throw "VM " + g.vmid + " is not running (HTTP 500)";
      }
      const ip = addresses[g.vmid];
      if (!ip) return { addresses: [], unknownWhy: "Proxmox can't tell this VM's address: the QEMU guest agent isn't running in it." };
      return { addresses: [ip], unknownWhy: null };
    },
    proxmox_snapshots: async ({ deviceId, guest: ref }: { deviceId: number; guest: GuestRef }) => {
      ready(deviceId);
      find(ref);
      await sleep(300);
      const s = snapshots.get(ref.vmid) ?? { list: [], current: null };
      return { snapshots: s.list.map((x) => ({ ...x })), currentParent: s.current };
    },
    proxmox_take_snapshot: async ({ deviceId, guest: ref, name, description, withRam }: { deviceId: number; guest: GuestRef; name: string; description: string; withRam: boolean }) => {
      ready(deviceId);
      const g = find(ref);
      const problem = nameProblem(name);
      if (problem) throw problem;
      const s = snapshots.get(g.vmid) ?? { list: [], current: null };
      if (s.list.some((x) => x.name === name.trim())) {
        throw `Proxmox tried to take snapshot "${name.trim()}" of ${ref.kind === "qemu" ? "VM" : "container"} ${g.vmid}, but it failed: snapshot name '${name.trim()}' already used`;
      }
      await sleep(withRam ? 2500 : 1500);
      s.list.push({ name: name.trim(), description: description.trim(), takenAt: nowSecs(), parent: s.current, withRam: withRam && g.kind === "qemu" && g.status === "running" });
      s.current = name.trim();
      snapshots.set(g.vmid, s);
      return { finished: true };
    },
    proxmox_rollback: async ({ deviceId, guest: ref, name, startAfter }: { deviceId: number; guest: GuestRef; name: string; startAfter: boolean }) => {
      ready(deviceId);
      const g = find(ref);
      const s = snapshots.get(g.vmid);
      const snap = s?.list.find((x) => x.name === name);
      if (!s || !snap) throw `snapshot '${name}' does not exist (HTTP 500)`;
      await sleep(1800);
      s.current = name;
      // As Proxmox does: a snapshot with RAM comes back running; without, a running guest is stopped to
      // roll back and left stopped.
      if (snap.withRam) {
        g.status = "running";
        since.set(g.vmid, nowSecs());
      } else {
        g.status = "stopped";
        since.set(g.vmid, null);
        if (startAfter) {
          await sleep(1200);
          g.status = "running";
          since.set(g.vmid, nowSecs());
        }
      }
      return { finished: true };
    },
    proxmox_delete_snapshot: async ({ deviceId, guest: ref, name }: { deviceId: number; guest: GuestRef; name: string }) => {
      ready(deviceId);
      const g = find(ref);
      const s = snapshots.get(g.vmid);
      if (!s || !s.list.some((x) => x.name === name)) throw `snapshot '${name}' does not exist (HTTP 500)`;
      await sleep(1200);
      const gone = s.list.find((x) => x.name === name);
      s.list = s.list.filter((x) => x.name !== name).map((x) => (x.parent === name ? { ...x, parent: gone?.parent ?? null } : x));
      if (s.current === name) s.current = gone?.parent ?? null;
      return { finished: true };
    },
  };
  return commands;
}
