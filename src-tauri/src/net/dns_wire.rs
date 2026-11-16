//! Just enough of the DNS message format to ask a device for its own name
//! over mDNS and to read the answer. Replies come straight from other
//! devices, so every read is checked and nothing in here can panic.

use std::net::Ipv4Addr;

const TYPE_A: u16 = 1;
const TYPE_PTR: u16 = 12;
/// The longest name DNS allows, in bytes. Anything longer is cut off.
const MAX_NAME: usize = 255;
/// Labels and pointer hops followed in one name before giving up, so a
/// pointer that points at itself cannot hang the app.
const MAX_HOPS: usize = 64;
/// Records read from one reply. A real answer holds a handful.
const MAX_RECORDS: usize = 64;

const SERVICE_LIST: &str = "_services._dns-sd._udp.local";

/// The question sent to a device's mDNS port: "what name goes with your
/// address?" plus "which services do you offer?". The second one makes some
/// devices add their address record, which also carries the name.
pub fn mdns_query(ip: Ipv4Addr) -> Vec<u8> {
    query(0x4843, &[(&reverse_name(ip), TYPE_PTR), (SERVICE_LIST, TYPE_PTR)])
}

/// The host name in one mDNS reply from `ip`, without ".local".
pub fn mdns_host(reply: &[u8], ip: Ipv4Addr) -> Option<String> {
    let records = records(reply);
    let reverse = reverse_name(ip);
    let pointer = records
        .iter()
        .filter(|r| r.rtype == TYPE_PTR && r.name.eq_ignore_ascii_case(&reverse))
        .find_map(|r| read_name(reply, r.data_at));
    // An address record for this very device names it too.
    let address = || records.iter().find(|r| r.rtype == TYPE_A && r.data == ip.octets()).map(|r| r.name.clone());
    let name = strip_local(&pointer.or_else(address)?);
    (!name.is_empty()).then_some(name)
}

/// Position just after the name that starts at `pos`.
pub fn skip_name(buf: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        let len = usize::from(*buf.get(pos)?);
        match len & 0xC0 {
            // A pointer to an earlier name: two bytes, and the name ends here.
            0xC0 => return pos.checked_add(2).filter(|end| *end <= buf.len()),
            0 => pos = pos.checked_add(1 + len)?,
            _ => return None,
        }
        if len == 0 {
            return Some(pos);
        }
    }
}

fn reverse_name(ip: Ipv4Addr) -> String {
    let [a, b, c, d] = ip.octets();
    format!("{d}.{c}.{b}.{a}.in-addr.arpa")
}

fn strip_local(name: &str) -> String {
    let name = name.trim_end_matches('.');
    let cut = name.len().checked_sub(".local".len());
    // `get` gives None when the cut would land inside a character.
    match cut.and_then(|at| name.get(at..).zip(name.get(..at))) {
        Some((end, start)) if end.eq_ignore_ascii_case(".local") => start.to_string(),
        _ => name.to_string(),
    }
}

fn query(id: u16, questions: &[(&str, u16)]) -> Vec<u8> {
    let mut p = Vec::with_capacity(96);
    p.extend_from_slice(&id.to_be_bytes());
    p.extend_from_slice(&[0, 0]); // an ordinary question
    p.extend_from_slice(&u16::try_from(questions.len()).unwrap_or(u16::MAX).to_be_bytes());
    p.extend_from_slice(&[0; 6]);
    for (name, qtype) in questions {
        put_name(&mut p, name);
        p.extend_from_slice(&qtype.to_be_bytes());
        p.extend_from_slice(&[0, 1]); // class "internet"
    }
    p
}

fn put_name(p: &mut Vec<u8>, name: &str) {
    for label in name.trim_end_matches('.').split('.') {
        let bytes = label.as_bytes();
        // A label holds 63 bytes at most.
        let bytes = bytes.get(..63).unwrap_or(bytes);
        p.push(u8::try_from(bytes.len()).unwrap_or(0));
        p.extend_from_slice(bytes);
    }
    p.push(0);
}

fn u16_at(buf: &[u8], pos: usize) -> Option<u16> {
    let pair = buf.get(pos..pos.checked_add(2)?)?;
    Some(u16::from_be_bytes(pair.try_into().ok()?))
}

