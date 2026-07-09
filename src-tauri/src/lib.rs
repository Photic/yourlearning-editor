mod controllers;

use controllers::extension::{export_extension, open_extension_folder};
use controllers::youtube_learning::{run_add_learning, RelayState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(RelayState(std::sync::Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            run_add_learning,
            export_extension,
            open_extension_folder
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
