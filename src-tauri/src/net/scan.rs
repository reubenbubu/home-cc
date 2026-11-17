//! Finding every device in a range: ping all of it while copying the
//! neighbour table, give quiet devices a second chance, then let the
//! neighbour table decide about the ones that never answered.

use super::ifaces::Plan;
use super::lan::{Lan, MOST_PINGS, Pinger};
use super::neigh::{NeighState, Neighbour, is_device_mac};
use super::ping::PingOutcome;
use super::{ScanPhase, ScanProgress};
use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::time::{Instant, MissedTickBehavior};

pub struct Timing {
    /// Pings in flight at once.
    pub in_flight: usize,
    pub sweep_timeout: Duration,
    /// Sleeping Wi-Fi devices often need more than a second for their first answer.
    pub second_timeout: Duration,
    /// How long the system takes to decide whether a remembered hardware
    /// address is still there: 5 seconds of waiting plus 3 tries.
    pub settle: Duration,
    /// How often the neighbour table is copied while the sweep runs.
    pub sample_every: Duration,
}

pub const REAL: Timing = Timing {
    in_flight: MOST_PINGS,
    sweep_timeout: Duration::from_millis(1000),
    second_timeout: Duration::from_millis(2000),
    settle: Duration::from_secs(8),
    sample_every: Duration::from_millis(1000),
};

/// One device the scan found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub ip: Ipv4Addr,
    pub mac: Option<[u8; 6]>,
    /// None when it never answered ping but its hardware address shows it is there.
    pub rtt: Option<Duration>,
    /// This PC. It is never pinged and never in its own neighbour table.
    pub is_self: bool,
}

/// Numbers for troubleshooting. The app itself doesn't need them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Counters {
    pub pinged: usize,
    pub answered_in_sweep: usize,
    pub ping_errors: usize,
    pub first_error: Option<String>,
    pub table_copies: usize,
    /// Addresses whose hardware address was copied during the sweep but had
    /// already gone from the table when the sweep ended.
    pub copied_but_gone: Vec<Ipv4Addr>,
    pub second_chance_tried: usize,
    pub second_chance_answered: usize,
    pub waited_to_settle: bool,
    pub cancelled: bool,
}

pub struct Swept {
    /// In address order.
    pub hits: Vec<Hit>,
    pub counters: Counters,
}

/// What the neighbour table says about a device that gave no ping reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    There,
    /// The address is only remembered; the system has not decided yet.
    Unsure,
    Gone,
}

