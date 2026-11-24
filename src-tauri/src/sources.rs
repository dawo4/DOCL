//! 分来源下载偏好：把设置页的四个下拉框落到具体 URL。
//!
//! - 元数据：版本清单、版本 JSON
//! - 文件：游戏库、资源、加载器、Java
//! - Modrinth / CurseForge：搜索接口、项目接口与文件 CDN
//!
//! 每个类别可单独选择「自动 / 官方优先 / 镜像优先 / CERNET 优先」，
//! 未设置时回退到旧的全局 `download_source`。

use crate::config::Config;
use crate::mirror;
use std::sync::RwLock;

const MR_OFFICIAL: &str = "https://api.modrinth.com/v2";
const MR_MIRROR: &str = "https://mod.mcimirror.top/modrinth/v2";
const CERNET_BASE: &str = "https://mirrors.cernet.edu.cn/bmclapi/";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    Metadata,
    Files,
    Modrinth,
    CurseForge,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Auto,
    Official,
    Mirror,
    Cernet,
}

impl Mode {
    fn parse(value: &str) -> Self {
        match value.trim() {
            "official" => Mode::Official,
            "mirror" => Mode::Mirror,
            "cernet" => Mode::Cernet,
            _ => Mode::Auto,
        }
    }
}

#[derive(Clone, Copy)]
struct Prefs {
    metadata: Mode,
    files: Mode,
    modrinth: Mode,
    curseforge: Mode,
    /// 旧的全局选项，用于未分类的 URL
    legacy: Mode,
}

impl Prefs {
    const fn new() -> Self {
        Self {
            metadata: Mode::Auto,
            files: Mode::Auto,
            modrinth: Mode::Auto,
            curseforge: Mode::Auto,
            legacy: Mode::Auto,
        }
    }
}

static PREFS: RwLock<Prefs> = RwLock::new(Prefs::new());

/// 配置加载/保存后调用，把下拉框选择同步到下载层。
pub fn apply(cfg: &Config) {
    let legacy = Mode::parse(&cfg.download_source);
    let pick = |value: &str| {
        if value.trim().is_empty() {
            legacy
        } else {
            Mode::parse(value)
        }
    };
    let next = Prefs {
        metadata: pick(&cfg.source_metadata),
        files: pick(&cfg.source_files),
        modrinth: pick(&cfg.source_modrinth),
        curseforge: pick(&cfg.source_curseforge),
        legacy,
    };
    if let Ok(mut guard) = PREFS.write() {
        *guard = next;
    }
}

fn prefs() -> Prefs {
    PREFS.read().map(|guard| *guard).unwrap_or(Prefs::new())
}

fn mode_of(category: Category) -> Mode {
    let prefs = prefs();
    match category {
        Category::Metadata => prefs.metadata,
        Category::Files => prefs.files,
        Category::Modrinth => prefs.modrinth,
        Category::CurseForge => prefs.curseforge,
        Category::Other => prefs.legacy,
    }
}

/// 接口请求是否镜像优先。自动模式下 CurseForge 默认镜像优先（MCIM 往往比直连稳），
/// Modrinth 保持官方优先。
pub fn api_mirror_first(category: Category) -> bool {
    match mode_of(category) {
        Mode::Mirror | Mode::Cernet => true,
        Mode::Official => false,
        Mode::Auto => category == Category::CurseForge,
    }
}

/// Modrinth 接口基地址：官方 + MCIM 镜像，按偏好排序。
pub fn modrinth_api_bases() -> [&'static str; 2] {
    if api_mirror_first(Category::Modrinth) {
        [MR_MIRROR, MR_OFFICIAL]
    } else {
        [MR_OFFICIAL, MR_MIRROR]
    }
}

/// 根据 URL 判断属于哪个下载类别。
pub fn category(url: &str) -> Category {
    let lower = url.to_ascii_lowercase();
    if lower.contains("mod.mcimirror.top/curseforge")
        || lower.contains("curseforge")
        || lower.contains("curse.tools")
        || lower.contains("cursemaven.com")
        || lower.contains("forgecdn.net")
    {
        return Category::CurseForge;
    }
    if lower.contains("mod.mcimirror.top/modrinth") || lower.contains("modrinth") {
        return Category::Modrinth;
    }
    if lower.contains("piston-meta.mojang.com") || lower.contains("launchermeta.mojang.com") {
        return Category::Metadata;
    }
    if lower.contains("bmclapi") || lower.contains("mirrors.cernet.edu.cn") {
        if lower.contains("/maven/")
            || lower.contains("/assets/")
            || lower.contains("/objects/")
            || lower.contains("/libraries/")
            || lower.contains("/forge/")
            || lower.contains("fabric-meta")
            || lower.contains("quilt-meta")
        {
            return Category::Files;
        }
        if lower.contains("version_manifest")
            || lower.contains("/v1/packages/")
            || lower.contains("/version/")
            || lower.contains("/versions/")
        {
            return Category::Metadata;
        }
        return Category::Files;
    }
    if lower.contains("piston-data.mojang.com")
        || lower.contains("libraries.minecraft.net")
        || lower.contains("resources.download.minecraft.net")
        || lower.contains("maven.minecraftforge.net")
        || lower.contains("maven.neoforged.net")
        || lower.contains("files.minecraftforge.net")
        || lower.contains("meta.fabricmc.net")
        || lower.contains("maven.fabricmc.net")
        || lower.contains("meta.quiltmc.org")
        || lower.contains("maven.quiltmc.org")
        || lower.contains("api.adoptium.net")
        || lower.contains("github.com/adoptium")
    {
        return Category::Files;
    }
    Category::Other
}

