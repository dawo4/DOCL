//! 游戏内设置预设：把某个版本的 options.txt 存成命名模板，之后一键应用到任意版本。
//! options.txt 覆盖视野、视频、按键、音量与资源包列表；模组自己的 config 不在其中。

use crate::config;
use crate::instance;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tauri::State;

use crate::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct GameOptionsPreset {
    /// 文件名（不含 .txt），也是应用/删除时用的 id
    pub id: String,
    pub name: String,
    pub saved_at: u64,
    pub size: u64,
}

fn preset_dir() -> PathBuf {
    config::data_dir().join("game-settings")
}

/// 预设名同时做文件名：去掉 Windows 非法字符并限制长度
fn preset_id(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|ch| match ch {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect();
    let cleaned = cleaned.trim_matches([' ', '.', '_']);
    if cleaned.is_empty() {
        "游戏设置".to_string()
    } else {
        cleaned.chars().take(48).collect()
    }
}

fn preset_path(id: &str) -> PathBuf {
    preset_dir().join(format!("{}.txt", preset_id(id)))
}

fn read_info(path: &Path) -> Option<GameOptionsPreset> {
    let id = path.file_stem()?.to_string_lossy().to_string();
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let saved_at = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    Some(GameOptionsPreset {
        name: id.clone(),
        id,
        saved_at,
        size: meta.len(),
    })
}

#[tauri::command]
pub fn game_options_presets() -> Vec<GameOptionsPreset> {
    let mut list: Vec<GameOptionsPreset> = std::fs::read_dir(preset_dir())
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| read_info(&entry.path()))
                .collect()
        })
        .unwrap_or_default();
    list.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
    list
}

#[tauri::command]
pub fn save_game_options_preset(
    state: State<'_, AppState>,
    version_id: String,
    name: String,
) -> Result<GameOptionsPreset, String> {
    let cfg = state.config.lock().unwrap().clone();
    let source = instance::game_dir(&cfg, &version_id).join("options.txt");
    let bytes = std::fs::read(&source)
        .map_err(|_| "当前版本还没有 options.txt，先在游戏里启动一次并调整设置".to_string())?;
    std::fs::create_dir_all(preset_dir()).map_err(|e| format!("创建预设目录失败：{e}"))?;
    let path = preset_path(&name);
    std::fs::write(&path, bytes).map_err(|e| format!("保存预设失败：{e}"))?;
    read_info(&path).ok_or_else(|| "保存预设后读取失败".to_string())
}

#[tauri::command]
pub fn apply_game_options_preset(
    state: State<'_, AppState>,
    version_id: String,
    id: String,
) -> Result<String, String> {
    let cfg = state.config.lock().unwrap().clone();
    let source = preset_path(&id);
    let bytes = std::fs::read(&source).map_err(|_| "预设不存在或已删除".to_string())?;
    let target = instance::game_dir(&cfg, &version_id).join("options.txt");
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建版本目录失败：{e}"))?;
    }
    // 覆盖前留一份，万一模板不合适还能改回来
    if target.is_file() {
        let backup = target.with_file_name("options.txt.bak");
        std::fs::copy(&target, &backup).map_err(|e| format!("备份原设置失败：{e}"))?;
    }
    std::fs::write(&target, bytes).map_err(|e| format!("写入 options.txt 失败：{e}"))?;
    Ok(target.to_string_lossy().to_string())
}

#[tauri::command]
pub fn delete_game_options_preset(id: String) -> Result<(), String> {
    let path = preset_path(&id);
    if path.is_file() {
        std::fs::remove_file(&path).map_err(|e| format!("删除预设失败：{e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::preset_id;

    #[test]
    fn sanitizes_preset_name() {
        assert_eq!(preset_id("  低配/流畅  "), "低配_流畅");
        assert_eq!(preset_id("///"), "游戏设置");
        assert_eq!(preset_id("1.21 默认"), "1.21 默认");
    }
}
