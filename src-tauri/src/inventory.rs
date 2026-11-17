//! The inventory: every device the owner has added or the scanner has found.
//! Stored in a local SQLite file in the user's app data folder.

use crate::error::{Error, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::AtomicI64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Pc,
    Server,
    Vm,
    Container,
    Mesh,
    Other,
    /// Seen on the network but the owner doesn't want to track it.
    Ignore,
}

impl DeviceKind {
    fn as_str(self) -> &'static str {
        match self {
            DeviceKind::Pc => "pc",
            DeviceKind::Server => "server",
            DeviceKind::Vm => "vm",
            DeviceKind::Container => "container",
            DeviceKind::Mesh => "mesh",
            DeviceKind::Other => "other",
            DeviceKind::Ignore => "ignore",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "pc" => DeviceKind::Pc,
            "server" => DeviceKind::Server,
            "vm" => DeviceKind::Vm,
            "container" => DeviceKind::Container,
            "mesh" => DeviceKind::Mesh,
            "ignore" => DeviceKind::Ignore,
            _ => DeviceKind::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Windows,
    Linux,
    Proxmox,
    Other,
}

impl Os {
    fn as_str(self) -> &'static str {
        match self {
            Os::Windows => "windows",
            Os::Linux => "linux",
            Os::Proxmox => "proxmox",
            Os::Other => "other",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "windows" => Os::Windows,
            "linux" => Os::Linux,
            "proxmox" => Os::Proxmox,
            _ => Os::Other,
        }
    }
}

/// A device as stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: i64,
    pub name: String,
    /// IP address or hostname.
    pub host: String,
    pub kind: DeviceKind,
    pub os: Os,
    /// User name for SSH, if this machine is reached over SSH.
    pub ssh_user: Option<String>,
    pub ssh_port: u16,
    pub notes: String,
    /// Hardware address, lower case with colons, when known.
    pub mac: Option<String>,
    /// Maker, worked out from the hardware address.
    pub vendor: Option<String>,
    /// Name the device gave itself on the network, if one was found.
    pub hostname: Option<String>,
    /// Who added it: the owner by hand, or a network scan.
    pub source: Source,
    /// False for a device a scan found that the owner hasn't looked at yet.
    pub reviewed: bool,
    /// Unix seconds.
    pub first_seen: i64,
    /// Unix seconds of the last answer to a scan or ping.
    pub last_seen: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Manual,
    Scan,
}

/// One device that answered a network scan.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub ip: String,
    pub mac: Option<String>,
    pub vendor: Option<String>,
    pub hostname: Option<String>,
    /// True when the maker suggests a mesh node (TP-Link, for the Deco).
    pub looks_like_mesh: bool,
    /// Ping time in milliseconds. Devices that are there but don't answer
    /// ping (many Windows PCs) have none.
    pub rtt_ms: Option<f64>,
    /// True for the PC home-cc is running on.
    pub is_self: bool,
}

/// What a scan changed in the inventory.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSaved {
    pub added: usize,
    pub updated: usize,
}

/// What the screens send to add or change a device.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInput {
    pub name: String,
    pub host: String,
    pub kind: DeviceKind,
    pub os: Os,
    pub ssh_user: Option<String>,
    pub ssh_port: Option<u16>,
    #[serde(default)]
    pub notes: String,
    /// Hardware address. Left as it is when not given; empty removes it.
    /// See [`Inventory::update`] for what a changed `host` does to it.
    #[serde(default)]
    pub mac: Option<String>,
}

impl DeviceInput {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(Error::Invalid("Name can't be empty".into()));
        }
        let host = self.host.trim();
        if host.is_empty() {
            return Err(Error::Invalid("Address can't be empty".into()));
        }
        if host.contains(char::is_whitespace) {
            return Err(Error::Invalid("Address can't contain spaces".into()));
        }
        // Only digits and dots is meant as an IP address. One with a part
        // missing must not be saved: the system would read "192.168.1" as
        // 192.168.0.1 and the app would talk to the wrong machine.
        if host.chars().all(|c| c.is_ascii_digit() || c == '.') && host.parse::<Ipv4Addr>().is_err() {
            return Err(Error::Invalid(
                "Address should be four numbers from 0 to 255 with dots between them, like 192.168.1.20".into(),
            ));
        }
        if let Some(mac) = clean(self.mac.clone()) {
            if normalize_mac(&mac).is_none() {
                return Err(Error::Invalid("Hardware address should look like aa:bb:cc:dd:ee:ff".into()));
            }
        }
        Ok(())
    }
}

pub struct Inventory {
    conn: Mutex<Connection>,
    /// When old ping results were last cleared out (see history.rs).
    pub(crate) purged: AtomicI64,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS devices (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    name      TEXT NOT NULL,
    host      TEXT NOT NULL,
    kind      TEXT NOT NULL,
    os        TEXT NOT NULL,
    ssh_user  TEXT,
    ssh_port  INTEGER NOT NULL DEFAULT 22,
    notes     TEXT NOT NULL DEFAULT ''
);
";

/// Added in step 3 (scan, ping, stats). Applied once, tracked by user_version.
const SCHEMA_V2: &str = "
ALTER TABLE devices ADD COLUMN mac TEXT;
ALTER TABLE devices ADD COLUMN vendor TEXT;
ALTER TABLE devices ADD COLUMN hostname TEXT;
ALTER TABLE devices ADD COLUMN source TEXT NOT NULL DEFAULT 'manual';
ALTER TABLE devices ADD COLUMN reviewed INTEGER NOT NULL DEFAULT 1;
ALTER TABLE devices ADD COLUMN first_seen INTEGER NOT NULL DEFAULT 0;
ALTER TABLE devices ADD COLUMN last_seen INTEGER;
CREATE INDEX devices_mac ON devices (mac);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- One row per check of a device.
CREATE TABLE pings (
    device_id INTEGER NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    ts        INTEGER NOT NULL,
    up        INTEGER NOT NULL,
    rtt_ms    REAL
);
CREATE INDEX pings_device_ts ON pings (device_id, ts);
";

/// Part of the step 3 additions: the SSH identity stored for a device the
/// first time it is reached. Kept apart from the rest because test builds of
/// step 3 made this table with other columns (see `from_conn`).
const HOST_KEYS_TABLE: &str = "
CREATE TABLE ssh_host_keys (
    device_id   INTEGER PRIMARY KEY REFERENCES devices (id) ON DELETE CASCADE,
    algorithm   TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    -- 1 once the owner has been shown the fingerprint and approved it.
    approved    INTEGER NOT NULL DEFAULT 0
);
";

const COLUMNS: &str = "id, name, host, kind, os, ssh_user, ssh_port, notes, \
     mac, vendor, hostname, source, reviewed, first_seen, last_seen";

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// "AA-BB-CC-DD-EE-FF" or "aabb.ccdd.eeff" to "aa:bb:cc:dd:ee:ff".
pub fn normalize_mac(s: &str) -> Option<String> {
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let others_ok = s.chars().all(|c| c.is_ascii_hexdigit() || matches!(c, ':' | '-' | '.' | ' '));
    if hex.len() != 12 || !others_ok {
        return None;
    }
    let hex = hex.to_ascii_lowercase();
    Some((0..6).map(|i| &hex[i * 2..i * 2 + 2]).collect::<Vec<_>>().join(":"))
}

impl Inventory {
    /// The database, for the other back-end modules that keep tables here.
    pub(crate) fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }

