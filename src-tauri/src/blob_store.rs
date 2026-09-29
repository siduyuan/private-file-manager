use emdb::Emdb;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// emdb 封装层：替代原有的 store.bin + free_space + chunk_locations
/// 每个数据库对应一个 emdb 文件，分片以 KV 形式存储
/// Key 格式: "f:{file_id}:{chunk_index}"
#[derive(Clone)]
pub struct BlobStore {
    db: Arc<Emdb>,
    path: PathBuf,
}

impl BlobStore {
    /// 打开或创建 emdb 文件
    pub fn open(emdb_path: &Path) -> Result<Self, String> {
        let db = Emdb::open(emdb_path).map_err(|e| format!("打开 blob store 失败: {}", e))?;
        Ok(BlobStore {
            db: Arc::new(db),
            path: emdb_path.to_path_buf(),
        })
    }

    /// 生成分片 key
    fn chunk_key(file_id: i64, chunk_index: i32) -> String {
        format!("f:{}:{}", file_id, chunk_index)
    }

    /// 写入单个分片
    pub fn write_chunk(&self, file_id: i64, chunk_index: i32, data: &[u8]) -> Result<(), String> {
        let key = Self::chunk_key(file_id, chunk_index);
        self.db.insert(key.as_bytes(), data)
            .map_err(|e| format!("写入分片失败: {}", e))
    }

    /// 读取单个分片
    pub fn read_chunk(&self, file_id: i64, chunk_index: i32) -> Result<Vec<u8>, String> {
        let key = Self::chunk_key(file_id, chunk_index);
        self.db.get(key.as_bytes())
            .map_err(|e| format!("读取分片失败: {}", e))?
            .ok_or_else(|| format!("分片不存在: file_id={}, chunk={}", file_id, chunk_index))
    }

    /// 删除文件的所有分片
    pub fn delete_file_chunks(&self, file_id: i64, chunk_count: i32) -> Result<(), String> {
        for i in 0..chunk_count {
            let key = Self::chunk_key(file_id, i);
            // 忽略不存在的 key（可能部分写入失败）
            let _ = self.db.remove(key.as_bytes());
        }
        Ok(())
    }

    /// 批量删除多个文件的所有分片
    pub fn delete_files_chunks(&self, file_ids: &[i64], chunk_counts: &[(i64, i32)]) -> Result<(), String> {
        // 构建 file_id -> chunk_count 映射
        let mut count_map = std::collections::HashMap::new();
        for (fid, count) in chunk_counts {
            count_map.insert(*fid, *count);
        }
        for file_id in file_ids {
            if let Some(&chunk_count) = count_map.get(file_id) {
                self.delete_file_chunks(*file_id, chunk_count)?;
            }
        }
        Ok(())
    }

    /// 获取存储统计信息
    /// 返回 (记录数, emdb 文件大小)
    pub fn stats(&self) -> Result<(u64, u64), String> {
        let stats = self.db.stats().map_err(|e| format!("获取统计失败: {}", e))?;
        Ok((stats.live_records, stats.file_size_bytes))
    }

    /// 执行压缩：重写有效记录，释放已删除数据占用的空间
    pub fn compact(&self) -> Result<(), String> {
        self.db.compact().map_err(|e| format!("压缩失败: {}", e))?;
        self.db.checkpoint().map_err(|e| format!("checkpoint 失败: {}", e))?;
        Ok(())
    }

    /// 持久化到磁盘
    pub fn flush(&self) -> Result<(), String> {
        self.db.flush().map_err(|e| format!("flush 失败: {}", e))
    }

    /// 获取 emdb 文件路径
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 获取 emdb 文件大小（字节）
    pub fn file_size(&self) -> u64 {
        std::fs::metadata(&self.path)
            .map(|m| m.len())
            .unwrap_or(0)
    }
}
