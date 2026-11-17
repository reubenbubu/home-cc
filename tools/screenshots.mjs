// Saves a screenshot of every screen at each of the owner's monitor sizes,
// using the sample back end (src/lib/mock), and checks that nothing scrolls
// sideways. Needs Playwright with Chromium, installed globally or locally.
//
//   node tools/screenshots.mjs <output folder> [--url http://127.0.0.1:1420]
//        [--only overview,settings] [--sizes 1920x1080,1280x720@1.5] [--zoom 200]
//
// --sizes takes any WIDTHxHEIGHT, with @SCALE for display scaling.
// --zoom 200 takes every picture as if the app were zoomed to 200% (Ctrl and +
// in the app), for example "--sizes 960x600 --zoom 200" for the smallest
// window zoomed right in. The app's zoom is the window's own, which is the same
// as a window that is that much smaller with that much more display scaling.
//
// Without --url it starts its own dev server on a free port and stops it at
// the end. Exits with an error if any screen is wider than its window, logs
// an error, can't cope with the unusual readings of "?awkward", or if one of
// the things the owner does on a screen (approving a machine's identity,
// emptying a hardware address, ...) doesn't end the way it should, or if the
// hardware address can't be read in full on the Network screen at some width.
import { execFileSync, spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { createRequire } from "node:module";
import { createServer } from "node:net";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// What Windows hands the app on each monitor. The last two are a 1080p screen
// at 125% scaling: once as a plain zoom of the full-size window, and once as
// the app really sees it (fewer, bigger pixels).
const SIZES = [
  { name: "1920x1080", width: 1920, height: 1080, scale: 1 },
  { name: "2560x1440", width: 2560, height: 1440, scale: 1 },
  { name: "3440x1440", width: 3440, height: 1440, scale: 1 },
  { name: "1920x1080@1.25", width: 1920, height: 1080, scale: 1.25 },
  { name: "1536x864@1.25", width: 1536, height: 864, scale: 1.25 },
];

const SCREENS = [
  { name: "overview", path: "/", ready: ".card" },
  { name: "network", path: "/network", ready: "table" },
  {
    name: "network-scanning",
    path: "/network",
    ready: "table",
    async before(page) {
      await page.getByRole("button", { name: "Scan network" }).click();
      // Part-way through the first step, with some devices already found.
      await page.waitForFunction(() => Number(document.querySelector('[role="progressbar"]')?.getAttribute("aria-valuenow")) >= 35);
    },
    async after(page) {
      await page.getByRole("button", { name: "Stop" }).click();
      await page.getByRole("button", { name: "Scan network" }).waitFor();
    },
  },
  {
    name: "machines",
    path: "/machines",
    ready: "article",
    async before(page) {
      // The sample data has one machine in each state that waits for the owner; all must be in the picture.
      await page.getByText("This machine's identity has changed").waitFor();
      await page.getByText("Before the stored password is used for Media PC").waitFor();
      await page.getByText("home-cc has stopped trying this machine").waitFor();
    },
  },
  {
    // Both approvals half given: the second click is the one that counts.
    name: "machines-approve",
    path: "/machines",
    ready: "article",
    async before(page) {
      await page.getByRole("button", { name: "Approve the new identity" }).click();
      await page.getByRole("button", { name: "Approve and use the password" }).click();
      await page.getByRole("button", { name: "Yes, it matches" }).nth(1).waitFor();
    },
  },
  {
    name: "machines-edit",
    path: "/machines",
    ready: "article",
    async before(page) {
      await page.getByRole("button", { name: "Edit NAS" }).click();
      await page.getByText("A password is stored").waitFor();
      await page.getByText("Identity home-cc remembers").waitFor();
    },
  },
  // Key login, in the form (components/KeySetup.svelte), with each way it can end in the sample data.
  {
    name: "machines-keys",
    path: "/machines",
    ready: "article",
    async before(page) {
      await page.getByRole("button", { name: "Edit Homelab VM" }).click();
      const form = page.locator("form");
      await form.getByRole("button", { name: "Show key login" }).click();
      await form.getByText("This PC's key", { exact: true }).waitFor();
      await form.getByRole("button", { name: "Set up key login", exact: true }).click();
      await form.getByText("Key login works.").waitFor();
    },
  },
  {
    name: "machines-keys-approve",
    path: "/machines",
    ready: "article",
    async before(page) {
      await page.getByRole("button", { name: "Edit Media PC" }).click();
      const form = page.locator("form");
      await form.getByRole("button", { name: "Show key login" }).click();
      await form.getByRole("button", { name: "Set up key login", exact: true }).click();
      await form.getByText("Before the password is used for Media PC, check its identity.").waitFor();
    },
  },
  {
    name: "machines-keys-refused",
    path: "/machines",
    ready: "article",
    async before(page) {
      await page.getByRole("button", { name: "Edit NAS" }).click();
      const form = page.locator("form");
      await form.getByRole("button", { name: "Show key login" }).click();
      await form.getByRole("button", { name: "Set up key login", exact: true }).click();
      await form.getByText("still didn't work").waitFor();
    },
  },
  {
    // From the card of a machine whose login failed: the form opens with key login showing.
    name: "machines-keys-card",
    path: "/machines",
    ready: "article",
    async before(page) {
      await page.locator("article", { hasText: "Pi-hole 1" }).getByRole("button", { name: "Set up key login" }).click();
      const form = page.locator("form");
      await form.getByText("This PC's key", { exact: true }).waitFor();
      await form.getByRole("button", { name: "Set up key login", exact: true }).click();
      await form.getByText("no password is typed or stored").waitFor();
    },
  },
  {
    name: "machines-keys-new",
    path: "/machines?nokey",
    ready: "article",
    async before(page) {
      await page.getByRole("button", { name: "Edit Homelab VM" }).click();
      const form = page.locator("form");
      await form.getByRole("button", { name: "Show key login" }).click();
      await form.getByText("This PC has no SSH key yet").waitFor();
    },
  },
  { name: "settings", path: "/settings", ready: "form" },
  // Terminals: two tabs, the one shown after a few commands; then a tab that couldn't log in.
  {
    name: "terminals",
    path: "/terminals",
    ready: ".empty",
    async before(page) {
      await openTerminalTab(page, "Dev VM (sam@192.168.68.22)");
      await openTerminalTab(page, "Homelab VM (sam@192.168.68.21)");
      await terminalShows(page, "Try ls");
      await page.locator('[role="tabpanel"]:not([hidden]) .xterm').click();
      for (const [line, shows] of [["ls", "update.sh"], ["docker ps", "homeassistant"], ["colours", "magenta"]]) {
        await page.keyboard.type(`${line}\r`);
        await terminalShows(page, shows);
      }
      // Every row in sight: the terminal is no taller than its place on the screen (nor much shorter).
      const fit = await page.evaluate(() => {
        const host = document.querySelector('[role="tabpanel"]:not([hidden]) .terminal-host').getBoundingClientRect();
        const drawn = document.querySelector('[role="tabpanel"]:not([hidden]) .xterm-screen').getBoundingClientRect();
        const row = document.querySelector('[role="tabpanel"]:not([hidden]) .xterm-rows > div').getBoundingClientRect();
        return { host: host.height, drawn: drawn.height, row: row.height };
      });
      if (fit.drawn > fit.host + 0.5 || fit.host - fit.drawn >= fit.row) {
        throw new Error(`the terminal doesn't fit its place: ${fit.drawn}px of rows in ${fit.host}px`);
      }
    },
  },
  {
    name: "terminals-failed",
    path: "/terminals",
    ready: ".empty",
    async before(page) {
      await openTerminalTab(page, "Media PC (sam@192.168.68.70)");
      await page.getByText("Couldn't open the terminal").waitFor();
    },
  },
  {
    name: "terminals-lost",
    path: "/terminals",
    ready: ".empty",
    async before(page) {
      await openTerminalTab(page, "Dev VM (sam@192.168.68.22)");
      await terminalShows(page, "Try ls");
      await page.keyboard.type("ls\rdrop\r");
      await page.getByText("was lost").waitFor();
    },
  },
  // --- Proxmox ---
  { name: "proxmox", path: "/proxmox", ready: "table" },
  {
    name: "proxmox-guest",
    path: "/proxmox",
    ready: "table",
    async before(page) {
      await page.getByRole("button", { name: "Manage homeassistant" }).click();
      await page.getByText("before_2026_11", { exact: true }).waitFor();
    },
  },
  // A container's console, and a VM whose address is a machine in home-cc: its serial console or a terminal there.
  {
    name: "proxmox-console",
    path: "/machines",
    ready: "article",
    async before(page) {
      await openProxmoxScreenLoggedIn(page);
      await page.getByRole("button", { name: "Manage pihole1" }).click();
      await page.getByRole("complementary").getByRole("button", { name: "Open console" }).click();
      await page.waitForURL("**/terminals");
      await terminalShows(page, "Entered container 101");
    },
  },
  {
    name: "proxmox-vm-console",
    path: "/proxmox",
    ready: "table",
    async before(page) {
      await page.getByRole("button", { name: "Manage homelab" }).click();
      await page.getByRole("complementary").getByRole("button", { name: "Terminal on Homelab VM" }).waitFor();
    },
  },
  { name: "proxmox-cert", path: "/proxmox?pvecert", ready: ".alert", before: (page) => page.getByText("check its certificate").waitFor() },
  { name: "proxmox-cert-changed", path: "/proxmox?pvechanged", ready: ".alert", before: (page) => page.getByText("different certificate").waitFor() },
  { name: "proxmox-setup", path: "/proxmox?pvenotoken", ready: "form", before: (page) => page.getByText("How to make a token").waitFor() },
  { name: "proxmox-permission", path: "/proxmox?pveperm", ready: ".alert", before: (page) => page.getByText("Privilege Separation").waitFor() },
  { name: "proxmox-states", path: "/proxmox?pvestates", ready: "table", before: (page) => page.getByText("Hibernated").first().waitFor() },
  // --- end Proxmox ---
  // "?noping": this PC isn't allowed to send pings, so no status check works.
  {
    name: "overview-check-problem",
    path: "/?noping",
    ready: ".card",
    before: (page) => page.getByText("Status checks aren't working").waitFor(),
  },
  {
    name: "network-check-problem",
    path: "/network?noping",
    ready: "table",
    before: (page) => page.getByText("Status checks aren't working").waitFor(),
  },
  // --- Docker (spec 5.5) ---
  {
    name: "docker",
    path: "/docker",
    ready: "article",
    // CPU and memory are read after the list; the picture should have them.
    before: (page) => page.getByText("612 MB").waitFor(),
  },
  {
    name: "docker-logs",
    path: "/docker",
    ready: "article",
    async before(page) {
      await page.getByRole("button", { name: "Logs of media-jellyfin-1" }).click();
      await page.locator(".line").nth(10).waitFor();
      await page.getByLabel("Search the log").fill("playback");
    },
  },
  {
    name: "docker-cleanup",
    path: "/docker",
    ready: "article",
    async before(page) {
      await page.getByRole("button", { name: "Clean up…" }).click();
      await page.getByText("Unused volumes").waitFor();
    },
  },
  {
    name: "docker-files",
    path: "/docker",
    ready: "article",
    async before(page) {
      await page.getByRole("article", { name: "Stack media" }).getByRole("button", { name: "Files" }).click();
      await page.locator("textarea").waitFor();
    },
  },
  {
    name: "docker-terminal",
    path: "/docker",
    ready: "article",
    async before(page) {
      await page.getByRole("button", { name: "Open a terminal in media-jellyfin-1" }).click();
      await page.waitForURL("**/terminals");
      await terminalShows(page, "Entered the container media-jellyfin-1");
    },
  },
  {
    name: "docker-hosts",
    path: "/docker",
    ready: "article",
    async before(page) {
      await page.getByRole("button", { name: "Docker hosts…" }).click();
      await page.getByText("isn't allowed to use it").first().waitFor();
    },
  },
];

/** From the Machines screen: approve the Proxmox host's new SSH identity (the sample has it changed), so a
 *  console can log in to it, then go to the Proxmox screen. */
async function openProxmoxScreenLoggedIn(page) {
  const proxmox = page.locator("article", { hasText: "Proxmox" });
  await proxmox.getByRole("button", { name: "Approve the new identity" }).click();
  await proxmox.getByRole("button", { name: "Yes, it matches" }).click();
  await proxmox.getByText("This machine's identity has changed").waitFor({ state: "detached" });
  await page.getByRole("link", { name: "Proxmox", exact: true }).click();
  await page.locator("table").waitFor();
}

/** On the Terminals screen: open a tab on the machine with this label in the list. */
async function openTerminalTab(page, label) {
  await page.getByLabel("Machine").selectOption({ label });
  await page.getByRole("button", { name: "Open terminal" }).click();
  await page.getByRole("tab", { selected: true, name: label.split(" (")[0] }).waitFor();
}

/** Waits until the terminal shown has this text on it. */
async function terminalShows(page, text) {
  await page.waitForFunction(
    (t) => document.querySelector('[role="tabpanel"]:not([hidden]) .xterm-rows')?.textContent.includes(t),
    text,
    { timeout: 10_000 },
  );
}

function parseArgs(argv) {
  const args = { out: null, url: null, only: null, sizes: null, zoom: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--url" || a === "--only" || a === "--sizes" || a === "--zoom") args[a.slice(2)] = argv[++i] ?? "";
    else if (!args.out) args.out = a;
  }
  return args;
}

function parseSize(name) {
  const m = /^(\d+)x(\d+)(?:@(\d+(?:\.\d+)?))?$/.exec(name);
  if (!m) throw new Error(`"${name}" is not a size. Write it like 1920x1080 or 1536x864@1.25.`);
  return { name, width: Number(m[1]), height: Number(m[2]), scale: Number(m[3] ?? 1) };
}

/** A size as the screens see it with the app zoomed to `percent`. */
function zoomed(size, percent) {
  const z = percent / 100;
  return {
    name: `${size.name}-zoom${percent}`,
    width: Math.round(size.width / z),
    height: Math.round(size.height / z),
    scale: size.scale * z,
  };
}

/** Playwright is usually installed globally, where a plain import from this folder can't see it. */
async function loadChromium() {
  try {
    return (await import("playwright")).chromium;
  } catch {
    // Fall through to the global folders.
  }
  const roots = [process.env.NODE_PATH, "/usr/local/lib/node_modules", "/usr/lib/node_modules"];
  try {
    roots.push(execFileSync("npm", ["root", "-g"], { encoding: "utf8" }).trim());
  } catch {
    // npm isn't needed if one of the other folders has it.
  }
  const require = createRequire(import.meta.url);
  for (const dir of roots.filter(Boolean)) {
    for (const name of ["playwright", "@playwright/test"]) {
      try {
        const mod = await import(pathToFileURL(require.resolve(name, { paths: [dir] })).href);
        const chromium = mod.chromium ?? mod.default?.chromium;
        if (chromium) return chromium;
      } catch {
        // Try the next place.
      }
    }
  }
  throw new Error("Playwright wasn't found. Install it with: npm install -g playwright && playwright install chromium");
}

function freePort() {
  return new Promise((ok, fail) => {
    const server = createServer();
    server.on("error", fail);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => ok(port));
    });
  });
}