    pub fn open(path: &Path) -> Result<Self> {
        Self::from_conn(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        Self::from_conn(Connection::open_in_memory()?)
    }

    fn from_conn(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        conn.execute_batch(SCHEMA)?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 2 {
            conn.execute_batch(&format!("BEGIN; {SCHEMA_V2} {HOST_KEYS_TABLE} PRAGMA user_version = 2; COMMIT;"))?;
        }
        // Early test builds of step 3 made this table with other columns
        // (identities stored by address, or with no record of the owner's
        // approval) that the queries here would fail on. It is replaced by an
        // empty one: the identities are learned again under the first-contact
        // rules.
        let current: Option<i64> = conn
            .query_row("SELECT 1 FROM pragma_table_info('ssh_host_keys') WHERE name = 'approved'", [], |r| r.get(0))
            .optional()?;
        if current.is_none() {
            conn.execute_batch(&format!("BEGIN; DROP TABLE IF EXISTS ssh_host_keys; {HOST_KEYS_TABLE} COMMIT;"))?;
        }
        Ok(Self { conn: Mutex::new(conn), purged: AtomicI64::new(i64::MIN) })
    }

    pub fn list(&self) -> Result<Vec<Device>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            &format!("SELECT {COLUMNS} FROM devices ORDER BY name COLLATE NOCASE"),
        )?;
        let rows = stmt.query_map([], row_to_device)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get(&self, id: i64) -> Result<Device> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            &format!("SELECT {COLUMNS} FROM devices WHERE id = ?1"),
            [id],
            row_to_device,
        )
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("device {id}")))
    }

    pub fn add(&self, input: DeviceInput) -> Result<Device> {
        input.validate()?;
        let id = {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO devices (name, host, kind, os, ssh_user, ssh_port, notes, mac, first_seen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    input.name.trim(),
                    input.host.trim(),
                    input.kind.as_str(),
                    input.os.as_str(),
                    clean(input.ssh_user),
                    input.ssh_port.unwrap_or(22),
                    input.notes,
                    clean(input.mac).and_then(|m| normalize_mac(&m)),
                    now(),
                ],
            )?;
            conn.last_insert_rowid()
        };
        self.get(id)
    }

    /// Save the owner's changes to a device.
    ///
    /// The hardware address is kept when none is given, removed by an empty
    /// one and replaced by any other. Pointing the device at another address
    /// without giving a new one makes the entry describe whatever is at the
    /// new address, so what was learned at the old one (hardware address,
    /// maker, network name) is dropped: otherwise the next scan would follow
    /// the old hardware address and undo the edit. The stored hardware address
    /// sent back unchanged counts as not given there, because the Machines
    /// form sends it with every save. Another address or SSH port is also
    /// another SSH server, so the stored identity goes and the first-contact
    /// rules apply again.
    pub fn update(&self, id: i64, input: DeviceInput) -> Result<Device> {
        input.validate()?;
        {
            let mut conn = self.conn.lock().unwrap();
            let tx = conn.transaction()?;
            let stored: Option<(String, i64, Option<String>)> = tx
                .query_row("SELECT host, ssh_port, mac FROM devices WHERE id = ?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .optional()?;
            let Some((old_host, old_port, old_mac)) = stored else {
                return Err(Error::NotFound(format!("device {id}")));
            };
            let host = input.host.trim();
            let port = input.ssh_port.unwrap_or(22);
            let moved = !host.eq_ignore_ascii_case(&old_host);
            // Trimmed as validate() does, so that what it accepted is what is
            // stored. Empty or blank gives none.
            let typed = input.mac.as_deref().map(|typed| normalize_mac(typed.trim()));
            let mac = match typed {
                // Only the address was changed: the hardware address that
                // came back is the one learned at the old address.
                Some(same) if moved && same == old_mac => None,
                Some(given) => given,
                None if moved => None,
                None => old_mac.clone(),
            };
            // The maker comes from the hardware address. The network name
            // belongs to whatever was at the old address, unless the owner
            // says which hardware this is.
            let keep_vendor = mac == old_mac;
            let keep_hostname = !moved || mac.is_some();
            tx.execute(
                // Saving a device counts as the owner having looked at it.
                "UPDATE devices SET name = ?1, host = ?2, kind = ?3, os = ?4,
                 ssh_user = ?5, ssh_port = ?6, notes = ?7, mac = ?8,
                 vendor = CASE WHEN ?9 THEN vendor END, hostname = CASE WHEN ?10 THEN hostname END,
                 reviewed = 1
                 WHERE id = ?11",
                params![
                    input.name.trim(),
                    host,
                    input.kind.as_str(),
                    input.os.as_str(),
                    clean(input.ssh_user),
                    port,
                    input.notes,
                    mac,
                    keep_vendor,
                    keep_hostname,
                    id,
                ],
            )?;
            if moved || i64::from(port) != old_port {
                tx.execute("DELETE FROM ssh_host_keys WHERE device_id = ?1", [id])?;
            }
            tx.commit()?;
        }
        self.get(id)
    }

    pub fn remove(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        if conn.execute("DELETE FROM devices WHERE id = ?1", [id])? == 0 {
            return Err(Error::NotFound(format!("device {id}")));
        }
        Ok(())
    }

    /// Save what a scan found. A device already in the inventory keeps the
    /// name and type the owner gave it; anything not recognised is added as
    /// new and unreviewed.
    ///
    /// `named` maps an IP address to the hand-added device whose host is a
    /// name that resolves to that address right now.
    ///
    /// Each thing found is matched to at most one device, and each device to
    /// at most one thing found, by the first of these that applies:
    /// 1. the device with this hardware address at this IP address;
    /// 2. the device with this hardware address, wherever it was, which is
    ///    then followed to the new IP address (a host name the owner typed is
    ///    left alone). Only when the hardware address stands for one machine:
    ///    it shows up once in this scan and one device has it. Bridged VMs
    ///    and shared service addresses answer for several IPs with one
    ///    hardware address, and those must not be folded into one device;
    /// 3. the device added by a name that resolves to this IP address;
    /// 4. the device at this IP address with no hardware address yet, or
    ///    with any, if the scan saw none here (nothing says it is another).
    ///
    /// Something found with a hardware address never takes over a device
    /// stored with a different one: that is a new device on a reused IP.
    pub fn record_scan(&self, found: &[Found], at: i64, named: &HashMap<String, i64>) -> Result<ScanSaved> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let macs: Vec<Option<String>> = found.iter().map(|f| f.mac.as_deref().and_then(normalize_mac)).collect();
        let mut times_seen: HashMap<&str, usize> = HashMap::new();
        for mac in macs.iter().flatten() {
            *times_seen.entry(mac.as_str()).or_default() += 1;
        }
        let mut matched: HashSet<i64> = HashSet::new();
        let mut saved = ScanSaved::default();

        // Everything a hardware address settles comes first, for the whole
        // scan, so that a result known only by its IP address can't claim
        // the entry of a device that has just moved away from that address.
        let mut rest = Vec::new();
        for (f, mac) in found.iter().zip(&macs) {
            let mac = mac.as_deref();
            let hit = match mac {
                Some(mac) => by_hardware(&tx, mac, &f.ip, times_seen.get(mac) == Some(&1), &matched)?,
                None => None,
            };
            match hit {
                Some((id, host)) => {
                    seen_again(&tx, id, &host, f, mac, at)?;
                    matched.insert(id);
                    saved.updated += 1;
                }
                None => rest.push((f, mac)),
            }
        }
        for (f, mac) in rest {
            match by_address(&tx, &f.ip, mac, named, &matched)? {
                // Matched by address, the device is where it was: its host
                // (an IP address or a name the owner typed) stays as it is.
                Some((id, host)) => {
                    seen_again(&tx, id, &host, f, mac, at)?;
                    matched.insert(id);
                    saved.updated += 1;
                }
                None => {
                    let name = f
                        .hostname
                        .clone()
                        .or_else(|| f.vendor.as_ref().map(|v| format!("{v} device")))
                        .unwrap_or_else(|| f.ip.clone());
                    let kind = if f.looks_like_mesh { DeviceKind::Mesh } else { DeviceKind::Other };
                    tx.execute(
                        "INSERT INTO devices (name, host, kind, os, ssh_port, notes, mac, vendor, hostname,
                         source, reviewed, first_seen, last_seen)
                         VALUES (?1, ?2, ?3, 'other', 22, '', ?4, ?5, ?6, 'scan', 0, ?7, ?7)",
                        params![name, f.ip, kind.as_str(), mac, f.vendor, f.hostname, at],
                    )?;
                    matched.insert(tx.last_insert_rowid());
                    saved.added += 1;
                }
            }
        }
        tx.commit()?;
        Ok(saved)
    }
}

