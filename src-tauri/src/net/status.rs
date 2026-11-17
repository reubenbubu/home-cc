//! The status round: is each known device there right now?
//!
//! A device is up when it answers ping. Some never do (a Windows PC with its
//! firewall on, say); for those the neighbour table is the witness. A device
//! that has answered ping before is not given that second way, so turning it
//! off shows as down in the same round.
//!
//! Two inventory entries can hold the same address with different hardware
//! addresses: the address was handed on from one device to another. Only one
//! of them is there, and the neighbour table says which.

use super::Target;
use super::lan::{Lan, MOST_PINGS, Pinger};
use super::neigh::{NeighState, Neighbour, is_device_mac};
use super::ping::PingOutcome;
use crate::history::Check;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::Instant;

pub struct Pace {
    /// Pings in flight at once; the same limit as the scan, for the same reason.
    pub in_flight: usize,
    /// Longer than the scan's: a Wi-Fi device that is saving power can take
    /// over a second, and a wrong "down" is worse here than a slow round.
    pub ping_timeout: Duration,
    /// No ping starts later than this into the round. With hundreds of dead
    /// devices the retries are what gets cut, then the first pings.
    pub ping_window: Duration,
    /// How long after the first ping the system has made up its mind about a
    /// hardware address it only remembered: 5 seconds of waiting plus 3 tries.
    pub settle: Duration,
    /// The round never waits for the neighbour table past this.
    pub longest: Duration,
}

pub const REAL: Pace = Pace {
    in_flight: MOST_PINGS,
    ping_timeout: Duration::from_secs(2),
    ping_window: Duration::from_secs(9),
    settle: Duration::from_millis(8500),
    longest: Duration::from_secs(12),
};

/// How long host names get to turn into addresses, all of them together.
pub const NAME_WAIT: Duration = Duration::from_secs(3);
/// How long a name's last address is still used when a new lookup is slow.
const NAME_KEEP: Duration = Duration::from_secs(300);

/// What earlier rounds learned about host names. One slow answer from the
/// name service should not show a device as down, and a name that takes
/// longer to fail than a round lasts should not be asked about twice at once.
#[derive(Debug)]
pub struct NameMemory {
    keep: Duration,
    /// The last address each name turned into, and when.
    known: HashMap<String, (Ipv4Addr, Instant)>,
    /// Names with a lookup still out.
    asking: HashSet<String>,
    /// Told each time a lookup ends, for whoever is waiting on one.
    answer_in: Arc<Notify>,
}

impl NameMemory {
    pub fn new(keep: Duration) -> Self {
        Self { keep, known: HashMap::new(), asking: HashSet::new(), answer_in: Arc::new(Notify::new()) }
    }

    /// A lookup has ended, in this round or long after it.
    fn answered(&mut self, name: &str, answer: Option<Ipv4Addr>) {
        self.asking.remove(name);
        match answer {
            Some(ip) => {
                self.known.insert(name.to_string(), (ip, Instant::now()));
            }
            // A clear "no such name" beats what was known before.
            None => {
                self.known.remove(name);
            }
        }
        self.answer_in.notify_waiters();
    }

    fn address(&self, name: &str) -> Option<Ipv4Addr> {
        self.known.get(name).filter(|(_, learned)| learned.elapsed() <= self.keep).map(|(ip, _)| *ip)
    }
}

/// What the rounds keep from one to the next.
#[derive(Debug)]
pub struct Kept {
    pub names: Arc<Mutex<NameMemory>>,
    /// Devices that have answered a ping, by inventory id.
    pub answers_ping: Mutex<BTreeSet<i64>>,
    /// Devices whose host was this PC itself in the latest round.
    own_devices: Mutex<BTreeSet<i64>>,
}

impl Kept {
    /// `name_keep` is how long a name's last address is still used.
    pub fn new(name_keep: Duration) -> Self {
        Self {
            names: Arc::new(Mutex::new(NameMemory::new(name_keep))),
            answers_ping: Mutex::new(BTreeSet::new()),
            own_devices: Mutex::new(BTreeSet::new()),
        }
    }

    /// A round has worked out where its targets are: the ones that are this
    /// PC take the place of the ones from the round before.
    pub fn note_own(&self, probes: &[Probe]) {
        let own = probes.iter().filter(|p| p.place == Place::Own).map(|p| p.device_id);
        *lock(&self.own_devices) = own.collect();
    }

    pub fn own_devices(&self) -> BTreeSet<i64> {
        lock(&self.own_devices).clone()
    }
}

/// The app's own memory, until it closes.
pub static KEPT: LazyLock<Kept> = LazyLock::new(|| Kept::new(NAME_KEEP));

/// Where a target's host turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// One of this PC's own addresses: up, and never pinged.
    Own,
    At(Ipv4Addr),
    /// A name that doesn't resolve, or an address no device can have: down.
    Nowhere,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    pub device_id: i64,
    pub place: Place,
    /// The hardware address the inventory holds for the device, if any.
    pub mac: Option<[u8; 6]>,
}

/// A hardware address as the inventory stores it ("8c:90:2d:a0:0e:02"), or
/// written with dashes, dots or nothing in between.
fn mac_bytes(text: &str) -> Option<[u8; 6]> {
    let mut digits = text.trim().chars().filter(|c| !matches!(c, ':' | '-' | '.'));
    let mut mac = [0u8; 6];
    for byte in &mut mac {
        let (high, low) = (digits.next()?.to_digit(16)?, digits.next()?.to_digit(16)?);
        *byte = u8::try_from(high * 16 + low).ok()?;
    }
    digits.next().is_none().then_some(mac)
}

fn place(ip: Ipv4Addr, own: &HashSet<Ipv4Addr>) -> Place {
    if ip.is_loopback() || own.contains(&ip) {
        Place::Own
    } else if ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast() {
        Place::Nowhere
    } else {
        Place::At(ip)
    }
}

