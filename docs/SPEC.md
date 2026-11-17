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
| Pi-hole | Pi-hole v6's own API (HTTPS, or HTTP if the owner chooses) | An app password (or the web password) |
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
  are PCs, VMs, servers, appliances (printers, TVs, cameras and other things
  with an address that aren't PCs) or "ignore". A new name is saved with
  Enter or by pressing elsewhere on the screen, and given up with Escape; one
  half typed when the owner goes to another screen is not saved, and is there
  to finish when they come back.

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
- Appliances are listed with the machines, with up or down, response time and
  when they were last seen. Most have no SSH, so they have no stats and the app
  doesn't ask for an SSH user; one that does have SSH (a NAS box, say) gets
  stats like any other machine.
- The PC home-cc runs on needs no SSH: its stats are read directly, with the
  same script and the same numbers an SSH reading would give, and without
  showing a window. If its system is still "Other", the app fills in Windows
  or Linux. A machine with an SSH login on another port than 22, or on this
  PC's loopback address (a VM, WSL or a container reached through this PC),
  is not this PC and is still read over SSH.
- The machine cards can be put in any order: drag a card by the grip in its
  header, or use the arrow keys on that grip. The order is remembered on this
  PC; new machines go at the end, and "Reset order" goes back to name order.
- Machines can be put in groups the owner names ("Proxmox lab", say): with the
  Group field in a machine's form (which suggests the groups there are), or
  with the group menu on its card. The cards show under a heading per group
  (name and count), groups in name order, machines in no group last under
  "Other machines". Each group can be collapsed, remembered on this PC.
  Rename changes the name on every machine in the group; a name another group
  already has (in any letter case) joins the two. Ungroup asks first, then
  takes the machines out of it. A card is moved only among its own group's
  cards; the saved order is one list for all cards, and a card that joins a
  group goes at its end. The group is kept with the machine in the back end
  and a scan never changes it. A machine set to Ignore (or made a phone, say)
  keeps its group and comes back in it; Rename and Ungroup change it too, and
  say so. A machine's open form only sends its group when the owner changed
  that field, so it never undoes a group given meanwhile from the card.

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
- View logs live, with search, and download them to a file. The search and
  how much of the log is shown stay while another screen is shown, and go
  when the log is closed (its Close, the container's Logs button again, or
  another panel or host in its place).
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

Image updates: the app checks whether the registry has a newer image for the
tag each container runs, without pulling anything. The check runs on the
Docker host over the same SSH connection, so the host's own network and
registry logins apply, and asks about each image once per host: first with
HEAD requests (which Docker Hub doesn't count against its pull limits), then
with `docker buildx imagetools inspect` or `docker manifest inspect`. It
compares the digest the running image was pulled with against the one the
tag points at now (for an image made for several kinds of machine, the
index's digest). The screens check by themselves at most every 6 hours per
host; "Check for updates" asks at once. Each container shows its answer: an
update, built on the host (nothing to compare), pinned by digest, or why the
registry couldn't answer (needs a login, unreachable, tag not found, rate
limit). A registry that doesn't answer costs its own image a short wait, and
a check stops asking after 20 minutes, so the answers it has are kept; an
image with no answer yet is said as "not checked yet", never as up to date.
Counts show on each stack card, the host tab, a line for the host
("3 updates in 2 stacks") and on the Overview. "Update" on a stack, or
"Update all" for the host, lists each service with its old and new digest
and the containers that restart, asks first (and asks again if that list
changes before the yes), then pulls and recreates only the ticked services
(`compose pull` and `up -d --no-deps`). A stopped service
starts unticked, because updating it starts it. A container outside any
stack is only offered a pull of its new image: its run options aren't known,
so the owner recreates it. Old images are left for Clean up to remove.

### 5.6 Proxmox

- Show each node's CPU, RAM and storage.
- List VMs and containers (LXC) with their state and resource use.
- Start, shut down, reboot, and force stop.
- Open a VM's console. (A terminal over SSH where possible; the graphical
  console is a later add-on.)
- Take, list and roll back snapshots.

Done when: the owner can see every VM, start and stop it, and take a snapshot
from the app.

### 5.7 Pi-hole

The owner runs two Pi-holes (containers on the Proxmox PC). The Pi-hole screen
shows them side by side.

- **Which machines:** any saved device can be a Pi-hole. The Add panel suggests
  machines whose name or hostname says Pi-hole, and, on request ("Look for
  Pi-holes on my machines"), those that answer Pi-hole's login page over plain
  HTTP on port 80 (nothing secret is sent). A machine is added only when the
  owner presses its Add button. "Remove from the Pi-hole screen" (asks first)
  logs out and forgets its settings and password; nothing on the Pi-hole changes.
- **Talking to Pi-hole:** Pi-hole v6's REST API under /api. home-cc logs in with
  POST /api/auth (an app password is best: it also works with two-factor login
  on) and sends the session ID in the X-FTL-SID header. Pi-hole allows only a
  few sessions at once (16 unless changed), so home-cc keeps one per Pi-hole,
  uses it until Pi-hole drops it (a 401 means one new login and one retry, no
  more; a request that times out or loses its connection keeps the session),
  and logs out (DELETE /api/auth) when the settings change, the device
  is removed or its address changes, and when the app closes. Readings use
  GET /api/stats/summary, /api/dns/blocking, /api/lists and /api/info/version;
  pausing uses POST /api/dns/blocking with {blocking, timer}. Pi-hole v5 (whose
  API is /admin/api.php) is recognised and said to be unsupported.
- **Connection:** HTTPS on 443 by default, with the certificate pinned like
  Proxmox's: on first contact the owner is shown its SHA-256 fingerprint and
  approves it; any other certificate stops the connection before the password
  is sent and needs approving again. Another port, scheme or address is a first
  contact again. The owner can choose plain HTTP instead, with a warning that
  the password and session then cross the network unencrypted.
- **Per Pi-hole:** blocking on, paused (with the time left, counting down) or
  off; queries and blocked today with the percentage; clients; domains on the
  blocklists; when the lists were last updated; version; lists that failed to
  download. Every failure is shown as a problem with an agent prompt, worded
  for the cause: no password, wrong password, two-factor login without an app
  password, no free API session, logins held back, certificate new or changed,
  unreachable, Pi-hole v5.
- **Pause and resume:** "Pause blocking…" offers 30 seconds, 5 minutes, 30
  minutes or "until I turn it back on", then asks for confirmation. "Pause
  both…" does the same for every Pi-hole at once; "Resume" turns blocking back
  on without asking. Pi-hole switches blocking back on by itself when a timer
  ends; the screen reads it again just after (only when its latest reading
  worked: a pause seen before a Pi-hole became unreadable doesn't make the
  screen read sooner than every 30 seconds).
- **Blocklists compared:** the lists (block and allow) of the Pi-holes are
  compared by address (a web address's scheme and host in any letter case, and
  a "/" at its end, count as the same list; both spellings are shown); lists
  only on some of them, or switched off on some, are named. Nothing is changed by home-cc; copying lists from one Pi-hole to
  another may come later. Allow and deny domain counts are not compared yet.
- **Overview:** a Pi-hole panel with one line per Pi-hole (blocking state and
  blocked today), and a "Needs attention" entry when a Pi-hole can't be read
  or its blocking is off or paused. Nothing when no machine is a Pi-hole.
- Screens read every 30 seconds while they are open and the window is in view.
  The password lives only in the keychain; problem reports never include it.

Done when: the owner sees both Pi-holes' numbers, pauses and resumes blocking
on one or both, and can tell which lists only one of them has.

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
  VM, roll back snapshot, pause Pi-hole blocking) asks for confirmation first.
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
5. **Pi-hole** (5.7).

## 9. Open questions

1. What runs the two Linux VMs on the NAS PC? (section 3)
2. Which machines run Docker besides the homelab VM? (section 3)
3. Which machines does Prometheus already watch? (5.3)
4. Which visual style (see the style mockups)?
