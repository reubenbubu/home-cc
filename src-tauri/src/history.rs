//! Ping results: the latest status of each device and a short history, so
//! the owner can see if something drops out now and then.

use crate::error::Result;
use crate::inventory::Inventory;
use rusqlite::params;
use serde::Serialize;
use std::sync::atomic::Ordering;

/// How long ping results are kept.
pub const KEEP_SECS: i64 = 24 * 60 * 60;
/// How many of the latest results the screens get for each device.
pub const SAMPLES: usize = 120;
/// Old results are cleared out once per this many seconds, not every round:
/// clearing has to look through the whole table.
const PURGE_EVERY_SECS: i64 = 600;

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
    /// True when this device is the PC home-cc runs on. The inventory can't
    /// tell, so it is false here and filled in by whoever knows.
    pub is_self: bool,
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
        let slot = at.div_euclid(PURGE_EVERY_SECS);
        let purge = self.purged.load(Ordering::Relaxed) != slot;
        if purge {
            tx.execute("DELETE FROM pings WHERE ts < ?1", [at - KEEP_SECS])?;
        }
        tx.commit()?;
        if purge {
            self.purged.store(slot, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Latest status and recent history of every device that has been pinged.
    pub fn statuses(&self) -> Result<Vec<DeviceStatus>> {
        let conn = self.conn();
        // Read only what is returned: the newest few results of each device,
        // found through the (device, time) index, never the whole table.
        let mut devices = conn.prepare("SELECT id, last_seen FROM devices ORDER BY id")?;
        let mut recent = conn.prepare(
            "SELECT ts, up, rtt_ms FROM pings WHERE device_id = ?1 ORDER BY ts DESC, rowid DESC LIMIT ?2",
        )?;
        let known = devices.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?)))?;
        let mut out = Vec::new();
        for device in known {
            let (device_id, last_seen) = device?;
            let mut history = recent
                .query_map(params![device_id, SAMPLES as i64], |r| {
                    Ok(Sample { ts: r.get(0)?, up: r.get(1)?, rtt_ms: r.get(2)? })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            history.reverse();
            // A device that has never been checked has no status yet.
            let Some(latest) = history.last().cloned() else { continue };
            out.push(DeviceStatus {
                device_id,
                up: latest.up,
                rtt_ms: latest.rtt_ms,
                checked_at: latest.ts,
                last_seen,
                is_self: false,
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

    fn rows(inv: &Inventory) -> i64 {
        inv.conn().query_row("SELECT COUNT(*) FROM pings", [], |r| r.get(0)).unwrap()
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
        // Which device is this PC is not the inventory's to say.
        assert!(s.iter().all(|d| !d.is_self));
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
        assert_eq!(rows(&inv), SAMPLES as i64 + 10);
    }

    #[test]
    fn each_device_gets_its_newest_results_oldest_first() {
        let inv = Inventory::in_memory().unwrap();
        let a = device(&inv, "a");
        let never_checked = device(&inv, "never");
        let b = device(&inv, "b");
        let rounds = SAMPLES as i64 + 30;
        for i in 0..rounds {
            // "a" answers every round; "b" joins late, answers only the early
            // rounds and is checked half as often.
            let mut round = vec![up(a, i as f64)];
            if i >= 20 && i % 2 == 0 {
                round.push(if i < 40 { up(b, 0.5) } else { down(b) });
            }
            inv.record_pings(&round, 5000 + i * 30).unwrap();
        }

        let s = inv.statuses().unwrap();
        assert_eq!(s.iter().map(|d| d.device_id).collect::<Vec<_>>(), [a, b]);
        assert!(s.iter().all(|d| d.device_id != never_checked));

        // The newest SAMPLES of "a", oldest first, ending with the latest round.
        let last = 5000 + (rounds - 1) * 30;
        let times: Vec<i64> = s[0].history.iter().map(|h| h.ts).collect();
        let expected: Vec<i64> = (rounds - SAMPLES as i64..rounds).map(|i| 5000 + i * 30).collect();
        assert_eq!(times, expected);
        assert_eq!(s[0].history.first().map(|h| h.rtt_ms), Some(Some(30.0)));
        assert_eq!((s[0].up, s[0].rtt_ms, s[0].checked_at, s[0].last_seen), (true, Some((rounds - 1) as f64), last, Some(last)));

        // "b" has fewer than SAMPLES: all of them, in order, with its own last answer.
        let checks = (20..rounds).filter(|i| i % 2 == 0).count();
        assert_eq!(s[1].history.len(), checks);
        assert!(s[1].history.windows(2).all(|pair| pair[0].ts < pair[1].ts));
        assert_eq!(s[1].history.first().map(|h| (h.ts, h.up)), Some((5000 + 20 * 30, true)));
        assert_eq!((s[1].up, s[1].rtt_ms, s[1].checked_at), (false, None, 5000 + (rounds - 2) * 30));
        assert_eq!(s[1].last_seen, Some(5000 + 38 * 30));
    }

    #[test]
    fn two_checks_in_the_same_second_keep_their_order() {
        let inv = Inventory::in_memory().unwrap();
        let a = device(&inv, "a");
        inv.record_pings(&[down(a)], 700).unwrap();
        inv.record_pings(&[up(a, 3.0)], 700).unwrap();
        let s = inv.statuses().unwrap().remove(0);
        assert_eq!((s.up, s.rtt_ms), (true, Some(3.0)));
        assert_eq!(s.history.iter().map(|h| h.up).collect::<Vec<_>>(), [false, true]);
    }

    #[test]
    fn old_results_are_cleared_now_and_then_not_every_round() {
        let inv = Inventory::in_memory().unwrap();
        let a = device(&inv, "a");
        let start = 200 * PURGE_EVERY_SECS;
        // The first round after start-up clears what an earlier run left behind.
        inv.conn()
            .execute("INSERT INTO pings (device_id, ts, up) VALUES (?1, ?2, 1)", params![a, start - 2 * KEEP_SECS])
            .unwrap();
        inv.record_pings(&[up(a, 1.0)], start - KEEP_SECS + 5).unwrap();
        assert_eq!(rows(&inv), 1);

        // A day later the first result is still just young enough to keep...
        inv.record_pings(&[up(a, 1.0)], start).unwrap();
        assert_eq!(rows(&inv), 2);
        // ...ten seconds on it is too old, but clearing waits for its turn...
        inv.record_pings(&[up(a, 1.0)], start + 10).unwrap();
        assert_eq!(rows(&inv), 3);
        // ...which comes within ten minutes.
        inv.record_pings(&[up(a, 1.0)], start + PURGE_EVERY_SECS).unwrap();
        assert_eq!(rows(&inv), 3);
        let s = inv.statuses().unwrap().remove(0);
        assert_eq!(s.history.iter().map(|h| h.ts).collect::<Vec<_>>(), [start, start + 10, start + PURGE_EVERY_SECS]);
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
