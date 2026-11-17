//! The status round: is each known device there right now?
//!
//! A device is up when it answers ping. Some never do (a Windows PC with its
//! firewall on, say); for those the neighbour table is the witness. A device
//! that has answered ping before is not given that second way, so turning it
//! off shows as down in the same round.

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
use tokio::task::JoinSet;
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
}

impl NameMemory {
    pub fn new(keep: Duration) -> Self {
        Self { keep, known: HashMap::new(), asking: HashSet::new() }
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
    }

    fn address(&self, name: &str) -> Option<Ipv4Addr> {
        self.known.get(name).filter(|(_, learned)| learned.elapsed() <= self.keep).map(|(ip, _)| *ip)
    }
}

/// The app's own memory of host names, until it closes.
pub static NAMES: LazyLock<Arc<Mutex<NameMemory>>> =
    LazyLock::new(|| Arc::new(Mutex::new(NameMemory::new(NAME_KEEP))));

/// Devices that have answered a ping since the app started, by inventory id.
pub static ANSWERS_PING: Mutex<BTreeSet<i64>> = Mutex::new(BTreeSet::new());

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
        let mut lookups = JoinSet::new();
        {
            let mut remembered = lock(memory);
            let keep = remembered.keep;
            remembered.known.retain(|_, (_, learned)| learned.elapsed() <= keep);
            for name in names {
                // A lookup left over from an earlier round will answer for this name.
                if !remembered.asking.insert(name.to_string()) {
                    continue;
                }
                let (name, memory, job) = (name.to_string(), memory.clone(), lookup(name.to_string()));
                lookups.spawn(async move {
                    let answer = job.await;
                    lock(&memory).answered(&name, answer);
                });
            }
        }
        let deadline = Instant::now() + wait;
        while let Ok(Some(_)) = tokio::time::timeout_at(deadline, lookups.join_next()).await {}
        // The ones still out are left to finish by themselves.
        lookups.detach_all();
    }
    let remembered = lock(memory);
    let locate = |host: &str| match host.parse::<Ipv4Addr>() {
        Ok(ip) => place(ip, own),
        Err(_) => remembered.address(host).map_or(Place::Nowhere, |ip| place(ip, own)),
    };
    targets.iter().map(|t| Probe { device_id: t.device_id, place: locate(t.host.trim()) }).collect()
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

fn lock<T>(shared: &Mutex<T>) -> MutexGuard<'_, T> {
    shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One round over resolved targets. `started` is when the round began, name
/// lookups included; the limits in `pace` count from there.
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
    let quiet: Vec<Ipv4Addr> =
        ips.iter().filter(|ip| may_use_table.contains(ip) && !seen.contains_key(ip)).copied().collect();
    if !quiet.is_empty() {
        let table = lan.neighbours().await;
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
            let table = lan.neighbours().await;
            for (ip, _) in unsure {
                if state_of(&table, ip) == Some(NeighState::Reachable) {
                    seen.entry(ip).or_default().neighbour = true;
                }
            }
        }
    }

    let mut answered_now = Vec::new();
    let mut checks = Vec::with_capacity(probes.len());
    for probe in probes {
        let (up, rtt_ms) = match probe.place {
            Place::Own => (true, None),
            Place::Nowhere => (false, None),
            Place::At(ip) => {
                verdict(seen.get(&ip).copied().unwrap_or_default(), known.contains(&probe.device_id))
            }
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

    fn at(device_id: i64, addr: &str) -> Probe {
        Probe { device_id, place: Place::At(ip(addr)) }
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
            Probe { device_id: 6, place: Place::Own },
            Probe { device_id: 7, place: Place::Nowhere },
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
        let targets: Vec<Target> = hosts
            .iter()
            .enumerate()
            .map(|(n, host)| Target { device_id: n as i64, host: host.to_string() })
            .collect();
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
        let target = |(n, host): (usize, &&str)| Target { device_id: n as i64, host: host.to_string() };
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
        let at = |addr: &str| [Probe { device_id: 0, place: Place::At(ip(addr)) }];

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
        assert_eq!(round().await, [Probe { device_id: 0, place: Place::Nowhere }]);
    }

    #[tokio::test]
    async fn a_name_that_is_no_longer_known_is_forgotten_at_once() {
        let service = NameService::answering(60);
        let (memory, own, targets) = (memory(300_000), HashSet::new(), named(&["nas.local"]));
        let round = || resolve(&targets, &own, service.lookup(), Duration::from_millis(40), &memory);
        assert_eq!(round().await, [Probe { device_id: 0, place: Place::At(ip("192.168.68.60")) }]);
        service.answer.store(0, Ordering::SeqCst);
        assert_eq!(round().await, [Probe { device_id: 0, place: Place::Nowhere }]);
    }

    #[tokio::test]
    async fn the_system_lookup_knows_this_computer() {
        assert_eq!(os_lookup("localhost".into()).await, Some(Ipv4Addr::LOCALHOST));
        assert_eq!(os_lookup("not\0a name".into()).await, None);
    }
}
