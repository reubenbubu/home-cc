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
use std::collections::{BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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

/// One device to check: its inventory id, its IP address or host name, and
/// the hardware address the inventory holds for it, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub device_id: i64,
    pub host: String,
    /// As the inventory stores it: "8c:90:2d:a0:0e:02".
    pub mac: Option<String>,
}

/// Where a device added by name is right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Located {
    At(Ipv4Addr),
    /// It is this PC itself.
    Own,
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
    let on = |l: &ifaces::Link| format!("{} (number {}) on {}", l.adapter.name, l.adapter.index, l.net);
    let adapters = match plan.links.as_slice() {
        [] => "no adapter on this range: it is reached through a router, so no hardware addresses".into(),
        links => format!("adapters on this range: {}", links.iter().map(on).collect::<Vec<_>>().join(", ")),
    };
    vec![
        adapters,
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
/// A host name that doesn't resolve counts as down; one whose lookup is slow
/// is checked at the address it had in the last few minutes.
///
/// It fails as a whole in one case only: this PC is not allowed to send
/// pings at all (some Linux setups). Nothing is checked then, and the error
/// is a message for the owner that says what to do about it. Store no
/// results for such a round: they would all read "down". No targets is not
/// an error, whatever the PC allows: the answer is an empty list.
///
/// A round takes a moment when everything answers, about 4 seconds when
/// something is down, and 12 at the most (a few more only if the system
/// loses track of a ping, which is then given up on). It remembers (until
/// the app closes) which devices have answered ping: for those a missing
/// reply means down at once, without asking the neighbour table. This PC's
/// own addresses are never pinged and always up; `own_devices` lists them.
///
/// Known limit: a device that never answers ping (a Windows PC whose
/// firewall drops it, say) is judged from the neighbour table alone, and
/// the system can go on calling it reachable there for most of a minute
/// after it is switched off. Such a device can still be reported as up in
/// the first round after it went off, and is down from the round after
/// that: within two timer periods rather than one. Devices that answer ping
/// are down in the first round.
///
/// Two targets at one address with different hardware addresses (the
/// address was handed on to another device): if the neighbour table holds
/// the hardware address of one of them, the others with a different one are
/// down. Where the table has no entry for the address, or one that matches
/// none of them, all are judged by what answers there, as are targets
/// without a hardware address.
///
/// Must run inside the tokio runtime.
pub async fn check(targets: Vec<Target>) -> Result<Vec<Check>> {
    check_on(Arc::new(lan::OsLan), targets, own_ips(), status::os_lookup, &status::REAL, &status::KEPT).await
}

/// The devices whose host turned out to be this PC itself (one of its own
/// addresses, or a name for one) in the most recent round of `check`,
/// replaced by each round. Empty before the first round, and left as it was
/// by a round that failed.
pub fn own_devices() -> BTreeSet<i64> {
    status::KEPT.own_devices()
}

/// Where the devices that were added by name are right now, so a scan can
/// tell that an address it found belongs to one of them. Only targets whose
/// host is a name are looked at, not ones with an IP address. A name that
/// doesn't resolve is left out; a name for this PC itself, or one that leads
/// to one of its own addresses, gives `Own`.
///
/// The names are looked up as `check` does it and share its memory: all at
/// once, for 3 seconds at the most, and a name that is slow to answer is
/// taken to be where it was in the last few minutes. Must run inside the
/// tokio runtime.
pub async fn locate(targets: Vec<Target>) -> Vec<(i64, Located)> {
    locate_on(targets, own_ips(), status::os_lookup, status::NAME_WAIT, &status::KEPT.names).await
}

/// Every IPv4 address this PC has right now.
async fn own_ips() -> HashSet<Ipv4Addr> {
    // Listing the adapters can block for a moment on Windows.
    let interfaces = tokio::task::spawn_blocking(netdev::get_interfaces).await.unwrap_or_default();
    ifaces::own_ips(&interfaces)
}

/// `check` on any network, with any name lookup and memory. `own` is only
/// waited for when there is something to check and pings are allowed.
async fn check_on<L, F, Fut>(
    lan: Arc<L>,
    targets: Vec<Target>,
    own: impl Future<Output = HashSet<Ipv4Addr>>,
    lookup: F,
    pace: &status::Pace,
    kept: &status::Kept,
) -> Result<Vec<Check>>
where
    L: lan::Lan,
    F: Fn(String) -> Fut,
    Fut: Future<Output = Option<Ipv4Addr>> + Send + 'static,
{
    if targets.is_empty() {
        kept.note_own(&[]);
        return Ok(Vec::new());
    }
    let started = tokio::time::Instant::now();
    lan.allowed().map_err(|e| Error::Network(ping::not_allowed_message(&e)))?;
    let own = own.await;
    let probes = status::resolve(&targets, &own, lookup, status::NAME_WAIT, &kept.names).await;
    kept.note_own(&probes);
    Ok(status::check_round(lan, &probes, pace, &kept.answers_ping, started).await)
}

/// `locate` with any name lookup and memory.
async fn locate_on<F, Fut>(
    targets: Vec<Target>,
    own: impl Future<Output = HashSet<Ipv4Addr>>,
    lookup: F,
    wait: Duration,
    names: &Arc<Mutex<status::NameMemory>>,
) -> Vec<(i64, Located)>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Option<Ipv4Addr>> + Send + 'static,
{
    let is_address = |t: &Target| t.host.trim().parse::<IpAddr>().is_ok();
    let named: Vec<Target> = targets.into_iter().filter(|t| !is_address(t)).collect();
    if named.is_empty() {
        return Vec::new();
    }
    let own = own.await;
    let probes = status::resolve(&named, &own, lookup, wait, names).await;
    let located = |probe: &status::Probe| match probe.place {
        status::Place::At(ip) => Some((probe.device_id, Located::At(ip))),
        status::Place::Own => Some((probe.device_id, Located::Own)),
        status::Place::Nowhere => None,
    };
    probes.iter().filter_map(located).collect()
}

#[cfg(test)]
mod tests {
    use super::lan::fake::{Device, FakeLan, ip};
    use super::neigh::NeighState::Reachable;
    use super::*;

    fn is_send<T: Send>(_: &T) {}

    /// The commands run these on a pool of threads, so they must be `Send`.
    /// Nothing is run here: the lookups of makers and names are not in yet.
    #[test]
    fn the_public_futures_can_move_between_threads() {
        is_send(&scan("", |_| {}, Arc::new(AtomicBool::new(false))));
        is_send(&scan_raw("", &|_| {}, &AtomicBool::new(false)));
        is_send(&check(Vec::new()));
        is_send(&locate(Vec::new()));
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

    /// The real steps, speeded up.
    const FAST: status::Pace = status::Pace {
        in_flight: 64,
        ping_timeout: Duration::from_millis(50),
        ping_window: Duration::from_millis(225),
        settle: Duration::from_millis(210),
        longest: Duration::from_millis(300),
    };

    const THIS_PC: &str = "192.168.68.78";

    fn target(device_id: i64, host: &str, mac: Option<&str>) -> Target {
        Target { device_id, host: host.into(), mac: mac.map(Into::into) }
    }

    fn kept() -> status::Kept {
        status::Kept::new(Duration::from_secs(300))
    }

    async fn own() -> HashSet<Ipv4Addr> {
        [ip(THIS_PC)].into()
    }

    /// A name service that knows a NAS, two names for this PC, and one name
    /// it never finishes with.
    async fn lookup(name: String) -> Option<Ipv4Addr> {
        match name.as_str() {
            "nas.lan" => Some(ip("192.168.68.60")),
            "this-pc" => Some(ip(THIS_PC)),
            "localhost" => Some(Ipv4Addr::LOCALHOST),
            "slow.lan" => std::future::pending().await,
            _ => None,
        }
    }

    async fn round(lan: &Arc<FakeLan>, targets: Vec<Target>, kept: &status::Kept) -> Result<Vec<Check>> {
        check_on(lan.clone(), targets, own(), lookup, &FAST, kept).await
    }

    fn up(device_id: i64, rtt_ms: Option<f64>) -> Check {
        Check { device_id, up: true, rtt_ms }
    }

    fn down(device_id: i64) -> Check {
        Check { device_id, up: false, rtt_ms: None }
    }

    #[tokio::test]
    async fn a_pc_that_may_not_ping_gets_an_error_and_no_round() {
        let (lan, kept) = (FakeLan::broken(), kept());
        let targets = vec![target(1, "192.168.68.60", None), target(2, THIS_PC, None)];
        let message = match round(&lan, targets, &kept).await {
            Err(Error::Network(message)) => message,
            other => panic!("{other:?}"),
        };
        // What is wrong and what to do, with the fix that lasts; nothing about scanning.
        assert!(message.contains("does not let apps send pings"), "{message}");
        assert!(message.contains("/etc/sysctl.d/"), "{message}");
        assert!(!message.to_lowercase().contains("scan"), "{message}");
        // Nothing was sent, looked up or remembered.
        assert!(lan.pinged.lock().unwrap().is_empty());
        assert_eq!(lan.table_reads.load(Ordering::SeqCst), 0);
        assert!(kept.own_devices().is_empty() && kept.answers_ping.lock().unwrap().is_empty());

        // Nothing to check is fine on any PC.
        assert_eq!(round(&lan, Vec::new(), &kept).await.ok(), Some(Vec::new()));
    }

    #[tokio::test]
    async fn a_round_on_a_pc_that_may_ping_gives_one_result_per_target() {
        let lan = FakeLan::with(vec![("192.168.68.60", Device::new(1).answers(2).state(Reachable))]);
        let targets = vec![
            target(1, "nas.lan", None),
            target(2, "192.168.68.60", None),
            target(3, "192.168.68.61", None),
            target(4, "gone.lan", None),
        ];
        let checks = round(&lan, targets, &kept()).await.unwrap();
        assert_eq!(checks, [up(1, Some(2.0)), up(2, Some(2.0)), down(3), down(4)]);
    }

    #[tokio::test]
    async fn own_devices_are_the_ones_of_the_latest_round() {
        let lan = FakeLan::with(vec![("192.168.68.60", Device::new(1).answers(2).state(Reachable))]);
        let kept = kept();
        assert!(kept.own_devices().is_empty());

        // This PC by address, by name, and by the name every computer has for itself.
        let targets = vec![
            target(1, THIS_PC, None),
            target(2, "nas.lan", None),
            target(3, "this-pc", None),
            target(4, "localhost", None),
            target(5, "127.0.0.1", None),
            target(6, "gone.lan", None),
        ];
        let checks = round(&lan, targets, &kept).await.unwrap();
        let expected = [up(1, None), up(2, Some(2.0)), up(3, None), up(4, None), up(5, None), down(6)];
        assert_eq!(checks, expected);
        assert_eq!(kept.own_devices(), BTreeSet::from([1, 3, 4, 5]));
        // This PC is never pinged.
        assert_eq!(*lan.pinged.lock().unwrap(), [ip("192.168.68.60")]);

        // The next round replaces the list, it does not add to it.
        round(&lan, vec![target(2, "nas.lan", None), target(3, "this-pc", None)], &kept).await.unwrap();
        assert_eq!(kept.own_devices(), BTreeSet::from([3]));
        // A round that can't run leaves it as it was...
        assert!(round(&FakeLan::broken(), vec![target(2, "nas.lan", None)], &kept).await.is_err());
        assert_eq!(kept.own_devices(), BTreeSet::from([3]));
        // ...and one with nothing to check empties it.
        round(&lan, Vec::new(), &kept).await.unwrap();
        assert!(kept.own_devices().is_empty());
    }

    #[tokio::test]
    async fn an_address_handed_on_shows_only_the_device_that_has_it_now() {
        // The table holds the hardware address `Device::new(2)` has.
        let lan = FakeLan::with(vec![("192.168.68.60", Device::new(2).answers(3).state(Reachable))]);
        let kept = kept();
        // A phone that left, the laptop that has its address now (once by
        // name, written the way Windows shows it), and an entry without one.
        let targets = vec![
            target(1, "192.168.68.60", Some("02:00:00:00:00:01")),
            target(2, "nas.lan", Some("02-00-00-00-00-02")),
            target(3, "192.168.68.60", None),
        ];
        let checks = round(&lan, targets, &kept).await.unwrap();
        assert_eq!(checks, [down(1), up(2, Some(3.0)), up(3, Some(3.0))]);
        // The phone is not remembered as a device that answers ping.
        assert_eq!(*kept.answers_ping.lock().unwrap(), BTreeSet::from([2, 3]));
    }

    #[tokio::test]
    async fn locate_finds_where_named_devices_are() {
        let names = kept().names;
        let targets = vec![
            target(1, "nas.lan", None),
            target(2, "gone.lan", None),
            // Addresses are not names: nothing to find out.
            target(3, "192.168.68.20", None),
            target(4, "this-pc", None),
            target(5, " localhost ", None),
            target(6, THIS_PC, None),
            target(7, "fe80::1", None),
            // A mistyped address is not asked about either.
            target(8, "192.168.68", None),
            target(9, "slow.lan", None),
            // Whether the inventory has a hardware address makes no difference here.
            target(10, "nas.lan", Some("02:00:00:00:00:01")),
        ];
        let wait = Duration::from_millis(100);
        let started = std::time::Instant::now();
        let located = locate_on(targets, own(), lookup, wait, &names).await;
        let nas = Located::At(ip("192.168.68.60"));
        assert_eq!(located, [(1, nas), (4, Located::Own), (5, Located::Own), (10, nas)]);
        // The name that never answers was not waited for beyond the limit.
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn locate_shares_what_the_status_rounds_remember() {
        let lan = FakeLan::with(vec![("192.168.68.60", Device::new(1).answers(2))]);
        let kept = kept();
        round(&lan, vec![target(1, "nas.lan", None)], &kept).await.unwrap();
        // The name service has gone quiet; the address of a moment ago is used.
        let quiet = |_name: String| std::future::pending::<Option<Ipv4Addr>>();
        let located =
            locate_on(vec![target(1, "nas.lan", None)], own(), quiet, Duration::from_millis(50), &kept.names)
                .await;
        assert_eq!(located, [(1, Located::At(ip("192.168.68.60")))]);
    }

    #[tokio::test]
    async fn locate_asks_nothing_when_no_device_has_a_name() {
        let asked = AtomicBool::new(false);
        let own = async {
            asked.store(true, Ordering::SeqCst);
            HashSet::new()
        };
        let lookup = |_name: String| {
            asked.store(true, Ordering::SeqCst);
            async { None }
        };
        let targets = vec![target(1, "192.168.68.20", None), target(2, "::1", None)];
        assert!(locate_on(targets, own, lookup, Duration::from_millis(50), &kept().names).await.is_empty());
        // Neither this PC's adapters nor the name service.
        assert!(!asked.load(Ordering::SeqCst));
    }

    /// The real thing, as far as it goes without sending anything onto the
    /// network. One test, because the two halves share the app's memory.
    #[tokio::test]
    async fn the_real_check_and_locate_know_this_pc() {
        // Nothing to check is never an error.
        assert_eq!(check(Vec::new()).await.ok(), Some(Vec::new()));
        assert!(own_devices().is_empty());

        match check(vec![target(7, "127.0.0.1", None), target(8, "localhost", None)]).await {
            // This PC is up without a ping, and known as this PC.
            Ok(checks) => {
                assert_eq!(checks, [up(7, None), up(8, None)]);
                assert_eq!(own_devices(), BTreeSet::from([7, 8]));
            }
            // A PC (or build container) that forbids pings says so instead.
            Err(Error::Network(message)) => {
                assert!(message.contains("/etc/sysctl.d/"), "{message}");
                assert!(own_devices().is_empty());
            }
            Err(other) => panic!("{other:?}"),
        }
        assert_eq!(check(Vec::new()).await.ok(), Some(Vec::new()));
        assert!(own_devices().is_empty());

        let located = locate(vec![target(1, "localhost", None), target(2, "127.0.0.1", None)]).await;
        assert_eq!(located, [(1, Located::Own)]);
    }
}
