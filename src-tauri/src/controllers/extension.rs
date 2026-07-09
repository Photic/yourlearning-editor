use std::path::PathBuf;
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

/// Returns the directory where the bundled extension files live at runtime.
/// In debug builds this points directly to the source assets folder.
/// In release builds it points to the Tauri resource directory.
fn extension_source_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("extension"))
    } else {
        app.path()
            .resource_dir()
            .map(|p| p.join("extension"))
            .map_err(|e| e.to_string())
    }
}

/// Copies the bundled extension files into a stable folder in the user's data
/// directory and returns that path so Chrome can load it as an unpacked extension.
#[tauri::command]
pub fn export_extension(app: tauri::AppHandle) -> Result<String, String> {
    let src = extension_source_dir(&app)?;
    let dest = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("extension");

    std::fs::create_dir_all(&dest).map_err(|e| format!("Failed to create extension dir: {e}"))?;

    for entry in std::fs::read_dir(&src).map_err(|e| format!("Failed to read extension assets: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let dest_file = dest.join(entry.file_name());
        std::fs::copy(entry.path(), &dest_file)
            .map_err(|e| format!("Failed to copy {}: {e}", entry.file_name().to_string_lossy()))?;
    }

    Ok(dest.to_string_lossy().into_owned())
}

/// Opens the exported extension folder in the OS file manager so the user can
/// drag it into Chrome's extensions page.
#[tauri::command]
pub fn open_extension_folder(app: tauri::AppHandle) -> Result<(), String> {
    let path = export_extension(app.clone())?;
    app.opener()
        .open_path(&path, None::<&str>)
        .map_err(|e| format!("Failed to open folder: {e}"))
}
