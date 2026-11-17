//! What the Proxmox screen can ask the back end to do. Registered in lib.rs.
//! The token's secret is stored with the general set_secret command, under
//! the name `token_key` gives; it is never sent back to the screen.

use tauri::State;

use super::model::SnapshotList;
use super::{GuestRef, PowerAction, Proxmox, ProxmoxSettings, ProxmoxStatus, TaskOutcome};
use crate::error::Result;
use crate::inventory::Inventory;

/// The token ID, port and approved certificate of one Proxmox host, and
/// whether a secret is stored for it.
#[tauri::command(async)]
pub fn proxmox_settings(inv: State<Inventory>, px: State<Proxmox>, device_id: i64) -> Result<ProxmoxSettings> {
    px.settings(&inv, device_id)
}

/// Saves the token ID (empty removes it) and the API port.
#[tauri::command(async)]
pub fn proxmox_save_settings(
    inv: State<Inventory>,
    px: State<Proxmox>,
    device_id: i64,
    token_id: String,
    port: u16,
) -> Result<ProxmoxSettings> {
    px.save_settings(&inv, device_id, &token_id, port)
}

/// Every node, VM, container and storage. A failed reading comes back inside
/// the answer, with what to do about it.
#[tauri::command]
pub async fn proxmox_status(inv: State<'_, Inventory>, px: State<'_, Proxmox>, device_id: i64) -> Result<ProxmoxStatus> {
    px.status(&inv, device_id).await
}

/// The owner checked the certificate on the Proxmox host and approves it.
#[tauri::command(async)]
pub fn proxmox_trust_certificate(
    inv: State<Inventory>,
    px: State<Proxmox>,
    device_id: i64,
    fingerprint: String,
) -> Result<()> {
    px.trust_certificate(&inv, device_id, &fingerprint)
}

#[tauri::command]
pub async fn proxmox_power(
    inv: State<'_, Inventory>,
    px: State<'_, Proxmox>,
    device_id: i64,
    guest: GuestRef,
    action: PowerAction,
) -> Result<TaskOutcome> {
    px.power(&inv, device_id, &guest, action).await
}

#[tauri::command]
pub async fn proxmox_snapshots(
    inv: State<'_, Inventory>,
    px: State<'_, Proxmox>,
    device_id: i64,
    guest: GuestRef,
) -> Result<SnapshotList> {
    px.snapshots(&inv, device_id, &guest).await
}

#[tauri::command]
pub async fn proxmox_take_snapshot(
    inv: State<'_, Inventory>,
    px: State<'_, Proxmox>,
    device_id: i64,
    guest: GuestRef,
    name: String,
    description: String,
    with_ram: bool,
) -> Result<TaskOutcome> {
    px.take_snapshot(&inv, device_id, &guest, &name, &description, with_ram).await
}

/// With `start_after`, a guest Proxmox stopped to roll back is started again.
#[tauri::command]
pub async fn proxmox_rollback(
    inv: State<'_, Inventory>,
    px: State<'_, Proxmox>,
    device_id: i64,
    guest: GuestRef,
    name: String,
    start_after: bool,
) -> Result<TaskOutcome> {
    px.rollback(&inv, device_id, &guest, &name, start_after).await
}

#[tauri::command]
pub async fn proxmox_delete_snapshot(
    inv: State<'_, Inventory>,
    px: State<'_, Proxmox>,
    device_id: i64,
    guest: GuestRef,
    name: String,
) -> Result<TaskOutcome> {
    px.delete_snapshot(&inv, device_id, &guest, &name).await
}
