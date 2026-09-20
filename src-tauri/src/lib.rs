mod db;
mod models;
mod storage;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::Manager;
use db::Database;
use models::*;

struct AppState {
    db: Database,
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
async fn import_files(
    state: tauri::State<'_, Mutex<AppState>>,
    file_paths: Vec<String>,
    folder_id: i64,
) -> Result<ImportResult, String> {
    let (db, store_dir, thumb_dir, temp_dir) = {
        let state = state.lock().unwrap();
        (
            state.db.clone(),
            state.store_dir.clone(),
            state.thumb_dir.clone(),
            state.temp_dir.clone(),
        )
    };

    let mut success_count = 0;
    let mut fail_count = 0;
    let mut errors = Vec::new();

    // Process each path
    for path_str in &file_paths {
        let path = PathBuf::from(path_str);
        
        if path.is_file() {
            // Import single file directly to target folder
            match storage::import_file(&db, &store_dir, &path, folder_id) {
                Ok(file_id) => {
                    // Generate thumbnail for images and videos
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    match ext.to_lowercase().as_str() {
                        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" => {
                            let _ = storage::generate_image_thumbnail(
                                &db, &store_dir, &thumb_dir, &temp_dir, file_id,
                            );
                        }
                        "mp4" | "avi" | "mkv" | "mov" | "wmv" | "flv" | "webm" => {
                            if let Err(e) = storage::generate_video_thumbnail(
                                &db, &store_dir, &thumb_dir, &temp_dir, file_id,
                            ) {
                                eprintln!("[thumbnail] video failed (file_id={}, path={}): {}", file_id, path_str, e);
                            }
                        }
                        _ => {}
                    }
                    success_count += 1;
                }
                Err(e) => {
                    fail_count += 1;
                    errors.push(format!("{}: {}", path_str, e));
                }
            }
        } else if path.is_dir() {
            // Import directory: create folder structure first, then import files
            let result = import_directory(&db, &store_dir, &thumb_dir, &temp_dir, &path, folder_id);
            success_count += result.0;
            fail_count += result.1;
            errors.extend(result.2);
        }
    }

    Ok(ImportResult {
        success_count,
        fail_count,
        errors,
    })
}

// Import a directory with its structure preserved
fn import_directory(
    db: &db::Database,
    store_dir: &Path,
    thumb_dir: &Path,
    temp_dir: &Path,
    source_dir: &Path,
    target_folder_id: i64,
) -> (i32, i32, Vec<String>) {
    let mut success_count = 0;
    let mut fail_count = 0;
    let mut errors = Vec::new();

    // Get the directory name
    let dir_name = source_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("imported_folder");

    // Create a new folder in the target location
    let new_folder_id = match db.create_folder(dir_name, Some(target_folder_id)) {
        Ok(id) => id,
        Err(e) => {
            errors.push(format!("创建文件夹 {} 失败: {}", dir_name, e));
            return (0, 1, errors);
        }
    };

    // Read directory contents
    let entries = match std::fs::read_dir(source_dir) {
        Ok(entries) => entries,
        Err(e) => {
            errors.push(format!("读取目录 {} 失败: {}", source_dir.display(), e));
            return (0, 1, errors);
        }
    };

    // Process each entry
    for entry in entries.flatten() {
        let entry_path = entry.path();
        
        if entry_path.is_file() {
            // Import file to the newly created folder
            match storage::import_file(db, store_dir, &entry_path, new_folder_id) {
                Ok(file_id) => {
                    // Generate thumbnail for images and videos
                    let ext = entry_path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    match ext.to_lowercase().as_str() {
                        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" => {
                            let _ = storage::generate_image_thumbnail(
                                db, store_dir, thumb_dir, temp_dir, file_id,
                            );
                        }
                        "mp4" | "avi" | "mkv" | "mov" | "wmv" | "flv" | "webm" => {
                            let _ = storage::generate_video_thumbnail(
                                db, store_dir, thumb_dir, temp_dir, file_id,
                            );
                        }
                        _ => {}
                    }
                    success_count += 1;
                }
                Err(e) => {
                    fail_count += 1;
                    errors.push(format!("{}: {}", entry_path.display(), e));
                }
            }
        } else if entry_path.is_dir() {
            // Recursively import subdirectory
            let result = import_directory(db, store_dir, thumb_dir, temp_dir, &entry_path, new_folder_id);
            success_count += result.0;
            fail_count += result.1;
            errors.extend(result.2);
        }
    }

    (success_count, fail_count, errors)
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
fn delete_folder(
    state: tauri::State<Mutex<AppState>>,
    folder_id: i64,
) -> Result<(), String> {
    let state = state.lock().unwrap();
    state.db.delete_folder(folder_id).map_err(|e| e.to_string())
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

#[derive(serde::Serialize)]
struct DirEntry {
    name: String,
    path: String,
    is_dir: bool,
    size: u64,
}

#[tauri::command]
fn list_dir(dir_path: String) -> Result<Vec<DirEntry>, String> {
    let path = Path::new(&dir_path);
    if !path.exists() {
        return Err("路径不存在".to_string());
    }
    if !path.is_dir() {
        return Err("不是目录".to_string());
    }

    let mut entries = Vec::new();
    let dir_entries = std::fs::read_dir(path).map_err(|e| e.to_string())?;

    for entry in dir_entries {
        if let Ok(entry) = entry {
            let metadata = entry.metadata().ok();
            let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path().to_string_lossy().to_string();

            entries.push(DirEntry {
                name,
                path,
                is_dir,
                size,
            });
        }
    }

    // Sort: folders first, then files, both alphabetically
    entries.sort_by(|a, b| {
        match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    });

    Ok(entries)
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

            // Ensure ffmpeg binary is available for video thumbnail generation
            let _ = ffmpeg_sidecar::download::auto_download();

            let db_path = app_data_dir.join("metadata.db");
            let database = Database::new(&db_path).expect("Failed to initialize database");

            let state = AppState {
                db: database,
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
            delete_folder,
            rename_file,
            move_file,
            search_files,
            get_thumbnail_path,
            list_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
