//! The two things scanning and checking ask of the network: ping an address,
//! and read the neighbour table. They sit behind a trait so the logic built
//! on them can be tested against a made-up network.

use super::neigh::{self, Neighbour};
use super::ping::{self, PingOutcome};
use std::future::Future;
use std::io;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Semaphore, mpsc};

/// The most pings in flight at once. With a 1 second timeout that is at most
/// 64 new addresses a second, which keeps the system's neighbour table from
/// overflowing: 128 did overflow it on Linux, and pings then fail to send.
pub const MOST_PINGS: usize = 64;

/// How long past its own timeout a ping is waited for before it is given up on.
const PING_GRACE: Duration = Duration::from_secs(3);

pub trait Lan: Send + Sync + 'static {
    /// Whether this PC lets the app send pings at all. Sends nothing.
    fn allowed(&self) -> io::Result<()> {
        Ok(())
    }

    /// One ping. It must come back soon after `timeout` whatever happens:
    /// scans and status rounds wait for every ping they start.
    fn ping(&self, ip: Ipv4Addr, timeout: Duration) -> impl Future<Output = io::Result<PingOutcome>> + Send;

    /// The neighbour table of all adapters together, so the same address can
    /// be in it more than once. Empty when it can't be read.
    fn neighbours(&self) -> impl Future<Output = Vec<Neighbour>> + Send;
}

/// The real network, through the operating system.
pub struct OsLan;

/// The limit holds for the whole app: a status round that falls in the
/// middle of a scan shares the scan's 64 rather than adding its own.
static PING_SLOTS: Semaphore = Semaphore::const_new(MOST_PINGS);

impl Lan for OsLan {
    fn allowed(&self) -> io::Result<()> {
        ping::allowed()
    }

    async fn ping(&self, ip: Ipv4Addr, timeout: Duration) -> io::Result<PingOutcome> {
        in_slot(&PING_SLOTS, timeout + PING_GRACE, ping::ping_once(ip, timeout)).await
    }

    async fn neighbours(&self) -> Vec<Neighbour> {
        let read = tokio::task::spawn_blocking(neigh::neighbours).await;
        // A scan without hardware addresses is still worth more than no scan.
        read.ok().and_then(Result::ok).unwrap_or_default()
    }
}

/// Run `ping` in one of `slots`, as a task of its own, and wait for it no
/// longer than `longest`.
///
/// The ping has a timeout of its own, but on Windows that one is kept by the
/// system, and a request the system never answers would otherwise hang the
/// scan or status round that sent it and keep its slot until the app is
/// closed. Such a ping is never stopped (see `ping::ping_once` for why): it
/// is left to end by itself, and only the waiting and the slot are given up.
async fn in_slot(
    slots: &Semaphore,
    longest: Duration,
    ping: impl Future<Output = io::Result<PingOutcome>> + Send + 'static,
) -> io::Result<PingOutcome> {
    let _slot = slots.acquire().await.map_err(io::Error::other)?;
    // Dropping the handle of a task lets it run on; it does not stop it.
    match tokio::time::timeout(longest, tokio::spawn(ping)).await {
        Ok(ended) => ended.map_err(io::Error::other)?,
        Err(_) => Err(io::Error::other("the ping never came back")),
    }
}

type Finished = (Ipv4Addr, io::Result<PingOutcome>);

/// Runs pings side by side, never more than `limit` at once.
///
/// Each ping is its own task that is never stopped early, even when the
/// `Pinger` is dropped: a ping that has started always runs to its end (see
/// `ping::ping_once` for why that matters on Windows). The `Pinger` waits for
/// every one of them, which `Lan::ping` keeps short.
pub struct Pinger<L> {
    lan: Arc<L>,
    limit: usize,
    in_flight: usize,
    tx: mpsc::UnboundedSender<Finished>,
    rx: mpsc::UnboundedReceiver<Finished>,
}

/// Hands in a ping's result when the task ends, however it ends, so the
/// count of pings in flight can never get stuck.
struct Report {
    ip: Ipv4Addr,
    outcome: Option<io::Result<PingOutcome>>,
    tx: mpsc::UnboundedSender<Finished>,
}

