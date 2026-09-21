use rusqlite::{Connection, params};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// 独立修复程序 - 用于诊断和修复私有文件管理器数据库
/// 功能：
///   1. 数据库完整性诊断
///   2. 导出数据库为SQL文件（数据库导出模式）
///   3. 从分片文件中还原所有源文件（源文件导出模式）
///   4. 检测并报告孤儿记录、重叠空间、used_bytes不一致等问题
fn main() {
    println!("=== 私有文件管理器 - 数据库修复工具 ===\n");

    let args: Vec<String> = std::env::args().collect();

    // 确定数据目录
    let data_dir = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        // 默认查找当前目录下的 pfm_data
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        exe_dir.join("pfm_data")
    };

    let db_path = data_dir.join("metadata.db");
    let store_dir = data_dir.join("store");

    if !db_path.exists() {
        eprintln!("错误: 数据库文件不存在: {}", db_path.display());
        eprintln!("用法: pfm-repair [数据目录路径]");
        std::process::exit(1);
    }

    println!("数据目录: {}", data_dir.display());
    println!("数据库: {}", db_path.display());
    println!("分片目录: {}\n", store_dir.display());

    // 显示菜单
    loop {
        println!("请选择操作:");
        println!("  1. 数据库完整性诊断");
        println!("  2. 导出数据库为SQL文件（数据库导出）");
        println!("  3. 还原所有源文件（源文件导出）");
        println!("  4. 全面诊断并修复");
        println!("  0. 退出");
        print!("> ");
        std::io::stdout().flush().ok();

        let mut input = String::new();
        std::io::stdin().read_line(&mut input).ok();
        let choice = input.trim();

        match choice {
            "1" => diagnose(&db_path, &store_dir),
            "2" => export_sql(&db_path, &data_dir),
            "3" => export_source_files(&db_path, &store_dir, &data_dir),
            "4" => full_diagnose_and_repair(&db_path, &store_dir, &data_dir),
            "0" => {
                println!("退出。");
                break;
            }
            _ => println!("无效选项，请重新选择。\n"),
        }
    }
}

