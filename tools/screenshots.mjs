// Saves a screenshot of every screen at each of the owner's monitor sizes,
// using the sample back end (src/lib/mock), and checks that nothing scrolls
// sideways (or the page itself grows taller than the window). Needs Playwright with Chromium, installed globally or locally.
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
  // With the Containers panel's count of image updates, which comes after the hosts are read and checked.
  { name: "overview", path: "/", ready: ".card", before: (page) => page.getByText("6 updates", { exact: true }).waitFor({ timeout: 15_000 }) },
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
  // Machine groups: a heading per group. The sample has "NAS PC" and "Proxmox lab", and some machines in none.
  {
    name: "machines-group-collapsed",
    path: "/machines",
    ready: "article",
    async before(page) {
      await page.getByRole("region", { name: "Proxmox lab", exact: true }).getByRole("button", { name: /^Proxmox lab/ }).click();
      await page.locator("article", { hasText: "Pi-hole 1" }).waitFor({ state: "detached" });
    },
    // The pictures share the window's storage: open it again for the ones after this.
    async after(page) {
      await page.getByRole("region", { name: "Proxmox lab", exact: true }).getByRole("button", { name: /^Proxmox lab/ }).click();
    },
  },
  {
    name: "machines-group-menu",
    path: "/machines",
    ready: "article",
    async before(page) {
      await page.getByRole("button", { name: "Group for Media PC" }).click();
      await page.getByRole("menu").waitFor();
    },
  },
  {
    name: "machines-group-form",
    path: "/machines",
    ready: "article",
    async before(page) {
      await page.getByRole("button", { name: "Edit Main PC" }).click();
      await page.getByLabel("Group (optional)").fill("nas pc");
      await page.getByText("Shown with the 3 other machines in NAS PC.").waitFor();
    },
  },
  {
    name: "machines-group-rename",
    path: "/machines",
    ready: "article",
    async before(page) {
      await page.getByRole("button", { name: "Rename the group NAS PC" }).click();
      await page.getByLabel("New name for NAS PC").fill("proxmox LAB");
      await page.getByText("Proxmox lab already exists").waitFor();
    },
  },
  {
    name: "machines-ungroup",
    path: "/machines",
    ready: "article",
    async before(page) {
      const nas = page.getByRole("region", { name: "NAS PC", exact: true });
      await nas.getByRole("button", { name: "Ungroup" }).click();
      await nas.getByRole("button", { name: "Yes, ungroup" }).waitFor();
    },
  },
  { name: "machines-no-groups", path: "/machines?nogroups", ready: "article" },
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
  // --- Pi-hole (see src/lib/mock/pihole.ts for the "?ph…" words) ---
  { name: "pihole", path: "/pihole", ready: "article", before: (page) => page.getByText("Only on Pi-hole 1").first().waitFor() },
  {
    name: "pihole-pause",
    path: "/pihole",
    ready: "article",
    async before(page) {
      const card = page.getByRole("article", { name: "Pi-hole Pi-hole 1" });
      await card.getByRole("button", { name: "Pause blocking…" }).click();
      await card.getByRole("button", { name: "5 minutes" }).click();
      await card.getByRole("button", { name: "Yes, pause blocking" }).waitFor();
    },
  },
  { name: "pihole-paused", path: "/pihole?phoff", ready: "article", before: (page) => page.getByText(/back on in/).waitFor() },
  { name: "pihole-cert", path: "/pihole?phcert", ready: "article", before: (page) => page.getByText("check its certificate").waitFor() },
  { name: "pihole-cert-changed", path: "/pihole?phchanged", ready: "article", before: (page) => page.getByText("shows a different certificate from the one you approved").first().waitFor() },
  { name: "pihole-wrong-password", path: "/pihole?phwrong", ready: "article", before: (page) => page.getByText("didn't accept the password").first().waitFor() },
  { name: "pihole-down", path: "/pihole?phdown", ready: "article", before: (page) => page.getByText("Nothing answered at").waitFor() },
  {
    name: "pihole-setup-http",
    path: "/pihole?phnopw",
    ready: "form",
    async before(page) {
      await page.getByLabel("HTTP: only if this Pi-hole doesn't offer HTTPS.").check();
      await page.getByText("cross your network unencrypted").waitFor();
    },
  },
  {
    name: "pihole-add",
    path: "/pihole?phnone",
    ready: ".add",
    async before(page) {
      await page.getByRole("button", { name: "Look for Pi-holes on my machines" }).click();
      await page.getByText("Answers like Pi-hole v5").waitFor();
    },
  },
  { name: "overview-pihole", path: "/?phoff&phdown", ready: ".card", before: (page) => page.getByText(/Pi-hole 1: paused, back on in/).waitFor() },
  // --- end Pi-hole ---
  // --- Tables: sorted, grouped and filtered (components/table). Chosen with the bar above each table, which
  // is there at every width (a narrow window hides the headers). The sort and grouping are kept in the
  // window's storage, which the pictures share, so each puts them back afterwards. ---
  {
    name: "network-table-sorted",
    path: "/network",
    ready: "table",
    async before(page) {
      const devices = panelOf(page, "Devices");
      await devices.getByLabel("Sort devices").selectOption("address:desc");
      await devices.locator("tbody tr").first().filter({ hasText: "192.168.68.102" }).waitFor();
      await panelOf(page, "Mesh nodes").getByLabel("Sort mesh nodes").selectOption("name:asc");
    },
    after: forgetTableChoices,
  },
  {
    name: "network-table-grouped",
    path: "/network",
    ready: "table",
    async before(page) {
      const devices = panelOf(page, "Devices");
      await devices.getByLabel("Group devices").selectOption("type");
      await devices.getByLabel("Sort devices").selectOption("response:asc");
      await devices.getByRole("button", { name: /^Container/ }).click();
      await devices.getByRole("button", { name: /^Container/, expanded: false }).waitFor();
    },
    after: forgetTableChoices,
  },
  {
    name: "network-table-filtered",
    path: "/network",
    ready: "table",
    async before(page) {
      const devices = panelOf(page, "Devices");
      await devices.getByLabel("Filter devices").fill("68.3");
      await devices.getByText("4 of 14 shown").waitFor();
      const mesh = panelOf(page, "Mesh nodes");
      await mesh.getByLabel("Filter mesh nodes").fill("garden");
      await mesh.getByText('No mesh nodes match "garden".').waitFor();
    },
  },
  {
    name: "machines-table-grouped",
    path: "/machines",
    ready: "table",
    async before(page) {
      const all = panelOf(page, "All machines");
      await all.getByLabel("Group machines").selectOption("group");
      await all.getByLabel("Show only machines with this Type").selectOption("Container");
      await all.getByRole("button", { name: /^Proxmox lab/ }).waitFor();
    },
    after: forgetTableChoices,
  },
  {
    name: "proxmox-table-grouped",
    path: "/proxmox",
    ready: "table",
    async before(page) {
      const guests = page.getByRole("region", { name: "VMs and containers" });
      await guests.getByLabel("Group VMs and containers").selectOption("type");
      await guests.getByLabel("Sort VMs and containers").selectOption("id:desc");
      await guests.getByLabel("Filter VMs and containers").fill("dns");
      await guests.getByText("2 of 6 shown").waitFor();
    },
    after: forgetTableChoices,
  },
  {
    name: "docker-table-grouped",
    path: "/docker",
    ready: "article",
    async before(page) {
      await page.getByText("612 MB").waitFor();
      const media = page.getByRole("article", { name: "Stack media" });
      await media.getByLabel("Group containers of media").selectOption("state");
      await media.getByLabel("Sort containers of media").selectOption("memory:desc");
      const others = page.getByRole("article", { name: "Other containers" });
      await others.getByLabel("Filter other containers").fill("old-backup");
      await others.getByText(/^1 of \d+ shown$/).waitFor();
    },
    after: forgetTableChoices,
  },
  // The old laptop silenced on the Overview, and the TV not yet (a tile's icon button). Kept in the window's
  // storage, so forgotten again afterwards.
  {
    name: "overview-silenced",
    path: "/",
    ready: ".card",
    async before(page) {
      await page.getByRole("button", { name: "Silence Old laptop" }).click();
      await page.getByRole("button", { name: "Undo silencing Old laptop" }).waitFor();
      await page.getByRole("button", { name: "Silence Living room TV" }).waitFor();
    },
    after: forgetSilences,
  },
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
    // CPU and memory are read after the list, and the images checked for updates; the picture should have them.
    async before(page) {
      await page.getByText("612 MB").waitFor();
      await page.getByText("5 updates in 2 stacks and 1 other container").waitFor();
    },
  },
  {
    name: "docker-updates",
    path: "/docker",
    ready: "article",
    async before(page) {
      await page.getByRole("button", { name: "Update all…" }).click();
      await page.getByRole("heading", { name: "Updates on Homelab VM" }).waitFor();
    },
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

/** A panel on a screen, by its heading: the "Devices" table on the Network screen, say. */
function panelOf(page, heading) {
  return page.locator(".panel", { has: page.getByRole("heading", { name: heading, exact: true }) });
}

/** Forgets every table's sort and grouping kept in the window's storage. */
async function forgetTableChoices(page) {
  await page.evaluate(() => {
    for (const key of Object.keys(localStorage)) if (key.startsWith("home-cc.table.")) localStorage.removeItem(key);
  });
}

/** Forgets the machines silenced on the Overview. */
async function forgetSilences(page) {
  await page.evaluate(() => localStorage.removeItem("home-cc.silenced"));
}

/** Runs in the page. From now on the back end reports the device of this name as up, seen just now (true),
 *  or as it really is (false). */
function reportUp([name, up]) {
  const tauri = window.__TAURI_INTERNALS__;
  tauri.realInvoke ??= tauri.invoke;
  const invoke = tauri.realInvoke;
  if (!up) {
    tauri.invoke = invoke;
    return;
  }
  tauri.invoke = async (cmd, ...rest) => {
    const result = await invoke(cmd, ...rest);
    if (cmd !== "device_statuses") return result;
    const id = (await invoke("list_devices")).find((d) => d.name === name)?.id;
    const now = Math.floor(Date.now() / 1000);
    return result.map((s) => (s.deviceId === id ? { ...s, up: true, rttMs: 3.2, checkedAt: now, lastSeen: now } : s));
  };
}

/** The text of one column's cells (by its class, such as "c-name"), top to bottom, in the rows shown. */
async function cellTexts(scope, cls, inner = "") {
  return (await scope.locator(`tbody tr:not(.group-row) td.${cls} ${inner}`).allInnerTexts()).map((t) => t.trim());
}

/** Waits for a column's cells to read `want`, top to bottom. */
async function expectCells(scope, cls, want, what, inner = "") {
  const until = Date.now() + 5000;
  let got;
  for (;;) {
    got = await cellTexts(scope, cls, inner);
    if (JSON.stringify(got) === JSON.stringify(want)) return;
    if (Date.now() > until) throw new Error(`${what}: ${JSON.stringify(got)}, not ${JSON.stringify(want)}`);
    await new Promise((r) => setTimeout(r, 100));
  }
}

/** Goes to another screen with the sidebar, waits until it is shown, and comes back. */
async function awayAndBack(page, away, back) {
  await page.getByRole("link", { name: away, exact: true }).click();
  await page.getByRole("heading", { level: 1, name: away, exact: true }).waitFor();
  await page.getByRole("link", { name: back, exact: true }).click();
  await page.getByRole("heading", { level: 1, name: back, exact: true }).waitFor();
}

/** Waits for a form field to hold the value (it may be filled a moment after the screen shows). */
async function expectValue(field, want, what) {
  const until = Date.now() + 5000;
  let got;
  for (;;) {
    got = (await field.count()) ? await field.inputValue() : "(not on screen)";
    if (got === want) return;
    if (Date.now() > until) throw new Error(`${what} was "${got.slice(0, 80)}", not "${want.slice(0, 80)}"`);
    await new Promise((r) => setTimeout(r, 100));
  }
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
  // Only the screen, the sidebar and panels scroll: a page taller than the window is a second scrollbar on
  // the right. app.css clips the page so the owner never sees one; measured without that, so the cause shows.
  const keep = [doc.style.overflow, document.body.style.overflow];
  doc.style.overflow = document.body.style.overflow = "visible";
  if (doc.scrollHeight > window.innerHeight) overflow.push(`the page is ${doc.scrollHeight}px tall in a ${window.innerHeight}px window (a second scrollbar)`);
  [doc.style.overflow, document.body.style.overflow] = keep;
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
  // Machine groups: a group's part of the Machines screen, its heading texts, and the cards in it in order.
  const group = (page, name) => page.getByRole("region", { name, exact: true });
  const headings = (page) =>
    page.evaluate(() => [...document.querySelectorAll(".group-title")].map((h) => h.textContent.trim().replace(/\s+/g, " ")));
  const cardsIn = (page, name) => group(page, name).locator("article h3").allTextContents();
  // Pi-hole: one Pi-hole's card on the Pi-hole screen.
  const piholeCard = (page, name) => page.getByRole("article", { name: `Pi-hole ${name}`, exact: true });
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
    // Machine groups.
    {
      what: "groups: the cards are under a heading per group by name, the ungrouped last, and the table has a Group column",
      path: "/machines",
      async run(page) {
        await page.locator("article").first().waitFor();
        const got = await headings(page);
        const want = ["NAS PC · 3", "Proxmox lab · 4", "Other machines · 5"];
        if (got.join("|") !== want.join("|")) throw new Error(`the headings read ${JSON.stringify(got)}`);
        const inLab = await cardsIn(page, "Proxmox lab");
        if (inLab.join() !== "Monitoring,Pi-hole 1,Pi-hole 2,Proxmox") throw new Error(`Proxmox lab holds ${inLab.join(", ")}`);
        const cell = page.locator("tbody tr", { hasText: "Pi-hole 2" }).locator("td.c-group");
        if ((await cell.textContent())?.trim() !== "Proxmox lab") throw new Error(`the Group column reads "${await cell.textContent()}"`);
      },
    },
    {
      what: "groups: with no machine in a group, the cards show under no heading",
      path: "/machines?nogroups",
      async run(page) {
        await page.locator("article").first().waitFor();
        if ((await page.locator(".group-head").count()) !== 0) throw new Error("there are group headings");
        if ((await page.locator("article").count()) !== 12) throw new Error(`${await page.locator("article").count()} cards, not 12`);
      },
    },
    {
      what: "groups: a collapsed group stays collapsed after a restart, and opens again",
      path: "/machines",
      async run(page) {
        const toggle = () => group(page, "Proxmox lab").getByRole("button", { name: /^Proxmox lab/ });
        await toggle().click();
        await card(page, "Pi-hole 1").waitFor({ state: "detached" });
        if ((await toggle().getAttribute("aria-expanded")) !== "false") throw new Error("the heading doesn't say it is collapsed");
        // Only that group.
        await card(page, "Homelab VM").waitFor();
        await page.reload({ waitUntil: "networkidle" });
        await card(page, "Homelab VM").waitFor();
        if ((await card(page, "Pi-hole 1").count()) !== 0) throw new Error("the group is open again after a restart");
        await toggle().click();
        await card(page, "Pi-hole 1").waitFor();
        await page.reload({ waitUntil: "networkidle" });
        await card(page, "Pi-hole 1").waitFor();
      },
    },
    {
      what: "groups: a group typed in the form puts the machine with the others of that name, in their spelling",
      path: "/machines",
      async run(page) {
        await page.getByRole("button", { name: "Edit Main PC" }).click();
        await page.getByLabel("Group (optional)").fill("  nas   pc ");
        await page.getByText("Shown with the 3 other machines in NAS PC.").waitFor();
        await page.getByRole("button", { name: "Save", exact: true }).click();
        await page.locator("form").waitFor({ state: "detached" });
        await group(page, "NAS PC").locator("article", { hasText: "Main PC" }).waitFor();
        const got = await headings(page);
        if (!got.includes("NAS PC · 4")) throw new Error(`the headings read ${JSON.stringify(got)}`);
        // A new name makes a new group.
        await page.getByRole("button", { name: "Edit Printer" }).click();
        await page.getByLabel("Group (optional)").fill("Attic");
        await page.getByText("A new group: Attic gets a heading of its own").waitFor();
        await page.getByRole("button", { name: "Save", exact: true }).click();
        await group(page, "Attic").locator("article", { hasText: "Printer" }).waitFor();
        // Emptied, it leaves the group.
        await page.getByRole("button", { name: "Edit Printer" }).click();
        await page.getByLabel("Group (optional)").fill("");
        await page.getByRole("button", { name: "Save", exact: true }).click();
        await group(page, "Attic").waitFor({ state: "detached" });
        await group(page, "Other machines").locator("article", { hasText: "Printer" }).waitFor();
      },
    },
    {
      what: "groups: the group menu on a card moves it, also with the keyboard, and New group opens the form there",
      path: "/machines",
      async run(page) {
        await page.getByRole("button", { name: "Group for Media PC" }).click();
        await page.getByRole("menuitemradio", { name: "Proxmox lab" }).click();
        await group(page, "Proxmox lab").locator("article", { hasText: "Media PC" }).waitFor();
        // The keyboard: Enter opens it on the group the machine is in ("No group"), Home goes to the first.
        await page.getByRole("button", { name: "Group for Printer" }).focus();
        await page.keyboard.press("Enter");
        const focused = () => page.evaluate(() => document.activeElement?.textContent?.trim());
        if ((await focused()) !== "No group") throw new Error(`the menu opened on "${await focused()}"`);
        await page.keyboard.press("Home");
        await page.keyboard.press("Enter");
        await group(page, "NAS PC").locator("article", { hasText: "Printer" }).waitFor();
        if ((await page.getByRole("menu").count()) !== 0) throw new Error("the menu is still open");
        // Escape closes it without changing anything.
        await page.getByRole("button", { name: "Group for Printer" }).focus();
        await page.keyboard.press("ArrowDown");
        await page.getByRole("menu").waitFor();
        await page.keyboard.press("Escape");
        await page.getByRole("menu").waitFor({ state: "detached" });
        // New group…: the form, with the cursor in the Group field.
        await page.getByRole("button", { name: "Group for Old laptop" }).click();
        await page.getByRole("menuitem", { name: "New group…" }).click();
        await page.locator("form").waitFor();
        const field = await page.evaluate(() => document.activeElement?.id);
        if (field !== "machine-group") throw new Error(`the focus is on "${field}", not the Group field`);
        await page.keyboard.type("Attic");
        await page.getByRole("button", { name: "Save", exact: true }).click();
        await group(page, "Attic").locator("article", { hasText: "Old laptop" }).waitFor();
      },
    },
    {
      what: "groups: renaming changes every machine in the group, keeps it collapsed, and a taken name joins the two",
      path: "/machines",
      async run(page) {
        await group(page, "Proxmox lab").getByRole("button", { name: /^Proxmox lab/ }).click();
        await page.getByRole("button", { name: "Rename the group Proxmox lab" }).click();
        await page.getByLabel("New name for Proxmox lab").fill("Proxmox");
        await page.getByText("Changes the group of the 4 machines in it.").waitFor();
        await page.locator("form").getByRole("button", { name: "Rename" }).click();
        await group(page, "Proxmox").waitFor();
        const toggle = group(page, "Proxmox").getByRole("button", { name: /^Proxmox ·/ });
        if ((await toggle.getAttribute("aria-expanded")) !== "false") throw new Error("the renamed group isn't collapsed any more");
        if ((await group(page, "Proxmox lab").count()) !== 0) throw new Error("Proxmox lab is still there");
        // Every machine in it, in the table too.
        const cells = await page.locator("tbody td.c-group").allTextContents();
        if (cells.filter((c) => c.trim() === "Proxmox").length !== 4) throw new Error(`the Group column reads ${JSON.stringify(cells)}`);
        // Into a name that is taken, in other letter case: the two groups become one.
        await page.getByRole("button", { name: "Rename the group NAS PC" }).click();
        await page.getByLabel("New name for NAS PC").fill("PROXMOX");
        await page.getByText("Proxmox already exists: the 3 machines in NAS PC join it.").waitFor();
        await page.locator("form").getByRole("button", { name: "Rename" }).click();
        await group(page, "NAS PC").waitFor({ state: "detached" });
        const got = await headings(page);
        if (got.join("|") !== "Proxmox · 7|Other machines · 5") throw new Error(`the headings read ${JSON.stringify(got)}`);
        // An empty name is refused, with the reason.
        await page.getByRole("button", { name: "Rename the group Proxmox" }).click();
        await page.getByLabel("New name for Proxmox").fill("  ");
        await page.locator("form").getByRole("button", { name: "Rename" }).click();
        await page.getByRole("alert").filter({ hasText: "A group needs a name" }).waitFor();
      },
    },
    {
      what: "groups: Ungroup asks first, then puts the machines under Other machines",
      path: "/machines",
      async run(page) {
        const nas = group(page, "NAS PC");
        await nas.getByRole("button", { name: "Ungroup" }).click();
        await nas.getByRole("button", { name: "Yes, ungroup" }).waitFor();
        if ((await cardsIn(page, "NAS PC")).length !== 3) throw new Error("the machines left the group before the yes");
        await nas.getByRole("button", { name: "Yes, ungroup" }).click();
        await nas.waitFor({ state: "detached" });
        const other = await cardsIn(page, "Other machines");
        for (const name of ["NAS", "Homelab VM", "Dev VM"]) if (!other.includes(name)) throw new Error(`${name} isn't under Other machines`);
        // Cancel leaves it as it was.
        const lab = group(page, "Proxmox lab");
        await lab.getByRole("button", { name: "Ungroup" }).click();
        await lab.getByRole("button", { name: "Cancel" }).click();
        if ((await cardsIn(page, "Proxmox lab")).length !== 4) throw new Error("Cancel changed the group");
      },
    },
    {
      what: "groups: a machine's form left open doesn't undo a group given meanwhile with the card menu or Ungroup",
      path: "/machines",
      async run(page) {
        const field = page.getByLabel("Group (optional)");
        const save = () => page.getByRole("button", { name: "Save", exact: true }).click();
        // The card menu while the form is open: the field follows, and saving the form keeps the group.
        await page.getByRole("button", { name: "Edit Printer", exact: true }).click();
        await page.getByRole("button", { name: "Group for Printer" }).click();
        await page.getByRole("menuitemradio", { name: "Proxmox lab" }).click();
        await group(page, "Proxmox lab").locator("article", { hasText: "Printer" }).waitFor();
        if ((await field.inputValue()) !== "Proxmox lab") throw new Error(`the Group field reads "${await field.inputValue()}"`);
        await page.getByLabel("Notes", { exact: true }).fill("By the stairs");
        await save();
        await page.locator("form").waitFor({ state: "detached" });
        await group(page, "Proxmox lab").locator("article", { hasText: "Printer" }).waitFor();
        // A group ungrouped while the form of a machine in it is open stays gone after Save.
        await page.getByRole("button", { name: "Edit NAS", exact: true }).click();
        const nas = group(page, "NAS PC");
        await nas.getByRole("button", { name: "Ungroup" }).click();
        await nas.getByRole("button", { name: "Yes, ungroup" }).click();
        await nas.waitFor({ state: "detached" });
        if ((await field.inputValue()) !== "") throw new Error(`after Ungroup the Group field reads "${await field.inputValue()}"`);
        await save();
        await page.locator("form").waitFor({ state: "detached" });
        if ((await group(page, "NAS PC").count()) !== 0) throw new Error("saving the form brought NAS PC back");
        // A group the owner typed into the field wins over one given with the menu meanwhile.
        await page.getByRole("button", { name: "Edit Old laptop", exact: true }).click();
        await field.fill("Attic");
        await page.getByRole("button", { name: "Group for Old laptop" }).click();
        await page.getByRole("menuitemradio", { name: "Proxmox lab" }).click();
        await group(page, "Proxmox lab").locator("article", { hasText: "Old laptop" }).waitFor();
        if ((await field.inputValue()) !== "Attic") throw new Error(`what was typed became "${await field.inputValue()}"`);
        await save();
        await group(page, "Attic").locator("article", { hasText: "Old laptop" }).waitFor();
      },
    },
    {
      what: "groups: a card put in no group is shown under Other machines even when that was collapsed, and keeps the focus",
      path: "/machines",
      async run(page) {
        const toggle = group(page, "Other machines").getByRole("button", { name: /^Other machines/ });
        await toggle.click();
        await card(page, "Printer").waitFor({ state: "detached" });
        await page.getByRole("button", { name: "Group for Pi-hole 1" }).click();
        await page.getByRole("menuitemradio", { name: "No group" }).click();
        await group(page, "Other machines").locator("article", { hasText: "Pi-hole 1" }).waitFor();
        if ((await toggle.getAttribute("aria-expanded")) !== "true") throw new Error("Other machines is still collapsed");
        const focused = await page.evaluate(() => document.activeElement?.getAttribute("aria-label"));
        if (focused !== "Group for Pi-hole 1") throw new Error(`the focus is on "${focused}"`);
      },
    },
    {
      what: "groups: opening a rename keeps a machine's form as typed, and the other way round",
      path: "/machines",
      async run(page) {
        const notes = page.getByLabel("Notes", { exact: true });
        await page.getByRole("button", { name: "Edit Main PC", exact: true }).click();
        await notes.fill("Half typed");
        await page.getByRole("button", { name: "Rename the group NAS PC" }).click();
        await page.getByText('What you typed for "Main PC" is kept').waitFor();
        const newName = page.getByLabel("New name for NAS PC");
        await newName.fill("Stor");
        // Back to the machine: as it was typed, and the rename waits behind it.
        await page.getByRole("button", { name: "Edit Main PC", exact: true }).click();
        if ((await notes.inputValue()) !== "Half typed") throw new Error(`the notes read "${await notes.inputValue()}"`);
        await page.getByText("The rename of NAS PC you started is kept").waitFor();
        await page.getByRole("button", { name: "Cancel", exact: true }).click();
        if ((await newName.inputValue()) !== "Stor") throw new Error(`the new name reads "${await newName.inputValue()}"`);
        await newName.fill("Storage");
        await page.getByRole("button", { name: "Rename", exact: true }).click();
        await group(page, "Storage").waitFor();
        await page.locator("form").waitFor({ state: "detached" });
      },
    },
    {
      what: "groups: a machine set to Ignore keeps its group, and the screen counts it and joins it as the back end does",
      path: "/machines",
      async run(page) {
        const save = () => page.getByRole("button", { name: "Save", exact: true }).click();
        await page.getByRole("button", { name: "Edit Dev VM", exact: true }).click();
        // The label holds the list itself, so its name also has the options' words in it.
        await page.getByRole("combobox", { name: /^Type/ }).selectOption("ignore");
        await save();
        await card(page, "Dev VM").waitFor({ state: "detached" });
        const title = await group(page, "NAS PC").getByRole("button", { name: "Ungroup" }).getAttribute("title");
        const want = "Take the 2 machines in NAS PC, and 1 device in it not shown as a card out of this group. The machines stay.";
        if (title !== want) throw new Error(`Ungroup says "${title}"`);
        await page.getByRole("button", { name: "Rename the group NAS PC" }).click();
        await page.getByText("Changes the group of the 2 machines in it, and 1 device in it not shown as a card.").waitFor();
        await page.getByRole("button", { name: "Cancel", exact: true }).click();
        // With only Dev VM left in it, a name typed in other letter case still joins it, in its spelling.
        for (const name of ["NAS", "Homelab VM"]) {
          await page.getByRole("button", { name: `Group for ${name}` }).click();
          await page.getByRole("menuitemradio", { name: "No group" }).click();
        }
        await group(page, "NAS PC").waitFor({ state: "detached" });
        await page.getByRole("button", { name: "Edit Main PC", exact: true }).click();
        await page.getByLabel("Group (optional)").fill("nas pc");
        await page.getByText("NAS PC gets a heading of its own on this screen. A device not shown as a card is in it too.").waitFor();
        await save();
        await group(page, "NAS PC").locator("article", { hasText: "Main PC" }).waitFor();
      },
    },
    {
      what: "groups: the keys move a card only among its group, the order is kept after a restart, and Reset order undoes it",
      path: "/machines",
      async run(page) {
        const nasBefore = (await cardsIn(page, "NAS PC")).join();
        await page.getByRole("button", { name: "Move Proxmox, place 4 of 4 in Proxmox lab" }).focus();
        await page.keyboard.press("Home");
        await page.locator("p[aria-live]", { hasText: "Proxmox moved to place 1 of 4 in Proxmox lab." }).waitFor({ state: "attached" });
        const want = "Proxmox,Monitoring,Pi-hole 1,Pi-hole 2";
        if ((await cardsIn(page, "Proxmox lab")).join() !== want) throw new Error(`Proxmox lab reads ${(await cardsIn(page, "Proxmox lab")).join(", ")}`);
        // Past the start of the group it stays first rather than leaving the group.
        await page.keyboard.press("ArrowLeft");
        if ((await cardsIn(page, "Proxmox lab")).join() !== want) throw new Error("the card left its place at the start");
        if ((await cardsIn(page, "NAS PC")).join() !== nasBefore) throw new Error("another group's order changed");
        await page.reload({ waitUntil: "networkidle" });
        await page.locator("article").first().waitFor();
        if ((await cardsIn(page, "Proxmox lab")).join() !== want) throw new Error("the order wasn't kept after a restart");
        // A card that joins a group whose order was changed goes at its end.
        await page.getByRole("button", { name: "Group for Main PC" }).click();
        await page.getByRole("menuitemradio", { name: "Proxmox lab" }).click();
        await group(page, "Proxmox lab").locator("article", { hasText: "Main PC" }).waitFor();
        const joined = (await cardsIn(page, "Proxmox lab")).join();
        if (joined !== `${want},Main PC`) throw new Error(`after Main PC joined, Proxmox lab reads ${joined}`);
        await page.getByRole("button", { name: "Reset order" }).click();
        const reset = (await cardsIn(page, "Proxmox lab")).join();
        if (reset !== "Main PC,Monitoring,Pi-hole 1,Pi-hole 2,Proxmox") throw new Error(`after Reset order, Proxmox lab reads ${reset}`);
      },
    },
    {
      what: "on the Overview the machines are under a heading per type, and appliances are small tiles like the other devices",
      path: "/",
      viewport: { width: 1920, height: 1080 },
      async run(page) {
        await page.locator(".card").first().waitFor();
        const got = await page.evaluate(() => ({
          headings: [...document.querySelectorAll(".kind-title")].map((h) => h.textContent.replace(/\s+/g, " ").trim()),
          cards: Object.fromEntries(
            [...document.querySelectorAll(".kind")].map((k) => [
              k.querySelector(".kind-title").textContent.trim().split(" ")[0],
              [...k.querySelectorAll("article.card .card-name")].map((n) => n.textContent.trim()),
            ]),
          ),
        }));
        const order = got.headings.map((h) => h.split(" ")[0]);
        const want = ["PC", "PCs", "Server", "Servers", "VM", "VMs", "Container", "Containers"];
        const ranks = order.map((h) => Math.floor(want.indexOf(h) / 2));
        if (ranks.some((r) => r < 0) || ranks.some((r, i) => i > 0 && r <= ranks[i - 1])) throw new Error(`the headings read ${JSON.stringify(got.headings)}`);
        if (!got.cards.VMs?.includes("Homelab VM")) throw new Error(`Homelab VM isn't under VMs: ${JSON.stringify(got.cards)}`);
        for (const name of ["Living room TV", "Printer"]) {
          if (Object.values(got.cards).flat().includes(name)) throw new Error(`${name} is still a machine card`);
        }
        const appliances = page.locator("section", { has: page.getByRole("heading", { name: "Appliances", exact: true }) });
        await appliances.locator(".chip", { hasText: "Printer" }).waitFor();
        await appliances.locator(".chip", { hasText: "Living room TV" }).waitFor();
        // As tall as the other devices' tiles.
        const tv = await appliances.locator(".chip", { hasText: "Living room TV" }).boundingBox();
        const phone = await page.locator(".chip", { hasText: "Sam's phone" }).boundingBox();
        if (Math.abs(tv.height - phone.height) > 1) throw new Error(`an appliance tile is ${tv.height}px tall, another device's ${phone.height}px`);
        // Types share a line when their cards fit (in a 1920px window PCs and Servers do), and the tiles come
        // on a line of their own after every card, Appliances and Other devices side by side.
        const at = (sel) => page.locator(sel).first().boundingBox();
        const pcs = await at("#kind-pc");
        const servers = await at("#kind-server");
        if (Math.abs(pcs.y - servers.y) > 1 || servers.x <= pcs.x) throw new Error("Servers didn't come next to PCs in a 1920px window");
        const lastCard = Math.max(...(await page.locator("article.card").evaluateAll((els) => els.map((e) => e.getBoundingClientRect().bottom))));
        const appl = await page.locator(".tile-group").filter({ hasText: "Printer" }).boundingBox();
        const other = await page.locator(".tile-group").filter({ hasText: "Tablet" }).boundingBox();
        if (appl.y < lastCard) throw new Error("the appliances start beside the cards, not below them");
        if (Math.abs(appl.y - other.y) > 1 || other.x <= appl.x) throw new Error("Other devices didn't come next to Appliances");
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
    {
      what: "on the Overview a down machine can be silenced: out of Needs attention and the Down counts, still on its card, kept, and undone",
      path: "/",
      async run(page) {
        const laptop = page.locator("article.card", { hasText: "Old laptop" });
        const tv = page.locator(".chip", { hasText: "Living room TV" });
        const said = page.locator(".issues").getByRole("link", { name: /^Old laptop is down/ });
        const downStat = page.locator(".stat", { has: page.getByText("Down", { exact: true }) });
        const downCount = async () => Number((await downStat.locator(".value").innerText()).trim());
        const pcs = page.locator("#kind-pc");
        await said.waitFor();
        await laptop.getByText("down", { exact: true }).waitFor();
        if (!(await pcs.innerText()).includes("· 1 down")) throw new Error(`the PCs heading reads "${await pcs.innerText()}"`);
        // Offered only on a machine that is down.
        if (await page.getByRole("button", { name: "Silence Main PC" }).count()) throw new Error("Silence is offered on a machine that is up");
        const before = await downCount();

        await laptop.getByRole("button", { name: "Silence Old laptop" }).click();
        await laptop.getByText("silenced", { exact: true }).waitFor();
        await said.waitFor({ state: "detached" });
        const after = await downCount();
        if (after !== before - 1) throw new Error(`the Down count went from ${before} to ${after}`);
        const heading = (await pcs.innerText()).replace(/\s+/g, " ");
        if (heading.includes("down") || !heading.includes("· 1 silenced")) throw new Error(`the PCs heading reads "${heading}"`);
        await downStat.getByText(/· 1 silenced/).waitFor();
        const dot = await laptop.getByRole("img").first().getAttribute("aria-label");
        if (dot !== "Down, silenced") throw new Error(`the laptop's status dot says "${dot}"`);

        // Kept on this PC: still silenced when the window is opened again.
        await page.reload({ waitUntil: "networkidle" });
        await laptop.getByRole("button", { name: "Undo silencing Old laptop" }).waitFor();
        if (await said.count()) throw new Error("the silenced laptop is back in Needs attention after a reload");

        // An appliance's tile too.
        await tv.getByRole("button", { name: "Silence Living room TV" }).click();
        await tv.getByText("silenced", { exact: true }).waitFor();
        if (await page.locator(".issues").getByRole("link", { name: /^Living room TV is down/ }).count()) throw new Error("the silenced TV is still in Needs attention");
        await downStat.getByText(/· 2 silenced/).waitFor();
        await tv.getByRole("button", { name: "Undo silencing Living room TV" }).click();
        await page.locator(".issues").getByRole("link", { name: /^Living room TV is down/ }).waitFor();

        await laptop.getByRole("button", { name: "Undo silencing Old laptop" }).click();
        await said.waitFor();
        await laptop.getByRole("button", { name: "Silence Old laptop" }).waitFor();
        if (!(await pcs.innerText()).includes("· 1 down")) throw new Error(`after Undo the PCs heading reads "${await pcs.innerText()}"`);
        const stored = await page.evaluate(() => localStorage.getItem("home-cc.silenced"));
        if (stored !== null) throw new Error(`after every Undo the window still keeps ${stored}`);
      },
    },
    {
      what: "on the Overview a silence ends when the machine is up again, so it counts as down when it goes again",
      path: "/",
      async run(page) {
        const laptop = page.locator("article.card", { hasText: "Old laptop" });
        const said = page.locator(".issues").getByRole("link", { name: /^Old laptop is down/ });
        await laptop.getByRole("button", { name: "Silence Old laptop" }).click();
        await said.waitFor({ state: "detached" });
        // A round of checks finds it up.
        await page.evaluate(reportUp, ["Old laptop", true]);
        await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("check_now"));
        await laptop.getByText("silenced", { exact: true }).waitFor({ state: "detached" });
        if (await laptop.getByRole("button", { name: /Silence|Undo/ }).count()) throw new Error("a machine that is up offers Silence or Undo");
        const stored = await page.evaluate(() => localStorage.getItem("home-cc.silenced"));
        if (stored !== null) throw new Error(`the silence of a machine that is up again is still kept: ${stored}`);
        // And down again: back in Needs attention, with Silence offered again.
        await page.evaluate(reportUp, ["Old laptop", false]);
        await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("check_now"));
        await said.waitFor();
        await laptop.getByRole("button", { name: "Silence Old laptop" }).waitFor();
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
        const shown = "docker exec -it media-jellyfin-1 sh -c 'command -v bash >/dev/null && exec bash || exec sh'";
        // Docker is looked for where the Docker module looks for it, the snap's /snap/bin among them.
        const want = `env PATH="$PATH:/usr/local/bin:/usr/bin:/bin:/snap/bin" ${shown}`;
        if (opened.length !== 1 || opened[0].command !== want) throw new Error(`opened ${JSON.stringify(opened)}`);
        // Beside the tabs, without that PATH, so what runs is what is in view.
        const line = (await page.locator(".where .command").textContent())?.trim();
        if (line !== shown) throw new Error(`beside the tabs: "${line}"`);
      },
    },
    {
      what: "Docker: at every window width a stack card's container rows fit in the card, controls included",
      path: "/docker",
      async run(page) {
        await page.getByText("612 MB").waitFor();
        // Every width in steps of 8px, so no band between the column layout and the narrow one is missed
        // (the five screenshot sizes all fall outside such a band).
        for (let width = 640; width <= 2600; width += 8) {
          await page.setViewportSize({ width, height: 900 });
          const bad = await page.evaluate(
            () =>
              new Promise((done) =>
                requestAnimationFrame(() => {
                  for (const card of document.querySelectorAll("article.stack")) {
                    const style = getComputedStyle(card);
                    const right = card.getBoundingClientRect().right - parseFloat(style.paddingRight);
                    for (const el of card.querySelectorAll(".head, .row, .row .acts")) {
                      if (el.clientWidth === 0) continue;
                      // A row reaches into the card's padding by its negative margin, on purpose.
                      const reach = -Math.min(0, parseFloat(getComputedStyle(el).marginRight));
                      const over = Math.max(el.scrollWidth - el.clientWidth, el.getBoundingClientRect().right - right - reach);
                      if (over > 1) return done(`${el.className.split(" ")[0]} in "${card.getAttribute("aria-label")}" runs ${Math.round(over)}px past the card`);
                    }
                  }
                  done(null);
                }),
              ),
          );
          if (bad) throw new Error(`at ${width}px wide: ${bad}`);
        }
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
      what: "Docker updates: newer images are marked on their rows and counted on the stack cards, the host tab and in one line",
      path: "/docker",
      async run(page) {
        await page.getByText("5 updates in 2 stacks and 1 other container").waitFor();
        const tab = page.getByRole("tab", { name: /Homelab VM/ });
        await tab.locator(".badge", { hasText: /^6 updates$/ }).waitFor();
        const media = page.getByRole("article", { name: "Stack media" });
        await media.getByText("3 updates", { exact: true }).waitFor();
        await page.getByRole("article", { name: "Stack photos" }).getByText("2 updates", { exact: true }).waitFor();
        const row = (name) => page.locator(".row", { has: page.getByRole("button", { name: `Logs of ${name}` }) });
        const mark = async (name) => ((await row(name).locator(".mark").count()) ? (await row(name).locator(".mark").textContent()).trim() : null);
        const want = {
          "media-sonarr-1": "Update",
          "media-prowlarr-1": "Update",
          "media-jellyfin-1": null,
          "speedtest-tracker": "Update",
          dashboard: "Built here",
          "nas-backup": "Can't check",
        };
        for (const [name, text] of Object.entries(want)) {
          const got = await mark(name);
          if (got !== text) throw new Error(`${name} is marked "${got}", not "${text}"`);
        }
        const title = await row("media-sonarr-1").locator(".mark").getAttribute("title");
        if (!/[0-9a-f]{12} → [0-9a-f]{12}/.test(title ?? "")) throw new Error(`the mark doesn't say old → new: "${title}"`);
        if ((await page.getByRole("tab", { name: /Monitoring/ }).locator(".badge").count()) !== 0) throw new Error("an up-to-date host has a count");
        // The image that couldn't be checked opens the updates, which say why.
        await page.getByRole("button", { name: "1 image couldn't be checked" }).click();
        await page.getByText("401 Unauthorized").first().waitFor();
        await page.getByText("wants a login this host doesn't have").waitFor();
      },
    },
    {
      what: "Docker updates: Check for updates asks the host again, not just the last answer, and says when it last checked",
      path: "/docker",
      async run(page) {
        await page.getByText(/^checked (just now|\d+ s ago)$/).waitFor();
        await page.evaluate(() => {
          const tauri = window.__TAURI_INTERNALS__;
          const invoke = tauri.invoke;
          window.__checks = [];
          tauri.invoke = (cmd, args, ...rest) => {
            if (cmd === "docker_check_updates") window.__checks.push(args);
            return invoke(cmd, args, ...rest);
          };
        });
        await page.getByRole("button", { name: "Check for updates" }).click();
        await page.getByText("Checking for newer images…").waitFor();
        await page.getByText("Checking for newer images…").waitFor({ state: "detached" });
        const asked = await page.evaluate(() => window.__checks);
        if (!asked.some((a) => a.auto === false && a.images == null)) throw new Error(`asked ${JSON.stringify(asked)}`);
        await page.getByText(/^checked (just now|\d+ s ago)$/).waitFor();
      },
    },
    {
      what: "Docker updates: a stack's Update lists what it pulls and restarts, asks first, updates only what is ticked and clears their marks",
      path: "/docker",
      async run(page) {
        await page.getByText("5 updates in 2 stacks and 1 other container").waitFor();
        await page.evaluate(() => {
          const tauri = window.__TAURI_INTERNALS__;
          const invoke = tauri.invoke;
          window.__updates = [];
          tauri.invoke = (cmd, args, ...rest) => {
            if (cmd === "docker_update_stack") window.__updates.push(args);
            return invoke(cmd, args, ...rest);
          };
        });
        const media = page.getByRole("article", { name: "Stack media" });
        await media.getByRole("button", { name: "Update…" }).click();
        const panel = page.getByRole("region", { name: "Image updates on Homelab VM" });
        await panel.getByRole("heading", { name: "Update media" }).waitFor();
        const box = (service) => panel.getByRole("checkbox", { name: new RegExp(`^${service}\\b`) });
        if (!(await box("sonarr").isChecked()) || !(await box("radarr").isChecked())) throw new Error("a running service with an update wasn't ticked");
        if (await box("prowlarr").isChecked()) throw new Error("a stopped service was ticked: updating it would start it");
        await panel.getByText("Stopped now: updating starts media-prowlarr-1").waitFor();
        await panel.getByText("Restarts media-sonarr-1").waitFor();
        await panel.getByText(/^[0-9a-f]{12} → [0-9a-f]{12}$/).first().waitFor();
        if (await panel.getByText("photos").count()) throw new Error("another stack's updates are in this stack's list");
        await panel.getByRole("button", { name: "Update 2 containers in 1 stack" }).click();
        if ((await page.evaluate(() => window.__updates)).length) throw new Error("it updated without asking");
        await panel.getByRole("button", { name: "Yes, pull and restart these 2" }).click();
        await media.getByText("Updating…").waitFor();
        await panel.getByText("Updated media.").waitFor({ timeout: 15_000 });
        await panel.getByRole("button", { name: "Clean up…" }).waitFor();
        const sent = await page.evaluate(() => window.__updates);
        if (sent.length !== 1 || sent[0].stack !== "media" || JSON.stringify(sent[0].services) !== '["radarr","sonarr"]') {
          throw new Error(`updated ${JSON.stringify(sent)}`);
        }
        await media.getByText("1 update", { exact: true }).waitFor();
        const sonarr = page.locator(".row", { has: page.getByRole("button", { name: "Logs of media-sonarr-1" }) });
        await sonarr.locator(".mark").waitFor({ state: "detached" });
        await page.getByText("3 updates in 2 stacks and 1 other container").waitFor();
        await page.getByRole("tab", { name: /Homelab VM/ }).locator(".badge", { hasText: /^4 updates$/ }).waitFor();
        await media.getByText("What Docker said").waitFor();
        // Put away by its Clean up link, the update's result goes with it: opened again, no old "Updated media.".
        await panel.getByRole("button", { name: "Clean up…" }).click();
        await page.getByRole("heading", { name: "Clean up Homelab VM" }).waitFor();
        await media.getByRole("button", { name: "Update…" }).click();
        await panel.getByRole("heading", { name: "Update media" }).waitFor();
        if (await panel.getByText("Updated media.").count()) throw new Error("an old update's result shows in a panel opened afresh");
      },
    },
    {
      what: "Docker updates: a confirm already asked goes back to asking when what it would update changes before the yes",
      path: "/docker",
      async run(page) {
        await page.getByText("5 updates in 2 stacks and 1 other container").waitFor();
        await page.evaluate(() => {
          const tauri = window.__TAURI_INTERNALS__;
          const invoke = tauri.invoke;
          window.__updates = [];
          tauri.invoke = (cmd, args, ...rest) => {
            if (cmd === "docker_update_stack") window.__updates.push(args);
            return invoke(cmd, args, ...rest);
          };
        });
        await page.getByRole("article", { name: "Stack media" }).getByRole("button", { name: "Update…" }).click();
        const panel = page.getByRole("region", { name: "Image updates on Homelab VM" });
        await panel.getByRole("button", { name: "Update 2 containers in 1 stack" }).click();
        await panel.getByRole("button", { name: "Yes, pull and restart these 2" }).waitFor();
        // What would run changes under the armed confirm (a tick here stands in for a service started or an
        // image found newer in the background).
        await panel.getByRole("checkbox", { name: /^prowlarr\b/ }).check();
        await panel.getByRole("button", { name: "Update 3 containers in 1 stack" }).waitFor();
        if (await panel.getByRole("button", { name: /^Yes, pull and restart/ }).count()) throw new Error("the confirm stayed armed for a different update");
        if ((await page.evaluate(() => window.__updates)).length) throw new Error("it updated without asking");
      },
    },
    {
      what: "Docker updates: an image with no answer yet is said, and the host isn't called up to date",
      path: "/docker",
      async run(page) {
        await page.getByRole("tab", { name: /Monitoring/ }).click();
        await page.getByText("All images up to date").waitFor({ timeout: 15_000 });
        // The registries' answer for grafana goes missing, as one cut short would.
        await page.evaluate(() => {
          const tauri = window.__TAURI_INTERNALS__;
          const invoke = tauri.invoke;
          tauri.invoke = async (cmd, args, ...rest) => {
            const got = await invoke(cmd, args, ...rest);
            if (cmd !== "docker_check_updates" || !got) return got;
            return { ...got, images: got.images.filter((u) => !u.image.startsWith("grafana/")) };
          };
        });
        await page.getByRole("button", { name: "Check for updates" }).click();
        await page.getByText("1 image not checked yet").waitFor({ timeout: 15_000 });
        await page.getByText("No updates found").waitFor();
        if (await page.getByText("All images up to date").count()) throw new Error("called up to date with an image unanswered");
      },
    },
    {
      what: "Docker updates: Update all lists every stack with updates and updates them after one yes",
      path: "/docker",
      async run(page) {
        await page.getByRole("button", { name: "Update all…" }).click();
        const panel = page.getByRole("region", { name: "Image updates on Homelab VM" });
        await panel.getByRole("heading", { name: "Updates on Homelab VM" }).waitFor();
        await panel.getByText("immich-machine-learning", { exact: true }).waitFor();
        await panel.getByText("sonarr", { exact: true }).waitFor();
        await panel.getByText("Containers outside any stack").waitFor();
        await panel.getByRole("button", { name: "Update 4 containers in 2 stacks" }).click();
        await panel.getByRole("button", { name: "Yes, pull and restart these 4" }).click();
        await panel.getByText("Updated media, photos.").waitFor({ timeout: 20_000 });
        await page.getByText("1 update in 1 stack and 1 other container").waitFor();
      },
    },
    {
      what: "Docker updates: a container outside any stack is offered its new image only, and is told to recreate it by hand",
      path: "/docker",
      async run(page) {
        const others = page.getByRole("article", { name: "Other containers" });
        await others.getByText("1 update", { exact: true }).waitFor();
        await others.getByRole("button", { name: "Pull new images…" }).click();
        const panel = page.getByRole("region", { name: "Containers outside any stack" });
        await panel.getByText("won't recreate them").waitFor();
        await panel.getByRole("button", { name: "Pull image" }).click();
        await panel.getByRole("button", { name: "Yes, pull it" }).click();
        await panel.getByText("New image pulled. Recreate speedtest-tracker by hand to use it.").waitFor();
        const row = page.locator(".row", { has: page.getByRole("button", { name: "Logs of speedtest-tracker" }) });
        await row.getByText("New image pulled").waitFor();
        // Still counted as an update: the container runs the old image until it is recreated.
        await page.getByText("5 updates in 2 stacks and 1 other container").waitFor();
      },
    },
    {
      what: "Drafts: update ticks stay after looking at another screen, and closing the updates lets them go",
      path: "/docker",
      async run(page) {
        await page.getByText("5 updates in 2 stacks and 1 other container").waitFor();
        await page.getByRole("article", { name: "Stack media" }).getByRole("button", { name: "Update…" }).click();
        const panel = page.getByRole("region", { name: "Image updates on Homelab VM" });
        const box = (service) => panel.getByRole("checkbox", { name: new RegExp(`^${service}\\b`) });
        await box("sonarr").uncheck();
        await box("prowlarr").check();
        await awayAndBack(page, "Machines", "Docker");
        await panel.getByRole("heading", { name: "Update media" }).waitFor();
        if (await box("sonarr").isChecked()) throw new Error("sonarr was ticked again");
        if (!(await box("prowlarr").isChecked())) throw new Error("prowlarr lost its tick");
        await panel.getByRole("button", { name: "Update 2 containers in 1 stack" }).waitFor();
        await panel.getByRole("button", { name: "Close the updates" }).click();
        await page.getByRole("article", { name: "Stack media" }).getByRole("button", { name: "Update…" }).click();
        await panel.getByRole("heading", { name: "Update media" }).waitFor();
        if (!(await box("sonarr").isChecked()) || (await box("prowlarr").isChecked())) throw new Error("after Close the ticks weren't the usual ones");
      },
    },
    {
      what: "the Overview's Containers panel counts each host's updates in its one line",
      path: "/",
      async run(page) {
        const panel = page.locator("section.panel", { has: page.getByRole("heading", { name: "Containers" }) });
        const homelab = panel.locator("li", { has: page.getByRole("button", { name: /Homelab VM/ }) });
        await homelab.getByText(/\d+ running · \d+ stacks · 6 updates · read/).waitFor({ timeout: 15_000 });
        const monitoring = panel.locator("li", { has: page.getByRole("button", { name: /Monitoring/ }) });
        await monitoring.getByText(/\d+ running · 1 stack\s+· read/).waitFor({ timeout: 15_000 });
        if (await monitoring.getByText(/update/).count()) throw new Error("an up-to-date host mentions updates");
      },
    },
    {
      what: "the Overview's Containers panel reads every Docker host by itself and opens one on the Docker screen",
      path: "/",
      async run(page) {
        const panel = page.locator("section.panel", { has: page.getByRole("heading", { name: "Containers" }) });
        await panel.getByText("Homelab VM").waitFor();
        await panel.getByText(/\d+ running · \d+ stacks?/).first().waitFor({ timeout: 10000 });
        await panel.getByText("zigbee2mqtt").waitFor();
        await page.getByRole("link", { name: /Homelab VM: \S*zigbee2mqtt\S* keeps restarting/ }).waitFor();
        if (await panel.getByText("once the Docker module is in").count()) throw new Error("the old placeholder is still there");
        await panel.getByRole("button", { name: /Dev VM/ }).click();
        await page.waitForURL("**/docker");
        await page.getByRole("tab", { name: /Dev VM/, selected: true }).waitFor();
      },
    },
    {
      what: "Docker: an open cleanup moves to the host tab picked next, and an open log closes",
      path: "/docker",
      async run(page) {
        await page.getByRole("button", { name: "Clean up…" }).click();
        await page.getByRole("heading", { name: "Clean up Homelab VM" }).waitFor();
        await page.getByRole("tab", { name: /Dev VM/ }).click();
        await page.getByRole("heading", { name: "Clean up Dev VM" }).waitFor();
        if (await page.getByRole("heading", { name: "Clean up Homelab VM" }).count()) throw new Error("the cleanup for Homelab VM is still open");
        // A log is about one container on one host: it closes rather than showing another host's.
        await page.getByRole("tab", { name: /Homelab VM/ }).click();
        await page.getByRole("heading", { name: "Clean up Homelab VM" }).waitFor();
        await page.getByRole("button", { name: "Close the cleanup" }).click();
        const logs = page.getByRole("button", { name: "Logs" }).first();
        await logs.click();
        await page.locator("aside.side").waitFor();
        await page.getByRole("tab", { name: /Dev VM/ }).click();
        await page.locator("aside.side").waitFor({ state: "detached" });
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
    // --- What was typed stays while another screen is shown (drafts.svelte.ts) ---
    {
      what: "Drafts: a Proxmox token ID and secret typed in stay after looking at Terminals, and saving lets them go",
      path: "/proxmox?pvenotoken",
      async run(page) {
        const SECRET = "11111111-2222-3333-4444-555555555555";
        const setup = page.locator("form.setup");
        await setup.getByText("How to make a token").waitFor();
        await setup.getByLabel("Token ID").fill("root@pam!homecc");
        await setup.getByLabel("Secret").fill(SECRET);
        await setup.getByLabel("API port").fill("8007");
        await awayAndBack(page, "Terminals", "Proxmox");
        await setup.getByText("How to make a token").waitFor();
        await expectValue(setup.getByLabel("Token ID"), "root@pam!homecc", "the token ID");
        await expectValue(setup.getByLabel("Secret"), SECRET, "the secret");
        await expectValue(setup.getByLabel("API port"), "8007", "the API port");
        await setup.getByText("Unsaved changes").waitFor();
        // Discard puts back what is saved; typed again and saved, the draft is gone.
        await setup.getByRole("button", { name: "Discard" }).click();
        await expectValue(setup.getByLabel("Token ID"), "", "the token ID after Discard");
        await expectValue(setup.getByLabel("Secret"), "", "the secret after Discard");
        await setup.getByLabel("Token ID").fill("root@pam!homecc");
        await setup.getByLabel("Secret").fill(SECRET);
        await page.evaluate(recordCalls);
        await setup.getByRole("button", { name: "Save", exact: true }).click();
        await page.waitForFunction(() => window.__calls.includes("proxmox_save_settings") && window.__calls.includes("set_secret"));
        // With a token that works the form goes away by itself; opened again, it shows only what is saved.
        await setup.waitFor({ state: "detached" });
        await awayAndBack(page, "Terminals", "Proxmox");
        await page.getByRole("button", { name: "Connection settings" }).click();
        await expectValue(setup.getByLabel("Token ID"), "root@pam!homecc", "the saved token ID");
        await expectValue(setup.getByLabel("Secret"), "", "the secret after saving");
        if (await setup.getByText("Unsaved changes").count()) throw new Error("a saved token still shows as unsaved");
      },
    },
    {
      what: "Drafts: unsaved changes to a compose file stay after looking at another screen, also when the file changed on the host",
      path: "/docker",
      async run(page) {
        await page.getByRole("article", { name: "Stack media" }).getByRole("button", { name: "Files" }).click();
        const box = page.locator("textarea");
        await box.waitFor();
        const first = await box.inputValue();
        const typed = first.replace("10.10", "10.11");
        if (typed === first) throw new Error("the sample compose file has changed: nothing to edit");
        await box.fill(typed);
        await awayAndBack(page, "Machines", "Docker");
        await expectValue(box, typed, "the compose file");
        await page.getByText("Unsaved changes, kept from before.").waitFor();

        // Changed on the host while the owner is on another screen: their text stays, and the screen says so.
        await page.getByRole("link", { name: "Machines", exact: true }).click();
        await page.locator("article").first().waitFor();
        await page.evaluate(async () => {
          const invoke = window.__TAURI_INTERNALS__.invoke;
          const id = (await invoke("list_devices")).find((d) => d.name === "Homelab VM").id;
          const args = { deviceId: id, stack: "media", path: "/srv/media/compose.yml" };
          const now = await invoke("docker_read_file", args);
          await invoke("docker_save_file", { ...args, original: now, text: `${now}# changed on the host\n` });
        });
        await page.getByRole("link", { name: "Docker", exact: true }).click();
        await page.getByText("was changed on Homelab VM while you were on another screen").waitFor();
        await expectValue(box, typed, "the compose file after it changed on the host");
        // Saving then works, and replaces that change with the owner's text.
        await page.getByRole("button", { name: "Save", exact: true }).click();
        await page.getByRole("button", { name: "Yes, save to Homelab VM" }).click();
        await page.getByText(/The old version is at/).waitFor();
        // Closed, the panel lets the draft go: opened again, it shows the file as saved, with nothing unsaved.
        await page.getByRole("button", { name: "Close the files" }).click();
        await page.getByRole("article", { name: "Stack media" }).getByRole("button", { name: "Files" }).click();
        await expectValue(box, typed, "the compose file as saved");
        if (!(await page.getByRole("button", { name: "Undo changes" }).isDisabled())) throw new Error("a saved file still shows changes");
      },
    },
    {
      what: "Drafts: cleanup ticks stay after looking at another screen, and putting the cleanup away from the toolbar lets them go",
      path: "/docker",
      async run(page) {
        const toolbar = page.getByRole("button", { name: "Clean up…" });
        await toolbar.click();
        const named = page.getByRole("checkbox", { name: /nextcloud_db/ });
        const image = page.getByRole("checkbox", { name: /postgres:14/ });
        await named.waitFor();
        if (await named.isChecked()) throw new Error("a named volume was ticked to begin with");
        if (!(await image.isChecked())) throw new Error("an unused image wasn't ticked to begin with");
        await named.check();
        await image.uncheck();
        await awayAndBack(page, "Machines", "Docker");
        await named.waitFor();
        if (!(await named.isChecked())) throw new Error("the named volume ticked before lost its tick");
        if (await image.isChecked()) throw new Error("the image unticked before was ticked again");
        // Put away with the toolbar button, not its own Close: opened again, it starts from the usual ticks.
        await toolbar.click();
        await named.waitFor({ state: "detached" });
        await toolbar.click();
        await named.waitFor();
        if (await named.isChecked()) throw new Error("a named volume ticked earlier came back ticked after the cleanup was put away");
        if (!(await image.isChecked())) throw new Error("the image didn't get its usual tick after the cleanup was put away");
      },
    },
    {
      what: "Drafts: a snapshot being written stays after looking at another screen, and Close lets it go",
      path: "/proxmox",
      async run(page) {
        await page.getByRole("button", { name: "Manage homeassistant" }).click();
        const panel = page.getByRole("complementary");
        await panel.getByText("after_setup", { exact: true }).waitFor();
        const name = panel.getByLabel("Name");
        await name.fill("before_update");
        await panel.getByLabel("Note (optional)").fill("typed, not taken");
        await panel.getByLabel(/Include RAM/).check();
        await awayAndBack(page, "Terminals", "Proxmox");
        await panel.getByRole("heading", { name: "homeassistant" }).waitFor();
        await expectValue(name, "before_update", "the snapshot name");
        await expectValue(panel.getByLabel("Note (optional)"), "typed, not taken", "the snapshot note");
        if (!(await panel.getByLabel(/Include RAM/).isChecked())) throw new Error("Include RAM lost its tick");
        await panel.getByRole("button", { name: "Close", exact: true }).click();
        await page.getByRole("button", { name: "Manage homeassistant" }).click();
        await panel.getByRole("heading", { name: "homeassistant" }).waitFor();
        const fresh = await name.inputValue();
        if (!/^snap_\d{8}_\d{4}$/.test(fresh)) throw new Error(`the name after Close was "${fresh}", not a suggested one`);
        await expectValue(panel.getByLabel("Note (optional)"), "", "the snapshot note after Close");
        if (await panel.getByLabel(/Include RAM/).isChecked()) throw new Error("Include RAM kept its tick after Close");
      },
    },
    {
      what: "Drafts: the Machines edit form as typed stays after looking at another screen, and Cancel lets it go",
      path: "/machines",
      async run(page) {
        await page.getByRole("button", { name: "Edit NAS" }).click();
        const form = page.locator("form");
        await form.getByText("A password is stored").waitFor();
        const notes = form.getByLabel("Notes");
        const saved = await notes.inputValue();
        await notes.fill("typed, not saved");
        await form.getByLabel("SSH user").fill("someone-else");
        await form.getByLabel("SSH password (optional)").fill("typed-password");
        await awayAndBack(page, "Network", "Machines");
        await form.getByRole("heading", { name: "Edit machine" }).waitFor();
        await expectValue(form.getByLabel("Name", { exact: true }), "NAS", "the machine being edited");
        await expectValue(notes, "typed, not saved", "the notes");
        await expectValue(form.getByLabel("SSH user"), "someone-else", "the SSH user");
        await expectValue(form.getByLabel("SSH password (optional)"), "typed-password", "the typed password");
        await form.getByText("Unsaved changes").waitFor();
        await form.getByRole("button", { name: "Cancel", exact: true }).click();
        await awayAndBack(page, "Network", "Machines");
        if (await page.locator("form").count()) throw new Error("the form opened again after Cancel");
        await page.getByRole("button", { name: "Edit NAS" }).click();
        await expectValue(notes, saved, "the notes after Cancel");
        await expectValue(form.getByLabel("SSH password (optional)"), "", "the password after Cancel");
      },
    },
    {
      what: "Drafts: a group rename half typed stays after looking at another screen, and Cancel lets it go",
      path: "/machines",
      async run(page) {
        await page.getByRole("button", { name: "Rename the group Proxmox lab" }).click();
        const box = page.getByLabel("New name for Proxmox lab");
        await box.fill("Proxmox ho");
        await awayAndBack(page, "Network", "Machines");
        await expectValue(box, "Proxmox ho", "the new name being typed");
        await page.locator("form").getByRole("button", { name: "Cancel", exact: true }).click();
        await awayAndBack(page, "Network", "Machines");
        if (await page.getByLabel("New name for Proxmox lab").count()) throw new Error("the rename opened again after Cancel");
      },
    },
    {
      what: "Drafts: a machine's form kept from before takes a group given meanwhile with the card menu, and keeps what was typed",
      path: "/machines",
      async run(page) {
        const form = page.locator("form");
        await page.getByRole("button", { name: "Edit Printer", exact: true }).click();
        await form.getByLabel("Notes", { exact: true }).fill("By the stairs");
        // Another machine's form: Printer's is kept as typed.
        await page.getByRole("button", { name: "Edit NAS", exact: true }).click();
        await expectValue(form.getByLabel("Name", { exact: true }), "NAS", "the machine being edited");
        await form.getByRole("button", { name: "Cancel", exact: true }).click();
        await page.getByRole("button", { name: "Group for Printer" }).click();
        await page.getByRole("menuitemradio", { name: "Proxmox lab" }).click();
        await group(page, "Proxmox lab").locator("article", { hasText: "Printer" }).waitFor();
        await page.getByRole("button", { name: "Edit Printer", exact: true }).click();
        await expectValue(form.getByLabel("Notes", { exact: true }), "By the stairs", "the notes typed before");
        await expectValue(form.getByLabel("Group (optional)"), "Proxmox lab", "the group given with the menu");
        await page.getByRole("button", { name: "Save", exact: true }).click();
        await form.waitFor({ state: "detached" });
        await group(page, "Proxmox lab").locator("article", { hasText: "Printer" }).waitFor();
      },
    },
    {
      what: "Drafts: unsaved Settings stay after looking at another screen, and Discard puts back what is saved",
      path: "/settings",
      async run(page) {
        const ping = page.getByLabel("How often to check that every device is there");
        await ping.waitFor();
        const saved = await ping.inputValue();
        await ping.fill("45");
        await page.getByLabel("Passphrase for the key file").fill("typed-passphrase");
        await awayAndBack(page, "Overview", "Settings");
        await expectValue(ping, "45", "the ping interval");
        await expectValue(page.getByLabel("Passphrase for the key file"), "typed-passphrase", "the passphrase");
        await page.getByRole("button", { name: "Discard" }).click();
        await expectValue(ping, saved, "the ping interval after Discard");
        await expectValue(page.getByLabel("Passphrase for the key file"), "", "the passphrase after Discard");
      },
    },
    // --- Pi-hole ---
    {
      what: "Pi-hole: a first connection waits for the certificate, and approving it reads the Pi-hole",
      path: "/pihole?phcert",
      async run(page) {
        const two = piholeCard(page, "Pi-hole 2");
        await two.getByText("The password has not been sent").waitFor();
        if (await two.getByText("Queries today").count()) throw new Error("numbers were read before the certificate was approved");
        await two.getByRole("button", { name: "Approve this certificate" }).click();
        await two.getByRole("button", { name: "Yes, it matches" }).click();
        await two.getByText("Queries today").waitFor();
        await two.getByText("check its certificate").waitFor({ state: "detached" });
      },
    },
    {
      what: "Pi-hole: pausing asks first, counts down, and Resume turns blocking back on",
      path: "/pihole",
      async run(page) {
        const one = piholeCard(page, "Pi-hole 1");
        await one.getByText("Blocking", { exact: true }).waitFor();
        await one.getByRole("button", { name: "Pause blocking…" }).click();
        await one.getByRole("button", { name: "5 minutes" }).click();
        await one.getByText("for 5 minutes? Ads and trackers get through").waitFor();
        // Not paused until the owner says yes.
        await page.waitForTimeout(400);
        if (!(await one.getByText("Blocking", { exact: true }).count())) throw new Error("blocking changed before the owner confirmed");
        await one.getByRole("button", { name: "Yes, pause blocking" }).click();
        await one.getByText("Paused", { exact: true }).waitFor();
        await one.getByText(/back on in 4:5\d|back on in 5:00/).waitFor();
        await one.getByRole("button", { name: "Resume blocking" }).click();
        await one.getByText("Blocking", { exact: true }).waitFor();
        await one.getByRole("button", { name: "Pause blocking…" }).waitFor();
      },
    },
    {
      what: "Pi-hole: Pause both asks once and pauses every Pi-hole, Resume both turns them all back on",
      path: "/pihole",
      async run(page) {
        await piholeCard(page, "Pi-hole 2").getByText("Queries today").waitFor();
        await page.getByRole("button", { name: "Pause both…" }).click();
        await page.getByRole("group", { name: "How long to pause blocking on both Pi-holes" }).getByRole("button", { name: "Until I turn it back on" }).click();
        await page.getByText("Pause blocking on both Pi-holes until you turn it back on?").waitFor();
        await page.getByRole("button", { name: "Yes, pause blocking" }).click();
        for (const name of ["Pi-hole 1", "Pi-hole 2"]) await piholeCard(page, name).getByText("Blocking off", { exact: true }).waitFor();
        await page.getByRole("button", { name: "Resume both" }).click();
        for (const name of ["Pi-hole 1", "Pi-hole 2"]) await piholeCard(page, name).getByText("Blocking", { exact: true }).waitFor();
        await page.getByRole("button", { name: "Pause both…" }).waitFor();
      },
    },
    {
      what: "Pi-hole: the lists only one Pi-hole has are named, and nothing offers to change them",
      path: "/pihole",
      async run(page) {
        const lists = page.getByRole("region", { name: "Blocklists compared" });
        await lists.getByText("2 lists differ").waitFor();
        const hagezi = lists.locator("li", { hasText: "hagezi" });
        await hagezi.getByText("Only on Pi-hole 1").waitFor();
        await hagezi.getByText("Pi-hole 2: not there").waitFor();
        const easy = lists.locator("li", { hasText: "Easyprivacy" });
        await easy.getByText("On Pi-hole 1; switched off on Pi-hole 2").waitFor();
        // Written differently on each Pi-hole (host capitals): one list, with both spellings shown.
        await easy.getByText(/Pi-hole 2: https:\/\/V\.Firebog\.net\/hosts\/Easyprivacy\.txt/).waitFor();
        await easy.getByText(/Pi-hole 1: https:\/\/v\.firebog\.net\/hosts\/Easyprivacy\.txt/).waitFor();
        if (await lists.locator("li", { hasText: "StevenBlack" }).count()) throw new Error("a trailing / made the StevenBlack list look like two lists");
        if (await lists.getByRole("button").count()) throw new Error("the comparison has buttons; it should only show the difference");
      },
    },
    {
      what: "Pi-hole: a wrong password is shown as a problem with an agent prompt, and a new password fixes it",
      path: "/pihole?phwrong",
      async run(page) {
        const two = piholeCard(page, "Pi-hole 2");
        const problem = two.locator(".alert.problem", { hasText: "didn't accept the password" });
        await problem.waitFor();
        await problem.getByRole("button", { name: "Copy a prompt for an agent" }).waitFor();
        await two.getByRole("button", { name: "Connection settings" }).click();
        await two.getByLabel("Password").fill("the-new-app-password");
        await two.getByRole("button", { name: "Save", exact: true }).click();
        await two.getByText("Queries today").waitFor();
        await problem.waitFor({ state: "detached" });
      },
    },
    {
      what: "Pi-hole: the Overview is quiet when all is well, and flags a paused or unreachable Pi-hole",
      path: "/",
      async run(page) {
        const panel = page.getByRole("region", { name: "Pi-hole", exact: true });
        await panel.getByText("Blocking").first().waitFor();
        await page.waitForTimeout(300);
        // The sample has other things that need attention (machine stats of Pi-hole 1 among them), but nothing
        // from the Pi-hole screen.
        const said = await page.locator('.issues a[href="/pihole"]').allTextContents();
        if (said.length) throw new Error(`the Overview flags a Pi-hole that is fine: ${said.join("; ")}`);
        await page.goto(page.url().replace(/\/$/, "") + "/?phoff&phdown", { waitUntil: "networkidle" });
        const issues = page.locator(".issues");
        await issues.getByRole("link", { name: /Pi-hole 1: paused, back on in/ }).waitFor();
        await issues.getByRole("link", { name: "Pi-hole 2 can't be reached" }).waitFor();
      },
    },
    {
      what: "Pi-hole: a password typed in the setup stays after looking at another screen, and Discard clears it",
      path: "/pihole?phnopw",
      async run(page) {
        const one = piholeCard(page, "Pi-hole 1");
        await one.getByLabel("Password").fill("typed-secret");
        await one.getByLabel("HTTP: only if this Pi-hole doesn't offer HTTPS.").check();
        await expectValue(one.getByLabel("Port"), "80", "the port after choosing HTTP");
        await awayAndBack(page, "Settings", "Pi-hole");
        await expectValue(one.getByLabel("Password"), "typed-secret", "the password typed before");
        if (!(await one.getByLabel("HTTP: only if this Pi-hole doesn't offer HTTPS.").isChecked())) throw new Error("the HTTP choice was lost");
        await one.getByText("Unsaved changes").waitFor();
        await one.getByRole("button", { name: "Discard" }).click();
        await expectValue(one.getByLabel("Password"), "", "the password after Discard");
        await expectValue(one.getByLabel("Port"), "443", "the port after Discard");
      },
    },
    {
      what: "Pi-hole: setup adds a machine only when the owner presses Add, then asks for the password and the certificate",
      path: "/pihole?phnone",
      async run(page) {
        await page.getByText("Its name says Pi-hole").first().waitFor();
        await page.waitForTimeout(300);
        if (await page.getByRole("article").count()) throw new Error("a suggested machine was added by itself");
        await page.getByRole("button", { name: "Add Pi-hole 2 as a Pi-hole" }).click();
        const two = piholeCard(page, "Pi-hole 2");
        await two.getByText("Add this Pi-hole's password").waitFor();
        await two.getByLabel("Password").fill("app-password");
        await two.getByRole("button", { name: "Save", exact: true }).click();
        await two.getByText("check its certificate").waitFor();
        await two.getByRole("button", { name: "Approve this certificate" }).click();
        await two.getByRole("button", { name: "Yes, it matches" }).click();
        await two.getByText("Queries today").waitFor();
      },
    },
    // --- end Pi-hole ---
    // Tables: sorting, grouping and filtering (components/table), on a table of each screen that has one.
    {
      what: "Tables: a header click sorts Network devices by address number by number, both ways, then back to the usual order; kept after another screen",
      path: "/network",
      async run(page) {
        const devices = panelOf(page, "Devices");
        const byNumber = [10, 21, 22, 30, 31, 32, 33, 40, 41, 60, 70, 95, 101, 102].map((n) => `192.168.68.${n}`);
        const address = devices.getByRole("button", { name: "Address", exact: true });
        // The header is a button, so the keyboard reaches it.
        await address.focus();
        await page.keyboard.press("Enter");
        await devices.locator('th.c-ip[aria-sort="ascending"]').waitFor();
        await expectCells(devices, "c-ip", byNumber, "addresses lowest first", "> .ip");
        await address.click();
        await devices.locator('th.c-ip[aria-sort="descending"]').waitFor();
        await expectCells(devices, "c-ip", [...byNumber].reverse(), "addresses highest first", "> .ip");
        await awayAndBack(page, "Machines", "Network");
        await devices.locator('th.c-ip[aria-sort="descending"]').waitFor();
        await expectCells(devices, "c-ip", [...byNumber].reverse(), "addresses highest first after another screen", "> .ip");
        await expectValue(devices.getByLabel("Sort devices"), "address:desc", "the sort menu");
        await address.click();
        if (await devices.locator("th[aria-sort]").count()) throw new Error("a third click left the table sorted");
        await expectValue(devices.getByLabel("Sort devices"), "", "the sort menu after a third click");
        await expectCells(devices, "c-ip", byNumber, "the usual order (by address) after a third click", "> .ip");
      },
    },
    {
      what: "Tables: devices with no maker or no response time go last, whichever way round they are sorted",
      path: "/network",
      async run(page) {
        const devices = panelOf(page, "Devices");
        const sort = devices.getByLabel("Sort devices");
        for (const [how, last] of [
          ["maker", ["Sam's phone", "Tablet"]],
          ["response", ["NAS", "Living room TV", "Main PC", "Media PC", "Old laptop"]],
        ]) {
          for (const dir of ["asc", "desc"]) {
            await sort.selectOption(`${how}:${dir}`);
            const names = await cellTexts(devices, "c-name");
            const got = names.slice(-last.length);
            if (got.join("|") !== last.join("|")) throw new Error(`sorted by ${how} (${dir}), the last rows are ${got.join(", ")}`);
          }
        }
        // The rest by their value: response times as numbers, not as text ("10 ms" after "2.3 ms").
        await sort.selectOption("response:asc");
        const times = (await cellTexts(devices, "c-rtt")).slice(0, 9).map((t) => parseFloat(t.replace("<", "")));
        if (times.some((t, i) => Number.isNaN(t) || (i > 0 && t < times[i - 1]))) throw new Error(`response times lowest first read ${times.join(", ")}`);
      },
    },
    {
      what: "Tables: the filter finds devices by hostname or address, counts them, says when nothing matches, and is kept after another screen but never stored",
      path: "/network",
      async run(page) {
        const devices = panelOf(page, "Devices");
        const box = devices.getByLabel("Filter devices");
        // Proxmox's hostname, which isn't in its name.
        await box.fill("pve");
        await expectCells(devices, "c-name", ["Proxmox"], "found by hostname");
        await devices.getByText("1 of 14 shown").waitFor();
        await box.fill("68.3");
        await expectCells(devices, "c-name", ["Proxmox", "Pi-hole 1", "Pi-hole 2", "Monitoring"], "found by address");
        await awayAndBack(page, "Machines", "Network");
        await expectValue(box, "68.3", "the filter after another screen");
        await devices.getByText("4 of 14 shown").waitFor();
        await box.fill("nothing-like-this");
        await devices.getByText('No devices match "nothing-like-this".').waitFor();
        if (await devices.locator("table").count()) throw new Error("an empty table is drawn under the message");
        await devices.getByRole("button", { name: "Clear the filter" }).click();
        await expectValue(box, "", "the filter after Clear the filter");
        if ((await cellTexts(devices, "c-name")).length !== 14) throw new Error("Clear the filter didn't bring every device back");
        // A quick choice of one status.
        await devices.getByLabel("Show only devices with this Status").selectOption("Down");
        await expectCells(devices, "c-name", ["Living room TV", "Old laptop"], "only the devices that are down");
        await devices.getByText("2 of 14 shown").waitFor();
        // The filter is never written down; the sort is, for next time.
        await box.fill("printer");
        await devices.getByLabel("Sort devices").selectOption("address:desc");
        const stored = await page.evaluate(() => JSON.stringify(localStorage));
        if (/printer/i.test(stored)) throw new Error("the filter text was put in the window's storage");
        await page.reload({ waitUntil: "networkidle" });
        await devices.locator("table").waitFor();
        await expectValue(box, "", "the filter after the app started again");
        await expectValue(devices.getByLabel("Sort devices"), "address:desc", "the sort after the app started again");
        await expectValue(devices.getByLabel("Show only devices with this Status"), "\u0000all", "the status choice after the app started again");
      },
    },
    {
      what: "Tables: in a grouped and sorted table, an address and its hostname still copy on a double-click",
      path: "/network",
      async run(page) {
        await page.evaluate(() => {
          window.__copied = [];
          const write = (t) => (window.__copied.push(t), Promise.resolve());
          Object.defineProperty(navigator, "clipboard", { value: { writeText: write }, configurable: true });
        });
        const devices = panelOf(page, "Devices");
        await devices.getByLabel("Group devices").selectOption("type");
        await devices.getByLabel("Sort devices").selectOption("address:desc");
        const row = devices.locator("tbody tr", { has: page.locator("td.c-ip", { hasText: "192.168.68.30" }) });
        for (const [what, locator, want] of [
          ["address", row.locator("td.c-ip .copyable"), "192.168.68.30"],
          ["hostname", row.locator("td.c-host .copyable"), "pve"],
        ]) {
          await locator.dblclick();
          await page.locator(".note", { hasText: "Copied" }).waitFor({ timeout: 2000 });
          const got = await page.evaluate(() => window.__copied.at(-1));
          if (got !== want) throw new Error(`a double-click on Proxmox's ${what} copied "${got}", not "${want}"`);
        }
      },
    },
    {
      what: "Tables: All machines grouped by group, with a heading and count for each, collapsing, and both kept after another screen",
      path: "/machines",
      async run(page) {
        const all = panelOf(page, "All machines");
        await all.getByLabel("Group machines").selectOption("group");
        const groups = () =>
          all.evaluate((el) =>
            [...el.querySelectorAll("tbody")].map((b) => ({
              label: b.querySelector(".group-row .label")?.textContent.trim(),
              count: Number(b.querySelector(".group-row .count")?.textContent.match(/\d+/)?.[0]),
              rows: b.querySelectorAll("tr:not(.group-row)").length,
            })),
          );
        const check = async (want, what) => {
          const got = await groups();
          const read = got.map((g) => `${g.label} ${g.count}/${g.rows}`).join(", ");
          if (read !== want) throw new Error(`${what}: the groups read ${read}, not ${want}`);
        };
        await check("NAS PC 3/3, Proxmox lab 4/4, In no group 5/5", "grouped by group");
        await all.getByRole("button", { name: /^NAS PC/ }).click();
        await all.getByRole("button", { name: /^NAS PC/, expanded: false }).waitFor();
        await check("NAS PC 3/0, Proxmox lab 4/4, In no group 5/5", "with NAS PC collapsed");
        await awayAndBack(page, "Network", "Machines");
        await expectValue(all.getByLabel("Group machines"), "group", "the grouping after another screen");
        await check("NAS PC 3/0, Proxmox lab 4/4, In no group 5/5", "after another screen");
        // Sorted by the Group column backwards, the groups go the other way round; machines in none stay last.
        await all.getByLabel("Sort machines").selectOption("group:desc");
        await check("Proxmox lab 4/4, NAS PC 3/0, In no group 5/5", "sorted by group backwards");
        // Inside a group, the rows follow the sort.
        await all.getByLabel("Sort machines").selectOption("name:desc");
        const lab = await all.locator("tbody", { has: page.getByRole("button", { name: /^Proxmox lab/ }) }).locator("td.c-name").allInnerTexts();
        if (lab.join() !== "Proxmox,Pi-hole 2,Pi-hole 1,Monitoring") throw new Error(`Proxmox lab, names Z to A: ${lab.join(", ")}`);
        await all.getByLabel("Group machines").selectOption("");
        if (await all.locator("tr.group-row").count()) throw new Error("No grouping still shows group headings");
      },
    },
    {
      what: "Tables: Proxmox guests sort by ID, group by type and filter by tag, all kept after another screen",
      path: "/proxmox",
      async run(page) {
        const guests = page.getByRole("region", { name: "VMs and containers" });
        await guests.locator("table").waitFor();
        await guests.getByLabel("Show templates").check();
        const id = guests.getByRole("button", { name: "ID", exact: true });
        await id.click();
        await expectCells(guests, "c-id", ["101", "102", "103", "110", "111", "120", "9000"], "IDs lowest first");
        await id.click();
        await expectCells(guests, "c-id", ["9000", "120", "111", "110", "103", "102", "101"], "IDs highest first");
        // A stopped guest uses no memory: last, whichever way round.
        for (const dir of ["asc", "desc"]) {
          await guests.getByLabel("Sort VMs and containers").selectOption(`ram:${dir}`);
          const last = (await cellTexts(guests, "c-name", ".name")).slice(-2);
          if (last.join() !== "win11-test,debian-12-template") throw new Error(`by RAM (${dir}), the last rows are ${last.join(", ")}`);
        }
        await guests.getByLabel("Group VMs and containers").selectOption("type");
        const headings = await guests.locator("tr.group-row .label").allInnerTexts();
        if (headings.join() !== "Container,VM") throw new Error(`grouped by type: ${headings.join(", ")}`);
        await guests.getByLabel("Sort VMs and containers").selectOption("id:asc");
        await guests.getByLabel("Filter VMs and containers").fill("dns");
        await expectCells(guests, "c-name", ["pihole1", "pihole2"], "filtered by the tag dns", ".name");
        await guests.getByText("2 of 7 shown").waitFor();
        await awayAndBack(page, "Terminals", "Proxmox");
        await guests.locator("table").waitFor();
        await expectValue(guests.getByLabel("Filter VMs and containers"), "dns", "the filter after another screen");
        await expectValue(guests.getByLabel("Group VMs and containers"), "type", "the grouping after another screen");
        await expectCells(guests, "c-name", ["pihole1", "pihole2"], "after another screen", ".name");
      },
    },
    {
      what: "Tables: a device being renamed stays where it is, with what was typed, when it goes down while only devices that are up are shown or they are grouped by status",
      path: "/network",
      async run(page) {
        // From here on the back end says the devices set with window.__setStatus are down or up, and
        // status-changed makes the screen ask, as the timer would.
        await page.evaluate(async () => {
          const tauri = window.__TAURI_INTERNALS__;
          const invoke = tauri.invoke;
          const list = await invoke("list_devices");
          const forced = new Map();
          tauri.invoke = async (cmd, args, ...rest) => {
            const got = await invoke(cmd, args, ...rest);
            if (cmd !== "device_statuses") return got;
            return got.map((s) => (forced.has(s.deviceId) ? { ...s, up: forced.get(s.deviceId), rttMs: forced.get(s.deviceId) ? 4 : null } : s));
          };
          window.__setStatus = async (name, up) => {
            forced.set(list.find((d) => d.name === name).id, up);
            await invoke("plugin:event|emit", { event: "status-changed", payload: null });
          };
        });
        const devices = panelOf(page, "Devices");
        const choice = devices.getByLabel("Show only devices with this Status");
        const downCount = (n) => choice.locator("option", { hasText: `Status: Down (${n})` }).waitFor({ state: "attached" });
        /** Starts renaming a device, types after its name, and takes the pointer off the table. */
        const rename = async (name) => {
          await devices.getByRole("button", { name, exact: true }).click();
          const field = page.getByLabel(`New name for ${name}`);
          await field.press("End");
          await field.pressSequentially(" box");
          await page.mouse.move(1, 1);
          return field;
        };
        const stillTyping = async (field, want, what) => {
          const got = await field.evaluate((el) => ({
            focused: document.activeElement === el,
            value: el.value,
            allSelected: el.value.length > 0 && el.selectionStart === 0 && el.selectionEnd === el.value.length,
          }));
          if (!got.focused || got.value !== want || got.allSelected) {
            throw new Error(`${what}: the rename field is ${got.focused ? "" : "not "}focused, reads "${got.value}"${got.allSelected ? ", all selected" : ""}`);
          }
        };

        // Only the devices that are up: Pi-hole 2 goes down while it is being renamed, and stays on screen.
        await choice.selectOption("Up");
        const pihole = await rename("Pi-hole 2");
        await page.evaluate(() => window.__setStatus("Pi-hole 2", false));
        await downCount(3);
        await stillTyping(pihole, "Pi-hole 2 box", "showing only devices that are up");
        // Let go of, it goes, as it is down.
        await pihole.press("Escape");
        await devices.getByRole("button", { name: "Pi-hole 2", exact: true }).waitFor({ state: "detached" });
        await choice.selectOption("\u0000all");

        // Grouped by status: Monitoring goes down while it is being renamed and stays under Up.
        await devices.getByLabel("Group devices").selectOption("status");
        // (The inner locator of "has" is looked for inside each tbody, so it starts from the page.)
        const groupOf = (has) => devices.locator("tbody", { has }).locator(".group-row .label").innerText();
        const monitoring = await rename("Monitoring");
        if ((await groupOf(monitoring)) !== "Up") throw new Error("Monitoring isn't under Up to begin with");
        await page.evaluate(() => window.__setStatus("Monitoring", false));
        await downCount(4);
        await stillTyping(monitoring, "Monitoring box", "grouped by status");
        if ((await groupOf(monitoring)) !== "Up") throw new Error("Monitoring moved to another group while it was being renamed");
        await monitoring.pressSequentially("2");
        await stillTyping(monitoring, "Monitoring box2", "typing on, grouped by status");
        // The rename given up, the field goes: the table is let go and Monitoring moves under Down.
        await monitoring.press("Escape");
        const button = page.getByRole("button", { name: "Monitoring", exact: true });
        const until = Date.now() + 5000;
        while ((await groupOf(button)) !== "Down") {
          if (Date.now() > until) throw new Error(`once let go, Monitoring is under ${await groupOf(button)}, not Down`);
          await page.waitForTimeout(100);
        }
      },
    },
    {
      what: "Tables: a status chosen above the table that no device has any more still shows in its menu, with none",
      path: "/network",
      async run(page) {
        await page.evaluate(async () => {
          const tauri = window.__TAURI_INTERNALS__;
          const invoke = tauri.invoke;
          const list = await invoke("list_devices");
          const ids = new Set(list.filter((d) => d.name === "Living room TV" || d.name === "Old laptop").map((d) => d.id));
          let allUp = false;
          tauri.invoke = async (cmd, args, ...rest) => {
            const got = await invoke(cmd, args, ...rest);
            if (cmd !== "device_statuses" || !allUp) return got;
            return got.map((s) => (ids.has(s.deviceId) ? { ...s, up: true, rttMs: 4 } : s));
          };
          window.__bringUp = async () => {
            allUp = true;
            await invoke("plugin:event|emit", { event: "status-changed", payload: null });
          };
        });
        const devices = panelOf(page, "Devices");
        const choice = devices.getByLabel("Show only devices with this Status");
        await choice.selectOption("Down");
        await expectCells(devices, "c-name", ["Living room TV", "Old laptop"], "only the devices that are down");
        await page.mouse.move(1, 1);
        await page.evaluate(() => window.__bringUp());
        await devices.getByText("No devices match Status: Down.").waitFor();
        const shown = await choice.evaluate((s) => ({ value: s.value, text: s.selectedOptions[0]?.text }));
        if (shown.value !== "Down" || shown.text !== "Status: Down (0)") throw new Error(`the status menu shows "${shown.text}" (${shown.value})`);
        await devices.getByRole("button", { name: "Clear the filter" }).click();
        if ((await choice.locator("option", { hasText: "Down" }).count()) !== 0) throw new Error("Down is still offered once it is no longer chosen");
      },
    },
    {
      what: "Tables: a Proxmox guest started from its row moves to its new place once the pointer has gone, though its Start button had focus",
      path: "/proxmox",
      async run(page) {
        const guests = page.getByRole("region", { name: "VMs and containers" });
        await guests.locator("table").waitFor();
        await guests.getByLabel("Sort VMs and containers").selectOption("state:asc");
        const before = await cellTexts(guests, "c-name", ".name");
        if (before[0] !== "win11-test") throw new Error(`stopped first, the first row is ${before[0]}`);
        const row = guests.locator("tbody tr", { hasText: "win11-test" });
        const start = row.getByRole("button", { name: "Start", exact: true });
        // As in the app's window on Linux (WebKit): a focused button taken away gives no focusout. Chromium
        // gives one, so it is held back here.
        await start.evaluate((button) =>
          window.addEventListener("focusout", (e) => e.target === button && e.stopImmediatePropagation(), true),
        );
        await start.click();
        await row.getByText("Starting…").waitFor();
        await page.mouse.move(1, 1);
        await expectCells(guests, "c-name", [...before.slice(1), "win11-test"], "once started", ".name");
      },
    },
    {
      what: "Tables: a stack's containers sort by memory, group by state and filter by port or address, all kept after another screen",
      path: "/docker",
      async run(page) {
        await page.getByText("612 MB").waitFor();
        const media = page.getByRole("article", { name: "Stack media" });
        const names = () => media.locator('[role="rowgroup"] [role="row"]:not(.group-row) .names .name').allInnerTexts();
        const expectNames = async (want, what) => {
          const until = Date.now() + 5000;
          let got;
          while ((got = await names()).join() !== want.join()) {
            if (Date.now() > until) throw new Error(`${what}: ${got.join(", ")}`);
            await page.waitForTimeout(100);
          }
        };
        const memory = media.getByRole("button", { name: "Memory", exact: true });
        await memory.click();
        await media.locator('[role="columnheader"][aria-sort="ascending"]').waitFor();
        // The stopped one uses none: last, whichever way round.
        await expectNames(["radarr", "sonarr", "jellyfin", "prowlarr"], "memory lowest first");
        await memory.click();
        await expectNames(["jellyfin", "sonarr", "radarr", "prowlarr"], "memory highest first");
        await media.getByLabel("Group containers of media").selectOption("state");
        const headings = await media.locator(".group-row .label").allInnerTexts();
        if (headings.join() !== "Running,Stopped") throw new Error(`grouped by state: ${headings.join(", ")}`);
        await expectNames(["jellyfin", "sonarr", "radarr", "prowlarr"], "grouped by state, by memory inside");
        const filter = media.getByLabel("Filter containers of media");
        await filter.fill("8096");
        await expectNames(["jellyfin"], "filtered by a port");
        await media.getByText("1 of 4 shown").waitFor();
        // The hostname shown with the address a port is published on.
        await filter.fill("homelab");
        await expectNames(["jellyfin"], "filtered by the hostname of the address a port is published on");
        await awayAndBack(page, "Machines", "Docker");
        await expectValue(filter, "homelab", "the filter after another screen");
        await expectValue(media.getByLabel("Group containers of media"), "state", "the grouping after another screen");
        await expectNames(["jellyfin"], "after another screen");
        await filter.fill("");
        await media.getByLabel("Show only containers of media with this State").selectOption("Stopped");
        await expectNames(["prowlarr"], "only the stopped ones");
        // Each stack's list is its own.
        const others = page.getByRole("article", { name: "Other containers" });
        if ((await others.getByLabel("Group other containers").inputValue()) !== "") throw new Error("another stack's list is grouped too");
      },
    },
    {
      what: "Tables: in the smallest window at 200%, with no headers, the bar sorts and groups All machines and names a sort whose column is hidden",
      path: "/machines",
      viewport: { width: 480, height: 300 },
      scale: 2,
      async run(page) {
        const all = panelOf(page, "All machines");
        const sort = all.getByLabel("Sort machines");
        await sort.selectOption("group:asc");
        if (await all.locator("th.c-group").isVisible()) throw new Error("the Group column is shown at this width, so this proves nothing");
        const named = await sort.evaluate((s) => s.selectedOptions[0]?.text);
        if (named !== "Sort: Group, A to Z") throw new Error(`the sort menu reads "${named}"`);
        const first = await cellTexts(all, "c-name");
        if (first.slice(0, 3).join() !== "Dev VM,Homelab VM,NAS") throw new Error(`sorted by group, the first rows are ${first.slice(0, 3).join(", ")}`);
        await all.getByLabel("Group machines").selectOption("type");
        const heading = all.locator("tr.group-row").first();
        const fits = await heading.evaluate((tr) => {
          const table = tr.closest("table").getBoundingClientRect();
          const box = tr.getBoundingClientRect();
          return { display: getComputedStyle(tr).display, wide: box.width >= table.width - 1 };
        });
        if (fits.display !== "block" || !fits.wide) throw new Error(`a group heading is ${fits.display}, ${fits.wide ? "" : "not "}as wide as the table`);
        const found = await page.evaluate(inspect);
        if (found.overflow.length) throw new Error(found.overflow.join("; "));
      },
    },
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
