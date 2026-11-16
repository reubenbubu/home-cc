//! The inventory: every device the owner has added or the scanner has found.
//! Stored in a local SQLite file in the user's app data folder.

use crate::error::{Error, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

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
    /// Hardware address. Left as it is when not given.
    #[serde(default)]
    pub mac: Option<String>,
}

impl DeviceInput {
    fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(Error::Invalid("Name can't be empty".into()));
        }
        if self.host.trim().is_empty() {
            return Err(Error::Invalid("Address can't be empty".into()));
        }
        if self.host.trim().contains(char::is_whitespace) {
            return Err(Error::Invalid("Address can't contain spaces".into()));
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

-- SSH host keys, trusted the first time a machine is reached.
CREATE TABLE ssh_host_keys (
    host        TEXT NOT NULL,
    port        INTEGER NOT NULL,
    algorithm   TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    PRIMARY KEY (host, port)
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
            conn.execute_batch(&format!("BEGIN; {SCHEMA_V2} PRAGMA user_version = 2; COMMIT;"))?;
        }
        Ok(Self { conn: Mutex::new(conn) })
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

    pub fn update(&self, id: i64, input: DeviceInput) -> Result<Device> {
        input.validate()?;
        let changed = {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                // Saving a device counts as the owner having looked at it.
                "UPDATE devices SET name = ?1, host = ?2, kind = ?3, os = ?4,
                 ssh_user = ?5, ssh_port = ?6, notes = ?7, mac = COALESCE(?8, mac), reviewed = 1
                 WHERE id = ?9",
                params![
                    input.name.trim(),
                    input.host.trim(),
                    input.kind.as_str(),
                    input.os.as_str(),
                    clean(input.ssh_user),
                    input.ssh_port.unwrap_or(22),
                    input.notes,
                    clean(input.mac).and_then(|m| normalize_mac(&m)),
                    id,
                ],
            )?
        };
        if changed == 0 {
            return Err(Error::NotFound(format!("device {id}")));
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

    /// Save what a scan found. A device already in the inventory is matched by
    /// hardware address first, then by IP address, and keeps the name and type
    /// the owner gave it. Anything else is added as new and unreviewed.
    pub fn record_scan(&self, found: &[Found], at: i64) -> Result<ScanSaved> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut saved = ScanSaved::default();
        for f in found {
            let mac = f.mac.as_deref().and_then(normalize_mac);
            let by_mac: Option<(i64, String)> = match &mac {
                Some(m) => tx
                    .query_row("SELECT id, host FROM devices WHERE mac = ?1 ORDER BY id LIMIT 1", [m], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })
                    .optional()?,
                None => None,
            };
            let existing = match by_mac {
                Some(hit) => Some(hit),
                // Only fall back to the address for a device with no known
                // hardware address; otherwise a reused IP would take over the
                // entry of the device that had it before.
                None => tx
                    .query_row(
                        "SELECT id, host FROM devices WHERE host = ?1 AND mac IS NULL ORDER BY id LIMIT 1",
                        [&f.ip],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?,
            };
            match existing {
                Some((id, host)) => {
                    // Follow the device to its new IP, but leave a host name
                    // the owner typed (such as "nas.lan") alone.
                    let new_host = if host.parse::<std::net::IpAddr>().is_ok() { f.ip.as_str() } else { host.as_str() };
                    tx.execute(
                        "UPDATE devices SET host = ?1, mac = COALESCE(?2, mac), vendor = COALESCE(?3, vendor),
                         hostname = COALESCE(?4, hostname), last_seen = ?5 WHERE id = ?6",
                        params![new_host, mac, f.vendor, f.hostname, at, id],
                    )?;
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
                    saved.added += 1;
                }
            }
        }
        tx.commit()?;
        Ok(saved)
    }
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
            .record_scan(&[found("192.168.68.50", Some("AA:BB:CC:00:00:01"), Some("deco-hall"))], 100)
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
            .record_scan(&[found("192.168.68.77", Some("aa:bb:cc:00:00:01"), Some("deco-hall"))], 200)
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
        let saved = inv.record_scan(&[found("192.168.68.20", Some("aa:bb:cc:00:00:03"), None)], 400).unwrap();
        assert_eq!(saved, ScanSaved { added: 1, updated: 0 });
        assert_eq!(inv.get(by_ip.id).unwrap().last_seen, Some(300));
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