/// True for text worth asking the system's name lookup about.
fn is_name(host: &str) -> bool {
    // All digits and dots is a mistyped address. The system would read
    // "192.168.1" as 192.168.0.1 and the wrong device would be checked.
    let mistyped_address = host.chars().all(|c| c.is_ascii_digit() || c == '.');
    !host.is_empty() && host.len() <= 253 && !mistyped_address && host.parse::<IpAddr>().is_err()
}

/// The system's own name lookup, IPv4 answers only.
pub async fn os_lookup(name: String) -> Option<Ipv4Addr> {
    let mut addrs = tokio::net::lookup_host((name.as_str(), 0)).await.ok()?;
    addrs.find_map(|addr| match addr {
        SocketAddr::V4(v4) => Some(*v4.ip()),
        SocketAddr::V6(_) => None,
    })
}

/// Turn each target's host into an address. All names are looked up at the
/// same time, so slow ones don't keep the others waiting, and the round
/// moves on after `wait`. A name not answered by then is taken to be where
/// `memory` last saw it, if that was not long ago, and otherwise counts as
/// not found. Its lookup runs on, for the next round to use.
///
/// A name whose lookup is still out from an earlier call is not asked about
/// a second time. If `memory` has an address for it, that is used at once;
/// if it has none, the lookup that is out is waited for like a new one, so
/// a name is never called not found without having had `wait` to answer.
pub async fn resolve<F, Fut>(
    targets: &[Target],
    own: &HashSet<Ipv4Addr>,
    lookup: F,
    wait: Duration,
    memory: &Arc<Mutex<NameMemory>>,
) -> Vec<Probe>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Option<Ipv4Addr>> + Send + 'static,
{
    let names: BTreeSet<&str> = targets.iter().map(|t| t.host.trim()).filter(|host| is_name(host)).collect();
    if !names.is_empty() {
        // The names this call waits for an answer about.
        let mut awaited: Vec<&str> = Vec::new();
        let answer_in = {
            let mut remembered = lock(memory);
            let keep = remembered.keep;
            remembered.known.retain(|_, (_, learned)| learned.elapsed() <= keep);
            for name in names {
                if remembered.asking.insert(name.to_string()) {
                    let (name, memory, job) = (name.to_string(), memory.clone(), lookup(name.to_string()));
                    // On its own, not tied to this call: it must end and free
                    // the name even if the caller gives up on the round.
                    tokio::spawn(async move {
                        let answer = job.await;
                        lock(&memory).answered(&name, answer);
                    });
                } else if remembered.address(name).is_some() {
                    // A lookup left over from an earlier round will answer
                    // for this name; until then there is its last address.
                    continue;
                }
                awaited.push(name);
            }
            remembered.answer_in.clone()
        };
        // The ones still out at the end are left to finish by themselves.
        let deadline = Instant::now() + wait;
        loop {
            // Made before looking, so an answer that comes in between is not missed.
            let told = answer_in.notified();
            let still_out = {
                let remembered = lock(memory);
                awaited.iter().any(|name| remembered.asking.contains(*name))
            };
            if !still_out || tokio::time::timeout_at(deadline, told).await.is_err() {
                break;
            }
        }
    }
    let remembered = lock(memory);
    let locate = |host: &str| match host.parse::<Ipv4Addr>() {
        Ok(ip) => place(ip, own),
        Err(_) => remembered.address(host).map_or(Place::Nowhere, |ip| place(ip, own)),
    };
    let probe = |t: &Target| Probe {
        device_id: t.device_id,
        place: locate(t.host.trim()),
        mac: t.mac.as_deref().and_then(mac_bytes),
    };
    targets.iter().map(probe).collect()
}

/// What one round learned about one address.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Seen {
    rtt: Option<Duration>,
    /// The neighbour table shows the system heard from it just now.
    neighbour: bool,
}

/// Up or down, and the ping time, for a device at an address.
fn verdict(seen: Seen, answers_ping: bool) -> (bool, Option<f64>) {
    match seen.rtt {
        Some(rtt) => (true, Some(super::millis(rtt))),
        // A device known to answer ping that now doesn't is down. Its entry
        // in the neighbour table can outlive it by most of a minute.
        None => (seen.neighbour && !answers_ping, None),
    }
}

/// The freshest state the table has for an address, on any adapter. A fixed
/// entry says nothing about whether the device is there, so it doesn't count.
fn state_of(table: &[Neighbour], ip: Ipv4Addr) -> Option<NeighState> {
    let mut best = None;
    for entry in table.iter().filter(|n| n.ip == ip && is_device_mac(&n.mac)) {
        match entry.state {
            NeighState::Reachable => return Some(NeighState::Reachable),
            NeighState::Unconfirmed => best = Some(NeighState::Unconfirmed),
            NeighState::Permanent => {}
        }
    }
    best
}

/// The hardware addresses the table has for an address, from its freshest
/// entries only: with two adapters an old entry can sit next to a new one.
/// A fixed entry counts here, as the least fresh: it says nothing about
/// whether the device is there, but it does say which device would answer.
fn holders(table: &[Neighbour], ip: Ipv4Addr) -> BTreeSet<[u8; 6]> {
    let freshness = |state| match state {
        NeighState::Reachable => 0,
        NeighState::Unconfirmed => 1,
        NeighState::Permanent => 2,
    };
    let entries = || table.iter().filter(|n| n.ip == ip && is_device_mac(&n.mac));
    let Some(freshest) = entries().map(|n| freshness(n.state)).min() else { return BTreeSet::new() };
    entries().filter(|n| freshness(n.state) == freshest).map(|n| n.mac).collect()
}

fn lock<T>(shared: &Mutex<T>) -> MutexGuard<'_, T> {
    shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One round over resolved targets. `started` is when the round began, name
