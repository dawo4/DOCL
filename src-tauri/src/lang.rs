use crate::config::Config;
use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use zip::ZipArchive;

/// 游戏规则在语言文件里的中文名与说明。
#[derive(Debug, Clone, Default, Serialize)]
pub struct RuleLang {
    pub name: Option<String>,
    pub description: Option<String>,
}

fn camelize(id: &str) -> String {
    if !id.contains('_') {
        return id.to_string();
    }
    let mut out = String::with_capacity(id.len());
    let mut upper = false;
    for ch in id.chars() {
        if ch == '_' {
            upper = true;
            continue;
        }
        if upper {
            out.extend(ch.to_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// 把 zh_cn.json 里的 gamerule.* 合进结果，后读到的资源包覆盖前面的。
fn merge_lang(text: &str, out: &mut HashMap<String, RuleLang>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    let Some(map) = value.as_object() else {
        return;
    };
    for (key, value) in map {
        let Some(value) = value.as_str() else {
            continue;
        };
        let Some(rest) = key.strip_prefix("gamerule.") else {
            continue;
        };
        // 新版语言文件里还有 gamerule.minecraft.<snake_id> 这种带命名空间的写法
        let rest = rest.strip_prefix("minecraft.").unwrap_or(rest);
        if let Some(id) = rest.strip_suffix(".description") {
            out.entry(camelize(id)).or_default().description = Some(value.to_string());
        } else {
            out.entry(camelize(rest)).or_default().name = Some(value.to_string());
        }
    }
}

fn lang_path() -> &'static str {
    "assets/minecraft/lang/zh_cn.json"
}

fn read_zip_lang(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = ZipArchive::new(file).ok()?;
    let index = (0..archive.len()).find(|index| {
        archive
            .by_index(*index)
            .map(|entry| entry.name() == lang_path())
            .unwrap_or(false)
    })?;
    let mut entry = archive.by_index(index).ok()?;
    let mut text = String::new();
    entry.read_to_string(&mut text).ok()?;
    Some(text)
}

fn read_dir_lang(path: &Path) -> Option<String> {
    std::fs::read_to_string(path.join(lang_path())).ok()
}

/// 1.6 之后客户端 jar 里不再带语言文件，中文实际放在 assets 的对象库里：
/// 先读 assets/indexes/<assetIndex.id>.json 找到 zh_cn.json 的 hash，
/// 再去 assets/objects/<hash 前两位>/<hash> 取出内容。
fn read_assets_lang(cfg: &Config, version_id: &str) -> Option<String> {
    let index_id = version_chain(cfg, version_id).into_iter().find_map(|id| {
        version_json(cfg, &id)
            .and_then(|json| {
                json.get("assetIndex")
                    .and_then(|value| value.get("id"))
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
    })?;
    let root = cfg.game_root();
    let index_path = root
        .join("assets")
        .join("indexes")
        .join(format!("{index_id}.json"));
    let index: serde_json::Value = serde_json::from_slice(&std::fs::read(index_path).ok()?).ok()?;
    let hash = index
        .get("objects")?
        .get("minecraft/lang/zh_cn.json")?
        .get("hash")?
        .as_str()?;
    if hash.len() < 2 {
        return None;
    }
    let path = root
        .join("assets")
        .join("objects")
        .join(&hash[..2])
        .join(hash);
    std::fs::read_to_string(path).ok()
}

fn version_json(cfg: &Config, version_id: &str) -> Option<serde_json::Value> {
    let path = cfg
        .game_root()
        .join("versions")
        .join(version_id)
        .join(format!("{version_id}.json"));
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// 版本可能是加载器实例（没有自己的 jar），顺着 jar / inheritsFrom 找到真正带资源的版本。
fn version_chain(cfg: &Config, version_id: &str) -> Vec<String> {
    let mut chain: Vec<String> = Vec::new();
    let mut current = Some(version_id.to_string());
    while let Some(id) = current {
        if chain.len() >= 8 || chain.iter().any(|item| item == &id) {
            break;
        }
        chain.push(id.clone());
        let json = version_json(cfg, &id);
        if let Some(jar) = json
            .as_ref()
            .and_then(|value| value.get("jar"))
            .and_then(|value| value.as_str())
        {
            if !jar.is_empty() && !chain.iter().any(|item| item == jar) {
                chain.push(jar.to_string());
            }
        }
        current = json
            .as_ref()
            .and_then(|value| value.get("inheritsFrom"))
            .and_then(|value| value.as_str())
            .map(str::to_string);
    }
    chain
}

fn push_jars(dir: &Path, jars: &mut Vec<PathBuf>) {
    let preferred = dir.join(
        dir.file_name()
            .map(|name| format!("{}.jar", name.to_string_lossy()))
            .unwrap_or_default(),
    );
    if preferred.is_file() {
        jars.push(preferred.clone());
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path == preferred || !path.is_file() {
                continue;
            }
            let is_jar = path
                .extension()
                .map(|ext| ext.eq_ignore_ascii_case("jar"))
                .unwrap_or(false);
            if is_jar {
                jars.push(path);
            }
        }
    }
}

fn version_jars(cfg: &Config, version_id: &str) -> Vec<PathBuf> {
    let mut jars = Vec::new();
    for id in version_chain(cfg, version_id) {
        push_jars(&cfg.game_root().join("versions").join(&id), &mut jars);
    }
    // 保持「实例自己 → 父版本」的顺序，只去重
    let mut seen = std::collections::HashSet::new();
    jars.retain(|path| seen.insert(path.clone()));
    jars
}

fn resource_packs(cfg: &Config, version_id: &str) -> Vec<PathBuf> {
    let dir = crate::instance::game_dir(cfg, version_id).join("resourcepacks");
    let mut packs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    packs.sort();
    packs
}

/// 依次读资源库、客户端 jar 与 resourcepacks 里的 zh_cn.json；
/// 找不到时返回空表，由前端的硬编码表兜底。
pub fn game_rule_translations(cfg: &Config, version_id: &str) -> HashMap<String, RuleLang> {
    let mut out = HashMap::new();

    if let Some(text) = read_assets_lang(cfg, version_id) {
        merge_lang(&text, &mut out);
    }

    if let Some(text) = version_jars(cfg, version_id)
        .into_iter()
        .find_map(|jar| read_zip_lang(&jar))
    {
        merge_lang(&text, &mut out);
    }

    for pack in resource_packs(cfg, version_id) {
        let text = if pack.is_dir() {
            read_dir_lang(&pack)
        } else {
            read_zip_lang(&pack)
        };
        if let Some(text) = text {
            merge_lang(&text, &mut out);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{game_rule_translations, merge_lang, RuleLang};
    use crate::config::Config;
    use std::collections::HashMap;
    use std::path::Path;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("docl-lang-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_jar(path: &Path, lang: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        writer
            .start_file("assets/minecraft/lang/zh_cn.json", options)
            .unwrap();
        std::io::Write::write_all(&mut writer, lang.as_bytes()).unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn merges_name_and_description_and_normalizes_ids() {
        let mut out: HashMap<String, RuleLang> = HashMap::new();
        merge_lang(
            r#"{
                "gamerule.keep_inventory": "保留物品栏",
                "gamerule.keep_inventory.description": "死亡后保留背包",
                "gamerule.advance_time": "时间推进",
                "item.minecraft.apple": "苹果"
            }"#,
            &mut out,
        );
        let rule = out.get("keepInventory").unwrap();
        assert_eq!(rule.name.as_deref(), Some("保留物品栏"));
        assert_eq!(rule.description.as_deref(), Some("死亡后保留背包"));
        assert!(out.contains_key("advanceTime"));
        assert!(!out.contains_key("item"));
    }

    /// 加载器实例自己没有 jar 时，要顺着 inheritsFrom 找到父版本的客户端 jar
    #[test]
    fn reads_lang_from_parent_version_jar() {
        let root = temp_dir("inherit");
        write_jar(
            &root.join("versions/1.21.1/1.21.1.jar"),
            r#"{"gamerule.keepInventory":"保留物品栏"}"#,
        );
        let loader_dir = root.join("versions/1.21.1-forge");
        std::fs::create_dir_all(&loader_dir).unwrap();
        std::fs::write(
            loader_dir.join("1.21.1-forge.json"),
            r#"{"id":"1.21.1-forge","inheritsFrom":"1.21.1"}"#,
        )
        .unwrap();

        let mut cfg = Config::default();
        cfg.game_dir = root.to_string_lossy().to_string();
        cfg.version_isolation = false;
        let map = game_rule_translations(&cfg, "1.21.1-forge");
        assert_eq!(
            map.get("keepInventory").and_then(|rule| rule.name.as_deref()),
            Some("保留物品栏")
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// 新版客户端不再把语言文件打进 jar，要从 assets 对象库里按 hash 取
    #[test]
    fn reads_lang_from_assets_objects() {
        let root = temp_dir("assets");
        let versions = root.join("versions/26.1");
        std::fs::create_dir_all(&versions).unwrap();
        std::fs::write(
            versions.join("26.1.json"),
            r#"{"id":"26.1","assetIndex":{"id":"30"}}"#,
        )
        .unwrap();
        let hash = "ab1234567890abcdef1234567890abcdef1234";
        let indexes = root.join("assets/indexes");
        std::fs::create_dir_all(&indexes).unwrap();
        std::fs::write(
            indexes.join("30.json"),
            format!(
                r#"{{"objects":{{"minecraft/lang/zh_cn.json":{{"hash":"{hash}","size":1}}}}}}"#
            ),
        )
        .unwrap();
        let object_dir = root.join("assets/objects/ab");
        std::fs::create_dir_all(&object_dir).unwrap();
        std::fs::write(
            object_dir.join(hash),
            r#"{
                "gamerule.keepInventory": "死亡后保留物品栏",
                "gamerule.minecraft.raids": "启用袭击",
                "gamerule.minecraft.fire_spread_radius_around_player": "火焰蔓延半径",
                "gamerule.minecraft.fire_spread_radius_around_player.description": "玩家周围火可以蔓延的方块半径。"
            }"#,
        )
        .unwrap();

        let mut cfg = Config::default();
        cfg.game_dir = root.to_string_lossy().to_string();
        let map = game_rule_translations(&cfg, "26.1");
        assert_eq!(
            map.get("keepInventory").and_then(|rule| rule.name.as_deref()),
            Some("死亡后保留物品栏")
        );
        assert_eq!(
            map.get("raids").and_then(|rule| rule.name.as_deref()),
            Some("启用袭击")
        );
        let fire = map.get("fireSpreadRadiusAroundPlayer").unwrap();
        assert_eq!(fire.name.as_deref(), Some("火焰蔓延半径"));
        assert_eq!(
            fire.description.as_deref(),
            Some("玩家周围火可以蔓延的方块半径。")
        );
        std::fs::remove_dir_all(&root).ok();
    }
}
