//! 从 HMCL（Hello Minecraft! Launcher）导入游戏目录与启动设置。
//!
//! 支持三种配置布局：
//! - 3.16+：`.hmcl/config/game-directories.json` + `launcher-settings.json` + `game-settings.json`；
//! - 3.x：启动器目录下的 `hmcl.json`（旧版 `.hmcl.json`），用 `configurations` 保存多套配置；
//! - 更早版本：`hmcl.json` 的扁平字段（`gameDir` / `selectedProfile` / `maxMemory` …）。
//!
//! 三种格式都只读取、不修改原文件。

use crate::config::Config;
use crate::launcher_layout::{
    content_isolation, count_versions, detect_with_root, root_has_content, version_ids,
    VersionIsolation,
};
use crate::{instance, java};
use anyhow::{bail, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// HMCL 里的一个启动配置（对应一个游戏文件夹）。
#[derive(Debug, Clone, Serialize)]
pub struct HmclGameDirInfo {
    pub path: String,
    /// HMCL 的启动配置名称
    pub name: String,
    /// 是否为 HMCL 上次使用的配置
    pub active: bool,
    /// 清单齐全的版本数
    pub version_count: usize,
    /// 各版本导入后应使用的隔离状态
    pub isolation: Vec<VersionIsolation>,
}

#[derive(Debug, Default, Serialize)]
pub struct HmclPreview {
    pub root: String,
    pub config_path: Option<String>,
    pub game_dir: Option<String>,
    pub game_dirs: Vec<HmclGameDirInfo>,
    pub java_path: Option<String>,
    pub memory_auto: Option<bool>,
    pub memory_mb: Option<u32>,
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub fullscreen: Option<bool>,
    pub download_source: Option<String>,
    pub java_args: Option<String>,
    pub game_args: Option<String>,
    pub pre_launch: Option<String>,
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
            .ok_or_else(|| anyhow::anyhow!("无法定位 HMCL 所在目录"));
    }
    Ok(path)
}

/// HMCL 的配置文件可能叫 hmcl.json，也可能是旧版的 .hmcl.json。
fn find_config(root: &Path) -> Option<PathBuf> {
    ["hmcl.json", ".hmcl.json"]
        .iter()
        .map(|name| root.join(name))
        .find(|path| path.is_file())
}

fn text_of(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn u32_of(value: &Value, key: &str) -> Option<u32> {
    // 个别 HMCL 版本会把数值写成浮点（818.0），整数读取会失败
    value.get(key).and_then(|item| {
        item.as_u64()
            .or_else(|| item.as_f64().map(|number| number.round() as u64))
    })
    .map(|item| item.min(u32::MAX as u64) as u32)
}

fn bool_of(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

/// 请求路径可能是 .minecraft，也可能是包含 .minecraft 的外层目录。
fn normalize_game_dir(path: PathBuf) -> Option<PathBuf> {
    if path.join("versions").is_dir() {
        return Some(path);
    }
    let inner = path.join(".minecraft");
    if inner.join("versions").is_dir() {
        return Some(inner);
    }
    None
}

/// 把配置里的 gameDir 换算成绝对路径。
fn resolve_game_dir(raw: &str, relative: bool, base: &Path, common: Option<&str>) -> Option<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        // 没有单独指定时用 HMCL 的公共游戏目录
        if let Some(common) = common.map(str::trim).filter(|item| !item.is_empty()) {
            return normalize_game_dir(PathBuf::from(common));
        }
        return normalize_game_dir(base.join(".minecraft"));
    }
    let path = PathBuf::from(raw);
    let path = if relative || !path.is_absolute() {
        base.join(path)
    } else {
        path
    };
    normalize_game_dir(path)
}

fn download_source_of(value: &Value) -> Option<String> {
    if value
        .get("autoChooseDownloadType")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Some("auto".into());
    }
    let kind = text_of(value, "downloadType").to_ascii_lowercase();
    let source = match kind.as_str() {
        "mojang" | "official" => "official",
        "bmclapi" | "mcbbs" => "mirror",
        "" => return None,
        _ => "auto",
    };
    Some(source.into())
}

/// 一套启动配置的公共字段（新版在 global 里，旧版在根上）。
struct LaunchSettings {
    memory_auto: Option<bool>,
    memory_mb: Option<u32>,
    width: Option<u32>,
    height: Option<u32>,
    fullscreen: Option<bool>,
    java_dir: String,
    java_args: String,
    game_args: String,
    pre_launch: String,
}

