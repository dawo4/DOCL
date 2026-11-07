use crate::config::Config;
use crate::{install, java};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
use winreg::RegKey;

/// 从 PCL 读到的可迁移项；未找到的字段为 None，不会覆盖现有设置。
#[derive(Debug, Default, Serialize)]
pub struct PclPreview {
    pub root: String,
    pub game_dir: Option<String>,
    pub java_path: Option<String>,
    pub memory_auto: Option<bool>,
    pub memory_mb: Option<u32>,
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub update_mode: Option<String>,
    pub download_source: Option<String>,
    /// PCL 上次启动的版本（对应 .minecraft\versions 下的目录名）
    pub selected_version: Option<String>,
    pub findings: Vec<String>,
}

fn normalize_root(raw: &str) -> Result<PathBuf> {
    let path = PathBuf::from(raw.trim().trim_matches('"'));
    if !path.exists() {
        bail!("路径不存在：{}", path.display());
    }
    if path.is_file() {
        return path
            .parent()
            .map(Path::to_path_buf)
            .context("无法定位 PCL 所在目录");
    }
    Ok(path)
}

/// PCL 的 Setup.ini 是 `键:值`，个别版本写成 `键=值`，两种都接受。
fn parse_ini(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('[') || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(index) = line.find([':', '=']) {
            let key = line[..index].trim();
            let value = line[index + 1..].trim();
            if !key.is_empty() {
                map.insert(key.to_string(), value.to_string());
            }
        }
    }
    map
}

fn reg_value(key: &RegKey, name: &str) -> Option<String> {
    if let Ok(value) = key.get_value::<String, _>(name) {
        return Some(value);
    }
    if let Ok(value) = key.get_value::<u32, _>(name) {
        return Some(value.to_string());
    }
    if let Ok(value) = key.get_value::<u64, _>(name) {
        return Some(value.to_string());
    }
    None
}

const WANTED_KEYS: [&str; 10] = [
    "LaunchRamType",
    "LaunchRamCustom",
    "SetupJavaPath",
    "SystemSystemUpdate",
    "ToolDownloadSource",
    "LaunchFolderSelect",
    "LaunchFolders",
    "LaunchArgumentWindowWidth",
    "LaunchArgumentWindowHeight",
    "LaunchArgumentIndieV2",
];

/// 注册表是 PCL 的主存储，便携版 Setup.ini 里的同名项优先。
fn read_settings(root: &Path) -> HashMap<String, String> {
    let mut settings = HashMap::new();
    if let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags("Software\\PCL", KEY_READ) {
        for name in WANTED_KEYS {
            if let Some(value) = reg_value(&key, name) {
                settings.insert(name.to_string(), value);
            }
        }
    }
    for candidate in [root.join("PCL").join("Setup.ini"), root.join("Setup.ini")] {
        if let Ok(text) = std::fs::read_to_string(&candidate) {
            for (key, value) in parse_ini(&text) {
                settings.insert(key, value);
            }
        }
    }
    settings
}

/// 检查某个键是否来自当前 PCL 目录的 Setup.ini，而不是全局注册表。
fn ini_has_setting(root: &Path, key: &str) -> bool {
    [root.join("PCL").join("Setup.ini"), root.join("Setup.ini")]
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .any(|text| parse_ini(&text).contains_key(key))
}

/// `$.minecraft\` 里的 `$` 表示 PCL 目录。
fn resolve_game_dir(root: &Path, settings: &HashMap<String, String>) -> Option<PathBuf> {
    // 优先级：当前 PCL 目录里明确配置的文件夹 > 当前目录自带的 .minecraft > 全局注册表里的其他文件夹
    let mut candidates: Vec<(u8, PathBuf)> = Vec::new();
    let mut push_raw = |priority: u8, raw: &str| {
        let raw = raw.trim().trim_matches('"');
        if raw.is_empty() {
            return;
        }
        let path = match raw.strip_prefix('$') {
            Some(rest) => root.join(rest.trim_start_matches(['\\', '/'])),
            None => PathBuf::from(raw),
        };
        if !path.exists() {
            return;
        }
        // PCL 的“游戏文件夹”既可能直接是 .minecraft，也可能是包含 .minecraft 的外层目录
        let dir = if path.join("versions").is_dir() {
            path
        } else if path.join(".minecraft").join("versions").is_dir() {
            path.join(".minecraft")
        } else if path.is_dir() {
            path
        } else {
            return;
        };
        candidates.push((priority, dir));
    };

    if let Some(raw) = settings.get("LaunchFolderSelect") {
        let priority = if ini_has_setting(root, "LaunchFolderSelect") { 4 } else { 2 };
        push_raw(priority, raw);
    }
    // 新版 PCL 用 LaunchFolders 保存列表，格式为「显示名>路径」，可能有多条。
    // 只有 Setup.ini 里的列表才代表当前这个 PCL；全局注册表里的列表可能属于另一个 PCL。
    if let Some(raw) = settings.get("LaunchFolders") {
        let priority = if ini_has_setting(root, "LaunchFolders") { 4 } else { 1 };
        for part in raw.split(['\n', '\r', '|']) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let path = part.split_once('>').map(|(_, path)| path.trim()).unwrap_or(part);
            push_raw(priority, path);
        }
    }
    let fallback = root.join(".minecraft");
    if fallback.is_dir() {
        candidates.push((3, fallback));
    }

    candidates
        .into_iter()
        .max_by_key(|(priority, path)| {
            let score = if read_selected_version(path).is_some() {
                2
            } else if path.join("versions").is_dir() {
                1
            } else {
                0
            };
            (*priority, score)
        })
        .map(|(_, path)| path)
}