impl Drop for Report {
    fn drop(&mut self) {
        let outcome = self.outcome.take().unwrap_or_else(|| Err(io::Error::other("the ping was stopped")));
        let _ = self.tx.send((self.ip, outcome));
    }
}

impl<L: Lan> Pinger<L> {
    pub fn new(lan: Arc<L>, limit: usize) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self { lan, limit: limit.max(1), in_flight: 0, tx, rx }
    }

    pub fn has_room(&self) -> bool {
        self.in_flight < self.limit
    }

    pub fn is_idle(&self) -> bool {
        self.in_flight == 0
    }

    /// Start one ping now. Check `has_room` first.
    pub fn start(&mut self, ip: Ipv4Addr, timeout: Duration) {
        let mut report = Report { ip, outcome: None, tx: self.tx.clone() };
        let lan = self.lan.clone();
        self.in_flight += 1;
        tokio::spawn(async move {
            let outcome = lan.ping(ip, timeout).await;
            report.outcome.replace(outcome);
        });
    }

    /// The next ping to finish. None when none are in flight. Safe to drop
    /// while waiting.
    pub async fn next(&mut self) -> Option<Finished> {
        if self.in_flight == 0 {
            return None;
        }
        let finished = self.rx.recv().await;
        self.in_flight -= 1;
        finished
    }

    /// Ping every address in `ips` and wait for them all. `may_start` is
    /// asked just before each ping starts; once it says no, no more are
    /// started and the ones in flight are waited for.
    pub async fn run(
        &mut self,
        ips: &[Ipv4Addr],
        timeout: Duration,
        mut may_start: impl FnMut(Ipv4Addr) -> bool,
        mut finished: impl FnMut(Ipv4Addr, io::Result<PingOutcome>),
    ) {
        let mut queue = ips.iter().copied();
        let mut stopped = false;
        loop {
            while !stopped && self.has_room() {
                match queue.next() {
                    Some(ip) if may_start(ip) => self.start(ip, timeout),
                    _ => stopped = true,
                }
            }
            match self.next().await {
                Some((ip, outcome)) => finished(ip, outcome),
                None => break,
            }
        }
    }
}

/// A made-up network for tests: devices with scripted ping and neighbour
/// table behaviour, and a record of what was pinged.
#[cfg(test)]
pub mod fake {
    use super::*;
    use crate::net::neigh::NeighState;
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub struct Device {
        pub mac: [u8; 6],
        /// The answer to the first, second, ... ping; the last one repeats.
        /// None is no answer.
        pub replies: Vec<Option<Duration>>,
        /// What the neighbour table says before anything else applies.
        pub state: Option<NeighState>,
        /// Later states as (pings received, table reads since the last ping,
        /// state): the last rule whose two counts are both reached wins.
        pub later: Vec<(usize, usize, Option<NeighState>)>,
        pings: usize,
        reads_since_ping: usize,
    }

    impl Device {
        pub fn new(last_mac_byte: u8) -> Self {
            let mac = [2, 0, 0, 0, 0, last_mac_byte];
            Self { mac, replies: vec![None], state: None, later: Vec::new(), pings: 0, reads_since_ping: 0 }
        }

        /// Answers every ping after `ms` milliseconds.
        pub fn answers(mut self, ms: u64) -> Self {
            self.replies = vec![Some(Duration::from_millis(ms))];
            self
        }

        pub fn replies(mut self, replies: &[Option<u64>]) -> Self {
            self.replies = replies.iter().map(|r| r.map(Duration::from_millis)).collect();
            self
        }

        pub fn state(mut self, state: NeighState) -> Self {
            self.state = Some(state);
            self
        }

        pub fn later(mut self, pings: usize, reads: usize, state: Option<NeighState>) -> Self {
            self.later.push((pings, reads, state));
            self
        }

        fn state_now(&self) -> Option<NeighState> {
            let rule = self
                .later
                .iter()
                .rev()
                .find(|(pings, reads, _)| self.pings >= *pings && self.reads_since_ping >= *reads);
            rule.map_or(self.state, |(_, _, state)| *state)
        }
    }

