# home-cc

A personal desktop app (Windows and Linux) for managing a home network:
mesh nodes, PCs and VMs, Docker containers, Proxmox, and Pi-hole.

Built with Tauri 2, Rust and Svelte. See [docs/SPEC.md](docs/SPEC.md) for what
it does and the build plan.

## Getting installers

GitHub Actions can build a Windows installer (`.msi` and `.exe`) and Linux
packages (`.deb` and `.AppImage`), but only when started by hand, to save the
free minutes: under the **Actions** tab pick **Build**, then **Run workflow**.
When it finishes, download them from **Artifacts** at the bottom of the run.
For everyday use, build locally instead (see below).

## Running from source

You need Node.js 22.18 or newer, Rust (stable), and the Tauri system packages:

- **Windows:** Microsoft C++ Build Tools and WebView2 (already on Windows 11).
- **Linux (Debian/Ubuntu):**
  `sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libssl-dev build-essential`

Then:

```sh
npm install
npm run tauri dev     # run the app with live reload
npm run tauri build   # make an installer for this system
```

On Windows, `build-and-install.bat` does it all: double-click it to get the
latest version from GitHub, install the packages, build, and open the new
installer. `build-and-install.bat --no-pull` builds what is in the folder
without updating first.

## Checks

```sh
npm run check                                   # type-check the screens
npm test                                        # front-end tests
cargo test --manifest-path src-tauri/Cargo.toml # back-end tests
```

### Local CI in Docker

To run the same checks as GitHub Actions without using its minutes (needs
Docker; nothing else has to be installed for the build itself):

```sh
npm run ci         # type-check, tests, then build the Linux packages
npm run ci:check   # type-check and tests only
```

The `.deb` and `.AppImage` land in `ci/out/`. The first run downloads and
compiles everything (about 10 minutes). After that the caches live in Docker
volumes: checks take seconds, and a full run takes about 3 minutes, most of it
the optimised release build.
The build works on a copy of the source inside the container, so nothing in
your checkout changes. Without npm on the machine, run
`docker compose -f ci/compose.yml run --rm --build ci` directly (add `check`
for checks only). To clear the caches:
`docker compose -f ci/compose.yml down -v`.

Windows installers can't be built this way; run `npm run tauri build` on a
Windows machine.

## Layout

