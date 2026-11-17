//! The operating system's neighbour table: which hardware address it has
//! seen for each IP address, and how sure it still is. Reading it needs no
//! special rights, and it is how devices that ignore ping are found.

use std::io;
use std::net::Ipv4Addr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeighState {
    /// The device answered the system a moment ago.
    Reachable,
    /// The address is remembered from earlier and not checked since.
    Unconfirmed,
    /// A fixed entry somebody typed in.
    Permanent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Neighbour {
    pub ip: Ipv4Addr,
    pub mac: [u8; 6],
    pub state: NeighState,
}

/// The table, for all adapters together: with two adapters on one network
/// only the system knows which of them it sends through, and the entries are
/// on that one. The same address can so be listed more than once. Entries the
/// system has no hardware address for are left out. Blocks briefly: call it
/// through `spawn_blocking`.
#[cfg(target_os = "linux")]
pub fn neighbours() -> io::Result<Vec<Neighbour>> {
    // The second way has no "how sure" detail, so it is only the fallback.
    netlink_neighbours().or_else(|_| Ok(parse_proc_arp(&std::fs::read_to_string("/proc/net/arp")?)))
}

/// Asks the kernel directly: no rights needed and no other program run.
#[cfg(target_os = "linux")]
fn netlink_neighbours() -> io::Result<Vec<Neighbour>> {
    use netlink_packet_core::{NLM_F_DUMP, NLM_F_REQUEST, NetlinkMessage, NetlinkPayload};
    use netlink_packet_route::neighbour::{
        NeighbourAddress, NeighbourAttribute, NeighbourMessage, NeighbourState,
    };
    use netlink_packet_route::{AddressFamily, RouteNetlinkMessage};
    use netlink_sys::{Socket, SocketAddr, protocols::NETLINK_ROUTE};

    let mut socket = Socket::new(NETLINK_ROUTE)?;
    socket.bind_auto()?;
    let mut body = NeighbourMessage::default();
    body.header.family = AddressFamily::Inet;
    let mut request = NetlinkMessage::from(RouteNetlinkMessage::GetNeighbour(body));
    request.header.flags = NLM_F_REQUEST | NLM_F_DUMP;
    request.header.sequence_number = 1;
    request.finalize();
    let mut bytes = vec![0u8; request.buffer_len()];
    request.serialize(&mut bytes);
    socket.send_to(&bytes, &SocketAddr::new(0, 0), 0)?;

    let mut out = Vec::new();
    loop {
        // One datagram holds several messages back to back.
        let (datagram, _) = socket.recv_from_full()?;
        let mut rest = datagram.as_slice();
        while !rest.is_empty() {
            let message =
                NetlinkMessage::<RouteNetlinkMessage>::deserialize(rest).map_err(io::Error::other)?;
            // Messages are padded to 4 bytes.
            let length = (message.header.length as usize).saturating_add(3) & !3;
            if length == 0 {
                return Err(io::Error::other("empty message from the kernel"));
            }
            rest = rest.get(length..).unwrap_or_default();
            match message.payload {
                NetlinkPayload::Done(_) => return Ok(out),
                NetlinkPayload::Error(e) => return Err(e.to_io()),
                NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewNeighbour(n)) => {
                    let state = match n.header.state {
                        NeighbourState::Reachable => NeighState::Reachable,
                        NeighbourState::Stale | NeighbourState::Delay | NeighbourState::Probe => {
                            NeighState::Unconfirmed
                        }
                        NeighbourState::Permanent => NeighState::Permanent,
                        // Still asking, or nobody answered: no hardware address.
                        _ => continue,
                    };
                    let (mut ip, mut mac) = (None, None);
                    for attribute in &n.attributes {
                        match attribute {
                            NeighbourAttribute::Destination(NeighbourAddress::Inet(v4)) => ip = Some(*v4),
                            NeighbourAttribute::LinkLayerAddress(b) => {
                                mac = <[u8; 6]>::try_from(b.as_slice()).ok()
                            }
                            _ => {}
                        }
                    }
                    if let (Some(ip), Some(mac)) = (ip, mac) {
                        out.push(Neighbour { ip, mac, state });
                    }
                }
                _ => {}
            }
        }
    }
}

/// Reads the text of /proc/net/arp. It does not say how fresh an entry is,
/// so every entry counts as remembered from earlier.
#[cfg(any(target_os = "linux", test))]
fn parse_proc_arp(text: &str) -> Vec<Neighbour> {
    const HAS_ADDRESS: u32 = 0x02;
    const PERMANENT: u32 = 0x04;
    let mut out = Vec::new();
    // Columns: IP address, HW type, Flags, HW address, Mask, Device.
    for line in text.lines().skip(1) {
        let columns: Vec<&str> = line.split_whitespace().collect();
        let [ip, _, flags, mac, _, _, ..] = columns.as_slice() else { continue };
        let Ok(ip) = ip.parse::<Ipv4Addr>() else { continue };
        let Ok(flags) = u32::from_str_radix(flags.trim_start_matches("0x"), 16) else { continue };
        let Some(mac) = parse_mac(mac) else { continue };
        if flags & HAS_ADDRESS == 0 {
            continue;
        }
        let state = if flags & PERMANENT != 0 { NeighState::Permanent } else { NeighState::Unconfirmed };
        out.push(Neighbour { ip, mac, state });
    }
    out
}

#[cfg(any(target_os = "linux", test))]
fn parse_mac(text: &str) -> Option<[u8; 6]> {
    let mut mac = [0u8; 6];
    let mut parts = text.split(':');
    for byte in &mut mac {
        *byte = u8::from_str_radix(parts.next()?, 16).ok()?;
    }
    parts.next().is_none().then_some(mac)
}

