# Backlog

Work the owner asked for, to be done later. The owner's words are quoted; the notes under each are how the
work is understood so far and are open to change. Remove an item once it has landed.

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
