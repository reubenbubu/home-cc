//! Names of devices on the network, from reverse DNS, mDNS and NetBIOS.
//!
//! All three use ordinary outgoing lookups: no administrator rights, and no
//! listening on a fixed port, which is what makes Windows show a firewall
//! prompt. Each device is asked directly, so nothing is broadcast.

#[path = "dns_wire.rs"]
mod dns_wire;
#[path = "netbios.rs"]
mod netbios;

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::{Instant, timeout_at};

/// Longest name handed back, in characters. Names come from other devices.
const MAX_NAME_CHARS: usize = 64;
/// The whole lookup stops waiting after this long.
const OVERALL: Duration = Duration::from_millis(2500);
/// Devices are asked directly twice, this long apart: Wi-Fi drops the odd packet.
const ROUND: Duration = Duration::from_millis(400);
const ROUNDS: u32 = 2;
const MDNS_PORT: u16 = 5353;
const NETBIOS_PORT: u16 = 137;
/// Reverse lookups allowed inside the operating system at once. Enough for
/// every device of a home network to start straight away.
const REVERSE_AT_ONCE: usize = 64;
/// Names a device gives that say what it is, not which one it is. They are
/// used only when nothing better turns up.
const GENERIC: [&str; 1] = ["pi.hole"];

/// The best name found for each address, already tidied for showing
/// ("deco-main.lan" becomes "deco-main"). Addresses with no name are left
/// out. Takes at most about 2.5 seconds. Must run inside the tokio runtime.
pub async fn discover(ips: &[Ipv4Addr]) -> HashMap<Ipv4Addr, String> {
    let mut seen = HashSet::new();
    let ips: Vec<Ipv4Addr> = ips.iter().copied().filter(|ip| is_device_address(*ip) && seen.insert(*ip)).collect();
    if ips.is_empty() {
        return HashMap::new();
    }
    // Devices are asked directly only on a home network: never send those
    // questions out to the internet, whatever range was asked for.
    let nearby: Vec<Ipv4Addr> = ips.iter().copied().filter(|ip| is_home_address(*ip)).collect();
    let deadline = Instant::now() + OVERALL;
    let (reverse, mdns, netbios) = tokio::join!(
        reverse_dns(&ips, reverse_gate(), deadline, os_reverse_lookup),
        ask_each(&nearby, MDNS_PORT, dns_wire::mdns_query, dns_wire::mdns_host, deadline),
        ask_each(&nearby, NETBIOS_PORT, netbios::status_query, |reply, _| netbios::computer_name(reply), deadline),
    );
    choose(&ips, &Sources { reverse, mdns, netbios })
}

fn is_device_address(ip: Ipv4Addr) -> bool {
    !(ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() || ip.is_broadcast())
}

/// True for addresses that can only be on a home or office network. The
/// last range is the one carriers and VPNs such as Tailscale hand out.
fn is_home_address(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_private() || ip.is_link_local() || (a == 100 && (64..128).contains(&b))
}

/// What each way of asking found, by address.
#[derive(Debug, Default)]
struct Sources {
    reverse: HashMap<Ipv4Addr, String>,
    mdns: HashMap<Ipv4Addr, String>,
    netbios: HashMap<Ipv4Addr, String>,
}

/// Picks one name per address. The network's own name for a device (reverse
/// DNS) comes first, because the owner usually set it; then the name the
/// device gives itself (mDNS), then its Windows-style name (NetBIOS).
fn choose(ips: &[Ipv4Addr], sources: &Sources) -> HashMap<Ipv4Addr, String> {
    // A reverse name that two addresses share tells them apart no better than
    // no name, and may be a server that answers the same for everything.
    let mut holders: HashMap<String, usize> = HashMap::new();
    for name in sources.reverse.values() {
        *holders.entry(whole_name(name).to_lowercase()).or_default() += 1;
    }
    let mut out = HashMap::new();
    for ip in ips {
        let reverse = sources.reverse.get(ip).map(|name| whole_name(name)).filter(|name| {
            holders.get(&name.to_lowercase()).is_some_and(|count| *count == 1) && !is_useless(name, *ip)
        });
        let (generic, specific) = match reverse {
            Some(name) if is_generic(name) => (Some(name), None),
            other => (None, other),
        };
        let candidates = [
            specific.map(|name| tidy(first_label(name))),
            sources.mdns.get(ip).map(|name| tidy(name)),
            sources.netbios.get(ip).map(|name| tidy(name)),
            generic.map(tidy),
        ];
        if let Some(name) = candidates.into_iter().flatten().find(|name| !name.is_empty()) {
            out.insert(*ip, name);
        }
    }
    out
}

