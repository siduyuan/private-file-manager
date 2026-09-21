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
    
    // Get chunk locations before deletion (for recording free space)
    let chunks = state.db.get_chunk_locations(file_id).map_err(|e| e.to_string())?;
    
    // Delete from database
    state.db.delete_file(file_id).map_err(|e| e.to_string())?;
    
    // Record freed space for later reuse
    for chunk in &chunks {
        state.db.add_free_space(&chunk.store_file, chunk.offset, chunk.length)
            .map_err(|e| format!("Failed to record free space: {}", e))?;
    }
    
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
    
    // Get all files in the folder and its subfolders before deletion
    let file_ids = state.db.get_all_file_ids_in_folder_recursive(folder_id).map_err(|e| e.to_string())?;
    
    // Collect chunk locations and thumbnail paths for all files
    let mut all_chunks = Vec::new();
    let mut thumb_paths = Vec::new();
    
    for file_id in &file_ids {
        if let Ok(Some(thumb_path)) = state.db.get_thumbnail_path(*file_id) {
            thumb_paths.push(thumb_path);
        }
        if let Ok(chunks) = state.db.get_chunk_locations(*file_id) {
            all_chunks.extend(chunks);
        }
    }
    
    // Delete folder and all associated database records
    state.db.delete_folder(folder_id).map_err(|e| e.to_string())?;
    
    // Record freed space for later reuse
    for chunk in &all_chunks {
        state.db.add_free_space(&chunk.store_file, chunk.offset, chunk.length)
            .map_err(|e| format!("Failed to record free space: {}", e))?;
    }
    
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
            // 收集文件夹递归下的所有文件信息
            if let Ok(file_ids) = state.db.get_all_file_ids_in_folder_recursive(*id) {
                for file_id in &file_ids {
                    if let Ok(chunks) = state.db.get_chunk_locations(*file_id) {
                        for chunk in &chunks {
                            let _ = state.db.add_free_space(&chunk.store_file, chunk.offset, chunk.length);
                        }
                    }
                    if let Ok(Some(thumb_path)) = state.db.get_thumbnail_path(*file_id) {
                        let thumb_file = std::path::Path::new(&thumb_path);
                        if thumb_file.exists() {
                            let _ = std::fs::remove_file(thumb_file);
                        }
                    }
                }
            }
            match state.db.delete_folder(*id) {
                Ok(_) => success_count += 1,
                Err(e) => {
                    fail_count += 1;
                    errors.push(format!("删除文件夹 {} 失败: {}", id, e));
                }
            }
        } else {
            if let Ok(Some(thumb_path)) = state.db.get_thumbnail_path(*id) {
                let thumb_file = std::path::Path::new(&thumb_path);
                if thumb_file.exists() {
                    let _ = std::fs::remove_file(thumb_file);
                }
            }
            if let Ok(chunks) = state.db.get_chunk_locations(*id) {
                for chunk in &chunks {
                    let _ = state.db.add_free_space(&chunk.store_file, chunk.offset, chunk.length);
                }
            }
            match state.db.delete_file(*id) {
                Ok(_) => success_count += 1,
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
            rename_folder,
            move_file,
            move_folder,
            search_files,
            get_thumbnail_path,
            list_dir,
            // 前后端分离命令
            get_folder_contents,
            get_breadcrumb_path,
            batch_delete,
            batch_move,
            batch_export,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
