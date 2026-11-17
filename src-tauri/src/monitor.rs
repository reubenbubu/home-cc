//! The background work: checking every device is up on a timer, reading
//! machine stats on another, and running network scans when asked. The
//! screens are told with an event whenever there is something new to show.

use crate::error::{Error, Result};
use crate::inventory::{Device, DeviceKind, Found, Inventory, now};
use crate::net::{self, Located, ScanProgress, Target};
use crate::secrets;
use crate::ssh::{HostKey, Login, SshError};
use crate::stats::{self, CpuTicks, MachineStats};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinSet;

pub const EVENT_STATUS: &str = "status-changed";
pub const EVENT_STATS: &str = "stats-changed";
pub const EVENT_SCAN: &str = "scan-progress";

/// Keychain entry for the SSH key passphrase.
pub const KEY_PASSPHRASE: &str = "ssh-key-passphrase";
/// The shortest pause between two rounds of a timer, however it was woken.
const MIN_GAP: Duration = Duration::from_secs(2);
/// Machines read at the same time.
const STATS_AT_ONCE: usize = 4;

/// Keychain entry for one machine's SSH password.
pub fn password_key(device_id: i64) -> String {
    format!("ssh-password:{device_id}")
}

/// The machine a keychain entry made by `password_key` belongs to.
pub fn password_owner(key: &str) -> Option<i64> {
    key.strip_prefix("ssh-password:")?.parse().ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StatsErrorKind {
    /// First connection, and getting in needs the stored password, which is
    /// held back until the owner approves the machine's identity.
    NewHost,
    HostKeyChanged,
    Auth,
    Unreachable,
    Other,
}

/// Stats for one machine, as the screens see them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineStatsEntry {
    pub device_id: i64,
    pub stats: Option<MachineStats>,
    /// Unix seconds of the reading in `stats`.
    pub collected_at: Option<i64>,
    /// Message from the latest try, if it failed. Older stats are kept.
    pub error: Option<String>,
    pub error_kind: Option<StatsErrorKind>,
    /// The identity the machine presented, when the owner has to approve it.
    pub offered_key: Option<OfferedKey>,
    /// With a changed identity: the fingerprint that was stored.
    pub expected_fingerprint: Option<String>,
    /// The identity stored for this machine, if any.
    pub pinned_fingerprint: Option<String>,
    /// True when the app has stopped trying by itself, so it doesn't keep
    /// knocking on a machine that turns it away.
    pub paused: bool,
}

impl MachineStatsEntry {
    fn empty(device_id: i64) -> Self {
        Self {
            device_id,
            stats: None,
            collected_at: None,
            error: None,
            error_kind: None,
            offered_key: None,
            expected_fingerprint: None,
            pinned_fingerprint: None,
            paused: false,
        }
    }
}

/// A machine's SSH identity, as shown to the owner.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferedKey {
    pub algorithm: String,
    pub fingerprint: String,
}

