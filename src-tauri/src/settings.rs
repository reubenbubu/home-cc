//! App settings, kept in the same SQLite file as the inventory.

use crate::error::{Error, Result};
use crate::inventory::Inventory;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Seconds between pings of every known device.
    pub ping_interval_secs: u32,
    /// Seconds between stats readings from each machine.
    pub stats_interval_secs: u32,
    /// IP range to scan, such as "192.168.68.0/22". Empty means work it out.
    pub scan_range: String,
    /// A disk fuller than this (percent) is shown as a warning.
    pub disk_warn_pct: u8,
    /// RAM use above this (percent) is shown as a warning.
    pub ram_warn_pct: u8,
    /// Private key file for SSH. Empty means the usual ones in ~/.ssh.
    pub ssh_key_path: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ping_interval_secs: 30,
            stats_interval_secs: 60,
            scan_range: String::new(),
            disk_warn_pct: 85,
            ram_warn_pct: 90,
            ssh_key_path: String::new(),
        }
    }
}

impl Settings {
    fn validate(&self) -> Result<()> {
        if !(5..=3600).contains(&self.ping_interval_secs) {
            return Err(Error::Invalid("Ping timer must be between 5 seconds and 1 hour".into()));
        }
        if !(10..=3600).contains(&self.stats_interval_secs) {
            return Err(Error::Invalid("Stats timer must be between 10 seconds and 1 hour".into()));
        }
        if !(1..=100).contains(&self.disk_warn_pct) || !(1..=100).contains(&self.ram_warn_pct) {
            return Err(Error::Invalid("Warning levels must be between 1 and 100 percent".into()));
        }
        Ok(())
    }
}

const KEY: &str = "settings";

/// A path without the quotes that came along when it was copied: Windows'
/// "Copy as path" wraps it in double quotes, and a file dragged into a Linux
/// terminal gets single ones. Only one matching pair is taken off.
fn unquote(path: &str) -> &str {
    let path = path.trim();
    ['"', '\'']
        .into_iter()
        .find_map(|quote| path.strip_prefix(quote)?.strip_suffix(quote))
        .map_or(path, str::trim)
}

impl Inventory {
    /// Missing or unreadable settings fall back to the defaults.
    pub fn settings(&self) -> Result<Settings> {
        use rusqlite::OptionalExtension;
        let json: Option<String> = self
            .conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", [KEY], |r| r.get(0))
            .optional()?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default())
    }

    pub fn save_settings(&self, mut settings: Settings) -> Result<Settings> {
        settings.scan_range = settings.scan_range.trim().to_string();
        settings.ssh_key_path = unquote(&settings.ssh_key_path).to_string();
        settings.validate()?;
        let json = serde_json::to_string(&settings).expect("settings serialise");
        self.conn().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![KEY, json],
        )?;
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_then_saved_values() {
        let inv = Inventory::in_memory().unwrap();
        assert_eq!(inv.settings().unwrap(), Settings::default());
        let s = Settings { ping_interval_secs: 10, scan_range: " 10.0.0.0/24 ".into(), ..Settings::default() };
        inv.save_settings(s).unwrap();
        let got = inv.settings().unwrap();
        assert_eq!((got.ping_interval_secs, got.scan_range.as_str()), (10, "10.0.0.0/24"));
    }

    #[test]
    fn a_key_path_pasted_with_quotes_is_stored_without_them() {
        let inv = Inventory::in_memory().unwrap();
        let cases = [
            // Windows 11 "Copy as path".
            (r#""C:\Users\jim\.ssh\id_ed25519""#, r"C:\Users\jim\.ssh\id_ed25519"),
            (r#"  "C:\Users\jim\My Keys\home key"  "#, r"C:\Users\jim\My Keys\home key"),
            // A file dragged into a Linux terminal.
            ("'/home/jim/.ssh/id_ed25519'", "/home/jim/.ssh/id_ed25519"),
            ("\" /home/jim/.ssh/id_rsa \"", "/home/jim/.ssh/id_rsa"),
            // Nothing to take off.
            ("/home/jim/.ssh/id_ed25519", "/home/jim/.ssh/id_ed25519"),
            ("  ~/.ssh/it's mine  ", "~/.ssh/it's mine"),
            ("", ""),
            // Only a matching pair around the whole path counts, and only one.
            ("\"/home/jim/key", "\"/home/jim/key"),
            ("/home/jim/key'", "/home/jim/key'"),
            ("\"/home/jim/key'", "\"/home/jim/key'"),
            ("\"", "\""),
            ("\"\"", ""),
            ("\"'/home/jim/key'\"", "'/home/jim/key'"),
        ];
        for (typed, stored) in cases {
            let saved = inv.save_settings(Settings { ssh_key_path: typed.into(), ..Settings::default() }).unwrap();
            assert_eq!(saved.ssh_key_path, stored, "returned for {typed:?}");
            assert_eq!(inv.settings().unwrap().ssh_key_path, stored, "stored for {typed:?}");
        }
    }

    #[test]
    fn rejects_out_of_range_values() {
        let inv = Inventory::in_memory().unwrap();
        let bad = Settings { ping_interval_secs: 1, ..Settings::default() };
        assert!(matches!(inv.save_settings(bad), Err(Error::Invalid(_))));
        let bad = Settings { disk_warn_pct: 0, ..Settings::default() };
        assert!(matches!(inv.save_settings(bad), Err(Error::Invalid(_))));
    }

    #[test]
    fn settings_added_later_get_defaults() {
        let inv = Inventory::in_memory().unwrap();
        inv.conn()
            .execute("INSERT INTO settings (key, value) VALUES ('settings', '{\"pingIntervalSecs\":45}')", [])
            .unwrap();
        let got = inv.settings().unwrap();
        assert_eq!((got.ping_interval_secs, got.disk_warn_pct), (45, 85));
    }
}
