//! The background work: checking every device is up on a timer, reading
//! machine stats on another, and running network scans when asked. The
//! screens are told with an event whenever there is something new to show.

use crate::error::{Error, Result};
use crate::inventory::{Device, DeviceKind, Found, Inventory, now};
use crate::net::{self, ScanProgress, Target};
use crate::secrets;
use crate::ssh::{Login, SshError};
use crate::stats::{self, CpuTicks, MachineStats};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinSet;

pub const EVENT_STATUS: &str = "status-changed";
pub const EVENT_STATS: &str = "stats-changed";
pub const EVENT_SCAN: &str = "scan-progress";

/// Keychain entry for the SSH key passphrase.
const KEY_PASSPHRASE: &str = "ssh-key-passphrase";
/// Machines read at the same time.
const STATS_AT_ONCE: usize = 4;

/// Keychain entry for one machine's SSH password.
pub fn password_key(device_id: i64) -> String {
    format!("ssh-password:{device_id}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StatsErrorKind {
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
}

impl MachineStatsEntry {
    fn empty(device_id: i64) -> Self {
        Self { device_id, stats: None, collected_at: None, error: None, error_kind: None }
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
        SshError::HostKeyChanged { .. } => StatsErrorKind::HostKeyChanged,
        SshError::AuthFailed(_) => StatsErrorKind::Auth,
        SshError::Unreachable(_) => StatsErrorKind::Unreachable,
        SshError::Other(_) => StatsErrorKind::Other,
    }
}

impl Monitor {
    /// Start both timers. They run until the app closes.
    pub fn start(app: &AppHandle) {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let monitor = handle.state::<Monitor>();
                if let Err(e) = monitor.check_round(&handle).await {
                    eprintln!("home-cc: status check failed: {e}");
                }
                let secs = handle.state::<Inventory>().settings().map(|s| s.ping_interval_secs).unwrap_or(30);
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(secs.into())) => {}
                    _ = monitor.wake_checks.notified() => {}
                }
            }
        });

        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let monitor = handle.state::<Monitor>();
                monitor.stats_round(&handle).await;
                let secs = handle.state::<Inventory>().settings().map(|s| s.stats_interval_secs).unwrap_or(60);
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(secs.into())) => {}
                    _ = monitor.wake_stats.notified() => {}
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
        if !targets.is_empty() {
            let results = net::check(targets).await?;
            app.state::<Inventory>().record_pings(&results, now())?;
        }
        app.emit(EVENT_STATUS, ()).ok();
        Ok(())
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
        let saved = app.state::<Inventory>().record_scan(&outcome.found, now())?;

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
        let known = self.stats.lock().unwrap();
        Ok(inv
            .list()?
            .iter()
            .filter(|d| wants_stats(d))
            .map(|d| known.get(&d.id).map(|s| s.entry.clone()).unwrap_or_else(|| MachineStatsEntry::empty(d.id)))
            .collect())
    }

    /// Forget what is held for a device that was removed.
    pub fn forget(&self, device_id: i64) {
        self.stats.lock().unwrap().remove(&device_id);
    }

    async fn stats_round(&self, app: &AppHandle) {
        let machines: Vec<Device> = match app.state::<Inventory>().list() {
            Ok(list) => list.into_iter().filter(wants_stats).collect(),
            Err(e) => {
                eprintln!("home-cc: stats round failed: {e}");
                return;
            }
        };
        // Drop machines that were removed or no longer have an SSH user.
        self.stats.lock().unwrap().retain(|id, _| machines.iter().any(|d| d.id == *id));
        if machines.is_empty() {
            return;
        }

        let gate = Arc::new(Semaphore::new(STATS_AT_ONCE));
        let mut running = JoinSet::new();
        for device in machines {
            let (app, gate) = (app.clone(), gate.clone());
            running.spawn(async move {
                let _slot = gate.acquire_owned().await;
                app.state::<Monitor>().read_stats(&app, &device).await;
                // Each machine shows as soon as its own reading is in.
                app.emit(EVENT_STATS, ()).ok();
            });
        }
        while running.join_next().await.is_some() {}
    }

    /// Read one machine's stats now and remember the outcome.
    pub async fn read_stats(&self, app: &AppHandle, device: &Device) -> MachineStatsEntry {
        let prev_ticks = self.stats.lock().unwrap().get(&device.id).and_then(|s| s.ticks);
        let outcome = match login_for(app, device).await {
            Ok(login) => match stats::collect(&login, device.os, prev_ticks).await {
                Ok(collected) => {
                    if login.known_key.is_none() {
                        let inv = app.state::<Inventory>();
                        if let Err(e) = inv.remember_host_key(&login.host, login.port, &collected.host_key) {
                            eprintln!("home-cc: could not store the identity of {}: {e}", login.host);
                        }
                    }
                    Ok(collected)
                }
                Err(e) => Err((e.to_string(), error_kind(&e))),
            },
            Err(e) => Err((e.to_string(), StatsErrorKind::Other)),
        };

        let mut known = self.stats.lock().unwrap();
        let state = known
            .entry(device.id)
            .or_insert_with(|| StatsState { entry: MachineStatsEntry::empty(device.id), ticks: None });
        match outcome {
            Ok(collected) => {
                state.ticks = collected.cpu_ticks;
                state.entry = MachineStatsEntry {
                    device_id: device.id,
                    stats: Some(collected.stats),
                    collected_at: Some(now()),
                    error: None,
                    error_kind: None,
                };
            }
            Err((message, kind)) => {
                // Keep the last good reading on screen next to the error.
                state.ticks = None;
                state.entry.error = Some(message);
                state.entry.error_kind = Some(kind);
            }
        }
        state.entry.clone()
    }
}

/// Gather what is needed to log in: stored identity, key file and secrets.
async fn login_for(app: &AppHandle, device: &Device) -> Result<Login> {
    let inv = app.state::<Inventory>();
    let settings = inv.settings()?;
    let known_key = inv.host_key(&device.host, device.ssh_port)?;
    let device_id = device.id;
    // The keychain can be slow to answer, so keep it off the async threads.
    // If it can't be read at all (locked, or no keychain service running),
    // carry on without the secrets: a key file with no passphrase or the
    // SSH agent may still get in, and if not the login error says what to do.
    let (key_passphrase, password) = tauri::async_runtime::spawn_blocking(move || {
        let read = |key: &str| {
            secrets::get(key).unwrap_or_else(|e| {
                eprintln!("home-cc: could not read {key} from the keychain: {e}");
                None
            })
        };
        (read(KEY_PASSPHRASE), read(&password_key(device_id)))
    })
    .await
    .unwrap_or((None, None));
    Ok(Login {
        host: device.host.clone(),
        port: device.ssh_port,
        user: device.ssh_user.clone().unwrap_or_default().trim().to_string(),
        known_key,
        key_path: Some(settings.ssh_key_path).filter(|p| !p.is_empty()).map(PathBuf::from),
        key_passphrase,
        password,
    })
}