/// 数据库完整性诊断
fn diagnose(db_path: &Path, store_dir: &Path) {
    println!("\n--- 数据库完整性诊断 ---\n");

    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("无法打开数据库: {}", e);
            return;
        }
    };

    let mut issues = Vec::new();

    // 1. SQLite 内置完整性检查
    print!("  SQLite完整性检查... ");
    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap_or_else(|_| "FAIL".to_string());
    if integrity == "ok" {
        println!("通过");
    } else {
        println!("失败: {}", integrity);
        issues.push(format!("SQLite完整性检查失败: {}", integrity));
    }

    // 2. 检查孤儿文件记录（files中有记录但file_folder中无关联）
    print!("  孤儿文件记录检查... ");
    let orphan_files: i64 = conn.query_row(
        "SELECT COUNT(*) FROM files WHERE file_id NOT IN (SELECT file_id FROM file_folder)",
        [], |row| row.get(0),
    ).unwrap_or(0);
    if orphan_files == 0 {
        println!("通过");
    } else {
        println!("发现 {} 条孤儿文件记录", orphan_files);
        issues.push(format!("发现 {} 条孤儿文件记录（files中有但file_folder中无关联）", orphan_files));
    }

    // 3. 检查孤儿分片记录（chunk_locations中有记录但files中无对应文件）
    print!("  孤儿分片记录检查... ");
    let orphan_chunks: i64 = conn.query_row(
        "SELECT COUNT(*) FROM chunk_locations WHERE file_id NOT IN (SELECT file_id FROM files)",
        [], |row| row.get(0),
    ).unwrap_or(0);
    if orphan_chunks == 0 {
        println!("通过");
    } else {
        println!("发现 {} 条孤儿分片记录", orphan_chunks);
        issues.push(format!("发现 {} 条孤儿分片记录（chunk_locations中有但files中无对应文件）", orphan_chunks));
    }

    // 4. 检查孤儿缩略图记录
    print!("  孤儿缩略图记录检查... ");
    let orphan_thumbs: i64 = conn.query_row(
        "SELECT COUNT(*) FROM thumbnail_index WHERE file_id NOT IN (SELECT file_id FROM files)",
        [], |row| row.get(0),
    ).unwrap_or(0);
    if orphan_thumbs == 0 {
        println!("通过");
    } else {
        println!("发现 {} 条孤儿缩略图记录", orphan_thumbs);
        issues.push(format!("发现 {} 条孤儿缩略图记录", orphan_thumbs));
    }

    // 5. 检查free_space重叠
    print!("  空闲空间重叠检查... ");
    let overlaps: i64 = conn.query_row(
        "SELECT COUNT(*) FROM free_space f1
         JOIN free_space f2 ON f1.store_file = f2.store_file
         AND f1.space_id < f2.space_id
         AND f1.offset < f2.offset + f2.length
         AND f2.offset < f1.offset + f1.length",
        [], |row| row.get(0),
    ).unwrap_or(0);
    if overlaps == 0 {
        println!("通过");
    } else {
        println!("发现 {} 对重叠的空闲空间", overlaps);
        issues.push(format!("发现 {} 对重叠的空闲空间记录", overlaps));
    }

    // 6. 检查used_bytes一致性
    print!("  store_files用量一致性检查... ");
    let mut stmt = conn.prepare(
        "SELECT sf.store_file, sf.used_bytes,
                COALESCE(SUM(cl.length), 0) as actual_used
         FROM store_files sf
         LEFT JOIN chunk_locations cl ON cl.store_file = sf.store_file
         GROUP BY sf.store_file
         HAVING sf.used_bytes != actual_used"
    ).unwrap();
    let mismatches: Vec<(String, i64, i64)> = stmt.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    }).unwrap().filter_map(|r| r.ok()).collect();
    if mismatches.is_empty() {
        println!("通过");
    } else {
        println!("发现 {} 个不一致", mismatches.len());
        for (sf, recorded, actual) in &mismatches {
            println!("    {} : 记录={} 实际={}", sf, recorded, actual);
            issues.push(format!("store_file {} used_bytes不一致: 记录={}, 实际={}", sf, recorded, actual));
        }
    }

    // 7. 检查分片数据是否可读
    print!("  分片数据可读性检查... ");
    let mut stmt = conn.prepare(
        "SELECT cl.store_file, cl.offset, cl.length, cl.file_id, cl.chunk_index
         FROM chunk_locations cl ORDER BY cl.file_id, cl.chunk_index"
    ).unwrap();
    let chunks: Vec<(String, i64, i64, i64, i32)> = stmt.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
    }).unwrap().filter_map(|r| r.ok()).collect();

    let mut unreadable = 0;
    for (store_file, offset, length, file_id, chunk_index) in &chunks {
        let store_path = store_dir.join(store_file);
        if !store_path.exists() {
            unreadable += 1;
            if unreadable <= 5 {
                println!("    分片文件不存在: {} (file_id={}, chunk={})", store_file, file_id, chunk_index);
            }
            continue;
        }
        let file_len = fs::metadata(&store_path).map(|m| m.len()).unwrap_or(0) as i64;
        if offset + length > file_len {
            unreadable += 1;
            if unreadable <= 5 {
                println!("    分片越界: {} offset={} length={} 文件大小={} (file_id={})", store_file, offset, length, file_len, file_id);
            }
        }
    }
    if unreadable == 0 {
        println!("通过 ({} 个分片)", chunks.len());
    } else {
        println!("发现 {} 个不可读分片 (共 {} 个)", unreadable, chunks.len());
        issues.push(format!("发现 {} 个不可读分片", unreadable));
    }

    // 8. 统计信息
    println!("\n--- 统计信息 ---");
    let file_count: i64 = conn.query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0)).unwrap_or(0);
    let folder_count: i64 = conn.query_row("SELECT COUNT(*) FROM folders", [], |row| row.get(0)).unwrap_or(0);
    let chunk_count: i64 = conn.query_row("SELECT COUNT(*) FROM chunk_locations", [], |row| row.get(0)).unwrap_or(0);
    let free_space_count: i64 = conn.query_row("SELECT COUNT(*) FROM free_space", [], |row| row.get(0)).unwrap_or(0);
    let total_size: i64 = conn.query_row("SELECT COALESCE(SUM(size_bytes), 0) FROM files", [], |row| row.get(0)).unwrap_or(0);
    let store_file_count: i64 = conn.query_row("SELECT COUNT(*) FROM store_files", [], |row| row.get(0)).unwrap_or(0);

    println!("  文件记录: {} 个", file_count);
    println!("  文件夹: {} 个", folder_count);
    println!("  分片记录: {} 个", chunk_count);
    println!("  空闲空间: {} 条", free_space_count);
    println!("  文件总大小: {}", format_size(total_size));
    println!("  分片文件: {} 个", store_file_count);

    // 汇总
    println!("\n--- 诊断结果 ---");
    if issues.is_empty() {
        println!("  未发现问题，数据库状态健康。");
    } else {
        println!("  发现 {} 个问题:", issues.len());
        for (i, issue) in issues.iter().enumerate() {
            println!("    {}. {}", i + 1, issue);
        }
    }
    println!();
}

