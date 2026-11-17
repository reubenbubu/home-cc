//! What home-cc keeps about each Proxmox host in the database: the API
//! token's ID (not its secret, which is in the keychain), the API port, and
//! the certificate the owner approved. A device's row goes when the device is
//! removed.

use rusqlite::{OptionalExtension, params};

use super::client::DEFAULT_PORT;
use crate::error::{Error, Result};
use crate::inventory::Inventory;

const TABLE: &str = "
CREATE TABLE IF NOT EXISTS proxmox_hosts (
    device_id   INTEGER PRIMARY KEY REFERENCES devices (id) ON DELETE CASCADE,
    -- Such as 'root@pam!homecc'. The secret is in the keychain.
    token_id    TEXT,
    port        INTEGER NOT NULL DEFAULT 8006,
    -- SHA-256 of the certificate the owner approved, as 'AB:CD:...'.
    certificate TEXT
);
";

/// One Proxmox host's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostConfig {
    pub token_id: Option<String>,
    pub port: u16,
    pub certificate: Option<String>,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self { token_id: None, port: DEFAULT_PORT, certificate: None }
    }
}

impl Inventory {
    /// Makes the table if it isn't there yet. Called once at start.
    pub fn proxmox_init(&self) -> Result<()> {
        self.conn().execute_batch(TABLE)?;
        Ok(())
    }

    /// The settings for one device; the defaults if none are saved.
    pub fn proxmox_host(&self, device_id: i64) -> Result<HostConfig> {
        let row = self
            .conn()
            .query_row(
                "SELECT token_id, port, certificate FROM proxmox_hosts WHERE device_id = ?1",
                [device_id],
                |r| {
                    let port: i64 = r.get(1)?;
                    Ok(HostConfig {
                        token_id: r.get(0)?,
                        port: u16::try_from(port).ok().filter(|p| *p > 0).unwrap_or(DEFAULT_PORT),
                        certificate: r.get(2)?,
                    })
                },
            )
            .optional()?;
        Ok(row.unwrap_or_default())
    }

    /// Saves the token ID and port. A new token keeps the approved
    /// certificate: it belongs to the host, not to the token. A new port
    /// doesn't: whatever answers there is another server as far as home-cc
    /// knows, so the first-contact check applies again, as it does for an
    /// SSH server at another port.
    pub fn save_proxmox_host(&self, device_id: i64, token_id: Option<&str>, port: u16) -> Result<()> {
        let stored = self.conn().execute(
            "INSERT INTO proxmox_hosts (device_id, token_id, port)
             SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM devices WHERE id = ?1)
             ON CONFLICT (device_id) DO UPDATE SET token_id = excluded.token_id, port = excluded.port,
             certificate = CASE WHEN proxmox_hosts.port = excluded.port THEN proxmox_hosts.certificate END",
            params![device_id, token_id, port],
        )?;
        if stored == 0 {
            return Err(Error::NotFound(format!("device {device_id}")));
        }
        Ok(())
    }

    /// Stores the certificate the owner approved, in place of any before it.
    pub fn approve_proxmox_certificate(&self, device_id: i64, fingerprint: &str) -> Result<()> {
        let stored = self.conn().execute(
            "INSERT INTO proxmox_hosts (device_id, certificate)
             SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM devices WHERE id = ?1)
             ON CONFLICT (device_id) DO UPDATE SET certificate = excluded.certificate",
            params![device_id, fingerprint],
        )?;
        if stored == 0 {
            return Err(Error::NotFound(format!("device {device_id}")));
        }
        Ok(())
    }

    /// Forgets the approved certificate, so the next contact is a first
    /// contact again. For when the device's address changes.
    pub fn forget_proxmox_certificate(&self, device_id: i64) -> Result<()> {
        self.conn().execute("UPDATE proxmox_hosts SET certificate = NULL WHERE device_id = ?1", [device_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{DeviceInput, DeviceKind, Os};

    fn proxmox(inv: &Inventory) -> i64 {
        inv.add(DeviceInput {
            name: "Proxmox".into(),
            host: "192.168.68.30".into(),
            kind: DeviceKind::Server,
            os: Os::Proxmox,
            ssh_user: None,
            ssh_port: None,
            notes: String::new(),
            mac: None,
        })
        .unwrap()
        .id
    }

    #[test]
    fn settings_and_certificate_are_kept_apart_and_go_with_the_device() {
        let inv = Inventory::in_memory().unwrap();
        inv.proxmox_init().unwrap();
        // Twice is fine: it runs at every start.
        inv.proxmox_init().unwrap();
        let id = proxmox(&inv);
        assert_eq!(inv.proxmox_host(id).unwrap(), HostConfig::default());

        inv.approve_proxmox_certificate(id, "AB:CD").unwrap();
        inv.save_proxmox_host(id, Some("root@pam!homecc"), 8006).unwrap();
        assert_eq!(
            inv.proxmox_host(id).unwrap(),
            HostConfig { token_id: Some("root@pam!homecc".into()), port: 8006, certificate: Some("AB:CD".into()) }
        );
        // A new token keeps the approved certificate; a new approval keeps the token.
        inv.save_proxmox_host(id, Some("homecc@pve!screen"), 8006).unwrap();
        assert_eq!(inv.proxmox_host(id).unwrap().certificate.as_deref(), Some("AB:CD"));
        inv.approve_proxmox_certificate(id, "EF:01").unwrap();
        assert_eq!(
            inv.proxmox_host(id).unwrap(),
            HostConfig { token_id: Some("homecc@pve!screen".into()), port: 8006, certificate: Some("EF:01".into()) }
        );
        // Another port is another server: its certificate is checked afresh.
        inv.save_proxmox_host(id, Some("homecc@pve!screen"), 8443).unwrap();
        assert_eq!(
            inv.proxmox_host(id).unwrap(),
            HostConfig { token_id: Some("homecc@pve!screen".into()), port: 8443, certificate: None }
        );
        // So is another address.
        inv.approve_proxmox_certificate(id, "EF:01").unwrap();
        inv.forget_proxmox_certificate(id).unwrap();
        assert_eq!(inv.proxmox_host(id).unwrap().certificate, None);
        assert_eq!(inv.proxmox_host(id).unwrap().token_id.as_deref(), Some("homecc@pve!screen"));

        inv.remove(id).unwrap();
        assert_eq!(inv.proxmox_host(id).unwrap(), HostConfig::default());
        assert!(matches!(inv.save_proxmox_host(id, None, 8006), Err(Error::NotFound(_))));
        assert!(matches!(inv.approve_proxmox_certificate(id, "AB:CD"), Err(Error::NotFound(_))));
    }
}
