use emdb::Emdb;
use rusqlite::{Connection, params};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// 独立修复程序 - 用于诊断和修复私有文件管理器数据库
/// 功能：
///   1. 数据库完整性诊断（SQLite + emdb）
///   2. 导出数据库为SQL文件
///   3. 从 emdb 中还原所有源文件
///   4. 检测并报告孤儿记录等问题
fn main() {
    println!("=== 私有文件管理器 - 数据库修复工具 (emdb) ===\n");

    let args: Vec<String> = std::env::args().collect();

    let data_dir = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        exe_dir.join("pfm_data")
    };

    let db_path = data_dir.join("metadata.db");
    let blob_path = data_dir.join("blobs.emdb");

    if !db_path.exists() {
        eprintln!("错误: 数据库文件不存在: {}", db_path.display());
        eprintln!("用法: pfm-repair [数据目录路径]");
        std::process::exit(1);
    }

    println!("数据目录: {}", data_dir.display());
    println!("数据库: {}", db_path.display());
    println!("Blob存储: {}\n", blob_path.display());

    loop {
        println!("请选择操作:");
        println!("  1. 数据库完整性诊断");
        println!("  2. 导出数据库为SQL文件");
        println!("  3. 还原所有源文件");
        println!("  4. 全面诊断并修复");
        println!("  5. 压缩 Blob 存储");
        println!("  0. 退出");
        print!("> ");
        std::io::stdout().flush().ok();

        let mut input = String::new();
        std::io::stdin().read_line(&mut input).ok();
        let choice = input.trim();

        match choice {
            "1" => diagnose(&db_path, &blob_path),
            "2" => export_sql(&db_path, &data_dir),
            "3" => export_source_files(&db_path, &blob_path, &data_dir),
            "4" => full_diagnose_and_repair(&db_path, &blob_path, &data_dir),
            "5" => compact_blob(&blob_path),
            "0" => {
                println!("退出。");
                break;
            }
            _ => println!("无效选项，请重新选择。\n"),
        }
    }
}