/// lookups included; the limits in `pace` count from there.
///
/// Where devices with different stored hardware addresses share an address
/// and something is there, the neighbour table decides which of them it is:
/// if it holds the hardware address of one, the others with a different one
/// are down. Devices without a stored hardware address are judged as ever,
/// and so is everyone at an address the table has no entry for, or only an
/// entry for a device that is none of them.
pub async fn check_round<L: Lan>(
    lan: Arc<L>,
    probes: &[Probe],
    pace: &Pace,
    answers_ping: &Mutex<BTreeSet<i64>>,
    started: Instant,
) -> Vec<Check> {
    let known = lock(answers_ping).clone();

    // Each address once, however many devices share it. Devices that answer
    // ping go first: they are quick when up, and they are the ones for which
    // a ping that was never sent would be reported wrongly as down.
    let mut wanted: BTreeMap<Ipv4Addr, bool> = BTreeMap::new();
    let mut may_use_table: BTreeSet<Ipv4Addr> = BTreeSet::new();
    for probe in probes {
        if let Place::At(ip) = probe.place {
            let answers = known.contains(&probe.device_id);
            *wanted.entry(ip).or_default() |= answers;
            if !answers {
                may_use_table.insert(ip);
            }
        }
    }
    let mut ips: Vec<Ipv4Addr> = wanted.keys().copied().collect();
    ips.sort_by_key(|ip| !wanted.get(ip).copied().unwrap_or_default());

    // The table as it is before our pings stir it up. An entry that is
    // fresh now can run out while the round is under way, and the system
    // then takes longer to make up its mind than the round has time for.
    let before = if may_use_table.is_empty() { Vec::new() } else { lan.neighbours().await };

    let mut seen: BTreeMap<Ipv4Addr, Seen> = BTreeMap::new();
    let mut first_ping: BTreeMap<Ipv4Addr, Instant> = BTreeMap::new();
    let mut silent: BTreeSet<Ipv4Addr> = BTreeSet::new();
    let window_ends = started + pace.ping_window;
    let mut pinger = Pinger::new(lan.clone(), pace.in_flight);

    // First ping for everyone.
    let may_start = |ip| {
        let now = Instant::now();
        if now < window_ends {
            first_ping.insert(ip, now);
        }
        now < window_ends
    };
    let finished = |ip, outcome| match outcome {
        Ok(PingOutcome::Up(rtt)) => seen.entry(ip).or_default().rtt = Some(rtt),
        _ => {
            silent.insert(ip);
        }
    };
    pinger.run(&ips, pace.ping_timeout, may_start, finished).await;

    // One more try for the ones that gave no reply, so a single lost ping
    // doesn't show a Wi-Fi device as down.
    let again: Vec<Ipv4Addr> = ips.iter().filter(|ip| silent.contains(ip)).copied().collect();
    let finished = |ip, outcome| {
        if let Ok(PingOutcome::Up(rtt)) = outcome {
            seen.entry(ip).or_default().rtt = Some(rtt);
        }
    };
    pinger.run(&again, pace.ping_timeout, |_| Instant::now() < window_ends, finished).await;

    // Still nothing: ask the neighbour table, for devices allowed that way.
    // `after` is the latest copy of the table taken after the pings.
    let mut after: Option<Vec<Neighbour>> = None;
    let quiet: Vec<Ipv4Addr> =
        ips.iter().filter(|ip| may_use_table.contains(ip) && !seen.contains_key(ip)).copied().collect();
    if !quiet.is_empty() {
        let mut table = lan.neighbours().await;
        let mut unsure = Vec::new();
        for ip in quiet {
            if state_of(&before, ip) == Some(NeighState::Reachable) {
                seen.entry(ip).or_default().neighbour = true;
                continue;
            }
            match (state_of(&table, ip), first_ping.get(&ip)) {
                (Some(NeighState::Reachable), _) => seen.entry(ip).or_default().neighbour = true,
                // Our ping made the system ask the device again; the answer
                // is in the table once the settle time has passed.
                (Some(NeighState::Unconfirmed), Some(pinged)) => unsure.push((ip, *pinged + pace.settle)),
                _ => {}
            }
        }
        if let Some(decided) = unsure.iter().map(|(_, at)| *at).max() {
            tokio::time::sleep_until(decided.min(started + pace.longest)).await;
            table = lan.neighbours().await;
            for (ip, _) in unsure {
                if state_of(&table, ip) == Some(NeighState::Reachable) {
                    seen.entry(ip).or_default().neighbour = true;
                }
            }
        }
        after = Some(table);
    }

    // An address handed on from one device to another: the inventory has
    // both there, with different hardware addresses, and something answers.
    // The one whose hardware address the table holds is the one that is there.
    let mut stored: BTreeMap<Ipv4Addr, BTreeSet<[u8; 6]>> = BTreeMap::new();
    for probe in probes {
        if let (Place::At(ip), Some(mac)) = (probe.place, probe.mac) {
            stored.entry(ip).or_default().insert(mac);
        }
    }
    stored.retain(|ip, macs| macs.len() > 1 && seen.contains_key(ip));
    let mut there: BTreeMap<Ipv4Addr, BTreeSet<[u8; 6]>> = BTreeMap::new();
    if !stored.is_empty() {
        let table = match after {
            Some(table) => table,
            None => lan.neighbours().await,
        };
        for (ip, macs) in stored {
            let holders = holders(&table, ip);
            // No entry (the address is behind a router, say) or only one for
            // a device the inventory doesn't have there: nothing to go by.
            if !holders.is_disjoint(&macs) {
                there.insert(ip, holders);
            }
        }
    }

    let mut answered_now = Vec::new();
    let mut checks = Vec::with_capacity(probes.len());
    for probe in probes {
        let (up, rtt_ms) = match probe.place {
            Place::Own => (true, None),
            Place::Nowhere => (false, None),
            Place::At(ip) => match (probe.mac, there.get(&ip)) {
                // What is at its address is another device.
                (Some(mac), Some(holders)) if !holders.contains(&mac) => (false, None),
                _ => verdict(seen.get(&ip).copied().unwrap_or_default(), known.contains(&probe.device_id)),
            },
        };
        if rtt_ms.is_some() {
            answered_now.push(probe.device_id);
        }
        checks.push(Check { device_id: probe.device_id, up, rtt_ms });
    }
    lock(answers_ping).extend(answered_now);
    checks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::lan::fake::{Device, FakeLan, ip};
    use crate::net::neigh::NeighState::{Permanent, Reachable, Unconfirmed};
    use std::pin::Pin;
    use std::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};

    /// The real steps, speeded up.
    const FAST: Pace = Pace {
        in_flight: 64,
        ping_timeout: Duration::from_millis(50),
        ping_window: Duration::from_millis(225),
        settle: Duration::from_millis(210),
        longest: Duration::from_millis(300),
    };

    /// A device the inventory has no hardware address for.
    fn probe(device_id: i64, place: Place) -> Probe {
        Probe { device_id, place, mac: None }
    }

    fn at(device_id: i64, addr: &str) -> Probe {
        probe(device_id, Place::At(ip(addr)))
    }

    /// A device stored with the hardware address `fake::Device::new(last_mac_byte)` has.
    fn stored(device_id: i64, addr: &str, last_mac_byte: u8) -> Probe {
        Probe { mac: Some([2, 0, 0, 0, 0, last_mac_byte]), ..at(device_id, addr) }
    }

    fn up(device_id: i64, rtt_ms: f64) -> Check {
        Check { device_id, up: true, rtt_ms: Some(rtt_ms) }
    }

    fn up_silent(device_id: i64) -> Check {
        Check { device_id, up: true, rtt_ms: None }
    }

    fn down(device_id: i64) -> Check {
        Check { device_id, up: false, rtt_ms: None }
    }

    async fn round(lan: &Arc<FakeLan>, probes: &[Probe], memory: &Mutex<BTreeSet<i64>>) -> Vec<Check> {
        check_round(lan.clone(), probes, &FAST, memory, Instant::now()).await
    }

    #[test]
    fn verdict_table() {
        let rtt = Some(Duration::from_micros(1234));
        // A ping reply is up, whatever else is known.
        assert_eq!(verdict(Seen { rtt, neighbour: false }, true), (true, Some(1.23)));
        assert_eq!(verdict(Seen { rtt, neighbour: true }, false), (true, Some(1.23)));
        // No reply: the neighbour table counts only for devices that never answer ping.
        assert_eq!(verdict(Seen { rtt: None, neighbour: true }, false), (true, None));
        assert_eq!(verdict(Seen { rtt: None, neighbour: true }, true), (false, None));
        assert_eq!(verdict(Seen { rtt: None, neighbour: false }, false), (false, None));
        assert_eq!(verdict(Seen::default(), true), (false, None));
    }

    #[test]
    fn freshest_neighbour_state_wins_and_fixed_entries_do_not_count() {
        let entry = |addr: &str, mac: [u8; 6], state| Neighbour { ip: ip(addr), mac, state };
        let mac = [2, 0, 0, 0, 0, 1];
        let table = [
            entry("10.0.0.1", mac, Unconfirmed),
            entry("10.0.0.1", mac, Reachable),
            entry("10.0.0.2", mac, Unconfirmed),
            entry("10.0.0.3", mac, Permanent),
            entry("10.0.0.4", [0xff; 6], Reachable),
        ];
        assert_eq!(state_of(&table, ip("10.0.0.1")), Some(Reachable));
        assert_eq!(state_of(&table, ip("10.0.0.2")), Some(Unconfirmed));
        assert_eq!(state_of(&table, ip("10.0.0.3")), None);
        assert_eq!(state_of(&table, ip("10.0.0.4")), None);
        assert_eq!(state_of(&table, ip("10.0.0.5")), None);
    }

    #[tokio::test]
    async fn one_result_per_target_from_ping_retry_and_neighbour_table() {
        let lan = FakeLan::with(vec![
            ("10.0.0.1", Device::new(1).answers(2).state(Reachable)),
            // Misses the first ping, answers the retry.
            ("10.0.0.2", Device::new(2).replies(&[None, Some(7)]).state(Reachable)),
            // Never answers ping but the system hears from it.
            ("10.0.0.3", Device::new(3).state(Reachable)),
            // Switched off: nothing anywhere.
            ("10.0.0.4", Device::new(4)),
            // A fixed entry is no sign of life.
            ("10.0.0.5", Device::new(5).state(Permanent)),
            // Heard from just before the round; the entry runs out during it
            // and the system is still asking when the round ends.
            ("10.0.0.6", Device::new(6).state(Reachable).later(1, 1, Some(Unconfirmed))),
        ]);
        let probes = [
            at(1, "10.0.0.1"),
            at(2, "10.0.0.2"),
            at(3, "10.0.0.3"),
            at(4, "10.0.0.4"),
            at(5, "10.0.0.5"),
            probe(6, Place::Own),
            probe(7, Place::Nowhere),
            // The same address under a second inventory entry.
            at(8, "10.0.0.1"),
            at(9, "10.0.0.6"),
        ];
        let memory = Mutex::new(BTreeSet::new());
        let checks = round(&lan, &probes, &memory).await;
        let expected = [
            up(1, 2.0),
            up(2, 7.0),
            up_silent(3),
            down(4),
            down(5),
            up_silent(6),
            down(7),
            up(8, 2.0),
            up_silent(9),
        ];
        assert_eq!(checks, expected);
        // One ping where it was answered, two where it was not, and the
        // shared address only once.
        assert_eq!((lan.pings_to("10.0.0.1"), lan.pings_to("10.0.0.2"), lan.pings_to("10.0.0.4")), (1, 2, 2));
        assert_eq!(lan.pinged.lock().unwrap().len(), 11);
        // Nothing was left unsure, so the table was read before and after the
        // pings and not waited for.
        assert_eq!(lan.table_reads.load(Ordering::SeqCst), 2);
        assert_eq!(*memory.lock().unwrap(), BTreeSet::from([1, 2, 8]));
    }

    #[tokio::test]
    async fn a_device_that_answered_before_is_down_as_soon_as_it_stops() {
        let lan =
            FakeLan::with(vec![("10.0.0.1", Device::new(1).replies(&[Some(2), None]).state(Reachable))]);
        let memory = Mutex::new(BTreeSet::new());
        let probes = [at(1, "10.0.0.1"), at(2, "10.0.0.1")];
        // Round one: it answers. Device 2 shares the address, so it is marked too.
        assert_eq!(round(&lan, &probes, &memory).await, [up(1, 2.0), up(2, 2.0)]);
        // Round two: switched off. The system still remembers it as reachable
        // for a while, but that is not believed for a device that answers ping.
        assert_eq!(round(&lan, &probes, &memory).await, [down(1), down(2)]);
        // A device at the same address that has never answered is still
        // given the benefit of the neighbour table.
        assert_eq!(
            round(&lan, &[at(1, "10.0.0.1"), at(3, "10.0.0.1")], &memory).await,
            [down(1), up_silent(3)]
        );
    }

    #[tokio::test]
    async fn a_remembered_address_is_decided_after_the_settle_time() {
        let lan = FakeLan::with(vec![
            // A firewalled PC: the system asks it again and it answers.
            ("10.0.0.3", Device::new(3).state(Unconfirmed).later(2, 2, Some(Reachable))),
            // A device that left: the system asks and gives up.
            ("10.0.0.4", Device::new(4).state(Unconfirmed).later(2, 2, None)),
            // Still undecided at the end counts as not there.
            ("10.0.0.5", Device::new(5).state(Unconfirmed)),
        ]);
        let memory = Mutex::new(BTreeSet::new());
        let started = std::time::Instant::now();
        let checks = round(&lan, &[at(3, "10.0.0.3"), at(4, "10.0.0.4"), at(5, "10.0.0.5")], &memory).await;
        assert_eq!(checks, [up_silent(3), down(4), down(5)]);
        // Read before and after the pings, and once more when the settle time was up.
        assert!(started.elapsed() >= FAST.settle);
        assert_eq!(lan.table_reads.load(Ordering::SeqCst), 3);
        assert!(memory.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_round_with_hundreds_of_dead_devices_is_cut_short() {
        // 600 dead addresses, 64 at a time, 50 ms each: only five batches fit
        // in the 225 ms window, so there are no retries and many get no ping.
        let mut devices = vec![("10.0.9.9", Device::new(9).answers(1).state(Reachable))];
        let dead: Vec<String> = (0..600).map(|n| format!("10.0.{}.{}", n / 250, n % 250 + 1)).collect();
        devices.extend(dead.iter().map(|addr| (addr.as_str(), Device::new(1))));
        let lan = FakeLan::with(devices);
        // The device known to answer ping is last in the list but pinged first.
        let mut probes: Vec<Probe> = dead.iter().enumerate().map(|(n, addr)| at(n as i64, addr)).collect();
        probes.push(at(999, "10.0.9.9"));
        let memory = Mutex::new(BTreeSet::from([999]));

        let checks = round(&lan, &probes, &memory).await;
        assert_eq!(checks.len(), 601);
        assert_eq!(checks.last(), Some(&up(999, 1.0)));
        assert!(checks.iter().take(600).all(|c| !c.up));
        let pinged = lan.pinged.lock().unwrap();
        assert_eq!(pinged.first(), Some(&ip("10.0.9.9")));
        assert!((64..=5 * 64 + 1).contains(&pinged.len()), "{}", pinged.len());
        assert_eq!(pinged.iter().collect::<BTreeSet<_>>().len(), pinged.len(), "no address was pinged twice");
        assert!(lan.most_in_flight.load(Ordering::SeqCst) <= 64);
    }

    #[tokio::test]
    async fn resolve_sorts_hosts_into_places() {
        let own: HashSet<Ipv4Addr> = [ip("192.168.68.78")].into();
        let lookup = |name: String| async move {
            match name.as_str() {
                "nas.lan" => Some(ip("192.168.68.60")),
                "this-pc" => Some(ip("192.168.68.78")),
                "slow.lan" => {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    Some(ip("192.168.68.99"))
                }
                _ => None,
            }
        };
        let hosts = [
            "192.168.68.1",
            " 192.168.68.50 ",
            "192.168.68.78",
            "127.0.0.1",
            "nas.lan",
            "this-pc",
            "gone.lan",
            "slow.lan",
            "",
            "192.168.68",
            "fe80::1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.251",
            "nas.lan",
        ];
        let targets = named(&hosts);
        let started = std::time::Instant::now();
        let probes = resolve(&targets, &own, lookup, Duration::from_millis(100), &memory(300_000)).await;
        assert!(started.elapsed() < Duration::from_secs(5));
        let places: Vec<Place> = probes.iter().map(|p| p.place).collect();
        let nas = Place::At(ip("192.168.68.60"));
        assert_eq!(
            places,
            [
                Place::At(ip("192.168.68.1")),
                Place::At(ip("192.168.68.50")),
                Place::Own,
                Place::Own,
                nas,
                Place::Own,
                Place::Nowhere,
                Place::Nowhere,
                Place::Nowhere,
                Place::Nowhere,
                Place::Nowhere,
                Place::Nowhere,
                Place::Nowhere,
                Place::Nowhere,
                nas,
            ]
        );
        assert_eq!(probes.iter().map(|p| p.device_id).collect::<Vec<_>>(), (0..15).collect::<Vec<i64>>());
    }

    /// A name memory that keeps addresses for `keep_ms` milliseconds.
    fn memory(keep_ms: u64) -> Arc<Mutex<NameMemory>> {
        Arc::new(Mutex::new(NameMemory::new(Duration::from_millis(keep_ms))))
    }

    fn named(hosts: &[&str]) -> Vec<Target> {
        let target =
            |(n, host): (usize, &&str)| Target { device_id: n as i64, host: host.to_string(), mac: None };
        hosts.iter().enumerate().map(target).collect()
    }

    #[tokio::test]
    async fn slow_names_do_not_keep_the_others_from_being_looked_up() {
        // Forty switched-off machines whose names take ages to fail, and one
        // that is on, last in line.
        let mut hosts: Vec<String> = (0..40).map(|n| format!("off-{n:02}.local")).collect();
        hosts.push("zz-on.local".into());
        let hosts: Vec<&str> = hosts.iter().map(String::as_str).collect();
        let lookup = |name: String| async move {
            if name.starts_with("off-") {
                tokio::time::sleep(Duration::from_secs(30)).await;
                return None;
            }
            Some(ip("192.168.68.60"))
        };
        let (own, wait) = (HashSet::new(), Duration::from_millis(100));
        let probes = resolve(&named(&hosts), &own, lookup, wait, &memory(300_000)).await;
        assert_eq!(probes.len(), 41);
        assert_eq!(probes.last().map(|p| p.place), Some(Place::At(ip("192.168.68.60"))));
        assert!(probes.iter().take(40).all(|p| p.place == Place::Nowhere));
    }

    /// A name service that can be made slow or empty, and counts its lookups.
    #[derive(Default)]
    struct NameService {
        asked: AtomicUsize,
        /// How long each lookup takes, in milliseconds.
        slowness: AtomicU64,
        /// The last part of the address it gives for every name; 0 for "no such name".
        answer: AtomicU8,
    }

    type Lookup = Pin<Box<dyn Future<Output = Option<Ipv4Addr>> + Send>>;

    impl NameService {
        fn answering(last_part: u8) -> Arc<Self> {
            Arc::new(Self { answer: AtomicU8::new(last_part), ..Self::default() })
        }

        fn lookup(self: &Arc<Self>) -> impl Fn(String) -> Lookup {
            let service = self.clone();
            move |_name| {
                let service = service.clone();
                Box::pin(async move {
                    service.asked.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(service.slowness.load(Ordering::SeqCst))).await;
                    let last_part = service.answer.load(Ordering::SeqCst);
                    (last_part != 0).then(|| Ipv4Addr::new(192, 168, 68, last_part))
                })
            }
        }
    }

    #[tokio::test]
    async fn a_slow_lookup_falls_back_on_the_last_address_for_a_while() {
        let service = NameService::answering(60);
        let (memory, own, targets) = (memory(400), HashSet::new(), named(&["nas.local"]));
        let round = || resolve(&targets, &own, service.lookup(), Duration::from_millis(40), &memory);
        let at = |addr: &str| [at(0, addr)];

        assert_eq!(round().await, at("192.168.68.60"));
        // The name service turns slow (and the device moves): the round does
        // not wait for it, and the device is checked where it was a moment ago.
        service.slowness.store(250, Ordering::SeqCst);
        service.answer.store(61, Ordering::SeqCst);
        let started = std::time::Instant::now();
        assert_eq!(round().await, at("192.168.68.60"));
        assert!(started.elapsed() < Duration::from_millis(200));
        // The lookup of that round is still out, so no second one is started.
        assert_eq!(round().await, at("192.168.68.60"));
        assert_eq!(service.asked.load(Ordering::SeqCst), 2);
        // Its late answer counts when it comes, and frees the name for a new lookup.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(round().await, at("192.168.68.61"));
        assert_eq!(service.asked.load(Ordering::SeqCst), 3);

        // With no answer for longer than addresses are kept, the old one is dropped.
        tokio::time::sleep(Duration::from_millis(300)).await;
        service.slowness.store(5_000, Ordering::SeqCst);
        assert_eq!(round().await, at("192.168.68.61"));
        tokio::time::sleep(Duration::from_millis(450)).await;
        assert_eq!(round().await, [probe(0, Place::Nowhere)]);
    }

    #[tokio::test]
    async fn a_lookup_still_out_from_an_earlier_round_is_waited_for_when_nothing_is_remembered() {
        // A name that has never resolved yet, and a name service that takes
        // longer than the first round has patience for.
        let service = NameService::answering(60);
        service.slowness.store(80, Ordering::SeqCst);
        let (memory, own, targets) = (memory(300_000), HashSet::new(), named(&["nas.local"]));
        let round =
            |wait_ms| resolve(&targets, &own, service.lookup(), Duration::from_millis(wait_ms), &memory);

        assert_eq!(round(10).await, [probe(0, Place::Nowhere)]);
        // The next call starts no second lookup, but it does not come back
        // empty-handed at once either: it waits for the one that is out, and
        // only until that has answered.
        let started = std::time::Instant::now();
        assert_eq!(round(3_000).await, [at(0, "192.168.68.60")]);
        assert!(started.elapsed() < Duration::from_millis(1_500), "{:?}", started.elapsed());
        assert_eq!(service.asked.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn waiting_for_a_lookup_that_is_still_out_has_the_same_limit() {
        let service = NameService::answering(60);
        service.slowness.store(30_000, Ordering::SeqCst);
        let (memory, own, targets) = (memory(300_000), HashSet::new(), named(&["nas.local", "192.168.68.9"]));
        let round =
            |wait_ms| resolve(&targets, &own, service.lookup(), Duration::from_millis(wait_ms), &memory);
        let nowhere = [probe(0, Place::Nowhere), at(1, "192.168.68.9")];

        assert_eq!(round(10).await, nowhere);
        let started = std::time::Instant::now();
        assert_eq!(round(120).await, nowhere);
        let took = started.elapsed();
        assert!(took >= Duration::from_millis(120) && took < Duration::from_secs(5), "{took:?}");
        assert_eq!(service.asked.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_lookup_still_out_is_not_waited_for_when_an_address_is_remembered() {
        let service = NameService::answering(60);
        let (memory, own, targets) = (memory(300_000), HashSet::new(), named(&["nas.local"]));
        let round =
            |wait_ms| resolve(&targets, &own, service.lookup(), Duration::from_millis(wait_ms), &memory);
        assert_eq!(round(1_000).await, [at(0, "192.168.68.60")]);

        // The name service stops answering; a round gives up on it...
        service.slowness.store(30_000, Ordering::SeqCst);
        assert_eq!(round(10).await, [at(0, "192.168.68.60")]);
        // ...and the next one goes by what is remembered, without waiting.
        let started = std::time::Instant::now();
        assert_eq!(round(3_000).await, [at(0, "192.168.68.60")]);
        assert!(started.elapsed() < Duration::from_millis(1_500), "{:?}", started.elapsed());
        assert_eq!(service.asked.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_lookup_goes_on_and_frees_its_name_when_the_round_is_given_up() {
        let service = NameService::answering(60);
        service.slowness.store(60, Ordering::SeqCst);
        let (memory, own, targets) = (memory(300_000), HashSet::new(), named(&["nas.local"]));
        // The caller drops the round while it is waiting for the name.
        let given_up = tokio::time::timeout(
            Duration::from_millis(10),
            resolve(&targets, &own, service.lookup(), Duration::from_secs(3), &memory),
        );
        assert!(given_up.await.is_err());
        // The answer still arrives and is there for the next round, which
        // has no need to ask or wait.
        tokio::time::sleep(Duration::from_millis(200)).await;
        service.slowness.store(30_000, Ordering::SeqCst);
        let again = resolve(&targets, &own, service.lookup(), Duration::from_millis(10), &memory).await;
        assert_eq!(again, [at(0, "192.168.68.60")]);
        assert!(memory.lock().unwrap().asking.contains("nas.local"), "a new lookup was started");
        assert_eq!(service.asked.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_name_that_is_no_longer_known_is_forgotten_at_once() {
        let service = NameService::answering(60);
        let (memory, own, targets) = (memory(300_000), HashSet::new(), named(&["nas.local"]));
        let round = || resolve(&targets, &own, service.lookup(), Duration::from_millis(40), &memory);
        assert_eq!(round().await, [at(0, "192.168.68.60")]);
        service.answer.store(0, Ordering::SeqCst);
        assert_eq!(round().await, [probe(0, Place::Nowhere)]);
    }

    #[test]
    fn stored_hardware_addresses_are_read_however_they_are_written() {
        let mac = Some([0x8c, 0x90, 0x2d, 0xa0, 0x0e, 0x02]);
        for text in ["8c:90:2d:a0:0e:02", " 8C-90-2D-A0-0E-02 ", "8c90.2da0.0e02", "8c902da00e02"] {
            assert_eq!(mac_bytes(text), mac, "{text}");
        }
        for text in ["", "8c:90:2d:a0:0e", "8c:90:2d:a0:0e:02:11", "8c:90:2d:a0:0e:0g", "not an address"] {
            assert_eq!(mac_bytes(text), None, "{text}");
        }
    }

    #[tokio::test]
    async fn resolve_carries_the_stored_hardware_address() {
        let target = |device_id, mac: Option<&str>| Target {
            device_id,
            host: "192.168.68.101".into(),
            mac: mac.map(Into::into),
        };
        let targets = [target(1, Some("02:00:00:00:00:0a")), target(2, None), target(3, Some("nonsense"))];
        let lookup = |_name: String| async { None };
        let probes = resolve(&targets, &HashSet::new(), lookup, Duration::from_millis(100), &memory(1)).await;
        assert_eq!(
            probes,
            [stored(1, "192.168.68.101", 0x0a), at(2, "192.168.68.101"), at(3, "192.168.68.101")]
        );
    }

    #[test]
    fn the_freshest_entries_say_who_holds_an_address() {
        let entry = |addr: &str, last, state| Neighbour { ip: ip(addr), mac: [2, 0, 0, 0, 0, last], state };
        let table = [
            // A new entry on one adapter, an old one on the other.
            entry("10.0.0.1", 1, Unconfirmed),
            entry("10.0.0.1", 2, Reachable),
            // Two adapters that are equally sure of different devices.
            entry("10.0.0.2", 1, Reachable),
            entry("10.0.0.2", 2, Reachable),
            entry("10.0.0.3", 3, Permanent),
            Neighbour { ip: ip("10.0.0.4"), mac: [0xff; 6], state: Reachable },
        ];
        let macs = |last: &[u8]| last.iter().map(|l| [2, 0, 0, 0, 0, *l]).collect::<BTreeSet<_>>();
        assert_eq!(holders(&table, ip("10.0.0.1")), macs(&[2]));
        assert_eq!(holders(&table, ip("10.0.0.2")), macs(&[1, 2]));
        assert_eq!(holders(&table, ip("10.0.0.3")), macs(&[3]));
        assert_eq!(holders(&table, ip("10.0.0.4")), macs(&[]));
        assert_eq!(holders(&table, ip("10.0.0.5")), macs(&[]));
    }

    #[tokio::test]
    async fn an_address_handed_on_to_another_device_shows_only_that_one_as_up() {
        // A guest's phone (1) had 10.0.0.1 and left; a laptop (2) has it now.
        let lan = FakeLan::with(vec![("10.0.0.1", Device::new(2).answers(3).state(Reachable))]);
        let memory = Mutex::new(BTreeSet::new());
        let probes = [stored(1, "10.0.0.1", 1), stored(2, "10.0.0.1", 2)];
        assert_eq!(round(&lan, &probes, &memory).await, [down(1), up(2, 3.0)]);
        // The phone is not taken for a device that answers ping.
        assert_eq!(*memory.lock().unwrap(), BTreeSet::from([2]));
        // Read once before the pings (neither had answered one yet) and once after.
        assert_eq!((lan.pings_to("10.0.0.1"), lan.table_reads.load(Ordering::SeqCst)), (1, 2));

        // Both known to answer ping: the table is read for this alone, once.
        let memory = Mutex::new(BTreeSet::from([1, 2]));
        assert_eq!(round(&lan, &probes, &memory).await, [down(1), up(2, 3.0)]);
        assert_eq!(lan.table_reads.load(Ordering::SeqCst), 3);

        // The order of the two makes no difference, an entry of the kind
        // "remembered" is as good, and a third entry there that has no stored
        // hardware address is left as it was.
        lan.set_state("10.0.0.1", Some(Unconfirmed));
        let probes = [stored(2, "10.0.0.1", 2), at(3, "10.0.0.1"), stored(1, "10.0.0.1", 1)];
        assert_eq!(round(&lan, &probes, &memory).await, [up(2, 3.0), up(3, 3.0), down(1)]);
    }

    #[tokio::test]
    async fn a_shared_address_is_left_alone_when_the_table_cannot_tell() {
        let both_up = [up(1, 3.0), up(2, 3.0)];
        let probes = [stored(1, "10.0.0.1", 1), stored(2, "10.0.0.1", 2)];
        let memory = || Mutex::new(BTreeSet::from([1, 2]));

        // No entry for the address, as for one behind a router.
        let lan = FakeLan::with(vec![("10.0.0.1", Device::new(2).answers(3))]);
        assert_eq!(round(&lan, &probes, &memory()).await, both_up);
        assert_eq!(lan.table_reads.load(Ordering::SeqCst), 1);

        // An entry for a device that is neither of the two.
        let lan = FakeLan::with(vec![("10.0.0.1", Device::new(9).answers(3).state(Reachable))]);
        assert_eq!(round(&lan, &probes, &memory()).await, both_up);

        // An entry that is not one device's own hardware address.
        let lan = FakeLan::with(vec![("10.0.0.1", Device::new(2).answers(3))]);
        let everyone = Neighbour { ip: ip("10.0.0.1"), mac: [0xff; 6], state: Reachable };
        lan.other_adapter.lock().unwrap().push(everyone);
        assert_eq!(round(&lan, &probes, &memory()).await, both_up);

        // Two adapters that are equally sure of one each.
        let lan = FakeLan::with(vec![("10.0.0.1", Device::new(2).answers(3).state(Reachable))]);
        let other = Neighbour { ip: ip("10.0.0.1"), mac: [2, 0, 0, 0, 0, 1], state: Reachable };
        lan.other_adapter.lock().unwrap().push(other.clone());
        assert_eq!(round(&lan, &probes, &memory()).await, both_up);
        // An old entry on the second adapter gives way to the fresh one on the first.
        *lan.other_adapter.lock().unwrap() = vec![Neighbour { state: Unconfirmed, ..other }];
        assert_eq!(round(&lan, &probes, &memory()).await, [down(1), up(2, 3.0)]);
    }

    #[tokio::test]
    async fn devices_that_do_not_disagree_about_an_address_are_not_looked_up() {
        let lan = FakeLan::with(vec![("10.0.0.1", Device::new(2).answers(3).state(Reachable))]);
        let memory = Mutex::new(BTreeSet::from([1, 2, 3]));
        // The same hardware address twice (an entry added by hand next to a
        // scanned one), and entries with none: nothing contradicts anything,
        // even though the table holds a different one than the first two have.
        let same = [stored(1, "10.0.0.1", 1), stored(2, "10.0.0.1", 1)];
        assert_eq!(round(&lan, &same, &memory).await, [up(1, 3.0), up(2, 3.0)]);
        let one_missing = [stored(1, "10.0.0.1", 1), at(2, "10.0.0.1")];
        assert_eq!(round(&lan, &one_missing, &memory).await, [up(1, 3.0), up(2, 3.0)]);
        let both_missing = [at(1, "10.0.0.1"), at(2, "10.0.0.1")];
        assert_eq!(round(&lan, &both_missing, &memory).await, [up(1, 3.0), up(2, 3.0)]);
        // Different hardware addresses at different addresses are no conflict either.
        let apart = [stored(1, "10.0.0.1", 1), stored(3, "10.0.0.2", 3)];
        assert_eq!(round(&lan, &apart, &memory).await, [up(1, 3.0), down(3)]);
        // The table was never needed.
        assert_eq!(lan.table_reads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn the_conflict_rule_also_holds_for_devices_that_never_answer_ping() {
        // A firewalled PC (2) took over the address of a device (1) that left.
        let lan = FakeLan::with(vec![
            ("10.0.0.1", Device::new(2).state(Reachable)),
            // Remembered from earlier; the system asks again and it answers.
            ("10.0.0.2", Device::new(4).state(Unconfirmed).later(2, 2, Some(Reachable))),
            // Nothing there any more: nobody is up, so nothing needs deciding.
            ("10.0.0.3", Device::new(6)),
        ]);
        let memory = Mutex::new(BTreeSet::new());
        let probes = [
            stored(1, "10.0.0.1", 1),
            stored(2, "10.0.0.1", 2),
            stored(3, "10.0.0.2", 3),
            stored(4, "10.0.0.2", 4),
            stored(5, "10.0.0.3", 5),
            stored(6, "10.0.0.3", 6),
        ];
        let checks = round(&lan, &probes, &memory).await;
        assert_eq!(checks, [down(1), up_silent(2), down(3), up_silent(4), down(5), down(6)]);
        // Before the pings, after them and after the settle time: the last
        // copy serves the conflicts too, no extra one is taken.
        assert_eq!(lan.table_reads.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn the_system_lookup_knows_this_computer() {
        assert_eq!(os_lookup("localhost".into()).await, Some(Ipv4Addr::LOCALHOST));
        assert_eq!(os_lookup("not\0a name".into()).await, None);
    }
}
