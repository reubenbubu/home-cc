mod commands;
mod diag;
// Public so the small diagnostic programs in examples/ can use them.
pub mod error;
pub mod history;
mod hostkeys;
pub mod inventory;
mod monitor;
pub mod net;
// Proxmox (spec 5.6): its own module and commands, registered below.
mod proxmox;
mod secrets;
pub mod settings;
pub mod ssh;
pub mod stats;

use inventory::Inventory;
use monitor::Monitor;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // Remembers window size, position and which monitor it was on.
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let inventory = Inventory::open(&dir.join("home-cc.db"))?;
            // --- Proxmox ---
            inventory.proxmox_init()?;
            app.manage(proxmox::Proxmox::default());
            // --- end Proxmox ---
            app.manage(inventory);
            app.manage(Monitor::default());
            Monitor::start(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::recent_log,
            commands::list_devices,
            commands::add_device,
            commands::update_device,
            commands::remove_device,
            commands::set_secret,
            commands::has_secret,
            commands::delete_secret,
            commands::get_settings,
            commands::save_settings,
            commands::list_subnets,
            commands::start_scan,
            commands::cancel_scan,
            commands::scan_running,
            commands::device_statuses,
            commands::check_now,
            commands::check_problem,
            commands::machine_stats,
            commands::refresh_stats,
            commands::trust_host_key,
            // --- Proxmox (src-tauri/src/proxmox/commands.rs) ---
            proxmox::commands::proxmox_settings,
            proxmox::commands::proxmox_save_settings,
            proxmox::commands::proxmox_status,
            proxmox::commands::proxmox_trust_certificate,
            proxmox::commands::proxmox_power,
            proxmox::commands::proxmox_snapshots,
            proxmox::commands::proxmox_take_snapshot,
            proxmox::commands::proxmox_rollback,
            proxmox::commands::proxmox_delete_snapshot,
            // --- end Proxmox ---
        ])
        .run(tauri::generate_context!())
        .expect("error while running home-cc");
}
