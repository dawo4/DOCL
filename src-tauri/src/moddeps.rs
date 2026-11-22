use crate::config::Config;
use crate::download::{self, FileTask, ProgressState};
use crate::instance;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
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

/// 从完整继承链推断实例的加载器与游戏版本。
/// 不能只看实例 id：整合包实例的名字通常不含加载器，直接猜会把 Forge 包当成原版，
/// 之后不带 loader 去 Modrinth 查前置就可能下到别的加载器的 jar。
fn instance_env(cfg: &Config, version_id: &str) -> (String, String) {
    crate::install::instance_context(cfg, version_id)
        .unwrap_or_else(|_| (version_id.to_string(), "vanilla".to_string()))
}

#[derive(Debug, Deserialize)]
struct MrVersion {
    #[serde(default)]
    version_number: String,
    #[serde(default)]
    files: Vec<MrFile>,
    #[serde(default)]
    project_id: String,
    #[serde(default)]
    dependencies: Vec<MrDep>,
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

#[derive(Debug, Clone, Deserialize)]
struct MrDep {
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    version_id: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    dependency_type: String,
}

struct Resolved {
    name: String,
    url: String,
    file_name: String,
    size: u64,
}

fn sha1_of(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = Sha1::new();
    hasher.update(&bytes);
    Some(format!("{:x}", hasher.finalize()))
}

/// 从 jar 的 fabric / quilt / forge 元数据里找 Modrinth 项目链接。
fn modrinth_slug_from_jar(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut archive = ZipArchive::new(file).ok()?;
    for name in [
        "fabric.mod.json",
        "quilt.mod.json",
        "META-INF/neoforge.mods.toml",
        "META-INF/mods.toml",
    ] {
        if let Some(text) = zip_text(&mut archive, name) {
            if let Some(slug) = modrinth_slug_from_text(&text) {
                return Some(slug);
            }
        }
    }
    None
}

fn modrinth_slug_from_text(text: &str) -> Option<String> {
    const MARKER: &str = "modrinth.com/mod/";
    let mut rest = text;
    while let Some(index) = rest.find(MARKER) {
        let tail = &rest[index + MARKER.len()..];
        let slug: String = tail
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if !slug.is_empty() {
            return Some(slug);
        }
        rest = tail;
    }
    None
}

/// 只查明确加载器的版本列表；不带 loaders 过滤会拿到别的加载器的 jar。
fn version_query(project: &str, loader: &str, game_version: &str) -> Option<reqwest::Url> {
    let tag = match loader {
        "fabric" => "fabric",
        "quilt" => "quilt",
        "forge" => "forge",
        "neoforge" => "neoforge",
        _ => return None,
    };
    let mut url = reqwest::Url::parse(&format!("{BASE}/project/{project}/version")).ok()?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("game_versions", &format!("[\"{game_version}\"]"));
        pairs.append_pair("loaders", &format!("[\"{tag}\"]"));
    }
    Some(url)
}

fn resolved_from(version: &MrVersion) -> Option<Resolved> {
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
        .unwrap_or_else(|| format!("{}.jar", version.project_id));
    Some(Resolved {
        name: if file.filename.is_empty() {
            file_name.clone()
        } else {
            file.filename.clone()
        },
        url: file.url.clone(),
        file_name,
        size: file.size,
    })
}

/// 在 Modrinth 上找某个 mod id 对应当前加载器 / 游戏版本的最新文件。
async fn resolve(
    client: &reqwest::Client,
    id: &str,
    loader: &str,
    game_version: &str,
) -> Option<Resolved> {
    let url = version_query(project_slug(id), loader, game_version)?;
    let versions: Vec<MrVersion> = download::get_json(client, url.as_str(), false)
        .await
        .ok()?;
    let version = versions.first()?;
    let mut item = resolved_from(version)?;
    item.name = if version.version_number.is_empty() {
        id.to_string()
    } else {
        format!("{id} {}", version.version_number)
    };
    Some(item)
}

