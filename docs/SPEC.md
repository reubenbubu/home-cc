# home-cc spec

home-cc is a personal desktop app for managing a home network: the mesh
nodes, the PCs and VMs on it, the Docker containers they run, and a Proxmox
server. It is for one person (the owner) and runs on their own Linux or
Windows PC.

This document says what the app should do, how it reaches each kind of
machine, and what "done" means for each feature. Items marked **OPEN** need
an answer from the owner.

## 1. Goals

- One window that shows everything on the home network and whether it is healthy.
- Act on any machine from that window: open a terminal, manage containers, manage VMs.
- Easy to grow. Each area is a separate module, so new things can be added later
  without touching the rest.

Not goals for now:

- Multiple users, logins or permissions.
- Running when the app is closed (no background alerts). A small helper service
  could be added later if this is wanted.
- Access from outside the home network. (A VPN such as WireGuard or Tailscale
  would cover this later without app changes.)

## 2. Platform and tech

- **App shell:** Tauri 2, building native installers for Windows and Linux.
- **Back end:** Rust. All network work (SSH, Docker, Proxmox, scanning) happens here.
- **Screens:** TypeScript with Svelte,
  plus xterm.js for terminals.
- **Secrets:** passwords, SSH key passphrases and API tokens are stored in the
  operating system's keychain (Windows Credential Manager, or the Linux Secret
  Service), never in plain config files. SSH private keys stay where they
  already are, in the owner's own key files or SSH agent; the app only stores
  the path of the key file.
- **Settings and inventory:** a local SQLite file in the user's app data folder.

Rust libraries:

| Job | Library |
| --- | --- |
| SSH and terminals | `russh` |
| Docker | `bollard` (to confirm in step 4) |
| Proxmox API | `reqwest` (plain HTTPS calls; to confirm in step 4) |
| Ping and scanning | `ping-async`, plus the system's neighbour (ARP) table, reverse DNS, mDNS and NetBIOS lookups |
| Keychain | `keyring` |
| Local database | `rusqlite` with SQLite |

`ping-async` replaced the first choice, `surge-ping`, because that one needs
administrator rights on Windows. Nothing in the app needs them.

## 3. Your network

- **Mesh:** TP-Link Deco BE85, 4 nodes.
- **Main PC (Windows):** the owner's daily machine. This is where home-cc runs.
- **NAS / server PC (Windows):** stores files, runs some services, and hosts two
  Linux VMs:
  - **Homelab VM (Linux):** home services, mostly as Docker compose stacks.
  - **Dev VM (Linux):** used for development.
- **Proxmox PC:** runs the Pi-holes, plus Prometheus and Grafana for monitoring.
- **Docker hosts:** at least the homelab VM. **OPEN:** any others?
- **OPEN:** what runs the two VMs on the NAS (Hyper-V, VirtualBox, VMware)?
  This decides whether the app can start and stop them, or only see them.
- **OPEN:** home IP range (for example 192.168.68.0/22, the Deco default). The
  app can also work this out by itself.

## 4. How the app reaches each machine

