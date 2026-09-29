use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::db::Database;

const STORE_FILE_NAME: &str = "store.bin";

/// Determine chunk size based on file size
fn get_chunk_size(file_size: i64) -> i64 {
    if file_size < 100 * 1024 {
        // < 100KB: single chunk
        file_size
    } else if file_size < 50 * 1024 * 1024 {
        // 100KB - 50MB: 64KB chunks
        64 * 1024
    } else {
        // > 50MB: 256KB chunks
        256 * 1024
    }
}

/// Categorize file based on extension and size
pub fn categorize_file(ext: &str, size: i64) -> &'static str {
    let ext_lower = ext.to_lowercase();
    match ext_lower.as_str() {
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" => {
            if size > 50 * 1024 * 1024 {
                "video_long"
            } else {
                "video_short"
            }
        }
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "svg" | "ico" => "image",
        _ => "doc",
    }
}

/// Import a file into the storage system
/// 原子性保证：先写分片数据到store文件，再在一个事务中提交所有DB记录
pub fn import_file(
    db: &Database,
    store_dir: &Path,
    source_path: &Path,
    folder_id: i64,
) -> Result<i64, String> {
    let file_name = source_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "Invalid file name".to_string())?;

    let ext = source_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    let metadata = fs::metadata(source_path).map_err(|e| format!("Failed to read metadata: {}", e))?;
    let file_size = metadata.len() as i64;

    let mime_type = ext
        .as_deref()
        .map(|e| mime_guess::from_ext(e).first_or_octet_stream().to_string());

    let category = ext
        .as_deref()
        .map(|e| categorize_file(e, file_size))
        .map(|s| s.to_string());

    let now = chrono::Utc::now().timestamp();

    let file_info = crate::models::FileInfo {
        file_id: 0,
        name: file_name.to_string(),
        ext,
        mime_type,
        size_bytes: file_size,
        duration_sec: None,
        width: None,
        height: None,
        category,
        thumbnail_path: None,
        created_at: now,
        updated_at: now,
    };

    // 阶段1：写分片数据到store文件（不涉及DB写操作）
    let chunk_size = get_chunk_size(file_size);
    let mut source_file =
        File::open(source_path).map_err(|e| format!("Failed to open source: {}", e))?;

    // (store_file, offset, length, free_space_id or 0)
    let mut chunk_data: Vec<(String, i64, i64, i64)> = Vec::new();
    // (store_file, additional_bytes) for new appends
    let mut store_file_updates: Vec<(String, i64)> = Vec::new();
    // (store_file, offset, remaining_length) 需要回写的残余free_space
    let mut new_free_space: Vec<(String, i64, i64)> = Vec::new();
    // 需要确保在store_files表中存在的store文件
    let mut new_store_files: Vec<String> = Vec::new();

    let store_file_name = STORE_FILE_NAME.to_string();
    let store_path = store_dir.join(&store_file_name);

    // 确保 store.bin 在 store_files 表中有记录
    new_store_files.push(store_file_name.clone());

    let mut remaining = file_size;

    while remaining > 0 {
        let current_chunk_size = std::cmp::min(chunk_size, remaining);

        // Read chunk from source
        let mut buffer = vec![0u8; current_chunk_size as usize];
        source_file
            .read_exact(&mut buffer)
            .map_err(|e| format!("Read error: {}", e))?;

        // 优先使用free_space（Best-fit）
        let (current_offset, space_id, free_length) =
            match db.find_free_space(current_chunk_size).map_err(|e| format!("DB free space error: {}", e))? {
                Some((sid, _sf, off)) => {
                    let free_len = db.get_free_space_length(sid).map_err(|e| format!("DB error: {}", e))?;
                    (off, sid, free_len)
                }
                None => {
                    // 追加到 store.bin 末尾
                    let current_offset = fs::metadata(&store_path)
                        .map(|m| m.len())
                        .unwrap_or(0);
                    (current_offset as i64, 0i64, 0i64)
                }
            };

        // Write chunk to store file
        {
            let mut store = if space_id > 0 {
                // Writing to reused free space
                let mut f = OpenOptions::new()
                    .write(true)
                    .open(&store_path)
                    .map_err(|e| format!("Failed to open store file for reuse: {}", e))?;
                f.seek(SeekFrom::Start(current_offset as u64))
                    .map_err(|e| format!("Seek error: {}", e))?;
                f
            } else {
                // Appending to end of store file
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&store_path)
                    .map_err(|e| format!("Failed to open store file: {}", e))?
            };
            store
                .write_all(&buffer)
                .map_err(|e| format!("Write to store error: {}", e))?;
        }

        if space_id > 0 {
            // 使用了free_space，处理残余空间
            let remainder = free_length - current_chunk_size;
            if remainder > 0 {
                new_free_space.push((store_file_name.clone(), current_offset + current_chunk_size, remainder));
            }
        } else {
            // Track new append for store_files update
            if let Some(entry) = store_file_updates.iter_mut().find(|(sf, _)| sf == &store_file_name) {
                entry.1 += current_chunk_size;
            } else {
                store_file_updates.push((store_file_name.clone(), current_chunk_size));
            }
        }

        chunk_data.push((store_file_name.clone(), current_offset, current_chunk_size, space_id));

        remaining -= current_chunk_size;
    }

    // 阶段2：原子提交所有DB记录（文件 + 分片位置 + free_space消耗/回写 + store_files更新）
    let file_id = db
        .insert_file_with_chunks(&file_info, folder_id, &chunk_data, &store_file_updates, &new_free_space, &new_store_files)
        .map_err(|e| format!("DB insert error: {}", e))?;

    Ok(file_id)
}