/// The device a hardware address points to, with the host it should now
/// have: rules 1 and 2 of [`Inventory::record_scan`]. `alone` says the
/// address shows up only once in the scan.
fn by_hardware(
    db: &Connection,
    mac: &str,
    ip: &str,
    alone: bool,
    taken: &HashSet<i64>,
) -> Result<Option<(i64, String)>> {
    let here = first_free(db, "SELECT id, host FROM devices WHERE mac = ?1 AND host = ?2 ORDER BY id", [mac, ip], taken)?;
    if here.is_some() || !alone {
        return Ok(here);
    }
    let mut stmt = db.prepare_cached("SELECT id, host FROM devices WHERE mac = ?1 ORDER BY id LIMIT 2")?;
    let with_mac = stmt
        .query_map([mac], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(match with_mac.as_slice() {
        [(id, host)] if !taken.contains(id) => {
            // Follow the device to its new IP, but leave a host name the
            // owner typed (such as "nas.lan") alone.
            let host = if host.parse::<IpAddr>().is_ok() { ip.to_string() } else { host.clone() };
            Some((*id, host))
        }
        _ => None,
    })
}

/// The device at an IP address, with its host as stored: rules 3 and 4 of
/// [`Inventory::record_scan`]. `mac` is the hardware address the scan saw
/// there, if it saw one.
fn by_address(
    db: &Connection,
    ip: &str,
    mac: Option<&str>,
    named: &HashMap<String, i64>,
    taken: &HashSet<i64>,
) -> Result<Option<(i64, String)>> {
    if let Some(&id) = named.get(ip).filter(|id| !taken.contains(*id)) {
        // The device is looked at again here, inside the transaction: it may
        // have been removed or given a hardware address since the name was
        // looked up.
        let row: Option<(String, Option<String>)> = db
            .prepare_cached("SELECT host, mac FROM devices WHERE id = ?1")?
            .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        if let Some((host, stored)) = row {
            let contradicted = matches!((stored.as_deref(), mac), (Some(stored), Some(seen)) if stored != seen);
            if !contradicted {
                return Ok(Some((id, host)));
            }
        }
    }
    let sql = match mac {
        // A device stored with another hardware address is a different one,
        // even at the same IP.
        Some(_) => "SELECT id, host FROM devices WHERE host = ?1 AND mac IS NULL ORDER BY id",
        // With no hardware address seen, the IP address is all there is to
        // go by. If two entries share it (the address was reused), prefer
        // the one with no hardware address, then the one seen most recently.
        None => "SELECT id, host FROM devices WHERE host = ?1 ORDER BY (mac IS NULL) DESC, last_seen DESC, id",
    };
    first_free(db, sql, [ip], taken)
}

/// The first row (id, host) of a lookup that nothing else found in this scan
/// has been matched to.
fn first_free<P: rusqlite::Params>(
    db: &Connection,
    sql: &str,
    params: P,
    taken: &HashSet<i64>,
) -> Result<Option<(i64, String)>> {
    let mut stmt = db.prepare_cached(sql)?;
    let mut rows = stmt.query(params)?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        if !taken.contains(&id) {
            return Ok(Some((id, row.get(1)?)));
        }
    }
    Ok(None)
}

/// Note that a known device answered a scan, and what the scan learned.
fn seen_again(db: &Connection, id: i64, host: &str, f: &Found, mac: Option<&str>, at: i64) -> Result<()> {
    db.prepare_cached(
        "UPDATE devices SET host = ?1, mac = COALESCE(?2, mac), vendor = COALESCE(?3, vendor),
         hostname = COALESCE(?4, hostname), last_seen = ?5 WHERE id = ?6",
    )?
    .execute(params![host, mac, f.vendor, f.hostname, at, id])?;
    Ok(())
}

