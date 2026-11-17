//! Which of this PC's network adapters is on the home network, and what part
//! of that network to scan.

use netdev::Interface;
use netdev::interface::types::InterfaceType;
use netdev::ipnet::Ipv4Net;
use std::collections::HashSet;
use std::net::Ipv4Addr;

/// The most the app scans when it picks the range itself: a /22 is 1022
/// addresses and takes about 16 seconds to ping.
pub const AUTO_PREFIX: u8 = 22;
/// The biggest range the owner may type: a /20 is 4094 addresses, about a minute.
pub const BIGGEST_PREFIX: u8 = 20;

/// A network adapter, as the operating system names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    pub index: u32,
    /// Linux: the kernel name ("eth0"). Windows: the adapter's GUID.
    pub name: String,
}

/// One of this PC's addresses that could be on the home network.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// The name to show the owner ("Wi-Fi", "Ethernet", or the kernel name on Linux).
    pub label: String,
    pub own_ip: Ipv4Addr,
    /// What to scan: the adapter's whole network (192.168.68.0/22), or the
    /// /22 around our own address when that network is bigger.
    pub scan_net: Ipv4Net,
    /// 3 = has a router and looks real, 2 = has a router but looks virtual
    /// (VPN, Hyper-V external switch), 1 = no router.
    pub score: u8,
    /// The adapter the system itself sends through by default.
    pub routed: bool,
}

/// Linux adapter names that are never the home network (matched as a prefix).
const LINUX_VIRTUAL_PREFIXES: &[&str] = &[
    "docker",
    "br-",
    "veth",
    "virbr",
    "lxcbr",
    "lxdbr",
    "cni",
    "flannel",
    "podman",
    "tailscale",
    "zt",
    "wg",
    "tun",
    "tap",
    "vboxnet",
    "vmnet",
];

/// Words in a Windows adapter's name or description that mark it as virtual.
const WINDOWS_VIRTUAL_WORDS: &[&str] = &[
    "hyper-v",
    "vethernet",
    "wsl",
    "vmware",
    "virtualbox",
    "tap-",
    "wintun",
    "wireguard",
    "openvpn",
    "vpn",
    "tailscale",
    "zerotier",
    "loopback",
    "bluetooth",
    "npcap",
];

fn looks_virtual(iface: &Interface) -> bool {
    let name = iface.name.to_lowercase();
    if LINUX_VIRTUAL_PREFIXES.iter().any(|p| name.starts_with(p)) {
        return true;
    }
    [&iface.friendly_name, &iface.description].into_iter().flatten().any(|text| {
        let text = text.to_lowercase();
        WINDOWS_VIRTUAL_WORDS.iter().any(|w| text.contains(w))
    })
}

/// The home network candidates among the operating system's adapters, best first.
pub fn rank(interfaces: &[Interface]) -> Vec<Candidate> {
    let mut out = Vec::new();
    for iface in interfaces {
        if !iface.is_up() || iface.is_loopback() || iface.is_point_to_point() {
            continue;
        }
        // Leaves out tunnels, dial-up and VPN adapters such as Tailscale's.
        if !matches!(
            iface.if_type,
            InterfaceType::Ethernet
                | InterfaceType::Wireless80211
                | InterfaceType::GigabitEthernet
                | InterfaceType::FastEthernetT
                | InterfaceType::FastEthernetFx
                | InterfaceType::Bridge
        ) {
            continue;
        }
        let gateways: &[Ipv4Addr] = iface.gateway.as_ref().map(|g| g.ipv4.as_slice()).unwrap_or_default();
        let is_virtual = looks_virtual(iface);
        for addr in &iface.ipv4 {
            let ip = addr.addr();
            // Home ranges only, which drops VPN and public addresses. A /31
            // or /32 is a link to one other machine, never a network to scan.
            if !ip.is_private() || addr.prefix_len() > 30 {
                continue;
            }
            let net = addr.trunc();
            // A router inside the network is the best sign of the real home
            // network: Docker, WSL and virtual machine adapters never have one.
            let score = match (gateways.iter().any(|g| net.contains(g)), is_virtual) {
                (true, false) => 3,
                (true, true) => 2,
                (false, false) => 1,
                (false, true) => continue,
            };
            let scan_net = if net.prefix_len() < AUTO_PREFIX {
                Ipv4Net::new(ip, AUTO_PREFIX).map(|n| n.trunc()).unwrap_or(net)
            } else {
                net
            };
            out.push(Candidate {
                label: iface.friendly_name.clone().unwrap_or_else(|| iface.name.clone()),
                own_ip: ip,
                scan_net,
                score,
                routed: iface.default,
            });
        }
    }
    // With a cable and Wi-Fi on the same network both score the same; the
    // one the system sends through goes first. The sort is stable: what is
    // still equal stays in the system's order.
    out.sort_by(|a, b| b.score.cmp(&a.score).then(b.routed.cmp(&a.routed)));
    out
}

