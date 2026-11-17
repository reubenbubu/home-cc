//! What the Pi-hole screen can ask the back end to do. Registered in lib.rs.
//! The password goes in with pihole_save_settings and is kept in the keychain;
//! it is never sent back to the screen.

use tauri::State;

use super::model::Blocking;
use super::{PiHole, PiholeSettings, PiholeStatus, Probe};
use crate::error::Result;
use crate::inventory::Inventory;

/// Every device marked as a Pi-hole, with its settings.
#[tauri::command(async)]
pub fn pihole_hosts(inv: State<Inventory>, ph: State<PiHole>) -> Result<Vec<PiholeSettings>> {
    ph.hosts(&inv)
}

/// Marks a device as a Pi-hole (HTTPS on port 443 to begin with). The screen asks the owner first.
#[tauri::command(async)]
pub fn pihole_add(inv: State<Inventory>, ph: State<PiHole>, device_id: i64) -> Result<PiholeSettings> {
    ph.add(&inv, device_id)
}

/// Stops treating a device as a Pi-hole: logs out, and forgets its settings and password.
#[tauri::command]
pub async fn pihole_remove(inv: State<'_, Inventory>, ph: State<'_, PiHole>, device_id: i64) -> Result<()> {
    ph.remove(&inv, device_id).await
}

/// Saves HTTPS or plain HTTP, the port, and the password when one is typed (empty or missing keeps the
/// stored one).
#[tauri::command]
pub async fn pihole_save_settings(
    inv: State<'_, Inventory>,
    ph: State<'_, PiHole>,
    device_id: i64,
    https: bool,
    port: u16,
    password: Option<String>,
) -> Result<PiholeSettings> {
    ph.save(&inv, device_id, https, port, password).await
}

#[tauri::command]
pub async fn pihole_remove_password(inv: State<'_, Inventory>, ph: State<'_, PiHole>, device_id: i64) -> Result<PiholeSettings> {
    ph.remove_password(&inv, device_id).await
}

/// Queries and blocked today, blocking state, lists and version. A failed reading comes back inside the
/// answer, with what to do about it.
#[tauri::command]
pub async fn pihole_status(inv: State<'_, Inventory>, ph: State<'_, PiHole>, device_id: i64) -> Result<PiholeStatus> {
    ph.status(&inv, device_id).await
}

/// The owner checked the certificate on the Pi-hole and approves it.
#[tauri::command(async)]
pub fn pihole_trust_certificate(inv: State<Inventory>, ph: State<PiHole>, device_id: i64, fingerprint: String) -> Result<()> {
    ph.trust_certificate(&inv, device_id, &fingerprint)
}

/// Blocking on, or off for `timer_secs` seconds (none: until switched on again). The screen asks the owner
/// before switching it off.
#[tauri::command]
pub async fn pihole_set_blocking(
    inv: State<'_, Inventory>,
    ph: State<'_, PiHole>,
    device_id: i64,
    blocking: bool,
    timer_secs: Option<u32>,
) -> Result<Blocking> {
    ph.set_blocking(&inv, device_id, blocking, timer_secs).await
}

/// Which of these saved devices answer like a Pi-hole (over HTTP on port 80, sending nothing secret).
#[tauri::command]
pub async fn pihole_probe(inv: State<'_, Inventory>, ph: State<'_, PiHole>, device_ids: Vec<i64>) -> Result<Vec<Probe>> {
    ph.probe(&inv, &device_ids).await
}