/// PCL 的 Java 设置一般指向 bin 目录，这里统一换算成 java.exe。
fn resolve_java(root: &Path, settings: &HashMap<String, String>) -> Option<PathBuf> {
    let raw = settings.get("SetupJavaPath")?.trim().trim_matches('"');
    if raw.is_empty() {
        return None;
    }
    let path = if PathBuf::from(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        root.join(raw)
    };
    java::resolve_java_path(&path.to_string_lossy())
}

/// 内存滑条换算：0-12 步 0.1GB，13-25 步 0.5GB，26-33 步 1GB，之后再 2GB 一档。
fn ram_from_slider(value: f64) -> f64 {
    if value <= 12.0 {
        value * 0.1 + 0.3
    } else if value <= 25.0 {
        (value - 12.0) * 0.5 + 1.5
    } else if value <= 33.0 {
        (value - 25.0) + 8.0
    } else {
        (value - 33.0) * 2.0 + 16.0
    }
}

fn read_memory(settings: &HashMap<String, String>) -> (Option<bool>, Option<u32>) {
    match settings.get("LaunchRamType").and_then(|v| v.parse::<i32>().ok()) {
        Some(0) => (Some(true), None),
        Some(1) => {
            let slider = settings
                .get("LaunchRamCustom")
                .and_then(|v| v.parse::<f64>().ok());
            match slider {
                Some(value) => {
                    let mb = ((ram_from_slider(value) * 1024.0).round() as u32).clamp(1024, 32768);
                    (Some(false), Some(mb))
                }
                None => (None, None),
            }
        }
        _ => (None, None),
    }
}

fn read_window(settings: &HashMap<String, String>) -> (Option<u32>, Option<u32>) {
    let parse = |key: &str| settings.get(key).and_then(|v| v.trim().parse::<u32>().ok());
    let width = parse("LaunchArgumentWindowWidth");
    let height = parse("LaunchArgumentWindowHeight");
    if width.is_some() && height.is_some() && width != Some(0) && height != Some(0) {
        (width, height)
    } else {
        (None, None)
    }
}

fn read_update_mode(settings: &HashMap<String, String>) -> Option<String> {
    let mode = match settings.get("SystemSystemUpdate")?.trim().parse::<u32>().ok()? {
        0 => "auto",
        1 | 2 => "notify",
        3 => "off",
        _ => return None,
    };
    Some(mode.to_string())
}

fn read_download_source(settings: &HashMap<String, String>) -> Option<String> {
    let source = match settings.get("ToolDownloadSource")?.trim().parse::<u32>().ok()? {
        0 => "mirror",
        1 => "auto",
        2 => "official",
        _ => return None,
    };
    Some(source.to_string())
}

/// PCL 把上次启动的版本记在 .minecraft\PCL.ini 的 Version 项里。
fn read_selected_version(game_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(game_dir.join("PCL.ini")).ok()?;
    let version = parse_ini(&text).remove("Version")?;
    let version = version.trim().to_string();
    if version.is_empty() || !game_dir.join("versions").join(&version).is_dir() {
        return None;
    }
    Some(version)
}

fn update_mode_label(mode: &str) -> &'static str {
    match mode {
        "auto" => "有新版本时自动下载",
        "notify" => "有新版本时提示",
        _ => "关闭自动检查",
    }
}

