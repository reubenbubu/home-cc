//! The maker of a device, from its hardware (MAC) address.
//!
//! The table comes from the public IEEE registries and is built into the app
//! (see tools/build_oui.py for the layout and how to refresh it). A lookup is
//! three binary searches over the raw bytes: nothing to load at start-up.

static DB: &[u8] = include_bytes!("../../data/oui.bin");

// A wrong or cut-short table stops the build. The running app still reads the
// table only through checked lookups, so even a bad one could not crash it.
const _: () = assert!(header_ok(DB), "src-tauri/data/oui.bin is damaged: rebuild it with tools/build_oui.py");

const HEADER: usize = 32;

/// Names the registry uses where it does not say who the maker is.
const PLACEHOLDERS: [&str; 2] = ["Private", "IEEE Registration Authority"];

/// Made-up address ranges that virtual machine software always uses, so they
/// can be labelled even though no maker registered them. Every entry is three
/// bytes long: a shorter one would also claim too many of the private
/// addresses that phones make up (about one in four million match one entry).
///
/// Docker builds an address from 02:42 and the container's own IP address,
/// so the third byte is the start of a home network range (10, 172 or 192).
const VIRTUAL: [([u8; 3], &str); 5] = [
    ([0x52, 0x54, 0x00], "QEMU/KVM"),
    ([0x02, 0x42, 0x0A], "Docker"),
    ([0x02, 0x42, 0xAC], "Docker"),
    ([0x02, 0x42, 0xC0], "Docker"),
    ([0x0A, 0x00, 0x27], "VirtualBox"),
];

/// Maker name for a hardware address in any common format
/// ("aa:bb:cc:dd:ee:ff", "AA-BB-..."). None when it is unknown, randomised
/// (phones and laptops often use a made-up address per network) or not a
/// real device address.
///
/// One exception: virtual machines and containers also use made-up addresses,
/// but from fixed ranges, so for those the answer is the name of the software
/// ("QEMU/KVM", "Docker", "VirtualBox") rather than of a maker, and
/// `is_randomised` is false.
pub fn vendor(mac: &str) -> Option<String> {
    let mac = parse_mac(mac)?;
    if !is_device(mac) {
        return None;
    }
    if is_local(mac) {
        return virtual_label(mac).map(str::to_string);
    }
    registered(mac).map(str::to_string)
}

/// True for an address a device made up for itself rather than one its maker
/// assigned. False for the fixed virtual machine ranges that `vendor` names.
pub fn is_randomised(mac: &str) -> bool {
    parse_mac(mac).is_some_and(|mac| is_device(mac) && is_local(mac) && virtual_label(mac).is_none())
}

/// True when the maker name is TP-Link in any of its spellings: the owner's
/// Deco mesh nodes are found this way.
pub fn looks_like_mesh(vendor: &str) -> bool {
    // Compares bytes: maker names are not all plain ASCII, and cutting a
    // `str` at a fixed position panics inside a multi-byte character.
    vendor.trim_start().as_bytes().get(..7).is_some_and(|start| start.eq_ignore_ascii_case(b"tp-link"))
}

/// Reads "8c:90:2d:a0:0e:02", "8C-90-2D-A0-0E-02", "8c90.2da0.0e02",
/// "8c902da00e02" and the short form some systems print ("8:0:27:1:2:3").
fn parse_mac(text: &str) -> Option<[u8; 6]> {
    let text = text.trim();
    let mut mac = [0u8; 6];
    if text.contains([':', '-']) {
        let mut groups = text.split([':', '-']);
        for byte in &mut mac {
            let group = groups.next()?;
            if group.len() > 2 {
                return None;
            }
            let mut digits = group.chars();
            *byte = hex(digits.next()?)?;
            if let Some(second) = digits.next() {
                *byte = (*byte << 4) | hex(second)?;
            }
        }
        return groups.next().is_none().then_some(mac);
    }
    // Dots only as three groups of four, so an IP address is never taken for one.
    if text.contains('.') && !(text.split('.').count() == 3 && text.split('.').all(|group| group.len() == 4)) {
        return None;
    }
    let mut count = 0;
    for c in text.chars().filter(|c| *c != '.') {
        let slot = mac.get_mut(count / 2)?;
        *slot = (*slot << 4) | hex(c)?;
        count += 1;
    }
    (count == 12).then_some(mac)
}

