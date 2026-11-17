# Backlog

Work the owner asked for, to be done later. The owner's words are quoted; the notes under each are how the
work is understood so far and are open to change. Remove an item once it has landed.

## Tell Linux from Windows by itself

> for later : auto detect linux and windows and set System value for machines

- Fill in a machine's System (Linux, Windows, Proxmox) without the owner choosing it: for example from the
  SSH login (`uname`, or the Windows version), from what answers on the network (open ports such as 22,
  135/445 or 3389, the ping TTL, the hostname), or from Proxmox's own API.
- Only fill it in while the owner hasn't set it by hand, and say on the machine where the value came from.
  A wrong guess must be easy to correct and must not come back.

## WSL and Hyper-V on Windows machines

> for later : wsl and hyper-v management for any windows machine

- For every Windows machine (this PC included, read locally without SSH), list its WSL distributions and its
  Hyper-V virtual machines with their state, and start, stop and restart them; for Hyper-V also
  checkpoints (take, list, apply), much as the Proxmox screen does for its VMs.
- Reach other Windows machines the way home-cc already does, over SSH (OpenSSH on Windows), running
  `wsl.exe` (`--list --verbose`, `--terminate`, `--shutdown`) and PowerShell's Hyper-V module (`Get-VM`,
  `Start-VM`, `Stop-VM`, `Checkpoint-VM`); Hyper-V needs an administrator login, so say clearly when the
  login lacks the rights or Hyper-V isn't installed.
- A terminal into a WSL distribution (`wsl -d <name>`) from the Terminals screen, and a console for a
  Hyper-V VM where one can be had.
- Shutting down, stopping or applying a checkpoint asks first.

## A Handheld device type

> later : add a device type Handheld

- A new type, "Handheld", for phones, tablets and handheld consoles (in the sample data, Sam's phone and the
  Tablet are "Other" today), chosen like the other types on the Network and Machines screens.
- On the Overview, handhelds are small tiles like appliances and other devices, under their own heading on
  the tiles' line. Like appliances, they need no SSH user and show no stats.
- Each place that lists or counts the types learns the new one: the Rust `DeviceKind` and the database
  (existing devices keep their type), `KIND_LABELS`, the Network screen's type choice, the Overview's
  counts, problem reports and the sample data. The network scan may suggest it from the maker (Apple,
  Samsung and so on) but never sets it without the owner.
