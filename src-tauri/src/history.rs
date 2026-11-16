//! Ping results: the latest status of each device and a short history, so
//! the owner can see if something drops out now and then.

use crate::error::Result;
use crate::inventory::Inventory;
use rusqlite::params;
use serde::Serialize;
use std::collections::BTreeMap;

/// How long ping results are kept.
pub const KEEP_SECS: i64 = 24 * 60 * 60;
/// How many of the latest results the screens get for each device.
pub const SAMPLES: usize = 120;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    /// Unix seconds.
    pub ts: i64,
    pub up: bool,
    /// Response time in milliseconds. A device that is up but doesn't answer
    /// ping (many Windows PCs) has none.
    pub rtt_ms: Option<f64>,
}

/// The outcome of checking one device.
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub device_id: i64,
    pub up: bool,
    pub rtt_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStatus {
    pub device_id: i64,
    /// Whether the device was there at the latest check.
    pub up: bool,
    pub rtt_ms: Option<f64>,
    /// Unix seconds of the latest ping.
    pub checked_at: i64,
    /// Unix seconds of the last answer to a ping or scan.
    pub last_seen: Option<i64>,
    /// Latest results, oldest first.
    pub history: Vec<Sample>,
}

impl Inventory {
    /// Store one round of checks.
    pub fn record_pings(&self, results: &[Check], at: i64) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for Check { device_id: id, up, rtt_ms: rtt } in results {
            // A device removed while the round was running is skipped.
            let known = tx.execute(
                "INSERT INTO pings (device_id, ts, up, rtt_ms)
                 SELECT ?1, ?2, ?3, ?4 WHERE EXISTS (SELECT 1 FROM devices WHERE id = ?1)",
                params![id, at, up, rtt],
            )?;
            if known > 0 && *up {
                tx.execute("UPDATE devices SET last_seen = ?1 WHERE id = ?2", params![at, id])?;
            }
        }
        tx.execute("DELETE FROM pings WHERE ts < ?1", [at - KEEP_SECS])?;
        tx.commit()?;
        Ok(())
    }

    /// Latest status and recent history of every device that has been pinged.
    pub fn statuses(&self) -> Result<Vec<DeviceStatus>> {
        let conn = self.conn();
        let mut by_device: BTreeMap<i64, Vec<Sample>> = BTreeMap::new();
        let mut stmt = conn.prepare("SELECT device_id, ts, up, rtt_ms FROM pings ORDER BY device_id, ts")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, Sample { ts: r.get(1)?, up: r.get(2)?, rtt_ms: r.get(3)? }))
        })?;
        for row in rows {
            let (id, sample) = row?;
            by_device.entry(id).or_default().push(sample);
        }
        let mut seen = conn.prepare("SELECT last_seen FROM devices WHERE id = ?1")?;
        let mut out = Vec::with_capacity(by_device.len());
        for (device_id, mut history) in by_device {
            let Some(latest) = history.last().cloned() else { continue };
            if history.len() > SAMPLES {
                history.drain(..history.len() - SAMPLES);
            }
            out.push(DeviceStatus {
                device_id,
                up: latest.up,
                rtt_ms: latest.rtt_ms,
                checked_at: latest.ts,
                last_seen: seen.query_row([device_id], |r| r.get(0))?,
                history,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{DeviceInput, DeviceKind, Os};

    fn device(inv: &Inventory, name: &str) -> i64 {
        let input = DeviceInput {
            name: name.into(),
            host: "10.0.0.1".into(),
            kind: DeviceKind::Pc,
            os: Os::Linux,
            ssh_user: None,
            ssh_port: None,
            notes: String::new(),
            mac: None,
        };
        inv.add(input).unwrap().id
    }

    fn up(device_id: i64, rtt: f64) -> Check {
        Check { device_id, up: true, rtt_ms: Some(rtt) }
    }

    fn down(device_id: i64) -> Check {
        Check { device_id, up: false, rtt_ms: None }
    }

    #[test]
    fn up_then_down_keeps_last_seen() {
        let inv = Inventory::in_memory().unwrap();
        let a = device(&inv, "a");
        let b = device(&inv, "b");
        inv.record_pings(&[up(a, 1.5), down(b)], 1000).unwrap();
        inv.record_pings(&[down(a), down(b)], 1030).unwrap();

        let s = inv.statuses().unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].device_id, s[0].up, s[0].checked_at, s[0].last_seen), (a, false, 1030, Some(1000)));
        assert_eq!(s[0].history.iter().map(|h| h.rtt_ms).collect::<Vec<_>>(), [Some(1.5), None]);
        assert_eq!((s[1].up, s[1].last_seen), (false, None));
    }

    #[test]
    fn up_without_a_ping_time_counts_as_seen() {
        let inv = Inventory::in_memory().unwrap();
        let a = device(&inv, "a");
        inv.record_pings(&[Check { device_id: a, up: true, rtt_ms: None }], 50).unwrap();
        let s = inv.statuses().unwrap().remove(0);
        assert_eq!((s.up, s.rtt_ms, s.last_seen), (true, None, Some(50)));
    }

    #[test]
    fn old_results_are_dropped_and_history_is_capped() {
        let inv = Inventory::in_memory().unwrap();
        let a = device(&inv, "a");
        inv.record_pings(&[up(a, 1.0)], 0).unwrap();
        for i in 0..(SAMPLES as i64 + 10) {
            inv.record_pings(&[up(a, 2.0)], KEEP_SECS + 1 + i).unwrap();
        }
        let s = inv.statuses().unwrap().remove(0);
        assert_eq!(s.history.len(), SAMPLES);
        let rows: i64 = inv.conn().query_row("SELECT COUNT(*) FROM pings", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, SAMPLES as i64 + 10);
    }

    #[test]
    fn removed_devices_leave_no_history() {
        let inv = Inventory::in_memory().unwrap();
        let a = device(&inv, "a");
        inv.record_pings(&[up(a, 1.0)], 10).unwrap();
        inv.remove(a).unwrap();
        // A result that arrives after the device was removed is ignored.
        inv.record_pings(&[up(a, 1.0)], 20).unwrap();
        assert!(inv.statuses().unwrap().is_empty());
    }
}
