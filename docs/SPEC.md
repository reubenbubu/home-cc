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
- **Screens:** TypeScript with a light front-end framework (proposed: Svelte),
  plus xterm.js for terminals.
- **Secrets:** passwords, SSH keys and API tokens are stored in the operating
  system's keychain (Windows Credential Manager, or the Linux Secret Service),
  never in plain config files.
- **Settings and inventory:** a local SQLite file in the user's app data folder.

Proposed Rust libraries (to confirm during step 2):

| Job | Library |
| --- | --- |
| SSH and terminals | `russh` |
| Docker | `bollard` |
| Proxmox API | `reqwest` (plain HTTPS calls) |
| Ping and scanning | `surge-ping`, plus ARP and mDNS lookups |
| Keychain | `keyring` |
| Local database | `sqlx` with SQLite |

## 3. Your network

**OPEN:** fill in what you have, roughly. This drives what to build first.

- Mesh system (brand and model): _?_
- Router (if separate from the mesh): _?_
- Proxmox host(s): _?_
- Linux machines and VMs: _?_
- Windows PCs: _?_
- Docker hosts: _?_
- Home IP range (for example 192.168.1.0/24): _?_

## 4. How the app reaches each machine

| What | How | What you set up once |
| --- | --- | --- |
| Mesh nodes | Depends on the brand (see 5.1) | Possibly a login for the mesh admin |
| Linux machines | SSH | An SSH key added to each machine |
| Windows PCs | SSH (Windows' built-in OpenSSH server) | Turn on OpenSSH Server in Windows settings |
| Docker | Through the same SSH connection to each host | Nothing extra |
| Proxmox | Proxmox's built-in HTTPS API | An API token created in Proxmox |

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

**OPEN:** how mesh nodes can be detected depends on the brand. Some (for
example TP-Link Deco, Eero) only have a phone app and a cloud service, so the
app would spot nodes by their maker and MAC address. Others (for example
Asus AiMesh, OpenWrt-based mesh) have a local admin page or API with more
detail, such as which device is connected to which node.

### 5.2 Ping and status

- Ping every known device on a regular timer (default every 30 seconds, adjustable).
- Show up or down, response time, and when it was last seen.
- Keep a short history so the owner can see if something drops out now and then.

Done when: turning a machine off shows it as down within one timer period, and
turning it on shows it as up again.

### 5.3 Machines: PCs and VMs

- List PCs and VMs from three sources combined: the network scan, the Proxmox
  VM list, and machines the owner adds by hand.
- Show for each: operating system, uptime, CPU load, RAM used and total, and
  disk space per drive.
- Warn (colour change) when a disk is above 85% full or RAM above 90%. Both
  levels can be changed.

Done when: stats for every connected Linux and Windows machine show and update
at least once a minute.

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

**OPEN:** do you mostly run containers with `docker compose` files, single
`docker run` commands, or both? This decides how the "manage Dockerfiles" part
is shaped.

### 5.6 Proxmox

- Show each node's CPU, RAM and storage.
- List VMs and containers (LXC) with their state and resource use.
- Start, shut down, reboot, and force stop.
- Open a VM's console. (A terminal over SSH where possible; the graphical
  console is a later add-on.)
- Take, list and roll back snapshots.

Done when: the owner can see every VM, start and stop it, and take a snapshot
from the app.

## 6. Safety

- Anything that deletes or stops things (remove container, cleanup, force stop
  VM, roll back snapshot) asks for confirmation first.
- Secrets live only in the OS keychain.
- The app talks only to machines on the home network that the owner has added
  or approved.

## 7. Build order

1. **This spec.**
2. **App skeleton:** Tauri app that builds on Windows and Linux, with a sidebar
   for each module (Network, Machines, Docker, Proxmox), local settings, and
   keychain storage.
3. **See everything:** network scan, ping and status (5.1, 5.2), and machine
   stats over SSH (5.3).
4. **Act on things:** terminal tabs (5.4), then Docker (5.5), then Proxmox (5.6).

## 8. Open questions

1. What mesh system do you have? (5.1)
2. Roughly what machines are on the network? (section 3)
3. Do you use compose files, single containers, or both? (5.5)
4. Svelte for the screens is a proposal. Any preference, such as React?
