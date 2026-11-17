// Sample Docker hosts for the browser (see index.ts): the homelab VM with several compose stacks, the
// monitoring container on the Proxmox PC, and a dev VM whose user isn't in the docker group yet. For image
// updates (REGISTRY below), the homelab has newer images in two stacks and for one container outside any stack,
// an image built on the host, and a private registry that wants a login; the monitoring host is up to date.
import { emit } from "@tauri-apps/api/event";
import type { Device } from "../api";
import type {
  CleanupItem,
  CleanupPick,
  CleanupPreview,
  Container,
  ContainerAction,
  ContainerStats,
  DockerHost,
  HostMode,
  ImageUpdate,
  Port,
  StackAction,
  UpdateCheck,
  UpdateState,
} from "../docker/api";

const sleep = (msecs: number) => new Promise<void>((r) => setTimeout(r, msecs));
const nowSecs = () => Math.floor(Date.now() / 1000);
const MB = 1000 ** 2;
const GB = 1000 ** 3;
const MiB = 1024 ** 2;

interface Template {
  service: string;
  image: string;
  ports?: Port[];
  /** How it is doing in the sample. */
  state?: "running" | "exited" | "restarting" | "created";
  health?: string | null;
  exitCode?: number | null;
  cpu?: number;
  memMiB?: number;
  /** Seconds since it started (or stopped). */
  age?: number;
}

interface MockStack {
  name: string;
  dir: string;
  files: Record<string, string>;
  templates: Template[];
}

const tcp = (host: number, inside = host): Port => ({ hostPort: host, hostIp: null, containerPort: inside, protocol: "tcp" });
const udp = (host: number, inside = host): Port => ({ hostPort: host, hostIp: null, containerPort: inside, protocol: "udp" });
const exposed = (inside: number): Port => ({ hostPort: null, hostIp: null, containerPort: inside, protocol: "tcp" });

const COMPOSE_MEDIA = `services:
  jellyfin:
    image: jellyfin/jellyfin:10.10
    restart: unless-stopped
    ports:
      - "8096:8096"
    volumes:
      - ./config:/config
      - /mnt/media:/media:ro

  sonarr:
    image: lscr.io/linuxserver/sonarr:latest
    restart: unless-stopped
    env_file: .env
    ports:
      - "8989:8989"

  radarr:
    image: lscr.io/linuxserver/radarr:latest
    restart: unless-stopped
    env_file: .env
    ports:
      - "7878:7878"

  prowlarr:
    image: lscr.io/linuxserver/prowlarr:latest
    restart: "no"
    ports:
      - "9696:9696"
`;

const COMPOSE_HA = `services:
  homeassistant:
    image: ghcr.io/home-assistant/home-assistant:stable
    network_mode: host
    restart: unless-stopped
    volumes:
      - ./ha:/config

  mosquitto:
    image: eclipse-mosquitto:2
    ports:
      - "1883:1883"

  zigbee2mqtt:
    image: koenkk/zigbee2mqtt:latest
    devices:
      - /dev/ttyUSB0:/dev/ttyACM0
    depends_on: [mosquitto]
`;

const COMPOSE_PHOTOS = `name: photos
services:
  immich-server:
    image: ghcr.io/immich-app/immich-server:release
    ports:
      - "2283:2283"
    env_file: .env
    depends_on: [redis, database]

  immich-machine-learning:
    image: ghcr.io/immich-app/immich-machine-learning:release

  redis:
    image: redis:7-alpine

  database:
    image: tensorchord/pgvecto-rs:pg16-v0.2.0
    volumes:
      - db:/var/lib/postgresql/data

volumes:
  db:
`;

const COMPOSE_MONITORING = `services:
  prometheus:
    image: prom/prometheus:v3.0.0
    ports: ["9090:9090"]
    volumes:
      - ./prometheus.yml:/etc/prometheus/prometheus.yml:ro
  grafana:
    image: grafana/grafana:11.3.0
    ports: ["3000:3000"]
  node-exporter:
    image: prom/node-exporter:v1.8.2
    ports: ["9100:9100"]
`;