impl From<&HostKey> for OfferedKey {
    fn from(key: &HostKey) -> Self {
        Self { algorithm: key.algorithm.clone(), fingerprint: key.fingerprint.clone() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub range: String,
    pub found: Vec<Found>,
    pub added: usize,
    pub updated: usize,
    pub cancelled: bool,
}

struct StatsState {
    entry: MachineStatsEntry,
    /// CPU counters from the last good reading, for the next CPU percentage.
    ticks: Option<CpuTicks>,
}

#[derive(Default)]
pub struct Monitor {
    stats: Mutex<HashMap<i64, StatsState>>,
    /// One reading of a machine at a time, so an older try can't finish last
    /// and overwrite a newer result.
    reading: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
    /// Why status checks can't work at all, if they can't.
    check_problem: Mutex<Option<String>>,
    /// Wakes the timers early, for example after settings change.
    wake_checks: Notify,
    wake_stats: Notify,
    /// Only one round of checks at a time.
    checking: tokio::sync::Mutex<()>,
    scanning: AtomicBool,
    cancel_scan: Arc<AtomicBool>,
}

/// Clears the "scan running" flag however the scan ends.
struct ScanGuard<'a>(&'a AtomicBool);

impl Drop for ScanGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Machines the app reads stats from: any tracked device with an SSH user.
fn wants_stats(d: &Device) -> bool {
    d.kind != DeviceKind::Ignore && d.ssh_user.as_deref().is_some_and(|u| !u.trim().is_empty())
}

fn error_kind(e: &SshError) -> StatsErrorKind {
    match e {
        SshError::NewHost(_) => StatsErrorKind::NewHost,
        SshError::HostKeyChanged { .. } => StatsErrorKind::HostKeyChanged,
        SshError::AuthFailed(_) => StatsErrorKind::Auth,
        SshError::Unreachable(_) => StatsErrorKind::Unreachable,
        SshError::Other(_) => StatsErrorKind::Other,
    }
}

impl Monitor {
    /// Start both timers. They run until the app closes. Each round starts
    /// one timer period after the previous one started, however long the
    /// round itself took.
    pub fn start(app: &AppHandle) {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let started = Instant::now();
                let monitor = handle.state::<Monitor>();
                // Problems are kept for the screens (see `check_problem`).
                monitor.check_round(&handle).await.ok();
                let secs = handle.state::<Inventory>().settings().map(|s| s.ping_interval_secs).unwrap_or(30);
                // Never straight into the next round: a round that overran, or
                // several wake-ups in a row, must not turn into a busy loop.
                let floor = tokio::time::Instant::now() + MIN_GAP;
                let next = tokio::time::Instant::from_std(started + Duration::from_secs(secs.into())).max(floor);
                tokio::select! {
                    biased;
                    _ = monitor.wake_checks.notified() => tokio::time::sleep_until(floor).await,
                    _ = tokio::time::sleep_until(next) => {}
                }
            }
        });

        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let started = Instant::now();
                let monitor = handle.state::<Monitor>();
                monitor.stats_round(&handle).await;
                let secs = handle.state::<Inventory>().settings().map(|s| s.stats_interval_secs).unwrap_or(60);
                // Never straight into the next round: a round that overran, or
                // several wake-ups in a row, must not turn into a busy loop.
                let floor = tokio::time::Instant::now() + MIN_GAP;
                let next = tokio::time::Instant::from_std(started + Duration::from_secs(secs.into())).max(floor);
                tokio::select! {
                    biased;
                    _ = monitor.wake_stats.notified() => tokio::time::sleep_until(floor).await,
                    _ = tokio::time::sleep_until(next) => {}
                }
            }
        });
    }

    /// Run both timers now, for example after settings or the inventory change.
    pub fn wake(&self) {
        self.wake_checks.notify_one();
        self.wake_stats.notify_one();
    }

    /// Check every tracked device once and store the results.
    pub async fn check_round(&self, app: &AppHandle) -> Result<()> {
        let _one_at_a_time = self.checking.lock().await;
        let targets: Vec<Target> = app
            .state::<Inventory>()
            .list()?
            .into_iter()
            .filter(|d| d.kind != DeviceKind::Ignore)
            .map(|d| Target { device_id: d.id, host: d.host, mac: d.mac })
            .collect();
        let outcome = match net::check(targets).await {
            Ok(results) => {
                *self.check_problem.lock().unwrap() = None;
                app.state::<Inventory>().record_pings(&results, now())
            }
            Err(e) => {
                // Nothing is recorded: "down" would be wrong for every device.
                *self.check_problem.lock().unwrap() = Some(e.to_string());
                Err(e)
            }
        };
        app.emit(EVENT_STATUS, ()).ok();
        outcome
    }

    /// Why status checks can't work at all, with what to do about it.
    pub fn check_problem(&self) -> Option<String> {
        self.check_problem.lock().unwrap().clone()
    }

    /// Scan the network and save what is found into the inventory.
    pub async fn scan(&self, app: &AppHandle) -> Result<ScanResult> {
        if self.scanning.swap(true, Ordering::SeqCst) {
            return Err(Error::Invalid("A scan is already running".into()));
        }
        let _guard = ScanGuard(&self.scanning);
        self.cancel_scan.store(false, Ordering::SeqCst);
        // Hold off the status timer while scanning: both together would send
        // more pings at once than the network's address table copes with.
        let quiet = self.checking.lock().await;

        let range = app.state::<Inventory>().settings()?.scan_range;
        let progress_app = app.clone();
        let progress = move |p: ScanProgress| {
            progress_app.emit(EVENT_SCAN, p).ok();
        };
        let outcome = net::scan(&range, progress, self.cancel_scan.clone()).await?;
        let named = named_devices(app, &outcome.found).await?;
        let (saved, added) = app.state::<Inventory>().record_scan_ids(&outcome.found, now(), &named)?;
        // Ids start again in a new database, so a password left in the
        // keychain by an earlier one must not end up belonging to a device
        // this scan just added. Done before the screens hear of the devices.
        if !added.is_empty() {
            tauri::async_runtime::spawn_blocking(move || {
                for id in added {
                    secrets::delete(&password_key(id)).ok();
                }
            })
            .await
            .ok();
        }

        drop(quiet);

        // Show the new devices straight away, then check them all.
        app.emit(EVENT_STATUS, ()).ok();
        self.wake_checks.notify_one();
        Ok(ScanResult {
            range: outcome.range,
            found: outcome.found,
            added: saved.added,
            updated: saved.updated,
            cancelled: self.cancel_scan.load(Ordering::SeqCst),
        })
    }

    pub fn cancel_scan(&self) {
        self.cancel_scan.store(true, Ordering::SeqCst);
    }

    pub fn scan_running(&self) -> bool {
        self.scanning.load(Ordering::SeqCst)
    }

    /// One entry per machine with an SSH user, read or not.
    pub fn stats_entries(&self, inv: &Inventory) -> Result<Vec<MachineStatsEntry>> {
        let machines: Vec<Device> = inv.list()?.into_iter().filter(wants_stats).collect();
        let mut entries = Vec::with_capacity(machines.len());
        for d in &machines {
            let known = self.stats.lock().unwrap().get(&d.id).map(|s| s.entry.clone());
            let mut entry = known.unwrap_or_else(|| MachineStatsEntry::empty(d.id));
            entry.pinned_fingerprint = inv.host_key(d.id)?.map(|k| k.fingerprint);
            entries.push(entry);
        }
        Ok(entries)
    }

    /// Forget what is held for a device that was removed, or that now points
    /// at a different machine.
    pub fn forget(&self, device_id: i64) {
        self.stats.lock().unwrap().remove(&device_id);
    }

    /// Let the timer try one machine again, after its password changed.
    pub fn retry_login(&self, device_id: i64) {
        if let Some(state) = self.stats.lock().unwrap().get_mut(&device_id) {
            state.entry.paused = false;
        }
        self.wake_stats.notify_one();
    }

    /// Let the timer try every machine again, after something they all share
    /// changed (the key file, or its passphrase).
    pub fn retry_logins(&self) {
        for state in self.stats.lock().unwrap().values_mut() {
            state.entry.paused = false;
        }
        self.wake_stats.notify_one();
    }

    /// The owner approved the identity a machine presented. Only the one they
    /// were shown is stored; from then on nothing else is accepted.
    pub fn trust_host_key(&self, inv: &Inventory, device_id: i64, fingerprint: &str) -> Result<()> {
        let offered = self.stats.lock().unwrap().get(&device_id).and_then(|s| s.entry.offered_key.clone());
        let Some(offered) = offered.filter(|k| k.fingerprint == fingerprint) else {
            return Err(Error::Invalid(
                "This machine has shown a different identity since. Check the new one and approve again.".into(),
            ));
        };
        inv.approve_host_key(device_id, &HostKey { algorithm: offered.algorithm, fingerprint: offered.fingerprint })?;
        if let Some(state) = self.stats.lock().unwrap().get_mut(&device_id) {
            state.entry.paused = false;
        }
        Ok(())
    }

    async fn stats_round(&self, app: &AppHandle) {
        let machines: Vec<Device> = match app.state::<Inventory>().list() {
            Ok(list) => list.into_iter().filter(wants_stats).collect(),
            Err(e) => {
                crate::diag::note(format!("stats round failed: {e}"));
                return;
            }
        };
        // Drop machines that were removed or no longer have an SSH user.
        self.stats.lock().unwrap().retain(|id, _| machines.iter().any(|d| d.id == *id));
        self.reading.lock().unwrap().retain(|id, _| machines.iter().any(|d| d.id == *id));
        let due: Vec<i64> = {
            let known = self.stats.lock().unwrap();
            machines.iter().map(|d| d.id).filter(|id| !known.get(id).is_some_and(|s| s.entry.paused)).collect()
        };
        if due.is_empty() {
            return;
        }

        let gate = Arc::new(Semaphore::new(STATS_AT_ONCE));
        let mut running = JoinSet::new();
        for device_id in due {
            let (app, gate) = (app.clone(), gate.clone());
            running.spawn(async move {
                let _slot = gate.acquire_owned().await;
                app.state::<Monitor>().read_stats(&app, device_id).await;
                // Each machine shows as soon as its own reading is in.
                app.emit(EVENT_STATS, ()).ok();
            });
        }
        while running.join_next().await.is_some() {}
    }

    /// Read one machine's stats now and remember the outcome.
    pub async fn read_stats(&self, app: &AppHandle, device_id: i64) -> MachineStatsEntry {
        let turn = self.reading.lock().unwrap().entry(device_id).or_default().clone();
        let _one_at_a_time = turn.lock().await;

        // Look the machine up only now: it may have been edited or removed
        // while this reading waited its turn.
        let device = match app.state::<Inventory>().get(device_id) {
            Ok(d) if wants_stats(&d) => d,
            _ => {
                self.forget(device_id);
                return MachineStatsEntry::empty(device_id);
            }
        };
        let prev_ticks = self.stats.lock().unwrap().get(&device_id).and_then(|s| s.ticks);
        let login = login_for(app, &device).await;
        // A machine reached for the first time has its identity remembered
        // before anything else happens, so a later change is noticed even if
        // the login never works. Remembered is not approved: the password
        // stays back until the owner has seen the fingerprint and said yes.
        let shown: Arc<Mutex<Option<HostKey>>> = Arc::default();
        let outcome = match &login {
            Ok(prepared) => {
                let (seen, first_app) = (shown.clone(), app.clone());
                let (host, port) = (prepared.login.host.clone(), prepared.login.port);
                let first_use = move |key: &HostKey| {
                    *seen.lock().unwrap() = Some(key.clone());
                    remember(&first_app, device_id, &host, port, key);
                };
                stats::collect(&prepared.login, device.os, prev_ticks, &first_use).await
            }
            Err(e) => Err(SshError::Other(e.to_string())),
        };
        let outcome = match (outcome, &login) {
            // No key got in and the password was held back because the owner
            // hasn't approved this machine's identity yet: ask them.
            (Err(SshError::AuthFailed(_)), Ok(prepared)) if prepared.password_held_back => {
                match prepared.login.known_key.clone().or_else(|| shown.lock().unwrap().clone()) {
                    Some(key) => Err(SshError::NewHost(key)),
                    None => Err(SshError::AuthFailed(
                        "Could not log in, and the machine did not show an identity to approve.".into(),
                    )),
                }
            }
            (other, _) => other,
        };

        // The owner may have pointed the device somewhere else, or changed
        // its login, while this reading ran. Its outcome is about the old
        // place, so it is dropped; the edit already started a fresh reading.
        let same_place = |d: &Device| {
            (d.host.as_str(), d.ssh_port, &d.ssh_user, d.os)
                == (device.host.as_str(), device.ssh_port, &device.ssh_user, device.os)
        };
        if !app.state::<Inventory>().get(device_id).is_ok_and(|d| same_place(&d)) {
            return self.stats.lock().unwrap().get(&device_id).map(|s| s.entry.clone())
                .unwrap_or_else(|| MachineStatsEntry::empty(device_id));
        }
        let pinned = app.state::<Inventory>().host_key(device_id).ok().flatten().map(|k| k.fingerprint);

        let mut known = self.stats.lock().unwrap();
        let state = known
            .entry(device_id)
            .or_insert_with(|| StatsState { entry: MachineStatsEntry::empty(device_id), ticks: None });
        let entry = &mut state.entry;
        entry.pinned_fingerprint = pinned;
        entry.offered_key = None;
        entry.expected_fingerprint = None;
        match outcome {
            Ok(collected) => {
                state.ticks = collected.cpu_ticks;
                entry.stats = Some(collected.stats);
                entry.collected_at = Some(now());
                entry.error = None;
                entry.error_kind = None;
                entry.paused = false;
            }
            Err(e) => {
                // Keep the last good reading on screen next to the error.
                state.ticks = None;
                entry.error = Some(match &e {
                    SshError::NewHost(_) => "The stored password has not been sent: this machine's identity \
                                             needs your approval first."
                        .to_string(),
                    other => other.to_string(),
                });
                entry.error_kind = Some(error_kind(&e));
                // A machine that turned the login away isn't tried again by
                // the timer: repeated failed logins get this PC locked out by
                // tools like fail2ban, and nothing changes until the owner
                // does something.
                entry.paused = matches!(e, SshError::AuthFailed(_) | SshError::NewHost(_));
                match e {
                    SshError::NewHost(key) => entry.offered_key = Some(OfferedKey::from(&key)),
                    SshError::HostKeyChanged { expected, got } => {
                        entry.offered_key = Some(OfferedKey::from(&got));
                        entry.expected_fingerprint = Some(expected.fingerprint);
                    }
                    _ => {}
                }
            }
        }
        entry.clone()
    }
}

