use crate::control::sqlite::{HistoryEntry, SqliteState};

const HF_API_TOKEN_KEY: &str = "HF_API_TOKEN";
const USE_AI_SUMMARY_KEY: &str = "USE_AI_SUMMARY";

#[tauri::command]
pub fn get_hf_api_token(sqlite: tauri::State<'_, SqliteState>) -> Result<Option<String>, String> {
    sqlite.get_setting(HF_API_TOKEN_KEY)
}

#[tauri::command]
pub fn set_hf_api_token(
    value: String,
    sqlite: tauri::State<'_, SqliteState>,
) -> Result<(), String> {
    println!("HF_API_TOKEN: {value}");
    sqlite.set_setting(HF_API_TOKEN_KEY, &value)
}

#[tauri::command]
pub fn get_use_ai_summary(sqlite: tauri::State<'_, SqliteState>) -> Result<Option<bool>, String> {
    match sqlite.get_setting(USE_AI_SUMMARY_KEY)? {
        Some(value) => Ok(Some(value == "true")),
        None => Ok(None),
    }
}

#[tauri::command]
pub fn set_use_ai_summary(
    value: bool,
    sqlite: tauri::State<'_, SqliteState>,
) -> Result<(), String> {
    sqlite.set_setting(USE_AI_SUMMARY_KEY, if value { "true" } else { "false" })
}

#[tauri::command]
pub fn get_history(sqlite: tauri::State<'_, SqliteState>) -> Result<Vec<HistoryEntry>, String> {
    sqlite.get_history(50)
}

#[tauri::command]
pub fn log_message(message: String) {
    println!("{message}");
}
