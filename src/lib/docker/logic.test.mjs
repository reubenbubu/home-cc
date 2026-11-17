// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  containerAttention,
  containerKey,
  commandShown,
  defaultPick,
  DOCKER_PATH,
  execCommand,
  keptSelection,
  highlight,
  keepLast,
  logFileName,
  logLine,
  matches,
  pickTotals,
  portShown,
  portText,
  shownHosts,
  stackKey,
  stackOrder,
  stackSummary,
  stateText,
  stateTone,
  stripAnsi,
  summarise,
} from "./logic.ts";

const container = (more = {}) => ({
  id: "a".repeat(64),
  name: "media-jellyfin-1",
  image: "jellyfin/jellyfin",
  state: "running",
  health: null,
  exitCode: null,
  startedAt: 1000,
  finishedAt: null,
  ports: [],
  project: "media",
  service: "jellyfin",
  ...more,
});

test("ports read like docker's own list", () => {
  assert.equal(portText({ hostPort: 8096, hostIp: null, containerPort: 8096, protocol: "tcp" }), "8096→8096/tcp");
  assert.equal(portText({ hostPort: 53, hostIp: "192.168.68.21", containerPort: 53, protocol: "udp" }), "192.168.68.21:53→53/udp");
  assert.equal(portText({ hostPort: 80, hostIp: "fe80::1", containerPort: 80, protocol: "tcp" }), "[fe80::1]:80→80/tcp");
  assert.equal(portText({ hostPort: null, hostIp: null, containerPort: 8920, protocol: "tcp" }), "8920/tcp");
  // On screen the address is a field of its own, with its hostname.
  assert.deepEqual(portShown({ hostPort: 53, hostIp: "192.168.68.21", containerPort: 53, protocol: "udp" }), { text: "53→53/udp", ip: "192.168.68.21" });
  assert.deepEqual(portShown({ hostPort: 8096, hostIp: null, containerPort: 8096, protocol: "tcp" }), { text: "8096→8096/tcp", ip: null });
  assert.deepEqual(portShown({ hostPort: null, hostIp: null, containerPort: 8920, protocol: "tcp" }), { text: "8920/tcp", ip: null });
});

test("state words and colours", () => {
  const now = 1000 + 3 * 86400 + 2 * 3600;
  assert.equal(stateText(container(), now), "Up 3 d 2 h");
  assert.equal(stateTone(container()), "ok");
  assert.equal(stateText(container({ health: "unhealthy" }), now), "Up 3 d 2 h · unhealthy");
  assert.equal(stateTone(container({ health: "unhealthy" })), "bad");
  assert.equal(stateTone(container({ health: "starting" })), "warn");
  const stopped = container({ state: "exited", exitCode: 1, finishedAt: now - 7200 });
  assert.equal(stateText(stopped, now), "Stopped 2 h 0 min ago (exit 1)");
  assert.equal(stateTone(stopped), "bad", "a crash");
  assert.equal(stateTone({ ...stopped, exitCode: 0 }), "idle", "a clean stop");
  assert.equal(stateTone({ ...stopped, exitCode: 143 }), "idle", "stopped by docker stop");
  assert.equal(stateText({ ...stopped, exitCode: 137 }, now), "Stopped 2 h 0 min ago", "no exit code for a normal stop");
  assert.equal(stateText(container({ state: "created", startedAt: null }), now), "Never started");
  assert.equal(stateText(container({ state: "restarting", exitCode: 2 }), now), "Restarting (exit 2)");
  assert.equal(stateText(container({ state: "removing" }), now), "Removing");
});

test("stack summaries", () => {
  const up = container();
  const down = container({ state: "exited", exitCode: 0 });
  const crashed = container({ state: "exited", exitCode: 1 });
  assert.deepEqual(stackSummary({ name: "s", workingDir: null, configFiles: [], containers: [] }), {
    running: 0,
    total: 0,
    tone: "idle",
    text: "Down",
  });
  assert.equal(stackSummary({ name: "s", workingDir: null, configFiles: [], containers: [up, up] }).text, "All 2 running");
  assert.equal(stackSummary({ name: "s", workingDir: null, configFiles: [], containers: [up] }).text, "Running");
  const part = stackSummary({ name: "s", workingDir: null, configFiles: [], containers: [up, down] });
  assert.deepEqual([part.text, part.tone], ["1 of 2 running", "warn"]);
  assert.equal(stackSummary({ name: "s", workingDir: null, configFiles: [], containers: [up, crashed] }).tone, "bad");
});