/// 导出数据库为SQL文件
fn export_sql(db_path: &Path, data_dir: &Path) {
    println!("\n--- 导出数据库为SQL文件 ---\n");

    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("无法打开数据库: {}", e);
            return;
        }
    };

    let output_path = data_dir.join(format!("db_export_{}.sql", chrono::Utc::now().format("%Y%m%d_%H%M%S")));
    let mut output = match File::create(&output_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("无法创建输出文件: {}", e);
            return;
        }
    };

    writeln!(output, "-- 私有文件管理器数据库导出").ok();
    writeln!(output, "-- 导出时间: {}", chrono::Utc::now()).ok();
    writeln!(output, "-- 数据库路径: {}", db_path.display()).ok();
    writeln!(output).ok();

    // 导出表结构
    let tables = ["files", "folders", "file_folder", "chunk_locations", "store_files", "thumbnail_index", "free_space"];
    for table in &tables {
        writeln!(output, "-- ========== {} ==========", table).ok();

        // 导出CREATE语句
        let schema: String = conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name=?",
            params![table],
            |row| row.get(0),
        ).unwrap_or_default();
        if !schema.is_empty() {
            writeln!(output, "{};", schema).ok();
            writeln!(output).ok();
        }

        // 导出数据
        let mut stmt = match conn.prepare(&format!("SELECT * FROM {}", table)) {
            Ok(s) => s,
            Err(e) => {
                writeln!(output, "-- 错误: {}", e).ok();
                continue;
            }
        };

        let col_count = stmt.column_count();
        let rows = stmt.query_map([], |row| {
            let mut values = Vec::new();
            for i in 0..col_count {
                let val: String = match row.get::<_, String>(i) {
                    Ok(v) => format!("'{}'", v.replace('\'', "''")),
                    Err(_) => {
                        // 尝试作为整数
                        match row.get::<_, i64>(i) {
                            Ok(v) => v.to_string(),
                            Err(_) => "NULL".to_string(),
                        }
                    }
                };
                values.push(val);
            }
            Ok(values.join(", "))
        });

        let mut count = 0;
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                writeln!(output, "INSERT INTO {} VALUES ({});", table, row).ok();
                count += 1;
            }
        }
        writeln!(output, "-- {} 条记录", count).ok();
        writeln!(output).ok();
    }

    // 导出索引
    writeln!(output, "-- ========== 索引 ==========").ok();
    let mut stmt = conn.prepare(
        "SELECT sql FROM sqlite_master WHERE type='index' AND sql IS NOT NULL"
    ).unwrap();
    let indexes: Vec<String> = stmt.query_map([], |row| row.get(0))
        .unwrap().filter_map(|r| r.ok()).collect();
    for idx in indexes {
        writeln!(output, "{};", idx).ok();
    }

    println!("  数据库已导出到: {}", output_path.display());
    println!();
}

