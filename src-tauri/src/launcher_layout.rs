//! 从其他启动器导入时共用的目录布局探测。
//!
//! 各个启动器对「版本隔离」的定义不同（PCL 有显式开关，HMCL 依赖按版本建目录），
//! 但最终都落到同一件事：mods / 资源包 / 光影 / 存档放在游戏根目录还是版本目录。

use serde::Serialize;
use std::path::Path;

/// 某个版本导入后使用独立目录，还是与游戏根目录共用。
#[derive(Debug, Clone, Serialize)]
pub struct VersionIsolation {
    pub version_id: String,
    /// true 表示隔离（内容在版本目录里），false 表示与根目录共用
    pub isolated: bool,
    /// 判定依据：pcl（PCL 设置） / content（按文件位置推断）
    pub source: String,
}

/// 目录里是否有实际内容（文件或非空子目录）。
pub fn has_content(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            return true;
        }
        if path.is_dir() && has_content(&path) {
            return true;
        }
    }
    false
}

/// 内容目录：判断隔离时只看这些位置。
const KINDS: [&str; 5] = ["mods", "resourcepacks", "shaderpacks", "saves", "config"];

/// 游戏根目录里是否存在内容。一次扫描供所有版本复用，避免逐个版本重复读盘。
pub fn root_has_content(game_dir: &Path) -> bool {
    KINDS.iter().any(|name| has_content(&game_dir.join(name)))
}

/// 按内容实际位置推断是否隔离，root_has 由调用方提前算好。
/// 两边都有内容或都没有时返回 None，保持调用方当前设置不动。
pub fn detect_with_root(game_dir: &Path, version: &str, root_has: bool) -> Option<bool> {
    let version_dir = game_dir.join("versions").join(version);
    let version_has = KINDS.iter().any(|name| has_content(&version_dir.join(name)));
    match (version_has, root_has) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    }
}

/// 扫描所有带清单的版本，只按内容位置判断隔离状态。
pub fn content_isolation(game_dir: &Path) -> Vec<VersionIsolation> {
    let mut list = Vec::new();
    let root_has = root_has_content(game_dir);
    for id in version_ids(game_dir) {
        let Some(isolated) = detect_with_root(game_dir, &id, root_has) else {
            continue;
        };
        list.push(VersionIsolation {
            version_id: id,
            isolated,
            source: "content".into(),
        });
    }
    list
}

/// 清单齐全（有 `<id>.json`）的版本 id。
pub fn version_ids(game_dir: &Path) -> Vec<String> {
    let mut list = Vec::new();
    let Ok(entries) = std::fs::read_dir(game_dir.join("versions")) else {
        return list;
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        if entry.path().join(format!("{id}.json")).is_file() {
            list.push(id);
        }
    }
    list.sort();
    list
}

/// 清单齐全的版本数。
pub fn count_versions(game_dir: &Path) -> usize {
    version_ids(game_dir).len()
}