/// PCL / 加载器合并的清单里，同一个 Maven 库可能留下多个版本（例如 asm 9.6 和 9.8）。
/// 这里只检测，不修改文件；实际清理在 apply 阶段做。
fn has_duplicate_library_versions(value: &Value) -> bool {
    let Some(libraries) = value.get("libraries").and_then(Value::as_array) else {
        return false;
    };
    let mut seen: HashMap<String, String> = HashMap::new();
    for library in libraries {
        let Some(name) = library.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some((key, version)) = install::maven_key_version(name) else {
            continue;
        };
        if let Some(previous) = seen.get(&key) {
            if previous != &version {
                return true;
            }
        } else {
            seen.insert(key, version);
        }
    }
    false
}

/// 扫描 game_dir 下的版本清单，返回（版本总数，含重复依赖的版本名）。
fn scan_version_libraries(game_dir: &Path) -> (usize, Vec<String>) {
    let Ok(entries) = std::fs::read_dir(game_dir.join("versions")) else {
        return (0, Vec::new());
    };
    let mut total = 0;
    let mut duplicates = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let json_path = entry.path().join(format!("{name}.json"));
        let Ok(text) = std::fs::read_to_string(&json_path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        total += 1;
        if has_duplicate_library_versions(&value) {
            duplicates.push(name);
        }
    }
    (total, duplicates)
}

/// 删除同一 Maven 坐标下的低版本条目，保留版本号最高的那一个。
/// 同版本的重复条目可能对应不同 rules / classifier，保留不动。
fn clean_library_versions(value: &mut Value) -> usize {
    let Some(libraries) = value.get_mut("libraries").and_then(Value::as_array_mut) else {
        return 0;
    };
    let mut grouped: HashMap<String, Vec<(usize, String)>> = HashMap::new();
    for (index, library) in libraries.iter().enumerate() {
        let Some(name) = library.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some((key, version)) = install::maven_key_version(name) else {
            continue;
        };
        grouped.entry(key).or_default().push((index, version));
    }

    let mut remove: Vec<usize> = Vec::new();
    for entries in grouped.values() {
        if entries.len() <= 1 {
            continue;
        }
        let mut best_version = entries[0].1.clone();
        for (_, version) in entries {
            if install::compare_versions(version, &best_version) == std::cmp::Ordering::Greater {
                best_version = version.clone();
            }
        }
        for (index, version) in entries {
            if version != &best_version {
                remove.push(*index);
            }
        }
    }
    if remove.is_empty() {
        return 0;
    }
    remove.sort_unstable();
    remove.dedup();
    let count = remove.len();
    for &index in remove.iter().rev() {
        if index < libraries.len() {
            libraries.remove(index);
        }
    }
    count
}

/// 导入时修正 PCL 版本清单：清理重复依赖，并保留 `.pcl-import.bak` 备份。
fn normalize_version_jsons(game_dir: &Path) -> (usize, usize) {
    let Ok(entries) = std::fs::read_dir(game_dir.join("versions")) else {
        return (0, 0);
    };
    let mut changed_versions = 0;
    let mut removed_entries = 0;
    for entry in entries.flatten() {
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let json_path = entry.path().join(format!("{name}.json"));
        let Ok(text) = std::fs::read_to_string(&json_path) else {
            continue;
        };
        let Ok(mut value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let removed = clean_library_versions(&mut value);
        if removed == 0 {
            continue;
        }
        let backup = entry
            .path()
            .join(format!("{name}.json.pcl-import.bak"));
        if std::fs::copy(&json_path, &backup).is_err() {
            continue;
        }
        let Ok(pretty) = serde_json::to_string_pretty(&value) else {
            continue;
        };
        if std::fs::write(&json_path, pretty).is_ok() {
            changed_versions += 1;
            removed_entries += removed;
        }
    }
    (changed_versions, removed_entries)
}

fn download_source_label(source: &str) -> &'static str {
    match source {
        "mirror" => "优先镜像源",
        "official" => "优先官方源",
        _ => "官方源优先，慢时切镜像",
    }
}

