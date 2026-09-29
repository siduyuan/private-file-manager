use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::blob_store::BlobStore;
use crate::db::Database;

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

/// 计算分片数量
pub fn get_chunk_count(file_size: i64) -> i32 {
    if file_size <= 0 {
        return 0;
    }
    let chunk_size = get_chunk_size(file_size);
    ((file_size + chunk_size - 1) / chunk_size) as i32
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
/// 分片写入 emdb，元数据写入 SQLite
pub fn import_file(
    db: &Database,
    blob: &BlobStore,
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

    // 先插入 DB 记录获取 file_id
    let file_id = db.insert_file(&file_info, folder_id)
        .map_err(|e| format!("DB insert error: {}", e))?;

    // 分片写入 emdb
    let chunk_size = get_chunk_size(file_size);
    let mut source_file = File::open(source_path)
        .map_err(|e| format!("Failed to open source: {}", e))?;

    let mut remaining = file_size;
    let mut chunk_index = 0i32;

    while remaining > 0 {
        let current_chunk_size = std::cmp::min(chunk_size, remaining);
        let mut buffer = vec![0u8; current_chunk_size as usize];
        source_file.read_exact(&mut buffer)
            .map_err(|e| format!("Read error: {}", e))?;

        blob.write_chunk(file_id, chunk_index, &buffer)?;

        remaining -= current_chunk_size;
        chunk_index += 1;
    }

    Ok(file_id)
}

/// Export a file from storage to a target path
pub fn export_file(
    db: &Database,
    blob: &BlobStore,
    file_id: i64,
    target_path: &Path,
) -> Result<PathBuf, String> {
    let file_info = db.get_file_info(file_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or_else(|| "File not found".to_string())?;

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

    let mut output_file = File::create(&output_path)
        .map_err(|e| format!("Failed to create output: {}", e))?;

    let chunk_count = get_chunk_count(file_info.size_bytes);
    for i in 0..chunk_count {
        let data = blob.read_chunk(file_id, i)?;
        output_file.write_all(&data)
            .map_err(|e| format!("Write error: {}", e))?;
    }

    Ok(output_path)
}

/// Read file chunks into a temporary file and return the temp file path
pub fn extract_to_temp(
    db: &Database,
    blob: &BlobStore,
    temp_dir: &Path,
    file_id: i64,
) -> Result<PathBuf, String> {
    let file_info = db.get_file_info(file_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or_else(|| "File not found".to_string())?;

    // Create temp file with original extension
    let ext = file_info.ext.as_deref().unwrap_or("");
    let temp_name = format!("{}_{}.{}", file_id, uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("tmp"), ext);
    let temp_path = temp_dir.join(&temp_name);

    let mut output_file = File::create(&temp_path)
        .map_err(|e| format!("Failed to create temp: {}", e))?;

    let chunk_count = get_chunk_count(file_info.size_bytes);
    for i in 0..chunk_count {
        let data = blob.read_chunk(file_id, i)?;
        output_file.write_all(&data)
            .map_err(|e| format!("Write error: {}", e))?;
    }

    Ok(temp_path)
}

/// Generate thumbnail for an image file
pub fn generate_image_thumbnail(
    db: &Database,
    blob: &BlobStore,
    thumb_dir: &Path,
    temp_dir: &Path,
    file_id: i64,
) -> Result<(), String> {
    let temp_path = extract_to_temp(db, blob, temp_dir, file_id)?;

    // 根据文件内容自动检测格式，不依赖扩展名
    let img = match image::ImageReader::open(&temp_path) {
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

/// Generate thumbnail for a video file using ffmpeg
pub fn generate_video_thumbnail(
    db: &Database,
    blob: &BlobStore,
    thumb_dir: &Path,
    temp_dir: &Path,
    file_id: i64,
) -> Result<(), String> {
    let temp_path = extract_to_temp(db, blob, temp_dir, file_id)?;

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