function homelabStacks(): MockStack[] {
  return [
    {
      name: "home-assistant",
      dir: "/srv/home-assistant",
      files: { "/srv/home-assistant/compose.yml": COMPOSE_HA },
      templates: [
        { service: "homeassistant", image: "ghcr.io/home-assistant/home-assistant:stable", cpu: 3.2, memMiB: 612, age: 9 * 86400 },
        { service: "mosquitto", image: "eclipse-mosquitto:2", ports: [tcp(1883)], cpu: 0.1, memMiB: 9, age: 9 * 86400 },
        { service: "zigbee2mqtt", image: "koenkk/zigbee2mqtt:latest", state: "restarting", exitCode: 1, age: 40 },
      ],
    },
    {
      name: "media",
      dir: "/srv/media",
      files: {
        "/srv/media/compose.yml": COMPOSE_MEDIA,
        "/srv/media/.env": "PUID=1000\nPGID=1000\nTZ=Europe/London\n",
      },
      templates: [
        { service: "jellyfin", image: "jellyfin/jellyfin:10.10", ports: [tcp(8096), { hostPort: 1900, hostIp: "192.168.68.21", containerPort: 1900, protocol: "udp" }, exposed(8920)], health: "healthy", cpu: 12.4, memMiB: 1430, age: 4 * 86400 + 3 * 3600 },
        { service: "sonarr", image: "lscr.io/linuxserver/sonarr:latest", ports: [tcp(8989)], cpu: 0.6, memMiB: 248, age: 4 * 86400 + 3 * 3600 },
        { service: "radarr", image: "lscr.io/linuxserver/radarr:latest", ports: [tcp(7878)], cpu: 0.4, memMiB: 231, age: 4 * 86400 + 3 * 3600 },
        { service: "prowlarr", image: "lscr.io/linuxserver/prowlarr:latest", ports: [tcp(9696)], state: "exited", exitCode: 0, age: 2 * 3600 },
      ],
    },
    {
      name: "nextcloud",
      dir: "/srv/nextcloud",
      files: { "/srv/nextcloud/compose.yml": "services:\n  app:\n    image: nextcloud:30\n    ports:\n      - \"8080:80\"\n" },
      // Down: remembered from an earlier look, with no containers now.
      templates: [],
    },
    {
      name: "photos",
      dir: "/srv/photos",
      files: {
        "/srv/photos/docker-compose.yml": COMPOSE_PHOTOS,
        "/srv/photos/.env": "UPLOAD_LOCATION=/mnt/photos\nDB_PASSWORD=sample-only\n",
      },
      templates: [
        { service: "immich-server", image: "ghcr.io/immich-app/immich-server:release", ports: [tcp(2283)], health: "healthy", cpu: 1.8, memMiB: 702, age: 6 * 86400 },
        { service: "immich-machine-learning", image: "ghcr.io/immich-app/immich-machine-learning:release", health: "healthy", cpu: 0.2, memMiB: 1210, age: 6 * 86400 },
        { service: "redis", image: "redis:7-alpine", health: "healthy", cpu: 0.3, memMiB: 12, age: 6 * 86400 },
        { service: "database", image: "tensorchord/pgvecto-rs:pg16-v0.2.0", ports: [exposed(5432)], health: "healthy", cpu: 0.9, memMiB: 318, age: 6 * 86400 },
      ],
    },
  ];
}

/** What the sample registries say about each image, compared with what the hosts pulled. Images not listed are
 *  up to date. */
type RegistrySays = { newer: true } | { builtHere: true } | { error: UpdateState; message: string };
const REGISTRY: Record<string, RegistrySays> = {
  "lscr.io/linuxserver/sonarr:latest": { newer: true },
  "lscr.io/linuxserver/radarr:latest": { newer: true },
  // Stopped: updating it would start it, so it starts unticked.
  "lscr.io/linuxserver/prowlarr:latest": { newer: true },
  "ghcr.io/immich-app/immich-server:release": { newer: true },
  "ghcr.io/immich-app/immich-machine-learning:release": { newer: true },
  "lscr.io/linuxserver/speedtest-tracker:latest": { newer: true },
  "homepage-custom:latest": { builtHere: true },
  "registry.home.lan/sam/nas-backup:1.4": {
    error: "needsLogin",
    message: "ERROR: unexpected status from HEAD request to https://registry.home.lan/v2/sam/nas-backup/manifests/1.4: 401 Unauthorized",
  },
};