pub fn inspect(raw_root: &str) -> Result<PclPreview> {
    let root = normalize_root(raw_root)?;
    let looks_like_pcl = root.join("PCL").is_dir()
        || root.join(".minecraft").is_dir()
        || root.join("Setup.ini").is_file();
    if !looks_like_pcl {
        bail!("这个文件夹不像 PCL 目录：没有找到 PCL 或 .minecraft 子目录");
    }

    let settings = read_settings(&root);
    let game_dir = resolve_game_dir(&root, &settings);
    let java_candidate = resolve_java(&root, &settings);
    // PCL 记录的 Java 路径可能已被移动或删除，先实际运行一次再决定是否导入
    let java = java_candidate
        .as_deref()
        .and_then(|path| java::inspect_java(path, "PCL"));
    let (memory_auto, memory_mb) = read_memory(&settings);
    let (window_width, window_height) = read_window(&settings);
    let update_mode = read_update_mode(&settings);
    let download_source = read_download_source(&settings);
    let selected_version = game_dir.as_deref().and_then(read_selected_version);
    let (version_total, duplicate_versions) = game_dir
        .as_deref()
        .map(scan_version_libraries)
        .unwrap_or((0, Vec::new()));

    let mut findings = Vec::new();
    if let Some(dir) = &game_dir {
        findings.push(format!("游戏目录：{}", dir.display()));
    }
    if memory_auto == Some(true) {
        findings.push("内存：自动分配".to_string());
    } else if let Some(mb) = memory_mb {
        findings.push(format!("内存：{:.1} GB", mb as f64 / 1024.0));
    }
    if let Some(info) = &java {
        findings.push(format!(
            "Java：{}（Java {} · {} {}）",
            info.path, info.major, info.vendor, info.kind
        ));
    } else if let Some(path) = &java_candidate {
        findings.push(format!("Java：{}（无法运行，已跳过）", path.display()));
    }
    if let (Some(width), Some(height)) = (window_width, window_height) {
        findings.push(format!("游戏窗口：{width}×{height}"));
    }
    if let Some(mode) = &update_mode {
        findings.push(format!("更新策略：{}", update_mode_label(mode)));
    }
    if let Some(source) = &download_source {
        findings.push(format!("下载源：{}", download_source_label(source)));
    }
    if let Some(version) = &selected_version {
        findings.push(format!("上次启动版本：{version}"));
    }
    if version_total > 0 && !duplicate_versions.is_empty() {
        let names = duplicate_versions
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("、");
        let suffix = if duplicate_versions.len() > 3 { " 等" } else { "" };
        findings.push(format!(
            "版本清单：{version_total} 个版本，其中 {} 个含重复依赖（{names}{suffix}），导入时会自动清理",
            duplicate_versions.len()
        ));
    }
    findings.push("账号登录信息在 PCL 中加密保存，无法直接迁移，导入后需要重新登录".to_string());

    Ok(PclPreview {
        root: root.to_string_lossy().to_string(),
        game_dir: game_dir.map(|path| path.to_string_lossy().to_string()),
        java_path: java.map(|info| info.path),
        memory_auto,
        memory_mb,
        window_width,
        window_height,
        update_mode,
        download_source,
        selected_version,
        findings,
    })
}

/// 只写入确实找到的字段，其余保持启动器当前设置。
pub fn apply(cfg: &mut Config, raw_root: &str) -> Result<PclPreview> {
    let mut preview = inspect(raw_root)?;
    if let Some(dir) = &preview.game_dir {
        cfg.game_dir = dir.clone();
    }
    if let Some(java) = &preview.java_path {
        cfg.java_path = Some(java.clone());
    }
    if let Some(auto) = preview.memory_auto {
        cfg.memory_auto = auto;
    }
    if let Some(mb) = preview.memory_mb {
        cfg.memory_mb = mb;
    }
    if let (Some(width), Some(height)) = (preview.window_width, preview.window_height) {
        cfg.window_width = width;
        cfg.window_height = height;
    }
    if let Some(mode) = &preview.update_mode {
        cfg.update_mode = mode.clone();
    }
    if let Some(source) = &preview.download_source {
        cfg.download_source = source.clone();
    }
    if let Some(version) = &preview.selected_version {
        cfg.selected_version = Some(version.clone());
    }
    // 修正 PCL 清单里的重复依赖（asm 9.6/9.8 这类），避免导入后 Fabric 启动失败
    let (cleaned_versions, cleaned_entries) = preview
        .game_dir
        .as_deref()
        .map(|dir| normalize_version_jsons(Path::new(dir)))
        .unwrap_or((0, 0));
    if cleaned_entries > 0 {
        preview.findings.push(format!(
            "已清理 {cleaned_versions} 个版本清单中的 {cleaned_entries} 条重复依赖（已保留 .pcl-import.bak 备份）"
        ));
    }
    cfg.sync_game_dirs();
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::{clean_library_versions, has_duplicate_library_versions};
    use serde_json::json;

    #[test]
    fn cleans_duplicate_library_versions() {
        let mut value = json!({
            "libraries": [
                { "name": "org.ow2.asm:asm:9.6" },
                { "name": "org.ow2.asm:asm:9.8" },
                { "name": "org.ow2.asm:asm-analysis:9.8" }
            ]
        });
        assert!(has_duplicate_library_versions(&value));
        assert_eq!(clean_library_versions(&mut value), 1);
        let libraries = value["libraries"].as_array().unwrap();
        assert_eq!(libraries.len(), 2);
        assert!(libraries
            .iter()
            .any(|item| item["name"] == "org.ow2.asm:asm:9.8"));
    }
}
