use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub default_db_path: String,
    pub connections: Vec<DbConnection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbConnection {
    pub uuid: String,
    pub display_name: String,
    pub db_type: String,
    pub path: String,
    pub is_default: bool,
}

pub struct Registry {
    config: AppConfig,
    config_path: PathBuf,
}

impl Registry {
    pub fn load(config_path: &Path) -> Result<Self, String> {
        let config = if config_path.exists() {
            let content = fs::read_to_string(config_path).map_err(|e| e.to_string())?;
            serde_json::from_str(&content).unwrap_or(AppConfig {
                default_db_path: String::new(),
                connections: Vec::new(),
            })
        } else {
            AppConfig {
                default_db_path: String::new(),
                connections: Vec::new(),
            }
        };
        Ok(Registry {
            config,
            config_path: config_path.to_path_buf(),
        })
    }

    pub fn default_db_path(&self) -> &str {
        &self.config.default_db_path
    }

    pub fn connections(&self) -> &[DbConnection] {
        &self.config.connections
    }

    pub fn set_default_db_path(&mut self, path: String) {
        self.config.default_db_path = path;
    }

    pub fn add_connection(&mut self, conn: DbConnection) -> Result<(), String> {
        if self.config.connections.iter().any(|c| c.uuid == conn.uuid) {
            return Err("数据库已存在".to_string());
        }
        self.config.connections.push(conn);
        self.save()
    }

    pub fn remove_connection(&mut self, uuid: &str) -> Result<(), String> {
        if let Some(conn) = self.config.connections.iter().find(|c| c.uuid == uuid) {
            if conn.is_default {
                return Err("默认数据库不可移除".to_string());
            }
        }
        self.config.connections.retain(|c| c.uuid != uuid);
        self.save()
    }

    pub fn update_display_name(&mut self, uuid: &str, name: String) -> Result<(), String> {
        if let Some(conn) = self.config.connections.iter_mut().find(|c| c.uuid == uuid) {
            conn.display_name = name;
            self.save()
        } else {
            Err("数据库未找到".to_string())
        }
    }

    pub fn update_path(&mut self, uuid: &str, new_path: String) -> Result<(), String> {
        if let Some(conn) = self.config.connections.iter_mut().find(|c| c.uuid == uuid) {
            conn.path = new_path;
            self.save()
        } else {
            Err("数据库未找到".to_string())
        }
    }

    pub fn get_connection(&self, uuid: &str) -> Option<&DbConnection> {
        self.config.connections.iter().find(|c| c.uuid == uuid)
    }

    pub fn get_default_connection(&self) -> Option<&DbConnection> {
        self.config.connections.iter().find(|c| c.is_default)
    }

    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let content = serde_json::to_string_pretty(&self.config).map_err(|e| e.to_string())?;
        fs::write(&self.config_path, content).map_err(|e| e.to_string())
    }
}