/// Export a file from storage to a target path
pub fn export_file(
    db: &Database,
    store_dir: &Path,
    file_id: i64,
    target_path: &Path,
) -> Result<PathBuf, String> {
    let file_info = db
        .get_file_info(file_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or_else(|| "File not found".to_string())?;

    let chunks = db
        .get_chunk_locations(file_id)
        .map_err(|e| format!("DB chunk error: {}", e))?;

    let output_path = target_path.join(&file_info.name);

    // If file already exists, add a suffix
    let output_path = if output_path.exists() {
        let stem = output_path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
        let ext = output_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let mut counter = 1;
        loop {
            let new_name = if ext.is_empty() {
                format!("{} ({})", stem, counter)
            } else {
                format!("{} ({}).{}", stem, counter, ext)
            };
            let new_path = target_path.join(new_name);
            if !new_path.exists() {
                break new_path;
            }
            counter += 1;
        }
    } else {
        output_path
    };

    let mut output_file =
        File::create(&output_path).map_err(|e| format!("Failed to create output: {}", e))?;

    for chunk in &chunks {
        let store_path = store_dir.join(&chunk.store_file);
        let mut store_file =
            File::open(&store_path).map_err(|e| format!("Failed to open store: {}", e))?;
        store_file
            .seek(SeekFrom::Start(chunk.offset as u64))
            .map_err(|e| format!("Seek error: {}", e))?;

        let mut buffer = vec![0u8; chunk.length as usize];
        store_file
            .read_exact(&mut buffer)
            .map_err(|e| format!("Read error: {}", e))?;
        output_file
            .write_all(&buffer)
            .map_err(|e| format!("Write error: {}", e))?;
    }

    Ok(output_path)
}

/// Read file chunks into a temporary file and return the temp file path
pub fn extract_to_temp(
    db: &Database,
    store_dir: &Path,
    temp_dir: &Path,
    file_id: i64,
) -> Result<PathBuf, String> {
    let file_info = db
        .get_file_info(file_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or_else(|| "File not found".to_string())?;

    let chunks = db
        .get_chunk_locations(file_id)
        .map_err(|e| format!("DB chunk error: {}", e))?;

    // Create temp file with original extension
    let ext = file_info.ext.as_deref().unwrap_or("");
    let temp_name = format!("{}_{}.{}", file_id, uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("tmp"), ext);
    let temp_path = temp_dir.join(&temp_name);

    let mut output_file =
        File::create(&temp_path).map_err(|e| format!("Failed to create temp: {}", e))?;

    for chunk in &chunks {
        let store_path = store_dir.join(&chunk.store_file);
        let mut store_file =
            File::open(&store_path).map_err(|e| format!("Failed to open store: {}", e))?;
        store_file
            .seek(SeekFrom::Start(chunk.offset as u64))
            .map_err(|e| format!("Seek error: {}", e))?;

        let mut buffer = vec![0u8; chunk.length as usize];
        store_file
            .read_exact(&mut buffer)
            .map_err(|e| format!("Read error: {}", e))?;
        output_file
            .write_all(&buffer)
            .map_err(|e| format!("Write error: {}", e))?;
    }

    Ok(temp_path)
}

/// Generate thumbnail for an image file
pub fn generate_image_thumbnail(
    db: &Database,
    store_dir: &Path,
    thumb_dir: &Path,
    temp_dir: &Path,
    file_id: i64,
) -> Result<(), String> {
    let temp_path = extract_to_temp(db, store_dir, temp_dir, file_id)?;

    // 根据文件内容自动检测格式，不依赖扩展名（有些文件扩展名与实际格式不符）
    let img = match image::io::Reader::open(&temp_path) {
        Ok(reader) => match reader.with_guessed_format() {
            Ok(reader) => reader.decode(),
            Err(e) => Err(image::ImageError::IoError(e)),
        },
        Err(e) => Err(image::ImageError::IoError(e)),
    };
    let img = match img {
        Ok(img) => img,
        Err(e) => {
            let _ = fs::remove_file(&temp_path);
            return Err(format!("image decode failed: {}", e));
        }
    };

    let thumbnail = img.thumbnail(256, 256);

    let img_dir = thumb_dir.join("img");
    fs::create_dir_all(&img_dir).map_err(|e| format!("Create dir error: {}", e))?;

    let thumb_path = img_dir.join(format!("{}.webp", file_id));
    thumbnail
        .save(&thumb_path)
        .map_err(|e| {
            let _ = fs::remove_file(&temp_path);
            format!("save webp failed: {}", e)
        })?;

    let _ = fs::remove_file(&temp_path);

    db.set_thumbnail_path(file_id, thumb_path.to_str().unwrap_or(""))
        .map_err(|e| format!("DB thumbnail error: {}", e))?;

    Ok(())
}

/// 深度清理：将所有有效分片紧凑重写到单个 store.bin，释放碎片空间
/// 返回 (释放字节数, 旧物理大小, 新物理大小)
pub fn compact_store(
    db: &Database,
    store_dir: &Path,
) -> Result<(i64, i64, i64), String> {
    let store_path = store_dir.join(STORE_FILE_NAME);
    let temp_path = store_dir.join("store_compact.tmp");
    let backup_path = store_dir.join("store_old.bin");

    // 获取旧物理大小
    let old_physical = fs::metadata(&store_path)
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    // 获取所有有效分片（已按 store_file, offset 排序）
    let chunks = db.get_all_valid_chunks()?;

    // 如果没有分片，直接清空 store 文件
    if chunks.is_empty() {
        db.compact_update(&[])?;
        // 关闭所有句柄后删除
        drop(chunks);
        if store_path.exists() {
            let _ = fs::remove_file(&store_path);
        }
        return Ok((old_physical, old_physical, 0));
    }

    // 写入紧凑的临时文件，记录新的偏移量
    let mut new_chunks: Vec<(i64, i32, String, i64, i64)> = Vec::with_capacity(chunks.len());
    let mut current_offset: i64 = 0;

    {
        let mut temp_file = File::create(&temp_path)
            .map_err(|e| format!("创建临时文件失败: {}", e))?;

        for chunk in &chunks {
            // 从旧 store 文件读取数据（每个 chunk 独立打开/关闭，避免长时间持有句柄）
            let src_path = store_dir.join(&chunk.store_file);
            {
                let mut src_file = File::open(&src_path)
                    .map_err(|e| format!("打开 store 文件失败: {}", e))?;
                src_file.seek(SeekFrom::Start(chunk.offset as u64))
                    .map_err(|e| format!("Seek 失败: {}", e))?;

                let mut buffer = vec![0u8; chunk.length as usize];
                src_file.read_exact(&mut buffer)
                    .map_err(|e| format!("读取分片失败: {}", e))?;

                temp_file.write_all(&buffer)
                    .map_err(|e| format!("写入临时文件失败: {}", e))?;
            } // src_file 在这里被 drop，释放文件句柄

            new_chunks.push((
                chunk.file_id,
                chunk.chunk_index,
                STORE_FILE_NAME.to_string(),
                current_offset,
                chunk.length,
            ));

            current_offset += chunk.length;
        }
        temp_file.flush().map_err(|e| format!("flush 失败: {}", e))?;
    } // temp_file 在这里被 drop，释放文件句柄

    let new_physical = current_offset;
    let freed_bytes = old_physical - new_physical;

    // 原子更新 DB（事务内替换所有分片记录 + 清空 free_space）
    db.compact_update(&new_chunks)?;

    // 替换 store 文件（Windows 安全方式：rename swap）
    // 1. 先清理可能残留的 backup
    if backup_path.exists() {
        fs::remove_file(&backup_path)
            .map_err(|e| format!("清理旧备份文件失败: {}", e))?;
    }
    // 2. 旧 store.bin → backup
    if store_path.exists() {
        fs::rename(&store_path, &backup_path)
            .map_err(|e| format!("备份旧 store 文件失败: {}", e))?;
    }
    // 3. temp → store.bin
    fs::rename(&temp_path, &store_path)
        .map_err(|e| {
            // 回滚：把 backup 改回 store.bin
            let _ = fs::rename(&backup_path, &store_path);
            format!("替换 store 文件失败: {}", e)
        })?;
    // 4. 删除 backup
    let _ = fs::remove_file(&backup_path);

    // 清理旧的多 store 文件（向后兼容迁移）
    if let Ok(entries) = fs::read_dir(store_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("store_") && name.ends_with(".bin") && name != STORE_FILE_NAME {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    Ok((freed_bytes, old_physical, new_physical))
}

/// Generate thumbnail for a video file using ffmpeg
pub fn generate_video_thumbnail(
    db: &Database,
    store_dir: &Path,
    thumb_dir: &Path,
    temp_dir: &Path,
    file_id: i64,
) -> Result<(), String> {
    let temp_path = extract_to_temp(db, store_dir, temp_dir, file_id)?;

    let vid_dir = thumb_dir.join("vid");
    fs::create_dir_all(&vid_dir).map_err(|e| format!("Create dir error: {}", e))?;

    let thumb_path = vid_dir.join(format!("{}.webp", file_id));

    // Extract a single frame at 1 second using ffmpeg
    let mut child = ffmpeg_sidecar::command::FfmpegCommand::new()
        .args(["-y", "-ss", "1", "-i"])
        .arg(&temp_path)
        .args(["-vframes", "1", "-q:v", "2"])
        .arg(&thumb_path)
        .spawn()
        .map_err(|e| format!("FFmpeg spawn error: {}", e))?;

    // Consume the iterator to wait for ffmpeg to finish
    if let Ok(iter) = child.iter() {
        for _ in iter {}
    }

    // Clean up temp file
    let _ = fs::remove_file(&temp_path);

    // Check if ffmpeg exited successfully
    let status = child.as_inner_mut().wait().map_err(|e| format!("FFmpeg wait error: {}", e))?;
    if !status.success() {
        return Err("FFmpeg frame extraction failed".to_string());
    }

    db.set_thumbnail_path(file_id, thumb_path.to_str().unwrap_or(""))
        .map_err(|e| format!("DB thumbnail error: {}", e))?;

    Ok(())
}
