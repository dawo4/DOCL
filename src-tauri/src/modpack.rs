use crate::config::Config;
use crate::curseforge;
use crate::download::{self, FileTask, ProgressState};
use crate::install;
use crate::instance::{self, InstanceMeta};
use anyhow::{anyhow, bail, Context, Result};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use zip::ZipArchive;

/* ---------- 预览 ---------- */

#[derive(Debug, Clone, Serialize)]
pub struct PackFilePreview {
    pub path: String,
    pub size: u64,
    pub url: Option<String>,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackPreview {
    pub format: String,
    pub name: String,
    pub version: String,
    pub game_version: String,
    pub loader: String,
    pub loader_version: Option<String>,
    pub detected_parent: Option<String>,
    pub file_count: u32,
    pub download_bytes: u64,
    pub override_bytes: u64,
    pub files: Vec<PackFilePreview>,
    pub warnings: Vec<String>,
    pub suggested_id: String,
    /// 服务端安装时会跳过的仅客户端文件数（Modrinth env.server）
    pub server_skips: u32,
    /// 压缩包特征更像服务端整合包（libraries/server.properties，且没有客户端 versions）
    pub server_like: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImportOptions {
    #[serde(default)]
    pub instance_name: Option<String>,
    #[serde(default)]
    pub instance_id: Option<String>,
    #[serde(default)]
    pub loader_version: Option<String>,
    #[serde(default)]
    pub parent_version: Option<String>,
    #[serde(default)]
    pub install_optional: bool,
    /// 安装为服务端：跳过仅客户端可用的文件
    #[serde(default)]
    pub server: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    pub instance_id: String,
    pub display_name: String,
    pub installed_files: u32,
    pub failed: Vec<String>,
}

/* ---------- CurseForge manifest ---------- */

#[derive(Debug, Clone, Deserialize)]
struct CfManifest {
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    overrides: String,
    minecraft: CfManifestMinecraft,
    #[serde(default)]
    files: Vec<CfManifestFile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfManifestMinecraft {
    version: String,
    #[serde(default)]
    mod_loaders: Vec<CfManifestLoader>,
}

#[derive(Debug, Clone, Deserialize)]
struct CfManifestLoader {
    #[serde(default)]
    id: String,
    #[serde(default)]
    primary: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct CfManifestFile {
    #[serde(rename = "projectID", alias = "projectId")]
    project_id: i64,
    #[serde(rename = "fileID", alias = "fileId")]
    file_id: i64,
    #[serde(default)]
    required: bool,
}

/* ---------- Modrinth index ---------- */

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MrIndex {
    #[serde(default)]
    name: String,
    #[serde(default)]
    version_id: String,
    #[serde(default)]
    dependencies: HashMap<String, String>,
    #[serde(default)]
    files: Vec<MrIndexFile>,
}

#[derive(Debug, Clone, Deserialize)]
struct MrIndexFile {
    #[serde(default)]
    path: String,
    #[serde(default)]
    downloads: Vec<String>,
    #[serde(default)]
    hashes: MrHashes,
    #[serde(default)]
    env: Option<MrEnv>,
    /// 用于下载后校验长度，索引里缺省时为 0（不校验）
    #[serde(default, rename = "fileSize")]
    file_size: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MrHashes {
    #[serde(default)]
    sha1: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MrEnv {
    #[serde(default)]
    client: String,
    #[serde(default)]
    server: String,
}

/* ---------- 内部解析结果 ---------- */

enum SourceData {
    Curse {
        game_version: String,
        loader: String,
        loader_version: Option<String>,
        overrides: String,
        files: Vec<(i64, i64, bool)>,
    },
    Modrinth {
        game_version: String,
        loader: String,
        loader_version: Option<String>,
        files: Vec<(String, String, Option<String>, u64, bool)>,
    },
    Generic {
        prefix: String,
        detected_parent: Option<String>,
    },
}

struct ParsedPack {
    preview: PackPreview,
    source: SourceData,
}

fn open_zip(path: &str) -> Result<ZipArchive<std::fs::File>> {
    let file = std::fs::File::open(path).with_context(|| format!("无法打开整合包：{path}"))?;
    ZipArchive::new(file).context("压缩包损坏或格式不支持")
}

fn zip_names(archive: &mut ZipArchive<std::fs::File>) -> Vec<String> {
    let mut names = Vec::new();
    for index in 0..archive.len() {
        if let Ok(entry) = archive.by_index(index) {
            if !entry.is_dir() {
                names.push(entry.name().to_string());
            }
        }
    }
    names
}

fn read_entry(archive: &mut ZipArchive<std::fs::File>, name: &str) -> Result<Vec<u8>> {
    let mut entry = archive.by_name(name)?;
    let mut buffer = Vec::new();
    entry.read_to_end(&mut buffer)?;
    Ok(buffer)
}

fn find_meta(names: &[String], file: &str) -> Option<String> {
    names.iter().find(|name| *name == file).cloned().or_else(|| {
        names
            .iter()
            .find(|name| name.ends_with(&format!("/{file}")) && name.matches('/').count() == 1)
            .cloned()
    })
}

fn slugify(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else if ch.is_whitespace() || ch == '/' || ch == '\\' {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches(['-', '.']).to_string();
    if trimmed.is_empty() {
        "pack".into()
    } else {
        trimmed.chars().take(48).collect()
    }
}

fn detect_loader(id: &str) -> (String, Option<String>) {
    let lower = id.to_lowercase();
    for name in ["neoforge", "forge", "quilt", "fabric"] {
        if lower.starts_with(name) {
            let version = id.split_once('-').map(|(_, rest)| rest.to_string());
            return (name.to_string(), version);
        }
    }
    ("vanilla".into(), None)
}

fn parse_pack(path: &str, server: bool) -> Result<ParsedPack> {
    let mut archive = open_zip(path)?;
    let names = zip_names(&mut archive);
    let zip_name = Path::new(path)
        .file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "整合包".into());

    if let Some(manifest_name) = find_meta(&names, "manifest.json") {
        let bytes = read_entry(&mut archive, &manifest_name)?;
        let manifest: CfManifest =
            serde_json::from_slice(&bytes).context("无法解析 CurseForge manifest.json")?;
        let loader_entry = manifest
            .minecraft
            .mod_loaders
            .iter()
            .find(|loader| loader.primary)
            .or_else(|| manifest.minecraft.mod_loaders.first());
        let (loader, loader_version) = loader_entry
            .map(|entry| detect_loader(&entry.id))
            .unwrap_or_else(|| ("vanilla".into(), None));
        let name = if manifest.name.is_empty() {
            zip_name.clone()
        } else {
            manifest.name.clone()
        };
        let version = manifest.version.clone();
        let suggested_id = slugify(&format!("{name}-{version}"));
        let files: Vec<PackFilePreview> = manifest
            .files
            .iter()
            .take(40)
            .map(|file| PackFilePreview {
                path: format!("mods/#{}-#{}", file.project_id, file.file_id),
                size: 0,
                url: None,
                required: file.required,
            })
            .collect();
        let mut warnings = Vec::new();
        if loader_version.is_none() && loader != "vanilla" {
            warnings.push("清单没有标明加载器版本，需要手动选择".into());
        }
        if manifest.files.is_empty() {
            warnings.push("清单里没有可下载文件".into());
        }
        return Ok(ParsedPack {
            preview: PackPreview {
                format: "curseforge".into(),
                name,
                version,
                game_version: manifest.minecraft.version.clone(),
                loader: loader.clone(),
                loader_version: loader_version.clone(),
                detected_parent: None,
                file_count: manifest.files.len() as u32,
                download_bytes: 0,
                override_bytes: 0,
                files,
                warnings,
                suggested_id,
                server_skips: 0,
                server_like: false,
            },
            source: SourceData::Curse {
                game_version: manifest.minecraft.version,
                loader,
                loader_version,
                overrides: if manifest.overrides.is_empty() {
                    "overrides".into()
                } else {
                    manifest.overrides
                },
                files: manifest
                    .files
                    .into_iter()
                    .map(|file| (file.project_id, file.file_id, file.required))
                    .collect(),
            },
        });
    }

    if let Some(index_name) = find_meta(&names, "modrinth.index.json") {
        let bytes = read_entry(&mut archive, &index_name)?;
        let index: MrIndex =
            serde_json::from_slice(&bytes).context("无法解析 modrinth.index.json")?;
        let game_version = index.dependencies.get("minecraft").cloned().unwrap_or_default();
        let (loader, loader_version) = ["neoforge", "forge", "quilt-loader", "fabric-loader"]
            .iter()
            .find_map(|key| {
                index
                    .dependencies
                    .get(*key)
                    .map(|version| (key.replace("-loader", ""), Some(version.clone())))
            })
            .unwrap_or_else(|| ("vanilla".into(), None));
        let name = if index.name.is_empty() {
            zip_name.clone()
        } else {
            index.name.clone()
        };
        let mut warnings = Vec::new();
        if game_version.is_empty() {
            warnings.push("缺少 Minecraft 版本信息".into());
        }
        let mut files = Vec::new();
        let mut tasks = Vec::new();
        // 服务端安装时会被跳过的仅客户端文件数，预览里提示给用户
        let server_skips = index
            .files
            .iter()
            .filter(|file| {
                file.env
                    .as_ref()
                    .map(|env| env.server == "unsupported")
                    .unwrap_or(false)
            })
            .count() as u32;
        for file in index.files {
            let env_client = file.env.as_ref().map(|env| env.client.clone()).unwrap_or_default();
            let env_server = file.env.as_ref().map(|env| env.server.clone()).unwrap_or_default();
            // 按目标端跳过不可用文件：客户端装跳 client-only，服务端装跳 server-only
            let unsupported = if server {
                env_server == "unsupported"
            } else {
                env_client == "unsupported"
            };
            if unsupported {
                continue;
            }
            let url = file.downloads.first().cloned().unwrap_or_default();
            let optional = if server {
                env_server == "optional"
            } else {
                env_client == "optional"
            };
            if files.len() < 40 {
                files.push(PackFilePreview {
                    path: file.path.clone(),
                    size: file.file_size,
                    url: if url.is_empty() { None } else { Some(url.clone()) },
                    required: !optional,
                });
            }
            tasks.push((
                file.path,
                url,
                if file.hashes.sha1.is_empty() {
                    None
                } else {
                    Some(file.hashes.sha1)
                },
                file.file_size,
                optional,
            ));
        }
        let suggested_id = slugify(&format!("{name}-{}", index.version_id));
        return Ok(ParsedPack {
            preview: PackPreview {
                format: "mrpack".into(),
                name,
                version: index.version_id,
                game_version: game_version.clone(),
                loader: loader.clone(),
                loader_version: loader_version.clone(),
                detected_parent: None,
                file_count: tasks.len() as u32,
                download_bytes: 0,
                override_bytes: 0,
                files,
                warnings,
                suggested_id,
                server_skips,
                server_like: false,
            },
            source: SourceData::Modrinth {
                game_version,
                loader,
                loader_version,
                files: tasks,
            },
        });
    }

    // 通用 ZIP
    let prefix = generic_prefix(&names);
    let mut file_count = 0u32;
    let mut override_bytes = 0u64;
    let mut detected = None;
    for name in &names {
        let rel = name.strip_prefix(&prefix).unwrap_or(name);
        if let Some(rest) = rel.strip_prefix("versions/") {
            let parts: Vec<&str> = rest.split('/').collect();
            if parts.len() == 2 && parts[1] == format!("{}.json", parts[0]) {
                if let Ok(bytes) = read_entry(&mut archive, name) {
                    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        let id = json
                            .get("id")
                            .and_then(|value| value.as_str())
                            .unwrap_or(parts[0]);
                        let parent = json
                            .get("inheritsFrom")
                            .and_then(|value| value.as_str())
                            .unwrap_or(id);
                        detected = Some(parent.to_string());
                    }
                }
            }
            continue;
        }
        if excluded_generic(rel) {
            continue;
        }
        file_count += 1;
        if let Ok(entry) = archive.by_name(name) {
            override_bytes += entry.size();
        }
    }
    let warnings = vec!["通用压缩包没有加载器信息，需要选择父版本".to_string()];
    let display_name = zip_name;
    let server_like = looks_like_server(&names, &prefix);
    Ok(ParsedPack {
        preview: PackPreview {
            format: "generic".into(),
            name: display_name.clone(),
            version: String::new(),
            game_version: detected.clone().unwrap_or_default(),
            loader: "vanilla".into(),
            loader_version: None,
            detected_parent: detected.clone(),
            file_count,
            download_bytes: 0,
            override_bytes,
            files: Vec::new(),
            warnings,
            suggested_id: slugify(&display_name),
            server_skips: 0,
            server_like,
        },
        source: SourceData::Generic {
            prefix,
            detected_parent: detected,
        },
    })
}

pub fn inspect(path: &str) -> Result<PackPreview> {
    Ok(parse_pack(path, false)?.preview)
}

/* ---------- 在线下载 ---------- */

/// 把在线整合包下载到临时目录，返回本地路径；导入完成后由前端调用清理。
pub async fn download(
    app: &AppHandle,
    client: &reqwest::Client,
    url: &str,
    file_name: &str,
    progress: Arc<ProgressState>,
) -> Result<String> {
    let name = Path::new(file_name)
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "modpack.zip".to_string());
    let dir = temp_dir();
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(&name);
    let task = FileTask {
        url: url.to_string(),
        dest: dest.clone(),
        sha1: None,
        size: 0,
        label: name,
    };
    download::run_tasks(app, client, vec![task], 1, false, progress).await?;
    Ok(dest.to_string_lossy().to_string())
}

pub fn temp_dir() -> PathBuf {
    crate::config::data_dir().join("temp").join("modpacks")
}

/// 删除临时目录里的整合包文件，仅允许清理 temp/modpacks 内的路径。
pub fn temp_remove(path: &str) -> Result<()> {
    let target = PathBuf::from(path);
    let base = temp_dir();
    if !target.starts_with(&base) {
        bail!("只能清理下载的临时整合包");
    }
    if target.is_file() {
        std::fs::remove_file(&target)?;
    }
    Ok(())
}

/// 判断通用压缩包是否更像服务端整合包：客户端整合包不会带 libraries/，
/// 服务端包通常有 libraries、server.properties、eula.txt 或启动脚本。
fn looks_like_server(names: &[String], prefix: &str) -> bool {
    let mut has_libraries = false;
    let mut has_versions = false;
    let mut has_server_marker = false;
    for name in names {
        let rel = name.strip_prefix(prefix).unwrap_or(name).to_lowercase();
        if rel.starts_with("libraries/") {
            has_libraries = true;
        }
        if rel.starts_with("versions/") {
            has_versions = true;
        }
        if matches!(
            rel.as_str(),
            "server.properties"
                | "eula.txt"
                | "start.sh"
                | "start.bat"
                | "run.sh"
                | "run.bat"
                | "user_jvm_args.txt"
        ) {
            has_server_marker = true;
        }
    }
    if has_versions {
        return false;
    }
    has_libraries || has_server_marker
}

fn generic_prefix(names: &[String]) -> String {
    if let Some(name) = names
        .iter()
        .find(|name| name.to_lowercase().contains(".minecraft/"))
    {
        let lower = name.to_lowercase();
        if let Some(index) = lower.find(".minecraft/") {
            return name[..index + ".minecraft/".len()].to_string();
        }
    }
    if let Some(first) = names.first() {
        if let Some((segment, _)) = first.split_once('/') {
            if !matches!(
                segment,
                "mods" | "config" | "versions" | "resourcepacks" | "shaderpacks" | "saves" | "defaultconfigs"
            ) && names.iter().all(|name| name.starts_with(&format!("{segment}/")))
            {
                return format!("{segment}/");
            }
        }
    }
    String::new()
}

fn excluded_generic(rel: &str) -> bool {
    let lower = rel.to_lowercase();
    lower.starts_with("logs/")
        || lower.starts_with("crash-reports/")
        || lower.starts_with("screenshots/")
        || lower.starts_with("versions/")
        || lower == "usercache.json"
        || lower.starts_with("launcher_profiles")
        || lower.ends_with("session.lock")
        || lower.ends_with(".part")
}

fn safe_relative(path: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(value) => out.push(value),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

fn copy_dir_all(source: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(source)?.flatten() {
        let path = entry.path();
        let target = dest.join(entry.file_name());
        if path.is_dir() {
            copy_dir_all(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

fn unique_instance_id(cfg: &Config, base: &str) -> String {
    let versions = cfg.game_root().join("versions");
    let mut candidate = base.to_string();
    let mut index = 2;
    while versions.join(&candidate).exists() {
        candidate = format!("{base}-{index}");
        index += 1;
    }
    candidate
}

fn write_instance_json(cfg: &Config, id: &str, parent: &str) -> Result<PathBuf> {
    let dir = cfg.game_root().join("versions").join(id);
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::json!({
        "id": id,
        "inheritsFrom": parent,
        "type": "release",
    });
    std::fs::write(
        dir.join(format!("{id}.json")),
        serde_json::to_vec_pretty(&json)?,
    )?;
    Ok(dir)
}

/// PCL 式自包含实例：把继承链的合并结果直接写成实例 json（不带 inheritsFrom），
/// client jar 落进实例目录，最后清掉不再被其他实例引用的父版本目录。
/// generic 导入的父版本可能是用户手动选的常用实例，remove_parents 传 false 不清理。
fn finalize_self_contained(
    cfg: &Config,
    instance_id: &str,
    remove_parents: bool,
    meta: Option<&crate::models::DoclMeta>,
) -> Result<()> {
    let mut merged = crate::install::resolve_detail(cfg, instance_id)?;
    // 合并后没有继承链，把游戏版本与加载器留在 json 里，供模组搜索和版本信息使用
    if let Some(meta) = meta {
        merged.detail.docl = Some(meta.clone());
    }
    let dir = cfg.game_root().join("versions").join(instance_id);
    std::fs::write(
        dir.join(format!("{instance_id}.json")),
        serde_json::to_vec_pretty(&merged.detail)?,
    )?;
    if let Some(jar_id) = &merged.jar_id {
        if jar_id != instance_id {
            let src = cfg
                .game_root()
                .join("versions")
                .join(jar_id)
                .join(format!("{jar_id}.jar"));
            let dst = dir.join(format!("{instance_id}.jar"));
            if src.exists() && !dst.exists() {
                std::fs::copy(&src, &dst)?;
            }
        }
    }
    if remove_parents {
        for part in &merged.chain {
            if part == instance_id {
                continue;
            }
            if !crate::install::is_referenced(cfg, part, std::slice::from_ref(&instance_id.to_string())) {
                let pdir = cfg.game_root().join("versions").join(part);
                let _ = std::fs::remove_dir_all(&pdir);
            }
        }
    }
    Ok(())
}

/// jar 打不开说明下载不完整（截断或 0 字节），FML 扫描时会直接抛
/// `ZipException: zip END header not found` 退出。
fn jar_is_broken(path: &Path) -> bool {
    match std::fs::File::open(path) {
        Ok(file) => zip::ZipArchive::new(file).is_err(),
        Err(_) => true,
    }
}

fn broken_jars(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_jar = path
            .extension()
            .map(|ext| ext.eq_ignore_ascii_case("jar"))
            .unwrap_or(false);
        if is_jar && jar_is_broken(&path) {
            out.push(path);
        }
    }
    out
}

/// 实例自检发现的损坏文件
#[derive(Debug, Clone, Serialize)]
pub struct BrokenFile {
    pub kind: String,
    pub name: String,
    pub size: u64,
}

/// 实例文件自检：返回扫描数量与无法作为 zip 打开的文件。
/// mods 里的损坏文件会让 FML 初始化直接退出，资源包/光影包同理但不致命。
pub fn scan_broken_files(root: &Path) -> (u32, Vec<BrokenFile>) {
    let mut scanned = 0u32;
    let mut broken = Vec::new();
    for kind in ["mods", "resourcepacks", "shaderpacks"] {
        let Ok(entries) = std::fs::read_dir(root.join(kind)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let is_archive = path
                .extension()
                .map(|ext| {
                    let ext = ext.to_string_lossy().to_ascii_lowercase();
                    ext == "jar" || ext == "zip"
                })
                .unwrap_or(false);
            if !is_archive {
                continue;
            }
            scanned += 1;
            if jar_is_broken(&path) {
                broken.push(BrokenFile {
                    kind: kind.to_string(),
                    name: path
                        .file_name()
                        .map(|value| value.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    size: std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0),
                });
            }
        }
    }
    broken.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.name.cmp(&b.name)));
    (scanned, broken)
}

/// 导入后自检 mods 目录：损坏的 jar 按本次导入的下载地址单文件补下
/// （download::refetch 内部最多 3 次换源重试），仍不可用则记入 failed。
async fn repair_broken_jars(
    client: &reqwest::Client,
    cfg: &Config,
    root: &Path,
    tasks: &[FileTask],
    failed: &mut Vec<String>,
    progress: &ProgressState,
) {
    for path in broken_jars(&root.join("mods")) {
        // 用户取消导入后不必再逐个补下
        if progress.is_cancelled() {
            break;
        }
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        // 优先按完整路径匹配；路径规范差异时退回同名文件（mods 下基本同名唯一）
        let task = tasks
            .iter()
            .find(|item| item.dest == path)
            .or_else(|| {
                tasks
                    .iter()
                    .find(|item| item.dest.file_name() == path.file_name())
            });
        let Some(task) = task else {
            failed.push(format!("{name}：文件损坏且没有下载地址，请手动替换"));
            continue;
        };
        let _ = download::refetch(client, task, cfg.use_mirror()).await;
        if jar_is_broken(&path) {
            failed.push(format!("{name}：文件损坏，自动补下失败，请删除后重新导入"));
        }
    }
}

async fn ensure_vanilla(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    game_version: &str,
    progress: Arc<ProgressState>,
) -> Result<()> {
    if install::list_installed(cfg)
        .iter()
        .any(|item| item.id == game_version)
    {
        return Ok(());
    }
    progress.set_message(format!("正在安装原版 {game_version}…"));
    install::install_version(app, client, cfg, game_version, progress).await?;
    Ok(())
}

fn normalize_loader_version(
    game_version: &str,
    loader: &str,
    version: Option<String>,
) -> Option<String> {
    version.map(|value| {
        let prefix = format!("{game_version}-");
        if matches!(loader, "forge" | "neoforge") && value.starts_with(&prefix) {
            value[prefix.len()..].to_string()
        } else {
            value
        }
    })
}

async fn ensure_loader(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    game_version: &str,
    loader: &str,
    loader_version: Option<&str>,
    progress: Arc<ProgressState>,
) -> Result<String> {
    if loader == "vanilla" || loader.is_empty() {
        return Ok(game_version.to_string());
    }
    progress.set_message(format!("正在安装 {loader}…"));
    let version =
        install::install_loader(app, client, cfg, game_version, loader, loader_version, progress)
            .await?;
    Ok(version.id)
}

fn extract_entries(
    path: &str,
    prefix: &str,
    dest: &Path,
    generic: bool,
    progress: &Arc<ProgressState>,
) -> Result<u32> {
    let mut archive = open_zip(path)?;
    let mut total = 0u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let relative = if prefix.is_empty() {
            name.as_str()
        } else {
            match name.strip_prefix(prefix) {
                Some(rest) => rest,
                None => continue,
            }
        };
        if relative.is_empty() || (generic && excluded_generic(relative)) {
            continue;
        }
        total += 1;
    }

    progress.files_total.store(total, Ordering::Relaxed);
    progress.files_done.store(0, Ordering::Relaxed);
    progress.bytes_total.store(0, Ordering::Relaxed);
    progress.active.store(true, Ordering::Relaxed);

    let mut written = 0u32;
    for index in 0..archive.len() {
        if progress.is_cancelled() {
            bail!("导入已取消");
        }
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let relative = if prefix.is_empty() {
            name.as_str()
        } else {
            match name.strip_prefix(prefix) {
                Some(rest) => rest,
                None => continue,
            }
        };
        if relative.is_empty() || (generic && excluded_generic(relative)) {
            continue;
        }
        let Some(relative_path) = safe_relative(relative) else {
            continue;
        };
        let target = dest.join(relative_path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&target)?;
        std::io::copy(&mut entry, &mut out)?;
        written += 1;
        progress.files_done.fetch_add(1, Ordering::Relaxed);
        progress
            .bytes_done
            .fetch_add(entry.size(), Ordering::Relaxed);
    }
    Ok(written)
}

/* ---------- 导出 ---------- */

#[derive(Debug, Clone, Serialize)]
pub struct ExportEntry {
    pub key: String,
    pub label: String,
    pub file_count: u32,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportPreview {
    pub instance_id: String,
    pub name: String,
    pub version: String,
    pub game_version: String,
    pub loader: String,
    pub loader_version: Option<String>,
    pub entries: Vec<ExportEntry>,
    pub total_bytes: u64,
    /// 检测到的常用自定义目录（tacz 枪包等），前端展示为可勾选项
    pub suggested_extras: Vec<ExportEntry>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ExportOptions {
    pub name: String,
    pub version: String,
    pub format: String,
    pub groups: Vec<String>,
    pub target: String,
    /// 额外打包的自定义文件夹（相对实例目录，如 tacz 或 config/tacz/custom）
    #[serde(default)]
    pub extra_dirs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportResult {
    pub paths: Vec<String>,
    pub bytes: u64,
    pub file_count: u32,
}

const EXPORT_GROUPS: &[(&str, &str)] = &[
    ("options", "游戏设置"),
    ("personal", "个人信息"),
    ("mods", "Mod"),
    ("config", "配置与脚本"),
    ("maps", "地图与坐标"),
    ("jei", "JEI / 手册"),
    ("resourcepacks", "资源包"),
    ("shaderpacks", "光影"),
    ("saves", "存档"),
    ("screenshots", "截图"),
];

fn group_roots(group: &str) -> (Vec<&'static str>, Vec<&'static str>) {
    match group {
        "options" => (Vec::new(), vec!["options.txt"]),
        "personal" => (
            Vec::new(),
            vec!["servers.dat", "command_history.txt"],
        ),
        "mods" => (vec!["mods"], Vec::new()),
        "config" => (
            vec!["config", "defaultconfigs", "scripts", "kubejs", "datapacks"],
            Vec::new(),
        ),
        "maps" => (
            vec![
                "journeymap",
                "xaero",
                "XaeroWaypoints",
                "XaeroWorldMap",
                "map",
            ],
            Vec::new(),
        ),
        "jei" => (vec!["jei", "emi", "patchouli"], Vec::new()),
        "resourcepacks" => (vec!["resourcepacks"], Vec::new()),
        "shaderpacks" => (vec!["shaderpacks"], vec!["optionsshaders.txt"]),
        "saves" => (vec!["saves"], Vec::new()),
        "screenshots" => (vec!["screenshots"], Vec::new()),
        _ => (Vec::new(), Vec::new()),
    }
}

fn walk_collect(root: &Path, relative: &Path, out: &mut Vec<(PathBuf, PathBuf)>) {
    let dir = root.join(relative);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        if name_text == "session.lock" || name_text.ends_with(".part") {
            continue;
        }
        let rel = relative.join(&name);
        if path.is_dir() {
            walk_collect(root, &rel, out);
        } else {
            out.push((path, rel));
        }
    }
}

fn collect_group_files(
    root: &Path,
    groups: &[String],
) -> Vec<(PathBuf, PathBuf)> {
    let mut seen = std::collections::HashSet::new();
    let mut files = Vec::new();
    for group in groups {
        let (dirs, names) = group_roots(group);
        for name in names {
            let path = root.join(name);
            if path.is_file() && seen.insert(name.to_string()) {
                files.push((path, PathBuf::from(name)));
            }
        }
        for dir in dirs {
            let mut collected = Vec::new();
            walk_collect(root, Path::new(dir), &mut collected);
            for (path, rel) in collected {
                let key = rel.to_string_lossy().to_string();
                if seen.insert(key) {
                    files.push((path, rel));
                }
            }
        }
    }
    files
}

/// 逻辑单元计数：存档按存档文件夹、光影/资源包按压缩包或文件夹，
/// 其余组按文件数。展示"1992 个文件"没有意义，用户关心的是"几个存档"。
fn count_units(root: &Path, group: &str) -> u32 {
    let (dirs, names) = group_roots(group);
    let mut count = 0u32;
    match group {
        "saves" | "shaderpacks" | "resourcepacks" | "maps" | "jei" => {
            for dir in dirs {
                if let Ok(entries) = std::fs::read_dir(root.join(dir)) {
                    count += entries.flatten().count() as u32;
                }
            }
        }
        _ => {
            for dir in dirs {
                let mut collected = Vec::new();
                walk_collect(root, Path::new(dir), &mut collected);
                count += collected.len() as u32;
            }
            for name in names {
                if root.join(name).is_file() {
                    count += 1;
                }
            }
        }
    }
    count
}

/// 常见的"启动器不会自动带上"的自定义目录，典型如 TACZ 枪械包
const SUGGESTED_EXTRA_DIRS: &[&str] = &["tacz", "config/tacz/custom", "customitems", "customworld"];

/// 校验用户输入的额外目录：拒绝绝对路径、越级逃逸与空路径，统一为 / 分隔
fn sanitize_extra_dir(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_matches(['/', '\\']);
    if trimmed.is_empty() {
        return None;
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return None;
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => {
                let text = part.to_string_lossy();
                if text == "." || text.eq_ignore_ascii_case("c:$recycle.bin") {
                    return None;
                }
                parts.push(text.to_string());
            }
            _ => return None,
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

/// 统计一个额外目录的内容
fn extra_dir_entry(root: &Path, relative: &str) -> Option<ExportEntry> {
    let dir = root.join(relative);
    if !dir.is_dir() {
        return None;
    }
    let mut collected = Vec::new();
    walk_collect(&dir, Path::new(""), &mut collected);
    if collected.is_empty() {
        return None;
    }
    let bytes: u64 = collected
        .iter()
        .map(|(path, _)| std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0))
        .sum();
    Some(ExportEntry {
        key: relative.to_string(),
        label: relative.to_string(),
        file_count: collected.len() as u32,
        bytes,
    })
}

fn loader_key(loader: &str) -> Option<&'static str> {
    match loader {
        "fabric" => Some("fabric-loader"),
        "quilt" => Some("quilt-loader"),
        "forge" => Some("forge"),
        "neoforge" => Some("neoforge"),
        _ => None,
    }
}

fn loader_version_of(cfg: &Config, version_id: &str, loader: &str) -> Option<String> {
    let merged = install::resolve_detail(cfg, version_id).ok()?;
    for id in merged.chain.iter().skip(1) {
        let lower = id.to_lowercase();
        match loader {
            "fabric" => {
                if let Some(rest) = lower.strip_prefix("fabric-loader-") {
                    let parts: Vec<&str> = rest.split('-').collect();
                    if parts.len() >= 2 {
                        return Some(parts[0].to_string());
                    }
                    return Some(rest.to_string());
                }
            }
            "quilt" => {
                if let Some(rest) = lower.strip_prefix("quilt-loader-") {
                    let parts: Vec<&str> = rest.split('-').collect();
                    if parts.len() >= 2 {
                        return Some(parts[0].to_string());
                    }
                    return Some(rest.to_string());
                }
            }
            "forge" => {
                if let Some((_, rest)) = lower.split_once("forge-") {
                    return Some(rest.to_string());
                }
            }
            "neoforge" => {
                if let Some((_, rest)) = lower.split_once("neoforge-") {
                    return Some(rest.to_string());
                }
            }
            _ => {}
        }
    }
    None
}

pub fn export_preview(cfg: &Config, version_id: &str) -> Result<ExportPreview> {
    let detail = instance::info(cfg, version_id)?;
    let root = instance::game_dir(cfg, version_id);
    let mut entries = Vec::new();
    let mut all = std::collections::HashSet::new();
    let mut total_bytes = 0u64;
    for (key, label) in EXPORT_GROUPS {
        let files = collect_group_files(&root, &[key.to_string()]);
        let bytes: u64 = files
            .iter()
            .map(|(path, _)| std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0))
            .sum();
        for (_, rel) in &files {
            all.insert(rel.to_string_lossy().to_string());
        }
        entries.push(ExportEntry {
            key: key.to_string(),
            label: label.to_string(),
            file_count: count_units(&root, key),
            bytes,
        });
    }
    let mut suggested_extras = Vec::new();
    for dir in SUGGESTED_EXTRA_DIRS {
        // 已被常规组覆盖的目录不再建议（如 config/tacz/custom 不在 config 展开时）
        if all.contains(&dir.to_string()) {
            continue;
        }
        if let Some(entry) = extra_dir_entry(&root, dir) {
            suggested_extras.push(entry);
        }
    }
    for (key, _) in EXPORT_GROUPS {
        let files = collect_group_files(&root, &[key.to_string()]);
        for (path, rel) in files {
            if all.remove(&rel.to_string_lossy().to_string()) {
                total_bytes += std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
            }
        }
    }
    Ok(ExportPreview {
        instance_id: version_id.to_string(),
        name: detail.display_name,
        version: "1.0.0".into(),
        game_version: detail.game_version,
        loader: detail.loader.clone(),
        loader_version: loader_version_of(cfg, version_id, &detail.loader),
        entries,
        total_bytes,
        suggested_extras,
    })
}

fn sanitize_entry(path: &str) -> String {
    path.replace('\\', "/")
}

pub async fn export(
    app: &AppHandle,
    cfg: &Config,
    version_id: &str,
    options: &ExportOptions,
    progress: Arc<ProgressState>,
) -> Result<ExportResult> {
    let preview = export_preview(cfg, version_id)?;
    let root = instance::game_dir(cfg, version_id);
    let groups: Vec<String> = if options.groups.is_empty() {
        EXPORT_GROUPS.iter().map(|(key, _)| key.to_string()).collect()
    } else {
        options.groups.clone()
    };
    let mut files = collect_group_files(&root, &groups);
    // 额外包含文件夹（TACZ 枪包等）：相对实例目录整体打包，路径必须安全
    for raw in &options.extra_dirs {
        let Some(relative) = sanitize_extra_dir(raw) else {
            continue;
        };
        let mut collected = Vec::new();
        walk_collect(&root, Path::new(&relative), &mut collected);
        for (path, rel) in collected {
            let key = sanitize_entry(&rel.to_string_lossy());
            files.push((path, PathBuf::from(key)));
        }
    }
    let mut seen_paths = std::collections::HashSet::new();
    files.retain(|(_, rel)| seen_paths.insert(sanitize_entry(&rel.to_string_lossy())));
    if files.is_empty() {
        bail!("没有可导出的文件");
    }

    let name = if options.name.trim().is_empty() {
        preview.name.clone()
    } else {
        options.name.trim().to_string()
    };
    let version = if options.version.trim().is_empty() {
        "1.0.0".to_string()
    } else {
        options.version.trim().to_string()
    };
    let format = if options.format.is_empty() {
        "zip"
    } else {
        options.format.as_str()
    };
    let target = PathBuf::from(&options.target);
    let target = match target.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("zip") || ext.eq_ignore_ascii_case("mrpack") => target,
        _ => target.with_extension(if format == "mrpack" { "mrpack" } else { "zip" }),
    };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let total_bytes: u64 = files
        .iter()
        .map(|(path, _)| std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0))
        .sum();
    progress.set_message("正在打包整合包…");
    progress.files_total.store(files.len() as u64, Ordering::Relaxed);
    progress.files_done.store(0, Ordering::Relaxed);
    progress.bytes_total.store(total_bytes, Ordering::Relaxed);
    progress.bytes_done.store(0, Ordering::Relaxed);
    progress.active.store(true, Ordering::Relaxed);

    let folder = slugify(&name);
    let target_text = target.to_string_lossy().to_string();
    let tmp_path = PathBuf::from(format!("{target_text}.part"));
    let _ = std::fs::remove_file(&tmp_path);

    let mut written_bytes = 0u64;
    {
        let file = std::fs::File::create(&tmp_path)?;
        let mut zip = zip::ZipWriter::new(file);
        let entry_options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        if format == "mrpack" {
            let mut dependencies = serde_json::Map::new();
            dependencies.insert(
                "minecraft".into(),
                serde_json::Value::String(preview.game_version.clone()),
            );
            if let Some(key) = loader_key(&preview.loader) {
                if let Some(loader_version) = &preview.loader_version {
                    dependencies.insert(
                        key.into(),
                        serde_json::Value::String(loader_version.clone()),
                    );
                }
            }
            let index = serde_json::json!({
                "formatVersion": 1,
                "game": "minecraft",
                "versionId": version,
                "name": name,
                "summary": "",
                "files": [],
                "dependencies": dependencies,
            });
            zip.start_file("modrinth.index.json", entry_options)?;
            use std::io::Write;
            zip.write_all(serde_json::to_string_pretty(&index)?.as_bytes())?;
        } else {
            let manifest = serde_json::json!({
                "name": name,
                "version": version,
                "base": preview.game_version,
                "loader": preview.loader,
                "loader_version": preview.loader_version,
                "groups": groups,
            });
            zip.start_file(format!("{folder}/manifest.docl.json"), entry_options)?;
            use std::io::Write;
            zip.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
        }

        for (path, rel) in &files {
            if progress.is_cancelled() {
                drop(zip);
                let _ = std::fs::remove_file(&tmp_path);
                bail!("导出已取消");
            }
            let relative = sanitize_entry(&rel.to_string_lossy());
            let archive_path = if format == "mrpack" {
                format!("overrides/{relative}")
            } else {
                format!("{folder}/.minecraft/{relative}")
            };
            let meta = std::fs::metadata(path)?;
            zip.start_file(archive_path, entry_options)?;
            let mut input = std::fs::File::open(path)?;
            std::io::copy(&mut input, &mut zip)?;
            written_bytes += meta.len();
            progress.files_done.fetch_add(1, Ordering::Relaxed);
            progress.bytes_done.fetch_add(meta.len(), Ordering::Relaxed);
        }
        zip.finish()?;
    }

    if target.exists() {
        std::fs::remove_file(&target)?;
    }
    std::fs::rename(&tmp_path, &target)?;
    progress.active.store(false, Ordering::Relaxed);

    let _ = app.emit(
        "modpack-export-done",
        serde_json::json!({ "path": target_text }),
    );

    Ok(ExportResult {
        paths: vec![target_text],
        bytes: written_bytes,
        file_count: files.len() as u32,
    })
}

pub async fn import(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    path: &str,
    options: &ImportOptions,
    progress: Arc<ProgressState>,
) -> Result<ImportResult> {
    download::begin_stage(app, &progress, "正在解析整合包…");
    let parsed = parse_pack(path, options.server)?;
    let preview = &parsed.preview;

    let display_name = options
        .instance_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| preview.name.clone());
    let base_id = options
        .instance_id
        .clone()
        .map(|value| slugify(&value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| preview.suggested_id.clone());
    let instance_id = unique_instance_id(cfg, &base_id);

    let mut failed: Vec<String> = Vec::new();
    let mut installed_files = 0u32;

    let (_, game_version, loader) = match &parsed.source {
        SourceData::Curse {
            game_version,
            loader,
            loader_version,
            files,
            overrides,
        } => {
            ensure_vanilla(app, client, cfg, game_version, progress.clone()).await?;
            let requested = normalize_loader_version(
                game_version,
                loader,
                options
                    .loader_version
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| loader_version.clone()),
            );
            let parent = ensure_loader(
                app,
                client,
                cfg,
                game_version,
                loader,
                requested.as_deref(),
                progress.clone(),
            )
            .await?;

            let dir = write_instance_json(cfg, &instance_id, &parent)?;
            instance::save(
                cfg,
                &instance_id,
                &InstanceMeta {
                    display_name: Some(display_name.clone()),
                    description: format!("从 CurseForge 整合包导入 · {game_version} · {loader}"),
                    icon: "grass".into(),
                    category: "modpack".into(),
                    isolated: Some(true),
                    ..Default::default()
                },
            )?;
            let _ = dir;

            let natives = install::natives_dir(cfg, &parent);
            if natives.exists() {
                let target = install::natives_dir(cfg, &instance_id);
                copy_dir_all(&natives, &target)?;
            }

            // 解析每个文件的下载地址
            let specs: Vec<(i64, i64, bool)> = files
                .iter()
                .cloned()
                .filter(|(_, _, required)| *required || options.install_optional)
                .collect();
            let total_specs = specs.len();
            download::begin_stage(
                app,
                &progress,
                format!("正在获取 {total_specs} 个文件地址…"),
            );
            let client_ref = client.clone();
            // API 源顺序跟随全局下载源设置：优先镜像时先试国内 MCIM
            let prefer_mirror = cfg.use_mirror();
            let mut tasks: Vec<FileTask> = Vec::new();
            let mut results: Vec<Result<Option<curseforge::FileDownload>>> = Vec::new();
            let mut stream = stream::iter(specs.into_iter().map(|(project_id, file_id, required)| {
                let client = client_ref.clone();
                async move {
                    match curseforge::file_download(&client, project_id, file_id, prefer_mirror).await {
                        Ok(pair) => Ok(Some(pair)),
                        Err(err) => {
                            if required {
                                // 带上 ID，失败时用户能定位到具体文件。
                                // 这里是「取文件信息」阶段：403 基本是国内直连 CurseForge 接口被 Cloudflare 拦截，
                                // 不是作者限制下载（那种情况接口会给空 downloadUrl，由 CDN 兜底），提示要指向代理。
                                let text = err.to_string();
                                let reason = if text.contains("403") {
                                    "无法访问 CurseForge 接口（国内网络需开启系统代理后重新导入）"
                                } else if text.contains("404") {
                                    "镜像未收录该文件（开启系统代理后重新导入即可补齐）"
                                } else {
                                    "获取下载信息失败，开启代理后重新导入可重试"
                                };
                                // 顺带查一次项目名，让用户知道缺的是哪个 Mod（失败时才查）
                                let name = curseforge::projects(&client, &[project_id])
                                    .await
                                    .into_iter()
                                    .next()
                                    .map(|project| project.name)
                                    .unwrap_or_default();
                                if name.is_empty() {
                                    Err(anyhow!("{project_id}/{file_id}：{reason}"))
                                } else {
                                    Err(anyhow!("{name}（{project_id}/{file_id}）：{reason}"))
                                }
                            } else {
                                Ok(None)
                            }
                        }
                    }
                }
            }))
            // 403 来自上游网络而非并发压力（PCL 同样高并发无此问题），保持原并发
            .buffer_unordered(8);
            let mut resolved = 0usize;
            while let Some(result) = stream.next().await {
                results.push(result);
                resolved += 1;
                if resolved % 10 == 0 || resolved == total_specs {
                    progress.set_detail(format!("正在获取文件地址 {resolved}/{total_specs}…"));
                }
            }

            let root = instance::game_dir(cfg, &instance_id);
            let mods_dir = root.join("mods");
            std::fs::create_dir_all(&mods_dir)?;
            for result in results {
                match result {
                    Ok(Some(file)) => {
                        let file_name = Path::new(&file.name)
                            .file_name()
                            .map(|value| value.to_string_lossy().to_string())
                            .unwrap_or_else(|| "mod.jar".into());
                        tasks.push(FileTask {
                            dest: mods_dir.join(file_name),
                            url: file.url,
                            sha1: file.sha1,
                            size: file.size,
                            label: file.name,
                        });
                    }
                    Ok(None) => failed.push("可选文件下载地址缺失".into()),
                    Err(err) => failed.push(err.to_string()),
                }
            }
            if !tasks.is_empty() {
                download::begin_stage(app, &progress, "正在下载整合包内容…");
                let count = tasks.len() as u32;
                download::run_tasks(
                    app,
                    client,
                    tasks.clone(),
                    (cfg.concurrency as usize).clamp(1, download::MAX_CONCURRENCY),
                    cfg.use_mirror(),
                    progress.clone(),
                )
                .await?;
                installed_files += count;
            }
            download::begin_stage(app, &progress, "正在校验下载文件…");
            repair_broken_jars(client, cfg, &root, &tasks, &mut failed, progress.as_ref()).await;

            download::begin_stage(app, &progress, "正在解压整合包覆盖文件…");
            let path_owned = path.to_string();
            let overrides_owned = overrides.clone();
            let dest = root.clone();
            let progress_clone = progress.clone();
            tokio::task::spawn_blocking(move || {
                extract_entries(
                    &path_owned,
                    &format!("{overrides_owned}/"),
                    &dest,
                    false,
                    &progress_clone,
                )
            })
            .await??;

            (parent, game_version.clone(), loader.clone())
        }

        SourceData::Modrinth {
            game_version,
            loader,
            loader_version,
            files,
        } => {
            ensure_vanilla(app, client, cfg, game_version, progress.clone()).await?;
            let requested = normalize_loader_version(
                game_version,
                loader,
                options
                    .loader_version
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| loader_version.clone()),
            );
            let parent = ensure_loader(
                app,
                client,
                cfg,
                game_version,
                loader,
                requested.as_deref(),
                progress.clone(),
            )
            .await?;

            write_instance_json(cfg, &instance_id, &parent)?;
            instance::save(
                cfg,
                &instance_id,
                &InstanceMeta {
                    display_name: Some(display_name.clone()),
                    description: format!("从 Modrinth 整合包导入 · {game_version} · {loader}"),
                    icon: "grass".into(),
                    category: "modpack".into(),
                    isolated: Some(true),
                    ..Default::default()
                },
            )?;
            let natives = install::natives_dir(cfg, &parent);
            if natives.exists() {
                copy_dir_all(&natives, &install::natives_dir(cfg, &instance_id))?;
            }

            let root = instance::game_dir(cfg, &instance_id);
            let mut tasks = Vec::new();
            for (file_path, url, sha1, size, optional) in files {
                if url.is_empty() {
                    if !optional || options.install_optional {
                        failed.push(format!("缺少下载地址：{file_path}"));
                    }
                    continue;
                }
                if *optional && !options.install_optional {
                    continue;
                }
                let Some(relative) = safe_relative(file_path) else {
                    continue;
                };
                tasks.push(FileTask {
                    url: url.clone(),
                    dest: root.join(relative),
                    sha1: sha1.clone(),
                    size: *size,
                    label: file_path.clone(),
                });
            }
            if !tasks.is_empty() {
                download::begin_stage(app, &progress, "正在下载整合包内容…");
                let count = tasks.len() as u32;
                download::run_tasks(
                    app,
                    client,
                    tasks.clone(),
                    (cfg.concurrency as usize).clamp(1, download::MAX_CONCURRENCY),
                    cfg.use_mirror(),
                    progress.clone(),
                )
                .await?;
                installed_files += count;
            }
            download::begin_stage(app, &progress, "正在校验下载文件…");
            repair_broken_jars(client, cfg, &root, &tasks, &mut failed, progress.as_ref()).await;

            download::begin_stage(app, &progress, "正在解压整合包覆盖文件…");
            let path_owned = path.to_string();
            let dest = root.clone();
            for prefix in ["overrides/", "client-overrides/"] {
                let prefix_owned = prefix.to_string();
                let dest_clone = dest.clone();
                let progress_clone = progress.clone();
                let path_clone = path_owned.clone();
                tokio::task::spawn_blocking(move || {
                    extract_entries(&path_clone, &prefix_owned, &dest_clone, false, &progress_clone)
                })
                .await??;
            }

            (parent, game_version.clone(), loader.clone())
        }

        SourceData::Generic {
            prefix,
            detected_parent,
        } => {
            let parent = options
                .parent_version
                .clone()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| {
                    detected_parent
                        .clone()
                        .filter(|value| install::list_installed(cfg).iter().any(|item| &item.id == value))
                })
                .ok_or_else(|| anyhow!("通用整合包需要选择一个已安装的父版本"))?;
            if !install::list_installed(cfg)
                .iter()
                .any(|item| item.id == parent)
            {
                bail!("父版本 {parent} 尚未安装");
            }

            write_instance_json(cfg, &instance_id, &parent)?;
            instance::save(
                cfg,
                &instance_id,
                &InstanceMeta {
                    display_name: Some(display_name.clone()),
                    description: "从本地压缩包导入".into(),
                    icon: "grass".into(),
                    category: "modpack".into(),
                    isolated: Some(true),
                    ..Default::default()
                },
            )?;
            let natives = install::natives_dir(cfg, &parent);
            if natives.exists() {
                copy_dir_all(&natives, &install::natives_dir(cfg, &instance_id))?;
            }

            download::begin_stage(app, &progress, "正在解压整合包文件…");
            let path_owned = path.to_string();
            let prefix_owned = prefix.clone();
            let dest = instance::game_dir(cfg, &instance_id);
            let progress_clone = progress.clone();
            let written = tokio::task::spawn_blocking(move || {
                extract_entries(
                    &path_owned,
                    &prefix_owned,
                    &dest,
                    true,
                    &progress_clone,
                )
            })
            .await??;
            installed_files += written;

            // 本地压缩包导入没有原始下载地址，只能报告损坏文件
            let instance_root = instance::game_dir(cfg, &instance_id);
            repair_broken_jars(client, cfg, &instance_root, &[], &mut failed, progress.as_ref())
                .await;

            // 通用整合包沿用父实例的版本与加载器，父实例可能是 Forge/NeoForge 实例
            let (game_version, loader) = install::instance_context(cfg, &parent)
                .unwrap_or_else(|_| (parent.clone(), "vanilla".to_string()));
            (parent, game_version, loader)
        }
    };

    // 导入完成后做 PCL 式自包含：合并 json + jar 进实例，清掉独占的父版本目录。
    // generic 的父版本可能是用户自己的实例，不清理。失败时保留 inheritsFrom 架构兜底。
    let remove_parents = !matches!(parsed.source, SourceData::Generic { .. });
    let meta = crate::models::DoclMeta {
        game_version: game_version.clone(),
        loader: loader.clone(),
    };
    if let Err(error) = finalize_self_contained(cfg, &instance_id, remove_parents, Some(&meta)) {
        progress.set_message(format!("自包含合并失败（实例仍可用）：{error}"));
    }

    Ok(ImportResult {
        instance_id,
        display_name,
        installed_files,
        failed,
    })
}

#[cfg(test)]
mod tests {
    use super::broken_jars;
    use std::io::Write;
    use std::path::PathBuf;

    fn temp_dir(label: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("docl-pack-{label}-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 0 字节与截断的 jar 要报出来，正常 zip 与无关文件不能误报
    #[test]
    fn reports_only_broken_jars() {
        let dir = temp_dir("broken");
        std::fs::write(dir.join("empty.jar"), b"").unwrap();
        let mut truncated = std::fs::File::create(dir.join("truncated.jar")).unwrap();
        truncated.write_all(&[0x50, 0x4b, 0x03, 0x04, 0x00]).unwrap();
        drop(truncated);

        let good = dir.join("good.jar");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&good).unwrap());
        zip.start_file("a.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"ok").unwrap();
        zip.finish().unwrap();
        std::fs::write(dir.join("readme.txt"), b"not a jar").unwrap();

        let mut names: Vec<String> = broken_jars(&dir)
            .iter()
            .filter_map(|path| path.file_name().map(|name| name.to_string_lossy().to_string()))
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec!["empty.jar".to_string(), "truncated.jar".to_string()]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
