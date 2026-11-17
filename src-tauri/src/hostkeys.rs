//! SSH server identities, stored the first time a machine is reached so a
//! later change is noticed.
//!
//! An identity belongs to a device, not to an address: it follows the device
//! when a scan finds it at a new IP address, and it goes when the device is
//! removed or the owner points the device at another address or port.
//!
//! An identity is either just remembered (the app noted it by itself after a
//! key login worked) or approved (the owner was shown its fingerprint and
//! said yes). A stored password is only ever sent to an approved identity.

use crate::error::{Error, Result};
use crate::inventory::Inventory;
use crate::ssh::HostKey;
use rusqlite::{OptionalExtension, params};

impl Inventory {
    pub fn host_key(&self, device_id: i64) -> Result<Option<HostKey>> {
        Ok(self
            .conn()
            .query_row("SELECT algorithm, fingerprint FROM ssh_host_keys WHERE device_id = ?1", [device_id], |r| {
                Ok(HostKey { algorithm: r.get(0)?, fingerprint: r.get(1)? })
            })
            .optional()?)
    }

    /// True if the owner has approved the identity stored for this device.
    pub fn host_key_approved(&self, device_id: i64) -> Result<bool> {
        Ok(self
            .conn()
            .query_row("SELECT approved FROM ssh_host_keys WHERE device_id = ?1", [device_id], |r| r.get(0))
            .optional()?
            .unwrap_or(false))
    }

