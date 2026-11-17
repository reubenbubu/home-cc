// The sample back end's Pi-holes, shaped like the owner's: "Pi-hole 1" and "Pi-hole 2", two containers on the
// Proxmox host, both set up over HTTPS. Pi-hole 1 has a blocklist Pi-hole 2 doesn't, and Pi-hole 2 has one
// switched off. Pausing counts down and blocking comes back by itself, as on a real Pi-hole.
//
// Extra words in the address show the states that wait for the owner:
//   ?phnone    no machine is a Pi-hole yet (the Add panel, with suggestions)
//   ?phcert    Pi-hole 2 is reached for the first time: its certificate must be approved
//   ?phchanged Pi-hole 2 shows another certificate than the one approved
//   ?phwrong   Pi-hole 2's stored password is wrong (until a new one is saved)
//   ?phdown    Pi-hole 2 doesn't answer
//   ?phnopw    Pi-hole 1 has no password stored yet
//   ?phoff     Pi-hole 1's blocking is paused for 5 minutes
import type { AdList, Blocking, PiholeSettings, PiholeStatus, Probe } from "../pihole/api";

const sleep = (msecs: number) => new Promise<void>((r) => setTimeout(r, msecs));
const nowSecs = () => Date.now() / 1000;

const PRINT_1 = "5C:21:9E:0B:7A:F4:13:88:D2:6E:4A:91:C0:3F:B7:58:1D:E6:92:4B:07:AC:35:F1:68:2E:D9:84:5B:C3:10:7E";
const PRINT_2 = "A7:3D:F0:62:9B:14:C8:5E:21:E9:7F:46:B3:0A:D5:8C:39:62:FE:17:AB:50:C4:2D:93:7B:E8:06:41:F5:CA:2F";
const PRINT_2_NEW = "0E:B4:57:C2:19:6A:FD:83:24:9E:71:D0:5B:C8:3F:A6:12:EB:47:90:6D:35:F8:C1:2A:7E:B5:04:D9:63:88:1C";

interface Context {
  find: (id: number) => { id: number; name: string; host: string; hostname: string | null; kind: string };
  devices: () => { id: number; name: string; kind: string }[];
  secrets: Set<string>;
  query: URLSearchParams;
}

const passwordKey = (id: number) => `pihole-password:${id}`;

const list = (address: string, domains: number, more: Partial<AdList> = {}): AdList => ({
  address,
  kind: "block",
  enabled: true,
  comment: null,
  domains,
  status: 2,
  ...more,
});

const SHARED: AdList[] = [
  list("https://raw.githubusercontent.com/StevenBlack/hosts/master/hosts", 81543, { comment: "Migrated from /etc/pihole/adlists.list" }),
  list("https://v.firebog.net/hosts/AdguardDNS.txt", 47120),
  list("https://v.firebog.net/hosts/Admiral.txt", 1352),
  list("https://raw.githubusercontent.com/anudeepND/blacklist/master/adservers.txt", 42536),
  list("https://v.firebog.net/hosts/Prigent-Ads.txt", 4270),
  list("https://raw.githubusercontent.com/PolishFiltersTeam/KADhosts/master/KADhosts.txt", 69844),
  list("https://raw.githubusercontent.com/FadeMind/hosts.extras/master/add.Spam/hosts", 57),
  list("https://raw.githubusercontent.com/anudeepND/whitelist/master/domains/whitelist.txt", 210, { kind: "allow" }),
];

interface Host {
  id: number;
  https: boolean;
  port: number;
  pinned: string | null;
  /** What the Pi-hole shows now. */
  shows: string;
  /** The certificate shown that waits for approval. */
  offered: string | null;
  /** The stored password is wrong until a new one is saved. */
  wrongPassword: boolean;
  down: boolean;
  /** Blocking off until this many Unix seconds (Infinity: until switched on); null: blocking. */
  pausedUntil: number | null;
  queries: number;
  blocked: number;
  clients: [number, number];
  lists: AdList[];
  version: string;
}

