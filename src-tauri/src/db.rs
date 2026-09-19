use rusqlite::{Connection, Result, params};
use std::path::Path;
use std::sync::Mutex;

use crate::models::*;

pub struct Database {
    pub conn: Mutex<Connection>,
}

impl Database {
    pub fn new(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        let db = Database {
            conn: Mutex::new(conn),
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

            CREATE INDEX IF NOT EXISTS idx_chunk_locations_file_id
                ON chunk_locations(file_id);
            CREATE INDEX IF NOT EXISTS idx_file_folder_folder_id
                ON file_folder(folder_id);
            CREATE INDEX IF NOT EXISTS idx_files_category
                ON files(category);
            "
        )?;

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
                    f.created_at, f.updated_at
             FROM files f
             JOIN file_folder ff ON f.file_id = ff.file_id
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
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
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

    pub fn insert_file(&self, file: &FileInfo, folder_id: i64) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO files (name, ext, mime_type, size_bytes, duration_sec,
                                width, height, category, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                file.name, file.ext, file.mime_type, file.size_bytes,
                file.duration_sec, file.width, file.height, file.category,
                file.created_at, file.updated_at
            ],
        )?;
        let file_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO file_folder (file_id, folder_id) VALUES (?, ?)",
            params![file_id, folder_id],
        )?;
        Ok(file_id)
    }

    pub fn insert_chunk_locations(&self, chunks: &[ChunkLocation]) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        for chunk in chunks {
            tx.execute(
                "INSERT INTO chunk_locations (file_id, chunk_index, store_file, offset, length)
                 VALUES (?, ?, ?, ?, ?)",
                params![chunk.file_id, chunk.chunk_index, chunk.store_file, chunk.offset, chunk.length],
            )?;
        }
        tx.commit()?;
        Ok(())
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

    pub fn get_or_create_store_file(&self, max_size: i64) -> Result<String> {
        let conn = self.conn.lock().unwrap();
        let existing: Option<String> = conn.query_row(
            "SELECT store_file FROM store_files WHERE used_bytes < ? ORDER BY store_file LIMIT 1",
            params![max_size],
            |row| row.get(0),
        ).ok();

        if let Some(sf) = existing {
            Ok(sf)
        } else {
            // Get next store file number
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM store_files", [], |row| row.get(0),
            )?;
            let store_file = format!("store_{:03}.bin", count + 1);
            conn.execute(
                "INSERT INTO store_files (store_file, used_bytes) VALUES (?, 0)",
                params![&store_file],
            )?;
            Ok(store_file)
        }
    }

    pub fn update_store_file_used(&self, store_file: &str, additional_bytes: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE store_files SET used_bytes = used_bytes + ? WHERE store_file = ?",
            params![additional_bytes, store_file],
        )?;
        Ok(())
    }

    pub fn delete_file(&self, file_id: i64) -> Result<Vec<ChunkLocation>> {
        let mut conn = self.conn.lock().unwrap();
        let chunks = self.get_chunk_locations(file_id)?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM chunk_locations WHERE file_id = ?", params![file_id])?;
        tx.execute("DELETE FROM thumbnail_index WHERE file_id = ?", params![file_id])?;
        tx.execute("DELETE FROM file_folder WHERE file_id = ?", params![file_id])?;
        tx.execute("DELETE FROM files WHERE file_id = ?", params![file_id])?;
        tx.commit()?;
        Ok(chunks)
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
            "SELECT file_id, name, ext, mime_type, size_bytes,
                    duration_sec, width, height, category,
                    created_at, updated_at
             FROM files WHERE file_id = ?",
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
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            }
        );
        match result {
            Ok(info) => Ok(Some(info)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn search_files(&self, keyword: &str) -> Result<Vec<FileInfo>> {
        let conn = self.conn.lock().unwrap();
        let pattern = format!("%{}%", keyword);
        let mut stmt = conn.prepare(
            "SELECT file_id, name, ext, mime_type, size_bytes,
                    duration_sec, width, height, category,
                    created_at, updated_at
             FROM files WHERE name LIKE ? ORDER BY name LIMIT 50"
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
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })?.collect::<Result<Vec<_>>>()?;
        Ok(files)
    }
}
