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
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "pc" => DeviceKind::Pc,
            "server" => DeviceKind::Server,
            "vm" => DeviceKind::Vm,
            "container" => DeviceKind::Container,
            "mesh" => DeviceKind::Mesh,
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

impl Inventory {
    pub fn open(path: &Path) -> Result<Self> {
        Self::from_conn(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        Self::from_conn(Connection::open_in_memory()?)
    }

    fn from_conn(conn: Connection) -> Result<Self> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn list(&self) -> Result<Vec<Device>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, host, kind, os, ssh_user, ssh_port, notes
             FROM devices ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], row_to_device)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get(&self, id: i64) -> Result<Device> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, name, host, kind, os, ssh_user, ssh_port, notes FROM devices WHERE id = ?1",
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
                "INSERT INTO devices (name, host, kind, os, ssh_user, ssh_port, notes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    input.name.trim(),
                    input.host.trim(),
                    input.kind.as_str(),
                    input.os.as_str(),
                    clean(input.ssh_user),
                    input.ssh_port.unwrap_or(22),
                    input.notes,
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
                "UPDATE devices SET name = ?1, host = ?2, kind = ?3, os = ?4,
                 ssh_user = ?5, ssh_port = ?6, notes = ?7 WHERE id = ?8",
                params![
                    input.name.trim(),
                    input.host.trim(),
                    input.kind.as_str(),
                    input.os.as_str(),
                    clean(input.ssh_user),
                    input.ssh_port.unwrap_or(22),
                    input.notes,
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