/// 新版 HMCL（3.16+）把配置拆到 `.hmcl/config/*.json`。
fn modern_config(root: &Path) -> Option<(PathBuf, PathBuf)> {
    // 用户可能选启动器目录，也可能直接选 .hmcl 目录
    let nested = root.join(".hmcl").join("config");
    if nested.join("game-directories.json").is_file() {
        return Some((root.to_path_buf(), nested));
    }
    let direct = root.join("config");
    if direct.join("game-directories.json").is_file() {
        let base = root.parent().unwrap_or(root).to_path_buf();
        return Some((base, direct));
    }
    None
}

fn read_json(path: &Path) -> Option<Value> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// localizedText 可能是字符串，也可能是「语言 → 文本」的对象。
fn localized_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(Value::Object(map)) => {
            for key in ["zh_cn", "zh_CN", "en_us"] {
                if let Some(text) = map.get(key).and_then(Value::as_str) {
                    if !text.trim().is_empty() {
                        return text.trim().to_string();
                    }
                }
            }
            map.values()
                .find_map(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string()
        }
        _ => String::new(),
    }
}

/// 新版配置里的单套设置映射到统一的启动字段。
fn modern_settings(preset: Option<&Value>) -> LaunchSettings {
    let Some(preset) = preset else {
        return LaunchSettings {
            memory_auto: None,
            memory_mb: None,
            width: None,
            height: None,
            fullscreen: None,
            java_dir: String::new(),
            java_args: String::new(),
            game_args: String::new(),
            pre_launch: String::new(),
        };
    };
    let fullscreen = match text_of(preset, "windowType").as_str() {
        "FULLSCREEN" => Some(true),
        "WINDOWED" | "MAXIMIZED" => Some(false),
        _ => None,
    };
    let java_dir = if text_of(preset, "javaType") == "CUSTOM" {
        text_of(preset, "customJavaPath")
    } else {
        String::new()
    };
    LaunchSettings {
        memory_auto: bool_of(preset, "autoMemory"),
        memory_mb: u32_of(preset, "maxMemory").filter(|item| *item > 0),
        width: u32_of(preset, "width").filter(|item| *item > 0),
        height: u32_of(preset, "height").filter(|item| *item > 0),
        fullscreen,
        java_dir,
        java_args: text_of(preset, "jvmOptions"),
        game_args: text_of(preset, "gameArguments"),
        pre_launch: text_of(preset, "preLaunchCommand"),
    }
}

/// 版本清单里有 inheritsFrom 说明是加载器或整合包版本，算「可装 Mod 的版本」。
fn version_is_modded(game_dir: &Path, version: &str) -> bool {
    let path = game_dir
        .join("versions")
        .join(version)
        .join(format!("{version}.json"));
    std::fs::read_to_string(path)
        .map(|text| text.contains("\"inheritsFrom\""))
        .unwrap_or(false)
}