/// 按 Modrinth 项目 / 版本精确解析一个前置。
async fn resolve_dependency(
    client: &reqwest::Client,
    project: Option<&str>,
    version_id: Option<&str>,
    loader: &str,
    game_version: &str,
) -> Option<Resolved> {
    if let Some(id) = version_id.filter(|value| !value.is_empty()) {
        let url = format!("{BASE}/version/{id}");
        let version: MrVersion = download::get_json(client, &url, false).await.ok()?;
        return resolved_from(&version);
    }
    let project = project.filter(|value| !value.is_empty())?;
    let url = version_query(project, loader, game_version)?;
    let versions: Vec<MrVersion> = download::get_json(client, url.as_str(), false)
        .await
        .ok()?;
    resolved_from(versions.first()?)
}

/// 优先用 jar 的 SHA1 精确命中 Modrinth 官方依赖；
/// 命中不了再读元数据里的项目链接，取该项目当前匹配版本的依赖列表。
async fn official_dependencies(
    client: &reqwest::Client,
    jar: &Path,
    loader: &str,
    game_version: &str,
) -> Vec<MrDep> {
    if let Some(hash) = sha1_of(jar) {
        let url = format!("{BASE}/version_file/{hash}");
        if let Ok(version) = download::get_json::<MrVersion>(client, &url, false).await {
            // 命中就以官方列表为准：即使没有 required，也说明这版确实不需要额外前置
            return version.dependencies;
        }
    }
    let Some(slug) = modrinth_slug_from_jar(jar) else {
        return Vec::new();
    };
    let Some(url) = version_query(&slug, loader, game_version) else {
        return Vec::new();
    };
    let Ok(versions) = download::get_json::<Vec<MrVersion>>(client, url.as_str(), false).await
    else {
        return Vec::new();
    };
    versions
        .first()
        .map(|version| version.dependencies.clone())
        .unwrap_or_default()
}

