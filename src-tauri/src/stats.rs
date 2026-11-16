//! Machine stats read over SSH: operating system, uptime, CPU, RAM and disks.

use crate::inventory::Os;
use crate::ssh::{HostKey, Login, SshError};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Disk {
    /// Where it is mounted: "/" or "/home" on Linux, "C:" on Windows.
    pub mount: String,
    /// Device name on Linux, volume label on Windows. May be empty.
    pub label: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineStats {
    /// For example "Ubuntu 24.04.1 LTS" or "Microsoft Windows 11 Pro".
    pub os_name: String,
    pub uptime_secs: Option<u64>,
    /// CPU in use, 0 to 100. On Linux this needs two readings, so the first
    /// one has none.
    pub cpu_pct: Option<f64>,
    /// One-minute load average (Linux only).
    pub load1: Option<f64>,
    pub cpu_count: Option<u32>,
    pub mem_total_kb: Option<u64>,
    pub mem_used_kb: Option<u64>,
    /// Real, local disks only.
    pub disks: Vec<Disk>,
    /// True when a Windows account wasn't allowed the full query and some
    /// values may be missing.
    pub reduced: bool,
}

/// CPU counters from one reading, kept so the next one can work out CPU use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTicks {
    pub total: u64,
    pub idle: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Collected {
    pub stats: MachineStats,
    /// The identity the server presented, to store if it is new.
    pub host_key: HostKey,
    /// Pass back in as `prev` next time.
    pub cpu_ticks: Option<CpuTicks>,
}

/// Log in to one machine, read its stats and log out. `os` picks the Linux
/// (also Proxmox) or Windows command; other systems are tried as Linux.
/// `prev` is the `cpu_ticks` from the previous reading of the same machine.
/// Bounded by timeouts: never takes more than about 30 seconds.
pub async fn collect(login: &Login, os: Os, prev: Option<CpuTicks>) -> Result<Collected, SshError> {
    let _ = (login, os, prev);
    todo!()
}
