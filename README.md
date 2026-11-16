# home-cc

A personal desktop app (Windows and Linux) for managing a home network:
mesh nodes, PCs and VMs, Docker containers, and Proxmox.

Built with Tauri 2, Rust and Svelte. See [docs/SPEC.md](docs/SPEC.md) for what
it does and the build plan.

## Getting installers

Every push builds a Windows installer (`.msi` and `.exe`) and Linux packages
(`.deb` and `.AppImage`) on GitHub Actions. Open the latest run under the
**Actions** tab and download them from **Artifacts** at the bottom.

## Running from source

You need Node.js 22+, Rust (stable), and the Tauri system packages:

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
cargo test --manifest-path src-tauri/Cargo.toml # back-end tests
```

## Layout

- `src/` – the screens (Svelte). One folder per module under `src/routes/`,
  listed in `src/lib/modules.ts`. Theme colours are in `src/app.css`.
- `src-tauri/src/` – the back end (Rust).
  - `inventory.rs` – saved devices, in a SQLite file in the app data folder.
  - `secrets.rs` – passwords and tokens, kept in the system keychain.
  - `commands.rs` – everything the screens can ask the back end to do.
