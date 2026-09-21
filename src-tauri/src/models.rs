use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub file_id: i64,
    pub name: String,
    pub ext: Option<String>,
    pub mime_type: Option<String>,
    pub size_bytes: i64,
    pub duration_sec: Option<f64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub category: Option<String>,
    pub thumbnail_path: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderInfo {
    pub folder_id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkLocation {
    pub file_id: i64,
    pub chunk_index: i32,
    pub store_file: String,
    pub offset: i64,
    pub length: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub success_count: i32,
    pub fail_count: i32,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderContentsItem {
    pub id: i64,
    pub name: String,
    pub is_folder: bool,
    pub size_bytes: i64,
    pub category: Option<String>,
    pub ext: Option<String>,
    pub thumbnail_path: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderContents {
    pub items: Vec<FolderContentsItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreadcrumbItem {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResult {
    pub success_count: i32,
    pub fail_count: i32,
    pub errors: Vec<String>,
}

// ==================== 数据库管理模型 ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbProperties {
    pub uuid: String,
    pub display_name: String,
    pub db_type: String,
    pub password_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbConnectionInfo {
    pub uuid: String,
    pub display_name: String,
    pub db_type: String,
    pub path: String,
    pub is_connected: bool,
    pub is_default: bool,
    pub has_password: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDbRequest {
    pub path: String,
    pub password: Option<String>,
    pub display_name: Option<String>,
    pub db_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDbResult {
    pub uuid: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasswordVerifyResult {
    pub valid: bool,
    pub has_password: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityCheckResult {
    pub valid: bool,
    pub details: Vec<String>,
}