/// 从分片文件中还原所有源文件
fn export_source_files(db_path: &Path, store_dir: &Path, data_dir: &Path) {
    println!("\n--- 还原所有源文件 ---\n");

    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("无法打开数据库: {}", e);
            return;
        }
    };

    let output_dir = data_dir.join(format!("exported_files_{}", chrono::Utc::now().format("%Y%m%d_%H%M%S")));
    if let Err(e) = fs::create_dir_all(&output_dir) {
        eprintln!("无法创建输出目录: {}", e);
        return;
    }

    // 获取所有文件信息及其分片位置
    let mut stmt = conn.prepare(
        "SELECT f.file_id, f.name, f.ext, f.size_bytes
         FROM files f
         ORDER BY f.file_id"
    ).unwrap();

    let files: Vec<(i64, String, Option<String>, i64)> = stmt.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    }).unwrap().filter_map(|r| r.ok()).collect();

    println!("  找到 {} 个文件记录", files.len());
    println!("  输出目录: {}\n", output_dir.display());

    let mut success = 0;
    let mut failed = 0;

    for (file_id, name, _ext, size_bytes) in &files {
        // 获取分片位置
        let mut chunk_stmt = conn.prepare(
            "SELECT store_file, offset, length FROM chunk_locations
             WHERE file_id = ? ORDER BY chunk_index"
        ).unwrap();

        let chunks: Vec<(String, i64, i64)> = chunk_stmt.query_map(params![file_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        }).unwrap().filter_map(|r| r.ok()).collect();

        if chunks.is_empty() {
            println!("  [跳过] file_id={} name={} (无分片记录)", file_id, name);
            failed += 1;
            continue;
        }

        // 构建输出路径（按file_id组织，避免同名冲突）
        let file_output_dir = output_dir.join(format!("file_{}", file_id));
        fs::create_dir_all(&file_output_dir).ok();
        let output_path = file_output_dir.join(&name);

        // 还原文件
        let mut output_file = match File::create(&output_path) {
            Ok(f) => f,
            Err(e) => {
                println!("  [失败] file_id={} name={}: {}", file_id, name, e);
                failed += 1;
                continue;
            }
        };

        let mut file_ok = true;
        let mut total_read = 0i64;

        for (store_file, offset, length) in &chunks {
            let store_path = store_dir.join(store_file);
            let mut store = match File::open(&store_path) {
                Ok(f) => f,
                Err(e) => {
                    println!("  [失败] file_id={} 分片文件{}打开失败: {}", file_id, store_file, e);
                    file_ok = false;
                    break;
                }
            };

            if let Err(e) = store.seek(SeekFrom::Start(*offset as u64)) {
                println!("  [失败] file_id={} seek失败: {}", file_id, e);
                file_ok = false;
                break;
            }

            let mut buffer = vec![0u8; *length as usize];
            if let Err(e) = store.read_exact(&mut buffer) {
                println!("  [失败] file_id={} 读取失败: {}", file_id, e);
                file_ok = false;
                break;
            }

            if let Err(e) = output_file.write_all(&buffer) {
                println!("  [失败] file_id={} 写入失败: {}", file_id, e);
                file_ok = false;
                break;
            }

            total_read += length;
        }

        if file_ok {
            if total_read != *size_bytes {
                println!("  [警告] file_id={} name={} 大小不匹配: 预期={} 实际={}", file_id, name, size_bytes, total_read);
            }
            success += 1;
            println!("  [成功] file_id={} name={} ({})", file_id, name, format_size(*size_bytes));
        } else {
            failed += 1;
            let _ = fs::remove_file(&output_path);
        }
    }

    println!("\n  导出完成: 成功 {} 个, 失败 {} 个", success, failed);
    println!("  输出目录: {}\n", output_dir.display());
}

