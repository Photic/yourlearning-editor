use tauri_plugin_shell::ShellExt;

#[tauri::command]
async fn run_add_learning(app: tauri::AppHandle, url: String) -> Result<String, String> {
    let script_path = "./add-learning.sh";

    let output = app
        .shell()
        .command("bash")
        .args([script_path, &url])
        .output()
        .await
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![run_add_learning])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
