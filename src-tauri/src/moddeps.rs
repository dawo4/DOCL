use crate::config::Config;
use crate::download::{self, FileTask, ProgressState};
use crate::instance;
use crate::models::VersionDetail;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use tauri::AppHandle;
use zip::ZipArchive;

const BASE: &str = "https://api.modrinth.com/v2";
/// 一次最多自动补几个前置，避免模组写错依赖时无限拉取
const MAX_AUTO_DEPS: usize = 8;

/// 由加载器或 Java 提供的内置依赖，不需要也不应该下载。
const RUNTIME_IDS: &[&str] = &[
    "minecraft",
    "java",
    "forge",
    "neoforge",
    "fabricloader",
    "fabric-loader",
    "quilt_loader",
    "quilt-loader",
    "fabric",
    "quilt",
    "optifine",
    "minecraftforge",
];

/// 模组 ID 和 Modrinth 项目 slug 不一致时的手工对照，其余直接按 ID 查。
fn project_slug(id: &str) -> &str {
    match id {
        "cloth-config2" | "cloth_config" => "cloth-config",
        "architectury" => "architectury-api",
        "geckolib3" | "geckolib3q" | "geckolib4" => "geckolib",
        "kotlinforforge" => "kotlin-for-forge",
        "ftblibrary" | "ftb-library-forge" => "ftb-library",
        "citadel" => "citadel",
        _ => id,
    }
}

#[derive(Debug, Default, Serialize, Clone)]
pub struct DependencyReport {
    /// 自动补装成功的前置（展示名）
    pub installed: Vec<String>,
    /// 找到了但下载失败的
    pub failed: Vec<String>,
    /// 在 Modrinth 找不到对应项目、需要手动安装的
    pub unresolved: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ModImportOutcome {
    pub imported: u32,
    pub dependencies: DependencyReport,
}

#[derive(Debug, Clone)]
struct Requirement {
    id: String,
    mandatory: bool,
}

fn zip_text(archive: &mut ZipArchive<File>, name: &str) -> Option<String> {
    let mut file = archive.by_name(name).ok()?;
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

/// fabric.mod.json 里的版本约束可以是字符串，也可以是字符串数组。
fn json_ranges(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(_) => true,
        serde_json::Value::Array(items) => items.iter().all(|item| item.is_string()),
        _ => false,
    }
}

fn push_requirement(list: &mut Vec<Requirement>, id: &str, mandatory: bool) {
    let id = id.trim().to_lowercase();
    if id.is_empty() || !mandatory {
        return;
    }
    if RUNTIME_IDS.contains(&id.as_str()) {
        return;
    }
    // fabric-api 拆出来的模块（fabric-api-base 等）由 fabric-api 本体提供，只补本体
    if id.starts_with("fabric-") && id != "fabric-api" {
        return;
    }
    if list.iter().any(|item| item.id == id) {
        return;
    }
    list.push(Requirement { id, mandatory });
}

fn parse_fabric(text: &str, list: &mut Vec<Requirement>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    let Some(depends) = value.get("depends").and_then(|item| item.as_object()) else {
        return;
    };
    for (id, range) in depends {
        if json_ranges(range) {
            push_requirement(list, id, true);
        }
    }
}

fn parse_quilt(text: &str, list: &mut Vec<Requirement>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    let Some(depends) = value
        .get("quilt_loader")
        .and_then(|loader| loader.get("depends"))
        .and_then(|item| item.as_array())
    else {
        return;
    };
    for item in depends {
        if let Some(id) = item.as_str() {
            push_requirement(list, id, true);
        } else if let Some(object) = item.as_object() {
            let id = object.get("id").and_then(|value| value.as_str());
            let optional = object
                .get("optional")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            if let Some(id) = id {
                push_requirement(list, id, !optional);
            }
        }
    }
}

fn toml_field(line: &str, key: &str) -> Option<String> {
    let rest = line.strip_prefix(key)?.trim_start();
    let rest = rest.strip_prefix('=')?.trim();
    Some(rest.trim_matches('"').trim_matches('\'').to_string())
}

/// mods.toml / neoforge.mods.toml 的前置写在 [[dependencies.<modid>]] 段里。
fn parse_forge(text: &str, list: &mut Vec<Requirement>) {
    let mut in_section = false;
    let mut current: Option<(String, bool)> = None;

    fn flush(current: &mut Option<(String, bool)>, list: &mut Vec<Requirement>) {
        if let Some((id, mandatory)) = current.take() {
            push_requirement(list, &id, mandatory);
        }
    }

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("[[") {
            flush(&mut current, list);
            let section = trimmed.replace(' ', "");
            in_section = section.starts_with("[[dependencies.") && section.ends_with("]]");
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some(id) = toml_field(trimmed, "modId") {
            // 新的一段开始，先把上一段收尾
            flush(&mut current, list);
            current = Some((id, true));
            continue;
        }
        if let Some(value) = toml_field(trimmed, "mandatory") {
            if let Some((_, mandatory)) = current.as_mut() {
                *mandatory = value.eq_ignore_ascii_case("true");
            }
        }
    }
    flush(&mut current, list);
}

/// 按实例加载器只读对应格式的元数据。
/// Forge 整合包里常混有多加载器 jar，它们也带 fabric.mod.json，
/// 读它会把 fabric-api 这类在 Forge 下无关的前置算成缺失。
fn read_requirements(path: &Path, loader: &str) -> Vec<Requirement> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let Ok(mut archive) = ZipArchive::new(file) else {
        return Vec::new();
    };
    let mut list = Vec::new();

