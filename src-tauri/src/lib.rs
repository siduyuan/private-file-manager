mod db;
mod models;
mod registry;
mod storage;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sha2::{Sha256, Digest};
use tauri::Manager;
use db::Database;
use models::*;
use registry::{Registry, DbConnection};

struct AppState {
    db: Database,
    store_dir: PathBuf,
    thumb_dir: PathBuf,
    temp_dir: PathBuf,
    registry: Registry,
    current_db_uuid: String,
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
    // Windows风格：同级不能有同名文件夹
    if state.db.check_folder_name_in_parent(parent_id, &name, None).map_err(|e| e.to_string())? {
        return Err(format!("该文件夹下已存在名为 '{}' 的文件夹", name));
    }
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
    folder_ids: Vec<i64>,
    target_dir: String,
) -> Result<Vec<String>, String> {
    let state = state.lock().unwrap();
    let target = PathBuf::from(&target_dir);
    let mut exported = Vec::new();

    // 导出文件
    for file_id in file_ids {
        let path = storage::export_file(&state.db, &state.store_dir, file_id, &target)?;
        exported.push(path.to_str().unwrap_or("").to_string());
    }

    // 导出文件夹（递归）
    for folder_id in folder_ids {
        export_folder_recursive(&state.db, &state.store_dir, folder_id, &target, &mut exported)?;
    }

    Ok(exported)
}

fn export_folder_recursive(
    db: &Database,
    store_dir: &Path,
    folder_id: i64,
    target_base: &Path,
    exported: &mut Vec<String>,
) -> Result<(), String> {
    // 获取文件夹信息
    let folders = db.get_folders().map_err(|e| e.to_string())?;
    let folder = folders.iter().find(|f| f.folder_id == folder_id)
        .ok_or_else(|| format!("文件夹 {} 不存在", folder_id))?;
    
    // 创建文件夹
    let folder_path = target_base.join(&folder.name);
    std::fs::create_dir_all(&folder_path).map_err(|e| format!("创建文件夹失败: {}", e))?;
    
    // 导出文件夹内的文件
    let files = db.get_files_in_folder(folder_id).map_err(|e| e.to_string())?;
    for file in files {
        let path = storage::export_file(db, store_dir, file.file_id, &folder_path)?;
        exported.push(path.to_str().unwrap_or("").to_string());
    }
    
    // 递归导出子文件夹
    let sub_folders: Vec<i64> = folders.iter()
        .filter(|f| f.parent_id == Some(folder_id))
        .map(|f| f.folder_id)
        .collect();
    
    for sub_folder_id in sub_folders {
        export_folder_recursive(db, store_dir, sub_folder_id, &folder_path, exported)?;
    }
    
    Ok(())
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
    
    // Get thumbnail path before deletion
    let thumb_path = state.db.get_thumbnail_path(file_id).map_err(|e| e.to_string())?;
    
    // Delete from database (free_space is recorded atomically in the transaction)
    state.db.delete_file(file_id).map_err(|e| e.to_string())?;
    
    // Delete thumbnail file if exists
    if let Some(path) = thumb_path {
        let thumb_file = std::path::Path::new(&path);
        if thumb_file.exists() {
            let _ = std::fs::remove_file(thumb_file);
        }
    }
    
    Ok(())
}

#[tauri::command]
fn delete_folder(
    state: tauri::State<Mutex<AppState>>,
    folder_id: i64,
) -> Result<(), String> {
    let state = state.lock().unwrap();
    
    // Get all thumbnail paths before deletion
    let file_ids = state.db.get_all_file_ids_in_folder_recursive(folder_id).map_err(|e| e.to_string())?;
    let mut thumb_paths = Vec::new();
    for file_id in &file_ids {
        if let Ok(Some(thumb_path)) = state.db.get_thumbnail_path(*file_id) {
            thumb_paths.push(thumb_path);
        }
    }
    
    // Delete folder and all associated database records (free_space recorded atomically)
    state.db.delete_folder(folder_id).map_err(|e| e.to_string())?;
    
    // Delete all thumbnail files
    for path in thumb_paths {
        let thumb_file = std::path::Path::new(&path);
        if thumb_file.exists() {
            let _ = std::fs::remove_file(thumb_file);
        }
    }
    
    Ok(())
}