test("stacks that are down go last", () => {
  const s = (name, n) => ({ name, workingDir: null, configFiles: [], containers: Array(n).fill(container()) });
  assert.deepEqual(stackOrder([s("b", 0), s("c", 1), s("a", 2), s("a-down", 0)]).map((x) => x.name), ["a", "c", "a-down", "b"]);
});

test("which hosts show", () => {
  const h = (mode, found) => ({ deviceId: 1, mode, found, version: null, compose: "plugin", checkedAt: null, problem: null, needsCheck: false });
  const shown = shownHosts([h("auto", true), h("auto", false), h("auto", null), h("on", null), h("off", true)]);
  assert.deepEqual(
    shown.map((x) => [x.mode, x.found]),
    [
      ["auto", true],
      ["on", null],
    ],
  );
});

test("the open host moves on when it loses its tab", () => {
  const h = (deviceId) => ({ deviceId, mode: "on", found: true, version: null, compose: "plugin", checkedAt: null, problem: null, needsCheck: false });
  assert.equal(keptSelection(2, [h(1), h(2)]), 2, "still shown: stays");
  assert.equal(keptSelection(3, [h(1), h(2)]), 1, "set to Never show: the first tab instead");
  assert.equal(keptSelection(null, [h(4)]), 4);
  assert.equal(keptSelection(3, []), null, "nothing left to show");
});

test("actions on same-named stacks of two hosts are kept apart", () => {
  assert.notEqual(stackKey(1, "monitoring"), stackKey(2, "monitoring"));
  assert.equal(stackKey(1, "monitoring"), stackKey(1, "monitoring"));
  assert.notEqual(containerKey(1, "abc"), containerKey(2, "abc"));
  assert.notEqual(stackKey(1, "abc"), containerKey(1, "abc"));
});

test("log lines are split from Docker's timestamp", () => {
  const l = logLine("2026-11-17T14:03:09.123456789Z \u001b[32mINFO\u001b[0m started");
  assert.equal(l.at, Date.parse("2026-11-17T14:03:09.123Z"));
  assert.equal(l.text, "INFO started");
  assert.deepEqual(logLine("no stamp here"), { at: null, text: "no stamp here" });
  assert.equal(logLine("2026-11-17T14:03:09+01:00 x").at, Date.parse("2026-11-17T13:03:09Z"));
});

test("search and highlight ignore case", () => {
  assert.ok(matches("Error: disk full", "error"));
  assert.ok(matches("anything", "  "));
  assert.ok(!matches("all fine", "error"));
  assert.deepEqual(highlight("an Error and error", "error"), [
    { text: "an ", hit: false },
    { text: "Error", hit: true },
    { text: " and ", hit: false },
    { text: "error", hit: true },
  ]);
  assert.deepEqual(highlight("x", ""), [{ text: "x", hit: false }]);
});

test("colour codes are removed", () => {
  assert.equal(stripAnsi("\u001b[1;31mred\u001b[0m \u001b]0;title\u0007ok"), "red ok");
});

test("the log view keeps the newest lines", () => {
  assert.deepEqual(keepLast([1, 2, 3, 4], 2), [3, 4]);
  const same = [1];
  assert.equal(keepLast(same, 5), same);
});

test("saved log names", () => {
  assert.equal(logFileName("Homelab VM", "media-jellyfin-1", new Date(2026, 10, 7, 9, 5)), "homelab-vm-media-jellyfin-1-2026-11-07-0905.log");
  assert.equal(logFileName("../x", "a/b", new Date(2026, 0, 1, 0, 0)), "..-x-a-b-2026-01-01-0000.log");
});