    /// Remember the identity a machine showed at `host` and `port`, as not yet
    /// approved. An identity already stored is kept: it only changes when the
    /// owner approves the new one. Nothing is stored if the device was removed
    /// or pointed somewhere else in the meantime: the identity belongs to
    /// what was at the old address.
    pub fn remember_host_key(&self, device_id: i64, host: &str, port: u16, key: &HostKey) -> Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO ssh_host_keys (device_id, algorithm, fingerprint)
             SELECT ?1, ?2, ?3 WHERE EXISTS
                 (SELECT 1 FROM devices WHERE id = ?1 AND host = ?4 COLLATE NOCASE AND ssh_port = ?5)",
            params![device_id, key.algorithm, key.fingerprint, host, port],
        )?;
        Ok(())
    }

    /// Store the identity the owner approved, in place of any stored before.
    pub fn approve_host_key(&self, device_id: i64, key: &HostKey) -> Result<()> {
        let stored = self.conn().execute(
            "INSERT OR REPLACE INTO ssh_host_keys (device_id, algorithm, fingerprint, approved)
             SELECT ?1, ?2, ?3, 1 WHERE EXISTS (SELECT 1 FROM devices WHERE id = ?1)",
            params![device_id, key.algorithm, key.fingerprint],
        )?;
        if stored == 0 {
            return Err(Error::NotFound(format!("device {device_id}")));
        }
        Ok(())
    }

    pub fn forget_host_key(&self, device_id: i64) -> Result<()> {
        self.conn().execute("DELETE FROM ssh_host_keys WHERE device_id = ?1", [device_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{DeviceInput, DeviceKind, Found, Os};
    use std::collections::HashMap;

    fn key(fp: &str) -> HostKey {
        HostKey { algorithm: "ssh-ed25519".into(), fingerprint: fp.into() }
    }

    fn input(name: &str, host: &str) -> DeviceInput {
        DeviceInput {
            name: name.into(),
            host: host.into(),
            kind: DeviceKind::Server,
            os: Os::Linux,
            ssh_user: Some("jim".into()),
            ssh_port: None,
            notes: String::new(),
            mac: None,
            group: None,
        }
    }

    /// Remember an identity seen at the address the device has right now.
    fn note(inv: &Inventory, device_id: i64, key: &HostKey) -> Result<()> {
        let device = inv.get(device_id);
        let (host, port) = device.map(|d| (d.host, d.ssh_port)).unwrap_or(("gone".into(), 22));
        inv.remember_host_key(device_id, &host, port, key)
    }

    fn pins(inv: &Inventory) -> i64 {
        inv.conn().query_row("SELECT COUNT(*) FROM ssh_host_keys", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn an_identity_seen_at_an_old_address_is_not_stored() {
        let inv = Inventory::in_memory().unwrap();
        let nas = inv.add(input("NAS", "192.168.68.20")).unwrap().id;
        // A reading of .20 was still running when the owner pointed the
        // device at .99: what .20 showed says nothing about .99.
        inv.update(nas, input("NAS", "192.168.68.99")).unwrap();
        inv.remember_host_key(nas, "192.168.68.20", 22, &key("SHA256:old-place")).unwrap();
        assert_eq!(inv.host_key(nas).unwrap(), None);
        inv.remember_host_key(nas, "192.168.68.99", 2222, &key("SHA256:other-port")).unwrap();
        assert_eq!(inv.host_key(nas).unwrap(), None);
        inv.remember_host_key(nas, "192.168.68.99", 22, &key("SHA256:new-place")).unwrap();
        assert_eq!(inv.host_key(nas).unwrap(), Some(key("SHA256:new-place")));
    }

    #[test]
    fn only_the_owner_approves_an_identity() {
        let inv = Inventory::in_memory().unwrap();
        let nas = inv.add(input("NAS", "192.168.68.20")).unwrap().id;
        assert!(!inv.host_key_approved(nas).unwrap());

        // Noted by the app itself: remembered, but not approved.
        note(&inv, nas, &key("SHA256:seen")).unwrap();
        assert!(!inv.host_key_approved(nas).unwrap());

        inv.approve_host_key(nas, &key("SHA256:seen")).unwrap();
        assert!(inv.host_key_approved(nas).unwrap());
        // Remembering again doesn't undo the approval.
        note(&inv, nas, &key("SHA256:other")).unwrap();
        assert!(inv.host_key_approved(nas).unwrap());
        assert_eq!(inv.host_key(nas).unwrap(), Some(key("SHA256:seen")));

        // Pointing the device somewhere else starts again from nothing.
        inv.update(nas, input("NAS", "192.168.68.99")).unwrap();
        assert_eq!(inv.host_key(nas).unwrap(), None);
        assert!(!inv.host_key_approved(nas).unwrap());
    }

    #[test]
    fn first_identity_is_kept_until_forgotten_or_replaced() {
        let inv = Inventory::in_memory().unwrap();
        let nas = inv.add(input("NAS", "nas.lan")).unwrap().id;
        let other = inv.add(input("Homelab", "192.168.68.21")).unwrap().id;
        assert_eq!(inv.host_key(nas).unwrap(), None);
        note(&inv, nas, &key("SHA256:first")).unwrap();
        note(&inv, nas, &key("SHA256:second")).unwrap();
        assert_eq!(inv.host_key(nas).unwrap(), Some(key("SHA256:first")));
        // Each device has its own.
        assert_eq!(inv.host_key(other).unwrap(), None);

        inv.approve_host_key(nas, &key("SHA256:approved")).unwrap();
        assert_eq!(inv.host_key(nas).unwrap(), Some(key("SHA256:approved")));
        assert_eq!(pins(&inv), 1);

        inv.forget_host_key(nas).unwrap();
        assert_eq!(inv.host_key(nas).unwrap(), None);
        note(&inv, nas, &key("SHA256:second")).unwrap();
        assert_eq!(inv.host_key(nas).unwrap(), Some(key("SHA256:second")));
        // Replacing also works when nothing was stored, and forgetting twice is fine.
        inv.approve_host_key(other, &key("SHA256:other")).unwrap();
        assert_eq!(inv.host_key(other).unwrap(), Some(key("SHA256:other")));
        inv.forget_host_key(other).unwrap();
        inv.forget_host_key(other).unwrap();
    }

    #[test]
    fn a_removed_device_has_no_identity() {
        let inv = Inventory::in_memory().unwrap();
        let nas = inv.add(input("NAS", "192.168.68.20")).unwrap().id;
        note(&inv, nas, &key("SHA256:nas")).unwrap();
        inv.remove(nas).unwrap();
        assert_eq!(pins(&inv), 0);

        // A reading that finishes after the device was removed stores nothing.
        note(&inv, nas, &key("SHA256:late")).unwrap();
        assert_eq!(inv.host_key(nas).unwrap(), None);
        assert!(matches!(inv.approve_host_key(nas, &key("SHA256:late")), Err(Error::NotFound(_))));
        assert_eq!(pins(&inv), 0);

        // A machine added later at the same address starts fresh.
        let again = inv.add(input("Another", "192.168.68.20")).unwrap().id;
        assert_eq!(inv.host_key(again).unwrap(), None);
    }

    #[test]
    fn the_identity_follows_a_device_the_scan_finds_at_a_new_address() {
        let inv = Inventory::in_memory().unwrap();
        let mut nas = input("NAS", "192.168.68.20");
        nas.mac = Some("aa:bb:cc:00:00:20".into());
        let nas = inv.add(nas).unwrap().id;
        let mut lab = input("Homelab", "192.168.68.21");
        lab.mac = Some("aa:bb:cc:00:00:21".into());
        let lab = inv.add(lab).unwrap().id;
        note(&inv, nas, &key("SHA256:nas")).unwrap();
        note(&inv, lab, &key("SHA256:lab")).unwrap();

        // The router hands the NAS a new address and gives its old one to the homelab.
        let found = |ip: &str, mac: &str| Found {
            ip: ip.into(),
            mac: Some(mac.into()),
            vendor: None,
            hostname: None,
            looks_like_mesh: false,
            rtt_ms: None,
            is_self: false,
        };
        inv.record_scan(
            &[found("192.168.68.20", "aa:bb:cc:00:00:21"), found("192.168.68.25", "aa:bb:cc:00:00:20")],
            100,
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(inv.get(nas).unwrap().host, "192.168.68.25");
        assert_eq!(inv.get(lab).unwrap().host, "192.168.68.20");
        assert_eq!(inv.host_key(nas).unwrap(), Some(key("SHA256:nas")));
        assert_eq!(inv.host_key(lab).unwrap(), Some(key("SHA256:lab")));
    }

    #[test]
    fn pointing_a_device_somewhere_else_forgets_its_identity() {
        let inv = Inventory::in_memory().unwrap();
        let nas = inv.add(input("NAS", "192.168.68.20")).unwrap().id;
        note(&inv, nas, &key("SHA256:nas")).unwrap();

        // A new name or type is the same machine.
        let mut renamed = input("Big NAS", "192.168.68.20");
        renamed.kind = DeviceKind::Pc;
        inv.update(nas, renamed).unwrap();
        assert_eq!(inv.host_key(nas).unwrap(), Some(key("SHA256:nas")));

        // Another address is not.
        inv.update(nas, input("Big NAS", "192.168.68.30")).unwrap();
        assert_eq!(inv.host_key(nas).unwrap(), None);

        // Nor is another port at the same address.
        note(&inv, nas, &key("SHA256:nas")).unwrap();
        let mut other_port = input("Big NAS", "192.168.68.30");
        other_port.ssh_port = Some(2222);
        inv.update(nas, other_port).unwrap();
        assert_eq!(inv.host_key(nas).unwrap(), None);

        // Saving the same address and port again keeps it.
        note(&inv, nas, &key("SHA256:nas")).unwrap();
        let mut same = input("Big NAS", " 192.168.68.30 ");
        same.ssh_port = Some(2222);
        inv.update(nas, same).unwrap();
        assert_eq!(inv.host_key(nas).unwrap(), Some(key("SHA256:nas")));
    }
}