/// Every IPv4 address this PC has, on any adapter.
pub fn own_ips(interfaces: &[Interface]) -> HashSet<Ipv4Addr> {
    interfaces.iter().flat_map(|i| i.ipv4.iter().map(|a| a.addr())).collect()
}

/// A network this PC is directly on, and the adapter that is on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub adapter: Adapter,
    /// The adapter's whole network, which can be bigger than the range scanned.
    pub net: Ipv4Net,
}

/// What a scan of one range needs to know about this PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub range: Ipv4Net,
    /// Every network this PC is directly on that reaches into the range. A
    /// PC with a cable and Wi-Fi on the home network has two, and only the
    /// system knows which one it sends through. Empty when the range is only
    /// reached through a router: such a scan finds no hardware addresses.
    pub links: Vec<Link>,
    /// This PC's own addresses inside the range, with their hardware address.
    pub own: Vec<(Ipv4Addr, Option<[u8; 6]>)>,
}

impl Plan {
    /// True for an address in the range that a device can have.
    ///
    /// A network's first address stands for the network and its last one
    /// reaches everybody on it, so neither is a device. Which two those are
    /// is decided by the network this PC is really on, not by the range: in
    /// the /22 cut out of a bigger network both ends are ordinary addresses,
    /// and a typed range wider than the network has the network's two in its
    /// middle. Only where this PC is on no such network does the range decide.
    pub fn is_host(&self, ip: Ipv4Addr) -> bool {
        if !self.range.contains(&ip) {
            return false;
        }
        let mut on_link = false;
        for net in self.links.iter().map(|link| &link.net).filter(|net| net.contains(&ip)) {
            if !is_host_of(net, ip) {
                return false;
            }
            on_link = true;
        }
        on_link || is_host_of(&self.range, ip)
    }

    /// Every address in the range a device can have, in order.
    pub fn hosts(&self) -> impl Iterator<Item = Ipv4Addr> + '_ {
        let (first, last) = (u32::from(self.range.network()), u32::from(self.range.broadcast()));
        (first..=last).map(Ipv4Addr::from).filter(|ip| self.is_host(*ip))
    }
}

fn overlap(a: &Ipv4Net, b: &Ipv4Net) -> bool {
    a.contains(&b.network()) || b.contains(&a.network())
}

/// Plan a scan of `range`, or of the best candidate's network when it is
/// None. None means there is no home network to scan.
pub fn plan(interfaces: &[Interface], range: Option<Ipv4Net>) -> Option<Plan> {
    let range = match range {
        Some(range) => range,
        None => rank(interfaces).first()?.scan_net,
    };
    // Not only the adapters the ranking likes: a typed range may be on one it
    // leaves out (a virtual machine network, say).
    let mut links: Vec<Link> = Vec::new();
    for iface in interfaces.iter().filter(|i| i.is_up() && !i.is_loopback() && !i.is_point_to_point()) {
        for addr in iface.ipv4.iter().filter(|a| a.prefix_len() <= 30 && overlap(&a.trunc(), &range)) {
            let adapter = Adapter { index: iface.index, name: iface.name.clone() };
            let link = Link { adapter, net: addr.trunc() };
            if !links.contains(&link) {
                links.push(link);
            }
        }
    }
    let mut own: Vec<(Ipv4Addr, Option<[u8; 6]>)> = Vec::new();
    for iface in interfaces {
        for addr in &iface.ipv4 {
            if range.contains(&addr.addr()) && !own.iter().any(|(ip, _)| *ip == addr.addr()) {
                own.push((addr.addr(), iface.mac_addr.map(|m| m.octets()).filter(|m| *m != [0; 6])));
            }
        }
    }
    Some(Plan { range, links, own })
}