fn clean(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn row_to_device(row: &rusqlite::Row) -> rusqlite::Result<Device> {
    Ok(Device {
        id: row.get(0)?,
        name: row.get(1)?,
        host: row.get(2)?,
        kind: DeviceKind::parse(&row.get::<_, String>(3)?),
        os: Os::parse(&row.get::<_, String>(4)?),
        ssh_user: row.get(5)?,
        ssh_port: row.get(6)?,
        notes: row.get(7)?,
        mac: row.get(8)?,
        vendor: row.get(9)?,
        hostname: row.get(10)?,
        source: if row.get::<_, String>(11)? == "scan" { Source::Scan } else { Source::Manual },
        reviewed: row.get(12)?,
        first_seen: row.get(13)?,
        last_seen: row.get(14)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, host: &str) -> DeviceInput {
        DeviceInput {
            name: name.into(),
            host: host.into(),
            kind: DeviceKind::Vm,
            os: Os::Linux,
            ssh_user: Some("  jim ".into()),
            ssh_port: None,
            notes: String::new(),
            mac: None,
        }
    }

    #[test]
    fn add_list_update_remove() {
        let inv = Inventory::in_memory().unwrap();
        let b = inv.add(input("homelab", "192.168.68.21")).unwrap();
        let a = inv.add(input("Dev VM", "192.168.68.22")).unwrap();
        assert_eq!(b.ssh_user.as_deref(), Some("jim"));
        assert_eq!(b.ssh_port, 22);

        // Sorted by name, case-insensitive.
        let names: Vec<_> = inv.list().unwrap().into_iter().map(|d| d.name).collect();
        assert_eq!(names, ["Dev VM", "homelab"]);

        let mut changed = input("Dev VM", "dev.lan");
        changed.ssh_port = Some(2222);
        changed.ssh_user = Some("   ".into());
        let a2 = inv.update(a.id, changed).unwrap();
        assert_eq!(a2.host, "dev.lan");
        assert_eq!(a2.ssh_port, 2222);
        assert_eq!(a2.ssh_user, None);

        inv.remove(b.id).unwrap();
        assert_eq!(inv.list().unwrap().len(), 1);
        assert!(matches!(inv.remove(b.id), Err(Error::NotFound(_))));
    }

    #[test]
    fn rejects_bad_input() {
        let inv = Inventory::in_memory().unwrap();
        assert!(matches!(inv.add(input(" ", "1.2.3.4")), Err(Error::Invalid(_))));
        assert!(matches!(inv.add(input("x", "")), Err(Error::Invalid(_))));
        assert!(matches!(inv.add(input("x", "bad host")), Err(Error::Invalid(_))));
    }

    fn found(ip: &str, mac: Option<&str>, hostname: Option<&str>) -> Found {
        Found {
            ip: ip.into(),
            mac: mac.map(Into::into),
            vendor: Some("TP-Link Systems Inc".into()),
            hostname: hostname.map(Into::into),
            looks_like_mesh: false,
            rtt_ms: Some(1.0),
            is_self: false,
        }
    }

    fn no_names() -> HashMap<String, i64> {
        HashMap::new()
    }

    #[test]
    fn mac_formats() {
        assert_eq!(normalize_mac("AA-BB-CC-DD-EE-0F").as_deref(), Some("aa:bb:cc:dd:ee:0f"));
        assert_eq!(normalize_mac("aabb.ccdd.ee0f").as_deref(), Some("aa:bb:cc:dd:ee:0f"));
        assert_eq!(normalize_mac("aa:bb:cc:dd:ee"), None);
        assert_eq!(normalize_mac("not a mac at all"), None);
    }

    #[test]
    fn scan_adds_new_devices_and_keeps_owner_names() {
        let inv = Inventory::in_memory().unwrap();
        let saved = inv
            .record_scan(&[found("192.168.68.50", Some("AA:BB:CC:00:00:01"), Some("deco-hall"))], 100, &no_names())
            .unwrap();
        assert_eq!(saved, ScanSaved { added: 1, updated: 0 });
        let d = inv.list().unwrap().remove(0);
        assert_eq!((d.name.as_str(), d.reviewed, d.source), ("deco-hall", false, Source::Scan));
        assert_eq!(d.mac.as_deref(), Some("aa:bb:cc:00:00:01"));
        assert_eq!((d.first_seen, d.last_seen), (100, Some(100)));

        // The owner renames it; a later scan at a new IP keeps the name.
        let mut renamed = input("Hallway", "192.168.68.50");
        renamed.kind = DeviceKind::Mesh;
        inv.update(d.id, renamed).unwrap();
        let saved = inv
            .record_scan(&[found("192.168.68.77", Some("aa:bb:cc:00:00:01"), Some("deco-hall"))], 200, &no_names())
            .unwrap();
        assert_eq!(saved, ScanSaved { added: 0, updated: 1 });
        let d = inv.list().unwrap().remove(0);
        assert_eq!((d.name.as_str(), d.host.as_str(), d.reviewed), ("Hallway", "192.168.68.77", true));
        assert_eq!(d.last_seen, Some(200));
    }

    #[test]
    fn scan_matches_hand_added_devices() {
        let inv = Inventory::in_memory().unwrap();
        let by_ip = inv.add(input("NAS", "192.168.68.20")).unwrap();
        let by_name = inv.add(input("Dev VM", "dev.lan")).unwrap();
        let mut with_mac = input("Proxmox", "pve.lan");
        with_mac.mac = Some("AA-BB-CC-00-00-09".into());
        let by_mac = inv.add(with_mac).unwrap();

        let saved = inv
            .record_scan(
                &[
                    found("192.168.68.20", Some("aa:bb:cc:00:00:02"), None),
                    found("192.168.68.30", Some("aa:bb:cc:00:00:09"), None),
                ],
                300,
                &no_names(),
            )
            .unwrap();
        assert_eq!(saved, ScanSaved { added: 0, updated: 2 });
        // Matched by IP: learns its hardware address.
        assert_eq!(inv.get(by_ip.id).unwrap().mac.as_deref(), Some("aa:bb:cc:00:00:02"));
        // Matched by hardware address: the typed host name is left alone.
        let pve = inv.get(by_mac.id).unwrap();
        assert_eq!((pve.host.as_str(), pve.last_seen), ("pve.lan", Some(300)));
        assert_eq!(inv.get(by_name.id).unwrap().last_seen, None);

        // A different device later takes the NAS's old IP: it is new, not the NAS.
        let saved =
            inv.record_scan(&[found("192.168.68.20", Some("aa:bb:cc:00:00:03"), None)], 400, &no_names()).unwrap();
        assert_eq!(saved, ScanSaved { added: 1, updated: 0 });
        assert_eq!(inv.get(by_ip.id).unwrap().last_seen, Some(300));
    }

    const MAC_A: &str = "aa:bb:cc:00:00:0a";
    const MAC_B: &str = "aa:bb:cc:00:00:0b";
    const MAC_C: &str = "aa:bb:cc:00:00:0c";
    const MAKER: &str = "TP-Link Systems Inc";

    fn scan(inv: &Inventory, found: &[Found], at: i64) -> ScanSaved {
        inv.record_scan(found, at, &no_names()).unwrap()
    }

    fn names(pairs: &[(&str, i64)]) -> HashMap<String, i64> {
        pairs.iter().map(|(ip, id)| (ip.to_string(), *id)).collect()
    }

    /// Every device as (host, hardware address), in the order they were added.
    fn rows(inv: &Inventory) -> Vec<(String, Option<String>)> {
        let mut all = inv.list().unwrap();
        all.sort_by_key(|d| d.id);
        all.into_iter().map(|d| (d.host, d.mac)).collect()
    }

    fn row(host: &str, mac: Option<&str>) -> (String, Option<String>) {
        (host.to_string(), mac.map(str::to_string))
    }

    #[test]
    fn rejects_an_address_with_a_number_missing() {
        let inv = Inventory::in_memory().unwrap();
        for host in ["192.168.1", "10.5", "22", "192.168.1.", " 192.168.1 ", "192.168.1.300", "1.2.3.4.5", "..."] {
            let Err(Error::Invalid(message)) = inv.add(input("x", host)) else { panic!("{host:?} was accepted") };
            assert!(message.contains("four numbers"), "{message}");
        }
        for host in ["192.168.1.20", " 10.0.0.5 ", "nas.lan", "pi4", "4pi", "3.lan", "fe80::1", "::1"] {
            assert!(inv.add(input("x", host)).is_ok(), "{host:?} was rejected");
        }
        // The same goes for changing a device, and nothing is stored.
        let id = inv.add(input("NAS", "192.168.68.20")).unwrap().id;
        assert!(matches!(inv.update(id, input("NAS", "192.168.68")), Err(Error::Invalid(_))));
        assert_eq!(inv.get(id).unwrap().host, "192.168.68.20");
    }

    #[test]
    fn update_keeps_clears_or_sets_the_hardware_address() {
        let inv = Inventory::in_memory().unwrap();
        let mut nas = input("NAS", "192.168.68.20");
        nas.mac = Some("AA-BB-CC-00-00-0A".into());
        let id = inv.add(nas).unwrap().id;
        // A scan fills in the maker and the name the device gives itself.
        scan(&inv, &[found("192.168.68.20", Some(MAC_A), Some("nas"))], 100);
        let d = inv.get(id).unwrap();
        assert_eq!((d.vendor.as_deref(), d.hostname.as_deref()), (Some(MAKER), Some("nas")));

        // Not given (a rename from the Network table): everything is kept.
        let d = inv.update(id, input("Big NAS", "192.168.68.20")).unwrap();
        assert_eq!((d.name.as_str(), d.mac.as_deref()), ("Big NAS", Some(MAC_A)));
        assert_eq!((d.vendor.as_deref(), d.hostname.as_deref()), (Some(MAKER), Some("nas")));

        // Given again, written another way: still the same hardware.
        let mut same = input("Big NAS", "192.168.68.20");
        same.mac = Some("aabb.cc00.000a".into());
        let d = inv.update(id, same).unwrap();
        assert_eq!((d.mac.as_deref(), d.vendor.as_deref()), (Some(MAC_A), Some(MAKER)));

        // Another one is stored, and the maker of the old one goes with it.
        let mut other = input("Big NAS", "192.168.68.20");
        other.mac = Some("AA:BB:CC:00:00:0B".into());
        let d = inv.update(id, other.clone()).unwrap();
        assert_eq!((d.mac.as_deref(), d.vendor.as_deref(), d.hostname.as_deref()), (Some(MAC_B), None, Some("nas")));

        // Pasted with white space around it (copied from a router's web
        // page): stored all the same, not removed.
        for (pasted, stored) in [("aa:bb:cc:00:00:0c\t", MAC_C), ("\u{a0}AA-BB-CC-00-00-0B \r\n", MAC_B)] {
            let mut padded = input("Big NAS", "192.168.68.20");
            padded.mac = Some(pasted.into());
            assert_eq!(inv.update(id, padded).unwrap().mac.as_deref(), Some(stored), "for {pasted:?}");
        }

        // One that isn't a hardware address is refused and changes nothing.
        let mut bad = input("Big NAS", "192.168.68.20");
        bad.mac = Some("not one".into());
        assert!(matches!(inv.update(id, bad), Err(Error::Invalid(_))));
        assert_eq!(inv.get(id).unwrap().mac.as_deref(), Some(MAC_B));

        // Emptied in the form: removed.
        for blank in ["", "   "] {
            inv.update(id, other.clone()).unwrap();
            let mut cleared = input("Big NAS", "192.168.68.20");
            cleared.mac = Some(blank.into());
            let d = inv.update(id, cleared).unwrap();
            assert_eq!((d.mac, d.vendor, d.hostname.as_deref()), (None, None, Some("nas")), "for {blank:?}");
        }
        // The next scan then learns it again by address (a replaced network card).
        assert_eq!(scan(&inv, &[found("192.168.68.20", Some(MAC_C), None)], 200), ScanSaved { added: 0, updated: 1 });
        let d = inv.get(id).unwrap();
        assert_eq!((d.mac.as_deref(), d.vendor.as_deref()), (Some(MAC_C), Some(MAKER)));

        assert!(matches!(inv.update(id + 1, input("x", "192.168.68.9")), Err(Error::NotFound(_))));
    }

    #[test]
    fn a_corrected_address_is_not_undone_by_the_next_scan() {
        let inv = Inventory::in_memory().unwrap();
        // The owner mistypes the NAS's address; a phone lives there.
        let nas = inv.add(input("NAS", "192.168.68.20")).unwrap().id;
        let phone = found("192.168.68.20", Some(MAC_A), Some("phone"));
        let real_nas = found("192.168.68.21", Some(MAC_B), Some("nas"));
        assert_eq!(scan(&inv, &[phone.clone(), real_nas.clone()], 100), ScanSaved { added: 1, updated: 1 });
        let d = inv.get(nas).unwrap();
        assert_eq!((d.mac.as_deref(), d.hostname.as_deref()), (Some(MAC_A), Some("phone")));

        // They remove the extra entry the scan made and correct their own.
        let extra = inv.list().unwrap().into_iter().find(|d| d.id != nas).unwrap();
        inv.remove(extra.id).unwrap();
        let d = inv.update(nas, input("NAS", "192.168.68.21")).unwrap();
        assert_eq!((d.host.as_str(), d.mac, d.vendor, d.hostname), ("192.168.68.21", None, None, None));

        // The next scan leaves the correction alone and learns the right hardware address.
        assert_eq!(scan(&inv, &[phone, real_nas], 200), ScanSaved { added: 1, updated: 1 });
        let d = inv.get(nas).unwrap();
        assert_eq!((d.host.as_str(), d.mac.as_deref(), d.hostname.as_deref()), ("192.168.68.21", Some(MAC_B), Some("nas")));
        assert_eq!(rows(&inv), [row("192.168.68.21", Some(MAC_B)), row("192.168.68.20", Some(MAC_A))]);
    }

    #[test]
    fn a_corrected_address_saved_from_the_machines_form_is_not_undone_by_the_next_scan() {
        let inv = Inventory::in_memory().unwrap();
        // The owner mistypes the NAS's address; a phone lives there, and the
        // first scan puts the phone's hardware address on the entry.
        let nas = inv.add(input("NAS", "192.168.68.20")).unwrap().id;
        let phone = found("192.168.68.20", Some(MAC_A), Some("phone"));
        assert_eq!(scan(&inv, &[phone.clone()], 100), ScanSaved { added: 0, updated: 1 });
        assert_eq!(inv.get(nas).unwrap().mac.as_deref(), Some(MAC_A));

        // Edit machine: the address is corrected, and the form sends the
        // hardware address back as it was filled in.
        let mut fixed = input("NAS", "192.168.68.21");
        fixed.mac = Some(MAC_A.into());
        let d = inv.update(nas, fixed).unwrap();
        assert_eq!((d.host.as_str(), d.mac, d.vendor, d.hostname), ("192.168.68.21", None, None, None));

        // The next scan leaves the address alone, learns the NAS's own
        // hardware address and gives the phone an entry of its own.
        let both = [phone, found("192.168.68.21", Some(MAC_B), Some("nas"))];
        assert_eq!(scan(&inv, &both, 200), ScanSaved { added: 1, updated: 1 });
        let d = inv.get(nas).unwrap();
        assert_eq!((d.host.as_str(), d.mac.as_deref(), d.hostname.as_deref()), ("192.168.68.21", Some(MAC_B), Some("nas")));
        let settled = [row("192.168.68.21", Some(MAC_B)), row("192.168.68.20", Some(MAC_A))];
        assert_eq!(rows(&inv), settled);
        // And it stays that way.
        assert_eq!(scan(&inv, &both, 300), ScanSaved { added: 0, updated: 2 });
        assert_eq!(rows(&inv), settled);
    }

    #[test]
    fn a_new_address_keeps_the_hardware_address_only_if_another_one_is_given() {
        let inv = Inventory::in_memory().unwrap();
        let id = scan_one(&inv, found("192.168.68.20", Some(MAC_A), Some("nas")));
        // The stored one sent back, however it is written, is what was
        // learned at the old address: it goes with the maker and the name.
        let mut moved = input("NAS", "192.168.68.30");
        moved.mac = Some(" AA-BB-CC-00-00-0A ".into());
        let d = inv.update(id, moved).unwrap();
        assert_eq!(d.host, "192.168.68.30");
        assert_eq!((d.mac, d.vendor, d.hostname), (None, None, None));
        // If the same hardware really is there, the next scan says so.
        let there = found("192.168.68.30", Some(MAC_A), Some("nas"));
        assert_eq!(scan(&inv, &[there], 200), ScanSaved { added: 0, updated: 1 });
        let d = inv.get(id).unwrap();
        assert_eq!((d.mac.as_deref(), d.vendor.as_deref(), d.hostname.as_deref()), (Some(MAC_A), Some(MAKER), Some("nas")));

        // A different one typed along with the new address is the owner
        // saying which hardware this is: stored.
        let mut other = input("NAS", "192.168.68.31");
        other.mac = Some(MAC_B.into());
        let d = inv.update(id, other).unwrap();
        assert_eq!(d.host, "192.168.68.31");
        assert_eq!((d.mac.as_deref(), d.vendor.as_deref(), d.hostname.as_deref()), (Some(MAC_B), None, Some("nas")));

        // Moved and the hardware address emptied: nothing of the old machine is left.
        let mut emptied = input("NAS", "192.168.68.32");
        emptied.mac = Some(String::new());
        let d = inv.update(id, emptied).unwrap();
        assert_eq!((d.mac, d.vendor, d.hostname), (None, None, None));
    }

    #[test]
    fn retyping_a_name_in_other_letter_case_is_not_a_new_address() {
        let inv = Inventory::in_memory().unwrap();
        let mut nas = input("NAS", "nas.lan");
        nas.mac = Some(MAC_A.into());
        let id = inv.add(nas).unwrap().id;
        let d = inv.update(id, input("NAS", " NAS.lan ")).unwrap();
        assert_eq!((d.host.as_str(), d.mac.as_deref()), ("NAS.lan", Some(MAC_A)));
        // A name swapped for an address is one.
        let d = inv.update(id, input("NAS", "192.168.68.20")).unwrap();
        assert_eq!((d.host.as_str(), d.mac), ("192.168.68.20", None));
    }

    /// Scan one new device into an empty spot and give back its id.
    fn scan_one(inv: &Inventory, f: Found) -> i64 {
        let before: HashSet<i64> = inv.list().unwrap().into_iter().map(|d| d.id).collect();
        assert_eq!(scan(inv, &[f], 100), ScanSaved { added: 1, updated: 0 });
        inv.list().unwrap().into_iter().map(|d| d.id).find(|id| !before.contains(id)).unwrap()
    }

    /// Bridged VMs, a shared service address or a Wi-Fi repeater: one
    /// hardware address answers for several IP addresses.
    fn three_on_one_mac() -> [Found; 3] {
        [
            found("192.168.68.20", Some(MAC_A), None),
            found("192.168.68.21", Some(MAC_A), None),
            found("192.168.68.22", Some(MAC_A), None),
        ]
    }

    #[test]
    fn hand_added_devices_sharing_a_hardware_address_keep_their_addresses() {
        let inv = Inventory::in_memory().unwrap();
        let nas = inv.add(input("NAS", "192.168.68.20")).unwrap().id;
        let lab = inv.add(input("Homelab", "192.168.68.21")).unwrap().id;
        let dev = inv.add(input("Dev", "192.168.68.22")).unwrap().id;

        for at in [100, 200] {
            assert_eq!(scan(&inv, &three_on_one_mac(), at), ScanSaved { added: 0, updated: 3 });
            for (id, name, host) in
                [(nas, "NAS", "192.168.68.20"), (lab, "Homelab", "192.168.68.21"), (dev, "Dev", "192.168.68.22")]
            {
                let d = inv.get(id).unwrap();
                assert_eq!((d.name.as_str(), d.host.as_str()), (name, host));
                assert_eq!((d.mac.as_deref(), d.last_seen), (Some(MAC_A), Some(at)));
            }
        }
        assert_eq!(inv.list().unwrap().len(), 3);
    }

    #[test]
    fn a_first_scan_keeps_addresses_sharing_a_hardware_address_apart() {
        let inv = Inventory::in_memory().unwrap();
        assert_eq!(scan(&inv, &three_on_one_mac(), 100), ScanSaved { added: 3, updated: 0 });
        let three =
            [row("192.168.68.20", Some(MAC_A)), row("192.168.68.21", Some(MAC_A)), row("192.168.68.22", Some(MAC_A))];
        assert_eq!(rows(&inv), three);

        // The same again adds nothing.
        assert_eq!(scan(&inv, &three_on_one_mac(), 200), ScanSaved { added: 0, updated: 3 });
        assert_eq!(rows(&inv), three);

        // Later only one of them answers, at an address not seen before. It
        // can't be told which of the three it is, so none of them is moved.
        assert_eq!(scan(&inv, &[found("192.168.68.40", Some(MAC_A), None)], 300), ScanSaved { added: 1, updated: 0 });
        let mut four = three.to_vec();
        four.push(row("192.168.68.40", Some(MAC_A)));
        assert_eq!(rows(&inv), four);
        let seen: Vec<_> = inv.list().unwrap().into_iter().map(|d| (d.host, d.last_seen)).collect();
        assert!(seen.iter().all(|(host, at)| *at == Some(if host == "192.168.68.40" { 300 } else { 200 })), "{seen:?}");
    }

    #[test]
    fn a_device_is_not_followed_to_two_addresses_at_once() {
        let inv = Inventory::in_memory().unwrap();
        let id = scan_one(&inv, found("192.168.68.20", Some(MAC_A), None));
        // Its hardware address turns up at two other addresses in one scan.
        let two = [found("192.168.68.30", Some(MAC_A), None), found("192.168.68.31", Some(MAC_A), None)];
        assert_eq!(scan(&inv, &two, 200), ScanSaved { added: 2, updated: 0 });
        let d = inv.get(id).unwrap();
        assert_eq!((d.host.as_str(), d.last_seen), ("192.168.68.20", Some(100)));
    }

    #[test]
    fn scan_matches_a_device_added_by_name() {
        let inv = Inventory::in_memory().unwrap();
        let dev = inv.add(input("Dev VM", "dev.lan")).unwrap().id;
        // The caller found that "dev.lan" is 192.168.68.22 right now.
        let saved = inv
            .record_scan(
                &[found("192.168.68.22", Some(MAC_A), Some("dev"))],
                100,
                &names(&[("192.168.68.22", dev), ("192.168.68.99", dev + 50)]),
            )
            .unwrap();
        assert_eq!(saved, ScanSaved { added: 0, updated: 1 });
        let d = inv.get(dev).unwrap();
        assert_eq!((d.name.as_str(), d.host.as_str(), d.source, d.reviewed), ("Dev VM", "dev.lan", Source::Manual, true));
        assert_eq!((d.mac.as_deref(), d.hostname.as_deref(), d.last_seen), (Some(MAC_A), Some("dev"), Some(100)));

        // From then on its hardware address is enough, wherever it is.
        assert_eq!(scan(&inv, &[found("192.168.68.23", Some(MAC_A), None)], 200), ScanSaved { added: 0, updated: 1 });
        assert_eq!(rows(&inv), [row("dev.lan", Some(MAC_A))]);
    }

    #[test]
    fn a_name_does_not_hand_a_device_to_other_hardware() {
        let inv = Inventory::in_memory().unwrap();
        let mut dev = input("Dev VM", "dev.lan");
        dev.mac = Some(MAC_A.into());
        let dev = inv.add(dev).unwrap().id;
        let gone = inv.add(input("Gone", "gone.lan")).unwrap().id;
        inv.remove(gone).unwrap();

        // The name resolves to an address where other hardware answers, and
        // one name was looked up for a device removed since.
        let saved = inv
            .record_scan(
                &[found("192.168.68.22", Some(MAC_B), None), found("192.168.68.23", Some(MAC_C), None)],
                100,
                &names(&[("192.168.68.22", dev), ("192.168.68.23", gone)]),
            )
            .unwrap();
        assert_eq!(saved, ScanSaved { added: 2, updated: 0 });
        let d = inv.get(dev).unwrap();
        assert_eq!((d.mac.as_deref(), d.last_seen), (Some(MAC_A), None));
    }

    #[test]
    fn a_named_device_with_a_hardware_address_is_still_matched_when_nothing_contradicts_it() {
        let inv = Inventory::in_memory().unwrap();
        // The NAS was added by name and hosts a VM that answers with the NAS's hardware address.
        let nas = inv.add(input("NAS", "nas.lan")).unwrap().id;
        let both = [found("192.168.68.20", Some(MAC_A), None), found("192.168.68.21", Some(MAC_A), None)];
        let lookup = names(&[("192.168.68.20", nas)]);
        assert_eq!(inv.record_scan(&both, 100, &lookup).unwrap(), ScanSaved { added: 1, updated: 1 });
        // The second scan must not add the NAS again under its IP address.
        assert_eq!(inv.record_scan(&both, 200, &lookup).unwrap(), ScanSaved { added: 0, updated: 2 });
        assert_eq!(rows(&inv), [row("nas.lan", Some(MAC_A)), row("192.168.68.21", Some(MAC_A))]);

        // Nor does a scan that sees no hardware addresses at all.
        let bare = [found("192.168.68.20", None, None)];
        assert_eq!(inv.record_scan(&bare, 300, &lookup).unwrap(), ScanSaved { added: 0, updated: 1 });
        assert_eq!(inv.get(nas).unwrap().last_seen, Some(300));
        assert_eq!(inv.list().unwrap().len(), 2);
    }

    #[test]
    fn a_device_is_matched_only_once_in_a_scan() {
        let inv = Inventory::in_memory().unwrap();
        let dev = inv.add(input("Dev VM", "dev.lan")).unwrap().id;
        // Two addresses are both said to be this one device.
        let saved = inv
            .record_scan(
                &[found("192.168.68.22", Some(MAC_A), None), found("192.168.68.23", Some(MAC_B), None)],
                100,
                &names(&[("192.168.68.22", dev), ("192.168.68.23", dev)]),
            )
            .unwrap();
        assert_eq!(saved, ScanSaved { added: 1, updated: 1 });
        assert_eq!(rows(&inv), [row("dev.lan", Some(MAC_A)), row("192.168.68.23", Some(MAC_B))]);
    }

    #[test]
    fn a_scan_that_sees_no_hardware_address_still_matches_by_ip() {
        let inv = Inventory::in_memory().unwrap();
        let hall = scan_one(&inv, found("192.168.68.50", Some(MAC_A), Some("deco-hall")));
        inv.update(hall, input("Hallway", "192.168.68.50")).unwrap();
        // Added by hand with its hardware address, on a network behind a router.
        let mut nas = input("NAS", "192.168.70.10");
        nas.mac = Some(MAC_B.into());
        let nas = inv.add(nas).unwrap().id;

        let bare = |ip: &str| Found { vendor: None, ..found(ip, None, None) };
        let saved = scan(&inv, &[bare("192.168.68.50"), bare("192.168.70.10")], 200);
        assert_eq!(saved, ScanSaved { added: 0, updated: 2 });
        let d = inv.get(hall).unwrap();
        assert_eq!((d.name.as_str(), d.mac.as_deref(), d.vendor.as_deref()), ("Hallway", Some(MAC_A), Some(MAKER)));
        assert_eq!((d.hostname.as_deref(), d.last_seen), (Some("deco-hall"), Some(200)));
        let d = inv.get(nas).unwrap();
        assert_eq!((d.name.as_str(), d.mac.as_deref(), d.last_seen), ("NAS", Some(MAC_B), Some(200)));
        assert_eq!(inv.list().unwrap().len(), 2);
    }

    #[test]
    fn with_no_hardware_address_seen_the_likeliest_device_at_that_ip_is_picked() {
        let inv = Inventory::in_memory().unwrap();
        // The address was handed from one device to another.
        let old = scan_one(&inv, found("192.168.68.20", Some(MAC_A), None));
        assert_eq!(scan(&inv, &[found("192.168.68.20", Some(MAC_B), None)], 200), ScanSaved { added: 1, updated: 0 });
        let new = inv.list().unwrap().into_iter().find(|d| d.id != old).unwrap().id;

        // The one seen most recently is the better guess.
        assert_eq!(scan(&inv, &[found("192.168.68.20", None, None)], 300), ScanSaved { added: 0, updated: 1 });
        assert_eq!((inv.get(old).unwrap().last_seen, inv.get(new).unwrap().last_seen), (Some(100), Some(300)));

        // An entry with no hardware address of its own comes before both.
        let hand = inv.add(input("Printer", "192.168.68.20")).unwrap().id;
        assert_eq!(scan(&inv, &[found("192.168.68.20", None, None)], 400), ScanSaved { added: 0, updated: 1 });
        assert_eq!((inv.get(hand).unwrap().last_seen, inv.get(new).unwrap().last_seen), (Some(400), Some(300)));
        assert_eq!(inv.get(hand).unwrap().mac, None);
    }

    #[test]
    fn a_device_that_moved_is_followed_before_its_old_address_is_matched() {
        let inv = Inventory::in_memory().unwrap();
        let id = scan_one(&inv, found("192.168.68.60", Some(MAC_A), None));
        // It moves to .70, and something whose hardware address the scan
        // didn't catch now answers at .60 (listed first, in address order).
        let saved = scan(&inv, &[found("192.168.68.60", None, None), found("192.168.68.70", Some(MAC_A), None)], 200);
        assert_eq!(saved, ScanSaved { added: 1, updated: 1 });
        assert_eq!(inv.get(id).unwrap().host, "192.168.68.70");
        assert_eq!(rows(&inv), [row("192.168.68.70", Some(MAC_A)), row("192.168.68.60", None)]);
    }

    #[test]
    fn upgrades_a_step_2_database() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute("INSERT INTO devices (name, host, kind, os) VALUES ('old', '10.0.0.1', 'pc', 'linux')", [])
            .unwrap();
        let inv = Inventory::from_conn(conn).unwrap();
        let d = inv.list().unwrap().remove(0);
        assert_eq!((d.name.as_str(), d.reviewed, d.source), ("old", true, Source::Manual));
        // Opening again doesn't re-run the upgrade.
        let conn = inv.conn.into_inner().unwrap();
        Inventory::from_conn(conn).unwrap();
    }

    #[test]
    fn replaces_the_identity_table_of_an_early_step_3_build() {
        use crate::ssh::HostKey;
        // As an early test build of step 3 left it: identities by address.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch(&format!(
            "{SCHEMA_V2}
             CREATE TABLE ssh_host_keys (host TEXT NOT NULL, port INTEGER NOT NULL, algorithm TEXT NOT NULL,
                 fingerprint TEXT NOT NULL, PRIMARY KEY (host, port));
             INSERT INTO devices (name, host, kind, os) VALUES ('nas', '192.168.68.20', 'server', 'linux');
             INSERT INTO ssh_host_keys VALUES ('192.168.68.20', 22, 'ssh-ed25519', 'SHA256:old');
             PRAGMA user_version = 2;"
        ))
        .unwrap();
        let inv = Inventory::from_conn(conn).unwrap();
        let nas = inv.list().unwrap().remove(0).id;

        // The old identities are gone, and one can be stored for a device.
        assert_eq!(inv.host_key(nas).unwrap(), None);
        let key = HostKey { algorithm: "ssh-ed25519".into(), fingerprint: "SHA256:new".into() };
        inv.remember_host_key(nas, &key).unwrap();
        assert_eq!(inv.host_key(nas).unwrap(), Some(key.clone()));
        // Changing a device's address, which removes its identity, works too.
        let d = inv.update(nas, input("nas", "192.168.68.21")).unwrap();
        assert_eq!((d.host.as_str(), inv.host_key(nas).unwrap()), ("192.168.68.21", None));

        // Opening again leaves the table and what is in it alone.
        inv.remember_host_key(nas, &key).unwrap();
        let inv = Inventory::from_conn(inv.conn.into_inner().unwrap()).unwrap();
        assert_eq!(inv.host_key(nas).unwrap(), Some(key));
    }

    #[test]
    fn persists_to_file() {
        let dir = std::env::temp_dir().join(format!("home-cc-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("inv.db");
        {
            let inv = Inventory::open(&path).unwrap();
            inv.add(input("nas", "192.168.68.20")).unwrap();
        }
        let inv = Inventory::open(&path).unwrap();
        assert_eq!(inv.list().unwrap()[0].name, "nas");
        std::fs::remove_dir_all(dir).ok();
    }
}