/// 数据库完整性诊断
fn diagnose(db_path: &Path, blob_path: &Path) {
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

    // 2. 检查孤儿文件记录
    print!("  孤儿文件记录检查... ");
    let orphan_files: i64 = conn.query_row(
        "SELECT COUNT(*) FROM files WHERE file_id NOT IN (SELECT file_id FROM file_folder)",
        [], |row| row.get(0),
    ).unwrap_or(0);
    if orphan_files == 0 {
        println!("通过");
    } else {
        println!("发现 {} 条孤儿文件记录", orphan_files);
        issues.push(format!("发现 {} 条孤儿文件记录", orphan_files));
    }

    // 3. 检查孤儿缩略图记录
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

    // 4. 检查 emdb blob 存储
    print!("  Blob存储检查... ");
    if blob_path.exists() {
        match Emdb::open(blob_path) {
            Ok(blob_db) => {
                match blob_db.stats() {
                    Ok(stats) => {
                        println!("通过 (记录数={}, 文件大小={})",
                            stats.live_records, format_size(stats.file_size_bytes as i64));
                    }
                    Err(e) => {
                        println!("统计失败: {}", e);
                        issues.push(format!("Blob统计失败: {}", e));
                    }
                }
            }
            Err(e) => {
                println!("打开失败: {}", e);
                issues.push(format!("Blob打开失败: {}", e));
            }
        }
    } else {
        println!("blob文件不存在");
        issues.push("Blob存储文件不存在".to_string());
    }

    // 5. 检查文件记录是否有对应的 blob 数据（抽样检查前10个文件）
    print!("  文件Blob可读性检查... ");
    {
        let mut stmt = conn.prepare(
            "SELECT file_id, name, size_bytes FROM files ORDER BY file_id LIMIT 10"
        ).unwrap();
        let files: Vec<(i64, String, i64)> = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        }).unwrap().filter_map(|r| r.ok()).collect();

        let mut readable = 0;
        let mut checked = 0;

        if let Ok(blob_db) = Emdb::open(blob_path) {
            for (file_id, name, size_bytes) in &files {
                let chunk_count = get_chunk_count(*size_bytes);
                let mut file_ok = true;
                for chunk_idx in 0..chunk_count {
                    let key = format!("f:{}:{}", file_id, chunk_idx);
                    match blob_db.get(key.as_bytes()) {
                        Ok(Some(_)) => {}
                        Ok(None) => {
                            file_ok = false;
                            break;
                        }
                        Err(_) => {
                            file_ok = false;
                            break;
                        }
                    }
                }
                if file_ok {
                    readable += 1;
                } else if checked < 5 {
                    println!("\n    Blob缺失: file_id={} name={}", file_id, name);
                }
                checked += 1;
            }
        }

        if files.is_empty() {
            println!("跳过 (无文件)");
        } else if readable == checked {
            println!("通过 (抽样 {} 个文件)", checked);
        } else {
            println!("{} / {} 个文件Blob不完整", checked - readable, checked);
            issues.push(format!("{} / {} 个文件Blob不可读", checked - readable, checked));
        }
    }

    // 6. 统计信息
    println!("\n--- 统计信息 ---");
    let file_count: i64 = conn.query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0)).unwrap_or(0);
    let folder_count: i64 = conn.query_row("SELECT COUNT(*) FROM folders", [], |row| row.get(0)).unwrap_or(0);
    let total_size: i64 = conn.query_row("SELECT COALESCE(SUM(size_bytes), 0) FROM files", [], |row| row.get(0)).unwrap_or(0);

    println!("  文件记录: {} 个", file_count);
    println!("  文件夹: {} 个", folder_count);
    println!("  文件总大小: {}", format_size(total_size));

    if blob_path.exists() {
        if let Ok(blob_db) = Emdb::open(blob_path) {
            if let Ok(stats) = blob_db.stats() {
                println!("  Blob记录数: {}", stats.live_records);
                println!("  Blob文件大小: {}", format_size(stats.file_size_bytes as i64));
            }
        }
    }

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

    writeln!(output, "-- 私有文件管理器数据库导出 (emdb)").ok();
    writeln!(output, "-- 导出时间: {}", chrono::Utc::now()).ok();
    writeln!(output, "-- 数据库路径: {}", db_path.display()).ok();
    writeln!(output).ok();

    let tables = ["files", "folders", "file_folder", "thumbnail_index", "db_properties"];
    for table in &tables {
        writeln!(output, "-- ========== {} ==========", table).ok();

        let schema: String = conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name=?",
            params![table],
            |row| row.get(0),
        ).unwrap_or_default();
        if !schema.is_empty() {
            writeln!(output, "{};", schema).ok();
            writeln!(output).ok();
        }

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
                        match row.get::<_, i64>(i) {
                            Ok(v) => v.to_string(),
                            Err(_) => {
                                match row.get::<_, f64>(i) {
                                    Ok(v) => v.to_string(),
                                    Err(_) => "NULL".to_string(),
                                }
                            }
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

/// 从 emdb 中还原所有源文件
fn export_source_files(db_path: &Path, blob_path: &Path, data_dir: &Path) {
    println!("\n--- 还原所有源文件 ---\n");

    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("无法打开数据库: {}", e);
            return;
        }
    };

    let blob_db = match Emdb::open(blob_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("无法打开Blob存储: {}", e);
            return;
        }
    };

    let output_dir = data_dir.join(format!("exported_files_{}", chrono::Utc::now().format("%Y%m%d_%H%M%S")));
    if let Err(e) = fs::create_dir_all(&output_dir) {
        eprintln!("无法创建输出目录: {}", e);
        return;
    }

    let mut stmt = conn.prepare(
        "SELECT f.file_id, f.name, f.size_bytes
         FROM files f
         ORDER BY f.file_id"
    ).unwrap();

    let files: Vec<(i64, String, i64)> = stmt.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    }).unwrap().filter_map(|r| r.ok()).collect();

    println!("  找到 {} 个文件记录", files.len());
    println!("  输出目录: {}\n", output_dir.display());

    let mut success = 0;
    let mut failed = 0;

    for (file_id, name, size_bytes) in &files {
        let chunk_count = get_chunk_count(*size_bytes);

        let file_output_path = output_dir.join(&name);
        // 避免同名覆盖
        let file_output_path = if file_output_path.exists() {
            let stem = file_output_path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
            let ext = file_output_path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext.is_empty() {
                output_dir.join(format!("{} ({})", stem, file_id))
            } else {
                output_dir.join(format!("{} ({}).{}", stem, file_id, ext))
            }
        } else {
            file_output_path
        };

        let mut output_file = match File::create(&file_output_path) {
            Ok(f) => f,
            Err(e) => {
                println!("  [失败] file_id={} name={}: {}", file_id, name, e);
                failed += 1;
                continue;
            }
        };

        let mut file_ok = true;
        let mut total_written = 0i64;

        for chunk_idx in 0..chunk_count {
            let key = format!("f:{}:{}", file_id, chunk_idx);
            match blob_db.get(key.as_bytes()) {
                Ok(Some(data)) => {
                    if let Err(e) = output_file.write_all(&data) {
                        println!("  [失败] file_id={} 写入失败: {}", file_id, e);
                        file_ok = false;
                        break;
                    }
                    total_written += data.len() as i64;
                }
                Ok(None) => {
                    println!("  [失败] file_id={} 分片缺失: chunk={}", file_id, chunk_idx);
                    file_ok = false;
                    break;
                }
                Err(e) => {
                    println!("  [失败] file_id={} 读取分片失败: {}", file_id, e);
                    file_ok = false;
                    break;
                }
            }
        }

        if file_ok {
            if total_written != *size_bytes {
                println!("  [警告] file_id={} name={} 大小不匹配: 预期={} 实际={}", file_id, name, size_bytes, total_written);
            }
            success += 1;
            println!("  [成功] file_id={} name={} ({})", file_id, name, format_size(*size_bytes));
        } else {
            failed += 1;
            let _ = fs::remove_file(&file_output_path);
        }
    }

    println!("\n  导出完成: 成功 {} 个, 失败 {} 个", success, failed);
    println!("  输出目录: {}\n", output_dir.display());
}