| What | How | What you set up once |
| --- | --- | --- |
| Deco mesh nodes | Detected on the network by maker and MAC address (see 5.1) | Nothing |
| Linux machines | SSH | An SSH key added to each machine |
| Windows PCs | SSH (Windows' built-in OpenSSH server) | Turn on OpenSSH Server in Windows settings |
| Docker | Through the same SSH connection to each host | Nothing extra |
| Proxmox | Proxmox's built-in HTTPS API | An API token created in Proxmox |
| Prometheus (optional) | Its HTTP query API | Nothing, if it already collects machine stats |

Using SSH for Docker means no Docker ports are opened on the network, which is
safer than exposing the Docker API.

## 5. Features

Each feature lists what "done" looks like.

### 5.1 Network: mesh nodes and discovery

- Scan the home IP range and list every device that answers, with IP address,
  MAC address, maker (from the MAC) and name where one can be found.
- Show mesh nodes as their own group.
- Remember devices between runs. Let the owner rename them and mark which ones
  are PCs, VMs, servers or "ignore".

Done when: a scan finds all the owner's known machines and mesh nodes, and
names survive a restart.

Deco BE85 notes: Deco has no official local API; it is managed through the
phone app and TP-Link's cloud. So the app finds the 4 nodes on the network by
their TP-Link MAC addresses, and lets the owner name each one (for example
"Living room"). Showing which device is connected to which node would need the
Deco's unofficial local admin interface. That is a possible later add-on, not
part of the first version.

### 5.2 Ping and status

- Ping every known device on a regular timer (default every 30 seconds, adjustable).
- Show up or down, response time, and when it was last seen.
- Keep a short history so the owner can see if something drops out now and then.

Done when: turning a machine off shows it as down within one timer period, and
turning it on shows it as up again.

One exception: a machine that never answers ping (many Windows PCs, whose
firewall drops it) is judged by whether the network still knows its hardware
address. That information lags by up to about 45 seconds, so such a machine
can take one more timer period to show as down.

### 5.3 Machines: PCs and VMs

- List PCs and VMs from three sources combined: the network scan, the Proxmox
  VM list, and machines the owner adds by hand.
- Show for each: operating system, uptime, CPU load, RAM used and total, and
  disk space per drive.
- Warn (colour change) when a disk is above 85% full or RAM above 90%. Both
  levels can be changed.

Done when: stats for every connected Linux and Windows machine show and update
at least once a minute.

Prometheus is already running on the Proxmox PC. If it already collects
machine stats (node_exporter on Linux, windows_exporter on Windows), the app
can read stats and history from Prometheus instead of asking each machine.
SSH stays as the fallback for machines Prometheus doesn't cover. **OPEN:** which
machines does Prometheus already watch?

### 5.4 Terminal

- Open a full terminal to any Linux or Windows machine in a tab inside the app.
- Must support colours, resizing, copy and paste, and full-screen programs
  such as vim, htop and tmux.
- Several tabs open at once, to different machines.
- Option to open a terminal straight into a running Docker container.

Done when: vim, htop and tmux all work correctly in a tab, and closing the tab
closes the connection.

### 5.5 Docker

- For each Docker host, list containers with name, image, state, ports,
  uptime, and CPU and memory use.
- Start, stop, restart and remove containers.
- View logs live, with search, and download them to a file.
- Clean up: remove stopped containers, unused images, unused volumes and build
  cache, with a preview of what will be removed and how much space it frees
  before anything is deleted.
- Dockerfiles and compose files: browse, edit and save them on the host, then
  build or bring a compose stack up or down.

Done when: the owner can follow a container's logs, restart it, and run a
cleanup on any host without opening a terminal.

The owner mostly uses compose files, so the app is built around compose
stacks: each stack shows as a group with its containers inside it, and the
main actions (up, down, pull new images and restart, edit the compose file)
work on the whole stack. Single containers still get the controls above.

### 5.6 Proxmox

- Show each node's CPU, RAM and storage.
- List VMs and containers (LXC) with their state and resource use.
- Start, shut down, reboot, and force stop.
- Open a VM's console. (A terminal over SSH where possible; the graphical
  console is a later add-on.)
- Take, list and roll back snapshots.

Done when: the owner can see every VM, start and stop it, and take a snapshot
from the app.

## 6. Screens and layout

The owner uses three monitors: 2560x1440, 3440x1440 ultrawide, and 1920x1080.
The app must work well on all three and when moved between them.

- The layout resizes smoothly with the window. Nothing is fixed to one size, and
  there's no sideways scrolling at any of these widths.
- Content reflows to use the space. Cards and tables add columns on wider
  screens: for example 2 to 3 columns of machine cards at 1080p, 3 to 4 at
  1440p, and 5 to 6 on the ultrawide.
- On the ultrawide, the app can show panels side by side, for example the
  machine list, a terminal and container logs all at once.
- Text and controls follow Windows' display scaling, so they stay a readable
  size if a monitor is scaled to 125% or 150%.
- The whole app can be zoomed from 50% to 200%: Ctrl with + / - (0 for 100%),
  Ctrl with the mouse wheel, or the buttons in Settings. The size is kept for
  next time, and every screen stays usable at 200% in the smallest window
  (960x600).
- The app remembers its window size and position, and which monitor it was on.
- Done when: every screen is checked at all three resolutions, at 100% and
  125% scaling, with no cut-off text or overlapping parts.

## 7. Safety

- Anything that deletes or stops things (remove container, cleanup, force stop
  VM, roll back snapshot) asks for confirmation first.
- Secrets live only in the OS keychain (SSH private keys stay in the owner's
  own files or SSH agent; see section 2).
- A machine reached over SSH is remembered by its identity (host key). If the
  identity later changes, nothing is sent to it until the owner has seen the
  old and new fingerprints and approved. A stored password is never sent to a
  machine whose identity the owner hasn't approved.
- The app talks only to machines on the home network that the owner has added
  or approved.

## 8. Build order

1. **This spec.**
2. **App skeleton:** Tauri app that builds on Windows and Linux, with a sidebar
   for each module (Network, Machines, Docker, Proxmox), local settings, and
   keychain storage.
3. **See everything:** network scan, ping and status (5.1, 5.2), and machine
   stats over SSH (5.3).
4. **Act on things:** terminal tabs (5.4), then Docker (5.5), then Proxmox (5.6).

## 9. Open questions

1. What runs the two Linux VMs on the NAS PC? (section 3)
2. Which machines run Docker besides the homelab VM? (section 3)
3. Which machines does Prometheus already watch? (5.3)
4. Which visual style (see the style mockups)?
