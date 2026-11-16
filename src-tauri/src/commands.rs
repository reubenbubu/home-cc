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
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo { version: env!("CARGO_PKG_VERSION"), os: std::env::consts::OS }
}

// --- Inventory ---

#[tauri::command]
pub fn list_devices(inv: State<Inventory>) -> Result<Vec<Device>> {
    inv.list()
}

#[tauri::command]
pub fn add_device(inv: State<Inventory>, monitor: State<Monitor>, input: DeviceInput) -> Result<Device> {
    let device = inv.add(input)?;
    monitor.wake();
    Ok(device)
}

#[tauri::command]
pub fn update_device(
    inv: State<Inventory>,
    monitor: State<Monitor>,
    id: i64,
    input: DeviceInput,
) -> Result<Device> {
    let device = inv.update(id, input)?;
    monitor.wake();
    Ok(device)
}

#[tauri::command]
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

#[tauri::command]
pub fn set_secret(key: String, value: String) -> Result<()> {
    secrets::set(&key, &value)
}

#[tauri::command]
pub fn has_secret(key: String) -> Result<bool> {
    secrets::has(&key)
}

#[tauri::command]
pub fn delete_secret(key: String) -> Result<()> {
    secrets::delete(&key)
}

// --- Settings ---

#[tauri::command]
pub fn get_settings(inv: State<Inventory>) -> Result<Settings> {
    inv.settings()
}

#[tauri::command]
pub fn save_settings(inv: State<Inventory>, monitor: State<Monitor>, settings: Settings) -> Result<Settings> {
    let saved = inv.save_settings(settings)?;
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

#[tauri::command]
pub fn device_statuses(inv: State<Inventory>) -> Result<Vec<DeviceStatus>> {
    inv.statuses()
}

#[tauri::command]
pub async fn check_now(app: AppHandle, monitor: State<'_, Monitor>) -> Result<()> {
    monitor.check_round(&app).await
}

// --- Machine stats ---

#[tauri::command]
pub fn machine_stats(inv: State<Inventory>, monitor: State<Monitor>) -> Result<Vec<MachineStatsEntry>> {
    monitor.stats_entries(&inv)
}

#[tauri::command]
pub async fn refresh_stats(
    app: AppHandle,
    inv: State<'_, Inventory>,
    monitor: State<'_, Monitor>,
    device_id: i64,
) -> Result<MachineStatsEntry> {
    let device = inv.get(device_id)?;
    if device.ssh_user.as_deref().is_none_or(|u| u.trim().is_empty()) {
        return Err(Error::Invalid("Add an SSH user to this machine first".into()));
    }
    let entry = monitor.read_stats(&app, &device).await;
    app.emit(monitor::EVENT_STATS, ()).ok();
    Ok(entry)
}

/// After the owner confirms a machine's identity really did change: forget
/// the stored one, so the next reading trusts what the machine presents.
#[tauri::command]
pub fn forget_host_key(inv: State<Inventory>, device_id: i64) -> Result<()> {
    let device = inv.get(device_id)?;
    inv.forget_host_key(&device.host, device.ssh_port)
}