test("cleanup picks everything but named volumes, and adds up what it frees", () => {
  const preview = {
    containers: [{ key: "c1", label: "old", detail: "", sizeBytes: 100 }],
    images: [
      { key: "nginx:1", label: "nginx:1", detail: "", sizeBytes: 1000 },
      { key: "sha256:abc", label: "Untagged", detail: "", sizeBytes: null },
    ],
    volumes: [
      { key: "photos_db", label: "photos_db", detail: "Named volume of the photos stack", sizeBytes: 5000 },
      { key: "f00", label: "Unnamed volume f00", detail: "Unnamed volume", sizeBytes: 10 },
    ],
    buildCacheCount: 3,
    buildCacheBytes: 7,
  };
  const pick = defaultPick(preview);
  assert.deepEqual(pick, { containers: ["c1"], images: ["nginx:1", "sha256:abc"], volumes: ["f00"], buildCache: true });
  assert.deepEqual(pickTotals(preview, pick), { count: 1 + 2 + 1 + 3, bytes: 100 + 1000 + 10 + 7 });
  assert.deepEqual(pickTotals(preview, { containers: [], images: [], volumes: ["photos_db"], buildCache: false }), { count: 1, bytes: 5000 });
  const empty = { containers: [], images: [], volumes: [], buildCacheCount: 0, buildCacheBytes: 0 };
  assert.equal(defaultPick(empty).buildCache, false);
});

test("a terminal into a container names it only as Docker could have", () => {
  const env = 'env PATH="$PATH:/usr/local/bin:/usr/bin:/bin:/snap/bin" ';
  const shell = "sh -c 'command -v bash >/dev/null && exec bash || exec sh'";
  assert.equal(execCommand({ id: "a".repeat(64), name: "media-jellyfin-1" }), `${env}docker exec -it media-jellyfin-1 ${shell}`);
  assert.equal(execCommand({ id: "a".repeat(64), name: "web_2.old" }), `${env}docker exec -it web_2.old ${shell}`);
  // A name that isn't one Docker gives: the id instead, if it is one.
  assert.equal(execCommand({ id: "0123456789ab", name: "x; rm -rf ~" }), `${env}docker exec -it 0123456789ab ${shell}`);
  for (const name of ["", "-it", "--privileged", "a b", "a'b", "$(id)", "/web", "ünïcode", "a\nb"]) {
    assert.equal(execCommand({ id: "not-hex!", name }), null, JSON.stringify(name));
  }
  for (const id of ["ABCDEF012345", "abc", "a".repeat(65), "0123456789ab;id"]) {
    assert.equal(execCommand({ id, name: "" }), null, id);
  }
});

test("a container's terminal looks for docker where the Docker module does, and that PATH isn't shown by the tab", () => {
  // The same list as PREAMBLE in src-tauri/src/docker/scripts.rs (a Rust test there checks this file).
  assert.equal(DOCKER_PATH, "/usr/local/bin:/usr/bin:/bin:/snap/bin");
  const command = execCommand({ id: "a".repeat(64), name: "web" });
  assert.equal(commandShown(command), "docker exec -it web sh -c 'command -v bash >/dev/null && exec bash || exec sh'");
  assert.equal(commandShown("pct enter 101"), "pct enter 101");
  assert.equal(commandShown('PATH="$PATH:/x" docker ps'), 'PATH="$PATH:/x" docker ps');
});

test("the Overview counts a host's containers and points at the ones that need a look", () => {
  const stack = (name, containers) => ({ name, workingDir: null, configFiles: [], containers });
  const o = {
    deviceId: 1,
    readAt: 0,
    stacks: [
      stack("media", [
        container({ name: "jellyfin" }),
        container({ name: "sonarr", health: "unhealthy" }),
        container({ name: "radarr", state: "exited", exitCode: 0 }),
        container({ name: "prowlarr", state: "exited", exitCode: 143 }),
      ]),
      stack("home", [container({ name: "zigbee", state: "restarting", exitCode: 1, project: "home" })]),
      stack("nextcloud", []),
    ],
    others: [
      container({ name: "backup", state: "exited", exitCode: 2, project: null }),
      container({ name: "old", state: "dead", project: null }),
    ],
  };
  const s = summarise(o);
  assert.equal(s.containers, 7);
  assert.equal(s.running, 2, "unhealthy still counts as running");
  assert.equal(s.stacks, 2, "a stack that is down isn't counted");
  assert.deepEqual(
    s.attention.map((a) => `${a.name}: ${a.why}`),
    ["backup: stopped with an error (exit 2)", "old: is dead", "sonarr: is unhealthy", "zigbee: keeps restarting (exit 1)"],
  );
});

test("a container stopped on purpose doesn't need a look", () => {
  for (const exitCode of [0, 137, 143, null]) assert.equal(containerAttention(container({ state: "exited", exitCode })), null, String(exitCode));
  assert.equal(containerAttention(container({ state: "exited", exitCode: 1 })), "stopped with an error (exit 1)");
  assert.equal(containerAttention(container({ state: "running", health: "starting" })), null);
});
