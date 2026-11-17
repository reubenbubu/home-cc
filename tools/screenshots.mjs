// Saves a screenshot of every screen at each of the owner's monitor sizes,
// using the sample back end (src/lib/mock), and checks that nothing scrolls
// sideways. Needs Playwright with Chromium, installed globally or locally.
//
//   node tools/screenshots.mjs <output folder> [--url http://127.0.0.1:1420]
//        [--only overview,settings] [--sizes 1920x1080,1280x720@1.5]
//
// --sizes takes any WIDTHxHEIGHT, with @SCALE for display scaling.
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
      await page.getByText("First connection to Media PC").waitFor();
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
];

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
  const args = { out: null, url: null, only: null, sizes: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--url" || a === "--only" || a === "--sizes") args[a.slice(2)] = argv[++i] ?? "";
    else if (!args.out) args.out = a;
  }
  return args;
}

function parseSize(name) {
  const m = /^(\d+)x(\d+)(?:@(\d+(?:\.\d+)?))?$/.exec(name);
  if (!m) throw new Error(`"${name}" is not a size. Write it like 1920x1080 or 1536x864@1.25.`);
  return { name, width: Number(m[1]), height: Number(m[2]), scale: Number(m[3] ?? 1) };
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
        await media.getByText("First connection").waitFor({ state: "detached" });
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
        await media.getByText("First connection").waitFor({ state: "detached" });
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
    console.error("usage: node tools/screenshots.mjs <output folder> [--url URL] [--only names] [--sizes names]");
    process.exit(2);
  }
  const out = resolve(args.out);
  mkdirSync(out, { recursive: true });
  const screens = args.only ? SCREENS.filter((s) => args.only.split(",").includes(s.name)) : SCREENS;
  const sizes = args.sizes ? args.sizes.split(",").map(parseSize) : SIZES;

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