fn hex(c: char) -> Option<u8> {
    c.to_digit(16).and_then(|d| u8::try_from(d).ok())
}

/// False for all zero (a neighbour that never answered), broadcast and group
/// addresses. Windows lists those next to real devices.
fn is_device(mac: [u8; 6]) -> bool {
    mac[0] & 0x01 == 0 && mac != [0; 6]
}

/// True for an address that software chose, not a maker.
fn is_local(mac: [u8; 6]) -> bool {
    mac[0] & 0x02 != 0
}

fn virtual_label(mac: [u8; 6]) -> Option<&'static str> {
    VIRTUAL.iter().find(|(prefix, _)| mac.starts_with(prefix)).map(|(_, label)| *label)
}

/// The registered maker of a maker-assigned address.
fn registered(mac: [u8; 6]) -> Option<&'static str> {
    let table = Table::open(DB)?;
    let [a, b, c, d, e, _] = mac;
    // Longest block first: the small blocks sit inside a large one that the
    // registry lists only under its own name.
    let slot = find(table.keys36, [a, b, c, d, e & 0xF0])
        .map(|i| (table.names36, i))
        .or_else(|| find(table.keys28, [a, b, c, d & 0xF0]).map(|i| (table.names28, i)))
        .or_else(|| find(table.keys24, [a, b, c]).map(|i| (table.names24, i)));
    let (indexes, i) = slot?;
    let name = table.name(usize::from(u16_at(indexes, i.checked_mul(2)?)?))?;
    (!name.is_empty() && !PLACEHOLDERS.contains(&name)).then_some(name)
}

/// The parts of the table. Every part is cut out with checked arithmetic, so
/// a damaged file gives "unknown", never a crash.
struct Table<'a> {
    keys24: &'a [u8],
    keys28: &'a [u8],
    keys36: &'a [u8],
    names24: &'a [u8],
    names28: &'a [u8],
    names36: &'a [u8],
    offsets: &'a [u8],
    blob: &'a [u8],
}

impl<'a> Table<'a> {
    fn open(db: &'a [u8]) -> Option<Self> {
        let (n24, n28, n36, names) = (u32_at(db, 8)?, u32_at(db, 12)?, u32_at(db, 16)?, u32_at(db, 20)?);
        let mut rest = db.get(HEADER..)?;
        let mut take = |count: usize, width: usize| {
            let (part, tail) = rest.split_at_checked(count.checked_mul(width)?)?;
            rest = tail;
            Some(part)
        };
        Some(Table {
            keys24: take(n24, 3)?,
            keys28: take(n28, 4)?,
            keys36: take(n36, 5)?,
            names24: take(n24, 2)?,
            names28: take(n28, 2)?,
            names36: take(n36, 2)?,
            offsets: take(names.checked_add(1)?, 4)?,
            blob: rest,
        })
    }

    fn name(&self, index: usize) -> Option<&'a str> {
        let at = index.checked_mul(4)?;
        let (start, end) = (u32_at(self.offsets, at)?, u32_at(self.offsets, at.checked_add(4)?)?);
        std::str::from_utf8(self.blob.get(start..end)?).ok()
    }
}

