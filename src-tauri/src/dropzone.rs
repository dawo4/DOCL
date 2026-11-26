use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};

/// 拖入窗口的文件或文件夹，识别出的分类与给用户看的说明。
#[derive(Debug, Clone, Serialize)]
pub struct DroppedItem {
    pub path: String,
    pub file_name: String,
    pub size: u64,
    /// 与 instance::content_dir 接受的分类保持一致；整合包与未识别项单独处理
    pub kind: String,
    pub label: String,
    pub detail: String,
}

fn label_of(kind: &str) -> &'static str {
    match kind {
        "mods" => "模组",
        "resourcepacks" => "资源包",
        "shaderpacks" => "光影包",
        "datapacks" => "数据包",
        "saves" => "存档",
        "schematics" => "投影文件",
        "modpack" => "整合包",
        _ => "未识别",
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// 压缩包外层常套一个同名文件夹，因此这里按「任意层级」匹配目录与文件名。
fn has_dir(names: &[String], dir: &str) -> bool {
    let nested = format!("/{dir}/");
    names
        .iter()
        .any(|name| name == dir || name.starts_with(&format!("{dir}/")) || name.contains(&nested))
}

fn has_file(names: &[String], file: &str) -> bool {
    let nested = format!("/{file}");
    names.iter().any(|name| name == file || name.ends_with(&nested))
}

fn archive_names(path: &Path) -> Option<Vec<String>> {
    let mut archive = crate::ziputil::open(path).ok()?;
    let mut names = Vec::new();
    for index in 0..archive.len() {
        if let Ok(entry) = archive.by_index(index) {
            if !entry.is_dir() {
                names.push(entry.name().to_string());
            }
        }
    }
    Some(names)
}

fn read_entry_ending(path: &Path, suffix: &str) -> Option<String> {
    let mut archive = crate::ziputil::open(path).ok()?;
    for index in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(index) else {
            continue;
        };
        if entry.is_dir() || !entry.name().ends_with(suffix) {
            continue;
        }
        let mut text = String::new();
        if entry.read_to_string(&mut text).is_ok() && !text.trim().is_empty() {
            return Some(text);
        }
    }
    None
}

fn classify_names(path: &Path, names: &[String]) -> String {
    if has_file(names, "modrinth.index.json") {
        return "modpack".into();
    }
    // 只看文件名会把普通资源包里同名的 manifest.json 误判成整合包，读内容确认
    if has_file(names, "manifest.json") {
        if let Some(text) = read_entry_ending(path, "manifest.json") {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                if value.get("minecraft").is_some() && value.get("files").is_some() {
                    return "modpack".into();
                }
            }
        }
    }
    // 没有清单的整合包：mods/ 加上配置目录或服务端特征文件。
    // 服务端整合包一般有 libraries/ 或 server.properties，客户端包一般有 config/scripts。
    if has_dir(names, "mods")
        && (has_dir(names, "config")
            || has_dir(names, "scripts")
            || has_dir(names, "defaultconfigs")
            || has_dir(names, "libraries")
            || has_file(names, "server.properties")
            || has_file(names, "eula.txt"))
    {
        return "modpack".into();
    }
    if has_file(names, "level.dat") {
        return "saves".into();
    }
    if has_file(names, "pack.mcmeta") {
        if has_dir(names, "data") {
            return "datapacks".into();
        }
        if has_dir(names, "assets") {
            return "resourcepacks".into();
        }
    }
    if has_dir(names, "shaders")
        || names
            .iter()
            .any(|name| name.ends_with(".fsh") || name.ends_with(".vsh"))
    {
        return "shaderpacks".into();
    }
    if names.iter().any(|name| {
        matches!(
            name.rsplit('/').next().unwrap_or_default(),
            "fabric.mod.json" | "quilt.mod.json"
        ) || name.ends_with("META-INF/mods.toml")
            || name.ends_with("META-INF/neoforge.mods.toml")
    }) {
        return "mods".into();
    }
    if path
        .extension()
        .and_then(|value| value.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("jar"))
        .unwrap_or(false)
    {
        return "mods".into();
    }
    "unknown".into()
}

fn classify_dir(path: &Path) -> String {
    if path.join("level.dat").is_file() {
        return "saves".into();
    }
    let mcmeta = path.join("pack.mcmeta").is_file();
    if mcmeta {
        if path.join("data").is_dir() {
            return "datapacks".into();
        }
        if path.join("assets").is_dir() {
            return "resourcepacks".into();
        }
    }
    if path.join("shaders").is_dir() {
        return "shaderpacks".into();
    }
    "unknown".into()
}

fn classify(path: &Path) -> (String, String) {
    if path.is_dir() {
        return (classify_dir(path), "文件夹".into());
    }
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase();
    if matches!(ext.as_str(), "litematic" | "schematic" | "nbt" | "schem") {
        return ("schematics".into(), human_size(file_size(path)));
    }
    if matches!(ext.as_str(), "jar" | "zip" | "mrpack" | "mcworld" | "mctemplate") {
        if let Some(names) = archive_names(path) {
            let kind = classify_names(path, &names);
            let detail = format!("{} 个文件 · {}", names.len(), human_size(file_size(path)));
            return (kind, detail);
        }
    }
    if ext == "jar" {
        return ("mods".into(), human_size(file_size(path)));
    }
    ("unknown".into(), human_size(file_size(path)))
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

pub fn inspect(paths: &[String]) -> Vec<DroppedItem> {
    let mut items = Vec::new();
    for raw in paths {
        let path = PathBuf::from(raw);
        if !path.exists() {
            continue;
        }
        let file_name = path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| raw.clone());
        let (kind, detail) = classify(&path);
        items.push(DroppedItem {
            path: path.to_string_lossy().to_string(),
            file_name,
            size: file_size(&path),
            label: label_of(&kind).to_string(),
            kind,
            detail,
        });
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn classifies_server_pack_without_manifest() {
        let list = names(&[
            "RLCraft/mods/a.jar",
            "RLCraft/libraries/x.jar",
            "RLCraft/server.properties",
        ]);
        assert_eq!(classify_names(Path::new("pack.zip"), &list), "modpack");
    }

    #[test]
    fn classifies_client_pack_without_manifest() {
        let list = names(&["mods/a.jar", "config/b.toml", "scripts/c.zs"]);
        assert_eq!(classify_names(Path::new("pack.zip"), &list), "modpack");
    }

    #[test]
    fn plain_mod_collection_is_not_modpack() {
        let list = names(&["a.jar", "b.jar"]);
        assert_eq!(classify_names(Path::new("mods.zip"), &list), "unknown");
    }
}
