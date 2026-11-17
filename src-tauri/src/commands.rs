//! Everything the screens can ask the back end to do.
//! Each module (network, machines, Docker, Proxmox) adds its commands here.

use crate::error::{Error, Result};
use crate::history::DeviceStatus;
use crate::inventory::{Device, DeviceInput, Inventory};
use crate::monitor::{self, MachineStatsEntry, Monitor, ScanResult};
use crate::net::{self, Subnet};
use crate::secrets;
use crate::settings::Settings;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
    os: &'static str,
    /// Processor family, such as "x86_64".
    arch: &'static str,
    /// False for a development build (`tauri dev`).
    release: bool,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        release: !cfg!(debug_assertions),
    }
}

/// Problems the back end ran into on its own, oldest first, for a problem report.
#[tauri::command]
pub fn recent_log() -> Vec<crate::diag::LogLine> {
    crate::diag::recent()
}

// --- Inventory ---

// Commands marked `async` here are ordinary functions that Tauri runs off
// the window's own thread, because they touch the database or the keychain.

#[tauri::command(async)]
pub fn list_devices(inv: State<Inventory>) -> Result<Vec<Device>> {
    inv.list()
}

#[tauri::command(async)]
pub fn add_device(inv: State<Inventory>, monitor: State<Monitor>, input: DeviceInput) -> Result<Device> {
    let device = inv.add(input)?;
    // Ids start again in a new database, so a password left in the keychain
    // by an earlier one must not end up belonging to this device.
    secrets::delete(&monitor::password_key(device.id)).ok();
    monitor.wake();
    Ok(device)
}

#[tauri::command(async)]
pub fn update_device(
    inv: State<Inventory>,
    monitor: State<Monitor>,
    id: i64,
    input: DeviceInput,
) -> Result<Device> {
    let before = inv.get(id)?;
    let device = inv.update(id, input)?;
    // Stats and login state belong to the machine the entry pointed at.
    if (&before.host, before.ssh_port, &before.ssh_user, before.os)
        != (&device.host, device.ssh_port, &device.ssh_user, device.os)
    {
        monitor.forget(id);
    }
    monitor.wake();
    Ok(device)
}

#[tauri::command(async)]
pub fn remove_device(app: AppHandle, inv: State<Inventory>, monitor: State<Monitor>, id: i64) -> Result<()> {
    inv.remove(id)?;
    monitor.forget(id);
    // Its stored password goes with it. A keychain hiccup here shouldn't
    // stop the device being removed.
    secrets::delete(&monitor::password_key(id)).ok();
    app.emit(monitor::EVENT_STATUS, ()).ok();
    app.emit(monitor::EVENT_STATS, ()).ok();
    Ok(())
}

// --- Secrets ---

#[tauri::command(async)]
pub fn set_secret(monitor: State<Monitor>, key: String, value: String) -> Result<()> {
    secrets::set(&key, &value)?;
    retry_after_secret(&monitor, &key);
    Ok(())
}

#[tauri::command(async)]
pub fn has_secret(key: String) -> Result<bool> {
    secrets::has(&key)
}

#[tauri::command(async)]
pub fn delete_secret(monitor: State<Monitor>, key: String) -> Result<()> {
    secrets::delete(&key)?;
    retry_after_secret(&monitor, &key);
    Ok(())
}

/// A new password or passphrase may be what a failed login was missing: try
/// again the machines it is for, and no others.
fn retry_after_secret(monitor: &Monitor, key: &str) {
    if let Some(device_id) = monitor::password_owner(key) {
        monitor.retry_login(device_id);
    } else if key == monitor::KEY_PASSPHRASE {
        monitor.retry_logins();
    }
}

// --- Settings ---

#[tauri::command]
pub fn get_settings(inv: State<Inventory>) -> Result<Settings> {
    inv.settings()
}

#[tauri::command]
pub fn save_settings(inv: State<Inventory>, monitor: State<Monitor>, settings: Settings) -> Result<Settings> {
    let key_file_before = inv.settings().ok().map(|s| s.ssh_key_path);
    let saved = inv.save_settings(settings)?;
    // A different key file may be what a failed login was missing.
    if key_file_before.as_deref() != Some(saved.ssh_key_path.as_str()) {
        monitor.retry_logins();
    }
    // Pick up new timers straight away.
    monitor.wake();
    Ok(saved)
}

// --- Network scan and status ---

#[tauri::command]
pub async fn list_subnets() -> Result<Vec<Subnet>> {
    tauri::async_runtime::spawn_blocking(net::subnets)
        .await
        .map_err(|e| Error::Network(format!("Could not read this PC's network adapters: {e}")))
}

#[tauri::command]
pub async fn start_scan(app: AppHandle, monitor: State<'_, Monitor>) -> Result<ScanResult> {
    monitor.scan(&app).await
}

#[tauri::command]
pub fn cancel_scan(monitor: State<Monitor>) {
    monitor.cancel_scan();
}

#[tauri::command]
pub fn scan_running(monitor: State<Monitor>) -> bool {
    monitor.scan_running()
}

#[tauri::command(async)]
pub fn device_statuses(inv: State<Inventory>) -> Result<Vec<DeviceStatus>> {
    let own = net::own_devices();
    let mut statuses = inv.statuses()?;
    for status in &mut statuses {
        status.is_self = own.contains(&status.device_id);
    }
    Ok(statuses)
}

#[tauri::command]
pub async fn check_now(app: AppHandle, monitor: State<'_, Monitor>) -> Result<()> {
    if monitor.scan_running() {
        return Err(Error::Invalid(
            "A scan is running. Every device is checked again as soon as it finishes.".into(),
        ));
    }
    monitor.check_round(&app).await
}

/// The name each address resolves to, for showing beside it. Remembered for
/// a while, so screens can ask as often as they like.
#[tauri::command]
pub async fn resolve_names(addresses: Vec<String>) -> Result<Vec<net::ResolvedName>> {
    net::resolve_names(addresses).await
}

/// Why status checks can't work at all, if they can't.
#[tauri::command]
pub fn check_problem(monitor: State<Monitor>) -> Option<String> {
    monitor.check_problem()
}

// --- Machine stats ---

#[tauri::command]
pub async fn machine_stats(inv: State<'_, Inventory>, monitor: State<'_, Monitor>) -> Result<Vec<MachineStatsEntry>> {
    monitor.stats_entries(&inv).await
}

#[tauri::command]
pub async fn refresh_stats(
    app: AppHandle,
    inv: State<'_, Inventory>,
    monitor: State<'_, Monitor>,
    device_id: i64,
) -> Result<MachineStatsEntry> {
    let device = inv.get(device_id)?;
    // This PC needs no SSH user: it is read here (localstats.rs).
    if device.ssh_user.as_deref().is_none_or(|u| u.trim().is_empty())
        && !crate::localstats::is_this_pc_now(&device).await
    {
        return Err(Error::Invalid("Add an SSH user to this machine first".into()));
    }
    let entry = monitor.read_stats(&app, device_id).await;
    app.emit(monitor::EVENT_STATS, ()).ok();
    Ok(entry)
}

/// The owner checked the identity a machine presented and approves it.
#[tauri::command(async)]
pub fn trust_host_key(
    inv: State<Inventory>,
    monitor: State<Monitor>,
    device_id: i64,
    fingerprint: String,
) -> Result<()> {
    monitor.trust_host_key(&inv, device_id, &fingerprint)
}
