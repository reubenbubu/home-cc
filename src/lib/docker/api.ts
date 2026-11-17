// Typed wrappers around the Docker commands (src-tauri/src/docker/commands.rs).
// Kept apart from ../api.ts so the Docker module stays in its own files.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** auto: shown when home-cc found Docker on it. on: the owner says it is a Docker host. off: never shown. */
export type HostMode = "auto" | "on" | "off";
/** Which compose command the host has: "docker compose" (plugin), the older "docker-compose", or none. */
export type Compose = "plugin" | "standalone" | "none";

/** What stops Docker from being used on a host.
 *  - notInstalled: no docker command. permission: the SSH user isn't in the docker group.
 *  - daemon: Docker isn't running. noCompose: no compose command, so stacks can't be managed.
 *  - newHost / hostKeyChanged / auth: logging in needs the owner (see the Machines screen); not retried by itself.
 *  - unreachable: the machine didn't answer. */
export type ProblemKind =
  | "notInstalled"
  | "permission"
  | "daemon"
  | "noCompose"
  | "newHost"
  | "hostKeyChanged"
  | "auth"
  | "unreachable"
  | "other";

export interface DockerProblem {
  kind: ProblemKind;
  /** Plain text for the owner, with what to do. */
  message: string;
}

/** A machine that is, or could be, a Docker host: any tracked Linux machine with an SSH user. */
export interface DockerHost {
  deviceId: number;
  mode: HostMode;
  /** True: Docker found. False: not there. Null: not looked at yet. */
  found: boolean | null;
  /** The Docker engine's version, when it answered. */
  version: string | null;
  compose: Compose;
  /** Unix seconds of the last look. */
  checkedAt: number | null;
  /** What went wrong the last time, if anything. */
  problem: DockerProblem | null;
  /** True when the screen should look for Docker on it now (dockerProbe). */
  needsCheck: boolean;
}

export interface Port {
  /** On the host, when published. */
  hostPort: number | null;
  /** Only when published on one address rather than all of them. */
  hostIp: string | null;
  containerPort: number;
  protocol: string;
}

export interface Container {
  /** Full id, 64 hex characters. */
  id: string;
  name: string;
  image: string;
  /** "running", "exited", "paused", "restarting", "created", "removing" or "dead". */
  state: string;
  /** "healthy", "unhealthy" or "starting", for containers with a health check. */
  health: string | null;
  /** How the last run ended, for a container that isn't running. */
  exitCode: number | null;
  /** Unix seconds. */
  startedAt: number | null;
  finishedAt: number | null;
  ports: Port[];
  project: string | null;
  service: string | null;
}

/** A compose stack. One with no containers is down, remembered from an earlier look. */
export interface Stack {
  name: string;
  workingDir: string | null;
  configFiles: string[];
  containers: Container[];
}

export interface Overview {
  deviceId: number;
  /** Unix seconds. */
  readAt: number;
  stacks: Stack[];
  /** Containers that aren't in a compose stack. */
  others: Container[];
}

export interface HostView {
  host: DockerHost;
  /** Null when it couldn't be read; host.problem says why. */
  overview: Overview | null;
}

export interface ContainerStats {
  id: string;
  /** Share of one CPU core: can go above 100 on a machine with several. */
  cpuPct: number | null;
  memUsedBytes: number | null;
  memLimitBytes: number | null;
}

export type ContainerAction = "start" | "stop" | "restart" | "remove";
/** up: create and start. down: stop and remove containers (volumes kept). pull: newer images, then recreate.
 *  restart: restart as they are. build: rebuild images from Dockerfiles, then recreate. */
export type StackAction = "up" | "down" | "pull" | "restart" | "build";

export interface ActionOutput {
  /** The last lines docker printed. */
  output: string;
}

export interface SavedFile {
  /** Where the copy of the old file was put, on the host. */
  backup: string;
  /** What compose says is wrong with the stack's files now, if anything. */
  warning: string | null;
}