/// 新版 HMCL 的隔离策略：NEVER / ALWAYS / MODDED。
/// 先看文件实际位置，判断不出来时再按策略推断。
fn modern_isolation(
    game_dir: &Path,
    preset: Option<&Value>,
    ids: &[String],
) -> Vec<VersionIsolation> {
    let root_has = root_has_content(game_dir);
    let kind = preset
        .and_then(|item| item.get("defaultIsolationType"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    ids.iter()
        .filter_map(|id| {
            let (isolated, source) = match detect_with_root(game_dir, id, root_has) {
                Some(value) => (value, "content"),
                None => match kind {
                    "NEVER" => (false, "hmcl"),
                    "ALWAYS" => (true, "hmcl"),
                    // 默认 MODDED：加载器/整合包版本隔离，原版与根目录共用
                    "MODDED" | "" => (version_is_modded(game_dir, id), "hmcl"),
                    _ => return None,
                },
            };
            Some(VersionIsolation {
                version_id: id.clone(),
                isolated,
                source: source.into(),
            })
        })
        .collect()
}

/// 单个实例的设置文件：只保存它相对预设改过的字段。
fn instance_settings_path(game_dir: &Path, version: &str) -> PathBuf {
    game_dir
        .join("versions")
        .join(version)
        .join(".hmcl")
        .join("config")
        .join("instance-game-settings.json")
}

/// 合并「目录预设 + 实例覆盖」，得到该实例实际生效的设置。
fn effective_instance_settings(
    game_dir: &Path,
    version: &str,
    presets: &HashMap<String, &Value>,
    dir_preset: Option<&Value>,
) -> Value {
    let instance = read_json(&instance_settings_path(game_dir, version));
    // 实例文件里的 parent 可能指向另一个预设
    let parent = instance
        .as_ref()
        .and_then(|value| value.get("parent"))
        .and_then(Value::as_str);
    let mut base = parent
        .and_then(|id| presets.get(id).map(|item| (*item).clone()))
        .or_else(|| dir_preset.cloned())
        .unwrap_or_else(|| Value::Object(Default::default()));
    let Some(instance) = instance else {
        return base;
    };
    // 只有 overrideProperties 里列出的字段才真正覆盖预设
    let overrides: Vec<String> = instance
        .get("overrideProperties")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if let Some(target) = base.as_object_mut() {
        for key in overrides {
            if let Some(value) = instance.get(&key) {
                target.insert(key, value.clone());
            }
        }
    }
    base
}

/// 读取 3.16+ 的 `.hmcl/config`。
fn inspect_modern(root: &Path, base: &Path, config_dir: &Path) -> Result<HmclPreview> {
    let Some(dirs_value) = read_json(&config_dir.join("game-directories.json")) else {
        bail!("game-directories.json 无法读取");
    };
    let settings_value =
        read_json(&config_dir.join("launcher-settings.json")).unwrap_or(Value::Null);
    let presets_value = read_json(&config_dir.join("game-settings.json")).unwrap_or(Value::Null);

    // 预设 id → 预设对象
    let mut presets: HashMap<String, &Value> = HashMap::new();
    if let Some(list) = presets_value.get("presets").and_then(Value::as_array) {
        for preset in list {
            let id = text_of(preset, "id");
            if !id.is_empty() {
                presets.insert(id, preset);
            }
        }
    }

    let selected_dir_id = text_of(&settings_value, "selectedGameDirectory");
    let default_preset_id = text_of(&settings_value, "defaultGameSettingsPreset");
    let selected_map = settings_value
        .get("selectedInstance")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let mut entries: Vec<(String, String, PathBuf, Option<Value>, String)> = Vec::new();
    let Some(list) = dirs_value.get("directories").and_then(Value::as_array) else {
        bail!("game-directories.json 里没有 directories");
    };
    for item in list {
        let id = text_of(item, "id");
        let raw = text_of(item, "path");
        if raw.is_empty() {
            continue;
        }
        let path = PathBuf::from(&raw);
        let path = if path.is_absolute() {
            path
        } else {
            base.join(path)
        };
        let Some(dir) = normalize_game_dir(path) else {
            continue;
        };
        let name = localized_text(item.get("name"));
        let preset_id = non_empty(text_of(item, "legacyGameSettings"), default_preset_id.clone());
        let preset = presets.get(&preset_id).map(|item| (*item).clone());
        let selected = selected_map
            .get(&id)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        entries.push((id, name, dir, preset, selected));
    }
    if entries.is_empty() {
        bail!("HMCL 配置里没有找到带 versions 目录的游戏文件夹");
    }

    // 当前目录优先，其次是有选中版本的
    let active_index = entries
        .iter()
        .position(|(id, _, _, _, _)| !selected_dir_id.is_empty() && id == &selected_dir_id)
        .or_else(|| {
            entries
                .iter()
                .position(|(_, _, _, _, selected)| !selected.is_empty())
        })
        .unwrap_or(0);
    let active = entries.swap_remove(active_index);
    let mut ordered = vec![active];
    ordered.extend(entries);

    let mut game_dirs: Vec<HmclGameDirInfo> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for (index, (_, name, dir, preset, _)) in ordered.iter().enumerate() {
        let key = dir.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let ids = version_ids(dir);
        let label = if name.is_empty() {
            dir.file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_default()
        } else {
            name.clone()
        };
        game_dirs.push(HmclGameDirInfo {
            path: dir.to_string_lossy().to_string(),
            name: label,
            active: index == 0,
            version_count: ids.len(),
            isolation: modern_isolation(dir, preset.as_ref(), &ids),
        });
    }
    let (_, _, active_dir, active_preset, selected) = ordered.first().expect("至少有一个目录");
    // 实例级覆盖（内存 / Java / 窗口等）只对当前实例生效，取它的有效设置
    let effective =
        effective_instance_settings(active_dir, selected, &presets, active_preset.as_ref());
    let settings = modern_settings(Some(&effective));

    // Java：CUSTOM 时才用指定路径，其余交给启动器自动选择
    let java_path = if settings.java_dir.is_empty() {
        None
    } else {
        java::resolve_java_path(&settings.java_dir)
            .and_then(|path| java::inspect_java(&path, "HMCL").map(|info| info.path))
    };

    let download_source = match text_of(&settings_value, "fileDownloadSource").as_str() {
        "OFFICIAL" => Some("official".to_string()),
        "MIRROR" => Some("mirror".to_string()),
        "DEFAULT" => Some("auto".to_string()),
        _ => None,
    };

    let mut findings = Vec::new();
    findings.push(format!("配置文件：{}", config_dir.display()));
    if game_dirs.len() > 1 {
        findings.push(format!(
            "游戏文件夹：共 {} 个，会全部加入目录列表",
            game_dirs.len()
        ));
    }
    for info in &game_dirs {
        let marker = if info.active { "（当前）" } else { "" };
        findings.push(format!(
            "游戏目录{marker}：{}（{} 个版本）",
            info.path, info.version_count
        ));
        let shared: Vec<&str> = info
            .isolation
            .iter()
            .filter(|item| !item.isolated)
            .map(|item| item.version_id.as_str())
            .collect();
        if !shared.is_empty() {
            let names = shared.iter().take(3).copied().collect::<Vec<_>>().join("、");
            let suffix = if shared.len() > 3 { " 等" } else { "" };
            findings.push(format!(
                "　版本隔离：{names}{suffix} 在 HMCL 中未隔离，会共用根目录的 mods / 资源包 / 光影 / 存档"
            ));
        }
    }
    if settings.memory_auto == Some(true) {
        findings.push("内存：自动分配".to_string());
    } else if let Some(mb) = settings.memory_mb {
        findings.push(format!("内存：{:.1} GB", mb as f64 / 1024.0));
    }
    if let Some(path) = &java_path {
        findings.push(format!("Java：{path}"));
    }
    if let (Some(width), Some(height)) = (settings.width, settings.height) {
        findings.push(format!("游戏窗口：{width}×{height}"));
    }
    if let Some(source) = &download_source {
        findings.push(format!("下载源：{source}"));
    }
    if !selected.is_empty() {
        findings.push(format!("上次启动版本：{selected}"));
    }
    findings.push("HMCL 的账号信息由它自己加密保存，无法直接迁移，导入后需要重新登录".to_string());

    Ok(HmclPreview {
        root: root.to_string_lossy().to_string(),
        config_path: Some(config_dir.to_string_lossy().to_string()),
        game_dir: Some(active_dir.to_string_lossy().to_string()),
        game_dirs,
        java_path,
        memory_auto: settings.memory_auto,
        memory_mb: settings.memory_mb,
        window_width: settings.width,
        window_height: settings.height,
        fullscreen: settings.fullscreen,
        download_source,
        java_args: (!settings.java_args.is_empty()).then(|| settings.java_args.clone()),
        game_args: (!settings.game_args.is_empty()).then(|| settings.game_args.clone()),
        pre_launch: (!settings.pre_launch.is_empty()).then(|| settings.pre_launch.clone()),
        selected_version: (!selected.is_empty()).then(|| selected.clone()),
        findings,
    })
}

/// 优先取第一个非空字段，兼容新旧两种键名。
fn non_empty(first: String, second: String) -> String {
    if first.is_empty() {
        second
    } else {
        first
    }
}

fn launch_settings(value: &Value) -> LaunchSettings {
    let width = u32_of(value, "width").filter(|item| *item > 0);
    let height = u32_of(value, "height").filter(|item| *item > 0);
    LaunchSettings {
        memory_auto: bool_of(value, "autoMemory"),
        memory_mb: u32_of(value, "maxMemory").filter(|item| *item > 0),
        width,
        height,
        fullscreen: bool_of(value, "fullscreen"),
        java_dir: text_of(value, "javaDir"),
        java_args: text_of(value, "javaArgs"),
        game_args: non_empty(text_of(value, "minecraftArgs"), text_of(value, "gameArgs")),
        pre_launch: text_of(value, "precalledCommand"),
    }
}

/// 解析一套配置，返回（选中版本，游戏目录）。
fn config_dir(value: &Value, base: &Path, common: Option<&str>) -> Option<(String, PathBuf)> {
    let raw = text_of(value, "gameDir");
    let relative = bool_of(value, "useRelativePath").unwrap_or(false);
    let dir = resolve_game_dir(&raw, relative, base, common)?;
    let selected = non_empty(
        text_of(value, "selectedMinecraftVersion"),
        text_of(value, "selectedProfile"),
    );
    Some((selected, dir))
}

fn inspect_impl(root: &Path, config_path: Option<&Path>, value: &Value) -> Result<HmclPreview> {
    let base = config_path
        .and_then(Path::parent)
        .unwrap_or(root)
        .to_path_buf();
    let common = value.get("commonpath").and_then(Value::as_str);

    // 收集所有配置：名称、选中版本、游戏目录、公共设置
    let mut entries: Vec<(String, String, PathBuf, LaunchSettings)> = Vec::new();
    if let Some(map) = value.get("configurations").and_then(Value::as_object) {
        for (name, cfg) in map {
            let Some((selected, dir)) = config_dir(cfg, &base, common) else {
                continue;
            };
            let settings = cfg
                .get("global")
                .map(launch_settings)
                .unwrap_or_else(|| launch_settings(cfg));
            entries.push((name.clone(), selected, dir, settings));
        }
    } else {
        // 旧版扁平格式：只有一个游戏目录
        let raw = text_of(value, "gameDir");
        // 选中目录本身可能就是一个 .minecraft
        let dir = if raw.is_empty() {
            normalize_game_dir(base.clone())
                .or_else(|| resolve_game_dir("", false, &base, common))
        } else {
            resolve_game_dir(&raw, false, &base, common)
        };
        let Some(dir) = dir else {
            bail!("HMCL 配置里没有找到可用的游戏目录");
        };
        let selected = non_empty(
            text_of(value, "selectedProfile"),
            text_of(value, "selectedMinecraftVersion"),
        );
        entries.push(("HMCL".into(), selected, dir, launch_settings(value)));
    }

    if entries.is_empty() {
        bail!("HMCL 配置里没有找到带 versions 目录的游戏文件夹");
    }

    // 排序：上次使用的配置 > 有选中版本的 > 版本更多的
    let last = text_of(value, "last");
    let active_index = entries
        .iter()
        .position(|(name, _, _, _)| !last.is_empty() && name == &last)
        .or_else(|| {
            entries
                .iter()
                .position(|(_, selected, _, _)| !selected.is_empty())
        })
        .unwrap_or(0);
    let active = entries.swap_remove(active_index);
    let mut ordered = vec![active];
    ordered.extend(entries);

    let mut game_dirs: Vec<HmclGameDirInfo> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for (index, (name, _, dir, _)) in ordered.iter().enumerate() {
        let key = dir.to_string_lossy().to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        game_dirs.push(HmclGameDirInfo {
            path: dir.to_string_lossy().to_string(),
            name: name.clone(),
            active: index == 0,
            version_count: count_versions(dir),
            isolation: content_isolation(dir),
        });
    }
    let active_entry = ordered.first().expect("至少有一个配置");
    let (_, selected, active_dir, settings) = active_entry;

    // Java：HMCL 记录的是 java.exe 的路径，先实际运行一次确认还能用
    let java_path = if settings.java_dir.is_empty() {
        None
    } else {
        java::resolve_java_path(&settings.java_dir)
            .and_then(|path| java::inspect_java(&path, "HMCL").map(|info| info.path))
    };

    let download_source = download_source_of(value);
    let mut findings = Vec::new();
    match config_path {
        Some(path) => findings.push(format!("配置文件：{}", path.display())),
        None => findings.push("没有找到 hmcl.json，只导入了选中的游戏目录".into()),
    }
    if game_dirs.len() > 1 {
        findings.push(format!(
            "启动配置：共 {} 个，会全部加入目录列表",
            game_dirs.len()
        ));
    }
    for info in &game_dirs {
        let marker = if info.active { "（当前）" } else { "" };
        findings.push(format!(
            "游戏目录{marker}：{}（配置「{}」，{} 个版本）",
            info.path, info.name, info.version_count
        ));
        let shared: Vec<&str> = info
            .isolation
            .iter()
            .filter(|item| !item.isolated)
            .map(|item| item.version_id.as_str())
            .collect();
        if !shared.is_empty() {
            let names = shared.iter().take(3).copied().collect::<Vec<_>>().join("、");
            let suffix = if shared.len() > 3 { " 等" } else { "" };
            findings.push(format!(
                "　版本隔离：{names}{suffix} 在 HMCL 中未隔离，会共用根目录的 mods / 资源包 / 光影 / 存档"
            ));
        }
    }
    if settings.memory_auto == Some(true) {
        findings.push("内存：自动分配".to_string());
    } else if let Some(mb) = settings.memory_mb {
        findings.push(format!("内存：{:.1} GB", mb as f64 / 1024.0));
    }
    if let Some(path) = &java_path {
        findings.push(format!("Java：{path}"));
    } else if !settings.java_dir.is_empty() {
        findings.push(format!("Java：{}（无法运行，已跳过）", settings.java_dir));
    }
    if let (Some(width), Some(height)) = (settings.width, settings.height) {
        findings.push(format!("游戏窗口：{width}×{height}"));
    }
    if let Some(source) = &download_source {
        findings.push(format!("下载源：{source}"));
    }
    if !selected.is_empty() {
        findings.push(format!("上次启动版本：{selected}"));
    }
    findings.push("HMCL 的账号信息由它自己加密保存，无法直接迁移，导入后需要重新登录".to_string());

    Ok(HmclPreview {
        root: root.to_string_lossy().to_string(),
        config_path: config_path.map(|path| path.to_string_lossy().to_string()),
        game_dir: Some(active_dir.to_string_lossy().to_string()),
        game_dirs,
        java_path,
        memory_auto: settings.memory_auto,
        memory_mb: settings.memory_mb,
        window_width: settings.width,
        window_height: settings.height,
        fullscreen: settings.fullscreen,
        download_source,
        java_args: (!settings.java_args.is_empty()).then(|| settings.java_args.clone()),
        game_args: (!settings.game_args.is_empty()).then(|| settings.game_args.clone()),
        pre_launch: (!settings.pre_launch.is_empty()).then(|| settings.pre_launch.clone()),
        selected_version: (!selected.is_empty()).then(|| selected.clone()),
        findings,
    })
}

pub fn inspect(raw_root: &str) -> Result<HmclPreview> {
    let root = normalize_root(raw_root)?;
    // 3.16+ 的配置分散在 .hmcl/config 下，优先按新格式解析
    if let Some((base, config_dir)) = modern_config(&root) {
        return inspect_modern(&root, &base, &config_dir);
    }
    let config_path = find_config(&root);
    match config_path {
        Some(path) => {
            let bytes = std::fs::read(&path)?;
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|err| anyhow::anyhow!("hmcl.json 格式不正确：{err}"))?;
            inspect_impl(&root, Some(&path), &value)
        }
        None => {
            // 允许直接把 .minecraft 或它的外层目录当成导入目标：没有 HMCL 设置，只接管目录
            if normalize_game_dir(root.clone()).is_some() {
                inspect_impl(&root, None, &Value::Null)
            } else {
                bail!("这个文件夹里没有 hmcl.json，也不像 .minecraft 目录")
            }
        }
    }
}

/// 只写入确实找到的字段，其余保持启动器当前设置。
pub fn apply(cfg: &mut Config, raw_root: &str) -> Result<HmclPreview> {
    let mut preview = inspect(raw_root)?;
    for info in &preview.game_dirs {
        cfg.add_dir(&info.path);
    }
    if let Some(dir) = &preview.game_dir {
        cfg.game_dir = dir.clone();
    }
    if let Some(java) = &preview.java_path {
        cfg.remember_java(java);
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
    if let Some(fullscreen) = preview.fullscreen {
        cfg.fullscreen = fullscreen;
    }
    if let Some(source) = &preview.download_source {
        cfg.download_source = source.clone();
    }
    if let Some(args) = &preview.java_args {
        cfg.java_args = args.clone();
    }
    if let Some(args) = &preview.game_args {
        cfg.game_args = args.clone();
    }
    if let Some(command) = &preview.pre_launch {
        cfg.pre_launch = command.clone();
    }
    if let Some(version) = &preview.selected_version {
        cfg.selected_version = Some(version.clone());
    }
    // 隔离状态按「游戏目录 + 版本」存储，每个文件夹用自己的 cfg 写入
    for info in &preview.game_dirs {
        let mut dir_cfg = cfg.clone();
        dir_cfg.game_dir = info.path.clone();
        for entry in &info.isolation {
            let mut meta = instance::get(&dir_cfg, &entry.version_id);
            if meta.isolated == Some(entry.isolated) {
                continue;
            }
            meta.isolated = Some(entry.isolated);
            if let Err(err) = instance::save(&dir_cfg, &entry.version_id, &meta) {
                preview.findings.push(format!(
                    "无法为 {} 写入版本隔离设置：{err}",
                    entry.version_id
                ));
            }
        }
    }
    cfg.sync_game_dirs();
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("docl-hmcl-{tag}-{stamp}"))
    }

    fn make_version(game_dir: &Path, id: &str) {
        let dir = game_dir.join("versions").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{id}.json")), "{}").unwrap();
    }

    /// 新版配置：多个启动配置各自指向不同游戏目录，`last` 决定当前项
    #[test]
    fn parses_multiple_configurations() {
        let root = temp_dir("configs");
        std::fs::create_dir_all(&root).unwrap();
        let first = root.join("a").join(".minecraft");
        let second = root.join("b").join(".minecraft");
        make_version(&first, "1.20.1");
        make_version(&second, "1.21.8");
        make_version(&second, "1.21.9");

        let value = json!({
            "_version": 2,
            "last": "B",
            "downloadType": "bmclapi",
            "configurations": {
                "A": {
                    "gameDir": "a/.minecraft",
                    "useRelativePath": true,
                    "selectedMinecraftVersion": "1.20.1",
                    "global": { "autoMemory": true, "maxMemory": 4096, "width": 1280, "height": 720 }
                },
                "B": {
                    "gameDir": "b/.minecraft",
                    "useRelativePath": true,
                    "selectedMinecraftVersion": "1.21.9",
                    "global": { "autoMemory": false, "maxMemory": 8192, "width": 1920, "height": 1080 }
                }
            }
        });
        let preview = inspect_impl(&root, None, &value).unwrap();
        assert_eq!(preview.game_dirs.len(), 2);
        assert!(preview.game_dirs[0].active);
        assert_eq!(preview.game_dirs[0].name, "B");
        assert_eq!(preview.game_dirs[0].version_count, 2);
        assert_eq!(preview.selected_version.as_deref(), Some("1.21.9"));
        assert_eq!(preview.memory_mb, Some(8192));
        assert_eq!(preview.memory_auto, Some(false));
        assert_eq!(preview.window_width, Some(1920));
        assert_eq!(preview.download_source.as_deref(), Some("mirror"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 旧版扁平字段也能读：gameDir、selectedProfile、javaArgs
    #[test]
    fn parses_legacy_flat_config() {
        let root = temp_dir("legacy");
        let game = root.join(".minecraft");
        make_version(&game, "1.12.2");
        let value = json!({
            "gameDir": ".minecraft",
            "selectedProfile": "1.12.2",
            "maxMemory": 4096,
            "javaArgs": "-XX:+UseG1GC",
            "precalledCommand": "echo hi"
        });
        let preview = inspect_impl(&root, None, &value).unwrap();
        assert_eq!(preview.game_dirs.len(), 1);
        assert_eq!(preview.game_dirs[0].version_count, 1);
        assert_eq!(preview.selected_version.as_deref(), Some("1.12.2"));
        assert_eq!(preview.java_args.as_deref(), Some("-XX:+UseG1GC"));
        assert_eq!(preview.pre_launch.as_deref(), Some("echo hi"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 指向不存在目录的配置会被跳过，不会生成空条目
    #[test]
    fn skips_configurations_without_versions() {
        let root = temp_dir("skip");
        std::fs::create_dir_all(&root).unwrap();
        let good = root.join("good");
        make_version(&good, "1.20.1");
        let value = json!({
            "last": "Missing",
            "configurations": {
                "Missing": { "gameDir": "nope", "useRelativePath": true },
                "Good": { "gameDir": "good", "useRelativePath": true }
            }
        });
        let preview = inspect_impl(&root, None, &value).unwrap();
        assert_eq!(preview.game_dirs.len(), 1);
        assert_eq!(preview.game_dirs[0].name, "Good");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 没有配置文件但选中的就是 .minecraft：只接管目录
    #[test]
    fn imports_plain_game_dir_without_config() {
        let root = temp_dir("plain");
        make_version(&root, "1.21.1");
        let preview = inspect(root.to_string_lossy().as_ref()).unwrap();
        assert!(preview.config_path.is_none());
        assert_eq!(preview.game_dirs.len(), 1);
        assert_eq!(preview.game_dirs[0].version_count, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 选中的是包含 .minecraft 的外层目录时，也能自动落到内层
    #[test]
    fn imports_outer_folder_with_nested_minecraft() {
        let root = temp_dir("outer");
        make_version(&root.join(".minecraft"), "1.20.1");
        let preview = inspect(root.to_string_lossy().as_ref()).unwrap();
        assert_eq!(preview.game_dirs.len(), 1);
        assert!(preview.game_dirs[0].path.ends_with(".minecraft"));
        assert_eq!(preview.game_dirs[0].version_count, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// HMCL 3.16+：配置拆在 .hmcl/config 下，目录用 id 关联
    #[test]
    fn parses_modern_hmcl_config_layout() {
        let root = temp_dir("modern");
        let config = root.join(".hmcl").join("config");
        std::fs::create_dir_all(&config).unwrap();
        let game = root.join(".minecraft");
        make_version(&game, "Fabulously Optimized");
        // 模组放在版本目录里，应判定为隔离
        std::fs::create_dir_all(game.join("versions").join("Fabulously Optimized").join("mods"))
            .unwrap();
        std::fs::write(
            game.join("versions")
                .join("Fabulously Optimized")
                .join("mods")
                .join("a.jar"),
            b"x",
        )
        .unwrap();

        let dir_id = "game-directory:7105bc1f-490e-5e8c-878c-f5844c3d4bc3";
        let preset_id = "game-settings-preset:01a1f81f-54ce-7b84-8cfc-88370cc570c4";
        std::fs::write(
            config.join("game-directories.json"),
            json!({
                "directories": [{ "id": dir_id, "path": ".minecraft" }]
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            config.join("launcher-settings.json"),
            json!({
                "selectedGameDirectory": dir_id,
                "defaultGameSettingsPreset": preset_id,
                "selectedInstance": { dir_id: "Fabulously Optimized" },
                "fileDownloadSource": "MIRROR"
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            config.join("game-settings.json"),
            json!({
                "presets": [{
                    "id": preset_id,
                    "autoMemory": false,
                    "maxMemory": 6144,
                    "windowType": "WINDOWED",
                    "width": 1280.0,
                    "height": 720.0,
                    "jvmOptions": "-XX:+UseG1GC",
                    "gameArguments": "--demo",
                    "preLaunchCommand": "echo hi",
                    "defaultIsolationType": "MODDED"
                }]
            })
            .to_string(),
        )
        .unwrap();

        let preview = inspect(root.to_string_lossy().as_ref()).unwrap();
        assert!(preview.config_path.as_deref().unwrap().ends_with("config"));
        assert_eq!(preview.game_dirs.len(), 1);
        assert_eq!(preview.game_dirs[0].version_count, 1);
        assert!(preview.game_dirs[0].isolation[0].isolated);
        assert_eq!(preview.selected_version.as_deref(), Some("Fabulously Optimized"));
        assert_eq!(preview.memory_auto, Some(false));
        assert_eq!(preview.memory_mb, Some(6144));
        assert_eq!(preview.window_width, Some(1280));
        assert_eq!(preview.window_height, Some(720));
        assert_eq!(preview.fullscreen, Some(false));
        assert_eq!(preview.java_args.as_deref(), Some("-XX:+UseG1GC"));
        assert_eq!(preview.game_args.as_deref(), Some("--demo"));
        assert_eq!(preview.pre_launch.as_deref(), Some("echo hi"));
        assert_eq!(preview.download_source.as_deref(), Some("mirror"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 实例级覆盖只应用 overrideProperties 里列出的字段
    #[test]
    fn modern_instance_overrides_apply_selectively() {
        let root = temp_dir("override");
        let config = root.join(".hmcl").join("config");
        std::fs::create_dir_all(&config).unwrap();
        let game = root.join(".minecraft");
        make_version(&game, "pack");
        let instance_dir = game
            .join("versions")
            .join("pack")
            .join(".hmcl")
            .join("config");
        std::fs::create_dir_all(&instance_dir).unwrap();
        std::fs::write(
            instance_dir.join("instance-game-settings.json"),
            json!({
                "overrideProperties": ["maxMemory", "width", "height"],
                "maxMemory": 8192,
                "width": 1600,
                "height": 900,
                // 没有列进 overrideProperties，应被忽略
                "jvmOptions": "-XX:+IgnoreThis"
            })
            .to_string(),
        )
        .unwrap();

        let dir_id = "game-directory:7105bc1f-490e-5e8c-878c-f5844c3d4bc3";
        let preset_id = "game-settings-preset:01a1f81f-54ce-7b84-8cfc-88370cc570c4";
        std::fs::write(
            config.join("game-directories.json"),
            json!({ "directories": [{ "id": dir_id, "path": ".minecraft" }] }).to_string(),
        )
        .unwrap();
        std::fs::write(
            config.join("launcher-settings.json"),
            json!({
                "selectedGameDirectory": dir_id,
                "defaultGameSettingsPreset": preset_id,
                "selectedInstance": { dir_id: "pack" }
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            config.join("game-settings.json"),
            json!({
                "presets": [{
                    "id": preset_id,
                    "autoMemory": false,
                    "maxMemory": 4096,
                    "width": 1280,
                    "height": 720,
                    "jvmOptions": "-XX:+UseG1GC"
                }]
            })
            .to_string(),
        )
        .unwrap();

        let preview = inspect(root.to_string_lossy().as_ref()).unwrap();
        assert_eq!(preview.memory_mb, Some(8192));
        assert_eq!(preview.window_width, Some(1600));
        assert_eq!(preview.window_height, Some(900));
        assert_eq!(preview.java_args.as_deref(), Some("-XX:+UseG1GC"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// MODDED 策略下，纯原版版本与根目录共用，加载器版本才隔离
    #[test]
    fn modern_isolation_follows_modded_policy() {
        let root = temp_dir("policy");
        let game = root.join(".minecraft");
        make_version(&game, "1.21.1");
        make_version(&game, "fabric-1.21.1");
        std::fs::write(
            game.join("versions")
                .join("fabric-1.21.1")
                .join("fabric-1.21.1.json"),
            r#"{"id":"fabric-1.21.1","inheritsFrom":"1.21.1"}"#,
        )
        .unwrap();
        let ids = version_ids(&game);
        let preset = json!({ "defaultIsolationType": "MODDED" });
        let list = modern_isolation(&game, Some(&preset), &ids);
        let vanilla = list.iter().find(|item| item.version_id == "1.21.1").unwrap();
        let modded = list.iter().find(|item| item.version_id == "fabric-1.21.1").unwrap();
        assert!(!vanilla.isolated);
        assert!(modded.isolated);
        let _ = std::fs::remove_dir_all(&root);
    }
}
