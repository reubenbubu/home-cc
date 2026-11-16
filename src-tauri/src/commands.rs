//! Everything the screens can ask the back end to do.
//! Each module (network, machines, Docker, Proxmox) adds its commands here.

use crate::error::Result;
use crate::inventory::{Device, DeviceInput, Inventory};
use crate::secrets;
use serde::Serialize;
use tauri::State;

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

#[tauri::command]
pub fn list_devices(inv: State<Inventory>) -> Result<Vec<Device>> {
    inv.list()
}

#[tauri::command]
pub fn add_device(inv: State<Inventory>, input: DeviceInput) -> Result<Device> {
    inv.add(input)
}

#[tauri::command]
pub fn update_device(inv: State<Inventory>, id: i64, input: DeviceInput) -> Result<Device> {
    inv.update(id, input)
}

#[tauri::command]
pub fn remove_device(inv: State<Inventory>, id: i64) -> Result<()> {
    inv.remove(id)
}

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
