//! SSH server identities, stored the first time a machine is reached so a
//! later change is noticed.

use crate::error::Result;
use crate::inventory::Inventory;
use crate::ssh::HostKey;
use rusqlite::{OptionalExtension, params};

impl Inventory {
    pub fn host_key(&self, host: &str, port: u16) -> Result<Option<HostKey>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT algorithm, fingerprint FROM ssh_host_keys WHERE host = ?1 AND port = ?2",
                params![host, port],
                |r| Ok(HostKey { algorithm: r.get(0)?, fingerprint: r.get(1)? }),
            )
            .optional()?)
    }

    /// Remember a machine's identity. An identity already stored is kept: it
    /// only changes after the owner asks to forget the old one.
    pub fn remember_host_key(&self, host: &str, port: u16, key: &HostKey) -> Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO ssh_host_keys (host, port, algorithm, fingerprint) VALUES (?1, ?2, ?3, ?4)",
            params![host, port, key.algorithm, key.fingerprint],
        )?;
        Ok(())
    }

    pub fn forget_host_key(&self, host: &str, port: u16) -> Result<()> {
        self.conn().execute("DELETE FROM ssh_host_keys WHERE host = ?1 AND port = ?2", params![host, port])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(fp: &str) -> HostKey {
        HostKey { algorithm: "ssh-ed25519".into(), fingerprint: fp.into() }
    }

    #[test]
    fn first_identity_is_kept_until_forgotten() {
        let inv = Inventory::in_memory().unwrap();
        assert_eq!(inv.host_key("nas.lan", 22).unwrap(), None);
        inv.remember_host_key("nas.lan", 22, &key("SHA256:first")).unwrap();
        inv.remember_host_key("nas.lan", 22, &key("SHA256:second")).unwrap();
        assert_eq!(inv.host_key("nas.lan", 22).unwrap(), Some(key("SHA256:first")));
        // Another port on the same machine is a different server.
        assert_eq!(inv.host_key("nas.lan", 2222).unwrap(), None);

        inv.forget_host_key("nas.lan", 22).unwrap();
        inv.remember_host_key("nas.lan", 22, &key("SHA256:second")).unwrap();
        assert_eq!(inv.host_key("nas.lan", 22).unwrap(), Some(key("SHA256:second")));
    }
}