/// Binary search in a sorted run of `W`-byte keys.
fn find<const W: usize>(keys: &[u8], key: [u8; W]) -> Option<usize> {
    let (mut low, mut high) = (0, keys.len().checked_div(W)?);
    while low < high {
        let mid = low + (high - low) / 2;
        match keys.get(mid * W..mid * W + W)?.cmp(key.as_slice()) {
            std::cmp::Ordering::Less => low = mid + 1,
            std::cmp::Ordering::Greater => high = mid,
            std::cmp::Ordering::Equal => return Some(mid),
        }
    }
    None
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    let pair = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes(pair.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<usize> {
    let four = bytes.get(at..at.checked_add(4)?)?;
    usize::try_from(u32::from_le_bytes(four.try_into().ok()?)).ok()
}

/// Checked while compiling: the file starts with the right marker and is
/// exactly as long as its own header says.
const fn header_ok(db: &[u8]) -> bool {
    const MAGIC: [u8; 8] = *b"HCOUI\x01\0\0";
    if db.len() < HEADER {
        return false;
    }
    let mut i = 0;
    while i < MAGIC.len() {
        if db[i] != MAGIC[i] {
            return false;
        }
        i += 1;
    }
    const fn field(db: &[u8], at: usize) -> u64 {
        u32::from_le_bytes([db[at], db[at + 1], db[at + 2], db[at + 3]]) as u64
    }
    let (n24, n28, n36, names, blob) = (field(db, 8), field(db, 12), field(db, 16), field(db, 20), field(db, 24));
    HEADER as u64 + n24 * 5 + n28 * 6 + n36 * 7 + (names + 1) * 4 + blob == db.len() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_names() -> Vec<&'static str> {
        let table = Table::open(DB).unwrap();
        (0..table.offsets.len() / 4 - 1).map(|i| table.name(i).unwrap()).collect()
    }

    #[test]
    fn known_prefixes() {
        assert_eq!(vendor("8c:90:2d:a0:0e:02").as_deref(), Some("TP-Link Systems Inc."));
        assert_eq!(vendor("00155d443505").as_deref(), Some("Microsoft Corporation"));
        assert_eq!(vendor("bc:24:11:8b:7a:ce").as_deref(), Some("Proxmox Server Solutions GmbH"));
        assert_eq!(vendor("0800.2712.3456").as_deref(), Some("PCS Systemtechnik GmbH"));
        // 00:00:01 is a real block, unlike the all-zero address next to it.
        assert_eq!(vendor("00:00:01:00:00:01").as_deref(), Some("XEROX CORPORATION"));
    }

    #[test]
    fn deco_prefix_looks_like_mesh() {
        let maker = vendor("8C-90-2D-00-00-00").unwrap();
        assert!(maker.starts_with("TP-Link"));
        assert!(looks_like_mesh(&maker));
        assert!(!is_randomised("8C-90-2D-00-00-00"));
    }

    #[test]
    fn mac_formats() {
        let want = Some([0x8c, 0x90, 0x2d, 0xa0, 0x0e, 0x02]);
        for text in ["8c:90:2d:a0:0e:02", "8C-90-2D-A0-0E-02", "8c90.2da0.0e02", "8C902DA00E02", " 8c:90:2d:a0:e:2\n"] {
            assert_eq!(parse_mac(text), want, "{text}");
        }
        assert_eq!(parse_mac("8:0:27:1:2:3"), Some([8, 0, 0x27, 1, 2, 3]));
    }

    #[test]
    fn parse_rejects_junk() {
        for text in [
            "",
            "+a+b+c+d+e+f",
            "+a:+b:+c:+d:+e:+f",
            "8c:90:2d:a0:0e",
            "8c:90:2d:a0:0e:02:11",
            "8c:90:2d:a0:0e:0g",
            "8c:90:2d:a0:0e:",
            "8c:90:2d:a0:0e:002",
            "8c902da00e0",
            "8c902da00e0211",
            "8c902da00e0é",
            "192.168.68.1",
            "192.168.168.100",
            "8c.902d.a00e.02",
            "8c902.da00e.02",
            "ÄÄ:ÄÄ:ÄÄ:ÄÄ:ÄÄ:ÄÄ",
        ] {
            assert_eq!(parse_mac(text), None, "{text:?}");
            assert_eq!(vendor(text), None, "{text:?}");
            assert!(!is_randomised(text), "{text:?}");
        }
    }

    #[test]
    fn randomised_and_virtual() {
        assert_eq!(vendor("da:a1:19:00:00:01"), None);
        assert!(is_randomised("da:a1:19:00:00:01"));
        assert!(is_randomised("DA-A1-19-00-00-01"));
        // A virtual machine's address is made up too, but always the same.
        assert_eq!(vendor("52:54:00:12:34:56").as_deref(), Some("QEMU/KVM"));
        assert_eq!(vendor("02:42:ac:11:00:02").as_deref(), Some("Docker"));
        assert_eq!(vendor("0a:00:27:00:00:10").as_deref(), Some("VirtualBox"));
        assert!(!is_randomised("52:54:00:12:34:56"));
        assert!(!looks_like_mesh("QEMU/KVM"));
        // Docker: 02:42 and then the container's address (10.x, 172.x, 192.x).
        for mac in ["02:42:0a:00:01:05", "02:42:ac:12:00:03", "02:42:c0:a8:44:32"] {
            assert_eq!(vendor(mac).as_deref(), Some("Docker"), "{mac}");
            assert!(!is_randomised(mac), "{mac}");
        }
        // Any other address starting 02:42 is a phone's or laptop's private one.
        for mac in ["02:42:5e:91:3c:07", "02:42:00:00:00:01", "02:42:ab:00:00:01", "02:42:c1:a8:44:32"] {
            assert_eq!(vendor(mac), None, "{mac}");
            assert!(is_randomised(mac), "{mac}");
        }
        // Labels are only for made-up addresses and never shorter than three bytes.
        assert!(VIRTUAL.iter().all(|(prefix, _)| is_local([prefix[0], prefix[1], prefix[2], 0, 0, 0])));
    }

    #[test]
    fn not_devices() {
        for mac in ["00:00:00:00:00:00", "ff:ff:ff:ff:ff:ff", "01:00:5e:00:00:fb", "33:33:00:00:00:01"] {
            assert_eq!(vendor(mac), None, "{mac}");
            assert!(!is_randomised(mac), "{mac}");
        }
    }

    #[test]
    fn registry_placeholders_are_unknown() {
        // B8:4C:87 belongs to the registry itself, and its sub-block F has
        // no owner in the 2026-11-17 data.
        assert_eq!(vendor("b8:4c:87:f0:00:01"), None);
        // E4:F1:4C is registered as "Private".
        assert_eq!(vendor("e4:f1:4c:00:00:01"), None);
        // 00:50:C2 is a registry block, but this small block in it has a real owner.
        assert_eq!(vendor("00:50:c2:ff:ff:ff").as_deref(), Some("MSR-Solutions GmbH"));
        // Not registered at all in the 2026-11-17 data.
        assert_eq!(vendor("f0:f0:f0:00:00:01"), None);
    }

    #[test]
    fn mesh_test_never_panics_and_matches_only_tp_link() {
        let names = all_names();
        assert!(names.len() > 30_000);
        let mut hits: Vec<&str> = names.iter().copied().filter(|n| looks_like_mesh(n)).collect();
        hits.sort();
        assert_eq!(hits, ["TP-LINK TECHNOLOGIES CO.,LTD.", "TP-Link Systems Inc", "TP-Link Systems Inc."]);
        assert!(!looks_like_mesh("BURG-WÄCHTER KG"));
        assert!(!looks_like_mesh("ÄÄÄÄ"));
        assert!(!looks_like_mesh("TP-Lin"));
        assert!(!looks_like_mesh(""));
    }

    #[test]
    fn table_is_sound() {
        let table = Table::open(DB).unwrap();
        let count = table.offsets.len() / 4 - 1;
        assert_eq!(u32_at(table.offsets, 0), Some(0));
        assert_eq!(u32_at(table.offsets, count * 4), Some(table.blob.len()));
        for i in 0..count {
            assert!(table.name(i).is_some(), "name {i}");
        }
        for (keys, width, names) in
            [(table.keys24, 3, table.names24), (table.keys28, 4, table.names28), (table.keys36, 5, table.names36)]
        {
            assert!(!keys.is_empty());
            assert_eq!(keys.len() / width, names.len() / 2);
            let keys: Vec<&[u8]> = keys.chunks(width).collect();
            assert!(keys.windows(2).all(|pair| pair[0] < pair[1]), "keys of {width} bytes are sorted");
            assert!((0..keys.len()).all(|i| usize::from(u16_at(names, i * 2).unwrap()) < count));
        }
    }

    #[test]
    fn damaged_tables_are_refused_or_read_as_unknown() {
        assert!(header_ok(DB));
        for cut in [0, 7, 31, 32, 1000, DB.len() - 1] {
            assert!(!header_ok(&DB[..cut]), "cut at {cut}");
            // Whatever opens must answer without panicking.
            if let Some(table) = Table::open(&DB[..cut]) {
                let _ = find(table.keys24, [0x8c, 0x90, 0x2d]);
                let _ = table.name(0);
                let _ = table.name(usize::MAX);
            }
        }
        let mut wrong = DB.to_vec();
        wrong[5] = 2;
        assert!(!header_ok(&wrong));
        // Counts that claim more than the file holds.
        let mut greedy = DB.to_vec();
        greedy[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(!header_ok(&greedy));
        assert!(Table::open(&greedy).is_none());
        assert_eq!(find::<3>(&[], [0; 3]), None);
        assert_eq!(find::<3>(&[1, 2], [0; 3]), None);
    }
}