/// Reads a name, following pointers to earlier parts of the message.
fn read_name(buf: &[u8], mut pos: usize) -> Option<String> {
    let mut out = String::new();
    for _ in 0..MAX_HOPS {
        let len = usize::from(*buf.get(pos)?);
        if len == 0 {
            return Some(out);
        }
        match len & 0xC0 {
            0xC0 => pos = ((len & 0x3F) << 8) | usize::from(*buf.get(pos.checked_add(1)?)?),
            0 => {
                let start = pos.checked_add(1)?;
                let end = start.checked_add(len)?;
                let label = buf.get(start..end)?;
                if out.len() + label.len() >= MAX_NAME {
                    return None;
                }
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(&String::from_utf8_lossy(label));
                pos = end;
            }
            _ => return None,
        }
    }
    None
}

struct Record<'a> {
    name: String,
    rtype: u16,
    /// Where the record's data starts in the whole message: a name in the
    /// data may point back into the message.
    data_at: usize,
    data: &'a [u8],
}

/// The records of a reply. One that is cut short gives the records before
/// the cut; anything that is not a reply gives none.
fn records(buf: &[u8]) -> Vec<Record<'_>> {
    let mut out = Vec::new();
    let is_reply = buf.get(2).is_some_and(|flags| flags & 0x80 != 0);
    let counts = (u16_at(buf, 4), u16_at(buf, 6), u16_at(buf, 8), u16_at(buf, 10));
    let (true, (Some(questions), Some(answers), Some(authority), Some(extra))) = (is_reply, counts) else {
        return out;
    };
    let mut pos = 12;
    for _ in 0..questions {
        let Some(next) = skip_name(buf, pos).and_then(|p| p.checked_add(4)) else { return out };
        pos = next;
    }
    let total = usize::from(answers) + usize::from(authority) + usize::from(extra);
    for _ in 0..total.min(MAX_RECORDS) {
        let Some(record) = record_at(buf, pos) else { break };
        pos = record.data_at + record.data.len();
        out.push(record);
    }
    out
}

fn record_at(buf: &[u8], pos: usize) -> Option<Record<'_>> {
    let name = read_name(buf, pos)?;
    let pos = skip_name(buf, pos)?;
    // type, class and lifetime, then the length of the data
    let rtype = u16_at(buf, pos)?;
    let len = usize::from(u16_at(buf, pos.checked_add(8)?)?);
    let data_at = pos.checked_add(10)?;
    let data = buf.get(data_at..data_at.checked_add(len)?)?;
    Some(Record { name, rtype, data_at, data })
}

#[cfg(test)]
mod tests {
    use super::*;

    const IP: Ipv4Addr = Ipv4Addr::new(192, 168, 68, 20);

    /// A reply shaped like the ones Linux and Apple devices send: the
    /// question echoed, then the answer with its name as a pointer to it.
    fn reply(host: &str) -> Vec<u8> {
        let mut p = query(0x4843, &[(&reverse_name(IP), TYPE_PTR)]);
        p[2] = 0x84;
        p[7] = 1;
        p.extend_from_slice(&[0xC0, 12, 0, 12, 0x80, 1, 0, 0, 0, 120]);
        let mut target = Vec::new();
        put_name(&mut target, host);
        p.extend_from_slice(&(target.len() as u16).to_be_bytes());
        p.extend_from_slice(&target);
        p
    }

    #[test]
    fn query_asks_for_the_address_and_the_services() {
        let q = mdns_query(IP);
        assert_eq!(&q[..12], &[0x48, 0x43, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0]);
        assert_eq!(read_name(&q, 12).as_deref(), Some("20.68.168.192.in-addr.arpa"));
        let second = skip_name(&q, 12).unwrap() + 4;
        assert_eq!(read_name(&q, second).as_deref(), Some(SERVICE_LIST));
        assert_eq!(skip_name(&q, second).unwrap() + 4, q.len());
    }

