//! Everything that touches the local network: finding the home IP range,
//! scanning it, working out what each device is, and checking devices are up.
//! None of it needs administrator rights.

mod ifaces;
mod lan;
pub mod names;
mod neigh;
pub mod oui;
mod ping;
mod scan;
mod status;
#[cfg(windows)]
mod winping;

use crate::error::{Error, Result};
use crate::history::Check;
use crate::inventory::Found;
use serde::Serialize;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

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

/// How far a scan has got. What `done` and `total` count depends on the
/// phase. Sweep: addresses pinged. Confirm: quiet devices decided; the phase
/// is left out when there are none, and it can stand still for 8 seconds
/// while the system makes up its mind about the last ones. Names: devices,
/// none and then all.
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

/// One device as a scan finds it, before its maker and name are looked up.
#[derive(Debug, Clone, PartialEq)]
pub struct RawFound {
    pub ip: Ipv4Addr,
    /// Lower-case with colons: "8c:90:2d:a0:0e:02".
    pub mac: Option<String>,
    pub rtt_ms: Option<f64>,
    pub is_self: bool,
}

/// What `scan_raw` returns.
#[derive(Debug, Clone, PartialEq)]
pub struct RawScan {
    /// The range that was scanned, for example "192.168.68.0/22".
    pub range: String,
    /// In address order.
    pub found: Vec<RawFound>,
    /// Counts from inside the scan, for troubleshooting only.
    pub notes: Vec<String>,
}

/// The local networks worth scanning, best first. Blocks briefly; call it
/// through `spawn_blocking`.
///
/// `cidr` is what a scan of that network covers. On a network bigger than a
/// /22 that is the /22 around this PC's own address, not the whole network.
pub fn subnets() -> Vec<Subnet> {
    let ranked = ifaces::rank(&netdev::get_interfaces());
    let subnet = |(place, c): (usize, &ifaces::Candidate)| Subnet {
        cidr: c.scan_net.to_string(),
        interface: c.label.clone(),
        own_ip: c.own_ip.to_string(),
        best: place == 0,
    };
    ranked.iter().enumerate().map(subnet).collect()
}

const NO_NETWORK: &str = "No home network was found. Check that this PC is connected to your network by cable \
                          or Wi-Fi, or type the range to scan in Settings.";
const NOTHING_SENT: &str = "The scan could not send any pings. Check that this PC is still connected to your \
                            network and try again.";

/// Milliseconds, to a hundredth.
fn millis(time: Duration) -> f64 {
    (time.as_secs_f64() * 100_000.0).round() / 100.0
}

/// The first half of `scan`: finds the devices, without makers and names.
/// Same rules for `range`, `progress` and `cancel`; reports the sweep and
/// confirm phases. The diagnostic program in examples/ shows its notes.
pub async fn scan_raw(
    range: &str,
    progress: &(dyn Fn(ScanProgress) + Send + Sync),
    cancel: &AtomicBool,
) -> Result<RawScan> {
    let range = ifaces::parse_range(range).map_err(Error::Network)?;
    // Listing the adapters can block for a moment on Windows.
    let interfaces = tokio::task::spawn_blocking(netdev::get_interfaces).await.unwrap_or_default();
    let plan = ifaces::plan(&interfaces, range).ok_or_else(|| Error::Network(NO_NETWORK.into()))?;
    ping::allowed().map_err(|e| Error::Network(ping::not_allowed_message(&e)))?;

    let swept = scan::find_devices(Arc::new(lan::OsLan), &plan, &scan::REAL, progress, cancel).await;
    let counters = &swept.counters;
    if counters.pinged > 0 && counters.ping_errors == counters.pinged && swept.hits.iter().all(|h| h.is_self)
    {
        return Err(Error::Network(NOTHING_SENT.into()));
    }
    let found = swept.hits.iter().map(|h| RawFound {
        ip: h.ip,
        mac: h.mac.as_ref().map(neigh::mac_text),
        rtt_ms: h.rtt.map(millis),
        is_self: h.is_self,
    });
    Ok(RawScan { range: plan.range.to_string(), found: found.collect(), notes: notes(&plan, counters) })
}

