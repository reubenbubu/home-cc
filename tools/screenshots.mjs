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
// an error, or can't cope with the unusual readings of "?awkward".
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
  { name: "machines", path: "/machines", ready: "article" },
  {
    name: "machines-edit",
    path: "/machines",
    ready: "article",
    async before(page) {
      await page.getByRole("button", { name: "Edit NAS" }).click();
      await page.getByText("A password is stored").waitFor();
    },
  },
  { name: "settings", path: "/settings", ready: "form" },
];

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
