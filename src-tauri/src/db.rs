use rusqlite::{Connection, Result, params};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::models::*;

#[derive(Clone)]
pub struct Database {
    pub conn: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn new(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        let db = Database {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.init_tables()?;
        Ok(db)
    }

    fn init_tables(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS files (
                file_id       INTEGER PRIMARY KEY AUTOINCREMENT,
                name          TEXT NOT NULL,
                ext           TEXT,
                mime_type     TEXT,
                size_bytes    INTEGER,
                duration_sec  REAL,
                width         INTEGER,
                height        INTEGER,
                category      TEXT,
                created_at    INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                updated_at    INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

            CREATE TABLE IF NOT EXISTS chunk_locations (
                file_id     INTEGER NOT NULL,
                chunk_index INTEGER NOT NULL,
                store_file  TEXT NOT NULL,
                offset      INTEGER NOT NULL,
                length      INTEGER NOT NULL,
                PRIMARY KEY (file_id, chunk_index)
            );

            CREATE TABLE IF NOT EXISTS store_files (
                store_file   TEXT PRIMARY KEY,
                used_bytes   INTEGER DEFAULT 0,
                created_at   INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

            CREATE TABLE IF NOT EXISTS folders (
                folder_id  INTEGER PRIMARY KEY AUTOINCREMENT,
                name       TEXT NOT NULL,
                parent_id  INTEGER,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                FOREIGN KEY (parent_id) REFERENCES folders(folder_id)
            );

            CREATE TABLE IF NOT EXISTS file_folder (
                file_id   INTEGER NOT NULL,
                folder_id INTEGER NOT NULL,
                PRIMARY KEY (file_id, folder_id)
            );

            CREATE TABLE IF NOT EXISTS thumbnail_index (
                file_id      INTEGER PRIMARY KEY,
                thumb_path   TEXT,
                generated_at INTEGER
            );

            CREATE TABLE IF NOT EXISTS free_space (
                space_id     INTEGER PRIMARY KEY AUTOINCREMENT,
                store_file   TEXT NOT NULL,
                offset       INTEGER NOT NULL,
                length       INTEGER NOT NULL,
                created_at   INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

            CREATE INDEX IF NOT EXISTS idx_free_space_length
                ON free_space(length);

            CREATE INDEX IF NOT EXISTS idx_chunk_locations_file_id
                ON chunk_locations(file_id);
            CREATE INDEX IF NOT EXISTS idx_file_folder_folder_id
                ON file_folder(folder_id);
            CREATE INDEX IF NOT EXISTS idx_files_category
                ON files(category);

            CREATE TABLE IF NOT EXISTS db_properties (
                key   TEXT PRIMARY KEY,
                value TEXT
            );
            "
        )?;

        // 确保 db_properties 有默认值
        self.ensure_db_property(&conn, "db_type", "local")?;
        self.ensure_db_property(&conn, "display_name", "")?;

        // Create root folder if not exists
        let root_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM folders WHERE parent_id IS NULL",
            [],
            |row| row.get(0),
        )?;
        if root_count == 0 {
            conn.execute(
                "INSERT INTO folders (name, parent_id) VALUES ('根目录', NULL)",
                [],
            )?;
        }

        Ok(())
    }

    pub fn get_files_in_folder(&self, folder_id: i64) -> Result<Vec<FileInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT f.file_id, f.name, f.ext, f.mime_type, f.size_bytes,
                    f.duration_sec, f.width, f.height, f.category,
                    t.thumb_path,
                    f.created_at, f.updated_at
             FROM files f
             JOIN file_folder ff ON f.file_id = ff.file_id
             LEFT JOIN thumbnail_index t ON f.file_id = t.file_id
             WHERE ff.folder_id = ?
             ORDER BY f.name"
        )?;
        let files = stmt.query_map(params![folder_id], |row| {
            Ok(FileInfo {
                file_id: row.get(0)?,
                name: row.get(1)?,
                ext: row.get(2)?,
                mime_type: row.get(3)?,
                size_bytes: row.get(4)?,
                duration_sec: row.get(5)?,
                width: row.get(6)?,
                height: row.get(7)?,
                category: row.get(8)?,
                thumbnail_path: row.get(9)?,
                created_at: row.get(10)?,
                updated_at: row.get(11)?,
            })
        })?.collect::<Result<Vec<_>>>()?;
        Ok(files)
    }

