//! The Windows-style computer name of a device, asked for with a NetBIOS
//! "node status" question. Windows PCs with file sharing on, Samba, most NAS
//! boxes and many printers answer it. Replies come straight from other
//! devices, so every read is checked and nothing in here can panic.

use super::dns_wire::skip_name;
use std::net::Ipv4Addr;

/// Record type of a node status answer.
const NODE_STATUS: u16 = 0x21;
/// One entry of the name table: 15 name bytes, a kind byte and two flag bytes.
const ENTRY: usize = 18;

/// The question to send to a device's port 137: "list all your names". The
/// name asked for is "*" padded with zero bytes, in the two-letters-per-byte
/// spelling NetBIOS uses ('*' becomes "CK", zero becomes "AA").
pub fn status_query(ip: Ipv4Addr) -> Vec<u8> {
    let [.., last] = ip.octets();
    let id = 0x4800 | u16::from(last);
    let mut p = Vec::with_capacity(50);
    p.extend_from_slice(&id.to_be_bytes());
    p.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0, 0, 0]); // one question
    p.push(32);
    p.extend_from_slice(b"CKAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    p.push(0);
    p.extend_from_slice(&[0x00, 0x21, 0x00, 0x01]); // node status, class "internet"
    p
}

/// The computer name in a node status reply.
pub fn computer_name(reply: &[u8]) -> Option<String> {
    let u16_at = |pos: usize| Some(u16::from_be_bytes(reply.get(pos..pos.checked_add(2)?)?.try_into().ok()?));
    let is_reply = reply.get(2)? & 0x80 != 0;
    if !is_reply || u16_at(6)? == 0 {
        return None;
    }
    let mut pos = 12;
    // Windows and Samba repeat no question in the reply; skip any that a device does.
    for _ in 0..u16_at(4)? {
        pos = skip_name(reply, pos)?.checked_add(4)?;
    }
    pos = skip_name(reply, pos)?; // the answer's own name
    if u16_at(pos)? != NODE_STATUS {
        return None;
    }
    pos = pos.checked_add(10)?; // type, class, lifetime, length
    let count = usize::from(*reply.get(pos)?);
    let table = reply.get(pos.checked_add(1)?..)?;

    // Kind 0x00 is the computer itself and 0x20 its file sharing; both carry
    // the computer name. Group entries are the workgroup, not the PC.
    let mut sharing_name = None;
    for entry in table.chunks_exact(ENTRY).take(count) {
        let (Some(name), Some(&kind), Some(&flags)) = (entry.get(..15), entry.get(15), entry.get(16)) else {
            continue;
        };
        let is_group = flags & 0x80 != 0;
        if is_group || !matches!(kind, 0x00 | 0x20) {
            continue;
        }
        let Some(name) = readable(name) else { continue };
        if kind == 0x00 {
            return Some(name);
        }
        sharing_name.get_or_insert(name);
    }
    sharing_name
}

