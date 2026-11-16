//! Everything that touches the local network: finding the home IP range,
//! scanning it, working out what each device is, and checking devices are up.
//! None of it needs administrator rights.

pub mod names;
pub mod oui;

use crate::error::Result;
use crate::history::Check;
use crate::inventory::Found;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// A local network the app could scan.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Subnet {
    /// For example "192.168.68.0/22".
    pub cidr: String,
    /// Name of the network adapter, as the operating system shows it.
    pub interface: String,
    /// This PC's address on that network.
    pub own_ip: String,
    /// True for the one the app would pick by itself.
    pub best: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScanPhase {
    /// Pinging every address in the range.
    Sweep,
    /// Second look at devices that were slow or gave no ping reply.
    Confirm,
    /// Looking up device names.
    Names,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub phase: ScanPhase,
    /// Addresses tried so far, and how many there are in all.
    pub done: usize,
    pub total: usize,
    /// Devices found so far.
    pub found: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScanOutcome {
    /// The range that was scanned, for example "192.168.68.0/22".
    pub range: String,
    pub found: Vec<Found>,
}

/// One device to check: its inventory id and its IP address or host name.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub device_id: i64,
    pub host: String,
}

/// The local networks worth scanning, best first. Blocks briefly; call it
/// through `spawn_blocking`.
pub fn subnets() -> Vec<Subnet> {
    todo!()
}

/// Scan `range` ("192.168.68.0/22"), or the best local network when it is
/// empty. Lists every device that is there, including ones that don't answer
/// ping, with hardware address, maker and name where they can be found.
/// `progress` is called often. Setting `cancel` stops the scan early; what was
/// found so far is still returned. Must run inside the tokio runtime.
pub async fn scan(
    range: &str,
    progress: impl Fn(ScanProgress) + Send + Sync + 'static,
    cancel: Arc<AtomicBool>,
) -> Result<ScanOutcome> {
    let _ = (range, &progress, cancel);
    todo!()
}

/// Check each device once. One result per target, in any order. A device is
/// up if it answers ping (with a retry, so Wi-Fi devices don't flap) or, for
/// devices that never answer ping, if the neighbour table shows it is there.
/// A host name that doesn't resolve counts as down. Never fails as a whole.
pub async fn check(targets: Vec<Target>) -> Vec<Check> {
    let _ = targets;
    todo!()
}