/// Read a range the owner typed. Ok(None) for empty text, which means "work
/// it out". The error text is shown to the owner as it is.
pub fn parse_range(text: &str) -> Result<Option<Ipv4Net>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let Ok(net) = text.parse::<Ipv4Net>() else {
        let shown: String = text.chars().filter(|c| !c.is_control()).take(40).collect();
        return Err(format!("\"{shown}\" is not a network range. It should look like 192.168.1.0/24."));
    };
    let net = net.trunc();
    if net.prefix_len() < BIGGEST_PREFIX {
        return Err(format!(
            "That range is too big to scan. Use a number from {BIGGEST_PREFIX} to 32 after the slash, \
             such as 192.168.1.0/24."
        ));
    }
    // With a /20 or smaller, a private first address means all of it is private.
    if !net.network().is_private() {
        return Err("Only home network ranges can be scanned: 192.168.x.x, 10.x.x.x, \
                    or 172.16.x.x to 172.31.x.x."
            .into());
    }
    Ok(Some(net))
}

/// True for an address a device on the network `range` can have: not its
/// first (network) or last (broadcast) address.
fn is_host_of(range: &Ipv4Net, ip: Ipv4Addr) -> bool {
    range.contains(&ip) && (range.prefix_len() >= 31 || (ip != range.network() && ip != range.broadcast()))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use netdev::NetworkDevice;

    // The flag values netdev uses on Linux. On Windows loopback is 0x4 and
    // point-to-point 0x8, so fixtures for that system need different numbers.
    pub const UP: u32 = 0x1;
    pub const BROADCAST: u32 = 0x2;
    const LOOPBACK: u32 = 0x8;
    const POINT_TO_POINT: u32 = 0x10;

    const ETHERNET: InterfaceType = InterfaceType::Ethernet;

    pub fn iface(
        name: &str,
        label: Option<(&str, &str)>,
        kind: InterfaceType,
        flags: u32,
        cidr: &str,
        gateway: Option<&str>,
    ) -> Interface {
        let mut i = Interface::dummy();
        i.index = name.bytes().map(u32::from).sum();
        i.name = name.into();
        i.friendly_name = label.map(|(friendly, _)| friendly.into());
        i.description = label.map(|(_, description)| description.into());
        i.if_type = kind;
        i.flags = flags;
        i.mac_addr = Some(netdev::MacAddr::new(2, 0, 0, 0, 0, 7));
        i.ipv4 = vec![cidr.parse().unwrap()];
        i.gateway = gateway.map(|g| {
            let mut device = NetworkDevice::new();
            device.ipv4 = vec![g.parse().unwrap()];
            device
        });
        i
    }

    fn linux_pc() -> Vec<Interface> {
        vec![
            iface("lo", None, InterfaceType::Loopback, UP | LOOPBACK, "127.0.0.1/8", None),
            iface("docker0", None, ETHERNET, UP | BROADCAST, "172.17.0.1/16", None),
            iface("br-3f2a", None, ETHERNET, UP | BROADCAST, "172.18.0.1/16", None),
            iface(
                "tailscale0",
                None,
                InterfaceType::UnknownWithValue(65534),
                UP | POINT_TO_POINT,
                "100.69.245.79/32",
                None,
            ),
            iface("eth0", None, ETHERNET, UP | BROADCAST, "192.168.68.78/22", Some("192.168.68.1")),
        ]
    }

    fn windows_pc() -> Vec<Interface> {
        let hyper_v = "Hyper-V Virtual Ethernet Adapter";
        vec![
            iface(
                "{A}",
                Some(("vEthernet (Default Switch)", hyper_v)),
                ETHERNET,
                UP | BROADCAST,
                "172.29.96.1/20",
                None,
            ),
            iface(
                "{B}",
                Some(("vEthernet (WSL (Hyper-V firewall))", hyper_v)),
                ETHERNET,
                UP | BROADCAST,
                "172.22.0.1/20",
                None,
            ),
            iface(
                "{C}",
                Some(("Ethernet 3", "VirtualBox Host-Only Ethernet Adapter")),
                ETHERNET,
                UP | BROADCAST,
                "192.168.56.1/24",
                None,
            ),
            iface(
                "{D}",
                Some(("Tailscale", "Tailscale Tunnel")),
                InterfaceType::ProprietaryVirtual,
                UP,
                "100.101.102.103/32",
                None,
            ),
            iface(
                "{E}",
                Some(("OpenVPN TAP", "TAP-Windows Adapter V9")),
                ETHERNET,
                UP | BROADCAST,
                "10.8.0.6/24",
                Some("10.8.0.1"),
            ),
            iface(
                "{F}",
                Some(("Ethernet 2", "Realtek USB GbE")),
                ETHERNET,
                UP | BROADCAST,
                "192.168.50.4/24",
                None,
            ),
            iface(
                "{G}",
                Some(("Wi-Fi", "Intel(R) Wi-Fi 7 BE200 320MHz")),
                InterfaceType::Wireless80211,
                UP | BROADCAST,
                "192.168.68.20/22",
                Some("192.168.68.1"),
            ),
        ]
    }

    #[test]
    fn linux_pc_has_one_candidate() {
        let ranked = rank(&linux_pc());
        assert_eq!(ranked.len(), 1);
        let c = &ranked[0];
        assert_eq!((c.label.as_str(), c.own_ip.to_string().as_str(), c.score), ("eth0", "192.168.68.78", 3));
        assert_eq!(c.scan_net.to_string(), "192.168.68.0/22");
    }

    #[test]
    fn windows_pc_ranks_real_network_first_and_drops_virtual_ones() {
        let ranked = rank(&windows_pc());
        let seen: Vec<(&str, u8)> = ranked.iter().map(|c| (c.label.as_str(), c.score)).collect();
        // The home Wi-Fi, then a VPN that has a router, then a cable with no router.
        assert_eq!(seen, [("Wi-Fi", 3), ("OpenVPN TAP", 2), ("Ethernet 2", 1)]);
    }

    #[test]
    fn hyper_v_external_switch_is_still_found() {
        // With a Hyper-V external switch the home address moves to a vEthernet adapter.
        let label = Some(("vEthernet (External)", "Hyper-V Virtual Ethernet Adapter"));
        let list = [iface("{A}", label, ETHERNET, UP | BROADCAST, "192.168.68.20/22", Some("192.168.68.1"))];
        let ranked = rank(&list);
        assert_eq!(ranked.iter().map(|c| c.score).collect::<Vec<_>>(), [2]);
    }

    #[test]
    fn adapters_that_are_down_or_have_no_home_address_are_left_out() {
        let list = [
            iface("eth0", None, ETHERNET, BROADCAST, "192.168.1.5/24", Some("192.168.1.1")),
            iface("eth1", None, ETHERNET, UP | BROADCAST, "8.8.4.4/24", Some("8.8.4.1")),
            iface("eth2", None, ETHERNET, UP | BROADCAST, "169.254.3.4/16", None),
            iface("eth3", None, ETHERNET, UP | BROADCAST, "192.168.9.9/32", None),
        ];
        assert!(rank(&list).is_empty());
        assert_eq!(plan(&list, None), None);
    }

    #[test]
    fn big_network_is_cut_to_the_part_around_this_pc() {
        let list = [iface("eth0", None, ETHERNET, UP | BROADCAST, "10.0.5.9/8", Some("10.0.0.1"))];
        let ranked = rank(&list);
        assert_eq!(ranked[0].scan_net.to_string(), "10.0.4.0/22");
        // The router is outside the part that is scanned; the score still counts it.
        assert_eq!(ranked[0].score, 3);
    }

    /// Each link of a plan as "adapter network".
    fn links(plan: &Plan) -> Vec<String> {
        plan.links.iter().map(|link| format!("{} {}", link.adapter.name, link.net)).collect()
    }

    fn hosts(plan: &Plan) -> Vec<String> {
        plan.hosts().map(|ip| ip.to_string()).collect()
    }

    #[test]
    fn plan_without_a_range_takes_the_best_network() {
        let p = plan(&linux_pc(), None).unwrap();
        assert_eq!(p.range.to_string(), "192.168.68.0/22");
        assert_eq!(links(&p), ["eth0 192.168.68.0/22"]);
        assert_eq!(p.own, [("192.168.68.78".parse().unwrap(), Some([2, 0, 0, 0, 0, 7]))]);
    }

    #[test]
    fn plan_for_a_typed_range_finds_its_adapter() {
        let on = |range: &str| plan(&linux_pc(), Some(range.parse().unwrap())).unwrap();
        // Part of the home network.
        let p = on("192.168.70.0/24");
        assert_eq!((links(&p), p.own.len()), (vec!["eth0 192.168.68.0/22".to_string()], 0));
        // A network the ranking leaves out is still matched to its adapter.
        let p = on("172.17.0.0/24");
        assert_eq!(links(&p), ["docker0 172.17.0.0/16"]);
        assert_eq!(p.own.iter().map(|(ip, _)| ip.to_string()).collect::<Vec<_>>(), ["172.17.0.1"]);
        // Somewhere only a router can reach: no adapter, nothing of our own in it.
        let p = on("10.20.30.0/24");
        assert_eq!((p.links.len(), p.own.len()), (0, 0));
    }

    /// A docked laptop: Wi-Fi and a cable on the same network. The system
    /// lists Wi-Fi first but sends through the cable.
    fn docked_laptop() -> Vec<Interface> {
        let mut wifi = iface(
            "wlan0",
            None,
            InterfaceType::Wireless80211,
            UP | BROADCAST,
            "192.168.68.20/22",
            Some("192.168.68.1"),
        );
        wifi.mac_addr = Some(netdev::MacAddr::new(2, 0, 0, 0, 0, 8));
        let mut cable =
            iface("eth1", None, ETHERNET, UP | BROADCAST, "192.168.68.21/22", Some("192.168.68.1"));
        cable.default = true;
        vec![wifi, cable]
    }

    #[test]
    fn of_two_adapters_on_one_network_the_one_the_system_sends_through_is_best() {
        let ranked = rank(&docked_laptop());
        let seen: Vec<(&str, u8)> = ranked.iter().map(|c| (c.label.as_str(), c.score)).collect();
        assert_eq!(seen, [("eth1", 3), ("wlan0", 3)]);
        // Without that hint the system's order stands.
        let mut unknown = docked_laptop();
        unknown.iter_mut().for_each(|i| i.default = false);
        assert_eq!(rank(&unknown).iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["wlan0", "eth1"]);
    }

    #[test]
    fn plan_keeps_every_adapter_on_the_range() {
        let p = plan(&docked_laptop(), None).unwrap();
        assert_eq!(links(&p), ["wlan0 192.168.68.0/22", "eth1 192.168.68.0/22"]);
        // Both of this PC's addresses are its own, each with its adapter's hardware address.
        let own = [
            ("192.168.68.20".parse().unwrap(), Some([2, 0, 0, 0, 0, 8])),
            ("192.168.68.21".parse().unwrap(), Some([2, 0, 0, 0, 0, 7])),
        ];
        assert_eq!(p.own, own);
        // An adapter that is down is not on the range.
        let mut list = docked_laptop();
        list[0].flags = BROADCAST;
        assert_eq!(links(&plan(&list, None).unwrap()), ["eth1 192.168.68.0/22"]);
    }

    #[test]
    fn a_range_that_is_the_network_leaves_out_its_two_ends() {
        let list = [iface("eth0", None, ETHERNET, UP | BROADCAST, "192.168.1.5/29", Some("192.168.1.1"))];
        let p = plan(&list, None).unwrap();
        // 192.168.1.1 to .6: not .0, which stands for the network, nor .7, which reaches everybody.
        assert_eq!(hosts(&p), (1..=6).map(|n| format!("192.168.1.{n}")).collect::<Vec<_>>());
        assert!(!p.is_host("192.168.1.8".parse().unwrap()));
    }

    #[test]
    fn a_range_inside_a_bigger_network_has_devices_at_both_ends() {
        // The part the app picks by itself on a big network.
        let list = [iface("eth0", None, ETHERNET, UP | BROADCAST, "10.0.5.9/16", Some("10.0.0.1"))];
        let p = plan(&list, None).unwrap();
        assert_eq!(p.range.to_string(), "10.0.4.0/22");
        assert_eq!(links(&p), ["eth0 10.0.0.0/16"]);
        let all = hosts(&p);
        assert_eq!(all.len(), 1024);
        assert_eq!((all.first().unwrap().as_str(), all.last().unwrap().as_str()), ("10.0.4.0", "10.0.7.255"));

        // A typed part of the home network: every address of it is a device's.
        let part = plan(&linux_pc(), Some("192.168.69.0/30".parse().unwrap())).unwrap();
        assert_eq!(hosts(&part), ["192.168.69.0", "192.168.69.1", "192.168.69.2", "192.168.69.3"]);
        // Unless it is the network's own first or last address.
        let start = plan(&linux_pc(), Some("192.168.68.0/30".parse().unwrap())).unwrap();
        assert_eq!(hosts(&start), ["192.168.68.1", "192.168.68.2", "192.168.68.3"]);
        let end = plan(&linux_pc(), Some("192.168.71.252/30".parse().unwrap())).unwrap();
        assert_eq!(hosts(&end), ["192.168.71.252", "192.168.71.253", "192.168.71.254"]);
    }

    #[test]
    fn a_range_wider_than_the_network_leaves_out_the_networks_ends_too() {
        let list = [iface("eth0", None, ETHERNET, UP | BROADCAST, "192.168.1.5/24", Some("192.168.1.1"))];
        let p = plan(&list, Some("192.168.0.0/22".parse().unwrap())).unwrap();
        let host = |ip: &str| p.is_host(ip.parse().unwrap());
        // The real network's first and last address, in the middle of the range.
        assert!(!host("192.168.1.0") && !host("192.168.1.255"));
        // The range's own ends, as before: nothing better is known about them.
        assert!(!host("192.168.0.0") && !host("192.168.3.255"));
        // Everything else, on the network or behind the router.
        assert!(host("192.168.1.1") && host("192.168.1.254") && host("192.168.0.255") && host("192.168.2.0"));
        assert_eq!(p.hosts().count(), 1024 - 4);
    }

    #[test]
    fn a_range_behind_a_router_is_judged_by_its_own_ends() {
        let p = plan(&linux_pc(), Some("10.20.30.0/30".parse().unwrap())).unwrap();
        assert_eq!(hosts(&p), ["10.20.30.1", "10.20.30.2"]);
        // One or two addresses have no ends to leave out.
        let pair = plan(&linux_pc(), Some("10.20.30.0/31".parse().unwrap())).unwrap();
        assert_eq!(hosts(&pair), ["10.20.30.0", "10.20.30.1"]);
        let single = plan(&linux_pc(), Some("192.168.70.7/32".parse().unwrap())).unwrap();
        assert_eq!(hosts(&single), ["192.168.70.7"]);
    }

    #[test]
    fn own_ips_covers_every_adapter() {
        let own = own_ips(&linux_pc());
        for ip in ["127.0.0.1", "172.17.0.1", "100.69.245.79", "192.168.68.78"] {
            assert!(own.contains(&ip.parse::<Ipv4Addr>().unwrap()), "{ip}");
        }
    }

    #[test]
    fn parse_range_accepts_home_ranges() {
        let ok = |text: &str| parse_range(text).unwrap().map(|n| n.to_string());
        assert_eq!(ok(""), None);
        assert_eq!(ok("   "), None);
        assert_eq!(ok("192.168.68.0/22"), Some("192.168.68.0/22".into()));
        // An address inside the range works too; the range is what counts.
        assert_eq!(ok(" 192.168.68.78/22 "), Some("192.168.68.0/22".into()));
        assert_eq!(ok("10.1.2.3/20"), Some("10.1.0.0/20".into()));
        assert_eq!(ok("172.31.255.0/24"), Some("172.31.255.0/24".into()));
        assert_eq!(ok("192.168.1.7/32"), Some("192.168.1.7/32".into()));
    }

    #[test]
    fn parse_range_refuses_everything_else() {
        let err = |text: &str| parse_range(text).unwrap_err();
        for text in
            ["192.168.68.0", "192.168.68.0/33", "home", "192.168.1.x/24", "fe80::1/64", "/22", "1.2.3/24"]
        {
            assert!(err(text).contains("is not a network range"), "{text}");
        }
        for text in ["10.0.0.0/8", "192.168.0.0/16", "192.168.64.0/19"] {
            assert!(err(text).contains("too big"), "{text}");
        }
        for text in
            ["8.8.8.0/24", "172.32.0.0/24", "100.64.0.0/22", "169.254.0.0/24", "127.0.0.0/24", "0.0.0.0/20"]
        {
            assert!(err(text).contains("Only home network ranges"), "{text}");
        }
        // Odd characters and very long text are not echoed back whole.
        let long = err(&format!("{}\u{7}\n", "9".repeat(500)));
        assert!(long.len() < 120 && !long.contains('\u{7}') && !long.contains('\n'), "{long}");
    }

    #[test]
    fn host_addresses_leave_out_the_ends_of_the_range() {
        let range: Ipv4Net = "192.168.68.0/22".parse().unwrap();
        let host = |ip: &str| is_host_of(&range, ip.parse().unwrap());
        assert!(
            host("192.168.68.1") && host("192.168.71.254") && host("192.168.69.0") && host("192.168.69.255")
        );
        assert!(!host("192.168.68.0") && !host("192.168.71.255") && !host("192.168.72.1"));
        let single: Ipv4Net = "192.168.1.7/32".parse().unwrap();
        assert!(is_host_of(&single, "192.168.1.7".parse().unwrap()));
    }
}
