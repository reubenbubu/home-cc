# home-cc

A personal desktop app (Windows and Linux) for managing a home network:
mesh nodes, PCs and VMs, Docker containers, and Proxmox.

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
- `src-tauri/src/` – the back end (Rust).
  - `inventory.rs` – saved devices, in a SQLite file in the app data folder.
    `settings.rs`, `history.rs` (ping results) and `hostkeys.rs` (SSH
    identities) keep their tables in the same file.
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
  - `commands.rs` – everything the screens can ask the back end to do.
- `src-tauri/examples/` – small command-line programs for trying the network
  and SSH code by itself, for example
  `cargo run --manifest-path src-tauri/Cargo.toml --example net_scan`.
- `tools/` – `screenshots.mjs` (pictures of every screen at each monitor size,
  using sample data) and `build_oui.py` (refreshes the list of device makers).