/** The repository part of an image name, as RepoDigests write it. */
const repoOf = (image: string) => image.replace(/:[^:/]+$/, "");

function hexId(seed: number): string {
  let s = seed * 2654435761;
  let out = "";
  while (out.length < 64) {
    s = (s * 1103515245 + 12345) >>> 0;
    out += s.toString(16).padStart(8, "0");
  }
  return out.slice(0, 64);
}

const LOG_LINES = [
  "[INFO] Request GET /System/Info/Public 200 in 3ms",
  "[INFO] Scheduled task \"Scan media library\" completed after 0 minute(s) and 4 seconds",
  "[WARN] Slow response from TMDB: 2.4 s",
  "[INFO] WebSocket connected: Living room TV",
  "[ERR] Failed to download artwork: connection reset by peer",
  "[INFO] Playback started: \"Big Buck Bunny\" on Living room TV",
  "[INFO] Transcoding finished in 41 s",
];

interface Ctx {
  devices: () => (Device & { pattern?: string })[];
  idOf: (name: string) => number;
}

export function dockerMock({ devices, idOf }: Ctx) {
  let nextSeed = 1;
  const hostState = new Map<number, { mode: HostMode; found: boolean | null; version: string | null; compose: DockerHost["compose"]; checkedAt: number | null; problem: DockerHost["problem"] }>();
  const homelab = idOf("Homelab VM");
  const monitoring = idOf("Monitoring");
  const dev = idOf("Dev VM");

  /** Containers and stacks per host. */
  const stacks = new Map<number, MockStack[]>([
    [homelab, homelabStacks()],
    [monitoring, [{ name: "monitoring", dir: "/opt/monitoring", files: { "/opt/monitoring/compose.yml": COMPOSE_MONITORING, "/opt/monitoring/prometheus.yml": "global:\n  scrape_interval: 30s\nscrape_configs:\n  - job_name: node\n    static_configs:\n      - targets: ['192.168.68.21:9100', '192.168.68.22:9100']\n" }, templates: [
      { service: "prometheus", image: "prom/prometheus:v3.0.0", ports: [tcp(9090)], cpu: 2.1, memMiB: 410, age: 21 * 86400 },
      { service: "grafana", image: "grafana/grafana:11.3.0", ports: [tcp(3000)], health: "healthy", cpu: 0.4, memMiB: 96, age: 21 * 86400 },
      { service: "node-exporter", image: "prom/node-exporter:v1.8.2", ports: [tcp(9100)], cpu: 0.1, memMiB: 14, age: 21 * 86400 },
    ] }]],
  ]);
  const containers = new Map<number, Container[]>();
  const statsOf = new Map<string, { cpu: number; memMiB: number }>();

  // Images on the hosts: by local id, the registry digest each came with (none for one built on the host), and
  // which local image each name points at now, per host. The registries have the digest pulled, or a newer one.
  let nextImage = 500;
  const localImages = new Map<string, { image: string; digest: string | null }>();
  const tagNow = new Map<string, string>();
  /** A digest that looks like one, always the same for the same text. */
  const digestOf = (text: string) => {
    let h = 2166136261;
    for (const ch of text) h = Math.imul(h ^ ch.charCodeAt(0), 16777619) >>> 0;
    let out = "";
    while (out.length < 64) {
      h ^= h << 13;
      h ^= h >>> 17;
      h ^= h << 5;
      h >>>= 0;
      out += h.toString(16).padStart(8, "0");
    }
    return `sha256:${out}`;
  };
  const pulledDigest = (image: string) => digestOf(`${image} as pulled`);
  const latestDigest = (image: string) => {
    const says = REGISTRY[image];
    return says && "newer" in says ? digestOf(`${image} now`) : pulledDigest(image);
  };
  /** A new local image for the name on a host, with this digest, which the name now points at. */
  function addImage(hostId: number, image: string, digest: string | null): string {
    const id = `sha256:${hexId(nextImage++)}`;
    localImages.set(id, { image, digest });
    tagNow.set(`${hostId}\n${image}`, id);
    return id;
  }
  /** The local image a name points at on a host, pulled (or built) when the sample began. */
  function imageFor(hostId: number, image: string): string {
    const known = tagNow.get(`${hostId}\n${image}`);
    if (known) return known;
    const says = REGISTRY[image];
    return addImage(hostId, image, says && "builtHere" in says ? null : pulledDigest(image));
  }

  function make(hostId: number, stack: string | null, t: Template, prefix = stack): Container {
    const id = hexId(nextSeed++);
    const state = t.state ?? "running";
    const age = t.age ?? 3600;
    statsOf.set(id, { cpu: t.cpu ?? 0.2, memMiB: t.memMiB ?? 40 });
    return {
      id,
      name: prefix ? `${prefix}-${t.service}-1` : t.service,
      image: t.image,
      imageId: imageFor(hostId, t.image),
      state,
      health: state === "running" ? (t.health ?? null) : null,
      exitCode: state === "running" ? null : (t.exitCode ?? 0),
      startedAt: state === "created" ? null : nowSecs() - (state === "running" ? age : age + 86400),
      finishedAt: state === "exited" ? nowSecs() - age : null,
      ports: state === "running" ? (t.ports ?? []) : [],
      project: stack,
      service: stack ? t.service : null,
    };
  }

  for (const [hostId, list] of stacks) {
    const all: Container[] = [];
    for (const s of list) for (const t of s.templates) all.push(make(hostId, s.name, t));
    containers.set(hostId, all);
  }
  containers.get(homelab)?.push(
    make(homelab, null, { service: "speedtest-tracker", image: "lscr.io/linuxserver/speedtest-tracker:latest", ports: [tcp(8765, 80)], cpu: 0.1, memMiB: 88, age: 30 * 86400 }),
    make(homelab, null, { service: "old-backup", image: "alpine:3.20", state: "exited", exitCode: 137, age: 12 * 86400 }),
    make(homelab, null, { service: "nas-backup", image: "registry.home.lan/sam/nas-backup:1.4", cpu: 0.1, memMiB: 22, age: 15 * 86400 }),
    make(homelab, null, { service: "dashboard", image: "homepage-custom:latest", ports: [tcp(3001, 3000)], cpu: 0.3, memMiB: 64, age: 15 * 86400 }),
  );

  // Update checks: what the sample registries say, compared with what each host runs.
  const checks = new Map<number, UpdateCheck>();
  function compareImages(hostId: number): ImageUpdate[] {
    const pairs = new Map<string, { image: string; imageId: string }>();
    for (const c of containers.get(hostId) ?? []) if (c.imageId) pairs.set(`${c.image}\n${c.imageId}`, { image: c.image, imageId: c.imageId });
    return [...pairs.values()].map(({ image, imageId }): ImageUpdate => {
      const local = localImages.get(imageId);
      const says = REGISTRY[image];
      const base = { image, imageId, current: local?.digest ?? null, latest: null, downloaded: false, message: null };
      if (!local?.digest) return { ...base, state: "builtHere" };
      if (says && "error" in says) return { ...base, state: says.error, message: says.message };
      const latest = latestDigest(image);
      if (local.digest === latest) return { ...base, state: "upToDate", latest };
      const named = tagNow.get(`${hostId}\n${image}`);
      const downloaded = named != null && named !== imageId && localImages.get(named)?.digest === latest;
      return { ...base, state: "available", latest, downloaded };
    });
  }

  // Every machine that may be a Docker host: Linux and Proxmox machines with an SSH user.
  const candidates = () => devices().filter((d) => d.kind !== "ignore" && d.os !== "windows" && d.sshUser);
  const stateFor = (id: number) => {
    let s = hostState.get(id);
    if (!s) {
      s = { mode: "auto", found: null, version: null, compose: "none", checkedAt: null, problem: null };
      hostState.set(id, s);
    }
    return s;
  };
  // Looked at before this session: the homelab VM and the monitoring container have Docker, Proxmox doesn't.
  Object.assign(stateFor(homelab), { found: true, version: "27.3.1", compose: "plugin", checkedAt: nowSecs() - 3 * 86400 });
  Object.assign(stateFor(monitoring), { found: true, version: "26.1.4", compose: "plugin", checkedAt: nowSecs() - 20 * 86400 });
  Object.assign(stateFor(idOf("Proxmox")), { found: false, compose: "none", checkedAt: nowSecs() - 3600 });

  function hostOf(id: number): DockerHost {
    const s = stateFor(id);
    const loginProblem = s.problem && ["newHost", "hostKeyChanged", "auth"].includes(s.problem.kind);
    const recheck = s.found === false && (s.checkedAt == null || nowSecs() - s.checkedAt > 86400);
    return {
      deviceId: id,
      ...s,
      needsCheck: s.mode !== "off" && !loginProblem && (s.found === null || recheck),
    };
  }

  function device(id: number) {
    const d = devices().find((x) => x.id === id);
    if (!d) throw `Not found: device ${id}`;
    if (d.os === "windows") throw `${d.name} is a Windows machine. home-cc manages Docker on Linux machines only.`;
    if (!d.sshUser) throw `Add an SSH user to ${d.name} first, on the Machines screen.`;
    return d;
  }

  /** What the sample machines say when they are looked at. */
  async function look(id: number): Promise<DockerHost> {
    const d = device(id);
    const s = stateFor(id);
    await sleep(700);
    if (id === dev) {
      Object.assign(s, {
        found: true,
        version: null,
        compose: "plugin",
        checkedAt: nowSecs(),
        problem: {
          kind: "permission",
          message: `Docker is installed on ${d.name}, but the user ${d.sshUser} isn't allowed to use it. On ${d.name}, run: sudo usermod -aG docker ${d.sshUser} — then press Try again here (home-cc logs in afresh, which picks up the new group).`,
        },
      });
    } else if (d.pattern === "gone") {
      s.problem = { kind: "unreachable", message: `${d.host} did not answer. Check that it is switched on and on the network.` };
    } else if (d.name === "Pi-hole 1") {
      s.problem = {
        kind: "newHost",
        message: "The stored password has not been sent: this machine's identity needs your approval first. Approve it on the Machines screen.",
      };
    } else if (!stacks.has(id)) {
      Object.assign(s, { found: false, version: null, compose: "none", checkedAt: nowSecs(), problem: { kind: "notInstalled", message: `There is no docker command on ${d.name}.` } });
    } else {
      Object.assign(s, { found: true, checkedAt: nowSecs(), problem: null });
    }
    return hostOf(id);
  }

  function overview(id: number) {
    const all = containers.get(id) ?? [];
    const list = stacks.get(id) ?? [];
    return {
      deviceId: id,
      readAt: nowSecs(),
      stacks: list.map((s) => ({
        name: s.name,
        workingDir: s.dir,
        configFiles: Object.keys(s.files).filter((f) => /compose/.test(f)),
        containers: all.filter((c) => c.project === s.name),
      })),
      others: all.filter((c) => c.project == null),
    };
  }

  function findContainer(hostId: number, containerId: string): Container {
    const c = containers.get(hostId)?.find((x) => x.id === containerId);
    if (!c) throw "Error response from daemon: No such container: " + containerId.slice(0, 12);
    return c;
  }

  function stackOf(hostId: number, name: string): MockStack {
    const s = stacks.get(hostId)?.find((x) => x.name === name);
    if (!s) throw `home-cc doesn't know a stack called ${name}. Refresh the list.`;
    return s;
  }

  // Cleanup: what each host keeps that nothing uses.
  const leftovers = new Map<number, CleanupPreview>([
    [
      homelab,
      {
        containers: [],
        images: [
          { key: "jellyfin/jellyfin:10.9", label: "jellyfin/jellyfin:10.9", detail: "Made 3 months ago", sizeBytes: 1.31 * GB },
          { key: "lscr.io/linuxserver/sonarr:4.0.9", label: "lscr.io/linuxserver/sonarr:4.0.9", detail: "Made 2 months ago", sizeBytes: 212 * MB },
          { key: "sha256:" + hexId(900), label: `Untagged image ${hexId(900).slice(0, 12)}`, detail: "Made 5 weeks ago", sizeBytes: 640 * MB },
          { key: "sha256:" + hexId(901), label: `Untagged image ${hexId(901).slice(0, 12)}`, detail: "Made 5 weeks ago", sizeBytes: 15.7 * 1000 },
          { key: "postgres:14", label: "postgres:14", detail: "Made 8 months ago", sizeBytes: 422 * MB },
        ],
        volumes: [
          { key: "nextcloud_db", label: "nextcloud_db", detail: "Named volume of the nextcloud stack", sizeBytes: 2.4 * GB },
          { key: hexId(950), label: `Unnamed volume ${hexId(950).slice(0, 12)}`, detail: "Unnamed volume", sizeBytes: 0 },
          { key: hexId(951), label: `Unnamed volume ${hexId(951).slice(0, 12)}`, detail: "Unnamed volume", sizeBytes: 48.2 * MB },
        ],
        buildCacheCount: 37,
        buildCacheBytes: 1.86 * GB,
      },
    ],
  ]);

  function preview(id: number): CleanupPreview {
    const base = leftovers.get(id) ?? { containers: [], images: [], volumes: [], buildCacheCount: 0, buildCacheBytes: 0 };
    const stopped: CleanupItem[] = (containers.get(id) ?? [])
      .filter((c) => c.state === "exited" || c.state === "created")
      .map((c) => ({ key: c.id, label: c.name, detail: `${c.image} · Exited (${c.exitCode ?? 0})`, sizeBytes: 64 * 1000 }));
    return { ...base, containers: stopped };
  }

  // Followed logs.
  const streams = new Map<number, ReturnType<typeof setInterval>>();
  let nextStream = 0;
  const stamp = (msAgo: number) => new Date(Date.now() - msAgo).toISOString().replace("Z", "123456Z");

  // Files as saved, with what the owner changed.
  return {
    docker_hosts: () => candidates().map((d) => hostOf(d.id)),
    docker_probe: ({ deviceId }: { deviceId: number }) => look(deviceId),
    docker_set_host_mode: ({ deviceId, mode }: { deviceId: number; mode: HostMode }) => {
      device(deviceId);
      stateFor(deviceId).mode = mode;
      return hostOf(deviceId);
    },
    docker_overview: async ({ deviceId, auto }: { deviceId: number; auto: boolean }) => {
      device(deviceId);
      const s = stateFor(deviceId);
      if (auto && s.problem && ["newHost", "hostKeyChanged", "auth"].includes(s.problem.kind)) return { host: hostOf(deviceId), overview: null };
      await sleep(350);
      if (deviceId === dev || s.found !== true) {
        await look(deviceId);
        return { host: hostOf(deviceId), overview: stacks.has(deviceId) ? overview(deviceId) : null };
      }
      s.problem = null;
      return { host: hostOf(deviceId), overview: overview(deviceId) };
    },
    docker_stats: async ({ deviceId }: { deviceId: number }): Promise<ContainerStats[]> => {
      await sleep(600);
      return (containers.get(deviceId) ?? [])
        .filter((c) => c.state === "running")
        .map((c) => {
          const s = statsOf.get(c.id) ?? { cpu: 0.1, memMiB: 20 };
          return { id: c.id, cpuPct: s.cpu, memUsedBytes: s.memMiB * MiB, memLimitBytes: 15.6 * 1024 * MiB };
        });
    },
    docker_container_action: async ({ deviceId, containerId, action }: { deviceId: number; containerId: string; action: ContainerAction }) => {
      const c = findContainer(deviceId, containerId);
      await sleep(action === "stop" || action === "restart" ? 1500 : 700);
      if (action === "remove") {
        containers.set(deviceId, (containers.get(deviceId) ?? []).filter((x) => x.id !== c.id));
      } else if (action === "stop") {
        Object.assign(c, { state: "exited", exitCode: 0, finishedAt: nowSecs(), health: null, ports: [] });
      } else {
        if (c.service === "zigbee2mqtt") throw "Error response from daemon: error gathering device information while adding custom device \"/dev/ttyUSB0\": no such file or directory";
        const template = stacks.get(deviceId)?.flatMap((s) => s.templates).find((t) => t.service === c.service);
        Object.assign(c, { state: "running", exitCode: null, startedAt: nowSecs(), finishedAt: null, ports: template?.ports ?? c.ports, health: template?.health ? "starting" : null });
      }
      return { output: c.id.slice(0, 12) };
    },
    docker_stack_action: async ({ deviceId, stack, action }: { deviceId: number; stack: string; action: StackAction }) => {
      const s = stackOf(deviceId, stack);
      await sleep(action === "pull" || action === "build" ? 3000 : 1500);
      const others = (containers.get(deviceId) ?? []).filter((c) => c.project !== stack);
      const mine = (containers.get(deviceId) ?? []).filter((c) => c.project === stack);
      if (action === "down") {
        containers.set(deviceId, others);
        return { output: mine.map((c) => ` Container ${c.name}  Removed`).join("\n") + `\n Network ${stack}_default  Removed` };
      }
      const templates = s.templates.length ? s.templates : [{ service: "app", image: "nextcloud:30", ports: [tcp(8080, 80)] }];
      if (!s.templates.length) s.templates = templates;
      if (action === "pull") for (const t of templates) addImage(deviceId, t.image, latestDigest(t.image));
      const fresh = templates.map((t) => make(deviceId, stack, { ...t, state: "running", age: 1, exitCode: null }));
      containers.set(deviceId, [...others, ...fresh]);
      const pulled = action === "pull" ? templates.map((t) => ` ${t.service} Pulled`).join("\n") + "\n" : "";
      return { output: pulled + fresh.map((c) => ` Container ${c.name}  Started`).join("\n") };
    },
    docker_check_updates: async ({ deviceId, auto, images }: { deviceId: number; auto: boolean; images: string[] | null }) => {
      device(deviceId);
      const s = stateFor(deviceId);
      const last = checks.get(deviceId) ?? null;
      if (auto && s.problem && ["newHost", "hostKeyChanged", "auth"].includes(s.problem.kind)) return last;
      if (auto && !images && last?.checkedAt != null && nowSecs() - last.checkedAt < 6 * 3600) return last;
      await sleep(images ? 500 : 1500);
      const check: UpdateCheck = { deviceId, checkedAt: images ? (last?.checkedAt ?? null) : nowSecs(), images: compareImages(deviceId) };
      checks.set(deviceId, check);
      return check;
    },
    docker_update_stack: async ({ deviceId, stack, services }: { deviceId: number; stack: string; services: string[] }) => {
      const s = stackOf(deviceId, stack);
      await sleep(2500);
      const lines: string[] = [];
      for (const service of services) {
        const mine = (containers.get(deviceId) ?? []).filter((c) => c.project === stack && c.service === service);
        if (!mine.length) throw `no such service: ${service}`;
        const imageId = addImage(deviceId, mine[0].image, latestDigest(mine[0].image));
        lines.push(` ${service} Pulled`);
        const template = s.templates.find((t) => t.service === service);
        for (const c of mine) {
          const id = hexId(nextSeed++);
          statsOf.set(id, statsOf.get(c.id) ?? { cpu: 0.2, memMiB: 40 });
          Object.assign(c, { id, imageId, state: "running", exitCode: null, startedAt: nowSecs(), finishedAt: null, ports: template?.ports ?? c.ports, health: template?.health ? "starting" : null });
          lines.push(` Container ${c.name}  Recreated`, ` Container ${c.name}  Started`);
        }
      }
      return { output: lines.join("\n") };
    },
    docker_pull_image: async ({ deviceId, image }: { deviceId: number; image: string }) => {
      device(deviceId);
      await sleep(1500);
      const latest = latestDigest(image);
      addImage(deviceId, image, latest);
      return { output: `${image.split(":").pop()}: Pulling from ${repoOf(image)}\nDigest: ${latest}\nStatus: Downloaded newer image for ${image}` };
    },
    docker_forget_stack: ({ deviceId, stack }: { deviceId: number; stack: string }) => {
      stacks.set(deviceId, (stacks.get(deviceId) ?? []).filter((s) => s.name !== stack));
    },
    docker_stack_files: async ({ deviceId, stack }: { deviceId: number; stack: string }) => {
      await sleep(250);
      return Object.keys(stackOf(deviceId, stack).files);
    },
    docker_read_file: async ({ deviceId, stack, path }: { deviceId: number; stack: string; path: string }) => {
      await sleep(250);
      const text = stackOf(deviceId, stack).files[path];
      if (text == null) throw `${path} isn't one of the files of the ${stack} stack.`;
      return text;
    },
    docker_save_file: async ({ deviceId, stack, path, original, text }: { deviceId: number; stack: string; path: string; original: string; text: string }) => {
      const s = stackOf(deviceId, stack);
      await sleep(600);
      if (s.files[path] !== original) throw `Not saved: ${path} was changed on the machine after you opened it. Open it again to see the new version.`;
      s.files[path] = text;
      const d = new Date();
      const p = (n: number) => String(n).padStart(2, "0");
      const backup = `${path}.${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}.bak`;
      const warning = text.includes("\t")
        ? `yaml: line ${text.split("\n").findIndex((l) => l.includes("\t")) + 1}: found character that cannot start any token`
        : null;
      return { backup, warning };
    },
    docker_cleanup_preview: async ({ deviceId }: { deviceId: number }) => {
      await sleep(900);
      return preview(deviceId);
    },
    docker_cleanup: async ({ deviceId, pick }: { deviceId: number; pick: CleanupPick }) => {
      const p = preview(deviceId);
      await sleep(1500);
      let removed = 0;
      let freed = 0;
      const failures: string[] = [];
      const take = (items: CleanupItem[], keys: string[]) =>
        items.filter((i) => {
          if (!keys.includes(i.key)) return true;
          if (i.key === "postgres:14") {
            failures.push(`${i.label}: Error response from daemon: conflict: unable to remove repository reference (must force) - container is using its referenced image`);
            return true;
          }
          removed++;
          freed += i.sizeBytes ?? 0;
          return false;
        });
      containers.set(deviceId, (containers.get(deviceId) ?? []).filter((c) => !pick.containers.includes(c.id) || c.state === "running"));
      removed += p.containers.filter((c) => pick.containers.includes(c.key)).length;
      freed += p.containers.filter((c) => pick.containers.includes(c.key)).reduce((n, c) => n + (c.sizeBytes ?? 0), 0);
      const base = leftovers.get(deviceId);
      if (base) {
        base.images = take(base.images, pick.images);
        base.volumes = take(base.volumes, pick.volumes);
        if (pick.buildCache && base.buildCacheCount) {
          removed++;
          freed += base.buildCacheBytes;
          base.buildCacheCount = 0;
          base.buildCacheBytes = 0;
        }
      }
      return { removed, freedBytes: freed, failures };
    },
    docker_logs_start: async ({ deviceId, containerId, tail }: { deviceId: number; containerId: string; tail: number | null }) => {
      const c = findContainer(deviceId, containerId);
      await sleep(300);
      const id = ++nextStream;
      const first = Math.min(tail ?? 400, 400);
      const lines = Array.from({ length: first }, (_, i) => `${stamp((first - i) * 9000)} ${LOG_LINES[i % LOG_LINES.length]}`);
      setTimeout(() => void emit("docker-logs", { id, lines }), 50);
      if (c.state !== "running") {
        setTimeout(() => void emit("docker-logs-end", { id, reason: "The container stopped, so its log ends here." }), 200);
        return id;
      }
      let n = 0;
      streams.set(
        id,
        setInterval(() => {
          n++;
          void emit("docker-logs", { id, lines: [`${stamp(0)} ${LOG_LINES[(n * 3) % LOG_LINES.length]}`] });
        }, 1200),
      );
      return id;
    },
    docker_logs_stop: ({ id }: { id: number }) => {
      const timer = streams.get(id);
      if (timer) clearInterval(timer);
      streams.delete(id);
      void emit("docker-logs-end", { id, reason: null });
    },
    docker_save_log: ({ name }: { name: string }) => `C:\\Users\\sam\\Downloads\\${name}`,
  };
}