    match loader {
        "fabric" => {
            if let Some(text) = zip_text(&mut archive, "fabric.mod.json") {
                parse_fabric(&text, &mut list);
            }
        }
        "quilt" => {
            if let Some(text) = zip_text(&mut archive, "quilt.mod.json") {
                parse_quilt(&text, &mut list);
            }
            // Quilt 也兼容 fabric 模组
            if list.is_empty() {
                if let Some(text) = zip_text(&mut archive, "fabric.mod.json") {
                    parse_fabric(&text, &mut list);
                }
            }
        }
        _ => {
            for name in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
                if let Some(text) = zip_text(&mut archive, name) {
                    parse_forge(&text, &mut list);
                }
            }
        }
    }
    list
}

/// 读取 jar 自身的 mod id，用来判断前置是否已经装过。
fn jar_mod_id(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut archive = ZipArchive::new(file).ok()?;
    if let Some(text) = zip_text(&mut archive, "fabric.mod.json") {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(id) = value.get("id").and_then(|item| item.as_str()) {
                return Some(id.to_lowercase());
            }
        }
    }
    if let Some(text) = zip_text(&mut archive, "quilt.mod.json") {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(id) = value
                .get("quilt_loader")
                .and_then(|item| item.get("id"))
                .and_then(|item| item.as_str())
            {
                return Some(id.to_lowercase());
            }
        }
    }
    for name in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
        if let Some(text) = zip_text(&mut archive, name) {
            let mut in_mods = false;
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("[[") {
                    in_mods = trimmed.replace(' ', "") == "[[mods]]";
                    continue;
                }
                if in_mods {
                    if let Some(id) = toml_field(trimmed, "modId") {
                        return Some(id.to_lowercase());
                    }
                }
            }
        }
    }
    None
}

/// 已安装模组提供的 ID 集合。
pub fn installed_ids(mods_dir: &Path) -> HashSet<String> {
    let mut set = HashSet::new();
    let Ok(entries) = std::fs::read_dir(mods_dir) else {
        return set;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if !name.ends_with(".jar") {
            continue;
        }
        if let Some(id) = jar_mod_id(&path) {
            set.insert(id);
        }
    }
    set
}

/// 同一个 mod id 装了多份 jar（作者换文件名、手动复制都会出现）。
/// 返回 (mod id, 该 id 下全部 jar 路径)，只包含多于一份的。
pub fn duplicate_mods(mods_dir: &Path) -> Vec<(String, Vec<std::path::PathBuf>)> {
    let mut map: std::collections::HashMap<String, Vec<std::path::PathBuf>> =
        std::collections::HashMap::new();
    let Ok(entries) = std::fs::read_dir(mods_dir) else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if !name.ends_with(".jar") {
            continue;
        }
        if let Some(id) = jar_mod_id(&path) {
            map.entry(id).or_default().push(path);
        }
    }
    let mut list: Vec<(String, Vec<std::path::PathBuf>)> = map
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .collect();
    for (_, files) in list.iter_mut() {
        files.sort();
    }
    list.sort_by(|a, b| a.0.cmp(&b.0));
    list
}

/// 已装 jar 声明为必需、但实例里找不到的前置 mod id（去重、排除加载器内置）。
pub fn missing_requirements(mods_dir: &Path, loader: &str) -> Vec<String> {
    let present = installed_ids(mods_dir);
    let mut missing: Vec<String> = Vec::new();
    let Ok(entries) = std::fs::read_dir(mods_dir) else {
        return missing;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if !name.ends_with(".jar") {
            continue;
        }
        for requirement in read_requirements(&path, loader)
            .into_iter()
            .filter(|item| item.mandatory)
        {
            if present.contains(&requirement.id) || missing.contains(&requirement.id) {
                continue;
            }
            missing.push(requirement.id);
        }
    }
    missing
}