fn whole_name(name: &str) -> &str {
    name.trim().trim_end_matches('.')
}

fn first_label(name: &str) -> &str {
    name.split('.').next().unwrap_or(name)
}

fn is_generic(name: &str) -> bool {
    GENERIC.iter().any(|generic| name.eq_ignore_ascii_case(generic))
}

/// True for reverse names that say nothing about the device: "localhost",
/// the address written backwards, or a name made from the address itself
/// ("ip-192-168-68-20.lan").
fn is_useless(name: &str, ip: Ipv4Addr) -> bool {
    let lower = name.to_ascii_lowercase();
    // Only the reverse-lookup endings: "printer.home.arpa" is a real name.
    if lower.ends_with(".in-addr.arpa") || lower.ends_with(".ip6.arpa") {
        return true;
    }
    // Names this PC made up, not the device's own: Linux calls the router
    // "_gateway", and Docker Desktop gives this PC's own address a
    // ".docker.internal" name.
    if lower == "localhost" || lower.starts_with("localhost.") || lower.starts_with('_') {
        return true;
    }
    if lower.ends_with(".docker.internal") {
        return true;
    }
    let numbers: Vec<&str> = lower.split(|c: char| !c.is_ascii_digit()).filter(|run| !run.is_empty()).collect();
    let octets = ip.octets().map(|octet| octet.to_string());
    let forwards = numbers.windows(4).any(|run| run.iter().eq(octets.iter()));
    let backwards = numbers.windows(4).any(|run| run.iter().eq(octets.iter().rev()));
    forwards || backwards
}

/// Makes text from the network safe and neat to show: no control or
/// invisible characters, single spaces, and not too long.
fn tidy(text: &str) -> String {
    let visible: String = text
        .chars()
        .filter_map(|c| match c {
            c if c.is_whitespace() => Some(' '),
            c if c.is_control() || is_invisible(c) => None,
            c => Some(c),
        })
        .collect();
    let spaced = visible.split(' ').filter(|word| !word.is_empty()).collect::<Vec<_>>().join(" ");
    let cut: String = spaced.chars().take(MAX_NAME_CHARS).collect();
    cut.trim_end().to_string()
}

/// Characters that take no space or change the direction of the text
/// around them, which could make one name look like another.
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{AD}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

