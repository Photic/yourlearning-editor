mod controllers;

use controllers::youtube_learning::run_add_learning;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![run_add_learning])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