fn remember(app: &AppHandle, device_id: i64, host: &str, port: u16, key: &HostKey) {
    if let Err(e) = app.state::<Inventory>().remember_host_key(device_id, host, port, key) {
        crate::diag::note(format!("could not store the identity of device {device_id}: {e}"));
    }
}

/// Hand-added devices whose address is a name: where each one is right now,
/// so the scan recognises them instead of adding them a second time.
async fn named_devices(app: &AppHandle, found: &[Found]) -> Result<HashMap<String, i64>> {
    let targets: Vec<Target> = app
        .state::<Inventory>()
        .list()?
        .into_iter()
        .filter(|d| d.host.parse::<std::net::IpAddr>().is_err())
        .map(|d| Target { device_id: d.id, host: d.host, mac: None })
        .collect();
    if targets.is_empty() {
        return Ok(HashMap::new());
    }
    let own_ip = found.iter().find(|f| f.is_self).map(|f| f.ip.clone());
    let mut named: HashMap<String, i64> = HashMap::new();
    let mut shared: Vec<String> = Vec::new();
    for (device_id, place) in net::locate(targets).await {
        let ip = match place {
            Located::At(ip) => ip.to_string(),
            Located::Own => match &own_ip {
                Some(ip) => ip.clone(),
                None => continue,
            },
        };
        // Two names for one address: can't tell which device it is.
        if named.insert(ip.clone(), device_id).is_some() {
            shared.push(ip);
        }
    }
    for ip in shared {
        named.remove(&ip);
    }
    Ok(named)
}

