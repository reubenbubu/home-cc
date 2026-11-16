//! Passwords, SSH key passphrases and API tokens live in the operating system's
//! keychain (Windows Credential Manager, or the Secret Service on Linux), never
//! in the database or config files. The screens can store, check and delete a
//! secret, but never read one back.

use crate::error::{Error, Result};

const SERVICE: &str = "home-cc";

fn entry(key: &str) -> Result<keyring::Entry> {
    if key.trim().is_empty() {
        return Err(Error::Invalid("Secret name can't be empty".into()));
    }
    Ok(keyring::Entry::new(SERVICE, key)?)
}

pub fn set(key: &str, value: &str) -> Result<()> {
    entry(key)?.set_password(value)?;
    Ok(())
}

pub fn has(key: &str) -> Result<bool> {
    match entry(key)?.get_password() {
        Ok(_) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

pub fn delete(key: &str) -> Result<()> {
    match entry(key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// For back-end use only (SSH, Proxmox). Not exposed as a command.
#[allow(dead_code)]
pub fn get(key: &str) -> Result<Option<String>> {
    match entry(key)?.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