pub fn presence(state: Option<NeighState>) -> Presence {
    match state {
        Some(NeighState::Reachable | NeighState::Permanent) => Presence::There,
        Some(NeighState::Unconfirmed) => Presence::Unsure,
        None => Presence::Gone,
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Seen {
    mac: Option<[u8; 6]>,
    rtt: Option<Duration>,
}

/// Everything learned about the range so far.
#[derive(Debug, Default)]
struct Tally {
    devices: BTreeMap<Ipv4Addr, Seen>,
}

impl Tally {
    fn answered(&mut self, ip: Ipv4Addr, rtt: Duration) {
        self.devices.entry(ip).or_default().rtt = Some(rtt);
    }

    /// A hardware address without a ping reply still makes a device worth a second look.
    fn learn(&mut self, table: &[Neighbour]) {
        for n in table {
            self.devices.entry(n.ip).or_default().mac = Some(n.mac);
        }
    }

    /// Devices with a hardware address that gave no ping reply.
    fn silent(&self) -> Vec<Ipv4Addr> {
        self.devices.iter().filter(|(_, seen)| seen.rtt.is_none()).map(|(ip, _)| *ip).collect()
    }

    /// Decide about the silent devices from a fresh table: the ones the
    /// system has given up on are dropped, and the ones it has not made up
    /// its mind about are returned.
    fn judge(&mut self, table: &[Neighbour]) -> Vec<Ipv4Addr> {
        let mut unsure = Vec::new();
        for ip in self.silent() {
            match presence(table.iter().find(|n| n.ip == ip).map(|n| n.state)) {
                Presence::There => {}
                Presence::Unsure => unsure.push(ip),
                Presence::Gone => {
                    self.devices.remove(&ip);
                }
            }
        }
        unsure
    }

    /// After the wait: an unsure device still in the table is there (the
    /// system has asked it again by now), one that is gone has left. Slow
    /// answers that arrived meanwhile are picked up too.
    fn settle(&mut self, unsure: &[Ipv4Addr], table: &[Neighbour]) {
        for ip in unsure {
            if !table.iter().any(|n| n.ip == *ip) {
                self.devices.remove(ip);
            }
        }
        self.learn(table);
    }

    fn forget(&mut self, ips: &[Ipv4Addr]) {
        for ip in ips {
            self.devices.remove(ip);
        }
    }

    fn len(&self) -> usize {
        self.devices.len()
    }
}

/// The neighbour table, cut down to real devices inside the range.
async fn read_table<L: Lan>(lan: &L, plan: &Plan) -> Vec<Neighbour> {
    if plan.links.is_empty() {
        return Vec::new();
    }
    devices_in_range(lan.neighbours().await, plan)
}

/// One entry for each device in the range, in address order. The table
/// covers every adapter, so with two of them on the network an address can be
/// in it twice: the entry the system is surest of is the one from the adapter
/// it really sends through.
fn devices_in_range(table: Vec<Neighbour>, plan: &Plan) -> Vec<Neighbour> {
    let sureness = |state| match state {
        NeighState::Reachable => 2,
        NeighState::Permanent => 1,
        NeighState::Unconfirmed => 0,
    };
    let mut surest: BTreeMap<Ipv4Addr, Neighbour> = BTreeMap::new();
    for entry in table {
        if !plan.is_host(entry.ip)
            || !is_device_mac(&entry.mac)
            || plan.own.iter().any(|(ip, _)| *ip == entry.ip)
        {
            continue;
        }
        match surest.get(&entry.ip) {
            Some(kept) if sureness(kept.state) >= sureness(entry.state) => {}
            _ => {
                surest.insert(entry.ip, entry);
            }
        }
    }
    surest.into_values().collect()
}

/// Find the devices in `plan.range`. `progress` is called through the sweep
/// and confirm phases. Setting `cancel` stops new pings from starting; what
/// is already known for sure is still returned.
pub async fn find_devices<L: Lan>(
    lan: Arc<L>,
    plan: &Plan,
    timing: &Timing,
    progress: &(dyn Fn(ScanProgress) + Send + Sync),
    cancel: &AtomicBool,
) -> Swept {
    let own: Vec<Ipv4Addr> = plan.own.iter().map(|(ip, _)| *ip).collect();
    // Our own address is never pinged: there is nothing to learn from it,
    // and an instant reply is the one case the Windows ping code handles badly.
    let targets: Vec<Ipv4Addr> = plan.hosts().filter(|ip| !own.contains(ip)).collect();
    // Without an adapter on the range there is no neighbour table to read.
    let on_link = !plan.links.is_empty();
    let cancelled = || cancel.load(Ordering::Relaxed);
    let report = |phase, done, total, found: usize| {
        progress(ScanProgress { phase, done, total, found: found + own.len() })
    };

    let mut tally = Tally::default();
    let mut counters = Counters::default();
    let mut pinger = Pinger::new(lan.clone(), timing.in_flight);

    // 1. Ping every address, which also makes the system look up each
    //    hardware address. Copy the neighbour table as we go: under the
    //    pressure of a thousand lookups the system throws entries out again
    //    (Windows keeps only 256), so reading it once at the end loses some.
    let mut copied: BTreeSet<Ipv4Addr> = BTreeSet::new();
    let mut ticker = tokio::time::interval_at(Instant::now() + timing.sample_every, timing.sample_every);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut queue = targets.iter().copied();
    let mut done = 0;
    report(ScanPhase::Sweep, 0, targets.len(), 0);
    loop {
        while pinger.has_room() && !cancelled() {
            let Some(ip) = queue.next() else { break };
            pinger.start(ip, timing.sweep_timeout);
            counters.pinged += 1;
        }
        if pinger.is_idle() {
            break;
        }
        tokio::select! {
            finished = pinger.next() => {
                match finished {
                    Some((ip, Ok(PingOutcome::Up(rtt)))) => {
                        tally.answered(ip, rtt);
                        counters.answered_in_sweep += 1;
                    }
                    Some((_, Ok(_))) | None => {}
                    Some((ip, Err(e))) => {
                        counters.ping_errors += 1;
                        counters.first_error.get_or_insert_with(|| format!("{ip}: {e}"));
                    }
                }
                done += 1;
                report(ScanPhase::Sweep, done, targets.len(), tally.len());
            }
            _ = ticker.tick(), if on_link => {
                let table = read_table(&*lan, plan).await;
                copied.extend(table.iter().map(|n| n.ip));
                tally.learn(&table);
                counters.table_copies += 1;
                report(ScanPhase::Sweep, done, targets.len(), tally.len());
            }
        }
    }

    // 2. The table as it is now, on top of the copies.
    let mut table = read_table(&*lan, plan).await;
    counters.copied_but_gone =
        copied.iter().filter(|ip| !table.iter().any(|n| n.ip == **ip)).copied().collect();
    tally.learn(&table);

    // 3. A second, longer ping for everything that has (or had) a hardware
    //    address but gave no reply in time.
    let silent = tally.silent();
    // Nothing silent means nothing to confirm, and no reports for that phase.
    let confirmed = |done: usize, found| {
        if !silent.is_empty() {
            report(ScanPhase::Confirm, done, silent.len(), found);
        }
    };
    confirmed(0, tally.len());
    if !silent.is_empty() && !cancelled() {
        let (mut tried, mut answered) = (0, 0);
        let may_start = |_| {
            let go = !cancelled();
            tried += usize::from(go);
            go
        };
        let finished = |ip, outcome| {
            if let Ok(PingOutcome::Up(rtt)) = outcome {
                tally.answered(ip, rtt);
                answered += 1;
                confirmed(answered, tally.len());
            }
        };
        pinger.run(&silent, timing.second_timeout, may_start, finished).await;
        (counters.second_chance_tried, counters.second_chance_answered) = (tried, answered);
        table = read_table(&*lan, plan).await;
    }

    // 4. The ones that still gave no reply are judged by the neighbour table.
    let unsure = tally.judge(&table);
    confirmed(silent.len().saturating_sub(unsure.len()), tally.len());

    // 5. Only when needed: wait for the system to make up its mind about
    //    remembered addresses, then look once more. Some pings failing to
    //    send is the other reason: their lookups may still come in.
    let some_failed = counters.ping_errors > 0 && counters.ping_errors < counters.pinged;
    if (!unsure.is_empty() || some_failed) && on_link && !cancelled() {
        counters.waited_to_settle = true;
        let until = Instant::now() + timing.settle;
        while Instant::now() < until && !cancelled() {
            tokio::time::sleep_until(until.min(Instant::now() + Duration::from_millis(200))).await;
        }
        if !cancelled() {
            tally.settle(&unsure, &read_table(&*lan, plan).await);
        }
    }
    if cancelled() {
        // No time to find out: keep only what is known for sure.
        tally.forget(&unsure);
        counters.cancelled = true;
    }
    confirmed(silent.len(), tally.len());

    let mut hits: BTreeMap<Ipv4Addr, Hit> = tally
        .devices
        .iter()
        .map(|(ip, seen)| (*ip, Hit { ip: *ip, mac: seen.mac, rtt: seen.rtt, is_self: false }))
        .collect();
    for (ip, mac) in &plan.own {
        hits.insert(*ip, Hit { ip: *ip, mac: *mac, rtt: None, is_self: true });
    }
    Swept { hits: hits.into_values().collect(), counters }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::ifaces::{Adapter, Link};
    use crate::net::lan::fake::{Device, FakeLan, ip};
    use crate::net::neigh::NeighState::{Permanent, Reachable, Unconfirmed};
    use std::sync::Mutex;

    /// The real steps, speeded up.
    const FAST: Timing = Timing {
        in_flight: 64,
        sweep_timeout: Duration::from_millis(100),
        second_timeout: Duration::from_millis(200),
        settle: Duration::from_millis(100),
        sample_every: Duration::from_millis(10),
    };

    /// A plan for a range that is this PC's whole network.
    fn plan(range: &str, own: &[&str]) -> Plan {
        plan_on(range, range, own)
    }

    /// A plan for a range, with this PC on the network `net`.
    fn plan_on(range: &str, net: &str, own: &[&str]) -> Plan {
        let adapter = Adapter { index: 2, name: "eth0".into() };
        Plan {
            range: range.parse().unwrap(),
            links: vec![Link { adapter, net: net.parse().unwrap() }],
            own: own.iter().map(|addr| (ip(addr), Some([2, 0, 0, 0, 0, 0xee]))).collect(),
        }
    }

    fn neighbour(addr: &str, state: NeighState) -> Neighbour {
        Neighbour { ip: ip(addr), mac: [2, 0, 0, 0, 0, 1], state }
    }

    async fn run(lan: &Arc<FakeLan>, plan: &Plan) -> Swept {
        find_devices(lan.clone(), plan, &FAST, &|_| {}, &AtomicBool::new(false)).await
    }

    /// (address, has a hardware address, answered ping, is this PC) per hit.
    fn summary(swept: &Swept) -> Vec<(String, bool, bool, bool)> {
        swept.hits.iter().map(|h| (h.ip.to_string(), h.mac.is_some(), h.rtt.is_some(), h.is_self)).collect()
    }

    #[test]
    fn neighbour_states_decide_about_silent_devices() {
        assert_eq!(presence(Some(Reachable)), Presence::There);
        assert_eq!(presence(Some(Permanent)), Presence::There);
        assert_eq!(presence(Some(Unconfirmed)), Presence::Unsure);
        assert_eq!(presence(None), Presence::Gone);
    }

    #[test]
    fn tally_judges_and_settles() {
        let mut tally = Tally::default();
        tally.answered(ip("10.0.0.1"), Duration::from_millis(3));
        tally.learn(&[
            neighbour("10.0.0.1", Unconfirmed),
            neighbour("10.0.0.2", Reachable),
            neighbour("10.0.0.3", Unconfirmed),
            neighbour("10.0.0.4", Unconfirmed),
            neighbour("10.0.0.5", Unconfirmed),
        ]);
        assert_eq!(tally.silent(), [ip("10.0.0.2"), ip("10.0.0.3"), ip("10.0.0.4"), ip("10.0.0.5")]);

        // .5 has dropped out of the table: gone. .3 and .4 are not decided yet.
        // A device that answered ping stays whatever the table says.
        let table = [
            neighbour("10.0.0.2", Reachable),
            neighbour("10.0.0.3", Unconfirmed),
            neighbour("10.0.0.4", Unconfirmed),
        ];
        let unsure = tally.judge(&table);
        assert_eq!(unsure, [ip("10.0.0.3"), ip("10.0.0.4")]);
        assert_eq!(tally.len(), 4);

        // After the wait .3 answered the system, .4 did not, and .6 turned up late.
        tally.settle(&unsure, &[neighbour("10.0.0.3", Reachable), neighbour("10.0.0.6", Reachable)]);
        let left: Vec<String> = tally.devices.keys().map(|ip| ip.to_string()).collect();
        assert_eq!(left, ["10.0.0.1", "10.0.0.2", "10.0.0.3", "10.0.0.6"]);
        assert_eq!(
            tally.devices[&ip("10.0.0.1")],
            Seen { mac: Some([2, 0, 0, 0, 0, 1]), rtt: Some(Duration::from_millis(3)) }
        );
    }

    #[tokio::test]
    async fn finds_devices_that_answer_and_devices_that_only_have_a_hardware_address() {
        let lan = FakeLan::with(vec![
            // An ordinary device: answers ping, the system knows its address.
            ("10.0.0.1", Device::new(1).answers(2).state(Reachable)),
            // A Windows PC with its firewall on: no ping reply, but it is there.
            ("10.0.0.9", Device::new(9).state(Reachable)),
            // A sleepy phone: too slow for the sweep, answers the second ping.
            ("10.0.0.20", Device::new(20).replies(&[None, Some(150)]).state(Unconfirmed)),
            // A device that left: its address is still remembered for a while.
            ("10.0.0.30", Device::new(30).state(Unconfirmed).later(2, 2, None)),
            // A quiet device whose remembered address the system confirms in the end.
            ("10.0.0.31", Device::new(31).state(Unconfirmed).later(2, 2, Some(Reachable))),
            // An address nobody has: the system asked and got nothing.
            ("10.0.0.40", Device::new(40)),
        ]);
        let swept = run(&lan, &plan("10.0.0.0/24", &["10.0.0.78"])).await;
        assert_eq!(
            summary(&swept),
            [
                ("10.0.0.1".into(), true, true, false),
                ("10.0.0.9".into(), true, false, false),
                ("10.0.0.20".into(), true, true, false),
                ("10.0.0.31".into(), true, false, false),
                ("10.0.0.78".into(), true, false, true),
            ]
        );
        let c = &swept.counters;
        assert_eq!((c.pinged, c.answered_in_sweep, c.ping_errors), (253, 1, 0));
        assert_eq!((c.second_chance_tried, c.second_chance_answered, c.waited_to_settle), (4, 1, true));
        // This PC is never pinged; the network and broadcast addresses neither.
        assert_eq!(lan.pings_to("10.0.0.78") + lan.pings_to("10.0.0.0") + lan.pings_to("10.0.0.255"), 0);
        // One ping in the sweep, one more only for the silent ones.
        assert_eq!(
            (lan.pings_to("10.0.0.1"), lan.pings_to("10.0.0.9"), lan.pings_to("10.0.0.40")),
            (1, 2, 1)
        );
        assert!(lan.most_in_flight.load(Ordering::SeqCst) <= 64);
    }

    #[tokio::test]
    async fn keeps_a_hardware_address_the_system_threw_out_during_the_sweep() {
        let lan = FakeLan::with(vec![("10.0.0.61", Device::new(61).answers(5).state(Reachable))]);
        // The table loses the entry when the last ping of the sweep is in,
        // just before the scan reads it "at the end".
        let evict = {
            let lan = lan.clone();
            move |p: ScanProgress| {
                if p.phase == ScanPhase::Sweep && p.done == p.total {
                    lan.set_state("10.0.0.61", None);
                }
            }
        };
        let swept =
            find_devices(lan.clone(), &plan("10.0.0.0/24", &[]), &FAST, &evict, &AtomicBool::new(false))
                .await;
        assert_eq!(summary(&swept), [("10.0.0.61".into(), true, true, false)]);
        assert_eq!(swept.counters.copied_but_gone, [ip("10.0.0.61")]);
        assert!(swept.counters.table_copies >= 1);
    }

    #[tokio::test]
    async fn nothing_to_confirm_means_no_waiting() {
        let lan = FakeLan::with(vec![("10.0.0.1", Device::new(1).answers(1).state(Reachable))]);
        let swept = run(&lan, &plan("10.0.0.0/28", &[])).await;
        assert_eq!(summary(&swept), [("10.0.0.1".into(), true, true, false)]);
        // One ping per address and nothing more.
        assert!(!swept.counters.waited_to_settle);
        assert_eq!((swept.counters.second_chance_tried, lan.pinged.lock().unwrap().len()), (0, 14));
    }

    #[tokio::test]
    async fn range_behind_a_router_gives_ping_results_only() {
        let lan = FakeLan::with(vec![
            ("10.9.0.5", Device::new(5).answers(4).state(Reachable)),
            ("10.9.0.6", Device::new(6).state(Reachable)),
        ]);
        let mut plan = plan("10.9.0.0/28", &[]);
        plan.links.clear();
        let swept = run(&lan, &plan).await;
        assert_eq!(summary(&swept), [("10.9.0.5".into(), false, true, false)]);
        assert_eq!(swept.counters.table_copies, 0);
    }

    #[tokio::test]
    async fn table_entries_that_are_not_devices_in_the_range_are_ignored() {
        let lan = FakeLan::with(vec![
            ("10.0.0.0", Device::new(1).state(Permanent)),
            ("10.0.0.15", Device::new(2).state(Permanent)),
            ("10.0.0.16", Device::new(3).state(Reachable)),
            ("10.0.0.7", Device::new(4).state(Permanent)),
            ("10.0.0.8", Device::new(5).state(Reachable)),
            ("10.0.0.3", Device::new(6).state(Reachable)),
        ]);
        // Windows lists group addresses with a made-up hardware address.
        lan.devices.lock().unwrap().get_mut(&ip("10.0.0.8")).unwrap().mac = [0x01, 0, 0x5e, 0, 0, 0xfb];
        let swept = run(&lan, &plan("10.0.0.0/28", &["10.0.0.3"])).await;
        assert_eq!(
            summary(&swept),
            [("10.0.0.3".into(), true, false, true), ("10.0.0.7".into(), true, false, false)]
        );
    }

    #[test]
    fn of_two_table_entries_for_one_address_the_surest_is_kept() {
        let entry = |addr: &str, last_mac_byte, state| Neighbour {
            ip: ip(addr),
            mac: [2, 0, 0, 0, 0, last_mac_byte],
            state,
        };
        let table = vec![
            // The same devices as two adapters see them, in either order.
            entry("10.0.0.5", 1, Unconfirmed),
            entry("10.0.0.5", 2, Reachable),
            entry("10.0.0.4", 3, Reachable),
            entry("10.0.0.4", 4, Unconfirmed),
            entry("10.0.0.6", 5, Unconfirmed),
            entry("10.0.0.6", 6, Permanent),
            // Equally sure: the first one stays.
            entry("10.0.0.7", 7, Unconfirmed),
            entry("10.0.0.7", 8, Unconfirmed),
            // This PC, a group address and an address outside the range.
            entry("10.0.0.3", 9, Reachable),
            Neighbour { ip: ip("10.0.0.8"), mac: [0x01, 0, 0x5e, 0, 0, 0xfb], state: Permanent },
            entry("10.0.0.16", 10, Reachable),
        ];
        let kept = devices_in_range(table, &plan("10.0.0.0/28", &["10.0.0.3"]));
        let expected = [
            entry("10.0.0.4", 3, Reachable),
            entry("10.0.0.5", 2, Reachable),
            entry("10.0.0.6", 6, Permanent),
            entry("10.0.0.7", 7, Unconfirmed),
        ];
        assert_eq!(kept, expected);
    }

    /// A PC with a cable and Wi-Fi on the same network: the system sends
    /// through one of them, and what it learns is listed under that one.
    #[tokio::test]
    async fn hardware_addresses_on_any_adapter_of_the_network_are_seen() {
        let lan = FakeLan::with(vec![
            ("10.0.0.1", Device::new(1).answers(2)),
            // The adapter that is not used still remembers this one from earlier.
            ("10.0.0.9", Device::new(0x99).state(Unconfirmed)),
            ("10.0.0.10", Device::new(10)),
        ]);
        let on_other_adapter = |addr: &str, last_mac_byte| Neighbour {
            ip: ip(addr),
            mac: [2, 0, 0, 0, 0, last_mac_byte],
            state: Reachable,
        };
        *lan.other_adapter.lock().unwrap() = vec![
            on_other_adapter("10.0.0.1", 1),
            on_other_adapter("10.0.0.9", 9),
            on_other_adapter("10.0.0.10", 10),
        ];
        let mut plan = plan("10.0.0.0/28", &[]);
        let net = plan.range;
        plan.links.insert(0, Link { adapter: Adapter { index: 7, name: "wlan0".into() }, net });
        let swept = run(&lan, &plan).await;
        // The one that answers ping has its hardware address, and the two
        // that don't are found at all.
        assert_eq!(
            summary(&swept),
            [
                ("10.0.0.1".into(), true, true, false),
                ("10.0.0.9".into(), true, false, false),
                ("10.0.0.10".into(), true, false, false),
            ]
        );
        // The fresh entry wins over the remembered one, so there was nothing to wait for.
        let fresh = swept.hits.iter().find(|h| h.ip == ip("10.0.0.9")).and_then(|h| h.mac);
        assert_eq!(fresh, Some([2, 0, 0, 0, 0, 9]));
        assert!(!swept.counters.waited_to_settle);
    }

    #[tokio::test]
    async fn part_of_a_bigger_network_is_scanned_to_both_its_ends() {
        // The first and last address of the part are ordinary devices here.
        let lan = FakeLan::with(vec![
            ("10.0.4.0", Device::new(1).answers(2).state(Reachable)),
            ("10.0.4.15", Device::new(2).state(Reachable)),
        ]);
        let swept = run(&lan, &plan_on("10.0.4.0/28", "10.0.0.0/16", &["10.0.4.9"])).await;
        assert_eq!(
            summary(&swept),
            [
                ("10.0.4.0".into(), true, true, false),
                ("10.0.4.9".into(), true, false, true),
                ("10.0.4.15".into(), true, false, false),
            ]
        );
        assert_eq!(swept.counters.pinged, 15);
    }

    #[tokio::test]
    async fn the_networks_own_two_addresses_inside_a_wider_range_are_left_alone() {
        // This PC is on 10.0.1.0/29; the owner typed a range twice as wide.
        let lan = FakeLan::with(vec![
            ("10.0.1.0", Device::new(1).answers(1).state(Reachable)),
            ("10.0.1.7", Device::new(2).answers(1).state(Reachable)),
            ("10.0.1.6", Device::new(3).answers(1).state(Reachable)),
            // Behind the router: it answers, but has no hardware address here.
            ("10.0.1.8", Device::new(4).answers(1)),
        ]);
        let swept = run(&lan, &plan_on("10.0.1.0/28", "10.0.1.0/29", &["10.0.1.2"])).await;
        assert_eq!(
            summary(&swept),
            [
                ("10.0.1.2".into(), true, false, true),
                ("10.0.1.6".into(), true, true, false),
                ("10.0.1.8".into(), false, true, false),
            ]
        );
        // Neither the network's first address nor the one that reaches everybody was pinged.
        assert_eq!(lan.pings_to("10.0.1.0") + lan.pings_to("10.0.1.7"), 0);
        assert_eq!(swept.counters.pinged, 12);
    }

    #[tokio::test]
    async fn cancel_stops_new_pings_and_returns_what_is_known() {
        let lan = FakeLan::with(vec![
            ("10.0.0.1", Device::new(1).answers(1).state(Reachable)),
            ("10.0.0.2", Device::new(2).state(Reachable)),
            ("10.0.0.3", Device::new(3).state(Unconfirmed)),
            // Would answer, but its turn never comes.
            ("10.0.3.9", Device::new(9).answers(1)),
        ]);
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = {
            let cancel = cancel.clone();
            move |p: ScanProgress| {
                if p.done >= 10 {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
        };
        let swept =
            find_devices(lan.clone(), &plan("10.0.0.0/22", &["10.0.1.1"]), &FAST, &stop, &cancel).await;
        // Far fewer than the 1021 addresses, and no second pings.
        assert!(swept.counters.pinged < 200, "{}", swept.counters.pinged);
        assert_eq!(lan.pinged.lock().unwrap().len(), swept.counters.pinged);
        assert!(swept.counters.cancelled && !swept.counters.waited_to_settle);
        // The device that answered, the one the system is sure about, and this
        // PC. Not the one that is only remembered, nor the one never reached.
        assert_eq!(
            summary(&swept),
            [
                ("10.0.0.1".into(), true, true, false),
                ("10.0.0.2".into(), true, false, false),
                ("10.0.1.1".into(), true, false, true)
            ]
        );
    }

    #[tokio::test]
    async fn cancel_during_the_wait_ends_it_early() {
        let lan = FakeLan::with(vec![("10.0.0.3", Device::new(3).state(Unconfirmed))]);
        let slow = Timing { settle: Duration::from_secs(30), ..FAST };
        let cancel = Arc::new(AtomicBool::new(false));
        // The sweep and the second ping take 0.3 seconds; then the wait starts.
        let stop = {
            let cancel = cancel.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(600)).await;
                cancel.store(true, Ordering::Relaxed);
            })
        };
        let started = std::time::Instant::now();
        let swept = find_devices(lan.clone(), &plan("10.0.0.0/28", &[]), &slow, &|_| {}, &cancel).await;
        stop.await.unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(swept.hits.is_empty() && swept.counters.cancelled);
    }

    #[tokio::test]
    async fn pings_that_cannot_be_sent_are_counted_not_fatal() {
        let lan = FakeLan::broken();
        let swept = run(&lan, &plan("10.0.0.0/28", &["10.0.0.2"])).await;
        assert_eq!((swept.counters.pinged, swept.counters.ping_errors), (13, 13));
        assert!(swept.counters.first_error.is_some() && !swept.counters.waited_to_settle);
        assert_eq!(summary(&swept), [("10.0.0.2".into(), true, false, true)]);
    }

    #[tokio::test]
    async fn progress_runs_forward_through_sweep_then_confirm() {
        let lan = FakeLan::with(vec![
            ("10.0.0.1", Device::new(1).answers(1).state(Reachable)),
            ("10.0.0.20", Device::new(20).replies(&[None, Some(150)]).state(Unconfirmed)),
            ("10.0.0.30", Device::new(30).state(Unconfirmed).later(2, 2, None)),
        ]);
        let seen: Arc<Mutex<Vec<ScanProgress>>> = Arc::default();
        let record = {
            let seen = seen.clone();
            move |p| seen.lock().unwrap().push(p)
        };
        let swept = find_devices(
            lan.clone(),
            &plan("10.0.0.0/26", &["10.0.0.7"]),
            &FAST,
            &record,
            &AtomicBool::new(false),
        )
        .await;
        let seen = seen.lock().unwrap();

        let first = seen.first().unwrap();
        assert_eq!((first.phase, first.done, first.total, first.found), (ScanPhase::Sweep, 0, 61, 1));
        let sweep: Vec<&ScanProgress> = seen.iter().filter(|p| p.phase == ScanPhase::Sweep).collect();
        assert!(sweep.windows(2).all(|w| w[0].done <= w[1].done && w[1].total == 61));
        assert_eq!(sweep.last().unwrap().done, 61);

        let confirm: Vec<&ScanProgress> = seen.iter().filter(|p| p.phase == ScanPhase::Confirm).collect();
        assert!(confirm.windows(2).all(|w| w[0].done <= w[1].done && w[1].total == 2));
        let last = seen.last().unwrap();
        assert_eq!(
            (last.phase, last.done, last.total, last.found),
            (ScanPhase::Confirm, 2, 2, swept.hits.len())
        );
        // Sweep reports all come before confirm reports.
        let first_confirm = seen.iter().position(|p| p.phase == ScanPhase::Confirm).unwrap();
        assert!(seen.iter().skip(first_confirm).all(|p| p.phase == ScanPhase::Confirm));
        assert_eq!(swept.hits.len(), 3);
    }
}