/// 按 Modrinth 官方依赖列表补前置。
/// 先下到临时目录读出 mod id，已经装过的直接丢掉，避免同一前置出现两份。
#[allow(clippy::too_many_arguments)]
async fn install_official(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    mods_dir: &Path,
    dependencies: Vec<MrDep>,
    loader: &str,
    game_version: &str,
    present: &mut HashSet<String>,
    progress: Arc<ProgressState>,
    report: &mut DependencyReport,
) {
    let mut seen: HashSet<String> = HashSet::new();
    let temp_dir = crate::config::data_dir().join("tmp");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let mut count = 0usize;

    for dep in dependencies {
        if count >= MAX_AUTO_DEPS || progress.is_cancelled() {
            break;
        }
        if dep.dependency_type != "required" {
            continue;
        }
        let key = dep
            .project_id
            .clone()
            .or_else(|| dep.version_id.clone())
            .unwrap_or_default();
        if key.is_empty() || !seen.insert(key.clone()) {
            continue;
        }
        // 报告的失败项尽量用文件名，project_id 是一串随机字符，用户看不懂
        let label = dep.file_name.clone().unwrap_or_else(|| key.clone());
        let Some(item) = resolve_dependency(
            client,
            dep.project_id.as_deref(),
            dep.version_id.as_deref(),
            loader,
            game_version,
        )
        .await
        else {
            report.unresolved.push(label);
            continue;
        };
        count += 1;

        if std::fs::create_dir_all(&temp_dir).is_err() {
            report.failed.push(item.name);
            continue;
        }
        let temp = temp_dir.join(format!("dep-{}-{}-{}", std::process::id(), nonce, count));
        let task = FileTask {
            url: item.url.clone(),
            dest: temp.clone(),
            sha1: None,
            size: item.size,
            label: item.file_name.clone(),
        };
        progress.set_message(format!("补齐前置 {}…", item.name));
        download::emit_now(app, &progress);
        if download::run_tasks(app, client, vec![task], 1, cfg.use_mirror(), progress.clone())
            .await
            .is_err()
        {
            let _ = std::fs::remove_file(&temp);
            report.failed.push(item.name);
            continue;
        }
        if let Some(id) = jar_mod_id(&temp) {
            if present.contains(&id) {
                let _ = std::fs::remove_file(&temp);
                continue;
            }
            present.insert(id);
        }
        let copied = std::fs::copy(&temp, mods_dir.join(&item.file_name)).is_ok();
        let _ = std::fs::remove_file(&temp);
        if copied {
            report.installed.push(item.name);
        } else {
            report.failed.push(item.name);
        }
    }
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
    // 原版实例没有加载器，前置装了也不会被加载
    if loader == "vanilla" {
        return report;
    }

    // 元数据里声明的前置都齐了就不用再联网：批量导入整合包时能省掉大量请求
    let requirements = read_requirements(jar, &loader);
    if requirements.is_empty()
        || !requirements
            .iter()
            .any(|item| item.mandatory && !present.contains(&item.id))
    {
        return report;
    }

    // 有缺的才查 Modrinth 官方依赖：jar 的 SHA1 精确命中版本，或者读元数据里的项目链接。
    // 拿不到官方列表时再退回解析 jar 元数据 + mod id 对照表。
    let official = official_dependencies(client, jar, &loader, &game_version).await;
    if !official.is_empty() {
        install_official(
            app,
            client,
            cfg,
            &mods_dir,
            official,
            &loader,
            &game_version,
            &mut present,
            progress,
            &mut report,
        )
        .await;
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
/// 先让声明了这些前置的 jar 走 Modrinth 官方依赖（SHA1 / 项目链接），
/// 剩下的再按 mod id 解析；只有明确匹配当前加载器 / 游戏版本才会下载。
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
    if loader == "vanilla" {
        report.unresolved = ids.to_vec();
        return report;
    }

    // 找出声明了这些缺失前置的 jar，用它们的 SHA1 拿官方依赖列表。
    // 一次修复最多查 24 个 jar，避免整合包场景打出过多请求。
    let mut declaring: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&mods_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if !path.is_file() || !name.ends_with(".jar") {
                continue;
            }
            if read_requirements(&path, &loader)
                .into_iter()
                .any(|item| item.mandatory && ids.contains(&item.id))
            {
                declaring.push(path);
            }
        }
    }
    let mut present = installed_ids(&mods_dir);
    for jar in declaring.iter().take(24) {
        let official = official_dependencies(client, jar, &loader, &game_version).await;
        if official.is_empty() {
            continue;
        }
        install_official(
            app,
            client,
            cfg,
            &mods_dir,
            official,
            &loader,
            &game_version,
            &mut present,
            progress.clone(),
            &mut report,
        )
        .await;
    }

    // 官方依赖补完后重新扫描，剩下的 mod id 再走原来的解析逻辑
    present = installed_ids(&mods_dir);
    let remaining: Vec<String> = ids
        .iter()
        .filter(|id| !present.contains(*id))
        .cloned()
        .collect();

    let mut resolved = Vec::new();
    for id in &remaining {
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

/// 安装一个已知的修复类 Mod：按实例的加载器与游戏版本匹配最新构建。
pub async fn install_project(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    instance_id: &str,
    slug: &str,
    label: &str,
    progress: Arc<ProgressState>,
) -> anyhow::Result<String> {
    let (loader, game_version) = instance_env(cfg, instance_id);
    if loader.is_empty() {
        anyhow::bail!("实例加载器未知，无法自动安装 {label}");
    }
    let Some(item) = resolve(client, slug, &loader, &game_version).await else {
        anyhow::bail!("没找到与 {game_version} {loader} 匹配的 {label}");
    };
    let mods_dir = instance::content_dir(cfg, instance_id, "mods")?;
    let dest = mods_dir.join(&item.file_name);
    if dest.exists() {
        return Ok(item.name);
    }
    progress.set_message(format!("正在安装 {label}…"));
    download::emit_now(app, &progress);
    let task = FileTask {
        url: item.url,
        dest,
        sha1: None,
        size: item.size,
        label: item.file_name.clone(),
    };
    download::run_tasks(app, client, vec![task], 1, cfg.use_mirror(), progress.clone()).await?;
    Ok(item.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 元数据里的 Modrinth 链接要能解析出 slug，供官方依赖查询兜底
    #[test]
    fn finds_modrinth_slug_in_metadata() {
        let fabric = r#"{"contact":{"homepage":"https://modrinth.com/mod/cloth-config","sources":"https://github.com/shedaniel/cloth-config"}}"#;
        assert_eq!(modrinth_slug_from_text(fabric).as_deref(), Some("cloth-config"));

        let forge = r#"displayURL="https://modrinth.com/mod/jei?tab=versions""#;
        assert_eq!(modrinth_slug_from_text(forge).as_deref(), Some("jei"));

        assert_eq!(modrinth_slug_from_text("https://example.com/mod/foo"), None);
        assert_eq!(modrinth_slug_from_text(""), None);
    }
}