/// A login ready to try, and whether the stored password was left out of it.
struct Prepared {
    login: Login,
    /// A password is stored, but the owner hasn't approved this machine's
    /// identity yet, so it must not be sent.
    password_held_back: bool,
}

/// Gather what is needed to log in: stored identity, key file and secrets.
async fn login_for(app: &AppHandle, device: &Device) -> Result<Prepared> {
    let inv = app.state::<Inventory>();
    let settings = inv.settings()?;
    let known_key = inv.host_key(device.id)?;
    let approved = inv.host_key_approved(device.id)?;
    let device_id = device.id;
    // The keychain can be slow to answer, so keep it off the async threads.
    // If it can't be read at all (locked, or no keychain service running),
    // carry on without the secrets: a key file with no passphrase or the
    // SSH agent may still get in, and if not the login error says what to do.
    let (key_passphrase, password) = tauri::async_runtime::spawn_blocking(move || {
        let read = |key: &str| {
            secrets::get(key).unwrap_or_else(|e| {
                crate::diag::note(format!("could not read {key} from the keychain: {e}"));
                None
            })
        };
        (read(KEY_PASSPHRASE), read(&password_key(device_id)))
    })
    .await
    .unwrap_or((None, None));
    let has_password = password.as_deref().is_some_and(|p| !p.is_empty());
    Ok(Prepared {
        login: Login {
            host: device.host.clone(),
            port: device.ssh_port,
            user: device.ssh_user.clone().unwrap_or_default().trim().to_string(),
            known_key,
            key_path: Some(settings.ssh_key_path).filter(|p| !p.is_empty()).map(PathBuf::from),
            key_passphrase,
            // A password only ever goes to an identity the owner approved.
            password: password.filter(|_| approved),
        },
        password_held_back: has_password && !approved,
    })
}