fn notes(plan: &ifaces::Plan, c: &scan::Counters) -> Vec<String> {
    let adapter = match &plan.adapter {
        Some(a) => format!("adapter {} (number {})", a.name, a.index),
        None => "no adapter on this range: it is reached through a router, so no hardware addresses".into(),
    };
    vec![
        adapter,
        format!(
            "{} addresses pinged, {} answered in the sweep, {} pings failed to send",
            c.pinged, c.answered_in_sweep, c.ping_errors
        ),
        format!("first failure: {}", c.first_error.as_deref().unwrap_or("none")),
        format!(
            "neighbour table copied {} times during the sweep; copied but gone at its end: {:?}",
            c.table_copies, c.copied_but_gone
        ),
        format!("second chance: {} tried, {} answered", c.second_chance_tried, c.second_chance_answered),
        format!(
            "waited for the neighbour table to settle: {}; cancelled: {}",
            c.waited_to_settle, c.cancelled
        ),
    ]
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
    let raw = scan_raw(range, &progress, &cancel).await?;
    let ips: Vec<Ipv4Addr> = raw.found.iter().map(|f| f.ip).collect();
    let names_step =
        |done| progress(ScanProgress { phase: ScanPhase::Names, done, total: ips.len(), found: ips.len() });
    names_step(0);
    // A cancelled scan returns straight away, without names.
    let names = if cancel.load(Ordering::Relaxed) { HashMap::new() } else { names::discover(&ips).await };
    names_step(ips.len());
    let found = raw.found.into_iter().map(|f| decorate(f, &names)).collect();
    Ok(ScanOutcome { range: raw.range, found })
}

fn decorate(raw: RawFound, names: &HashMap<Ipv4Addr, String>) -> Found {
    let vendor = raw.mac.as_deref().and_then(oui::vendor);
    Found {
        ip: raw.ip.to_string(),
        looks_like_mesh: vendor.as_deref().is_some_and(oui::looks_like_mesh),
        vendor,
        mac: raw.mac,
        hostname: names.get(&raw.ip).cloned(),
        rtt_ms: raw.rtt_ms,
        is_self: raw.is_self,
    }
}

/// Check each device once. One result per target, in any order. A device is
/// up if it answers ping (with a retry, so Wi-Fi devices don't flap) or, for
/// devices that never answer ping, if the neighbour table shows it is there.
/// A host name that doesn't resolve counts as down. Never fails as a whole.
///
/// A round takes a moment when everything answers, about 4 seconds when
/// something is down, and 12 at the very most. It remembers (until the app
/// closes) which devices have answered ping: for those a missing reply means
/// down at once, without asking the neighbour table. This PC's own addresses
/// are never pinged and always up. Must run inside the tokio runtime.
pub async fn check(targets: Vec<Target>) -> Vec<Check> {
    if targets.is_empty() {
        return Vec::new();
    }
    let started = tokio::time::Instant::now();
    let interfaces = tokio::task::spawn_blocking(netdev::get_interfaces).await.unwrap_or_default();
    let own = ifaces::own_ips(&interfaces);
    let probes = status::resolve(&targets, &own, status::os_lookup, status::NAME_WAIT).await;
    status::check_round(Arc::new(lan::OsLan), &probes, &status::REAL, &status::ANSWERS_PING, started).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_send<T: Send>(_: &T) {}

    /// The commands run these on a pool of threads, so they must be `Send`.
    /// Nothing is run here: the lookups of makers and names are not in yet.
    #[test]
    fn the_public_futures_can_move_between_threads() {
        is_send(&scan("", |_| {}, Arc::new(AtomicBool::new(false))));
        is_send(&scan_raw("", &|_| {}, &AtomicBool::new(false)));
        is_send(&check(Vec::new()));
    }

    #[test]
    fn times_are_in_milliseconds_to_a_hundredth() {
        assert_eq!(millis(Duration::from_micros(1234)), 1.23);
        assert_eq!(millis(Duration::from_micros(455)), 0.46);
        assert_eq!(millis(Duration::from_millis(532)), 532.0);
        assert_eq!(millis(Duration::ZERO), 0.0);
    }

    #[test]
    fn subnets_marks_only_the_first_as_best() {
        // What is listed depends on the machine; the marking must not.
        let listed = subnets();
        assert!(listed.iter().skip(1).all(|s| !s.best));
        assert!(listed.first().is_none_or(|s| s.best && s.cidr.contains('/')));
    }

    #[tokio::test]
    async fn bad_ranges_are_refused_in_plain_words() {
        let scan_of = |range: &'static str| async move {
            match scan_raw(range, &|_| {}, &AtomicBool::new(false)).await {
                Err(Error::Network(message)) => message,
                other => panic!("{range}: {other:?}"),
            }
        };
        assert!(scan_of("192.168.68.0").await.contains("is not a network range"));
        assert!(scan_of("10.0.0.0/8").await.contains("too big"));
        assert!(scan_of("8.8.8.0/24").await.contains("Only home network ranges"));
    }

    #[tokio::test]
    async fn an_empty_round_of_checks_is_empty() {
        assert!(check(Vec::new()).await.is_empty());
    }
}
