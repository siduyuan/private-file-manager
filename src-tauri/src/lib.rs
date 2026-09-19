mod db;
mod models;
mod storage;

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::Manager;
use db::Database;
use models::*;

struct AppState {
    db: Database,
    data_dir: PathBuf,
    store_dir: PathBuf,
    thumb_dir: PathBuf,
    temp_dir: PathBuf,
}

#[tauri::command]
fn get_folders(state: tauri::State<Mutex<AppState>>) -> Result<Vec<FolderInfo>, String> {
    let state = state.lock().unwrap();
    state.db.get_folders().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_files_in_folder(
    state: tauri::State<Mutex<AppState>>,
    folder_id: i64,
) -> Result<Vec<FileInfo>, String> {
    let state = state.lock().unwrap();
    state.db.get_files_in_folder(folder_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn create_folder(
    state: tauri::State<Mutex<AppState>>,
    name: String,
    parent_id: Option<i64>,
) -> Result<i64, String> {
    let state = state.lock().unwrap();
    state.db.create_folder(&name, parent_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn import_files(
    state: tauri::State<Mutex<AppState>>,
    file_paths: Vec<String>,
    folder_id: i64,
) -> Result<ImportResult, String> {
    let state = state.lock().unwrap();
    let mut success_count = 0;
    let mut fail_count = 0;
    let mut errors = Vec::new();

    for path_str in &file_paths {
        let path = PathBuf::from(path_str);
        if path.is_file() {
            match storage::import_file(&state.db, &state.store_dir, &path, folder_id) {
                Ok(file_id) => {
                    // Try to generate thumbnail for images
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if matches!(ext.to_lowercase().as_str(), "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp") {
                        let _ = storage::generate_image_thumbnail(
                            &state.db,
                            &state.store_dir,
                            &state.thumb_dir,
                            &state.temp_dir,
                            file_id,
                        );
                    }
                    success_count += 1;
                }
                Err(e) => {
                    fail_count += 1;
                    errors.push(format!("{}: {}", path_str, e));
                }
            }
        } else if path.is_dir() {
            // Import all files in directory
            if let Ok(entries) = std::fs::read_dir(&path) {
                for entry in entries.flatten() {
                    let entry_path = entry.path();
                    if entry_path.is_file() {
                        match storage::import_file(
                            &state.db,
                            &state.store_dir,
                            &entry_path,
                            folder_id,
                        ) {
                            Ok(file_id) => {
                                let ext = entry_path.extension().and_then(|e| e.to_str()).unwrap_or("");
                                if matches!(ext.to_lowercase().as_str(), "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp") {
                                    let _ = storage::generate_image_thumbnail(
                                        &state.db,
                                        &state.store_dir,
                                        &state.thumb_dir,
                                        &state.temp_dir,
                                        file_id,
                                    );
                                }
                                success_count += 1;
                            }
                            Err(e) => {
                                fail_count += 1;
                                let p = entry_path.to_str().unwrap_or("?");
                                errors.push(format!("{}: {}", p, e));
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(ImportResult {
        success_count,
        fail_count,
        errors,
    })
}

#[tauri::command]
fn export_files(
    state: tauri::State<Mutex<AppState>>,
    file_ids: Vec<i64>,
    target_dir: String,
) -> Result<Vec<String>, String> {
    let state = state.lock().unwrap();
    let target = PathBuf::from(&target_dir);
    let mut exported = Vec::new();

    for file_id in file_ids {
        let path = storage::export_file(&state.db, &state.store_dir, file_id, &target)?;
        exported.push(path.to_str().unwrap_or("").to_string());
    }

    Ok(exported)
}

#[tauri::command]
fn open_file(
    state: tauri::State<Mutex<AppState>>,
    file_id: i64,
) -> Result<String, String> {
    let state = state.lock().unwrap();
    let temp_path = storage::extract_to_temp(
        &state.db,
        &state.store_dir,
        &state.temp_dir,
        file_id,
    )?;

    let path_str = temp_path.to_str().unwrap_or("").to_string();

    // Open with system default application
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &path_str])
            .spawn()
            .map_err(|e| format!("Failed to open: {}", e))?;
    }

    Ok(path_str)
}

#[tauri::command]
fn delete_file(
    state: tauri::State<Mutex<AppState>>,
    file_id: i64,
) -> Result<(), String> {
    let state = state.lock().unwrap();
    let _chunks = state.db.delete_file(file_id).map_err(|e| e.to_string())?;
    // Note: chunk data in store files is not physically removed
    // Could implement space reclamation later
    Ok(())
}

#[tauri::command]
fn rename_file(
    state: tauri::State<Mutex<AppState>>,
    file_id: i64,
    new_name: String,
) -> Result<(), String> {
    let state = state.lock().unwrap();
    state.db.rename_file(file_id, &new_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn move_file(
    state: tauri::State<Mutex<AppState>>,
    file_id: i64,
    target_folder_id: i64,
) -> Result<(), String> {
    let state = state.lock().unwrap();
    state.db.move_file(file_id, target_folder_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn search_files(
    state: tauri::State<Mutex<AppState>>,
    keyword: String,
) -> Result<Vec<FileInfo>, String> {
    let state = state.lock().unwrap();
    state.db.search_files(&keyword).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_thumbnail_path(
    state: tauri::State<Mutex<AppState>>,
    file_id: i64,
) -> Result<Option<String>, String> {
    let state = state.lock().unwrap();
    state.db.get_thumbnail_path(file_id).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            // Use a data directory next to the executable for development
            // In production, switch to app.path().app_data_dir()
            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let app_data_dir = exe_dir.join("pfm_data");
            std::fs::create_dir_all(&app_data_dir).ok();

            let store_dir = app_data_dir.join("store");
            let thumb_dir = app_data_dir.join("thumb_cache");
            let temp_dir = app_data_dir.join("temp");

            std::fs::create_dir_all(&store_dir).ok();
            std::fs::create_dir_all(&thumb_dir.join("img")).ok();
            std::fs::create_dir_all(&thumb_dir.join("vid")).ok();
            std::fs::create_dir_all(&temp_dir).ok();

            let db_path = app_data_dir.join("metadata.db");
            let database = Database::new(&db_path).expect("Failed to initialize database");

            let state = AppState {
                db: database,
                data_dir: app_data_dir,
                store_dir,
                thumb_dir,
                temp_dir,
            };

            app.manage(Mutex::new(state));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_folders,
            get_files_in_folder,
            create_folder,
            import_files,
            export_files,
            open_file,
            delete_file,
            rename_file,
            move_file,
            search_files,
            get_thumbnail_path,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
