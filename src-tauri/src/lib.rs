mod control;
mod controllers;

use control::sqlite::SqliteState;
use controllers::extension::{export_extension, open_extension_folder};
use controllers::token::{
    get_hf_api_token, get_use_ai_summary, log_message, set_hf_api_token, set_use_ai_summary,
};
use controllers::learning_entry::{run_add_learning, RelayState};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&app_data_dir).map_err(|e| e.to_string())?;
            let sqlite = SqliteState::new(app_data_dir.join("owls.sqlite"))?;
            app.manage(sqlite);
            Ok(())
        })
        .manage(RelayState(std::sync::Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            run_add_learning,
            export_extension,
            open_extension_folder,
            get_hf_api_token,
            set_hf_api_token,
            get_use_ai_summary,
            set_use_ai_summary,
            log_message
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