    #[test]
    fn host_from_pointer_answer() {
        assert_eq!(mdns_host(&reply("homelab-server.local"), IP).as_deref(), Some("homelab-server"));
        assert_eq!(mdns_host(&reply("Emilys-iPad.LOCAL."), IP).as_deref(), Some("Emilys-iPad"));
        assert_eq!(mdns_host(&reply("nas.lan"), IP).as_deref(), Some("nas.lan"));
        // An answer about some other address is not this device's name.
        assert_eq!(mdns_host(&reply("homelab-server.local"), Ipv4Addr::new(192, 168, 68, 21)), None);
        // A question is not an answer.
        assert_eq!(mdns_host(&mdns_query(IP), IP), None);
    }

    #[test]
    fn host_from_address_record() {
        let mut p = vec![0, 0, 0x84, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        put_name(&mut p, "Living-Room-TV.local");
        p.extend_from_slice(&[0, 1, 0x80, 1, 0, 0, 0, 120, 0, 4]);
        p.extend_from_slice(&IP.octets());
        assert_eq!(mdns_host(&p, IP).as_deref(), Some("Living-Room-TV"));
        assert_eq!(mdns_host(&p, Ipv4Addr::new(192, 168, 68, 21)), None);
    }

    #[test]
    fn cut_short_reply_keeps_the_records_before_the_cut() {
        let mut p = reply("pihole1.local");
        p[7] = 3; // claims two more answers than it holds
        assert_eq!(records(&p).len(), 1);
        assert_eq!(mdns_host(&p, IP).as_deref(), Some("pihole1"));
    }

    #[test]
    fn garbage_never_panics() {
        let good = reply("homelab-server.local");
        for n in 0..=good.len() {
            let _ = mdns_host(&good[..n], IP);
            let _ = skip_name(&good[..n], 12);
        }
        // Every single-byte change of a good reply.
        for i in 0..good.len() {
            for value in [0x00, 0x3F, 0x40, 0x80, 0xC0, 0xFF] {
                let mut bad = good.clone();
                bad[i] = value;
                let _ = mdns_host(&bad, IP);
            }
        }
        // Header that promises 65535 of everything, with nothing behind it.
        let _ = mdns_host(&[0, 0, 0x84, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], IP);
        let _ = mdns_host(&[0, 0, 0x84, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xC0], IP);
        // Names that are not text.
        let mut odd = vec![0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0, 3, 0xFF, 0xFE, 0x00, 0];
        odd.extend_from_slice(&[0, 1, 0, 1, 0, 0, 0, 0, 0, 4]);
        odd.extend_from_slice(&IP.octets());
        assert!(mdns_host(&odd, IP).is_some());
    }

    #[test]
    fn name_pointer_loop_is_bounded() {
        assert_eq!(read_name(&[0xC0, 0x00], 0), None);
        assert_eq!(read_name(&[0xC0, 0x02, 0xC0, 0x00], 0), None);
        // A pointer past the end, and one whose second byte is missing.
        assert_eq!(read_name(&[0xC0, 0x09], 0), None);
        assert_eq!(read_name(&[0xC0], 0), None);
        assert_eq!(skip_name(&[0xC0], 0), None);
        assert_eq!(skip_name(&[0xC0, 0x00], 0), Some(2));
    }

    #[test]
    fn over_long_names_are_refused_or_cut() {
        // 100 labels chained by hand: more than a real name can hold.
        let mut p = Vec::new();
        for _ in 0..100 {
            p.extend_from_slice(&[3, b'a', b'b', b'c']);
        }
        p.push(0);
        assert_eq!(read_name(&p, 0), None);
        assert_eq!(skip_name(&p, 0), Some(p.len()));
        // Our own questions keep each label within what DNS allows.
        let long = "x".repeat(200);
        assert_eq!(query(1, &[(&long, TYPE_PTR)])[12], 63);
        let wide = "é".repeat(200);
        assert!(query(1, &[(&wide, TYPE_PTR)])[12] <= 63);
    }

    #[test]
    fn local_suffix() {
        assert_eq!(strip_local("printer.local."), "printer");
        assert_eq!(strip_local("printer"), "printer");
        assert_eq!(strip_local(".local"), "");
        assert_eq!(strip_local("local"), "local");
        assert_eq!(strip_local("éé"), "éé");
        assert_eq!(strip_local("ab\u{e9}local"), "ab\u{e9}local");
    }
}
