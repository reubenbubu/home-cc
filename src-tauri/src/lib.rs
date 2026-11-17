mod commands;
mod diag;
// Public so the small diagnostic programs in examples/ can use them.
pub mod error;
pub mod history;
mod hostkeys;
pub mod inventory;
mod monitor;
pub mod net;
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
            app.manage(inventory);
            app.manage(Monitor::default());
            app.manage(terminal::Terminals::default());
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
        ])
        .build(tauri::generate_context!())
        .expect("error while running home-cc")
        .run(|app, event| {
            // Open terminals say goodbye to their machines instead of just dropping off.
            if let tauri::RunEvent::Exit = event {
                tauri::async_runtime::block_on(app.state::<terminal::Terminals>().close_all());
            }
        });
}