export function piholeMock(ctx: Context, ids: [number, number]) {
  const { query, secrets } = ctx;
  const start = nowSecs();
  const hosts = new Map<number, Host>();
  if (!query.has("phnone")) {
    const [one, two] = ids;
    hosts.set(one, {
      id: one,
      https: true,
      port: 443,
      pinned: PRINT_1,
      shows: PRINT_1,
      offered: null,
      wrongPassword: false,
      down: false,
      pausedUntil: query.has("phoff") ? start + 300 : null,
      queries: 48213,
      blocked: 9872,
      clients: [23, 31],
      lists: [
        ...SHARED,
        // Only on Pi-hole 1.
        list("https://raw.githubusercontent.com/hagezi/dns-blocklists/main/hosts/pro.txt", 168420, { comment: "Hagezi Pro" }),
        list("https://v.firebog.net/hosts/Easyprivacy.txt", 41322),
      ],
      version: "v6.1.4",
    });
    hosts.set(two, {
      id: two,
      https: true,
      port: 443,
      pinned: query.has("phcert") ? null : PRINT_2,
      shows: query.has("phchanged") ? PRINT_2_NEW : PRINT_2,
      offered: null,
      wrongPassword: query.has("phwrong"),
      down: query.has("phdown"),
      pausedUntil: null,
      queries: 21040,
      blocked: 4115,
      clients: [9, 14],
      // Easyprivacy is there but switched off, and typed with capitals in the host (still the same list). The
      // StevenBlack list ends in "/" here, which also counts as the same list.
      lists: [
        ...SHARED.map((l) => (l.address.includes("StevenBlack") ? { ...l, address: `${l.address}/` } : l)),
        list("https://V.Firebog.net/hosts/Easyprivacy.txt", 41322, { enabled: false }),
      ],
      version: "v6.1.4",
    });
    for (const h of hosts.values()) {
      if (!(h.id === one && query.has("phnopw"))) secrets.add(passwordKey(h.id));
    }
  }

  function get(id: number): Host {
    const d = ctx.find(id);
    const h = hosts.get(id);
    if (!h) throw `"${d.name}" isn't set up as a Pi-hole. Add it on the Pi-hole screen.`;
    return h;
  }

  const settings = (h: Host): PiholeSettings => ({
    deviceId: h.id,
    https: h.https,
    port: h.port,
    pinnedFingerprint: h.pinned,
    hasPassword: secrets.has(passwordKey(h.id)),
  });

  function blocking(h: Host): Blocking {
    const now = nowSecs();
    if (h.pausedUntil != null && h.pausedUntil <= now) h.pausedUntil = null;
    if (h.pausedUntil == null) return { state: "enabled", timerSecs: null };
    return { state: "disabled", timerSecs: Number.isFinite(h.pausedUntil) ? Math.round((h.pausedUntil - now) * 10) / 10 : null };
  }

  /** What a call finds wrong first, as the back end words it; null when the Pi-hole can be used. */
  function failure(h: Host, d: { host: string }): { kind: PiholeStatus["errorKind"]; error: string } | null {
    const place = `${d.host} on port ${h.port}`;
    if (!secrets.has(passwordKey(h.id))) {
      return { kind: "noPassword", error: "Add this Pi-hole's password: an app password (best) or the password of its web interface." };
    }
    if (h.down) {
      return { kind: "unreachable", error: `Nothing answered at ${place} within 5 seconds. Check that the Pi-hole is on and that the address and port are right.` };
    }
    if (h.https && h.pinned !== h.shows) {
      h.offered = h.shows;
      return h.pinned
        ? { kind: "certChanged", error: "This Pi-hole shows a different certificate from the one you approved, so home-cc sent it nothing. Check the new one before you approve it." }
        : { kind: "newCert", error: "First connection to this Pi-hole over HTTPS. Check its certificate before home-cc sends it the password." };
    }
    if (h.wrongPassword) {
      return {
        kind: "auth",
        error:
          "Pi-hole didn't accept the password. Check it, or better, use an app password. In the Pi-hole's web interface open Settings › Web interface / API, " +
          'switch the page to Expert (top right), and use "Configure app password". Copy the app password it shows (it is shown only once), press "Enable new app password", and paste it here.',
      };
    }
    return null;
  }

  let tick = 0;

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const commands: Record<string, (args: any) => unknown> = {
    pihole_hosts: () => [...hosts.values()].filter((h) => ctx.devices().some((d) => d.id === h.id)).map(settings),
    pihole_add: ({ deviceId }: { deviceId: number }) => {
      ctx.find(deviceId);
      if (!hosts.has(deviceId)) {
        hosts.set(deviceId, {
          id: deviceId,
          https: true,
          port: 443,
          pinned: null,
          shows: PRINT_2_NEW,
          offered: null,
          wrongPassword: false,
          down: false,
          pausedUntil: null,
          queries: 3120,
          blocked: 402,
          clients: [4, 4],
          lists: SHARED.slice(0, 3),
          version: "v6.1.4",
        });
      }
      return settings(get(deviceId));
    },
    pihole_remove: async ({ deviceId }: { deviceId: number }) => {
      get(deviceId);
      await sleep(200);
      hosts.delete(deviceId);
      secrets.delete(passwordKey(deviceId));
    },
    pihole_save_settings: async ({ deviceId, https, port, password }: { deviceId: number; https: boolean; port: number; password: string | null }) => {
      const h = get(deviceId);
      if (!(port >= 1 && port <= 65535)) throw "The port must be between 1 and 65535. Pi-hole uses 443 for HTTPS and 80 for HTTP.";
      await sleep(150);
      if (h.https !== https || h.port !== port) {
        h.pinned = null;
        h.offered = null;
      }
      h.https = https;
      h.port = port;
      if (password) {
        secrets.add(passwordKey(deviceId));
        h.wrongPassword = password === "wrong";
      }
      return settings(h);
    },
    pihole_remove_password: ({ deviceId }: { deviceId: number }) => {
      const h = get(deviceId);
      secrets.delete(passwordKey(deviceId));
      return settings(h);
    },
    pihole_status: async ({ deviceId }: { deviceId: number }): Promise<PiholeStatus> => {
      const h = get(deviceId);
      const d = ctx.find(deviceId);
      await sleep(h.down ? 600 : 220);
      tick++;
      const base = { deviceId, pinnedFingerprint: h.pinned, https: h.https, port: h.port, offeredFingerprint: null, expectedFingerprint: null };
      const f = failure(h, d);
      if (f) {
        const cert = f.kind === "newCert" || f.kind === "certChanged";
        return {
          ...base,
          reading: null,
          fetchedAt: null,
          error: f.error,
          errorKind: f.kind,
          offeredFingerprint: cert ? h.shows : null,
          expectedFingerprint: f.kind === "certChanged" ? h.pinned : null,
        };
      }
      h.offered = null;
      // A few more queries every reading, about a fifth of them blocked while blocking is on.
      const b = blocking(h);
      const more = 3 + (tick % 5);
      h.queries += more;
      if (b.state === "enabled" && tick % 4 === 0) h.blocked += 1;
      return {
        ...base,
        reading: {
          summary: {
            queries: h.queries,
            blocked: h.blocked,
            percentBlocked: (h.blocked * 100) / h.queries,
            clientsActive: h.clients[0],
            clientsTotal: h.clients[1],
            domainsOnLists: h.lists.filter((l) => l.enabled && l.kind === "block").reduce((n, l) => n + (l.domains ?? 0), 0) - 61210,
            gravityUpdated: Math.floor(start - 2 * 86400 - 3 * 3600),
          },
          blocking: b,
          lists: h.lists.map((l) => ({ ...l })),
          listsError: null,
          version: { core: h.version, web: "v6.2.1", ftl: "v6.2.3", docker: null },
        },
        fetchedAt: Math.floor(nowSecs()),
        error: null,
        errorKind: null,
      };
    },
    pihole_trust_certificate: ({ deviceId, fingerprint }: { deviceId: number; fingerprint: string }) => {
      const h = get(deviceId);
      if (!h.offered || h.offered !== fingerprint) {
        throw "This Pi-hole has shown a different certificate since you looked, or none. Check the one shown now before you approve it.";
      }
      h.pinned = h.offered;
      h.offered = null;
    },
    pihole_set_blocking: async ({ deviceId, blocking: on, timerSecs }: { deviceId: number; blocking: boolean; timerSecs: number | null }) => {
      const h = get(deviceId);
      const f = failure(h, ctx.find(deviceId));
      if (f) throw f.error;
      if (!on && timerSecs != null && (timerSecs < 1 || timerSecs > 86400)) {
        throw 'A pause can be at most a day. To keep blocking off for longer, choose "Until I turn it back on".';
      }
      await sleep(400);
      h.pausedUntil = on ? null : timerSecs == null ? Infinity : nowSecs() + timerSecs;
      return blocking(h);
    },
    pihole_probe: async ({ deviceIds }: { deviceIds: number[] }): Promise<Probe[]> => {
      if (deviceIds.length > 64) throw "home-cc looks for Pi-hole on at most 64 machines at a time.";
      await sleep(900);
      return deviceIds
        .map((id) => ctx.find(id))
        .filter((d) => d.kind !== "ignore")
        .map((d) => ({
          deviceId: d.id,
          // The Dev VM runs an old Pi-hole v5 to try things on.
          found: /pi-?hole/i.test(d.name) ? "v6" : d.name === "Dev VM" ? "v5" : null,
        }));
    },
  };
  return commands;
}