    pub fn get_folders(&self) -> Result<Vec<FolderInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT folder_id, name, parent_id, created_at FROM folders ORDER BY name"
        )?;
        let folders = stmt.query_map([], |row| {
            Ok(FolderInfo {
                folder_id: row.get(0)?,
                name: row.get(1)?,
                parent_id: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?.collect::<Result<Vec<_>>>()?;
        Ok(folders)
    }

    pub fn create_folder(&self, name: &str, parent_id: Option<i64>) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO folders (name, parent_id) VALUES (?, ?)",
            params![name, parent_id],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// 原子插入文件记录 + 分片位置 + 更新store_files用量 + 消耗free_space
    /// 所有DB写操作在同一事务中，保证数据一致性
    pub fn insert_file_with_chunks(
        &self,
        file: &FileInfo,
        folder_id: i64,
        chunks: &[(String, i64, i64, i64)], // (store_file, offset, length, space_id or 0)
        store_file_updates: &[(String, i64)], // (store_file, additional_bytes) for new appends
    ) -> Result<i64> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        tx.execute(
            "INSERT INTO files (name, ext, mime_type, size_bytes, duration_sec,
                                width, height, category, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                file.name, file.ext, file.mime_type, file.size_bytes,
                file.duration_sec, file.width, file.height, file.category,
                file.created_at, file.updated_at
            ],
        )?;
        let file_id = tx.last_insert_rowid();

        tx.execute(
            "INSERT INTO file_folder (file_id, folder_id) VALUES (?, ?)",
            params![file_id, folder_id],
        )?;

        for (i, (store_file, offset, length, _space_id)) in chunks.iter().enumerate() {
            tx.execute(
                "INSERT INTO chunk_locations (file_id, chunk_index, store_file, offset, length)
                 VALUES (?, ?, ?, ?, ?)",
                params![file_id, i as i32, store_file, offset, length],
            )?;
        }

        // 消耗free_space（从查询阶段收集的space_id）
        for (_, _, _, space_id) in chunks.iter() {
            if *space_id > 0 {
                tx.execute("DELETE FROM free_space WHERE space_id = ?", params![space_id])?;
            }
        }

        // 新追加的store_file更新used_bytes
        for (store_file, additional) in store_file_updates.iter() {
            tx.execute(
                "UPDATE store_files SET used_bytes = used_bytes + ? WHERE store_file = ?",
                params![additional, store_file],
            )?;
        }

        tx.commit()?;
        Ok(file_id)
    }

    pub fn get_chunk_locations(&self, file_id: i64) -> Result<Vec<ChunkLocation>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT file_id, chunk_index, store_file, offset, length
             FROM chunk_locations WHERE file_id = ? ORDER BY chunk_index"
        )?;
        let chunks = stmt.query_map(params![file_id], |row| {
            Ok(ChunkLocation {
                file_id: row.get(0)?,
                chunk_index: row.get(1)?,
                store_file: row.get(2)?,
                offset: row.get(3)?,
                length: row.get(4)?,
            })
        })?.collect::<Result<Vec<_>>>()?;
        Ok(chunks)
    }

    /// 只读查询：查找可用store_file（不创建新记录）
    pub fn find_available_store_file(&self, max_size: i64) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let result: Option<String> = conn.query_row(
            "SELECT store_file FROM store_files WHERE used_bytes < ? ORDER BY store_file LIMIT 1",
            params![max_size],
            |row| row.get(0),
        ).ok();
        Ok(result)
    }

    /// 只读查询：获取下一个store_file名称
    pub fn get_next_store_file_name(&self) -> Result<String> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM store_files", [], |row| row.get(0),
        )?;
        Ok(format!("store_{:03}.bin", count + 1))
    }

    pub fn delete_file(&self, file_id: i64) -> Result<Vec<ChunkLocation>> {
        let mut conn = self.conn.lock().unwrap();
        // Query chunks inline to avoid re-locking
        let mut stmt = conn.prepare(
            "SELECT file_id, chunk_index, store_file, offset, length
             FROM chunk_locations WHERE file_id = ? ORDER BY chunk_index"
        )?;
        let chunks = stmt.query_map(params![file_id], |row| {
            Ok(ChunkLocation {
                file_id: row.get(0)?,
                chunk_index: row.get(1)?,
                store_file: row.get(2)?,
                offset: row.get(3)?,
                length: row.get(4)?,
            })
        })?.collect::<Result<Vec<_>>>()?;
        drop(stmt);
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM chunk_locations WHERE file_id = ?", params![file_id])?;
        tx.execute("DELETE FROM thumbnail_index WHERE file_id = ?", params![file_id])?;
        tx.execute("DELETE FROM file_folder WHERE file_id = ?", params![file_id])?;
        tx.execute("DELETE FROM files WHERE file_id = ?", params![file_id])?;
        // 原子记录free_space，避免崩溃导致空间泄漏
        for chunk in &chunks {
            tx.execute(
                "INSERT INTO free_space (store_file, offset, length) VALUES (?, ?, ?)",
                params![chunk.store_file, chunk.offset, chunk.length],
            )?;
        }
        tx.commit()?;
        Ok(chunks)
    }

    pub fn delete_folder(&self, folder_id: i64) -> Result<Vec<ChunkLocation>> {
        let mut conn = self.conn.lock().unwrap();
        
        // 先查询所有需要释放的分片位置
        let mut stmt = conn.prepare(
            "SELECT cl.file_id, cl.chunk_index, cl.store_file, cl.offset, cl.length
             FROM chunk_locations cl
             JOIN file_folder ff ON cl.file_id = ff.file_id
             WHERE ff.folder_id IN (
                 WITH RECURSIVE subfolders AS (
                     SELECT folder_id FROM folders WHERE folder_id = ?
                     UNION ALL
                     SELECT f.folder_id FROM folders f
                     JOIN subfolders s ON f.parent_id = s.folder_id
                 )
                 SELECT folder_id FROM subfolders
             )
             ORDER BY cl.file_id, cl.chunk_index"
        )?;
        let all_chunks = stmt.query_map(params![folder_id], |row| {
            Ok(ChunkLocation {
                file_id: row.get(0)?,
                chunk_index: row.get(1)?,
                store_file: row.get(2)?,
                offset: row.get(3)?,
                length: row.get(4)?,
            })
        })?.collect::<Result<Vec<_>>>()?;
        drop(stmt);
        
        let tx = conn.transaction()?;
        
        // 1. 删除所有子文件夹中文件的缩略图索引
        tx.execute(
            "DELETE FROM thumbnail_index WHERE file_id IN (
                SELECT file_id FROM file_folder WHERE folder_id IN (
                    WITH RECURSIVE subfolders AS (
                        SELECT folder_id FROM folders WHERE folder_id = ?
                        UNION ALL
                        SELECT f.folder_id FROM folders f
                        JOIN subfolders s ON f.parent_id = s.folder_id
                    )
                    SELECT folder_id FROM subfolders
                )
            )",
            params![folder_id],
        )?;
        
        // 2. 删除所有子文件夹中文件的chunk位置记录
        tx.execute(
            "DELETE FROM chunk_locations WHERE file_id IN (
                SELECT file_id FROM file_folder WHERE folder_id IN (
                    WITH RECURSIVE subfolders AS (
                        SELECT folder_id FROM folders WHERE folder_id = ?
                        UNION ALL
                        SELECT f.folder_id FROM folders f
                        JOIN subfolders s ON f.parent_id = s.folder_id
                    )
                    SELECT folder_id FROM subfolders
                )
            )",
            params![folder_id],
        )?;
        
        // 3. 删除所有子文件夹中的文件记录
        tx.execute(
            "DELETE FROM files WHERE file_id IN (
                SELECT file_id FROM file_folder WHERE folder_id IN (
                    WITH RECURSIVE subfolders AS (
                        SELECT folder_id FROM folders WHERE folder_id = ?
                        UNION ALL
                        SELECT f.folder_id FROM folders f
                        JOIN subfolders s ON f.parent_id = s.folder_id
                    )
                    SELECT folder_id FROM subfolders
                )
            )",
            params![folder_id],
        )?;
        
        // 4. 删除所有子文件夹的文件关联
        tx.execute(
            "DELETE FROM file_folder WHERE folder_id IN (
                WITH RECURSIVE subfolders AS (
                    SELECT folder_id FROM folders WHERE folder_id = ?
                    UNION ALL
                    SELECT f.folder_id FROM folders f
                    JOIN subfolders s ON f.parent_id = s.folder_id
                )
                SELECT folder_id FROM subfolders
            )",
            params![folder_id],
        )?;
        
        // 5. 删除所有子文件夹
        tx.execute(
            "DELETE FROM folders WHERE folder_id IN (
                WITH RECURSIVE subfolders AS (
                    SELECT folder_id FROM folders WHERE folder_id = ?
                    UNION ALL
                    SELECT f.folder_id FROM folders f
                    JOIN subfolders s ON f.parent_id = s.folder_id
                )
                SELECT folder_id FROM subfolders
            )",
            params![folder_id],
        )?;
        
        // 6. 原子记录free_space，避免崩溃导致空间泄漏
        for chunk in &all_chunks {
            tx.execute(
                "INSERT INTO free_space (store_file, offset, length) VALUES (?, ?, ?)",
                params![chunk.store_file, chunk.offset, chunk.length],
            )?;
        }
        
        tx.commit()?;
        Ok(all_chunks)
    }

    pub fn get_all_file_ids_in_folder_recursive(&self, folder_id: i64) -> Result<Vec<i64>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT file_id FROM file_folder WHERE folder_id IN (
                WITH RECURSIVE subfolders AS (
                    SELECT folder_id FROM folders WHERE folder_id = ?
                    UNION ALL
                    SELECT f.folder_id FROM folders f
                    JOIN subfolders s ON f.parent_id = s.folder_id
                )
                SELECT folder_id FROM subfolders
            )"
        )?;
        let ids = stmt.query_map(params![folder_id], |row| row.get(0))?
            .collect::<Result<Vec<i64>, _>>()?;
        Ok(ids)
    }

    pub fn rename_file(&self, file_id: i64, new_name: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let ext = Path::new(new_name)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_string());
        conn.execute(
            "UPDATE files SET name = ?, ext = ?, updated_at = strftime('%s','now') WHERE file_id = ?",
            params![new_name, ext, file_id],
        )?;
        Ok(())
    }

    pub fn move_file(&self, file_id: i64, target_folder_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE file_folder SET folder_id = ? WHERE file_id = ?",
            params![target_folder_id, file_id],
        )?;
        Ok(())
    }

    pub fn get_thumbnail_path(&self, file_id: i64) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT thumb_path FROM thumbnail_index WHERE file_id = ?",
            params![file_id],
            |row| row.get(0),
        );
        match result {
            Ok(path) => Ok(Some(path)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn set_thumbnail_path(&self, file_id: i64, thumb_path: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO thumbnail_index (file_id, thumb_path, generated_at)
             VALUES (?, ?, strftime('%s','now'))",
            params![file_id, thumb_path],
        )?;
        Ok(())
    }

    pub fn get_file_info(&self, file_id: i64) -> Result<Option<FileInfo>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT f.file_id, f.name, f.ext, f.mime_type, f.size_bytes,
                    f.duration_sec, f.width, f.height, f.category,
                    t.thumb_path,
                    f.created_at, f.updated_at
             FROM files f
             LEFT JOIN thumbnail_index t ON f.file_id = t.file_id
             WHERE f.file_id = ?",
            params![file_id],
            |row| {
                Ok(FileInfo {
                    file_id: row.get(0)?,
                    name: row.get(1)?,
                    ext: row.get(2)?,
                    mime_type: row.get(3)?,
                    size_bytes: row.get(4)?,
                    duration_sec: row.get(5)?,
                    width: row.get(6)?,
                    height: row.get(7)?,
                    category: row.get(8)?,
                    thumbnail_path: row.get(9)?,
                    created_at: row.get(10)?,
                    updated_at: row.get(11)?,
                })
            }
        );
        match result {
            Ok(info) => Ok(Some(info)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// 检查同一文件夹下是否已存在同名文件（排除指定file_id）
    pub fn check_file_name_in_folder(&self, folder_id: i64, name: &str, exclude_file_id: Option<i64>) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let mut sql = String::from(
            "SELECT COUNT(*) FROM files f
             JOIN file_folder ff ON f.file_id = ff.file_id
             WHERE ff.folder_id = ? AND f.name = ?"
        );
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
            Box::new(folder_id),
            Box::new(name.to_string()),
        ];
        if let Some(eid) = exclude_file_id {
            sql.push_str(" AND f.file_id != ?");
            params_vec.push(Box::new(eid));
        }
        let params_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
        let count: i64 = conn.query_row(&sql, params_refs.as_slice(), |row| row.get(0))?;
        Ok(count > 0)
    }

    /// 检查同一父文件夹下是否已存在同名子文件夹（排除指定folder_id）
    pub fn check_folder_name_in_parent(&self, parent_id: Option<i64>, name: &str, exclude_folder_id: Option<i64>) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let mut sql = String::from("SELECT COUNT(*) FROM folders WHERE ");
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(name.to_string())];
        match parent_id {
            Some(pid) => {
                sql.push_str("parent_id = ? AND name = ?");
                params_vec.insert(0, Box::new(pid));
            }
            None => {
                sql.push_str("parent_id IS NULL AND name = ?");
            }
        }
        if let Some(eid) = exclude_folder_id {
            sql.push_str(" AND folder_id != ?");
            params_vec.push(Box::new(eid));
        }
        let params_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
        let count: i64 = conn.query_row(&sql, params_refs.as_slice(), |row| row.get(0))?;
        Ok(count > 0)
    }

    /// 重命名文件夹
    pub fn rename_folder(&self, folder_id: i64, new_name: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE folders SET name = ? WHERE folder_id = ?",
            params![new_name, folder_id],
        )?;
        Ok(())
    }

    /// 移动文件夹到另一个父文件夹
    pub fn move_folder(&self, folder_id: i64, target_parent_id: Option<i64>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE folders SET parent_id = ? WHERE folder_id = ?",
            params![target_parent_id, folder_id],
        )?;
        Ok(())
    }

    /// 获取文件所在的文件夹ID
    pub fn get_file_folder_id(&self, file_id: i64) -> Result<Option<i64>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT folder_id FROM file_folder WHERE file_id = ?",
            params![file_id],
            |row| row.get(0),
        );
        match result {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn search_files(&self, keyword: &str) -> Result<Vec<FileInfo>> {
        let conn = self.conn.lock().unwrap();
        let pattern = format!("%{}%", keyword);
        let mut stmt = conn.prepare(
            "SELECT f.file_id, f.name, f.ext, f.mime_type, f.size_bytes,
                    f.duration_sec, f.width, f.height, f.category,
                    t.thumb_path,
                    f.created_at, f.updated_at
             FROM files f
             LEFT JOIN thumbnail_index t ON f.file_id = t.file_id
             WHERE f.name LIKE ? ORDER BY f.name LIMIT 50"
        )?;
        let files = stmt.query_map(params![pattern], |row| {
            Ok(FileInfo {
                file_id: row.get(0)?,
                name: row.get(1)?,
                ext: row.get(2)?,
                mime_type: row.get(3)?,
                size_bytes: row.get(4)?,
                duration_sec: row.get(5)?,
                width: row.get(6)?,
                height: row.get(7)?,
                category: row.get(8)?,
                thumbnail_path: row.get(9)?,
                created_at: row.get(10)?,
                updated_at: row.get(11)?,
            })
        })?.collect::<Result<Vec<_>>>()?;
        Ok(files)
    }

    /// 只读查询：查找可用free_space（不修改DB）
    /// 返回 (space_id, store_file, offset)
    /// space_id=0 表示无可用空间
    pub fn find_free_space(&self, needed: i64) -> Result<Option<(i64, String, i64)>> {
        let conn = self.conn.lock().unwrap();
        let result: Option<(i64, String, i64)> = conn.query_row(
            "SELECT space_id, store_file, offset FROM free_space
             WHERE length >= ? ORDER BY length ASC LIMIT 1",
            params![needed],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).ok();
        Ok(result)
    }

    // ==================== 数据库属性管理 ====================

    fn ensure_db_property(&self, conn: &Connection, key: &str, default_value: &str) -> Result<()> {
        let exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM db_properties WHERE key = ?",
            params![key],
            |row| row.get::<_, i64>(0),
        ).map(|c| c > 0)?;
        if !exists {
            conn.execute(
                "INSERT INTO db_properties (key, value) VALUES (?, ?)",
                params![key, default_value],
            )?;
        }
        Ok(())
    }

    pub fn init_db_properties(&self, uuid: &str, display_name: &str, db_type: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO db_properties (key, value) VALUES ('uuid', ?)",
            params![uuid],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO db_properties (key, value) VALUES ('display_name', ?)",
            params![display_name],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO db_properties (key, value) VALUES ('db_type', ?)",
            params![db_type],
        )?;
        Ok(())
    }

    pub fn get_db_properties(&self) -> Result<DbProperties> {
        let conn = self.conn.lock().unwrap();
        let get_prop = |key: &str| -> Result<Option<String>> {
            let result = conn.query_row(
                "SELECT value FROM db_properties WHERE key = ?",
                params![key],
                |row| row.get(0),
            );
            match result {
                Ok(v) => Ok(Some(v)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        };
        Ok(DbProperties {
            uuid: get_prop("uuid")?.unwrap_or_default(),
            display_name: get_prop("display_name")?.unwrap_or_default(),
            db_type: get_prop("db_type")?.unwrap_or_else(|| "local".to_string()),
            password_hash: get_prop("password_hash")?,
        })
    }

    pub fn set_password_hash(&self, hash: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        match hash {
            Some(h) => conn.execute(
                "INSERT OR REPLACE INTO db_properties (key, value) VALUES ('password_hash', ?)",
                params![h],
            )?,
            None => conn.execute(
                "DELETE FROM db_properties WHERE key = 'password_hash'",
                [],
            )?,
        };
        Ok(())
    }

    pub fn set_display_name(&self, name: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO db_properties (key, value) VALUES ('display_name', ?)",
            params![name],
        )?;
        Ok(())
    }

    pub fn check_integrity(&self) -> Result<(bool, Vec<String>)> {
        let conn = self.conn.lock().unwrap();
        let mut details = Vec::new();

        let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            details.push(format!("SQLite完整性检查失败: {}", integrity));
            return Ok((false, details));
        }

        let required_tables = ["files", "folders", "file_folder", "chunk_locations", "store_files", "thumbnail_index", "free_space", "db_properties"];
        for table in &required_tables {
            let exists: bool = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
                params![table],
                |row| row.get::<_, i64>(0),
            ).map(|c| c > 0)?;
            if !exists {
                details.push(format!("缺少表: {}", table));
            }
        }

        let orphan_chunks: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chunk_locations WHERE file_id NOT IN (SELECT file_id FROM files)",
            [], |row| row.get(0),
        )?;
        if orphan_chunks > 0 {
            details.push(format!("发现 {} 条孤儿分片记录", orphan_chunks));
        }

        Ok((details.is_empty(), details))
    }
}
