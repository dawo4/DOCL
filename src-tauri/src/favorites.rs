//! 社区资源收藏：把 CurseForge / Modrinth 项目存进本地 JSON。
//! 只存展示和跳转需要的字段，打开详情时仍按项目 id 去源站拉文件列表。

use crate::config;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Favorite {
    pub id: String,
    /// curseforge / modrinth
    pub source: String,
    /// 资源分类：mods / modpacks / resourcepacks 等
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub website_url: String,
    #[serde(default)]
    pub logo_url: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub download_count: u64,
    #[serde(default)]
    pub class_id: u32,
    #[serde(default)]
    pub updated: String,
    #[serde(default)]
    pub categories: Vec<String>,
    /// 收藏时间（Unix 秒），列表按它倒序
    #[serde(default)]
    pub added_at: u64,
}

fn store_path() -> PathBuf {
    config::data_dir().join("favorites.json")
}

fn load() -> Vec<Favorite> {
    let mut list: Vec<Favorite> = std::fs::read(store_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    list.sort_by(|a, b| b.added_at.cmp(&a.added_at));
    list
}

fn save(list: &[Favorite]) -> Result<(), String> {
    let path = store_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn favorites_list() -> Vec<Favorite> {
    load()
}

/// 切换收藏状态，返回切换后是否为已收藏。
#[tauri::command]
pub fn favorites_toggle(mut favorite: Favorite) -> Result<bool, String> {
    let mut list = load();
    if let Some(index) = list
        .iter()
        .position(|item| item.id == favorite.id && item.source == favorite.source)
    {
        list.remove(index);
        save(&list)?;
        return Ok(false);
    }
    favorite.added_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    list.insert(0, favorite);
    save(&list)?;
    Ok(true)
}
