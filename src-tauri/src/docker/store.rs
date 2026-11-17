//! What the Docker module keeps in the app's database: which machines are Docker
//! hosts (found by looking, or marked by the owner), and the compose stacks seen on
//! each, so a stack that is down (no containers left to read it from) can still be
//! brought up again.
//!
//! Both belong to a device and go with it. They also go when the device is pointed at
//! another machine: each row remembers where it was learned ("host:port:user").

use super::parse::Compose;
use super::scripts::StackPlace;
use crate::error::Result;
use crate::inventory::{Device, Inventory};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

const TABLES: &str = "
CREATE TABLE IF NOT EXISTS docker_hosts (
    device_id  INTEGER PRIMARY KEY REFERENCES devices (id) ON DELETE CASCADE,
    -- 'auto': shown if Docker was found; 'on': always shown; 'off': never shown.
    mode       TEXT NOT NULL DEFAULT 'auto',
    -- 1 Docker is there, 0 it is not, NULL not looked yet.
    found      INTEGER,
    version    TEXT,
    compose    TEXT NOT NULL DEFAULT 'none',
    checked_at INTEGER,
    place      TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS docker_stacks (
    device_id    INTEGER NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    project      TEXT NOT NULL,
    working_dir  TEXT,
    -- One path per line.
    config_files TEXT NOT NULL DEFAULT '',
    place        TEXT NOT NULL DEFAULT '',
    last_seen    INTEGER NOT NULL,
    PRIMARY KEY (device_id, project)
);
";

/// Whether a machine is shown on the Docker screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostMode {
    /// Shown when home-cc found Docker on it.
    Auto,
    /// The owner said it is a Docker host: shown even when it can't be reached.
    On,
    /// The owner said it is not: never shown or looked at.
    Off,
}

impl HostMode {
    fn as_str(self) -> &'static str {
        match self {
            HostMode::Auto => "auto",
            HostMode::On => "on",
            HostMode::Off => "off",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "on" => HostMode::On,
            "off" => HostMode::Off,
            _ => HostMode::Auto,
        }
    }
}

/// What is stored about one machine. A machine with no row is `auto` and unlooked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRow {
    pub mode: HostMode,
    pub found: Option<bool>,
    pub version: Option<String>,
    pub compose: Compose,
    pub checked_at: Option<i64>,
}

impl Default for HostRow {
    fn default() -> Self {
        Self { mode: HostMode::Auto, found: None, version: None, compose: Compose::None, checked_at: None }
    }
}

/// Where a device's Docker facts were learned. A device pointed somewhere else is
/// another machine as far as Docker goes.
pub fn place(d: &Device) -> String {
    format!("{}:{}:{}", d.host.trim().to_lowercase(), d.ssh_port, d.ssh_user.as_deref().unwrap_or("").trim())
}

pub fn init(inv: &Inventory) -> Result<()> {
    inv.conn().execute_batch(TABLES)?;
    Ok(())
}