async function startDevServer() {
  const port = await freePort();
  const child = spawn(
    process.execPath,
    [join(root, "node_modules/vite/bin/vite.js"), "dev", "--port", String(port), "--strictPort", "--host", "127.0.0.1"],
    { cwd: root, stdio: ["ignore", "pipe", "pipe"] },
  );
  let log = "";
  child.stdout.on("data", (d) => (log += d));
  child.stderr.on("data", (d) => (log += d));
  const url = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 150; i++) {
    if (child.exitCode !== null) throw new Error(`The dev server stopped:\n${log}`);
    try {
      if ((await fetch(url)).ok) return { url, stop: () => child.kill() };
    } catch {
      // Not up yet.
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  child.kill();
  throw new Error(`The dev server didn't start in 30 seconds:\n${log}`);
}

/** Runs in the page. Lists what scrolls sideways, and (for information) text that is cut short with "…". */
function inspect() {
  const describe = (el) => {
    const cls = typeof el.className === "string" && el.className ? `.${el.className.trim().split(/\s+/).join(".")}` : "";
    return `${el.tagName.toLowerCase()}${cls}`;
  };
  const overflow = [];
  const doc = document.documentElement;
  if (doc.scrollWidth > window.innerWidth) overflow.push(`the page is ${doc.scrollWidth}px wide in a ${window.innerWidth}px window`);
  const cut = [];
  for (const el of document.querySelectorAll("body *")) {
    if (el.scrollWidth <= el.clientWidth + 1 || el.clientWidth === 0) continue;
    const style = getComputedStyle(el);
    if (style.overflowX === "auto" || style.overflowX === "scroll") {
      overflow.push(`${describe(el)} scrolls sideways (${el.scrollWidth}px of content in ${el.clientWidth}px)`);
    } else if (style.textOverflow === "ellipsis" && style.overflowX === "hidden") {
      cut.push(`${describe(el)} "${el.textContent.trim().replace(/\s+/g, " ").slice(0, 40)}"`);
    }
  }
  const content = document.querySelector("main");
  return { overflow, cut, tall: content ? content.scrollHeight - content.clientHeight : 0 };
}

/** Readings that are allowed but unusual (the sample back end adds them for "?awkward": two disks on one
 *  folder) must not break a screen. Returns what went wrong. */
async function checkAwkward(browser, url) {
  const checks = [
    {
      path: "/machines?awkward",
      what: "both /mnt/data disks on the Homelab VM card",
      find: (page) => page.locator("article", { hasText: "Homelab VM" }).getByText("/mnt/data", { exact: true }),
    },
    {
      path: "/?awkward",
      what: "both /mnt/data warnings under Needs attention",
      find: (page) => page.getByRole("link", { name: /Homelab VM: disk \/mnt\/data/ }),
    },
  ];
  const problems = [];
  const page = await browser.newPage({ viewport: { width: 1920, height: 1080 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  for (const check of checks) {
    await page.goto(url + check.path, { waitUntil: "networkidle" });
    try {
      await check.find(page).nth(1).waitFor({ timeout: 10_000 });
    } catch {
      problems.push(`${check.path}: didn't show ${check.what}`);
    }
    for (const error of errors.splice(0)) problems.push(`${check.path}: error in the page: ${error}`);
  }
  await page.close();
  return problems;
}

/** Runs in the page. From now on the name of every request to the back end is noted in window.__calls,
 *  in the order the requests were made. */
function recordCalls() {
  const tauri = window.__TAURI_INTERNALS__;
  const invoke = tauri.invoke;
  window.__calls = [];
  tauri.invoke = (cmd, ...rest) => {
    window.__calls.push(cmd);
    return invoke(cmd, ...rest);
  };
}

/** Runs in the page. From now on the back end behaves as if a round of readings the screen hasn't heard
 *  about found the machines that were waiting for an approval unreachable: nothing is on offer any more,
 *  so an approval is refused. */
function withdrawOffers() {
  const tauri = window.__TAURI_INTERNALS__;
  const invoke = tauri.invoke;
  tauri.invoke = async (cmd, ...rest) => {
    if (cmd === "trust_host_key") throw "This machine isn't waiting for its identity to be approved.";
    const result = await invoke(cmd, ...rest);
    if (cmd !== "machine_stats") return result;
    return result.map((e) =>
      e.errorKind === "hostKeyChanged"
        ? { ...e, error: "Couldn't reach this machine: the connection timed out.", errorKind: "unreachable", offeredKey: null, expectedFingerprint: null }
        : e,
    );
  };
}

/** The sample NAS, a device with a hardware address that is on the Network screen from the start. */
const NAS_MAC = "d8:bb:c1:7a:02:6e";

/** Runs in the page, on the Network screen. Says how the hardware address of one device (and the line that
 *  stands in for the column in every row) is shown. */
function hardwareAddressShown(mac) {
  const shown = (el) => !!el && el.getClientRects().length > 0;
  const whole = (el) => el.scrollWidth <= el.clientWidth;
  /** Inside the cell it is in, so the cell doesn't cut it either. */
  const inside = (el, cell) => {
    const a = el.getBoundingClientRect();
    const b = cell.getBoundingClientRect();
    return a.left >= b.left - 0.5 && a.right <= b.right + 0.5 && a.top >= b.top - 0.5 && a.bottom <= b.bottom + 0.5;
  };
  const rows = [...document.querySelectorAll("tbody tr")].filter((tr) => tr.textContent.includes(mac));
  const row = rows[0];
  const line = row?.querySelector(".mac-line");
  const address = row?.querySelector("td.c-ip");
  const column = row?.querySelector("td.c-mac");
  const lines = [...document.querySelectorAll(".mac-line")];
  return {
    rows: rows.length,
    title: address?.getAttribute("title") ?? "",
    lineShown: shown(line),
    lineText: line?.textContent ?? "",
    lineWhole: shown(line) && whole(line) && inside(line, address),
    columnShown: shown(column),
    columnText: column?.textContent.trim() ?? "",
    columnWhole: shown(column) && whole(column),
    lines: lines.length,
    linesShown: lines.filter(shown).length,
    linesCut: lines.filter((el) => shown(el) && !(whole(el) && inside(el, el.closest("td")))).length,
    pageScrolls: document.documentElement.scrollWidth > window.innerWidth,
  };
}

/** Things the owner does that a still picture can't show, tried against the sample back end. Each starts
 *  from a fresh page, so from fresh sample data. Returns what went wrong. */
async function checkFlows(browser, url) {
  const card = (page, name) => page.locator("article", { hasText: name });
  const flows = [
    {
      what: "approving a first connection stores that identity and reads the machine",
      path: "/machines",
      async run(page) {
        const media = card(page, "Media PC");
        await media.getByText("The password has not been sent").waitFor();
        await media.getByRole("button", { name: "Approve and use the password" }).click();
        await media.getByRole("button", { name: "Yes, it matches" }).click();
        await media.getByText("RAM", { exact: true }).waitFor();
        await media.getByText("Before the stored password is used").waitFor({ state: "detached" });
        await page.getByRole("button", { name: "Edit Media PC" }).click();
        await page.getByText("Identity home-cc remembers").waitFor();
      },
    },
    {
      what: "an approval overtaken by a newer identity is refused, and the newer one is shown to judge",
      path: "/machines?awkward",
      async run(page) {
        const proxmox = card(page, "Proxmox");
        const now = proxmox.locator("dd").nth(1);
        const first = await now.textContent();
        await proxmox.getByRole("button", { name: "Approve the new identity" }).click();
        await proxmox.getByRole("button", { name: "Yes, it matches" }).click();
        await proxmox.getByText("Not approved:").waitFor();
        await proxmox.getByText("ssh-rsa SHA256:").waitFor();
        if ((await now.textContent()) === first) throw new Error("the card still shows the identity that was overtaken");
        await proxmox.getByRole("button", { name: "Approve the new identity" }).click();
        await proxmox.getByRole("button", { name: "Yes, it matches" }).click();
        await proxmox.getByText("This machine's identity has changed").waitFor({ state: "detached" });
        await proxmox.getByText(/^Updated/).waitFor();
      },
    },
    {
      what: '"Check now" is off while a scan runs, and a check asked for anyway says why it was refused',
      path: "/network",
      async run(page) {
        await page.getByRole("button", { name: "Scan network" }).click();
        const check = page.getByRole("button", { name: "Check now" });
        await page.waitForFunction(() => [...document.querySelectorAll("button")].some((b) => b.textContent.trim() === "Check now" && b.disabled));
        const title = await check.getAttribute("title");
        if (!/scan is running/i.test(title ?? "")) throw new Error(`the button doesn't say why it is off (title: "${title}")`);
        // As if pressed from somewhere that didn't know about the scan.
        await check.evaluate((b) => {
          b.disabled = false;
          b.click();
        });
        await page.getByText("Not checked: A scan is running").waitFor();
        await page.getByRole("button", { name: "Stop" }).click();
        await page.getByRole("button", { name: "Scan network" }).waitFor();
        // The round of checks that follows the scan makes the refusal old news.
        await page.getByText("Not checked:").waitFor({ state: "detached" });
      },
    },
    {
      what: "with pings not allowed, the standing explanation is shown once, also after Check now",
      path: "/network?noping",
      async run(page) {
        await page.getByText("Status checks aren't working").waitFor();
        await page.getByRole("button", { name: "Check now" }).click();
        await page.getByRole("button", { name: "Check now" }).waitFor();
        await page.waitForTimeout(300);
        if ((await page.getByText("/etc/sysctl.d").count()) !== 1) throw new Error("the explanation is missing or shown twice");
      },
    },
    {
      what: "the hardware address: emptied is removed, untouched is kept, and a bad address is explained",
      path: "/machines",
      async run(page) {
        const mac = page.getByLabel("Hardware address (optional)");
        const save = page.getByRole("button", { name: "Save", exact: true });
        await page.getByRole("button", { name: "Edit NAS" }).click();
        if ((await mac.inputValue()) !== "d8:bb:c1:7a:02:6e") throw new Error("the form didn't show the stored hardware address");
        await mac.fill("");
        await page.getByText("Saving removes the hardware address").waitFor();
        await save.click();
        await save.waitFor({ state: "detached" });
        await page.getByRole("button", { name: "Edit NAS" }).click();
        if ((await mac.inputValue()) !== "") throw new Error("the emptied hardware address came back");

        await page.getByRole("button", { name: "Edit Homelab VM" }).click();
        await page.getByLabel("Notes").fill("changed");
        await save.click();
        await save.waitFor({ state: "detached" });
        await page.getByRole("button", { name: "Edit Homelab VM" }).click();
        if ((await mac.inputValue()) !== "00:15:5d:44:0a:01") throw new Error("an untouched hardware address was lost");

        await page.getByLabel("Address (IP or hostname)").fill("192.168.1");
        await page.getByText("this hardware address is forgotten when you save").waitFor();
        await save.click();
        await page.locator("form").getByRole("alert").filter({ hasText: "needs four" }).waitFor();
      },
    },
    {
      what: "a new password is stored first for a machine that stays where it is, and last when its address or port changes",
      path: "/machines",
      async run(page) {
        await page.evaluate(recordCalls);
        /** What was asked of the back end since the last look, as far as saving goes. */
        const calls = () =>
          page.evaluate(() =>
            window.__calls
              .splice(0)
              .filter((c) => c === "set_secret" || c === "update_device")
              .join(", "),
          );
        const expectCalls = async (want, when) => {
          const got = await calls();
          if (got !== want) throw new Error(`${when}: asked the back end for "${got}", not "${want}"`);
        };
        const save = page.getByRole("button", { name: "Save", exact: true });
        const password = page.getByLabel("SSH password (optional)");
        const alert = page.locator("form").getByRole("alert");
        const edit = async (name) => {
          await page.getByRole("button", { name: `Edit ${name}` }).click();
          await page.waitForFunction((n) => document.querySelector("form input")?.value === n, name);
        };

        await edit("Homelab VM");
        await page.getByLabel("Notes").fill("same place, new password");
        await password.fill("first");
        await save.click();
        await save.waitFor({ state: "detached" });
        await expectCalls("set_secret, update_device", "same address and port");

        await edit("Homelab VM");
        await page.getByLabel("Address (IP or hostname)").fill("192.168.68.99");
        await password.fill("second");
        await save.click();
        await save.waitFor({ state: "detached" });
        await expectCalls("update_device, set_secret", "another address");

        await edit("Monitoring");
        await page.getByLabel("SSH port").fill("2222");
        await password.fill("third");
        await save.click();
        await save.waitFor({ state: "detached" });
        await expectCalls("update_device, set_secret", "another SSH port");

        // An address the back end refuses: nothing is saved, so the password must not be stored either.
        await edit("NAS");
        await page.getByLabel("Address (IP or hostname)").fill("192.168.1");
        await password.fill("fourth");
        await save.click();
        await alert.filter({ hasText: "needs four" }).waitFor();
        await expectCalls("update_device", "an address that is refused");
        await page.getByRole("button", { name: "Cancel", exact: true }).click();
        await save.waitFor({ state: "detached" });

        // Same place, but the rest of the form is refused: the password is in by then, and the form says so.
        await edit("NAS");
        await page.getByLabel("Hardware address (optional)").fill("zz");
        await password.fill("fifth");
        await save.click();
        await alert.filter({ hasText: "The new password was stored, but the rest wasn't saved: Hardware address should look like" }).waitFor();
        await expectCalls("set_secret, update_device", "a hardware address that is refused");
        if ((await password.inputValue()) !== "") throw new Error("the password that was stored is still in its box, to be stored again");
        await page.getByText("A password is stored").waitFor();
      },
    },
    {
      what: "an approval given while the machine is being read is held back, and is followed by a reading of its own",
      path: "/machines",
      async run(page) {
        await page.evaluate(recordCalls);
        const media = card(page, "Media PC");
        await media.getByRole("button", { name: "Approve and use the password" }).click();
        const yes = media.getByRole("button", { name: "Yes, it matches" });
        await yes.waitFor();
        // A reading starts before the approval is stored. It comes back still asking for the approval.
        await media.getByRole("button", { name: "Read stats for Media PC now" }).click();
        if (!(await yes.isDisabled())) throw new Error("the approval can be completed while the machine is being read");
        // As if it had been pressed all the same, while that reading was still going.
        await yes.evaluate((b) => {
          b.disabled = false;
          b.click();
        });
        await media.getByText("RAM", { exact: true }).waitFor();
        await media.getByText("Before the stored password is used").waitFor({ state: "detached" });
        const calls = await page.evaluate(() => window.__calls.filter((c) => c === "trust_host_key" || c === "refresh_stats").join(", "));
        if (calls !== "refresh_stats, trust_host_key, refresh_stats") {
          throw new Error(`expected a second reading after the approval, but the back end was asked for "${calls}"`);
        }
      },
    },
    {
      what: "a refused approval says why, also when the machine has nothing left to approve",
      path: "/machines",
      async run(page) {
        const proxmox = card(page, "Proxmox");
        await proxmox.getByRole("button", { name: "Approve the new identity" }).click();
        await proxmox.getByRole("button", { name: "Yes, it matches" }).waitFor();
        await page.evaluate(withdrawOffers);
        await proxmox.getByRole("button", { name: "Yes, it matches" }).click();
        await proxmox.getByText("This machine's identity has changed").waitFor({ state: "detached" });
        await proxmox.getByText("Couldn't reach this machine").waitFor();
        await proxmox.getByRole("alert").filter({ hasText: "Not approved: This machine isn't waiting for its identity to be approved." }).waitFor();
        if ((await proxmox.getByText("Not approved:").count()) !== 1) throw new Error("the reason is shown more than once");
        // Trying the machine again makes the refusal old news.
        await proxmox.getByRole("button", { name: "Read stats for Proxmox now" }).click();
        await proxmox.getByText("Not approved:").waitFor({ state: "detached" });
      },
    },
    {
      // A 1080p monitor at 125% scaling: the table is too narrow for the Hardware address column.
      what: "in a narrow window the hardware address is under the address, in full, and in the tooltip",
      path: "/network",
      viewport: { width: 1536, height: 864 },
      scale: 1.25,
      async run(page) {
        await page.locator("tbody tr", { hasText: NAS_MAC }).first().waitFor();
        const got = await page.evaluate(hardwareAddressShown, NAS_MAC);
        if (got.rows !== 1) throw new Error(`expected one row with the NAS's hardware address, found ${got.rows}`);
        if (got.columnShown) throw new Error("the Hardware address column is shown at this width: this check needs a width where it is not");
        if (!got.lineShown) throw new Error("no hardware address is shown in the NAS row");
        if (!got.lineText.includes(NAS_MAC)) throw new Error(`the line under the address reads "${got.lineText}"`);
        if (!got.lineWhole) throw new Error("the hardware address under the address is cut short");
        if (!got.title.includes(NAS_MAC)) throw new Error(`the address's tooltip doesn't have the hardware address (it reads "${got.title}")`);
        if (got.linesShown !== got.lines) throw new Error(`${got.lines - got.linesShown} of ${got.lines} rows don't show their hardware address`);
        if (got.linesCut > 0) throw new Error(`${got.linesCut} hardware addresses are cut short`);
        if (got.pageScrolls) throw new Error("the page scrolls sideways");
      },
    },
    {
      what: "in a wide window the hardware address has its own column, and is not repeated under the address",
      path: "/network",
      viewport: { width: 2560, height: 1440 },
      async run(page) {
        await page.locator("tbody tr", { hasText: NAS_MAC }).first().waitFor();
        const got = await page.evaluate(hardwareAddressShown, NAS_MAC);
        if (got.rows !== 1) throw new Error(`expected one row with the NAS's hardware address, found ${got.rows}`);
        if (!got.columnShown) throw new Error("the Hardware address column is missing");
        if (got.columnText !== NAS_MAC) throw new Error(`the Hardware address column reads "${got.columnText}"`);
        if (!got.columnWhole) throw new Error("the hardware address in its column is cut short");
        if (got.linesShown > 0) throw new Error("the hardware address is shown twice: in its column and under the address");
        if (!got.title.includes(NAS_MAC)) throw new Error(`the address's tooltip doesn't have the hardware address (it reads "${got.title}")`);
      },
    },
    {
      what: "the zoom keys, the wheel and the Settings buttons change the size, and it is kept after a restart",
      path: "/settings",
      async run(page) {
        const now = page.locator("output.zoom-now");
        const expectZoom = async (want, when) => {
          await page.waitForFunction((w) => document.querySelector("output.zoom-now")?.textContent === `${w}%`, want, { timeout: 3000 }).catch(() => {});
          const shown = await now.textContent();
          const applied = await page.evaluate(() => document.documentElement.style.zoom || "1");
          if (shown !== `${want}%` || Number(applied) !== want / 100) {
            throw new Error(`${when}: Settings shows ${shown} and the page is at ${applied}, not ${want}%`);
          }
        };
        await expectZoom(100, "at the start");
        await page.keyboard.press("Control+Equal");
        await expectZoom(110, "after Ctrl and =");
        // In a box too: the keys work everywhere.
        await page.getByLabel("How often to check that every device is there").focus();
        await page.keyboard.press("Control+Minus");
        await page.keyboard.press("Control+Minus");
        await expectZoom(90, "after Ctrl and - twice");
        await page.mouse.move(600, 400);
        await page.keyboard.down("Control");
        await page.mouse.wheel(0, -100);
        await page.keyboard.up("Control");
        await expectZoom(100, "after Ctrl and the wheel");
        await page.getByRole("button", { name: "Bigger" }).click();
        await page.getByRole("button", { name: "Bigger" }).click();
        await expectZoom(125, "after Bigger twice");
        await page.reload({ waitUntil: "networkidle" });
        await expectZoom(125, "after starting again");
        await page.keyboard.press("Control+Digit0");
        await expectZoom(100, "after Ctrl and 0");
        await page.reload({ waitUntil: "networkidle" });
        await expectZoom(100, "after starting again at 100%");
      },
    },
    {
      what: "the hostname under the form's address is never the one of an address no longer in the field",
      path: "/machines",
      async run(page) {
        await page.getByRole("button", { name: "Edit NAS" }).click();
        const line = page.locator("form p.resolved");
        await line.getByText("NAS", { exact: true }).waitFor();
        await page.getByLabel("Address (IP or hostname)").fill("192.168.68.30");
        const right = await line.textContent();
        if (/\bNAS\b/.test(right ?? "")) throw new Error(`the line under 192.168.68.30 reads "${right?.trim()}"`);
        await line.getByText("pve", { exact: true }).waitFor();
      },
    },
    {
      what: "a double-click on an address or a hostname copies all of it and says so",
      path: "/machines",
      async run(page) {
        // What was put on the clipboard, whichever way the page got it there.
        await page.evaluate(() => {
          window.__copied = [];
          const write = (t) => (window.__copied.push(t), Promise.resolve());
          Object.defineProperty(navigator, "clipboard", { value: { writeText: write }, configurable: true });
        });
        const nas = card(page, "NAS");
        for (const [what, locator, want] of [
          ["address", nas.locator(".address .ip"), "192.168.68.10"],
          ["hostname", nas.locator(".address .hostname"), null],
        ]) {
          await locator.waitFor();
          const text = want ?? (await locator.textContent())?.trim();
          if (!text || text === "looking…") throw new Error(`NAS's ${what} has no text to copy`);
          await locator.dblclick();
          await page.locator(".note", { hasText: "Copied" }).waitFor({ timeout: 2000 });
          const got = await page.evaluate(() => ({ copied: window.__copied.at(-1), selected: getSelection()?.toString().trim() }));
          if (got.copied !== text) throw new Error(`a double-click on NAS's ${what} copied "${got.copied}", not "${text}"`);
          if (got.selected !== text) throw new Error(`a double-click on NAS's ${what} selected "${got.selected}", not all of it`);
        }
      },
    },
    {
      what: "on the Overview each Deco mesh node's hostname is shown in full",
      path: "/",
      viewport: { width: 1536, height: 864 },
      scale: 1.25,
      async run(page) {
        await page.locator(".mesh-node .hostname").first().waitFor();
        const got = await page.evaluate(() =>
          [...document.querySelectorAll(".mesh-node")].map((n) => {
            const h = n.querySelector(".hostname");
            return { node: n.querySelector(".card-name")?.textContent?.trim(), text: h?.textContent?.trim() ?? "", whole: !!h && h.scrollWidth <= h.clientWidth };
          }),
        );
        if (got.length === 0) throw new Error("no mesh nodes on the Overview");
        for (const n of got) {
          if (!n.text) throw new Error(`${n.node} has no hostname`);
          if (!n.whole) throw new Error(`${n.node}'s hostname "${n.text}" is cut short`);
        }
      },
    },
    // Terminals
    {
      what: "a machine's terminal button opens a tab that keeps running on other screens, and closing it ends the connection",
      path: "/machines",
      async run(page) {
        await page.evaluate(recordCalls);
        await page.getByRole("button", { name: "Open a terminal on Homelab VM" }).click();
        await page.waitForURL("**/terminals");
        await terminalShows(page, "Try ls");
        await page.locator('[role="tabpanel"]:not([hidden]) .xterm').click();
        await page.keyboard.type("ls\r");
        await terminalShows(page, "update.sh");
        // Away and back: the same terminal, still showing what it showed, with no new connection.
        await page.getByRole("link", { name: "Machines" }).click();
        await page.locator("article").first().waitFor();
        await page.getByRole("link", { name: "Terminals" }).click();
        await terminalShows(page, "update.sh");
        await page.keyboard.type("size\r");
        await terminalShows(page, "columns");
        const opened = await page.evaluate(() => window.__calls.filter((c) => c === "terminal_open").length);
        if (opened !== 1) throw new Error(`the terminal was opened ${opened} times`);
        await page.getByRole("button", { name: "Close Homelab VM" }).click();
        await page.getByText("No terminals open").waitFor();
        if (!(await page.evaluate(() => window.__calls.includes("terminal_close")))) {
          throw new Error("closing the tab didn't close the connection");
        }
      },
    },
    {
      what: "in a terminal, Ctrl+Shift+V pastes, Ctrl+C without a selection interrupts, and exit ends the session",
      path: "/terminals",
      async run(page) {
        await openTerminalTab(page, "Dev VM (sam@192.168.68.22)");
        await terminalShows(page, "Try ls");
        await page.locator('[role="tabpanel"]:not([hidden]) .xterm').click();
        await page.keyboard.press("Control+Shift+V");
        await terminalShows(page, "echo pasted from the clipboard");
        await page.keyboard.press("Control+c");
        await terminalShows(page, "^C");
        await page.keyboard.type("exit\r");
        await page.getByText("Session ended.").waitFor();
        await page.getByRole("button", { name: "Reconnect" }).click();
        await page.getByText("Session ended.").waitFor({ state: "detached" });
        await page.getByRole("tab", { selected: true, name: "Dev VM" }).waitFor();
      },
    },
    {
      what: "a new terminal takes the keyboard by itself, also what is typed while it connects",
      path: "/machines",
      async run(page) {
        await page.getByRole("button", { name: "Open a terminal on Homelab VM" }).click();
        await page.waitForURL("**/terminals");
        // No click in the terminal first: typing straight away has to reach it.
        await page.keyboard.type("size\r");
        await terminalShows(page, "columns");
        await page.keyboard.type("ls\r");
        await terminalShows(page, "update.sh");
      },
    },
    {
      what: "a terminal whose connection is lost says so as a problem that can be handed on, and can reconnect",
      path: "/terminals",
      async run(page) {
        await openTerminalTab(page, "Dev VM (sam@192.168.68.22)");
        await terminalShows(page, "Try ls");
        await page.keyboard.type("drop\r");
        const problem = page.getByRole("alert").filter({ hasText: "was lost" });
        await problem.waitFor();
        if (!(await problem.getByText("Session ended. The connection to").count())) {
          throw new Error("the lost connection isn't shown as a problem");
        }
        await page.getByRole("button", { name: "Reconnect" }).click();
        await problem.waitFor({ state: "detached" });
        await terminalShows(page, "Try ls");
      },
    },
    // --- Docker (spec 5.5) ---
    {
      what: "Docker: a container is restarted, its log followed and searched, and closing the log stops following it",
      path: "/docker",
      async run(page) {
        await page.evaluate(recordCalls);
        await page.getByRole("button", { name: "Restart media-sonarr-1" }).click();
        await page.getByText("Restarting…").waitFor();
        await page.getByText("Restarting…").waitFor({ state: "detached" });
        await page.getByRole("button", { name: "Logs of media-jellyfin-1" }).click();
        await page.locator(".line").nth(10).waitFor();
        await page.getByLabel("Search the log").fill("ERR");
        await page.getByText(/lines hold “ERR”/).waitFor();
        const texts = await page.locator(".line .text").allTextContents();
        if (!texts.length || texts.some((t) => !t.toLowerCase().includes("err"))) throw new Error("the search shows lines without the text");
        await page.getByRole("button", { name: "Close the log" }).click();
        await page.waitForFunction(() => window.__calls.includes("docker_logs_stop"));
      },
    },
    {
      what: "Docker: a running container's terminal opens a tab on its host that runs docker exec, and a stopped one has none",
      path: "/docker",
      async run(page) {
        await page.evaluate(() => {
          const tauri = window.__TAURI_INTERNALS__;
          const invoke = tauri.invoke;
          window.__opened = [];
          tauri.invoke = (cmd, args, ...rest) => {
            if (cmd === "terminal_open") window.__opened.push(args);
            return invoke(cmd, args, ...rest);
          };
        });
        if (await page.getByRole("button", { name: "Open a terminal in media-prowlarr-1" }).count()) {
          throw new Error("a stopped container offers a terminal");
        }
        await page.getByRole("button", { name: "Open a terminal in media-jellyfin-1" }).click();
        await page.waitForURL("**/terminals");
        await page.getByRole("tab", { selected: true, name: "media-jellyfin-1 on Homelab VM" }).waitFor();
        await terminalShows(page, "Entered the container media-jellyfin-1");
        const opened = await page.evaluate(() => window.__opened);
        const want = "docker exec -it media-jellyfin-1 sh -c 'command -v bash >/dev/null && exec bash || exec sh'";
        if (opened.length !== 1 || opened[0].command !== want) throw new Error(`opened ${JSON.stringify(opened)}`);
      },
    },
    {
      what: "Docker: stopping and removing a container ask first, in its row",
      path: "/docker",
      async run(page) {
        await page.evaluate(recordCalls);
        await page.getByRole("button", { name: "Stop media-radarr-1" }).click();
        await page.getByText("Stop radarr?").waitFor();
        if (await page.evaluate(() => window.__calls.includes("docker_container_action"))) throw new Error("it stopped without asking");
        await page.getByRole("button", { name: "Yes, stop" }).click();
        await page.getByRole("button", { name: "Start media-radarr-1" }).waitFor();
        await page.getByRole("button", { name: "Remove old-backup" }).click();
        await page.getByRole("button", { name: "Cancel", exact: true }).click();
        await page.getByRole("button", { name: "Remove old-backup" }).waitFor();
      },
    },
    {
      what: "Docker: a cleanup shows what it would remove, leaves named volumes unticked, and removes only after a yes",
      path: "/docker",
      async run(page) {
        await page.getByRole("button", { name: "Clean up…" }).click();
        await page.getByText("Unused volumes").waitFor();
        if (await page.getByRole("checkbox", { name: /nextcloud_db/ }).isChecked()) throw new Error("a named volume was ticked");
        const remove = page.getByRole("button", { name: /^Remove \d+ things/ });
        await remove.click();
        await page.getByRole("button", { name: "Yes, remove them from Homelab VM" }).click();
        await page.getByText(/^Removed \d+ things?, freeing about/).waitFor();
        await page.getByText("postgres:14: Error response from daemon").waitFor();
      },
    },
    {
      what: "Docker: a compose file is saved after a confirm, says where the old one went, and offers to start the stack",
      path: "/docker",
      async run(page) {
        await page.getByRole("article", { name: "Stack media" }).getByRole("button", { name: "Files" }).click();
        const box = page.locator("textarea");
        await box.waitFor();
        await box.fill((await box.inputValue()).replace("10.10", "10.11"));
        await page.getByRole("button", { name: "Save", exact: true }).click();
        await page.getByRole("button", { name: "Yes, save to Homelab VM" }).click();
        await page.getByText(/The old version is at \/srv\/media\/compose\.yml\.\d{8}-\d{6}\.bak/).waitFor();
        await page.getByRole("button", { name: "Start the stack with it" }).waitFor();
      },
    },
    // --- Proxmox ---
    {
      what: "Proxmox: approving the certificate on first contact shows the VMs",
      path: "/proxmox?pvecert",
      async run(page) {
        await page.getByText("check its certificate").waitFor();
        if ((await page.locator("table").count()) !== 0) throw new Error("guests are shown before the certificate is approved");
        await page.getByRole("button", { name: "Approve this certificate" }).click();
        await page.getByRole("button", { name: "Yes, it matches" }).click();
        await page.getByText("check its certificate").waitFor({ state: "detached" });
        await page.getByText("homeassistant", { exact: true }).waitFor();
      },
    },
    {
      what: "Proxmox: a stopped VM starts, a snapshot is taken, and force stop asks first",
      path: "/proxmox",
      async run(page) {
        const row = page.locator("tbody tr", { hasText: "win11-test" });
        await row.getByRole("button", { name: "Start", exact: true }).click();
        await row.getByText("Running").waitFor();

        await page.getByRole("button", { name: "Manage win11-test" }).click();
        const panel = page.getByRole("complementary");
        await panel.getByText("No snapshots yet.").waitFor();
        await panel.getByLabel("Name").fill("bad name");
        await panel.getByRole("button", { name: "Take snapshot" }).click();
        await panel.getByText("no spaces").waitFor();
        await panel.getByLabel("Name").fill("before_test");
        await panel.getByRole("button", { name: "Take snapshot" }).click();
        await panel.getByText('Snapshot "before_test" taken.').waitFor();
        await panel.getByText("before_test", { exact: true }).waitFor();

        await panel.getByRole("button", { name: "Force stop" }).click();
        await panel.getByRole("button", { name: "Yes, force stop" }).waitFor();
        if (!(await row.getByText("Running").isVisible())) throw new Error("the first click on Force stop already stopped it");
        await panel.getByRole("button", { name: "Yes, force stop" }).click();
        await row.getByText("Stopped").waitFor();
      },
    },
    {
      what: "Proxmox: shut down asks first, also in the list",
      path: "/proxmox",
      async run(page) {
        const row = page.locator("tbody tr", { hasText: "pihole2" });
        await row.getByRole("button", { name: "Shut down", exact: true }).click();
        await row.getByRole("button", { name: "Yes, shut down" }).waitFor();
        if (!(await row.getByText("Running").isVisible())) throw new Error("the first click on Shut down already shut it down");
        await row.getByRole("button", { name: "Cancel" }).click();
        await row.getByRole("button", { name: "Shut down", exact: true }).waitFor();

        await page.getByRole("button", { name: "Manage pihole2" }).click();
        const panel = page.getByRole("complementary");
        await panel.getByRole("button", { name: "Restart" }).click();
        await panel.getByRole("button", { name: "Yes, restart" }).waitFor();
        // Another guest's panel doesn't inherit the first click.
        await page.getByRole("button", { name: "Manage pihole1" }).click();
        await panel.getByRole("heading", { name: "pihole1" }).waitFor();
        if (await panel.getByRole("button", { name: "Yes, restart" }).count()) throw new Error("a first click carried over to another guest");
      },
    },
    {
      what: "Proxmox: rolling a running VM back to a snapshot without RAM starts it again",
      path: "/proxmox",
      async run(page) {
        await page.getByRole("button", { name: "Manage homeassistant" }).click();
        const panel = page.getByRole("complementary");
        const snap = panel.locator("li", { hasText: "after_setup" });
        await snap.getByRole("button", { name: "Roll back" }).click();
        await snap.getByRole("button", { name: "Yes, roll back and restart" }).click();
        await panel.getByText('Rolled back to "after_setup" and started again.').waitFor();
        await page.locator("tbody tr", { hasText: "homeassistant" }).getByText("Running").waitFor();
      },
    },
    {
      what: "Proxmox: a paused VM and a hibernated VM can be resumed",
      path: "/proxmox?pvestates",
      async run(page) {
        const paused = page.locator("tbody tr", { hasText: "homeassistant" });
        await paused.getByText("Paused").waitFor();
        if (await paused.getByRole("button", { name: "Shut down" }).count()) throw new Error("Shut down is offered for a paused VM");
        await paused.getByRole("button", { name: "Resume" }).click();
        await paused.getByText("Running").waitFor();

        const hibernated = page.locator("tbody tr", { hasText: "win11-test" });
        await hibernated.getByText("Hibernated").waitFor();
        const resume = hibernated.getByRole("button", { name: "Resume" });
        if (await resume.isDisabled()) throw new Error("a hibernated VM's Resume is held back by its lock");
        await resume.click();
        await hibernated.getByText("Running").waitFor();
      },
    },
    {
      what: "Proxmox: a container's console runs pct enter on the host, and a VM offers its serial console or a terminal on its machine",
      path: "/machines",
      async run(page) {
        await openProxmoxScreenLoggedIn(page);
        await page.evaluate(() => {
          const tauri = window.__TAURI_INTERNALS__;
          const invoke = tauri.invoke;
          window.__opened = [];
          tauri.invoke = (cmd, args, ...rest) => {
            if (cmd === "terminal_open") window.__opened.push(args);
            return invoke(cmd, args, ...rest);
          };
        });
        const panel = page.getByRole("complementary");
        await page.getByRole("button", { name: "Manage homelab" }).click();
        await panel.getByText("needs a serial port set up on the VM").waitFor();
        await panel.getByRole("button", { name: "Serial console" }).waitFor();
        await panel.getByRole("button", { name: "Terminal on Homelab VM" }).click();
        await page.waitForURL("**/terminals");
        await page.getByRole("tab", { selected: true, name: "Homelab VM" }).waitFor();
        await page.getByRole("link", { name: "Proxmox", exact: true }).click();
        await page.getByRole("button", { name: "Manage pihole1" }).click();
        await panel.getByRole("button", { name: "Open console" }).click();
        await page.waitForURL("**/terminals");
        await page.getByRole("tab", { selected: true, name: "pihole1 (CT 101)" }).waitFor();
        await terminalShows(page, "Entered container 101");
        const opened = await page.evaluate(() => window.__opened.map((o) => o.command ?? null));
        if (JSON.stringify(opened) !== JSON.stringify([null, "pct enter 101"])) throw new Error(`opened ${JSON.stringify(opened)}`);
        // A stopped guest has no console.
        await page.getByRole("link", { name: "Proxmox", exact: true }).click();
        await page.getByRole("button", { name: "Manage win11-test" }).click();
        await panel.getByRole("heading", { name: "win11-test" }).waitFor();
        if (await panel.getByRole("button", { name: "Serial console" }).count()) throw new Error("a stopped VM offers a console");
      },
    },
    {
      what: "Proxmox: without an SSH user for the host, the console says why and links to adding one",
      path: "/proxmox?pvenossh",
      async run(page) {
        await page.getByRole("button", { name: "Manage pihole1" }).click();
        const panel = page.getByRole("complementary");
        if (!(await panel.getByRole("button", { name: "Open console" }).isDisabled())) throw new Error("the console can be opened without an SSH user");
        await panel.getByRole("link", { name: "Add an SSH user to Proxmox" }).click();
        await page.waitForURL("**/machines?edit=*");
        await page.getByRole("button", { name: "Save", exact: true }).waitFor();
      },
    },
    {
      what: "Proxmox: a token whose secret is missing says so",
      path: "/proxmox?pvenosecret",
      async run(page) {
        await page.getByText("How to make a token").waitFor();
        await page.getByRole("alert").filter({ hasText: "secret isn't stored" }).waitFor();
      },
    },
    // --- end Proxmox ---
  ];
  const problems = [];
  for (const flow of flows) {
    const page = await browser.newPage({
      viewport: flow.viewport ?? { width: 1920, height: 1080 },
      deviceScaleFactor: flow.scale ?? 1,
    });
    page.setDefaultTimeout(10_000);
    const errors = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
    try {
      await page.goto(url + flow.path, { waitUntil: "networkidle" });
      await page.evaluate(() => document.fonts.ready);
      await flow.run(page);
    } catch (e) {
      problems.push(`${flow.what}: ${String(e.message ?? e).split("\n")[0]}`);
    }
    for (const error of errors) problems.push(`${flow.what}: error in the page: ${error}`);
    await page.close();
    console.log(`${problems.some((p) => p.startsWith(flow.what)) ? "FAIL" : "ok  "} ${flow.what}`);
  }
  return problems;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (!args.out) {
    console.error("usage: node tools/screenshots.mjs <output folder> [--url URL] [--only names] [--sizes names] [--zoom percent]");
    process.exit(2);
  }
  const out = resolve(args.out);
  mkdirSync(out, { recursive: true });
  const screens = args.only ? SCREENS.filter((s) => args.only.split(",").includes(s.name)) : SCREENS;
  let sizes = args.sizes ? args.sizes.split(",").map(parseSize) : SIZES;
  if (args.zoom) {
    const percent = Number(args.zoom);
    if (!(percent >= 50 && percent <= 200)) throw new Error(`--zoom takes a percentage from 50 to 200, not "${args.zoom}".`);
    sizes = sizes.map((s) => zoomed(s, percent));
  }

  const chromium = await loadChromium();
  const server = args.url ? { url: args.url.replace(/\/$/, ""), stop() {} } : await startDevServer();
  const failures = [];
  // Launched inside the try: if the browser can't start, the dev server must still be stopped.
  let browser;
  try {
    browser = await chromium.launch();
    // The first visit makes the dev server prepare each screen (and may reload the page once); do that before measuring.
    const warm = await browser.newPage();
    for (const path of [...new Set(SCREENS.map((s) => s.path))]) {
      await warm.goto(server.url + path, { waitUntil: "networkidle" });
      await warm.waitForTimeout(500);
    }
    await warm.close();

    for (const size of sizes) {
      const context = await browser.newContext({
        viewport: { width: size.width, height: size.height },
        deviceScaleFactor: size.scale,
        reducedMotion: "reduce",
      });
      for (const screen of screens) {
        const page = await context.newPage();
        const errors = [];
        page.on("pageerror", (e) => errors.push(String(e)));
        page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
        await page.goto(server.url + screen.path, { waitUntil: "networkidle" });
        await page.waitForSelector(screen.ready);
        await page.evaluate(() => document.fonts.ready);
        await screen.before?.(page);

        const label = `${screen.name} @ ${size.name}`;
        const found = await page.evaluate(inspect);
        for (const problem of found.overflow) failures.push(`${label}: ${problem}`);
        for (const error of errors) failures.push(`${label}: error in the page: ${error}`);
        const file = join(out, `${screen.name}-${size.name}.png`);
        await page.screenshot({ path: file });
        console.log(`${found.overflow.length ? "FAIL" : "ok  "} ${label}${found.cut.length ? `  (${found.cut.length} texts cut short)` : ""}`);
        if (process.env.SHOW_CUT) for (const c of found.cut) console.log(`       … ${c}`);

        await screen.after?.(page);
        if (found.tall > 0) {
          // Content below the fold: a second, taller picture shows all of it at the same width.
          await page.setViewportSize({ width: size.width, height: size.height + found.tall });
          await page.screenshot({ path: join(out, `${screen.name}-${size.name}-full.png`) });
        }
        await page.close();
      }
      await context.close();
    }

    const awkward = await checkAwkward(browser, server.url);
    failures.push(...awkward);
    console.log(`${awkward.length ? "FAIL" : "ok  "} unusual readings (two disks on one folder)`);
    failures.push(...(await checkFlows(browser, server.url)));
  } finally {
    try {
      await browser?.close();
    } finally {
      server.stop();
    }
  }

  if (failures.length > 0) {
    console.error(`\n${failures.length} problem(s):`);
    for (const f of failures) console.error(`  ${f}`);
    process.exit(1);
  }
  console.log(`\nAll ${sizes.length * screens.length} screenshots saved to ${out}, none wider than its window.`);
}

main().catch((e) => {
  console.error(e.stack ?? String(e));
  process.exit(1);
});