/// CERNET 静态镜像地址：BMCLAPI 的 maven 目录在它上面不带 `/maven/` 前缀。
fn cernet_url(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://bmclapi2.bangbang93.com/maven/")
        .or_else(|| url.strip_prefix("https://bmclapi.bangbang93.com/maven/"))
        .or_else(|| url.strip_prefix("https://maven.minecraftforge.net/releases/"))
        .or_else(|| url.strip_prefix("https://maven.minecraftforge.net/"))
        .or_else(|| url.strip_prefix("https://maven.neoforged.net/releases/"))
        .or_else(|| url.strip_prefix("https://libraries.minecraft.net/"))?;
    Some(format!("{CERNET_BASE}{rest}"))
}

/// 文件类是否明确选了「官方优先」；加载器安装器等特殊链路据此调整候选顺序。
pub fn files_official_first() -> bool {
    matches!(mode_of(Category::Files), Mode::Official)
}

/// 生成某个 URL 的下载候选顺序。`use_mirror` 是调用方给的老式提示，
/// 只在「自动」模式下参与排序；分类选项优先。
pub fn candidates(url: &str, use_mirror: bool) -> Vec<String> {
    candidates_with(mode_of(category(url)), url, use_mirror)
}

fn candidates_with(mode: Mode, url: &str, use_mirror: bool) -> Vec<String> {
    let official = url.to_string();
    let mut mirrors: Vec<String> = Vec::new();
    if mode == Mode::Cernet {
        if let Some(cernet) = cernet_url(url) {
            mirrors.push(cernet);
        }
    }
    let rewritten = mirror::rewrite(url);
    if rewritten != official {
        mirrors.push(rewritten);
    }
    // 去重，同时排掉和官方地址相同的项
    let mut seen = std::collections::HashSet::new();
    mirrors.retain(|item| item != &official && seen.insert(item.clone()));

    let mirror_first = match mode {
        Mode::Official => false,
        Mode::Mirror | Mode::Cernet => true,
        Mode::Auto => use_mirror && mirror::prefer_mirror_first(url),
    };
    let mut out = Vec::new();
    if mirror_first {
        out.extend(mirrors);
        out.push(official);
    } else {
        out.push(official);
        out.extend(mirrors);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{candidates_with, category, Category, Mode};

    #[test]
    fn categorizes_common_urls() {
        assert_eq!(
            category("https://piston-meta.mojang.com/v1/packages/x/1.20.1.json"),
            Category::Metadata
        );
        assert_eq!(
            category("https://bmclapi2.bangbang93.com/mc/game/version_manifest_v2.json"),
            Category::Metadata
        );
        assert_eq!(
            category("https://maven.minecraftforge.net/releases/net/minecraftforge/forge/x.jar"),
            Category::Files
        );
        assert_eq!(
            category("https://edge.forgecdn.net/files/1/2/mod.jar"),
            Category::CurseForge
        );
        assert_eq!(
            category("https://cdn.modrinth.com/data/abc/versions/1/file.jar"),
            Category::Modrinth
        );
        assert_eq!(
            category("https://api.adoptium.net/v3/binary/latest/17/ga/windows/x64/jdk/hotspot/normal/eclipse"),
            Category::Files
        );
        assert_eq!(category("https://search.mcmod.cn/s?key=x"), Category::Other);
    }

    #[test]
    fn official_mode_puts_official_first() {
        let list = candidates_with(
            Mode::Official,
            "https://cdn.modrinth.com/data/abc/versions/1/file.jar",
            true,
        );
        assert!(list[0].starts_with("https://cdn.modrinth.com/"));
        assert!(list.iter().any(|item| item.contains("mcimirror")));
    }

    #[test]
    fn mirror_mode_puts_mirror_first() {
        let list = candidates_with(
            Mode::Mirror,
            "https://maven.minecraftforge.net/releases/net/minecraftforge/forge/x.jar",
            true,
        );
        assert!(list[0].contains("bmclapi"));
        assert!(list.last().unwrap().starts_with("https://maven.minecraftforge.net/"));
    }

    #[test]
    fn cernet_mode_leads_with_cernet_maven() {
        let list = candidates_with(
            Mode::Cernet,
            "https://maven.minecraftforge.net/releases/net/minecraftforge/forge/x.jar",
            true,
        );
        assert_eq!(
            list[0],
            "https://mirrors.cernet.edu.cn/bmclapi/net/minecraftforge/forge/x.jar"
        );
    }
}
