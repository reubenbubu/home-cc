mod commands;
mod error;
mod inventory;
mod secrets;

use inventory::Inventory;
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::list_devices,
            commands::add_device,
            commands::update_device,
            commands::remove_device,
            commands::set_secret,
            commands::has_secret,
            commands::delete_secret,
        ])
        .run(tauri::generate_context!())
        .expect("error while running home-cc");
}