#[tauri::command]
fn rename_file(
    state: tauri::State<Mutex<AppState>>,
    file_id: i64,
    new_name: String,
) -> Result<(), String> {
    let state = state.lock().unwrap();
    // 获取文件当前所在文件夹
    let folder_id = state.db.get_file_folder_id(file_id).map_err(|e| e.to_string())?
        .ok_or_else(|| "文件不存在".to_string())?;
    // Windows风格：同级不能有同名文件
    if state.db.check_file_name_in_folder(folder_id, &new_name, Some(file_id)).map_err(|e| e.to_string())? {
        return Err(format!("该文件夹下已存在名为 '{}' 的文件", new_name));
    }
    state.db.rename_file(file_id, &new_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn rename_folder(
    state: tauri::State<Mutex<AppState>>,
    folder_id: i64,
    new_name: String,
) -> Result<(), String> {
    let state = state.lock().unwrap();
    // 获取文件夹的父文件夹
    let folders = state.db.get_folders().map_err(|e| e.to_string())?;
    let parent_id = folders.iter().find(|f| f.folder_id == folder_id).and_then(|f| f.parent_id);
    // Windows风格：同级不能有同名文件夹
    if state.db.check_folder_name_in_parent(parent_id, &new_name, Some(folder_id)).map_err(|e| e.to_string())? {
        return Err(format!("该文件夹下已存在名为 '{}' 的文件夹", new_name));
    }
    state.db.rename_folder(folder_id, &new_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn move_file(
    state: tauri::State<Mutex<AppState>>,
    file_id: i64,
    target_folder_id: i64,
) -> Result<(), String> {
    let state = state.lock().unwrap();
    // 获取文件名
    let file_info = state.db.get_file_info(file_id).map_err(|e| e.to_string())?
        .ok_or_else(|| "文件不存在".to_string())?;
    // Windows风格：目标文件夹下不能有同名文件
    if state.db.check_file_name_in_folder(target_folder_id, &file_info.name, Some(file_id)).map_err(|e| e.to_string())? {
        return Err(format!("目标文件夹下已存在名为 '{}' 的文件", file_info.name));
    }
    state.db.move_file(file_id, target_folder_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn move_folder(
    state: tauri::State<Mutex<AppState>>,
    folder_id: i64,
    target_parent_id: Option<i64>,
) -> Result<(), String> {
    let state = state.lock().unwrap();
    let folders = state.db.get_folders().map_err(|e| e.to_string())?;
    let folder = folders.iter().find(|f| f.folder_id == folder_id)
        .ok_or_else(|| "文件夹不存在".to_string())?;
    // 不能移动到自身或自身的子文件夹
    if target_parent_id == Some(folder_id) {
        return Err("不能将文件夹移动到自身".to_string());
    }
    // 检查是否是子文件夹（防止循环引用）
    let mut check_id = target_parent_id;
    while let Some(pid) = check_id {
        if pid == folder_id {
            return Err("不能将文件夹移动到其子文件夹中".to_string());
        }
        check_id = folders.iter().find(|f| f.folder_id == pid).and_then(|f| f.parent_id);
    }
    // Windows风格：目标父文件夹下不能有同名文件夹
    if state.db.check_folder_name_in_parent(target_parent_id, &folder.name, Some(folder_id)).map_err(|e| e.to_string())? {
        return Err(format!("目标文件夹下已存在名为 '{}' 的文件夹", folder.name));
    }
    state.db.move_folder(folder_id, target_parent_id).map_err(|e| e.to_string())
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

// ==================== 前后端分离命令 ====================

/// 获取文件夹完整内容（子文件夹 + 文件），前端无需做任何合并
#[tauri::command]
fn get_folder_contents(
    state: tauri::State<Mutex<AppState>>,
    folder_id: i64,
) -> Result<FolderContents, String> {
    let state = state.lock().unwrap();
    let all_folders = state.db.get_folders().map_err(|e| e.to_string())?;
    let files = state.db.get_files_in_folder(folder_id).map_err(|e| e.to_string())?;

    let mut items: Vec<FolderContentsItem> = Vec::new();

    // 子文件夹
    for f in &all_folders {
        if f.parent_id == Some(folder_id) {
            items.push(FolderContentsItem {
                id: f.folder_id,
                name: f.name.clone(),
                is_folder: true,
                size_bytes: 0,
                category: None,
                ext: None,
                thumbnail_path: None,
                updated_at: f.created_at,
            });
        }
    }

    // 文件
    for file in &files {
        items.push(FolderContentsItem {
            id: file.file_id,
            name: file.name.clone(),
            is_folder: false,
            size_bytes: file.size_bytes,
            category: file.category.clone(),
            ext: file.ext.clone(),
            thumbnail_path: file.thumbnail_path.clone(),
            updated_at: file.updated_at,
        });
    }

    Ok(FolderContents { items })
}

/// 获取面包屑路径，后端负责路径拼装
#[tauri::command]
fn get_breadcrumb_path(
    state: tauri::State<Mutex<AppState>>,
    folder_id: i64,
) -> Result<Vec<BreadcrumbItem>, String> {
    let state = state.lock().unwrap();
    let all_folders = state.db.get_folders().map_err(|e| e.to_string())?;

    let mut path = Vec::new();
    let mut current = all_folders.iter().find(|f| f.folder_id == folder_id);
    while let Some(f) = current {
        path.push(BreadcrumbItem {
            id: f.folder_id,
            name: f.name.clone(),
        });
        current = f.parent_id.and_then(|pid| all_folders.iter().find(|f| f.folder_id == pid));
    }
    path.reverse();
    Ok(path)
}

/// 批量删除（混合文件和文件夹ID），后端负责类型判断
/// free_space在db层事务中原子记录，无需调用方单独处理
#[tauri::command]
fn batch_delete(
    state: tauri::State<Mutex<AppState>>,
    ids: Vec<i64>,
) -> Result<BatchResult, String> {
    let state = state.lock().unwrap();
    let all_folders = state.db.get_folders().map_err(|e| e.to_string())?;
    let mut success_count = 0;
    let mut fail_count = 0;
    let mut errors = Vec::new();

    for id in &ids {
        let is_folder = all_folders.iter().any(|f| f.folder_id == *id);
        if is_folder {
            // 收集缩略图路径（只读）
            let mut thumbs_to_delete = Vec::new();
            if let Ok(file_ids) = state.db.get_all_file_ids_in_folder_recursive(*id) {
                for file_id in &file_ids {
                    if let Ok(Some(thumb_path)) = state.db.get_thumbnail_path(*file_id) {
                        thumbs_to_delete.push(thumb_path);
                    }
                }
            }
            // DB删除（含free_space原子记录）
            match state.db.delete_folder(*id) {
                Ok(_) => {
                    for thumb_path in &thumbs_to_delete {
                        let thumb_file = std::path::Path::new(thumb_path);
                        if thumb_file.exists() {
                            let _ = std::fs::remove_file(thumb_file);
                        }
                    }
                    success_count += 1;
                }
                Err(e) => {
                    fail_count += 1;
                    errors.push(format!("删除文件夹 {} 失败: {}", id, e));
                }
            }
        } else {
            let thumb_path = state.db.get_thumbnail_path(*id).ok().flatten();
            match state.db.delete_file(*id) {
                Ok(_) => {
                    if let Some(path) = thumb_path {
                        let thumb_file = std::path::Path::new(&path);
                        if thumb_file.exists() {
                            let _ = std::fs::remove_file(thumb_file);
                        }
                    }
                    success_count += 1;
                }
                Err(e) => {
                    fail_count += 1;
                    errors.push(format!("删除文件 {} 失败: {}", id, e));
                }
            }
        }
    }

    Ok(BatchResult { success_count, fail_count, errors })
}

/// 批量移动（混合文件和文件夹ID），后端负责类型判断
#[tauri::command]
fn batch_move(
    state: tauri::State<Mutex<AppState>>,
    ids: Vec<i64>,
    target_folder_id: i64,
) -> Result<BatchResult, String> {
    let state = state.lock().unwrap();
    let all_folders = state.db.get_folders().map_err(|e| e.to_string())?;
    let mut success_count = 0;
    let mut fail_count = 0;
    let mut errors = Vec::new();

    for id in &ids {
        let is_folder = all_folders.iter().any(|f| f.folder_id == *id);
        if is_folder {
            if *id == target_folder_id {
                fail_count += 1;
                errors.push("不能将文件夹移动到自身".to_string());
                continue;
            }
            // 防止循环引用
            let mut check_id = Some(target_folder_id);
            let mut is_circular = false;
            while let Some(pid) = check_id {
                if pid == *id {
                    is_circular = true;
                    break;
                }
                check_id = all_folders.iter().find(|f| f.folder_id == pid).and_then(|f| f.parent_id);
            }
            if is_circular {
                fail_count += 1;
                errors.push("不能将文件夹移动到其子文件夹中".to_string());
                continue;
            }
            let folder = all_folders.iter().find(|f| f.folder_id == *id).unwrap();
            if state.db.check_folder_name_in_parent(Some(target_folder_id), &folder.name, Some(*id)).map_err(|e| e.to_string())? {
                fail_count += 1;
                errors.push(format!("目标文件夹下已存在名为 '{}' 的文件夹", folder.name));
                continue;
            }
            match state.db.move_folder(*id, Some(target_folder_id)) {
                Ok(_) => success_count += 1,
                Err(e) => {
                    fail_count += 1;
                    errors.push(format!("移动文件夹 {} 失败: {}", id, e));
                }
            }
        } else {
            let file_info = state.db.get_file_info(*id).map_err(|e| e.to_string())?
                .ok_or_else(|| "文件不存在".to_string())?;
            if state.db.check_file_name_in_folder(target_folder_id, &file_info.name, Some(*id)).map_err(|e| e.to_string())? {
                fail_count += 1;
                errors.push(format!("目标文件夹下已存在名为 '{}' 的文件", file_info.name));
                continue;
            }
            match state.db.move_file(*id, target_folder_id) {
                Ok(_) => success_count += 1,
                Err(e) => {
                    fail_count += 1;
                    errors.push(format!("移动文件 {} 失败: {}", id, e));
                }
            }
        }
    }

    Ok(BatchResult { success_count, fail_count, errors })
}

/// 批量导出（混合文件和文件夹ID），后端负责类型判断和递归导出
#[tauri::command]
fn batch_export(
    state: tauri::State<Mutex<AppState>>,
    ids: Vec<i64>,
    target_dir: String,
) -> Result<Vec<String>, String> {
    let state = state.lock().unwrap();
    let target = PathBuf::from(&target_dir);
    let all_folders = state.db.get_folders().map_err(|e| e.to_string())?;
    let mut exported = Vec::new();

    for id in &ids {
        let is_folder = all_folders.iter().any(|f| f.folder_id == *id);
        if is_folder {
            export_folder_recursive(&state.db, &state.store_dir, *id, &target, &mut exported)?;
        } else {
            let path = storage::export_file(&state.db, &state.store_dir, *id, &target)?;
            exported.push(path.to_str().unwrap_or("").to_string());
        }
    }

    Ok(exported)
}

// ==================== 辅助函数 ====================

fn hash_password(password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn get_app_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
        })
        .join("PrivateFileManager")
}

fn get_db_subdirs(db_dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    (
        db_dir.join("store"),
        db_dir.join("thumb_cache"),
        db_dir.join("temp"),
    )
}

fn create_db_dirs(db_dir: &Path) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let (store_dir, thumb_dir, temp_dir) = get_db_subdirs(db_dir);
    std::fs::create_dir_all(&store_dir).map_err(|e| format!("创建store目录失败: {}", e))?;
    std::fs::create_dir_all(thumb_dir.join("img")).map_err(|e| format!("创建thumb目录失败: {}", e))?;
    std::fs::create_dir_all(thumb_dir.join("vid")).ok();
    std::fs::create_dir_all(&temp_dir).map_err(|e| format!("创建temp目录失败: {}", e))?;
    Ok((store_dir, thumb_dir, temp_dir))
}

#[allow(dead_code)]
fn find_root_folder(db: &Database) -> Result<i64, String> {
    let folders = db.get_folders().map_err(|e| e.to_string())?;
    folders.iter()
        .find(|f| f.parent_id.is_none())
        .map(|f| f.folder_id)
        .ok_or_else(|| "根目录不存在".to_string())
}

// ==================== 数据库管理命令 ====================

#[tauri::command]
fn list_connections(state: tauri::State<Mutex<AppState>>) -> Result<Vec<DbConnectionInfo>, String> {
    let state = state.lock().unwrap();
    let mut result = Vec::new();
    for conn in state.registry.connections() {
        let db_path = Path::new(&conn.path).join("metadata.db");
        let (has_password, db_type) = if db_path.exists() {
            match Database::new(&db_path) {
                Ok(db) => {
                    let props = db.get_db_properties().ok();
                    (
                        props.as_ref().and_then(|p| p.password_hash.clone()).is_some(),
                        props.map(|p| p.db_type).unwrap_or_else(|| conn.db_type.clone()),
                    )
                }
                Err(_) => (false, conn.db_type.clone()),
            }
        } else {
            (false, conn.db_type.clone())
        };
        result.push(DbConnectionInfo {
            uuid: conn.uuid.clone(),
            display_name: conn.display_name.clone(),
            db_type,
            path: conn.path.clone(),
            is_connected: conn.uuid == state.current_db_uuid,
            is_default: conn.is_default,
            has_password,
        });
    }
    Ok(result)
}

#[tauri::command]
fn get_current_db(state: tauri::State<Mutex<AppState>>) -> Result<DbConnectionInfo, String> {
    let state = state.lock().unwrap();
    let conn = state.registry.get_connection(&state.current_db_uuid)
        .ok_or("当前数据库未找到")?;
    let props = state.db.get_db_properties().map_err(|e| e.to_string())?;
    Ok(DbConnectionInfo {
        uuid: conn.uuid.clone(),
        display_name: props.display_name,
        db_type: props.db_type,
        path: conn.path.clone(),
        is_connected: true,
        is_default: conn.is_default,
        has_password: props.password_hash.is_some(),
    })
}

#[tauri::command]
fn create_database(
    state: tauri::State<Mutex<AppState>>,
    request: CreateDbRequest,
) -> Result<CreateDbResult, String> {
    let db_dir = PathBuf::from(&request.path);
    if db_dir.join("metadata.db").exists() {
        return Err("该路径已存在数据库".to_string());
    }
    let (store_dir, _thumb_dir, _temp_dir) = create_db_dirs(&db_dir)?;
    let db_path = db_dir.join("metadata.db");
    let database = Database::new(&db_path).map_err(|e| e.to_string())?;
    let uuid = uuid::Uuid::new_v4().to_string();
    let display_name = request.display_name
        .unwrap_or_else(|| uuid[uuid.len()-3..].to_string());
    database.init_db_properties(&uuid, &display_name, &request.db_type).map_err(|e| e.to_string())?;
    if let Some(ref pw) = request.password {
        if !pw.is_empty() {
            database.set_password_hash(Some(&hash_password(pw))).map_err(|e| e.to_string())?;
        }
    }
    drop(database);
    drop(store_dir);
    let mut state = state.lock().unwrap();
    state.registry.add_connection(DbConnection {
        uuid: uuid.clone(),
        display_name: display_name.clone(),
        db_type: request.db_type,
        path: request.path,
        is_default: false,
    })?;
    Ok(CreateDbResult { uuid, display_name })
}

#[tauri::command]
fn connect_database(
    state: tauri::State<Mutex<AppState>>,
    path: String,
    password: Option<String>,
) -> Result<DbConnectionInfo, String> {
    let db_dir = PathBuf::from(&path);
    let db_path = db_dir.join("metadata.db");
    if !db_path.exists() {
        return Err("数据库文件不存在".to_string());
    }
    let database = Database::new(&db_path).map_err(|e| e.to_string())?;
    let props = database.get_db_properties().map_err(|e| e.to_string())?;
    if props.uuid.is_empty() {
        return Err("无效的数据库：缺少UUID".to_string());
    }
    if let Some(ref stored_hash) = props.password_hash {
        let pw = password.ok_or("需要密码")?;
        let hash = hash_password(&pw);
        if hash != *stored_hash {
            return Err("密码错误".to_string());
        }
    } else if password.is_some() {
        // No password set but one was provided - that's fine, just ignore
    }
    let (valid, details) = database.check_integrity().map_err(|e| e.to_string())?;
    if !valid {
        return Err(format!("数据库完整性检查失败: {:?}", details));
    }
    let uuid = props.uuid.clone();
    let display_name = props.display_name.clone();
    let db_type = props.db_type.clone();
    let has_password = props.password_hash.is_some();
    drop(database);
    let mut state = state.lock().unwrap();
    if state.registry.get_connection(&uuid).is_none() {
        state.registry.add_connection(DbConnection {
            uuid: uuid.clone(),
            display_name: display_name.clone(),
            db_type: db_type.clone(),
            path: path.clone(),
            is_default: false,
        })?;
    }
    Ok(DbConnectionInfo {
        uuid,
        display_name,
        db_type,
        path,
        is_connected: false,
        is_default: false,
        has_password,
    })
}

#[tauri::command]
fn disconnect_database(
    state: tauri::State<Mutex<AppState>>,
    uuid: String,
) -> Result<(), String> {
    let state = state.lock().unwrap();
    if state.current_db_uuid == uuid {
        return Err("不能断开当前正在使用的数据库".to_string());
    }
    let conn = state.registry.get_connection(&uuid)
        .ok_or("数据库未找到")?;
    if conn.is_default {
        return Err("默认数据库不可断开".to_string());
    }
    Ok(())
}

#[tauri::command]
fn remove_database(
    state: tauri::State<Mutex<AppState>>,
    uuid: String,
) -> Result<(), String> {
    let mut state = state.lock().unwrap();
    if state.current_db_uuid == uuid {
        return Err("不能移除当前正在使用的数据库".to_string());
    }
    state.registry.remove_connection(&uuid)
}

#[tauri::command]
fn switch_database(
    state: tauri::State<Mutex<AppState>>,
    uuid: String,
) -> Result<(), String> {
    let mut state = state.lock().unwrap();
    if state.current_db_uuid == uuid {
        return Ok(());
    }
    let conn = state.registry.get_connection(&uuid)
        .ok_or("数据库未找到")?
        .clone();
    let db_path = PathBuf::from(&conn.path).join("metadata.db");
    if !db_path.exists() {
        return Err("数据库文件不存在".to_string());
    }
    let database = Database::new(&db_path).map_err(|e| e.to_string())?;
    let (store_dir, thumb_dir, temp_dir) = create_db_dirs(Path::new(&conn.path))?;
    state.db = database;
    state.store_dir = store_dir;
    state.thumb_dir = thumb_dir;
    state.temp_dir = temp_dir;
    state.current_db_uuid = uuid;
    Ok(())
}

#[tauri::command]
fn set_database_password(
    state: tauri::State<Mutex<AppState>>,
    uuid: String,
    password: Option<String>,
) -> Result<(), String> {
    let state = state.lock().unwrap();
    let conn = state.registry.get_connection(&uuid)
        .ok_or("数据库未找到")?;
    let db_path = PathBuf::from(&conn.path).join("metadata.db");
    let database = Database::new(&db_path).map_err(|e| e.to_string())?;
    match password {
        Some(pw) if !pw.is_empty() => {
            database.set_password_hash(Some(&hash_password(&pw))).map_err(|e| e.to_string())?;
        }
        _ => {
            database.set_password_hash(None).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
fn reset_database_password(
    state: tauri::State<Mutex<AppState>>,
    uuid: String,
    old_password: String,
    new_password: String,
) -> Result<(), String> {
    let state = state.lock().unwrap();
    let conn = state.registry.get_connection(&uuid)
        .ok_or("数据库未找到")?;
    let db_path = PathBuf::from(&conn.path).join("metadata.db");
    let database = Database::new(&db_path).map_err(|e| e.to_string())?;
    let props = database.get_db_properties().map_err(|e| e.to_string())?;
    if let Some(ref stored_hash) = props.password_hash {
        let old_hash = hash_password(&old_password);
        if old_hash != *stored_hash {
            return Err("原密码错误".to_string());
        }
    } else {
        return Err("数据库未设置密码".to_string());
    }
    if new_password.is_empty() {
        database.set_password_hash(None).map_err(|e| e.to_string())?;
    } else {
        database.set_password_hash(Some(&hash_password(&new_password))).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn rename_database(
    state: tauri::State<Mutex<AppState>>,
    uuid: String,
    new_name: String,
) -> Result<(), String> {
    let mut state = state.lock().unwrap();
    let conn = state.registry.get_connection(&uuid)
        .ok_or("数据库未找到")?
        .clone();
    let db_path = PathBuf::from(&conn.path).join("metadata.db");
    let database = Database::new(&db_path).map_err(|e| e.to_string())?;
    database.set_display_name(&new_name).map_err(|e| e.to_string())?;
    state.registry.update_display_name(&uuid, new_name)?;
    Ok(())
}

#[tauri::command]
fn change_database_path(
    state: tauri::State<Mutex<AppState>>,
    uuid: String,
    new_path: String,
) -> Result<(), String> {
    let mut state = state.lock().unwrap();
    let conn = state.registry.get_connection(&uuid)
        .ok_or("数据库未找到")?
        .clone();
    let old_dir = PathBuf::from(&conn.path);
    let new_dir = PathBuf::from(&new_path);
    if !old_dir.exists() {
        return Err("原数据库目录不存在".to_string());
    }
    if new_dir.exists() && new_dir.join("metadata.db").exists() {
        let existing_db = Database::new(&new_dir.join("metadata.db")).map_err(|e| e.to_string())?;
        let props = existing_db.get_db_properties().map_err(|e| e.to_string())?;
        if !props.uuid.is_empty() {
            return Err("目标路径已存在有效数据库，请先处理冲突".to_string());
        }
    }
    std::fs::create_dir_all(&new_dir).map_err(|e| format!("创建目标目录失败: {}", e))?;
    // 迁移文件
    let items = ["metadata.db", "metadata.db-wal", "metadata.db-shm"];
    for item in &items {
        let src = old_dir.join(item);
        let dst = new_dir.join(item);
        if src.exists() && !src.is_dir() {
            std::fs::rename(&src, &dst).or_else(|_| std::fs::copy(&src, &dst).map(|_| ()))
                .map_err(|e| format!("迁移{}失败: {}", item, e))?;
        }
    }
    let dir_items = ["store", "thumb_cache"];
    for item in &dir_items {
        let src = old_dir.join(item);
        let dst = new_dir.join(item);
        if src.exists() {
            if dst.exists() {
                let _ = std::fs::remove_dir_all(&dst);
            }
            std::fs::rename(&src, &dst).map_err(|e| format!("迁移{}失败: {}", item, e))?;
        }
    }
    // 清理旧目录
    if old_dir.exists() {
        let _ = std::fs::remove_dir_all(&old_dir);
    }
    state.registry.update_path(&uuid, new_path.clone())?;
    // 如果迁移的是当前数据库，更新路径
    if state.current_db_uuid == uuid {
        let (store_dir, thumb_dir, temp_dir) = get_db_subdirs(&new_dir);
        state.store_dir = store_dir;
        state.thumb_dir = thumb_dir;
        state.temp_dir = temp_dir;
    }
    Ok(())
}

#[tauri::command]
fn verify_password(
    state: tauri::State<Mutex<AppState>>,
    uuid: String,
    password: String,
) -> Result<PasswordVerifyResult, String> {
    let state = state.lock().unwrap();
    let conn = state.registry.get_connection(&uuid)
        .ok_or("数据库未找到")?;
    let db_path = PathBuf::from(&conn.path).join("metadata.db");
    let database = Database::new(&db_path).map_err(|e| e.to_string())?;
    let props = database.get_db_properties().map_err(|e| e.to_string())?;
    match props.password_hash {
        Some(ref stored_hash) => {
            let hash = hash_password(&password);
            Ok(PasswordVerifyResult {
                valid: hash == *stored_hash,
                has_password: true,
            })
        }
        None => Ok(PasswordVerifyResult {
            valid: true,
            has_password: false,
        }),
    }
}

#[tauri::command]
fn check_integrity(
    state: tauri::State<Mutex<AppState>>,
    uuid: String,
) -> Result<IntegrityCheckResult, String> {
    let state = state.lock().unwrap();
    let conn = if uuid == state.current_db_uuid {
        let (valid, details) = state.db.check_integrity().map_err(|e| e.to_string())?;
        return Ok(IntegrityCheckResult { valid, details });
    } else {
        state.registry.get_connection(&uuid)
            .ok_or("数据库未找到")?
            .clone()
    };
    let db_path = PathBuf::from(&conn.path).join("metadata.db");
    let database = Database::new(&db_path).map_err(|e| e.to_string())?;
    let (valid, details) = database.check_integrity().map_err(|e| e.to_string())?;
    Ok(IntegrityCheckResult { valid, details })
}

#[tauri::command]
fn check_auth_status(state: tauri::State<Mutex<AppState>>) -> Result<bool, String> {
    let state = state.lock().unwrap();
    let default_conn = state.registry.get_default_connection()
        .ok_or("默认数据库未配置")?;
    let db_path = PathBuf::from(&default_conn.path).join("metadata.db");
    if !db_path.exists() {
        return Ok(true); // No DB = no auth needed
    }
    let database = Database::new(&db_path).map_err(|e| e.to_string())?;
    let props = database.get_db_properties().map_err(|e| e.to_string())?;
    Ok(props.password_hash.is_none())
}

#[tauri::command]
fn login(
    state: tauri::State<Mutex<AppState>>,
    password: String,
) -> Result<bool, String> {
    let state = state.lock().unwrap();
    let default_conn = state.registry.get_default_connection()
        .ok_or("默认数据库未配置")?;
    let db_path = PathBuf::from(&default_conn.path).join("metadata.db");
    let database = Database::new(&db_path).map_err(|e| e.to_string())?;
    let props = database.get_db_properties().map_err(|e| e.to_string())?;
    match props.password_hash {
        Some(ref stored_hash) => {
            let hash = hash_password(&password);
            Ok(hash == *stored_hash)
        }
        None => Ok(true),
    }
}

#[tauri::command]
fn change_password(
    state: tauri::State<Mutex<AppState>>,
    uuid: String,
    old_password: String,
    new_password: String,
) -> Result<(), String> {
    let state = state.lock().unwrap();
    let conn = state.registry.get_connection(&uuid)
        .ok_or("数据库未找到")?;
    let db_path = PathBuf::from(&conn.path).join("metadata.db");
    let database = Database::new(&db_path).map_err(|e| e.to_string())?;
    let props = database.get_db_properties().map_err(|e| e.to_string())?;
    if let Some(ref stored_hash) = props.password_hash {
        let old_hash = hash_password(&old_password);
        if old_hash != *stored_hash {
            return Err("原密码错误".to_string());
        }
    }
    if new_password.is_empty() {
        database.set_password_hash(None).map_err(|e| e.to_string())?;
    } else {
        database.set_password_hash(Some(&hash_password(&new_password))).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn change_default_db_path(
    state: tauri::State<Mutex<AppState>>,
    new_path: String,
) -> Result<(), String> {
    let mut state = state.lock().unwrap();
    let default_conn = state.registry.get_default_connection()
        .ok_or("默认数据库未配置")?
        .clone();
    let old_dir = PathBuf::from(&default_conn.path);
    let new_dir = PathBuf::from(&new_path);
    if old_dir == new_dir {
        return Err("新路径与当前路径相同".to_string());
    }
    // 创建新目录结构
    std::fs::create_dir_all(&new_dir).map_err(|e| format!("创建目录失败: {}", e))?;
    let new_db_path = new_dir.join("metadata.db");
    if new_db_path.exists() {
        // 目标已有数据库，检查是否有效
        match Database::new(&new_db_path) {
            Ok(db) => {
                let props = db.get_db_properties().map_err(|e| e.to_string())?;
                if !props.uuid.is_empty() {
                    return Err("目标路径已存在有效数据库".to_string());
                }
            }
            Err(_) => {
                // 无效数据库，备份后覆盖
                let backup = new_dir.with_extension("bak");
                let _ = std::fs::rename(&new_dir, &backup);
                std::fs::create_dir_all(&new_dir).ok();
            }
        }
    }
    // 迁移文件
    let items = ["metadata.db", "metadata.db-wal", "metadata.db-shm"];
    for item in &items {
        let src = old_dir.join(item);
        let dst = new_dir.join(item);
        if src.exists() && !src.is_dir() {
            std::fs::rename(&src, &dst).or_else(|_| std::fs::copy(&src, &dst).map(|_| ()))
                .map_err(|e| format!("迁移{}失败: {}", item, e))?;
        }
    }
    let dir_items = ["store", "thumb_cache"];
    for item in &dir_items {
        let src = old_dir.join(item);
        let dst = new_dir.join(item);
        if src.exists() {
            if dst.exists() {
                let _ = std::fs::remove_dir_all(&dst);
            }
            std::fs::rename(&src, &dst).map_err(|e| format!("迁移{}失败: {}", item, e))?;
        }
    }
    // 清理旧目录
    if old_dir.exists() {
        let _ = std::fs::remove_dir_all(&old_dir);
    }
    // 更新注册表
    state.registry.update_path(&default_conn.uuid, new_path.clone())?;
    state.registry.set_default_db_path(new_path.clone());
    // 更新当前状态
    let (store_dir, thumb_dir, temp_dir) = get_db_subdirs(&new_dir);
    state.store_dir = store_dir;
    state.thumb_dir = thumb_dir;
    state.temp_dir = temp_dir;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let app_data_dir = get_app_data_dir();
            std::fs::create_dir_all(&app_data_dir).ok();

            // 确保 ffmpeg 可用
            let _ = ffmpeg_sidecar::download::auto_download();

            // 加载注册表
            let config_path = app_data_dir.join("config.json");
            let mut registry = Registry::load(&config_path)
                .expect("无法加载注册表");

            let default_db_dir = app_data_dir.join("databases").join("default");

            // 迁移旧数据
            let legacy_dir = app_data_dir.join("pfm_data");
            if legacy_dir.exists() && legacy_dir.join("metadata.db").exists()
                && !default_db_dir.exists()
            {
                std::fs::create_dir_all(&default_db_dir).ok();
                let legacy_items = ["metadata.db", "metadata.db-wal", "metadata.db-shm"];
                for item in &legacy_items {
                    let src = legacy_dir.join(item);
                    let dst = default_db_dir.join(item);
                    if src.exists() {
                        let _ = std::fs::rename(&src, &dst);
                    }
                }
                let legacy_dirs = ["store", "thumb_cache"];
                for item in &legacy_dirs {
                    let src = legacy_dir.join(item);
                    let dst = default_db_dir.join(item);
                    if src.exists() {
                        let _ = std::fs::rename(&src, &dst);
                    }
                }
                let _ = std::fs::remove_dir_all(legacy_dir.join("temp"));
                let _ = std::fs::remove_dir_all(&legacy_dir);
            }

            // 初始化默认数据库
            let (store_dir, thumb_dir, temp_dir) = create_db_dirs(&default_db_dir)
                .expect("无法创建默认数据库目录");

            let db_path = default_db_dir.join("metadata.db");
            let database = Database::new(&db_path).expect("无法初始化数据库");

            // 注册默认数据库
            if registry.get_default_connection().is_none() {
                let uuid = uuid::Uuid::new_v4().to_string();
                let display_name = uuid[uuid.len()-3..].to_string();
                database.init_db_properties(&uuid, &display_name, "local").ok();
                registry.add_connection(DbConnection {
                    uuid: uuid.clone(),
                    display_name,
                    db_type: "local".to_string(),
                    path: default_db_dir.to_string_lossy().to_string(),
                    is_default: true,
                }).ok();
                registry.set_default_db_path(default_db_dir.to_string_lossy().to_string());
            }

            let default_uuid = registry.get_default_connection()
                .map(|c| c.uuid.clone())
                .unwrap_or_default();

            // 连接其他已注册的数据库（仅记录，不打开）
            let connections = registry.connections().to_vec();
            for conn in &connections {
                if conn.is_default {
                    continue; // 默认数据库已在上面打开
                }
                let conn_db_path = PathBuf::from(&conn.path).join("metadata.db");
                if !conn_db_path.exists() {
                    continue; // 跳过不存在的数据库
                }
            }

            let state = AppState {
                db: database,
                store_dir,
                thumb_dir,
                temp_dir,
                registry,
                current_db_uuid: default_uuid,
            };

            app.manage(Mutex::new(state));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // 文件管理
            get_folders,
            get_files_in_folder,
            create_folder,
            import_files,
            export_files,
            open_file,
            delete_file,
            delete_folder,
            rename_file,
            rename_folder,
            move_file,
            move_folder,
            search_files,
            get_thumbnail_path,
            list_dir,
            get_folder_contents,
            get_breadcrumb_path,
            batch_delete,
            batch_move,
            batch_export,
            // 数据库管理
            list_connections,
            get_current_db,
            create_database,
            connect_database,
            disconnect_database,
            remove_database,
            switch_database,
            set_database_password,
            reset_database_password,
            rename_database,
            change_database_path,
            verify_password,
            check_integrity,
            // 认证
            check_auth_status,
            login,
            change_password,
            change_default_db_path,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