/// 从版本 JSON 判断实例的加载器与游戏版本。
fn instance_env(cfg: &Config, version_id: &str) -> (String, String) {
    let json = cfg
        .game_root()
        .join("versions")
        .join(version_id)
        .join(format!("{version_id}.json"));
    let mut game_version = version_id.to_string();
    if let Ok(bytes) = std::fs::read(&json) {
        if let Ok(detail) = serde_json::from_slice::<VersionDetail>(&bytes) {
            if let Some(parent) = detail.inherits_from {
                game_version = parent;
            }
        }
    }
    let id = version_id.to_lowercase();
    let loader = if id.starts_with("fabric-loader") {
        "fabric"
    } else if id.starts_with("quilt-loader") {
        "quilt"
    } else if id.contains("neoforge") {
        "neoforge"
    } else if id.contains("forge") {
        "forge"
    } else {
        "vanilla"
    };
    (loader.to_string(), game_version)
}

#[derive(Debug, Deserialize)]
struct MrVersion {
    #[serde(default)]
    version_number: String,
    #[serde(default)]
    files: Vec<MrFile>,
}

#[derive(Debug, Deserialize)]
struct MrFile {
    #[serde(default)]
    url: String,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    primary: bool,
    #[serde(default)]
    size: u64,
}

struct Resolved {
    name: String,
    url: String,
    file_name: String,
    size: u64,
}

/// 在 Modrinth 上找某个 mod id 对应当前加载器 / 游戏版本的最新文件。
async fn resolve(
    client: &reqwest::Client,
    id: &str,
    loader: &str,
    game_version: &str,
) -> Option<Resolved> {
    let slug = project_slug(id);
    let mut url = reqwest::Url::parse(&format!("{BASE}/project/{slug}/version")).ok()?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("game_versions", &format!("[\"{game_version}\"]"));
        let tag = match loader {
            "fabric" => "fabric",
            "quilt" => "quilt",
            "forge" => "forge",
            "neoforge" => "neoforge",
            _ => "",
        };
        if !tag.is_empty() {
            pairs.append_pair("loaders", &format!("[\"{tag}\"]"));
        }
    }
    let versions: Vec<MrVersion> = download::get_json(client, url.as_str(), false)
        .await
        .ok()?;
    let version = versions.first()?;
    let file = version
        .files
        .iter()
        .find(|file| file.primary)
        .or_else(|| version.files.first())?;
    if file.url.is_empty() {
        return None;
    }
    let file_name = Path::new(&file.filename)
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| format!("{id}.jar"));
    Some(Resolved {
        name: if version.version_number.is_empty() {
            id.to_string()
        } else {
            format!("{id} {}", version.version_number)
        },
        url: file.url.clone(),
        file_name,
        size: file.size,
    })
}

/// 安装完一个模组后，补齐它在元数据里声明但当前实例缺少的前置。
pub async fn install_missing(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    instance_id: &str,
    jar: &Path,
    progress: Arc<ProgressState>,
) -> DependencyReport {
    let mut report = DependencyReport::default();
    let Ok(mods_dir) = instance::content_dir(cfg, instance_id, "mods") else {
        return report;
    };
    let mut present = installed_ids(&mods_dir);
    if let Some(id) = jar_mod_id(jar) {
        present.insert(id);
    }

    let (loader, game_version) = instance_env(cfg, instance_id);
    let requirements = read_requirements(jar, &loader);
    if requirements.is_empty() {
        return report;
    }

    let mut resolved = Vec::new();
    for requirement in requirements.into_iter().filter(|item| item.mandatory) {
        if resolved.len() >= MAX_AUTO_DEPS {
            break;
        }
        if present.contains(&requirement.id) {
            continue;
        }
        let Some(item) = resolve(client, &requirement.id, &loader, &game_version).await else {
            report.unresolved.push(requirement.id);
            continue;
        };
        present.insert(requirement.id);
        resolved.push(item);
    }

    for item in resolved {
        if progress.is_cancelled() {
            break;
        }
        progress.set_message(format!("补齐前置 {}…", item.name));
        download::emit_now(app, &progress);
        let task = FileTask {
            url: item.url,
            dest: mods_dir.join(&item.file_name),
            sha1: None,
            size: item.size,
            label: item.file_name.clone(),
        };
        match download::run_tasks(app, client, vec![task], 1, cfg.use_mirror(), progress.clone()).await {
            Ok(()) => report.installed.push(item.name),
            Err(_) => report.failed.push(item.name),
        }
    }

    report
}