/// 全面诊断并尝试修复
fn full_diagnose_and_repair(db_path: &Path, store_dir: &Path, data_dir: &Path) {
    println!("\n--- 全面诊断并修复 ---\n");

    // 先诊断
    diagnose(db_path, store_dir);

    println!("是否尝试自动修复? (y/N): ");
    std::io::stdout().flush().ok();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).ok();
    if input.trim().to_lowercase() != "y" {
        println!("取消修复。");
        return;
    }

    // 修复前先备份
    let backup_path = data_dir.join(format!("metadata_backup_{}.db", chrono::Utc::now().format("%Y%m%d_%H%M%S")));
    println!("\n  备份数据库到: {}", backup_path.display());
    if let Err(e) = fs::copy(db_path, &backup_path) {
        eprintln!("  备份失败: {}", e);
        return;
    }
    // 同时备份WAL和SHM文件
    let wal_path = db_path.with_extension("db-wal");
    if wal_path.exists() {
        let _ = fs::copy(&wal_path, backup_path.with_extension("db-wal"));
    }
    let shm_path = db_path.with_extension("db-shm");
    if shm_path.exists() {
        let _ = fs::copy(&shm_path, backup_path.with_extension("db-shm"));
    }
    println!("  备份完成。\n");

    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("无法打开数据库: {}", e);
            return;
        }
    };
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;").ok();

    // 1. 清理孤儿分片记录
    print!("  清理孤儿分片记录... ");
    let deleted = conn.execute(
        "DELETE FROM chunk_locations WHERE file_id NOT IN (SELECT file_id FROM files)",
        [],
    ).unwrap_or(0);
    println!("删除 {} 条", deleted);

    // 2. 清理孤儿缩略图记录
    print!("  清理孤儿缩略图记录... ");
    let deleted = conn.execute(
        "DELETE FROM thumbnail_index WHERE file_id NOT IN (SELECT file_id FROM files)",
        [],
    ).unwrap_or(0);
    println!("删除 {} 条", deleted);

    // 3. 清理孤儿文件记录（无folder关联的）
    print!("  清理孤儿文件记录... ");
    let deleted = conn.execute(
        "DELETE FROM files WHERE file_id NOT IN (SELECT file_id FROM file_folder)",
        [],
    ).unwrap_or(0);
    println!("删除 {} 条", deleted);

    // 4. 清理重叠的free_space
    print!("  清理重叠空闲空间... ");
    let deleted = conn.execute(
        "DELETE FROM free_space WHERE space_id IN (
            SELECT f2.space_id FROM free_space f1
            JOIN free_space f2 ON f1.store_file = f2.store_file
            AND f1.space_id < f2.space_id
            AND f1.offset < f2.offset + f2.length
            AND f2.offset < f1.offset + f1.length
        )",
        [],
    ).unwrap_or(0);
    println!("删除 {} 条", deleted);

    // 5. 修正used_bytes
    print!("  修正store_files用量... ");
    let updated = conn.execute(
        "UPDATE store_files SET used_bytes = (
            SELECT COALESCE(SUM(cl.length), 0) FROM chunk_locations cl
            WHERE cl.store_file = store_files.store_file
        )",
        [],
    ).unwrap_or(0);
    println!("更新 {} 条", updated);

    // 6. 清理与chunk_locations重叠的free_space（已被占用的空间不应在free_space中）
    print!("  清理被占用的空闲空间... ");
    let deleted = conn.execute(
        "DELETE FROM free_space WHERE EXISTS (
            SELECT 1 FROM chunk_locations cl
            WHERE cl.store_file = free_space.store_file
            AND cl.offset < free_space.offset + free_space.length
            AND free_space.offset < cl.offset + cl.length
        )",
        [],
    ).unwrap_or(0);
    println!("删除 {} 条", deleted);

    // 7. VACUUM 压缩数据库
    print!("  压缩数据库... ");
    match conn.execute_batch("VACUUM") {
        Ok(_) => println!("完成"),
        Err(e) => println!("失败: {}", e),
    }

    println!("\n  修复完成。如有问题，可从备份恢复:");
    println!("  备份路径: {}\n", backup_path.display());
}

fn format_size(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