    #[derive(Default)]
    pub struct FakeLan {
        pub devices: Mutex<BTreeMap<Ipv4Addr, Device>>,
        /// Every ping sent, in order.
        pub pinged: Mutex<Vec<Ipv4Addr>>,
        in_flight: AtomicUsize,
        pub most_in_flight: AtomicUsize,
        pub table_reads: AtomicUsize,
        /// When set, every ping fails outright, as on a PC that forbids them.
        broken: bool,
        /// Rows a second adapter adds to every table read, after the devices' own.
        pub other_adapter: Mutex<Vec<Neighbour>>,
    }

    pub fn ip(text: &str) -> Ipv4Addr {
        text.parse().unwrap()
    }

    impl FakeLan {
        pub fn with(devices: Vec<(&str, Device)>) -> Arc<Self> {
            let devices = devices.into_iter().map(|(addr, device)| (ip(addr), device)).collect();
            Arc::new(Self { devices: Mutex::new(devices), ..Self::default() })
        }

        /// A network where every ping fails outright.
        pub fn broken() -> Arc<Self> {
            Arc::new(Self { broken: true, ..Self::default() })
        }

        pub fn set_state(&self, addr: &str, state: Option<NeighState>) {
            if let Some(device) = self.devices.lock().unwrap().get_mut(&ip(addr)) {
                device.state = state;
            }
        }

        pub fn pings_to(&self, addr: &str) -> usize {
            self.pinged.lock().unwrap().iter().filter(|p| **p == ip(addr)).count()
        }
    }

    impl Lan for FakeLan {
        fn allowed(&self) -> io::Result<()> {
            if self.broken { Err(io::Error::from(io::ErrorKind::PermissionDenied)) } else { Ok(()) }
        }

        async fn ping(&self, ip: Ipv4Addr, timeout: Duration) -> io::Result<PingOutcome> {
            self.pinged.lock().unwrap().push(ip);
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.most_in_flight.fetch_max(now, Ordering::SeqCst);
            let reply = match self.devices.lock().unwrap().get_mut(&ip) {
                Some(device) => {
                    let reply = device.replies.get(device.pings).or(device.replies.last()).copied().flatten();
                    device.pings += 1;
                    device.reads_since_ping = 0;
                    reply
                }
                None => None,
            };
            let outcome = match reply {
                _ if self.broken => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
                Some(rtt) if rtt <= timeout => {
                    tokio::time::sleep(rtt).await;
                    Ok(PingOutcome::Up(rtt))
                }
                _ => {
                    tokio::time::sleep(timeout).await;
                    Ok(PingOutcome::Timeout)
                }
            };
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            outcome
        }

        async fn neighbours(&self) -> Vec<Neighbour> {
            self.table_reads.fetch_add(1, Ordering::SeqCst);
            let mut devices = self.devices.lock().unwrap();
            let mut table = Vec::new();
            for (ip, device) in devices.iter_mut() {
                device.reads_since_ping += 1;
                if let Some(state) = device.state_now() {
                    table.push(Neighbour { ip: *ip, mac: device.mac, state });
                }
            }
            table.extend(self.other_adapter.lock().unwrap().iter().cloned());
            table
        }
    }

    #[tokio::test]
    async fn pinger_keeps_to_its_limit_and_reports_every_ping() {
        let lan = FakeLan::with(vec![
            ("10.0.0.1", Device::new(1).answers(1)),
            ("10.0.0.2", Device::new(2).answers(3)),
        ]);
        let ips: Vec<Ipv4Addr> = (1..=20).map(|n| Ipv4Addr::new(10, 0, 0, n)).collect();
        let mut up = Vec::new();
        let mut finished = 0;
        let mut pinger = Pinger::new(lan.clone(), 4);
        let record = |ip, outcome| {
            finished += 1;
            if matches!(outcome, Ok(PingOutcome::Up(_))) {
                up.push(ip);
            }
        };
        pinger.run(&ips, Duration::from_millis(10), |_| true, record).await;
        up.sort();
        assert_eq!((finished, up), (20, vec![ip("10.0.0.1"), ip("10.0.0.2")]));
        assert_eq!(lan.most_in_flight.load(Ordering::SeqCst), 4);
        assert!(pinger.is_idle());
    }