- `src/` – the screens (Svelte). One folder per module under `src/routes/`,
  listed in `src/lib/modules.ts`. Theme colours are in `src/app.css`.
  Shared parts are in `src/lib/components/`. Every IP address on screen is
  shown with `Address.svelte`, which adds the hostname it resolves to as a
  field of its own (in a table, a "Hostname" column), so the owner can tell
  which machine it is. New screens do the same.
  A screen is drawn afresh each time it is opened, so anything typed into a
  form, and which panel was open, is kept in `src/lib/drafts.svelte.ts` while
  another screen is shown (in memory only, as it can hold a password). A form
  fills itself from its draft first, shows "Unsaved changes" with Discard, and
  clears the draft when it is saved, cancelled or closed. New forms do the same.
  Choices that are about this PC's screen rather than the machines are kept in
  the window's own storage, never in the back end: the order of the Machines
  cards and collapsed groups (`src/lib/order.svelte.ts`) and the machines
  silenced on the Overview (`src/lib/silence.svelte.ts`, its rules in
  `overview.ts`). That storage can be missing, so every read and write is in a
  try/catch and the screen still works without it.
  Every table can be sorted, grouped and filtered the same way. The rules are
  in `src/lib/table.ts` (tested by `table.test.mjs`): text in a person's order
  ("PC 2" before "PC 10"), numbers by value, IP addresses number by number,
  times by time, states down → warning → up, and empty cells last both ways.
  To give a new table the same:
  1. List its columns as `Column`s: `value` (what it sorts on) and `kind`,
     `text` (the words shown, for group headings and choices), `search` (what
     the filter looks at, such as an address and its hostname), `group: true`
     for values many rows share (type, state, group; not names or addresses),
     `choose: true` for a quick choice above the table, `sortable: false` for
     buttons.
  2. Make a `TableView` (`src/lib/table.svelte.ts`) with a unique `id`, the
     columns, the rows, a key per row, and its own order in words (`usual`).
     It keeps the filter, sort, grouping and collapsed groups while another
     screen is shown (the drafts store), and the sort and grouping in the
     window's storage for next time; the filter text is never stored.
  3. Put `<TableTools>` above the table, `use:view.watch` on the `<table>`,
     `<SortButton>` in each sortable `<th>` (with `aria-sort={view.ariaSort(id)}`),
     and draw `view.sections` as one `<tbody>` each, starting with
     `<GroupRow>` when `section.label` isn't null and showing the rows only
     when `section.open`. All three parts are in `src/lib/components/table/`.
     A list built of other elements (the containers of a Docker stack) takes
     `role="table"`, one `role="rowgroup"` per section, and `<GroupRow grid>`.
  The bar names the sort and grouping in use, so a column hidden at a narrow
  width (or the headers, when rows become blocks) never sorts without saying.
  Rows don't move under the pointer or while something in a row has focus:
  not to another place, another group, or out of the filter (`arrange` in
  `table.ts`). Focus is checked where it is now, so a focused button that is
  taken away (Start swapped for "Starting…") lets the table go again.
  The zoom keys (Ctrl with + / - / 0) and Ctrl with the wheel are caught for
  the whole window in `src/lib/zoom.svelte.ts`, before any box or terminal
  sees them. Inside a terminal (anything in a `.terminal-host`) they change
  `terminalZoom`, the terminals' own text size, and are kept from the program;
  elsewhere the app's zoom. Each terminal's font is the chosen size divided by
  the app's zoom (`terminalFontSize` in `src/lib/zoom.ts`), so one never
  changes the other on the screen.
  Every failure is shown with `Problem.svelte`, which offers prompts to copy
  for a coding agent (`AgentPrompt.svelte`; the text is built in
  `src/lib/report.ts`, tested by `report.test.mjs`). A problem about a machine
  or a service on it gets three: Investigate (look into it there, change
  nothing), Remediate (fix it there, saying first what will change and asking
  before anything destructive) and I have a bug (find the fault in home-cc's
  code). A problem about home-cc itself gets only the last. Which is which is
  decided in one place, `AREA_REACH` and `reachOf` in `report.ts`: by the
  problem's area, and for a machine's area only when its details name the
  machine's address (`deviceDetails`). A problem on a machine's screen that
  is really about home-cc (a setting or password it couldn't store) or this
  PC (its own SSH key) says so with `about`. Give a new screen's problems the
  device's details, and a new area its entry there. Never put a secret in the
  details: say whether one is stored instead.
- `src-tauri/src/` – the back end (Rust).
  - `inventory.rs` – saved devices, in a SQLite file in the app data folder.
    `settings.rs`, `history.rs` (ping results) and `hostkeys.rs` (SSH
    identities) keep their tables in the same file. A value that is the
    owner's own (the default SSH user, the key file, the range to scan) is a
    field of `Settings` in `settings.rs` (with `#[serde(default)]`, so a
    settings file from before it still loads), never written into the code;
    hints on screen use made-up examples.
  - `secrets.rs` – passwords and tokens, kept in the system keychain.
  - `net/` – finding the home network, scanning it, naming devices and
    checking they are up.
  - `ssh.rs`, `stats.rs` – SSH logins, and the machine stats read over them.
    `localstats.rs` reads this PC's own stats with the same scripts, without SSH.
  - `keysetup.rs` – key login: shows this PC's key, makes one if there is
    none, and adds it to a machine (like ssh-copy-id), then checks it works.
  - `monitor.rs` – the background timers for status checks and stats.
  - `terminal.rs` – terminal tabs over SSH, and copy and paste for them. Its
    test against a real SSH server (vim, tmux, htop) runs with
    `HOME_CC_TEST_SSH=host:port:user:password cargo test real_server -- --ignored`.
  - `pihole/` – Pi-hole v6's API: logging in and keeping one session per
    Pi-hole (`auth.rs`), HTTPS with the approved certificate or HTTP
    (`client.rs`), and which devices are Pi-holes (`store.rs`).
  - `commands.rs` – everything the screens can ask the back end to do.
- `src-tauri/examples/` – small command-line programs for trying the network
  and SSH code by itself, for example
  `cargo run --manifest-path src-tauri/Cargo.toml --example net_scan`.
- `tools/` – `screenshots.mjs` (pictures of every screen at each monitor size,
  using sample data) and `build_oui.py` (refreshes the list of device makers).