/// 按 mod id 列表补齐前置（一键修复用）。
/// 只有在 Modrinth 上能明确解析到当前加载器 / 游戏版本的文件才会下载，其余记为未解决。
pub async fn install_requirements(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    instance_id: &str,
    ids: &[String],
    progress: Arc<ProgressState>,
) -> DependencyReport {
    let mut report = DependencyReport::default();
    if ids.is_empty() {
        return report;
    }
    let Ok(mods_dir) = instance::content_dir(cfg, instance_id, "mods") else {
        return report;
    };
    let (loader, game_version) = instance_env(cfg, instance_id);

    let mut resolved = Vec::new();
    for id in ids {
        if resolved.len() >= MAX_AUTO_DEPS {
            break;
        }
        let Some(item) = resolve(client, id, &loader, &game_version).await else {
            report.unresolved.push(id.clone());
            continue;
        };
        resolved.push(item);
    }

    for item in resolved {
        if progress.is_cancelled() {
            break;
        }
        progress.set_message(format!("补齐前置 {}…", item.name));
        download::emit_now(app, &progress);
        let task = FileTask {
            url: item.url,
            dest: mods_dir.join(&item.file_name),
            sha1: None,
            size: item.size,
            label: item.file_name.clone(),
        };
        match download::run_tasks(app, client, vec![task], 1, cfg.use_mirror(), progress.clone()).await {
            Ok(()) => report.installed.push(item.name),
            Err(_) => report.failed.push(item.name),
        }
    }

    report
}

/// 光影加载器检测结果：有没有装、装的是哪个、没装时推荐哪个
#[derive(Debug, Clone, Serialize)]
pub struct ShaderSupport {
    pub loader: String,
    pub game_version: String,
    /// 已检测到的光影加载器；空表示没有
    pub provider: Option<String>,
    /// 推荐安装的 Modrinth 项目 id
    pub recommend: Option<String>,
    pub recommend_label: Option<String>,
}

/// 只在本地扫描 mods 目录，不联网
pub fn shader_support(cfg: &Config, version_id: &str) -> ShaderSupport {
    let (loader, game_version) = instance_env(cfg, version_id);
    let mut provider: Option<&str> = None;
    if let Ok(dir) = instance::content_dir(cfg, version_id, "mods") {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if !name.ends_with(".jar") {
                continue;
            }
            if name.contains("iris") {
                provider = Some("Iris");
                break;
            }
            if name.contains("oculus") {
                provider = Some("Oculus");
            } else if provider.is_none() && name.contains("optifine") {
                provider = Some("OptiFine");
            }
        }
    }
    let (recommend, recommend_label) = match loader.as_str() {
        "fabric" | "quilt" | "neoforge" => (Some("iris".to_string()), Some("Iris".to_string())),
        "forge" => (Some("oculus".to_string()), Some("Oculus".to_string())),
        _ => (None, None),
    };
    ShaderSupport {
        loader,
        game_version,
        provider: provider.map(str::to_string),
        recommend,
        recommend_label,
    }
}

/// 给实例装上一个推荐的光影加载器，返回装上的名字
pub async fn install_shader_loader(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    instance_id: &str,
    progress: Arc<ProgressState>,
) -> anyhow::Result<String> {
    let support = shader_support(cfg, instance_id);
    if let Some(provider) = support.provider.clone() {
        return Ok(provider);
    }
    let Some(project) = support.recommend.clone() else {
        anyhow::bail!("原版实例没有光影支持，建议先安装 Fabric + Iris，或改装 OptiFine");
    };
    let Some(item) = resolve(client, &project, &support.loader, &support.game_version).await
    else {
        anyhow::bail!(
            "没找到与 {} {} 匹配的光影加载器，请手动下载",
            support.loader,
            support.game_version
        );
    };
    let mods_dir = instance::content_dir(cfg, instance_id, "mods")?;
    progress.set_message(format!("正在安装 {}…", item.name));
    download::emit_now(app, &progress);
    let task = FileTask {
        url: item.url,
        dest: mods_dir.join(&item.file_name),
        sha1: None,
        size: item.size,
        label: item.file_name.clone(),
    };
    download::run_tasks(app, client, vec![task], 1, cfg.use_mirror(), progress.clone()).await?;
    Ok(item.name)
}