/// 全面诊断并尝试修复
fn full_diagnose_and_repair(db_path: &Path, blob_path: &Path, data_dir: &Path) {
    println!("\n--- 全面诊断并修复 ---\n");

    diagnose(db_path, blob_path);

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

    // 1. 清理孤儿缩略图记录
    print!("  清理孤儿缩略图记录... ");
    let deleted = conn.execute(
        "DELETE FROM thumbnail_index WHERE file_id NOT IN (SELECT file_id FROM files)",
        [],
    ).unwrap_or(0);
    println!("删除 {} 条", deleted);

    // 2. 清理孤儿文件记录（无folder关联的）
    print!("  清理孤儿文件记录... ");
    let deleted = conn.execute(
        "DELETE FROM files WHERE file_id NOT IN (SELECT file_id FROM file_folder)",
        [],
    ).unwrap_or(0);
    println!("删除 {} 条", deleted);

    // 3. VACUUM 压缩数据库
    print!("  压缩数据库... ");
    match conn.execute_batch("VACUUM") {
        Ok(_) => println!("完成"),
        Err(e) => println!("失败: {}", e),
    }

    // 4. 压缩 emdb
    print!("  压缩Blob存储... ");
    if blob_path.exists() {
        match Emdb::open(blob_path) {
            Ok(blob_db) => {
                match blob_db.compact() {
                    Ok(_) => println!("完成"),
                    Err(e) => println!("失败: {}", e),
                }
            }
            Err(e) => println!("打开失败: {}", e),
        }
    } else {
        println!("跳过 (blob文件不存在)");
    }

    println!("\n  修复完成。如有问题，可从备份恢复:");
    println!("  备份路径: {}\n", backup_path.display());
}

/// 压缩 Blob 存储
fn compact_blob(blob_path: &Path) {
    println!("\n--- 压缩 Blob 存储 ---\n");

    if !blob_path.exists() {
        println!("Blob文件不存在: {}", blob_path.display());
        return;
    }

    let old_size = fs::metadata(blob_path).map(|m| m.len()).unwrap_or(0);
    println!("  压缩前大小: {}", format_size(old_size as i64));

    match Emdb::open(blob_path) {
        Ok(blob_db) => {
            match blob_db.compact() {
                Ok(_) => {
                    let new_size = fs::metadata(blob_path).map(|m| m.len()).unwrap_or(0);
                    println!("  压缩后大小: {}", format_size(new_size as i64));
                    println!("  释放空间: {}", format_size((old_size - new_size) as i64));
                }
                Err(e) => println!("  压缩失败: {}", e),
            }
        }
        Err(e) => println!("  打开失败: {}", e),
    }
    println!();
}

/// 计算分片数量（与 storage.rs 中的逻辑一致）
fn get_chunk_count(file_size: i64) -> i32 {
    if file_size <= 0 {
        return 0;
    }
    let chunk_size = get_chunk_size(file_size);
    ((file_size + chunk_size - 1) / chunk_size) as i32
}

fn get_chunk_size(file_size: i64) -> i64 {
    if file_size < 100 * 1024 {
        file_size
    } else if file_size < 50 * 1024 * 1024 {
        64 * 1024
    } else {
        256 * 1024
    }
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