impl Inventory {
    /// What is stored for this machine. Facts learned at another place are not
    /// returned, only the owner's own choice of mode.
    pub fn docker_host(&self, d: &Device) -> Result<HostRow> {
        let row = self
            .conn()
            .query_row(
                "SELECT mode, found, version, compose, checked_at, place FROM docker_hosts WHERE device_id = ?1",
                [d.id],
                |r| {
                    Ok((
                        HostRow {
                            mode: HostMode::parse(&r.get::<_, String>(0)?),
                            found: r.get(1)?,
                            version: r.get(2)?,
                            compose: Compose::parse(&r.get::<_, String>(3)?),
                            checked_at: r.get(4)?,
                        },
                        r.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?;
        Ok(match row {
            Some((row, at)) if at == place(d) => row,
            Some((row, _)) => HostRow { mode: row.mode, ..HostRow::default() },
            None => HostRow::default(),
        })
    }

    pub fn set_docker_mode(&self, d: &Device, mode: HostMode) -> Result<()> {
        self.conn().execute(
            "INSERT INTO docker_hosts (device_id, mode, place) VALUES (?1, ?2, ?3)
             ON CONFLICT (device_id) DO UPDATE SET mode = excluded.mode",
            params![d.id, mode.as_str(), place(d)],
        )?;
        Ok(())
    }

    /// Store what a look for Docker found. Stacks learned at an older place are
    /// dropped: they were on another machine.
    pub fn record_docker_probe(
        &self,
        d: &Device,
        found: bool,
        version: Option<&str>,
        compose: Compose,
        at: i64,
    ) -> Result<()> {
        let here = place(d);
        let conn = self.conn();
        conn.execute(
            "INSERT INTO docker_hosts (device_id, found, version, compose, checked_at, place)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (device_id) DO UPDATE SET found = excluded.found, version = excluded.version,
                 compose = excluded.compose, checked_at = excluded.checked_at, place = excluded.place",
            params![d.id, found, version, compose.as_str(), at, here],
        )?;
        conn.execute("DELETE FROM docker_stacks WHERE device_id = ?1 AND place <> ?2", params![d.id, here])?;
        Ok(())
    }

    /// Remember the stacks just seen on a machine, with where their files are.
    pub fn record_docker_stacks(&self, d: &Device, stacks: &[StackPlace], at: i64) -> Result<()> {
        let here = place(d);
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM docker_stacks WHERE device_id = ?1 AND place <> ?2", params![d.id, here])?;
        for s in stacks {
            tx.execute(
                "INSERT INTO docker_stacks (device_id, project, working_dir, config_files, place, last_seen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (device_id, project) DO UPDATE SET
                     working_dir = COALESCE(excluded.working_dir, docker_stacks.working_dir),
                     config_files = CASE WHEN excluded.config_files = '' THEN docker_stacks.config_files
                                         ELSE excluded.config_files END,
                     place = excluded.place, last_seen = excluded.last_seen",
                params![d.id, s.project, s.working_dir, s.config_files.join("\n"), here, at],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Every stack remembered for this machine, by name.
    pub fn docker_stacks(&self, d: &Device) -> Result<Vec<StackPlace>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT project, working_dir, config_files FROM docker_stacks
             WHERE device_id = ?1 AND place = ?2 ORDER BY project",
        )?;
        let rows = stmt.query_map(params![d.id, place(d)], |r| {
            let files: String = r.get(2)?;
            Ok(StackPlace {
                project: r.get(0)?,
                working_dir: r.get(1)?,
                config_files: files.lines().filter(|l| !l.is_empty()).map(str::to_string).collect(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn docker_stack(&self, d: &Device, project: &str) -> Result<Option<StackPlace>> {
        Ok(self.docker_stacks(d)?.into_iter().find(|s| s.project == project))
    }

    pub fn forget_docker_stack(&self, d: &Device, project: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM docker_stacks WHERE device_id = ?1 AND project = ?2", params![d.id, project])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{DeviceInput, DeviceKind, Os};

    fn setup() -> (Inventory, Device) {
        let inv = Inventory::in_memory().unwrap();
        init(&inv).unwrap();
        let d = inv
            .add(DeviceInput {
                name: "Homelab VM".into(),
                host: "192.168.68.21".into(),
                kind: DeviceKind::Vm,
                os: Os::Linux,
                ssh_user: Some("sam".into()),
                ssh_port: Some(22),
                notes: String::new(),
                mac: None,
                group: None,
            })
            .unwrap();
        (inv, d)
    }

    fn stack(name: &str) -> StackPlace {
        StackPlace {
            project: name.into(),
            working_dir: Some(format!("/srv/{name}")),
            config_files: vec![format!("/srv/{name}/compose.yml"), format!("/srv/{name}/override.yml")],
        }
    }

    #[test]
    fn a_machine_starts_unlooked_and_keeps_what_was_found() {
        let (inv, d) = setup();
        assert_eq!(inv.docker_host(&d).unwrap(), HostRow::default());
        inv.record_docker_probe(&d, true, Some("29.0.1"), Compose::Plugin, 100).unwrap();
        let row = inv.docker_host(&d).unwrap();
        assert_eq!((row.found, row.version.as_deref(), row.compose, row.checked_at), (Some(true), Some("29.0.1"), Compose::Plugin, Some(100)));
        assert_eq!(row.mode, HostMode::Auto);
        inv.set_docker_mode(&d, HostMode::Off).unwrap();
        let row = inv.docker_host(&d).unwrap();
        assert_eq!((row.mode, row.found), (HostMode::Off, Some(true)), "the mark leaves what was found alone");
        // Init again (next start of the app) changes nothing.
        init(&inv).unwrap();
        assert_eq!(inv.docker_host(&d).unwrap().mode, HostMode::Off);
    }

    #[test]
    fn stacks_are_remembered_and_updated() {
        let (inv, d) = setup();
        inv.record_docker_stacks(&d, &[stack("media"), stack("photos")], 10).unwrap();
        // Seen again later with no file labels (older compose): the files stay.
        let bare = StackPlace { project: "media".into(), working_dir: None, config_files: vec![] };
        inv.record_docker_stacks(&d, &[bare], 20).unwrap();
        let stacks = inv.docker_stacks(&d).unwrap();
        assert_eq!(stacks, [stack("media"), stack("photos")]);
        inv.forget_docker_stack(&d, "photos").unwrap();
        assert_eq!(inv.docker_stack(&d, "photos").unwrap(), None);
        assert_eq!(inv.docker_stack(&d, "media").unwrap(), Some(stack("media")));
    }

    #[test]
    fn facts_belong_to_the_machine_that_was_there() {
        let (inv, d) = setup();
        inv.set_docker_mode(&d, HostMode::On).unwrap();
        inv.record_docker_probe(&d, true, Some("29"), Compose::Plugin, 100).unwrap();
        inv.record_docker_stacks(&d, &[stack("media")], 100).unwrap();

        // The owner points the device at another address.
        let moved = inv
            .update(
                d.id,
                DeviceInput {
                    name: d.name.clone(),
                    host: "192.168.68.99".into(),
                    kind: d.kind,
                    os: d.os,
                    ssh_user: d.ssh_user.clone(),
                    ssh_port: Some(22),
                    notes: String::new(),
                    mac: None,
                    group: None,
                },
            )
            .unwrap();
        let row = inv.docker_host(&moved).unwrap();
        assert_eq!(row, HostRow { mode: HostMode::On, ..HostRow::default() }, "only the owner's mark carries over");
        assert!(inv.docker_stacks(&moved).unwrap().is_empty());

        // A look at the new place clears the old stacks for good.
        inv.record_docker_probe(&moved, false, None, Compose::None, 200).unwrap();
        assert_eq!(inv.docker_host(&moved).unwrap().found, Some(false));
        assert_eq!(inv.conn().query_row("SELECT COUNT(*) FROM docker_stacks", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn rows_go_with_the_device() {
        let (inv, d) = setup();
        inv.record_docker_probe(&d, true, None, Compose::Plugin, 1).unwrap();
        inv.record_docker_stacks(&d, &[stack("media")], 1).unwrap();
        inv.remove(d.id).unwrap();
        let count = |table: &str| {
            inv.conn().query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get::<_, i64>(0)).unwrap()
        };
        assert_eq!((count("docker_hosts"), count("docker_stacks")), (0, 0));
    }
}