    #[tokio::test]
    async fn pinger_stops_starting_when_told_and_still_waits_for_the_rest() {
        let lan = FakeLan::with(vec![]);
        let ips: Vec<Ipv4Addr> = (1..=20).map(|n| Ipv4Addr::new(10, 0, 0, n)).collect();
        let mut finished = 0;
        let mut pinger = Pinger::new(lan.clone(), 4);
        pinger
            .run(&ips, Duration::from_millis(10), |ip| ip != Ipv4Addr::new(10, 0, 0, 7), |_, _| finished += 1)
            .await;
        assert_eq!((finished, lan.pinged.lock().unwrap().len()), (6, 6));
    }

    #[tokio::test]
    async fn a_dropped_pinger_does_not_stop_pings_already_sent() {
        let lan = FakeLan::with(vec![]);
        let mut pinger = Pinger::new(lan.clone(), 4);
        pinger.start(ip("10.0.0.1"), Duration::from_millis(20));
        tokio::task::yield_now().await;
        drop(pinger);
        assert_eq!(lan.in_flight.load(Ordering::SeqCst), 1);
        // It ends by itself, at its own timeout.
        let ended = async {
            while lan.in_flight.load(Ordering::SeqCst) != 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), ended).await.expect("the ping never finished");
    }

    #[tokio::test]
    async fn a_ping_that_never_comes_back_is_given_up_on_but_not_stopped() {
        let slots = Semaphore::new(1);
        let ended = Arc::new(AtomicUsize::new(0));
        let stuck = {
            let ended = ended.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(150)).await;
                ended.fetch_add(1, Ordering::SeqCst);
                Ok(PingOutcome::Timeout)
            }
        };
        let started = std::time::Instant::now();
        let outcome = in_slot(&slots, Duration::from_millis(30), stuck).await;
        assert!(outcome.is_err() && started.elapsed() < Duration::from_millis(140), "{outcome:?}");
        // The only slot is free again for the next ping...
        let quick = async { Ok(PingOutcome::Up(Duration::from_millis(1))) };
        let outcome = in_slot(&slots, Duration::from_millis(30), quick).await;
        assert_eq!(outcome.ok(), Some(PingOutcome::Up(Duration::from_millis(1))));
        // ...and the one given up on still ran to its own end.
        assert_eq!(ended.load(Ordering::SeqCst), 0);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(ended.load(Ordering::SeqCst), 1);
    }

    /// With every slot taken by a ping that never comes back, the pings
    /// queued behind them still get their turn and the `Pinger` still ends.
    #[tokio::test]
    async fn stuck_pings_do_not_hold_up_the_ones_behind_them() {
        struct StuckLan(Semaphore);
        impl Lan for StuckLan {
            async fn ping(&self, ip: Ipv4Addr, timeout: Duration) -> io::Result<PingOutcome> {
                // Odd addresses hang for ever; even ones answer at once.
                let stuck = ip.octets()[3] % 2 == 1;
                let ping = async move {
                    if stuck {
                        std::future::pending::<()>().await;
                    }
                    Ok(PingOutcome::Up(Duration::from_millis(1)))
                };
                in_slot(&self.0, timeout, ping).await
            }

            async fn neighbours(&self) -> Vec<Neighbour> {
                Vec::new()
            }
        }
        let ips: Vec<Ipv4Addr> = [1, 3, 5, 7, 2, 4].map(|n| Ipv4Addr::new(10, 0, 0, n)).to_vec();
        let (mut up, mut failed) = (0, 0);
        let mut pinger = Pinger::new(Arc::new(StuckLan(Semaphore::new(2))), 6);
        let record = |_, outcome: io::Result<PingOutcome>| match outcome {
            Ok(_) => up += 1,
            Err(_) => failed += 1,
        };
        let all = pinger.run(&ips, Duration::from_millis(40), |_| true, record);
        tokio::time::timeout(Duration::from_secs(10), all).await.expect("the pings were waited for for ever");
        assert_eq!((up, failed), (2, 4));
    }
}