export interface CleanupItem {
  /** What is handed to docker to remove it. */
  key: string;
  label: string;
  detail: string;
  sizeBytes: number | null;
}

export interface CleanupPreview {
  containers: CleanupItem[];
  images: CleanupItem[];
  /** Named ones may hold a stack's data. */
  volumes: CleanupItem[];
  buildCacheCount: number;
  buildCacheBytes: number;
}

/** Keys from a CleanupPreview the owner picked. */
export interface CleanupPick {
  containers: string[];
  images: string[];
  volumes: string[];
  buildCache: boolean;
}

export interface CleanupResult {
  removed: number;
  /** From the preview's sizes of what was removed. */
  freedBytes: number;
  /** One line per thing not removed, with docker's reason. */
  failures: string[];
}

/** Sent as "docker-logs": new lines of a followed log. Each starts with Docker's timestamp. */
export interface LogLines {
  id: number;
  lines: string[];
}

/** Sent as "docker-logs-end" once, when a followed log ends or is stopped. */
export interface LogEnd {
  id: number;
  /** Why it ended; null when the screen stopped it. */
  reason: string | null;
}

export const dockerApi = {
  hosts: () => invoke<DockerHost[]>("docker_hosts"),
  /** Look for Docker on a machine now. */
  probe: (deviceId: number) => invoke<DockerHost>("docker_probe", { deviceId }),
  setHostMode: (deviceId: number, mode: HostMode) => invoke<DockerHost>("docker_set_host_mode", { deviceId, mode }),
  /** auto: set by the screen's own timer. A host that turned the login away is then not tried again. */
  overview: (deviceId: number, auto: boolean) => invoke<HostView>("docker_overview", { deviceId, auto }),
  /** Takes about two seconds. */
  stats: (deviceId: number) => invoke<ContainerStats[]>("docker_stats", { deviceId }),
  containerAction: (deviceId: number, containerId: string, action: ContainerAction) =>
    invoke<ActionOutput>("docker_container_action", { deviceId, containerId, action }),
  /** Pull and build can take many minutes. */
  stackAction: (deviceId: number, stack: string, action: StackAction) =>
    invoke<ActionOutput>("docker_stack_action", { deviceId, stack, action }),
  forgetStack: (deviceId: number, stack: string) => invoke<void>("docker_forget_stack", { deviceId, stack }),
  /** The stack's compose files first, then compose files, Dockerfiles and .env files in its folder. */
  stackFiles: (deviceId: number, stack: string) => invoke<string[]>("docker_stack_files", { deviceId, stack }),
  readFile: (deviceId: number, stack: string, path: string) => invoke<string>("docker_read_file", { deviceId, stack, path }),
  /** Refused if the file changed on the host since `original` was read. Keeps a backup next to it. */
  saveFile: (deviceId: number, stack: string, path: string, original: string, text: string) =>
    invoke<SavedFile>("docker_save_file", { deviceId, stack, path, original, text }),
  cleanupPreview: (deviceId: number) => invoke<CleanupPreview>("docker_cleanup_preview", { deviceId }),
  cleanup: (deviceId: number, pick: CleanupPick) => invoke<CleanupResult>("docker_cleanup", { deviceId, pick }),
  /** Follow a log; lines arrive as "docker-logs" events with the returned id. tail null: from the start. */
  logsStart: (deviceId: number, containerId: string, tail: number | null) =>
    invoke<number>("docker_logs_start", { deviceId, containerId, tail }),
  logsStop: (id: number) => invoke<void>("docker_logs_stop", { id }),
  /** Into this PC's Downloads folder. Resolves with the full path. */
  saveLog: (name: string, text: string) => invoke<string>("docker_save_log", { name, text }),
};

export const dockerEvents = {
  onLogs: (fn: (l: LogLines) => void): Promise<UnlistenFn> => listen<LogLines>("docker-logs", (e) => fn(e.payload)),
  onLogsEnd: (fn: (l: LogEnd) => void): Promise<UnlistenFn> => listen<LogEnd>("docker-logs-end", (e) => fn(e.payload)),
};