/// Sends `question` to every address on `port`, then gathers names from the
/// replies with `answer`. Addresses that stay silent for a whole round are
/// asked once more. One socket on a port the system picks serves them all;
/// replies are matched to devices by where they came from.
async fn ask_each(
    ips: &[Ipv4Addr],
    port: u16,
    question: fn(Ipv4Addr) -> Vec<u8>,
    answer: fn(&[u8], Ipv4Addr) -> Option<String>,
    deadline: Instant,
) -> HashMap<Ipv4Addr, String> {
    let mut names = HashMap::new();
    let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await else {
        return names;
    };
    let wanted: HashSet<Ipv4Addr> = ips.iter().copied().collect();
    let mut answered: HashSet<Ipv4Addr> = HashSet::new();
    // 9000 bytes is the largest mDNS message. On Windows a reply longer than
    // the buffer is an error and is lost, not cut short.
    let mut buf = vec![0u8; 9000];

    for _ in 0..ROUNDS {
        for (i, ip) in ips.iter().enumerate() {
            if answered.contains(ip) {
                continue;
            }
            // Sending can fail for one address (no route to it); that must
            // not stop the others.
            let _ = socket.send_to(&question(*ip), (*ip, port)).await;
            // A short breath every few packets, so a burst doesn't get dropped.
            if i % 16 == 15 {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        }
        let round_end = (Instant::now() + ROUND).min(deadline);
        while names.len() < wanted.len() {
            match timeout_at(round_end, socket.recv_from(&mut buf)).await {
                Err(_) => break, // round over
                Ok(Ok((len, SocketAddr::V4(from)))) if wanted.contains(from.ip()) => {
                    let ip = *from.ip();
                    answered.insert(ip);
                    if !names.contains_key(&ip)
                        && let Some(name) = buf.get(..len).and_then(|reply| answer(reply, ip))
                    {
                        names.insert(ip, name);
                    }
                }
                Ok(Ok(_)) => {}
                // Windows reports "nothing listens on that port" from some
                // other device as an error on this socket. It says nothing
                // about the socket itself, so carry on, but never past the
                // end of the round if the error keeps coming.
                Ok(Err(_)) => {
                    if Instant::now() >= round_end {
                        break;
                    }
                }
            }
        }
        if answered.len() == wanted.len() || Instant::now() >= deadline {
            break;
        }
    }
    names
}

/// One gate for the whole app: it bounds how many threads can be stuck in
/// the operating system's resolver at once, however many scans run.
fn reverse_gate() -> Arc<Semaphore> {
    static GATE: OnceLock<Arc<Semaphore>> = OnceLock::new();
    GATE.get_or_init(|| Arc::new(Semaphore::new(REVERSE_AT_ONCE))).clone()
}

/// The operating system's own reverse lookup: its hosts file, the DNS
/// servers it is set up with, and whatever else it tries. Blocks, on Windows
/// for several seconds when the address has no name.
fn os_reverse_lookup(ip: Ipv4Addr) -> Option<String> {
    let name = dns_lookup::lookup_addr(&IpAddr::V4(ip)).ok()?;
    // Never show an address as a name.
    name.parse::<IpAddr>().is_err().then_some(name)
}

/// Reverse lookup for every address, returning what arrived by `deadline`.
///
/// A lookup that has started cannot be stopped, so its place at the gate
/// travels into the blocking call and is given back only when the operating
/// system really returns, not when we stop waiting for it.
async fn reverse_dns(
    ips: &[Ipv4Addr],
    gate: Arc<Semaphore>,
    deadline: Instant,
    lookup: fn(Ipv4Addr) -> Option<String>,
) -> HashMap<Ipv4Addr, String> {
    let mut lookups = JoinSet::new();
    for &ip in ips {
        let gate = gate.clone();
        lookups.spawn(async move {
            let permit = gate.acquire_owned().await.ok()?;
            let name = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                lookup(ip)
            })
            .await
            .ok()??;
            Some((ip, name))
        });
    }
    let mut names = HashMap::new();
    while let Ok(Some(done)) = timeout_at(deadline, lookups.join_next()).await {
        if let Ok(Some((ip, name))) = done {
            names.insert(ip, name);
        }
    }
    // Dropping `lookups` cancels the ones still waiting at the gate. The ones
    // already inside the operating system finish in their own time and then
    // free their place.
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

    fn ip(last: u8) -> Ipv4Addr {
        Ipv4Addr::new(192, 168, 68, last)
    }

    fn map(entries: &[(u8, &str)]) -> HashMap<Ipv4Addr, String> {
        entries.iter().map(|(last, name)| (ip(*last), name.to_string())).collect()
    }

    fn chosen(sources: Sources) -> HashMap<Ipv4Addr, String> {
        let ips: Vec<Ipv4Addr> = (1..=9).map(ip).collect();
        choose(&ips, &sources)
    }

    #[test]
    fn reverse_dns_wins_and_loses_its_ending() {
        let names = chosen(Sources {
            reverse: map(&[(1, "deco-main.lan"), (2, "reuben-pc.lan."), (3, "nas")]),
            mdns: map(&[(1, "something-else"), (4, "homelab-server")]),
            netbios: map(&[(2, "REUBEN-PC"), (4, "HOMELAB"), (5, "PRINTER-1")]),
        });
        assert_eq!(names, map(&[(1, "deco-main"), (2, "reuben-pc"), (3, "nas"), (4, "homelab-server"), (5, "PRINTER-1")]));
    }

    #[test]
    fn generic_reverse_name_is_the_last_resort() {
        let names = chosen(Sources {
            reverse: map(&[(1, "pi.hole"), (2, "PI.Hole.")]),
            mdns: map(&[(1, "pihole1")]),
            ..Default::default()
        });
        // Two addresses share it here, so it is not used even as a last resort.
        assert_eq!(names, map(&[(1, "pihole1")]));

        let names = chosen(Sources {
            reverse: map(&[(1, "pi.hole"), (2, "nas.lan")]),
            netbios: map(&[(1, "PIHOLE1")]),
            ..Default::default()
        });
        assert_eq!(names, map(&[(1, "PIHOLE1"), (2, "nas")]));

        // Alone, it is shown whole rather than as "pi".
        let names = chosen(Sources { reverse: map(&[(1, "pi.hole")]), ..Default::default() });
        assert_eq!(names, map(&[(1, "pi.hole")]));
    }

    #[test]
    fn shared_reverse_name_falls_through() {
        let names = chosen(Sources {
            reverse: map(&[(1, "gateway.lan"), (2, "Gateway.LAN."), (3, "nas.lan"), (4, "gateway.home")]),
            mdns: map(&[(1, "router-a")]),
            netbios: map(&[(1, "ROUTER"), (2, "ROUTER-B")]),
        });
        assert_eq!(names, map(&[(1, "router-a"), (2, "ROUTER-B"), (3, "nas"), (4, "gateway")]));
    }

    #[test]
    fn useless_reverse_names_fall_through() {
        let names = chosen(Sources {
            reverse: map(&[
                (1, "localhost"),
                (2, "localhost.localdomain"),
                (3, "3.68.168.192.in-addr.arpa"),
                (4, "ip-192-168-68-4.lan"),
                (5, "host-5.68.168.192.example.net"),
                (6, "192-168-68-66.lan"),
                (7, ".lan"),
            ]),
            mdns: map(&[(1, "this-pc"), (4, "tablet")]),
            ..Default::default()
        });
        // .6 is named after some other address, which is odd but is a name.
        assert_eq!(names, map(&[(1, "this-pc"), (4, "tablet"), (6, "192-168-68-66")]));
        assert!(is_useless("4.68.168.192.dsl.example.net", ip(4)));
        assert!(!is_useless("pc-192.lan", ip(4)));
        assert!(!is_useless("node4", ip(4)));
        // "home.arpa" is a proper ending for home networks, not a backwards address.
        assert!(!is_useless("nas.home.arpa", ip(4)));
    }

    #[test]
    fn empty_results_are_dropped() {
        let names = chosen(Sources {
            reverse: map(&[(1, ""), (2, "."), (3, " \t ")]),
            mdns: map(&[(1, "\u{0}\u{7}"), (2, "ok"), (4, "\u{200B}\u{202E}")]),
            netbios: map(&[(1, "   ")]),
        });
        assert_eq!(names, map(&[(2, "ok")]));
        // Only addresses that were asked for come back.
        let sources = Sources { mdns: map(&[(1, "one"), (200, "stray")]), ..Default::default() };
        assert_eq!(choose(&[ip(1)], &sources), map(&[(1, "one")]));
        assert!(choose(&[], &sources).is_empty());
    }

    #[test]
    fn names_are_tidied() {
        assert_eq!(tidy("  Emily's\tiPad \r\n"), "Emily's iPad");
        assert_eq!(tidy("evil\u{1b}[31m\u{0}name\u{7f}\u{85}"), "evil[31mname");
        assert_eq!(tidy("gnp.\u{202E}exe"), "gnp.exe");
        assert_eq!(tidy("Wohnzimmer-Gerät"), "Wohnzimmer-Gerät");
        assert_eq!(tidy("\u{FEFF}"), "");
        // Cut by characters, never in the middle of one.
        let long = "é".repeat(300);
        assert_eq!(tidy(&long), "é".repeat(MAX_NAME_CHARS));
        let spaced = format!("{} tail", "a".repeat(MAX_NAME_CHARS - 1));
        assert_eq!(tidy(&spaced), "a".repeat(MAX_NAME_CHARS - 1));

        let long_reverse = format!("{}.lan", "x".repeat(200));
        let names = chosen(Sources {
            reverse: map(&[(1, long_reverse.as_str())]),
            mdns: map(&[(2, "Living\u{0} Room   TV\n")]),
            netbios: map(&[(3, "  REUBEN-PC  ")]),
        });
        let cut = "x".repeat(MAX_NAME_CHARS);
        assert_eq!(names, map(&[(1, cut.as_str()), (2, "Living Room TV"), (3, "REUBEN-PC")]));
    }

    #[test]
    fn only_device_addresses_are_asked() {
        assert!(is_device_address(ip(1)));
        for odd in [Ipv4Addr::UNSPECIFIED, Ipv4Addr::LOCALHOST, Ipv4Addr::BROADCAST, Ipv4Addr::new(224, 0, 0, 251)] {
            assert!(!is_device_address(odd), "{odd}");
        }
    }

    #[test]
    fn made_up_reverse_names_fall_through() {
        let names = chosen(Sources {
            reverse: map(&[
                (1, "_gateway"),
                (2, "host.docker.internal"),
                (3, "printer.home.arpa"),
                (4, "_gateway.lan"),
            ]),
            netbios: map(&[(2, "MAIN-PC")]),
            ..Default::default()
        });
        assert_eq!(names, map(&[(2, "MAIN-PC"), (3, "printer")]));
        assert!(is_useless("1.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.8.e.f.ip6.arpa", ip(1)));
    }

    #[test]
    fn only_home_addresses_are_asked_directly() {
        for text in ["192.168.68.1", "10.1.2.3", "172.16.0.9", "169.254.3.4", "100.100.1.2"] {
            assert!(is_home_address(text.parse().unwrap()), "{text}");
        }
        for text in ["8.8.8.8", "172.32.0.1", "100.128.0.1", "100.63.0.1", "1.1.1.1"] {
            assert!(!is_home_address(text.parse().unwrap()), "{text}");
        }
    }

    #[test]
    fn discover_can_run_on_any_thread() {
        fn sendable<T: Send>(_: T) {}
        sendable(discover(&[]));
    }

    #[tokio::test]
    async fn discover_with_nothing_to_ask_is_instant() {
        let started = Instant::now();
        assert!(discover(&[]).await.is_empty());
        assert!(discover(&[Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST]).await.is_empty());
        assert!(started.elapsed() < Duration::from_millis(200));
    }

    /// A pretend device on this machine that answers every packet with `reply`.
    async fn responder(reply: Vec<u8>) -> (u16, tokio::task::JoinHandle<()>) {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = socket.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let mut buf = [0u8; 512];
            while let Ok((_, from)) = socket.recv_from(&mut buf).await {
                let _ = socket.send_to(&reply, from).await;
            }
        });
        (port, task)
    }

    fn fixed_question(_: Ipv4Addr) -> Vec<u8> {
        b"who are you?".to_vec()
    }

    fn text_answer(reply: &[u8], _: Ipv4Addr) -> Option<String> {
        std::str::from_utf8(reply).ok().filter(|text| !text.is_empty()).map(str::to_string)
    }

    #[tokio::test]
    async fn ask_each_reads_a_reply_and_stops_early() {
        let (port, device) = responder(b"test-device".to_vec()).await;
        let started = Instant::now();
        let names = ask_each(&[Ipv4Addr::LOCALHOST], port, fixed_question, text_answer, started + OVERALL).await;
        assert_eq!(names.get(&Ipv4Addr::LOCALHOST).map(String::as_str), Some("test-device"));
        assert!(started.elapsed() < ROUND, "every device answered, so there is nothing to wait for");
        device.abort();
    }

    #[tokio::test]
    async fn ask_each_gives_up_on_silence_and_on_nonsense() {
        // Nothing listens on this port: both rounds pass, then it gives up.
        let unused = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap().local_addr().unwrap().port();
        let started = Instant::now();
        let names = ask_each(&[Ipv4Addr::LOCALHOST], unused, fixed_question, text_answer, started + OVERALL).await;
        assert!(names.is_empty());
        assert!(started.elapsed() < ROUND * ROUNDS + Duration::from_millis(300));

        // A reply that isn't a name counts as an answer, so it isn't asked
        // twice, but gives no name.
        let (port, device) = responder(vec![0xFF, 0xFE]).await;
        let started = Instant::now();
        let names = ask_each(&[Ipv4Addr::LOCALHOST], port, fixed_question, text_answer, started + OVERALL).await;
        assert!(names.is_empty());
        assert!(started.elapsed() < ROUND + Duration::from_millis(300));
        device.abort();
    }

    #[tokio::test]
    async fn ask_each_respects_the_overall_deadline() {
        let unused = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap().local_addr().unwrap().port();
        let started = Instant::now();
        let deadline = started + Duration::from_millis(50);
        let names = ask_each(&[Ipv4Addr::LOCALHOST], unused, fixed_question, text_answer, deadline).await;
        assert!(names.is_empty());
        assert!(started.elapsed() < Duration::from_millis(300));
    }

    #[tokio::test]
    async fn ask_each_reads_real_reply_formats() {
        // The mDNS and NetBIOS readers, driven through a real socket.
        let mut mdns = vec![0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0];
        mdns.extend_from_slice(b"\x011\x010\x010\x03127\x07in-addr\x04arpa\x00");
        mdns.extend_from_slice(&[0, 12, 0x80, 1, 0, 0, 0, 120, 0, 15]);
        mdns.extend_from_slice(b"\x07test-pc\x05local\x00");
        let (port, device) = responder(mdns).await;
        let deadline = Instant::now() + OVERALL;
        let names = ask_each(&[Ipv4Addr::LOCALHOST], port, dns_wire::mdns_query, dns_wire::mdns_host, deadline).await;
        assert_eq!(names.get(&Ipv4Addr::LOCALHOST).map(String::as_str), Some("test-pc"));
        device.abort();
    }

    static RUNNING: AtomicUsize = AtomicUsize::new(0);
    static PEAK: AtomicUsize = AtomicUsize::new(0);
    static STARTED: AtomicUsize = AtomicUsize::new(0);

    /// Stands in for a system lookup that takes 600 ms to give up.
    fn slow_lookup(_: Ipv4Addr) -> Option<String> {
        STARTED.fetch_add(1, SeqCst);
        let now = RUNNING.fetch_add(1, SeqCst) + 1;
        PEAK.fetch_max(now, SeqCst);
        std::thread::sleep(Duration::from_millis(600));
        RUNNING.fetch_sub(1, SeqCst);
        None
    }

    #[tokio::test]
    async fn reverse_gate_really_limits_threads() {
        let ips: Vec<Ipv4Addr> = (1..=48).map(ip).collect();
        let gate = Arc::new(Semaphore::new(4));
        let started = Instant::now();
        let names = reverse_dns(&ips, gate.clone(), started + Duration::from_millis(1000), slow_lookup).await;
        assert!(names.is_empty());
        // Back on time, although no lookup had an answer by then.
        assert!(started.elapsed() < Duration::from_millis(1300));
        tokio::time::sleep(Duration::from_millis(1500)).await;
        // Never more than four at once, and every place given back once the
        // slow lookups returned.
        assert_eq!(PEAK.load(SeqCst), 4);
        assert_eq!(RUNNING.load(SeqCst), 0);
        assert_eq!(gate.available_permits(), 4);
        // Only lookups that got a place before the deadline ever started.
        assert!(STARTED.load(SeqCst) <= 8, "started {}", STARTED.load(SeqCst));
    }

    #[tokio::test]
    async fn reverse_dns_collects_what_arrives() {
        fn lookup(ip: Ipv4Addr) -> Option<String> {
            let [.., last] = ip.octets();
            (last % 2 == 1).then(|| format!("device-{last}.lan"))
        }
        let ips: Vec<Ipv4Addr> = (1..=6).map(ip).collect();
        let gate = Arc::new(Semaphore::new(2));
        let names = reverse_dns(&ips, gate.clone(), Instant::now() + OVERALL, lookup).await;
        assert_eq!(names, map(&[(1, "device-1.lan"), (3, "device-3.lan"), (5, "device-5.lan")]));
        assert_eq!(gate.available_permits(), 2);
    }

    #[test]
    fn system_lookup_never_gives_an_address_back() {
        // Whatever this machine calls itself, it is a name and not a number.
        if let Some(name) = os_reverse_lookup(Ipv4Addr::LOCALHOST) {
            assert!(name.parse::<IpAddr>().is_err());
            assert!(!name.is_empty());
        }
    }
}