/// Windows keeps the same table; this is the call behind `arp -a`, without
/// running it and without needing administrator rights.
#[cfg(windows)]
pub fn neighbours() -> io::Result<Vec<Neighbour>> {
    use windows::Win32::Foundation::{ERROR_NOT_FOUND, NO_ERROR};
    use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIpNetTable2, MIB_IPNET_TABLE2};
    use windows::Win32::Networking::WinSock::{
        AF_INET, NlnsDelay, NlnsPermanent, NlnsProbe, NlnsReachable, NlnsStale,
    };

    let mut table: *mut MIB_IPNET_TABLE2 = std::ptr::null_mut();
    // SAFETY: on success the system allocates the table; it is freed below.
    let status = unsafe { GetIpNetTable2(AF_INET, &mut table) };
    if status == ERROR_NOT_FOUND {
        // What Windows answers when the table is empty.
        return Ok(Vec::new());
    }
    if status != NO_ERROR || table.is_null() {
        return Err(io::Error::from_raw_os_error(status.0 as i32));
    }
    let mut out = Vec::new();
    // SAFETY: Table is the first of NumEntries rows laid out one after another.
    let rows = unsafe { std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize) };
    for row in rows {
        if row.PhysicalAddressLength != 6 {
            continue;
        }
        let state = match row.State {
            s if s == NlnsReachable => NeighState::Reachable,
            s if s == NlnsStale || s == NlnsDelay || s == NlnsProbe => NeighState::Unconfirmed,
            s if s == NlnsPermanent => NeighState::Permanent,
            // Still asking, or nobody answered: no hardware address.
            _ => continue,
        };
        // SAFETY: only IPv4 rows were asked for. S_addr is in network byte order.
        let ip = Ipv4Addr::from(unsafe { row.Address.Ipv4.sin_addr.S_un.S_addr }.to_ne_bytes());
        let [a, b, c, d, e, f, ..] = row.PhysicalAddress;
        out.push(Neighbour { ip, mac: [a, b, c, d, e, f], state });
    }
    // SAFETY: the table came from GetIpNetTable2 and is not used after this.
    unsafe { FreeMibTable(table as *const _) };
    Ok(out)
}

/// Other systems: no table, so only devices that answer ping are found.
#[cfg(not(any(target_os = "linux", windows)))]
pub fn neighbours() -> io::Result<Vec<Neighbour>> {
    Ok(Vec::new())
}

/// True for the hardware address of one real device: not the "everyone" or
/// group addresses that Windows also keeps in its table, and not all zeros.
pub fn is_device_mac(mac: &[u8; 6]) -> bool {
    mac[0] & 1 == 0 && *mac != [0; 6]
}

/// Lower-case, colon-separated: "8c:90:2d:a0:0e:02".
pub fn mac_text(mac: &[u8; 6]) -> String {
    let [a, b, c, d, e, f] = mac;
    format!("{a:02x}:{b:02x}:{c:02x}:{d:02x}:{e:02x}:{f:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROC_ARP: &str = "\
IP address       HW type     Flags       HW address            Mask     Device
192.168.68.1     0x1         0x2         8c:90:2d:a0:0e:02     *        eth0
192.168.68.200   0x1         0x0         00:00:00:00:00:00     *        eth0
192.168.68.9     0x1         0x6         00:15:5d:44:35:01     *        eth0
172.17.0.2       0x1         0x2         02:42:ac:11:00:02     *        docker0
not an address   0x1         0x2         02:42:ac:11:00:02     *        eth0
192.168.68.7     0x1         0x2         02:42:ac:11:00       *        eth0
short line
";

    #[test]
    fn proc_arp_is_read_for_every_adapter() {
        assert_eq!(
            parse_proc_arp(PROC_ARP),
            [
                Neighbour {
                    ip: "192.168.68.1".parse().unwrap(),
                    mac: [0x8c, 0x90, 0x2d, 0xa0, 0x0e, 0x02],
                    state: NeighState::Unconfirmed
                },
                Neighbour {
                    ip: "192.168.68.9".parse().unwrap(),
                    mac: [0, 0x15, 0x5d, 0x44, 0x35, 1],
                    state: NeighState::Permanent
                },
                Neighbour {
                    ip: "172.17.0.2".parse().unwrap(),
                    mac: [0x02, 0x42, 0xac, 0x11, 0, 0x02],
                    state: NeighState::Unconfirmed
                },
            ]
        );
        assert!(parse_proc_arp("").is_empty());
    }

    #[test]
    fn mac_helpers() {
        assert_eq!(mac_text(&[0x8c, 0x90, 0x2d, 0xa0, 0x0e, 0x02]), "8c:90:2d:a0:0e:02");
        assert_eq!(parse_mac("8C:90:2d:a0:0e:02"), Some([0x8c, 0x90, 0x2d, 0xa0, 0x0e, 0x02]));
        assert_eq!(parse_mac("8c:90:2d:a0:0e"), None);
        assert_eq!(parse_mac("8c:90:2d:a0:0e:02:11"), None);
        assert!(is_device_mac(&[0x8c, 0x90, 0x2d, 0xa0, 0x0e, 0x02]));
        // Everyone, a group, and nothing.
        assert!(
            !is_device_mac(&[0xff; 6])
                && !is_device_mac(&[0x01, 0, 0x5e, 0, 0, 0xfb])
                && !is_device_mac(&[0; 6])
        );
    }

    /// The real table can be read without special rights. What is in it
    /// depends on the machine, so only the shape is checked.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_real_table_can_be_read() {
        neighbours().expect("neighbour table");
        netlink_neighbours().expect("neighbour table straight from the kernel");
    }
}