/// Names are in the PC's own old-style character set; in practice plain
/// letters and digits. Anything else is shown as "?".
fn readable(name: &[u8]) -> Option<String> {
    let text: String = name.iter().map(|&b| if b.is_ascii_graphic() || b == b' ' { char::from(b) } else { '?' }).collect();
    let text = text.trim();
    text.chars().any(|c| c.is_ascii_alphanumeric()).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reply in the shape Windows sends: no question, one answer, the name
    /// table, then the adapter's hardware address and counters.
    fn reply(questions: u8, entries: &[(&[u8], u8, u8)]) -> Vec<u8> {
        let mut p = vec![0x48, 0x01, 0x84, 0x00, 0, questions, 0, 1, 0, 0, 0, 0];
        for _ in 0..questions {
            p.push(32);
            p.extend_from_slice(b"CKAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
            p.extend_from_slice(&[0, 0x00, 0x21, 0x00, 0x01]);
        }
        p.push(32);
        p.extend_from_slice(b"CKAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
        p.extend_from_slice(&[0, 0x00, 0x21, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
        p.push(entries.len() as u8);
        for (name, kind, flags) in entries {
            let mut padded = name.to_vec();
            padded.resize(15, b' ');
            p.extend_from_slice(&padded);
            p.extend_from_slice(&[*kind, *flags, 0]);
        }
        p.extend_from_slice(&[0x00, 0x15, 0x5d, 1, 2, 3]);
        p
    }

    #[test]
    fn question_is_the_standard_one() {
        let q = status_query(Ipv4Addr::new(192, 168, 68, 20));
        assert_eq!(q.len(), 50);
        assert_eq!(&q[..2], &[0x48, 20]);
        assert_eq!(skip_name(&q, 12), Some(46));
        // A question is not a reply.
        assert_eq!(computer_name(&q), None);
    }

    #[test]
    fn windows_reply() {
        let p = reply(0, &[(b"NAS-PC", 0x00, 0x04), (b"WORKGROUP", 0x00, 0x84), (b"NAS-PC", 0x20, 0x04)]);
        assert_eq!(computer_name(&p).as_deref(), Some("NAS-PC"));
    }

    #[test]
    fn reply_with_echoed_question() {
        let p = reply(1, &[(b"SAMBA-BOX", 0x00, 0x04)]);
        assert_eq!(computer_name(&p).as_deref(), Some("SAMBA-BOX"));
    }

    #[test]
    fn workgroup_alone_is_not_a_computer_name() {
        let p = reply(0, &[(b"WORKGROUP", 0x00, 0x84), (b"\x01\x02__MSBROWSE__\x02", 0x01, 0x84)]);
        assert_eq!(computer_name(&p), None);
    }

    #[test]
    fn file_sharing_name_is_the_fallback() {
        let p = reply(0, &[(b"WORKGROUP", 0x00, 0x84), (b"PRINTER-1", 0x20, 0x04)]);
        assert_eq!(computer_name(&p).as_deref(), Some("PRINTER-1"));
        let p = reply(0, &[(b"OLD-NAME", 0x20, 0x04), (b"REUBEN-PC", 0x00, 0x04)]);
        assert_eq!(computer_name(&p).as_deref(), Some("REUBEN-PC"));
    }

    #[test]
    fn unreadable_names() {
        // Not plain letters: shown with "?" where it can be, dropped when nothing is left.
        let p = reply(0, &[(&[0x8F, 0x8A, b'-', b'P', b'C'], 0x00, 0x04)]);
        assert_eq!(computer_name(&p).as_deref(), Some("??-PC"));
        let p = reply(0, &[(&[0x8F, 0x8A, 0x00, 0x07], 0x00, 0x04)]);
        assert_eq!(computer_name(&p), None);
        let p = reply(0, &[(b"", 0x00, 0x04)]);
        assert_eq!(computer_name(&p), None);
    }

    #[test]
    fn garbage_never_panics() {
        let good = reply(1, &[(b"NAS-PC", 0x00, 0x04), (b"WORKGROUP", 0x00, 0x84), (b"NAS-PC", 0x20, 0x04)]);
        for n in 0..=good.len() {
            let _ = computer_name(&good[..n]);
        }
        for i in 0..good.len() {
            for value in [0x00, 0x3F, 0x40, 0x80, 0xC0, 0xFF] {
                let mut bad = good.clone();
                bad[i] = value;
                let _ = computer_name(&bad);
            }
        }
        // Claims 65535 questions, then a pointer and nothing else.
        assert_eq!(computer_name(&[0x48, 0, 0x84, 0, 0xFF, 0xFF, 0, 1, 0, 0, 0, 0, 0xC0]), None);
        // Claims 255 names but holds one: the one that is there still counts.
        let mut short = reply(0, &[(b"NAS-PC", 0x00, 0x04)]);
        let count_at = short.len() - 6 - ENTRY - 1;
        short[count_at] = 255;
        assert_eq!(computer_name(&short).as_deref(), Some("NAS-PC"));
        // An mDNS-style reply sent to the wrong parser.
        assert_eq!(computer_name(&[0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, b'a', 0, 0, 12, 0, 1]), None);
    }
}
