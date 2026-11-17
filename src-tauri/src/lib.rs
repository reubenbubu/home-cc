mod commands;
mod diag;
// Docker (spec 5.5): containers and compose stacks over SSH.
mod docker;
// Public so the small diagnostic programs in examples/ can use them.
pub mod error;
pub mod history;
mod hostkeys;
pub mod inventory;
mod keysetup;
mod localstats;
mod monitor;
pub mod net;
// Pi-hole: its own module and commands, registered below.
mod pihole;
// Proxmox (spec 5.6): its own module and commands, registered below.
mod proxmox;
mod secrets;
pub mod settings;
pub mod ssh;
pub mod stats;
mod terminal;

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
            // Docker (spec 5.5): its tables live in the same file.
            docker::init(&inventory)?;
            // --- Proxmox ---
            inventory.proxmox_init()?;
            app.manage(proxmox::Proxmox::default());
            // --- end Proxmox ---
            // --- Pi-hole ---
            inventory.pihole_init()?;
            app.manage(pihole::PiHole::default());
            // --- end Pi-hole ---
            app.manage(inventory);
            app.manage(Monitor::default());
            app.manage(keysetup::KeySetup::default());
            app.manage(terminal::Terminals::default());
            Monitor::start(app.handle());
            // Docker (spec 5.5).
            app.manage(docker::Docker::default());
            docker::Docker::start(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::recent_log,
            commands::list_devices,
            commands::add_device,
            commands::update_device,
            commands::set_device_group,
            commands::rename_group,
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
            commands::resolve_names,
            commands::machine_stats,
            commands::refresh_stats,
            commands::trust_host_key,
            keysetup::local_ssh_key,
            keysetup::create_ssh_key,
            keysetup::setup_key_login,
            keysetup::approve_key_setup_identity,
            // Terminals
            commands::terminal_open,
            commands::terminal_write,
            commands::terminal_write_binary,
            commands::terminal_resize,
            commands::terminal_ack,
            commands::terminal_close,
            commands::terminal_close_all,
            commands::clipboard_read_text,
            commands::clipboard_write_text,
            // --- Docker (spec 5.5) ---
            docker::commands::docker_hosts,
            docker::commands::docker_probe,
            docker::commands::docker_set_host_mode,
            docker::commands::docker_overview,
            docker::commands::docker_stats,
            docker::commands::docker_container_action,
            docker::commands::docker_stack_action,
            docker::commands::docker_forget_stack,
            docker::commands::docker_stack_files,
            docker::commands::docker_read_file,
            docker::commands::docker_save_file,
            docker::commands::docker_cleanup_preview,
            docker::commands::docker_cleanup,
            docker::commands::docker_logs_start,
            docker::commands::docker_logs_stop,
            docker::commands::docker_save_log,
            // --- end of Docker ---
            // --- Proxmox (src-tauri/src/proxmox/commands.rs) ---
            proxmox::commands::proxmox_settings,
            proxmox::commands::proxmox_save_settings,
            proxmox::commands::proxmox_status,
            proxmox::commands::proxmox_trust_certificate,
            proxmox::commands::proxmox_power,
            proxmox::commands::proxmox_guest_addresses,
            proxmox::commands::proxmox_snapshots,
            proxmox::commands::proxmox_take_snapshot,
            proxmox::commands::proxmox_rollback,
            proxmox::commands::proxmox_delete_snapshot,
            // --- end Proxmox ---
            // --- Pi-hole (src-tauri/src/pihole/commands.rs) ---
            pihole::commands::pihole_hosts,
            pihole::commands::pihole_add,
            pihole::commands::pihole_remove,
            pihole::commands::pihole_save_settings,
            pihole::commands::pihole_remove_password,
            pihole::commands::pihole_status,
            pihole::commands::pihole_trust_certificate,
            pihole::commands::pihole_set_blocking,
            pihole::commands::pihole_probe,
            // --- end Pi-hole ---
        ])
        .build(tauri::generate_context!())
        .expect("error while running home-cc")
        .run(|app, event| {
            // Open terminals say goodbye to their machines instead of just dropping off.
            if let tauri::RunEvent::Exit = event {
                tauri::async_runtime::block_on(app.state::<terminal::Terminals>().close_all());
                // Pi-hole: its API sessions are few, so each one home-cc holds is ended (pihole/auth.rs).
                tauri::async_runtime::block_on(app.state::<pihole::PiHole>().logout_all());
            }
        });
}
