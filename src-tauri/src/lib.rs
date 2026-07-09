use std::path::PathBuf;
use tauri::Manager;
use tauri_plugin_shell::ShellExt;

#[tauri::command]
async fn run_add_learning(app: tauri::AppHandle, url: String) -> Result<String, String> {
    // During `tauri dev` the binary lives in target/debug and resource_dir() points there,
    // but the script hasn't been copied. Use the source location in dev, resource_dir in release.
    let script_path: PathBuf = if cfg!(debug_assertions) {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("add-learning.sh")
    } else {
        app.path()
            .resource_dir()
            .map_err(|e| e.to_string())?
            .join("add-learning.sh")
    };

    let output = app
        .shell()
        .command("bash")
        .args([script_path.to_str().unwrap(), &url])
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
