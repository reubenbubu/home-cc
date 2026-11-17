//! Which devices are Pi-holes, and how home-cc reaches each: HTTPS or HTTP,
//! the port, and the certificate the owner approved. A row here is what makes
//! a device a Pi-hole; it goes when the device is removed. The password is in
//! the keychain, never here.

use rusqlite::{OptionalExtension, params};

use super::client::{HTTPS_PORT, Scheme};
use crate::error::{Error, Result};
use crate::inventory::Inventory;

const TABLE: &str = "
CREATE TABLE IF NOT EXISTS pihole_hosts (
    device_id   INTEGER PRIMARY KEY REFERENCES devices (id) ON DELETE CASCADE,
    -- 1: HTTPS with the approved certificate. 0: plain HTTP, chosen by the owner.
    https       INTEGER NOT NULL DEFAULT 1,
    port        INTEGER NOT NULL DEFAULT 443,
    -- SHA-256 of the certificate the owner approved, as 'AB:CD:...'.
    certificate TEXT
);
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostConfig {
    pub scheme: Scheme,
    pub port: u16,
    pub certificate: Option<String>,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self { scheme: Scheme::Https, port: HTTPS_PORT, certificate: None }
    }
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, HostConfig)> {
    let https: i64 = r.get(1)?;
    let port: i64 = r.get(2)?;
    Ok((
        r.get(0)?,
        HostConfig {
            scheme: if https != 0 { Scheme::Https } else { Scheme::Http },
            port: u16::try_from(port).ok().filter(|p| *p > 0).unwrap_or(HTTPS_PORT),
            certificate: r.get(3)?,
        },
    ))
}

impl Inventory {
    /// Makes the table if it isn't there yet. Called once at start.
    pub fn pihole_init(&self) -> Result<()> {
        self.conn().execute_batch(TABLE)?;
        Ok(())
    }

    /// Every device marked as a Pi-hole, by device ID.
    pub fn pihole_hosts(&self) -> Result<Vec<(i64, HostConfig)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT device_id, https, port, certificate FROM pihole_hosts ORDER BY device_id")?;
        let rows = stmt.query_map([], row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// One Pi-hole's settings, or None if the device isn't marked as one.
    pub fn pihole_host(&self, device_id: i64) -> Result<Option<HostConfig>> {
        let found = self
            .conn()
            .query_row("SELECT device_id, https, port, certificate FROM pihole_hosts WHERE device_id = ?1", [device_id], row)
            .optional()?;
        Ok(found.map(|(_, c)| c))
    }

    /// Marks a device as a Pi-hole, with HTTPS on 443. Marking it again changes nothing.
    pub fn add_pihole(&self, device_id: i64) -> Result<()> {
        let stored = self.conn().execute(
            "INSERT INTO pihole_hosts (device_id) SELECT ?1 WHERE EXISTS (SELECT 1 FROM devices WHERE id = ?1)
             ON CONFLICT (device_id) DO NOTHING",
            [device_id],
        )?;
        if stored == 0 && self.pihole_host(device_id)?.is_none() {
            return Err(Error::NotFound(format!("device {device_id}")));
        }
        Ok(())
    }

    pub fn remove_pihole(&self, device_id: i64) -> Result<()> {
        self.conn().execute("DELETE FROM pihole_hosts WHERE device_id = ?1", [device_id])?;
        Ok(())
    }

    /// Saves HTTPS or HTTP and the port. Either one changed drops the approved certificate: whatever answers
    /// there is another server as far as home-cc knows, as for Proxmox and SSH.
    pub fn save_pihole(&self, device_id: i64, scheme: Scheme, port: u16) -> Result<()> {
        let https = i64::from(scheme == Scheme::Https);
        let stored = self.conn().execute(
            "UPDATE pihole_hosts SET https = ?2, port = ?3,
             certificate = CASE WHEN https = ?2 AND port = ?3 THEN certificate END
             WHERE device_id = ?1",
            params![device_id, https, port],
        )?;
        if stored == 0 {
            return Err(Error::NotFound(format!("Pi-hole {device_id}")));
        }
        Ok(())
    }

    /// Stores the certificate the owner approved, in place of any before it.
    pub fn approve_pihole_certificate(&self, device_id: i64, fingerprint: &str) -> Result<()> {
        let stored = self
            .conn()
            .execute("UPDATE pihole_hosts SET certificate = ?2 WHERE device_id = ?1", params![device_id, fingerprint])?;
        if stored == 0 {
            return Err(Error::NotFound(format!("Pi-hole {device_id}")));
        }
        Ok(())
    }

    /// For when the device's address changes: the next contact is a first contact again.
    pub fn forget_pihole_certificate(&self, device_id: i64) -> Result<()> {
        self.conn().execute("UPDATE pihole_hosts SET certificate = NULL WHERE device_id = ?1", [device_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{DeviceInput, DeviceKind, Os};

    fn device(inv: &Inventory) -> i64 {
        inv.add(DeviceInput {
            name: "Pi-hole 1".into(),
            host: "192.168.68.31".into(),
            kind: DeviceKind::Server,
            os: Os::Linux,
            ssh_user: None,
            ssh_port: None,
            notes: String::new(),
            mac: None,
            group: None,
        })
        .unwrap()
        .id
    }

    #[test]
    fn marking_settings_and_certificate() {
        let inv = Inventory::in_memory().unwrap();
        inv.pihole_init().unwrap();
        inv.pihole_init().unwrap();
        let id = device(&inv);
        assert_eq!(inv.pihole_host(id).unwrap(), None);
        assert!(inv.save_pihole(id, Scheme::Https, 443).is_err(), "not a Pi-hole yet");

        inv.add_pihole(id).unwrap();
        inv.add_pihole(id).unwrap();
        assert_eq!(inv.pihole_host(id).unwrap(), Some(HostConfig::default()));
        assert_eq!(inv.pihole_hosts().unwrap(), vec![(id, HostConfig::default())]);

        inv.approve_pihole_certificate(id, "AB:CD").unwrap();
        // The same scheme and port keep the approval; another one drops it.
        inv.save_pihole(id, Scheme::Https, 443).unwrap();
        assert_eq!(inv.pihole_host(id).unwrap().unwrap().certificate.as_deref(), Some("AB:CD"));
        inv.save_pihole(id, Scheme::Https, 8443).unwrap();
        assert_eq!(inv.pihole_host(id).unwrap().unwrap().certificate, None);
        inv.approve_pihole_certificate(id, "AB:CD").unwrap();
        inv.save_pihole(id, Scheme::Http, 8443).unwrap();
        assert_eq!(
            inv.pihole_host(id).unwrap(),
            Some(HostConfig { scheme: Scheme::Http, port: 8443, certificate: None })
        );
        inv.approve_pihole_certificate(id, "EF").unwrap();
        inv.forget_pihole_certificate(id).unwrap();
        assert_eq!(inv.pihole_host(id).unwrap().unwrap().certificate, None);

        inv.remove_pihole(id).unwrap();
        assert_eq!(inv.pihole_host(id).unwrap(), None);
        inv.add_pihole(id).unwrap();
        // The device going takes its row with it.
        inv.remove(id).unwrap();
        assert!(inv.pihole_hosts().unwrap().is_empty());
        assert!(matches!(inv.add_pihole(id), Err(Error::NotFound(_))));
    }
}
