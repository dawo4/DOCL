use crate::config::Config;
use crate::curseforge;
use crate::download::{self, FileTask, ProgressState};
use crate::install;
use crate::instance::{self, InstanceMeta};
use anyhow::{anyhow, bail, Context, Result};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
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

#[derive(Debug, Clone, Serialize, Default)]
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
    /// 清单里标记为可选的文件数；默认不装，导入表单里可以勾选
    pub optional_count: u32,
    /// 加载器启动器能否自动安装；false 时按原版导入并保留全部文件
    pub loader_supported: bool,
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

/// 整合包清单里没装上的单个文件。
#[derive(Debug, Clone, Serialize)]
pub struct ImportFailure {
    /// 所属的 CurseForge 项目；Modrinth 清单不带项目信息时为 0
    pub project_id: i64,
    pub file_id: i64,
    /// 给用户认的名字：项目名，拿不到时退回文件名或 ID
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    pub instance_id: String,
    pub display_name: String,
    pub installed_files: u32,
    /// 清单里没装上的文件：可以逐个补下，也可以去项目页手动下载
    pub failures: Vec<ImportFailure>,
    /// 不属于单个文件的提醒：加载器暂不支持、清单写入失败等
    pub warnings: Vec<String>,
    /// 导入完成后的使用建议，例如内存分配
    pub hints: Vec<String>,
}

/* ---------- 用新版整合包文件更新实例 ---------- */

/// 更新预览里展示的单个文件。
#[derive(Debug, Clone, Serialize)]
pub struct PackUpdateFile {
    pub path: String,
    /// added / changed / removed
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackUpdatePreview {
    pub format: String,
    pub name: String,
    /// 旧清单里的版本，找不到时为空
    pub old_version: String,
    pub new_version: String,
    pub game_version: String,
    pub loader: String,
    pub loader_version: Option<String>,
    pub added: u32,
    pub changed: u32,
    pub removed: u32,
    /// 覆盖文件数量（overrides / client-overrides）
    pub overrides: u32,
    pub download_bytes: u64,
    /// 文件示例，最多 40 条
    pub files: Vec<PackUpdateFile>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackUpdateResult {
    pub downloaded: u32,
    pub removed: u32,
    /// 解压覆盖的文件数
    pub overrides: u32,
    /// 下载失败的文件，可在下载面板重试
    pub failed: Vec<String>,
    /// 解析/准备阶段的跳过项（如 CurseForge 文件信息取不到），没有重试入口
    pub warnings: Vec<String>,
    /// 更新前的文件备份目录，None 表示没有覆盖任何旧文件
    pub backup_dir: Option<String>,
}

/// 旧版整合包清单：用于判断哪些文件已经从新版里移除。
#[derive(Debug, Default)]
struct OldPack {
    name: String,
    version: String,
    paths: HashSet<String>,
}

fn text_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn read_json_file(path: &Path) -> Option<Value> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn collect_paths(list: &[Value], paths: &mut HashSet<String>) {
    for item in list {
        if let Some(path) = item.get("path").and_then(Value::as_str) {
            if !path.trim().is_empty() {
                paths.insert(path.to_string());
            }
        }
    }
}

/// 依次尝试 DOCL 记录、Modrinth 清单、HMCL 的 modpack.cfg。
fn old_pack_manifest(root: &Path) -> Option<OldPack> {
    let mut pack = OldPack::default();
    let mut found = false;

    if let Some(value) = read_json_file(&root.join(".docl-pack.json")) {
        pack.name = text_field(&value, "name");
        pack.version = text_field(&value, "version");
        if let Some(list) = value.get("files").and_then(Value::as_array) {
            collect_paths(list, &mut pack.paths);
            found = true;
        }
    }
    if let Some(value) = read_json_file(&root.join("modrinth.index.json")) {
        if pack.name.is_empty() {
            pack.name = text_field(&value, "name");
        }
        if pack.version.is_empty() {
            pack.version = text_field(&value, "versionId");
        }
        if let Some(list) = value.get("files").and_then(Value::as_array) {
            collect_paths(list, &mut pack.paths);
            found = true;
        }
    }
    if let Some(value) = read_json_file(&root.join("modpack.cfg")) {
        if pack.name.is_empty() {
            pack.name = text_field(&value, "name");
        }
        if pack.version.is_empty() {
            pack.version = text_field(&value, "version");
        }
        if let Some(list) = value
            .get("manifest")
            .and_then(|manifest| manifest.get("files"))
            .and_then(Value::as_array)
        {
            collect_paths(list, &mut pack.paths);
            found = true;
        }
    }
    found.then_some(pack)
}

/// 写实例内的整合包清单，供下次更新判断「已移除的文件」。
fn write_pack_record(
    root: &Path,
    preview: &PackPreview,
    game_version: &str,
    loader: &str,
    loader_version: Option<&str>,
    paths: &[String],
) -> Result<()> {
    let mut paths = paths.to_vec();
    paths.sort();
    paths.dedup();
    let record = serde_json::json!({
        "format": preview.format,
        "name": preview.name,
        "version": preview.version,
        "game_version": game_version,
        "loader": loader,
        "loader_version": loader_version.unwrap_or_default(),
        "files": paths
            .iter()
            .map(|path| serde_json::json!({ "path": path }))
            .collect::<Vec<_>>(),
    });
    std::fs::write(
        root.join(".docl-pack.json"),
        serde_json::to_vec_pretty(&record)?,
    )?;
    Ok(())
}

fn sha1_file(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha1::new();
    std::io::copy(&mut reader, &mut hasher).ok()?;
    Some(format!("{:x}", hasher.finalize()))
}

const UPDATE_SAMPLE_LIMIT: usize = 40;

/// 新版清单里的单个文件：(路径, 下载地址, SHA-1, 大小, 是否可选)。
type UpdateFile = (String, String, Option<String>, u64, bool);

struct UpdatePlan {
    tasks: Vec<FileTask>,
    added: u32,
    changed: u32,
    removed: Vec<String>,
    download_bytes: u64,
    sample: Vec<PackUpdateFile>,
    /// 新版清单里的全部路径，用于判断旧文件是否已被移除
    paths: HashSet<String>,
}

/// 对比「磁盘现状」与「新版清单」，只把确实需要的文件放进下载任务。
fn plan_update(
    root: &Path,
    files: &[UpdateFile],
    old: Option<&OldPack>,
    install_optional: bool,
) -> UpdatePlan {
    let mut plan = UpdatePlan {
        tasks: Vec::new(),
        added: 0,
        changed: 0,
        removed: Vec::new(),
        download_bytes: 0,
        sample: Vec::new(),
        paths: HashSet::new(),
    };
    for (path, url, sha1, size, optional) in files {
        // 可选文件也记入清单，避免下次更新时被当成「已移除」
        plan.paths.insert(path.clone());
        if *optional && !install_optional {
            continue;
        }
        let Some(relative) = safe_relative(path) else {
            continue;
        };
        let dest = root.join(&relative);
        let exists = dest.is_file();
        let stale = match sha1 {
            Some(hash) => sha1_file(&dest)
                .map(|actual| !actual.eq_ignore_ascii_case(hash))
                .unwrap_or(true),
            None => {
                !exists
                    || std::fs::metadata(&dest)
                        .map(|meta| meta.len() != *size)
                        .unwrap_or(true)
            }
        };
        if !stale {
            continue;
        }
        plan.download_bytes += *size;
        if exists {
            plan.changed += 1;
        } else {
            plan.added += 1;
        }
        if plan.sample.len() < UPDATE_SAMPLE_LIMIT {
            plan.sample.push(PackUpdateFile {
                path: path.clone(),
                kind: if exists { "changed" } else { "added" }.into(),
            });
        }
        if url.is_empty() {
            continue;
        }
        plan.tasks.push(FileTask {
            url: url.clone(),
            dest,
            sha1: sha1.clone(),
            size: *size,
            label: path.clone(),
        });
    }
    if let Some(old) = old {
        for path in &old.paths {
            if plan.paths.contains(path) {
                continue;
            }
            let Some(relative) = safe_relative(path) else {
                continue;
            };
            if !root.join(&relative).is_file() {
                continue;
            }
            plan.removed.push(path.clone());
            if plan.sample.len() < UPDATE_SAMPLE_LIMIT {
                plan.sample.push(PackUpdateFile {
                    path: path.clone(),
                    kind: "removed".into(),
                });
            }
        }
    }
    plan
}

/// 压缩包里的覆盖文件条目；prefixes 需带结尾 `/`（CurseForge 的 overrides 目录名可自定义）。
fn override_entries(path: &str, prefixes: &[String]) -> Result<Vec<String>> {
    let mut archive = open_zip(path)?;
    let mut out = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let Some(rest) = prefixes
            .iter()
            .find_map(|prefix| name.strip_prefix(prefix.as_str()))
        else {
            continue;
        };
        if rest.is_empty() || safe_relative(rest).is_none() {
            continue;
        }
        out.push(rest.to_string());
    }
    Ok(out)
}

/// 更新前把会被覆盖/删除的文件备份到数据目录。
struct PackBackup {
    root: PathBuf,
    game_root: PathBuf,
    saved: usize,
}

impl PackBackup {
    fn new(game_root: &Path, version_id: &str) -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_secs())
            .unwrap_or(0);
        Self {
            root: crate::config::data_dir()
                .join("backups")
                .join(format!("pack-{}-{stamp}", slugify(version_id))),
            game_root: game_root.to_path_buf(),
            saved: 0,
        }
    }

    fn save(&mut self, path: &Path) -> Result<()> {
        if !path.is_file() {
            return Ok(());
        }
        let relative = path
            .strip_prefix(&self.game_root)
            .context("备份文件不在实例目录内")?;
        let dest = self.root.join(relative);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(path, dest)?;
        self.saved += 1;
        Ok(())
    }

    fn path(&self) -> Option<String> {
        (self.saved > 0).then(|| self.root.to_string_lossy().to_string())
    }
}

/// 更新只允许在隔离实例上做：否则会改到同目录其他版本的 Mod 与配置。
fn ensure_isolated(cfg: &Config, version_id: &str) -> Result<()> {
    let isolated = instance::get(cfg, version_id)
        .isolated
        .unwrap_or(cfg.version_isolation);
    if !isolated {
        bail!("该实例未启用版本隔离，更新会写入共享的游戏目录；请先在实例设置里开启版本隔离");
    }
    Ok(())
}

/// 新版整合包解析出的统一更新来源。
struct UpdateSource {
    game_version: String,
    loader: String,
    loader_version: Option<String>,
    files: Vec<UpdateFile>,
    /// 覆盖文件目录前缀（带结尾 `/`）
    prefixes: Vec<String>,
    /// 解析阶段的问题，提示给用户
    warnings: Vec<String>,
    /// 是否拿到了完整文件清单；不完整时不能按清单判断「已移除」
    complete: bool,
}

/// CurseForge 的 overrides 目录名可自定义，统一成安全相对路径 + 结尾 `/`。
fn normalize_prefix(value: &str) -> String {
    let normalised = value.trim().replace('\\', "/");
    let trimmed = normalised.trim_matches('/');
    if trimmed.is_empty() || safe_relative(trimmed).is_none() {
        "overrides/".into()
    } else {
        format!("{trimmed}/")
    }
}

fn download_file_name(file: &curseforge::FileDownload) -> String {
    let from_name = Path::new(&file.name)
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_default();
    if !from_name.trim().is_empty() {
        return from_name;
    }
    file.url
        .split('?')
        .next()
        .unwrap_or("")
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("mod.jar")
        .to_string()
}

/// CurseForge 项目大类 → 实例子目录。
/// 整合包清单里的光影、资源包、数据包必须分开放：全塞进 mods 不但不会生效，
/// 还会让光影列表里看不到（个别整合包就是这样把光影下进 mods 的）。
const CLASS_MOD: i64 = 6;
const CLASS_RESOURCEPACK: i64 = 12;
const CLASS_WORLD: i64 = 17;
const CLASS_SHADER: i64 = 6552;
const CLASS_DATAPACK: i64 = 6945;

fn pack_subdir(class_id: Option<i64>, file_name: &str, modules: &[String]) -> &'static str {
    match class_id {
        Some(CLASS_SHADER) => "shaderpacks",
        Some(CLASS_RESOURCEPACK) => "resourcepacks",
        Some(CLASS_DATAPACK) => "datapacks",
        // 地图是 zip，落进 saves 后由收尾阶段解压成存档文件夹
        Some(CLASS_WORLD) => "saves",
        Some(CLASS_MOD) => "mods",
        _ => infer_subdir(file_name, modules),
    }
}

/// 项目大类缺失时的兜底：和 PCL 一样看压缩包内容。
/// 数据包与资源包都有 pack.mcmeta，用是否含 data/ 区分。
fn infer_subdir(file_name: &str, modules: &[String]) -> &'static str {
    if modules.is_empty() {
        // 拿不到内容清单时按 Mod 处理，与 PCL 的 #1006 兜底一致
        return "mods";
    }
    let has = |name: &str| modules.iter().any(|item| item.eq_ignore_ascii_case(name));
    if file_name.to_ascii_lowercase().ends_with(".jar")
        || has("META-INF")
        || has("mcmod.info")
        || has("fabric.mod.json")
    {
        return "mods";
    }
    if has("shaders") {
        return "shaderpacks";
    }
    if has("pack.mcmeta") {
        return if has("data") { "datapacks" } else { "resourcepacks" };
    }
    "saves"
}

/// 整合包收尾：解压 saves 里的地图 zip，并把数据包放到真正会被加载的位置。
/// 原版只从 world/datapacks 读取数据包；装了 Open Loader / Paxi 时改放它们的全局目录。
fn sync_content_dirs(root: &std::path::Path) -> Result<()> {
    let saves = root.join("saves");
    // 1) 地图：清单里下载的是 zip，解压成存档文件夹。
    // zip 必须保留：更新时 plan_update 按「文件是否存在」判断，删掉会导致每次更新都重下。
    // 用隐藏标记记录已解压过，避免重复同步时生成「世界 (2)」这样的副本。
    if saves.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&saves) {
            let zips: Vec<std::path::PathBuf> = entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_file()
                        && path
                            .extension()
                            .map(|ext| ext.eq_ignore_ascii_case("zip"))
                            .unwrap_or(false)
                })
                .collect();
            let marks = saves.join(".docl-map-marks");
            for zip in zips {
                let Some(name) = zip.file_name().map(|value| value.to_string_lossy().to_string())
                else {
                    continue;
                };
                let marker = marks.join(format!("{name}.done"));
                if marker.is_file() {
                    continue;
                }
                if crate::curseforge::install_world_zip(&zip, &saves).is_ok() {
                    let _ = std::fs::create_dir_all(&marks);
                    let _ = std::fs::write(&marker, b"");
                }
            }
        }
    }

    // 2) 数据包：实例根目录只是暂存位置，真正生效要放进存档或全局加载器目录
    let datapacks = root.join("datapacks");
    let Ok(entries) = std::fs::read_dir(&datapacks) else {
        return Ok(());
    };
    let files: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    if files.is_empty() {
        return Ok(());
    }

    // 2a) 装了 Open Loader / Paxi 时写它们的全局目录：所有存档（含以后新建的）都会加载。
    // 此时不再往每个存档各复制一份，避免同一数据包被加载两次。
    if let Some(global) = global_datapack_dir(root) {
        copy_datapacks(&files, &global)?;
        return Ok(());
    }

    // 2b) 没有全局加载器：复制进每个已有存档，没有存档时保留在实例根目录
    if !saves.is_dir() {
        return Ok(());
    }
    let Ok(worlds) = std::fs::read_dir(&saves) else {
        return Ok(());
    };
    for world in worlds.flatten() {
        let path = world.path();
        if !path.is_dir() || !path.join("level.dat").is_file() {
            continue;
        }
        copy_datapacks(&files, &path.join("datapacks"))?;
    }
    Ok(())
}

/// 实例里装了全局数据包加载器时，返回它扫描的目录。
/// Open Loader 与 Paxi 是 CurseForge 推荐的两个，只认优先级最高的一个，避免重复加载。
fn global_datapack_dir(root: &std::path::Path) -> Option<std::path::PathBuf> {
    let mods = root.join("mods");
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&mods) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name.ends_with(".jar") {
                names.push(name);
            }
        }
    }
    let has = |needle: &str| names.iter().any(|name| name.contains(needle));
    if has("openloader") {
        // 默认目录，可在 config/openloader/advanced_options.json 里改；改过的用户需要手动同步
        return Some(root.join("config").join("openloader").join("data"));
    }
    if has("paxi") {
        return Some(root.join("config").join("paxi").join("datapacks"));
    }
    None
}

fn copy_datapacks(files: &[std::path::PathBuf], target: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(target)?;
    for file in files {
        let Some(name) = file.file_name() else {
            continue;
        };
        // 同名覆盖，兼容更新后换了数据包版本；单个失败不影响其它文件
        let _ = std::fs::copy(file, target.join(name));
    }
    Ok(())
}

/// 把新版整合包解析成统一的更新文件清单：
/// Modrinth 清单自带路径与直链，CurseForge 需要联网把 projectID/fileID 换成文件名与直链。
async fn resolve_update_source(
    client: &reqwest::Client,
    cfg: &Config,
    parsed: &ParsedPack,
) -> Result<UpdateSource> {
    match &parsed.source {
        SourceData::Modrinth {
            game_version,
            loader,
            loader_version,
            files,
        } => Ok(UpdateSource {
            game_version: game_version.clone(),
            loader: loader.clone(),
            loader_version: loader_version.clone(),
            files: files.clone(),
            prefixes: vec!["overrides/".into(), "client-overrides/".into()],
            warnings: Vec::new(),
            complete: true,
        }),
        SourceData::Curse {
            game_version,
            loader,
            loader_version,
            overrides,
            files,
        } => {
            let prefer_mirror = cfg.use_mirror();
            let project_ids: Vec<i64> = files.iter().map(|(id, _, _)| *id).collect();
            let specs = files.clone();
            let mut stream = stream::iter(specs.into_iter().map(|(project_id, file_id, required)| {
                let client = client.clone();
                async move {
                    let outcome =
                        curseforge::file_download(&client, project_id, file_id, prefer_mirror).await;
                    (project_id, file_id, required, outcome)
                }
            }))
            .buffer_unordered(8);
            let mut raw: Vec<(i64, bool, curseforge::FileDownload)> = Vec::new();
            let mut warnings = Vec::new();
            while let Some((project_id, file_id, required, outcome)) = stream.next().await {
                match outcome {
                    Ok(file) => raw.push((project_id, required, file)),
                    Err(err) => warnings
                        .push(format!("无法获取 CurseForge 文件 {project_id}/{file_id}：{err}")),
                }
            }
            // 光影、资源包按项目大类放进对应目录，不能一律塞 mods
            let class_ids =
                curseforge::project_class_ids(client, &project_ids, prefer_mirror).await;
            let mut resolved: Vec<UpdateFile> = Vec::new();
            for (project_id, required, file) in raw {
                let class_id = match class_ids.get(&project_id).copied() {
                    Some(id) => Some(id),
                    // 批量接口缺项时才逐个查，且只查非 jar 文件（Mod 一定是 jar）
                    None if !file.name.to_lowercase().ends_with(".jar") => {
                        curseforge::project_class_id(client, project_id, prefer_mirror).await
                    }
                    None => None,
                };
                let path = format!(
                    "{}/{}",
                    pack_subdir(class_id, &file.name, &file.modules),
                    download_file_name(&file)
                );
                resolved.push((path, file.url, file.sha1, file.size, !required));
            }
            if resolved.is_empty() && !warnings.is_empty() {
                bail!("{}", warnings.remove(0));
            }
            // 有文件没解析出来时，清单是不完整的：不能据此删除旧文件，
            // 否则同名且已安装的文件会被误判为「已移除」直接删掉
            let mut complete = warnings.is_empty();
            if resolved.is_empty() {
                // 空清单多半是包损坏或下载不完整，同样不做删除
                complete = false;
                warnings.push("新版整合包清单里没有可更新的文件，本次不会删除旧文件".into());
            }
            if !warnings.is_empty() {
                // 失败项太多时只列前几条，避免预览弹窗被刷屏
                warnings.truncate(5);
                warnings.push("部分文件取不到下载信息，更新时会跳过；开启代理后重试可补齐".into());
            }
            Ok(UpdateSource {
                game_version: game_version.clone(),
                loader: loader.clone(),
                loader_version: loader_version.clone(),
                files: resolved,
                prefixes: vec![normalize_prefix(overrides)],
                warnings,
                complete,
            })
        }
        SourceData::Generic { .. } => bail!(
            "通用压缩包没有文件清单，无法用来更新；请使用整合包原始文件（.mrpack 或 CurseForge 压缩包）"
        ),
    }
}

/// 预览：只做对比，不下载、不修改任何文件。CurseForge 包需要联网解析文件名。
pub async fn update_preview(
    client: &reqwest::Client,
    cfg: &Config,
    version_id: &str,
    path: &str,
) -> Result<PackUpdatePreview> {
    let parsed = {
        let path = path.to_string();
        // 解压包可能很大，放到阻塞线程池
        tokio::task::spawn_blocking(move || parse_pack(&path, false)).await??
    };
    let source = resolve_update_source(client, cfg, &parsed).await?;
    let root = instance::game_dir(cfg, version_id);
    if !root.is_dir() {
        bail!("实例目录不存在：{}", root.display());
    }
    ensure_isolated(cfg, version_id)?;
    let old = old_pack_manifest(&root);
    let has_old = old.is_some();
    let old_version = old
        .as_ref()
        .map(|pack| pack.version.clone())
        .unwrap_or_default();
    let files = source.files;
    let prefixes = source.prefixes;
    let archive = path.to_string();
    let game_version = source.game_version;
    let loader = source.loader;
    let loader_version = source.loader_version;
    let complete = source.complete;
    let mut warnings = source.warnings;
    // 清单不完整时不按「已移除」处理，避免把没解析出来的文件误删
    let plan_old = if complete {
        old
    } else {
        warnings.push("新版整合包有文件没能解析，本次不会删除任何旧文件".into());
        None
    };
    let plan_root = root.clone();
    let (plan, overrides) = tokio::task::spawn_blocking(move || {
        let overrides = override_entries(&archive, &prefixes)
            .map(|list| list.len())
            .unwrap_or(0) as u32;
        (
            plan_update(&plan_root, &files, plan_old.as_ref(), false),
            overrides,
        )
    })
    .await?;

    if !has_old {
        warnings.push("没有找到旧版整合包清单，本次不会删除已从整合包移除的文件".into());
    }
    let (current_game, current_loader) = install::instance_context(cfg, version_id)
        .unwrap_or_else(|_| (String::new(), String::new()));
    if !current_game.is_empty() && !game_version.is_empty() && current_game != game_version {
        warnings.push(format!(
            "新版整合包要求游戏 {game_version}，当前实例是 {current_game}；更新只替换文件，不会切换游戏版本"
        ));
    }
    if !loader.is_empty()
        && !current_loader.is_empty()
        && !current_loader.eq_ignore_ascii_case(&loader)
    {
        warnings.push(format!(
            "新版整合包使用 {loader}，当前实例是 {current_loader}；需要先换成对应加载器再更新"
        ));
    }
    if plan.tasks.iter().any(|task| task.url.is_empty()) {
        warnings.push("有文件缺少下载地址，更新时会跳过".into());
    }

    Ok(PackUpdatePreview {
        format: parsed.preview.format,
        name: parsed.preview.name,
        old_version,
        new_version: parsed.preview.version,
        game_version,
        loader,
        loader_version,
        added: plan.added,
        changed: plan.changed,
        removed: plan.removed.len() as u32,
        overrides,
        download_bytes: plan.download_bytes,
        files: plan.sample,
        warnings,
    })
}

/// 应用更新：备份 → 删除旧文件 → 下载变更 → 解压覆盖 → 写新清单。
pub async fn update_apply(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    version_id: &str,
    path: &str,
    install_optional: bool,
    progress: Arc<ProgressState>,
) -> Result<PackUpdateResult> {
    download::begin_stage(app, &progress, "正在解析新版整合包…");
    let parsed = {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || parse_pack(&path, false)).await??
    };
    let source = resolve_update_source(client, cfg, &parsed).await?;
    let root = instance::game_dir(cfg, version_id);
    if !root.is_dir() {
        bail!("实例目录不存在：{}", root.display());
    }
    ensure_isolated(cfg, version_id)?;
    let old = old_pack_manifest(&root);
    let mut warnings = source.warnings;
    // 清单不完整时不按「已移除」处理，避免把没解析出来的文件误删
    let plan_old = if source.complete {
        old
    } else {
        warnings.push("新版整合包有文件没能解析，本次不会删除任何旧文件".into());
        None
    };
    let plan = plan_update(&root, &source.files, plan_old.as_ref(), install_optional);

    // 先备份所有会被覆盖或删除的文件，再动手
    let mut backup = PackBackup::new(&root, version_id);
    for task in &plan.tasks {
        backup.save(&task.dest)?;
    }
    for relative in &plan.removed {
        if let Some(rel) = safe_relative(relative) {
            backup.save(&root.join(rel))?;
        }
    }
    let override_list = override_entries(path, &source.prefixes).unwrap_or_default();
    for relative in &override_list {
        if let Some(rel) = safe_relative(relative) {
            backup.save(&root.join(rel))?;
        }
    }

    // CurseForge 里个别文件取不到直链时只跳过，不影响其余文件更新
    let mut failed = Vec::new();
    let mut failed_count = 0u32;
    let task_count = plan.tasks.len() as u32;
    if !plan.tasks.is_empty() {
        download::begin_stage(
            app,
            &progress,
            format!("正在下载 {} 个更新文件…", plan.tasks.len()),
        );
        let outcome = download::run_tasks_reporting(
            app,
            client,
            plan.tasks.clone(),
            (cfg.concurrency as usize).clamp(1, download::MAX_CONCURRENCY),
            cfg.use_mirror(),
            progress.clone(),
        )
        .await?;
        failed_count = outcome.failed.len() as u32;
        failed.extend(outcome.failed.into_iter().map(|(label, reason)| format!("{label}：{reason}")));
    }

    // 覆盖文件：overrides 里的配置、资源包等
    download::begin_stage(app, &progress, "正在解压整合包覆盖文件…");
    let path_owned = path.to_string();
    let prefixes = source.prefixes.clone();
    let dest = root.clone();
    let progress_clone = progress.clone();
    let written = tokio::task::spawn_blocking(move || {
        let mut count = 0u32;
        for prefix in &prefixes {
            count += extract_entries(&path_owned, prefix, &dest, false, &progress_clone)?;
        }
        Ok::<u32, anyhow::Error>(count)
    })
    .await??;

    // 删除旧版清单里已被移除的文件放到下载之后，避免下载失败时先丢文件
    let mut removed = 0u32;
    for relative in &plan.removed {
        let Some(rel) = safe_relative(relative) else {
            continue;
        };
        let dest = root.join(rel);
        if std::fs::remove_file(&dest).is_ok() {
            removed += 1;
        }
    }

    // 与导入一致：地图 zip 解压成存档，数据包同步进每个已有存档
    let root_for_sync = root.clone();
    match tokio::task::spawn_blocking(move || sync_content_dirs(&root_for_sync)).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => warnings.push(format!("整理存档内容失败：{err}")),
        Err(err) => warnings.push(format!("整理存档内容失败：{err}")),
    }

    // 记录本次清单，下次更新时用来判断「已移除的文件」；
    // 文件已经改完，写清单失败只提示，不应把整次更新判为失败
    let inventory: Vec<String> = plan.paths.into_iter().collect();
    if let Err(err) = write_pack_record(
        &root,
        &parsed.preview,
        &source.game_version,
        &source.loader,
        source.loader_version.as_deref(),
        &inventory,
    ) {
        warnings.push(format!("写入整合包清单失败：{err}"));
    }

    Ok(PackUpdateResult {
        downloaded: task_count.saturating_sub(failed_count),
        removed,
        overrides: written,
        failed,
        warnings,
        backup_dir: backup.path(),
    })
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
    crate::ziputil::open(std::path::Path::new(path))
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

/// 清单里的加载器 ID 拆成「名称 + 版本」。
/// `fabric-loader-0.15.11` 的版本是 `0.15.11`：不能从第一个连字符就切，
/// 否则会拿 `loader-0.15.11` 去请求加载器配置，整包导入直接失败。
/// 认不出的加载器保留原名字母（小写），由调用方决定是跳过安装还是提示，不能悄悄当成原版。
fn detect_loader(id: &str) -> (String, Option<String>) {
    let id = id.trim();
    let (head, version) = match id.split_once('-') {
        Some((head, rest)) => {
            // fabric-loader-0.15.11 / quilt-loader-0.21.0：名字里的 `-loader` 不是分隔符
            let (head, rest) = if let Some(version) = rest.strip_prefix("loader-") {
                (format!("{head}-loader"), version)
            } else if rest == "loader" {
                (format!("{head}-loader"), "")
            } else {
                (head.to_string(), rest)
            };
            (
                head.to_ascii_lowercase(),
                (!rest.is_empty()).then(|| rest.to_string()),
            )
        }
        None => (id.to_ascii_lowercase(), None),
    };
    let name = match head.as_str() {
        "minecraftforge" | "forge" => "forge",
        "neoforge" => "neoforge",
        "fabric-loader" | "fabric" => "fabric",
        "quilt-loader" | "quilt" => "quilt",
        "cleanroom" => "cleanroom",
        "liteloader" => "liteloader",
        _ => return (unknown_loader_name(&head), version),
    };
    (name.to_string(), version)
}

/// 未知加载器的内部名只留小写字母数字，例如 `rift-1.0.4-1.13` → `rift`；
/// 连名字都提取不出来时统一叫 unknown，交给界面显示「未知加载器」。
fn unknown_loader_name(head: &str) -> String {
    let name: String = head
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    if name.is_empty() {
        "unknown".into()
    } else {
        name
    }
}

/// 整合包声明的加载器启动器装不了时，导入前后给用户看的同一句话。
fn unsupported_loader_warning(loader: &str) -> String {
    format!(
        "该整合包使用{}加载器，启动器暂不支持自动安装。文件会全部导入并保留，但直接启动不会加载模组",
        install::loader_display_name(loader)
    )
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
        let loader_supported = install::is_supported_loader(&loader);
        if !loader_supported {
            warnings.push(unsupported_loader_warning(&loader));
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
                optional_count: manifest.files.iter().filter(|file| !file.required).count() as u32,
                loader_supported,
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
            .unwrap_or_else(|| {
                // 其余依赖键（例如 liteloader）按未知加载器原样报出，不归成原版
                index
                    .dependencies
                    .iter()
                    .find(|(key, _)| key.as_str() != "minecraft")
                    .map(|(key, version)| {
                        (
                            unknown_loader_name(&key.replace("-loader", "").to_ascii_lowercase()),
                            Some(version.clone()),
                        )
                    })
                    .unwrap_or_else(|| ("vanilla".into(), None))
            });
        let name = if index.name.is_empty() {
            zip_name.clone()
        } else {
            index.name.clone()
        };
        let mut warnings = Vec::new();
        if game_version.is_empty() {
            warnings.push("缺少 Minecraft 版本信息".into());
        }
        let loader_supported = install::is_supported_loader(&loader);
        if !loader_supported {
            warnings.push(unsupported_loader_warning(&loader));
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
                optional_count: tasks.iter().filter(|(_, _, _, _, optional)| *optional).count() as u32,
                loader_supported,
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
    let mut warnings = vec!["通用压缩包没有加载器信息，需要选择父版本".to_string()];
    let pack_loader = generic_loader(&mut archive, &names, &prefix);
    let loader_supported = pack_loader
        .as_ref()
        .map(|(loader, _)| install::is_supported_loader(loader))
        .unwrap_or(true);
    if let Some((loader, _)) = &pack_loader {
        if !loader_supported {
            warnings.push(unsupported_loader_warning(loader));
        }
    }
    let display_name = zip_name;
    let server_like = looks_like_server(&names, &prefix);
    Ok(ParsedPack {
        preview: PackPreview {
            format: "generic".into(),
            name: display_name.clone(),
            version: String::new(),
            game_version: detected.clone().unwrap_or_default(),
            loader: pack_loader
                .as_ref()
                .map(|(loader, _)| loader.clone())
                .unwrap_or_else(|| "vanilla".into()),
            loader_version: pack_loader.as_ref().and_then(|(_, version)| version.clone()),
            detected_parent: detected.clone(),
            file_count,
            download_bytes: 0,
            override_bytes,
            files: Vec::new(),
            warnings,
            suggested_id: slugify(&display_name),
            server_skips: 0,
            server_like,
            optional_count: 0,
            loader_supported,
        },
        source: SourceData::Generic {
            prefix,
            detected_parent: detected,
        },
    })
}

/// 通用压缩包里的加载器线索：`versions/<id>/<id>.json` 的主类与库坐标，
/// 以及 MultiMC / Prism 实例的 `mmc-pack.json` + `patches/*.json`。
/// 这里只负责认出加载器并如实显示；组件合并与依赖安装属于实例导入，另行处理。
fn generic_loader(
    archive: &mut ZipArchive<std::fs::File>,
    names: &[String],
    prefix: &str,
) -> Option<(String, Option<String>)> {
    // Prism / MultiMC：补丁里的主类最准。Cleanroom 沿用 net.minecraftforge 这个 uid，只能靠主类区分
    let patches = format!("{prefix}patches/");
    let patch_names: Vec<String> = names
        .iter()
        .filter(|name| name.starts_with(&patches) && name.ends_with(".json"))
        .cloned()
        .collect();
    for name in &patch_names {
        let Ok(bytes) = read_entry(archive, name) else {
            continue;
        };
        let Ok(json) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let loader = profile_loader(&json);
        if loader != "vanilla" {
            return Some((loader, None));
        }
    }
    if let Ok(bytes) = read_entry(archive, &format!("{prefix}mmc-pack.json")) {
        if let Ok(json) = serde_json::from_slice::<Value>(&bytes) {
            if let Some(found) = mmc_loader(&json) {
                return Some(found);
            }
        }
    }
    // 自带 versions/ 的包（其他启动器导出、服务端包）：逐个看版本清单
    for name in names {
        let rel = name.strip_prefix(prefix).unwrap_or(name);
        let Some(rest) = rel.strip_prefix("versions/") else {
            continue;
        };
        let mut parts = rest.split('/');
        let id = parts.next().unwrap_or_default();
        if parts.next() != Some(&format!("{id}.json")) {
            continue;
        }
        let Ok(bytes) = read_entry(archive, name) else {
            continue;
        };
        let Ok(json) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let loader = profile_loader(&json);
        if loader != "vanilla" {
            return Some((loader, None));
        }
    }
    None
}

/// 从一份版本清单 JSON 里读主类与库坐标来判断加载器。
/// MultiMC 的叠加补丁用 `+mainClass` / `+libraries`，两种键都认。
fn profile_loader(json: &Value) -> String {
    let main_class = json
        .get("mainClass")
        .or_else(|| json.get("+mainClass"))
        .and_then(Value::as_str);
    let libraries: Vec<String> = ["libraries", "+libraries"]
        .iter()
        .filter_map(|key| json.get(*key).and_then(Value::as_array))
        .flatten()
        .filter_map(|lib| lib.get("name").and_then(Value::as_str))
        .map(|name| name.to_ascii_lowercase())
        .collect();
    install::loader_of_profile(main_class, |prefix| {
        libraries.iter().any(|name| name.starts_with(prefix))
    })
}

/// `mmc-pack.json` 的组件列表 → 加载器。原版与 LWJGL 是基础组件，跳过。
fn mmc_loader(pack: &Value) -> Option<(String, Option<String>)> {
    let components = pack.get("components")?.as_array()?;
    for component in components {
        let uid = text_field(component, "uid");
        let version = component
            .get("version")
            .and_then(Value::as_str)
            .map(str::to_string);
        let name = match uid.as_str() {
            "net.minecraftforge" => "forge".to_string(),
            "net.neoforged" => "neoforge".to_string(),
            "org.quiltmc.quilt-loader" => "quilt".to_string(),
            "net.fabricmc.fabric-loader" => "fabric".to_string(),
            "com.mumfrey.liteloader" => "liteloader".to_string(),
            "net.minecraft" | "org.lwjgl" | "org.lwjgl3" => continue,
            _ => unknown_loader_name(uid.rsplit('.').next().unwrap_or(&uid)),
        };
        return Some((name, version));
    }
    None
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
/// （download::refetch 内部最多 3 次换源重试），返回仍然不可用的「文件名 + 原因」。
async fn repair_broken_jars(
    client: &reqwest::Client,
    cfg: &Config,
    root: &Path,
    tasks: &[FileTask],
    progress: &ProgressState,
) -> Vec<(String, String)> {
    let mut broken_after = Vec::new();
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
            broken_after.push((name, "文件损坏且没有下载地址，请手动替换".into()));
            continue;
        };
        let _ = download::refetch(client, task, cfg.use_mirror()).await;
        if jar_is_broken(&path) {
            broken_after.push((name, "文件损坏，自动补下失败，请删除后重新导入".into()));
        }
    }
    broken_after
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

/// 整合包声明的加载器，以及用户在导入表单里改过的版本
struct LoaderPlan<'a> {
    game_version: &'a str,
    loader: &'a str,
    /// 清单里写的加载器版本
    pack_version: &'a Option<String>,
    /// 表单里填的版本，优先于清单
    preferred: Option<String>,
}

/// 表单里填了加载器版本就用它，否则按清单
fn loader_plan<'a>(
    game_version: &'a str,
    loader: &'a str,
    pack_version: &'a Option<String>,
    options: &ImportOptions,
) -> LoaderPlan<'a> {
    LoaderPlan {
        game_version,
        loader,
        pack_version,
        preferred: options
            .loader_version
            .clone()
            .filter(|value| !value.trim().is_empty()),
    }
}

/// 按整合包声明的加载器准备父版本。启动器装不了的加载器不去装：
/// 实例挂在原版上，文件照常导入，并把原因作为第三项带回，由调用方放进提醒。
/// 返回（父版本 ID，实际采用的加载器版本，给用户的提醒）。
async fn ensure_pack_loader(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    plan: LoaderPlan<'_>,
    progress: Arc<ProgressState>,
) -> Result<(String, Option<String>, Option<String>)> {
    let LoaderPlan {
        game_version,
        loader,
        pack_version,
        preferred,
    } = plan;
    if !install::is_supported_loader(loader) {
        return Ok((
            game_version.to_string(),
            None,
            Some(unsupported_loader_warning(loader)),
        ));
    }
    let requested = normalize_loader_version(
        game_version,
        loader,
        preferred.or_else(|| pack_version.clone()),
    );
    let parent = ensure_loader(
        app,
        client,
        cfg,
        game_version,
        loader,
        requested.as_deref(),
        progress,
    )
    .await?;
    Ok((parent, requested, None))
}

/// 实例描述里的加载器一栏：装不了的标注出来，版本列表里一眼能看出这个实例不能直接玩
fn loader_tag(loader: &str) -> String {
    if install::is_supported_loader(loader) {
        loader.to_string()
    } else {
        format!("{}（暂不支持）", install::loader_display_name(loader))
    }
}

/// 一个待下载的 CurseForge 文件，连同它在清单里的身份与落盘位置
#[derive(Clone)]
struct Planned {
    task: FileTask,
    project_id: i64,
    file_id: i64,
    /// 相对实例目录的路径，写进整合包清单用
    relative: String,
}

/// 把一个 CurseForge 文件安排到实例里的位置：光影、资源包按项目大类分目录
fn curse_task(
    file: &curseforge::FileDownload,
    file_id: i64,
    class_id: Option<i64>,
    root: &Path,
) -> Planned {
    let sub = pack_subdir(class_id, &file.name, &file.modules);
    // 与更新路径保持一致：文件名为空时用下载直链里的文件名
    let file_name = download_file_name(file);
    Planned {
        task: FileTask {
            dest: root.join(sub).join(&file_name),
            url: file.url.clone(),
            sha1: file.sha1.clone(),
            size: file.size,
            label: file.name.clone(),
        },
        project_id: file.project_id,
        file_id,
        relative: format!("{sub}/{file_name}"),
    }
}

/// 「取文件信息」阶段失败的原因。
/// 403 基本是国内直连 CurseForge 接口被 Cloudflare 拦截，不是作者限制下载
/// （那种情况接口会给空 downloadUrl，由 CDN 兜底），提示要指向代理。
fn resolve_failure_reason(error: &anyhow::Error) -> &'static str {
    let text = error.to_string();
    if text.contains("403") {
        "无法访问 CurseForge 接口（国内网络需开启系统代理后重试）"
    } else if text.contains("404") {
        "镜像未收录该文件（开启系统代理后重试即可补齐）"
    } else {
        "获取下载信息失败，开启代理后重试即可补齐"
    }
}

/// 「取文件信息」阶段失败的文件。顺带查一次项目名，让用户知道缺的是哪个 Mod（失败时才查）。
async fn resolve_failure(
    client: &reqwest::Client,
    project_id: i64,
    file_id: i64,
    error: &anyhow::Error,
) -> ImportFailure {
    let name = curseforge::projects(client, &[project_id])
        .await
        .into_iter()
        .next()
        .map(|project| project.name)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| format!("项目 {project_id} / 文件 {file_id}"));
    ImportFailure {
        project_id,
        file_id,
        name,
        reason: resolve_failure_reason(error).into(),
    }
}

/// 下载阶段的失败项：按下载标签找回清单里的 projectID / fileID
fn push_download_failures(
    failures: &mut Vec<ImportFailure>,
    planned: &[Planned],
    failed: Vec<(String, String)>,
) {
    for (label, reason) in failed {
        let found = planned.iter().find(|item| item.task.label == label);
        failures.push(ImportFailure {
            project_id: found.map(|item| item.project_id).unwrap_or(0),
            file_id: found.map(|item| item.file_id).unwrap_or(0),
            name: label,
            reason,
        });
    }
}

/// 导入后自检发现的损坏 jar：能对上清单文件的按文件上报（可补下），对不上的归入提醒
fn push_broken_files(
    failures: &mut Vec<ImportFailure>,
    warnings: &mut Vec<String>,
    planned: &[Planned],
    broken: Vec<(String, String)>,
) {
    for (name, reason) in broken {
        let found = planned.iter().find(|item| {
            item.task
                .dest
                .file_name()
                .map(|value| value.to_string_lossy() == name)
                .unwrap_or(false)
        });
        match found {
            Some(item) => failures.push(ImportFailure {
                project_id: item.project_id,
                file_id: item.file_id,
                name,
                reason,
            }),
            None => warnings.push(format!("{name}：{reason}")),
        }
    }
}

/* ---------- 解析阶段失败的补下 ---------- */

/// 解析阶段就失败的文件没有下载地址，进不了下载面板的失败批次；
/// 按任务记下来，之后在任务卡片上批量补下或单项重试时重新解析。
#[derive(Clone)]
pub(crate) struct PendingSpecs {
    pub(crate) instance_id: String,
    pub(crate) specs: Vec<(i64, i64)>,
}

fn pending_slot() -> &'static Mutex<HashMap<String, PendingSpecs>> {
    static SLOT: OnceLock<Mutex<HashMap<String, PendingSpecs>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 锁被污染（别处持锁时 panic）时照样取出数据：这里只是进程内的备忘，不该因此拒绝补下
fn lock_pending() -> std::sync::MutexGuard<'static, HashMap<String, PendingSpecs>> {
    pending_slot().lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 新任务开始前只清掉它自己上一轮的记录
pub fn reset_pending_specs(job: &str) {
    lock_pending().remove(job);
}

pub fn add_pending_specs(
    job: &str,
    instance_id: &str,
    specs: impl IntoIterator<Item = (i64, i64)>,
) {
    let specs: Vec<(i64, i64)> = specs.into_iter().collect();
    if specs.is_empty() {
        // 没有待补文件就不建条目：空条目会让补下流程误判成「有解析失败要处理」
        return;
    }
    let mut slot = lock_pending();
    let entry = slot.entry(job.to_string()).or_insert_with(|| PendingSpecs {
        instance_id: instance_id.to_string(),
        specs: Vec::new(),
    });
    for spec in specs {
        if !entry.specs.contains(&spec) {
            entry.specs.push(spec);
        }
    }
}

/// 取走某个任务名下的全部待补文件（批量补下用）
pub fn take_pending_specs(job: &str) -> Option<PendingSpecs> {
    lock_pending().remove(job)
}

/// 单项补下只读不取：失败时原条目要留在原任务上，卡片才不会丢了重试入口
pub fn pending_specs(job: &str) -> Option<PendingSpecs> {
    lock_pending().get(job).cloned()
}

pub fn remove_pending_spec(job: &str, project_id: i64, file_id: i64) {
    let mut slot = lock_pending();
    let now_empty = if let Some(entry) = slot.get_mut(job) {
        entry.specs.retain(|spec| *spec != (project_id, file_id));
        entry.specs.is_empty()
    } else {
        false
    };
    // 最后一个也补上了：连条目一起清掉，避免留下空壳
    if now_empty {
        slot.remove(job);
    }
}

/// 把补下成功的文件追加进实例的整合包清单，之后用新版整合包更新时不会被当成包外文件
fn add_pack_paths(root: &Path, paths: &[String]) -> Result<()> {
    let path = root.join(".docl-pack.json");
    let Ok(bytes) = std::fs::read(&path) else {
        // 不是整合包实例，没有清单可写
        return Ok(());
    };
    let mut record: Value = serde_json::from_slice(&bytes)?;
    let files = record
        .as_object_mut()
        .ok_or_else(|| anyhow!("整合包清单格式异常"))?
        .entry("files")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !files.is_array() {
        bail!("整合包清单的 files 不是列表");
    }
    let list = files.as_array_mut().expect("上面已确认是列表");
    let mut known: HashSet<String> = list
        .iter()
        .filter_map(|item| item.get("path").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    for item in paths {
        if known.insert(item.clone()) {
            list.push(serde_json::json!({ "path": item }));
        }
    }
    std::fs::write(path, serde_json::to_vec_pretty(&record)?)?;
    Ok(())
}

/// 补下整合包清单里的文件：重新解析地址，按项目大类落进实例。
/// 返回仍然没装上的项，由调用方决定记到哪个任务名下。
pub async fn install_specs(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    instance_id: &str,
    specs: &[(i64, i64)],
    progress: Arc<ProgressState>,
) -> Vec<ImportFailure> {
    let root = instance::game_dir(cfg, instance_id);
    let prefer_mirror = cfg.use_mirror();
    let mut failures: Vec<ImportFailure> = Vec::new();
    let mut planned: Vec<Planned> = Vec::new();
    let total = specs.len();

    // 与导入时一样并发解析：整批失败后一次补几十个文件，串行会等很久。
    // 迭代器按值产出：引用模式 + async 块会触发编译器的高阶生命周期推断缺陷
    let client_ref = client.clone();
    let mut resolved = stream::iter(specs.iter().copied().map(|(project_id, file_id)| {
        let client = client_ref.clone();
        async move {
            let outcome =
                curseforge::file_download(&client, project_id, file_id, prefer_mirror).await;
            (project_id, file_id, outcome)
        }
    }))
    .buffer_unordered(8);
    let mut done = 0usize;
    while let Some((project_id, file_id, outcome)) = resolved.next().await {
        done += 1;
        progress.set_detail(format!("正在获取文件地址 {done}/{total}…"));
        // 用户取消后没处理的一律按「已取消」带回，不能从待补清单里消失
        if progress.is_cancelled() {
            failures.push(ImportFailure {
                project_id,
                file_id,
                name: format!("项目 {project_id} / 文件 {file_id}"),
                reason: "已取消".into(),
            });
            continue;
        }
        match outcome {
            Ok(file) => {
                // Mod 一定是 jar，其余才需要查项目大类
                let class_id = if file.name.to_lowercase().ends_with(".jar") {
                    None
                } else {
                    curseforge::project_class_id(client, file.project_id, prefer_mirror).await
                };
                planned.push(curse_task(&file, file_id, class_id, &root));
            }
            Err(err) => failures.push(resolve_failure(client, project_id, file_id, &err).await),
        }
    }

    let mut installed = Vec::new();
    if !planned.is_empty() {
        download::begin_stage(
            app,
            &progress,
            format!("正在补下 {} 个文件…", planned.len()),
        );
        let tasks: Vec<FileTask> = planned.iter().map(|item| item.task.clone()).collect();
        let outcome = download::run_tasks_reporting(
            app,
            client,
            tasks,
            (cfg.concurrency as usize).clamp(1, download::MAX_CONCURRENCY),
            cfg.use_mirror(),
            progress.clone(),
        )
        .await;
        match outcome {
            Ok(outcome) => {
                push_download_failures(&mut failures, &planned, outcome.failed);
            }
            // 被取消或整体失败：没落盘的都算未完成，原因照实带回
            Err(err) => {
                let reason = err.to_string();
                for item in planned.iter().filter(|item| !item.task.dest.exists()) {
                    failures.push(ImportFailure {
                        project_id: item.project_id,
                        file_id: item.file_id,
                        name: item.task.label.clone(),
                        reason: reason.clone(),
                    });
                }
            }
        }
        installed.extend(
            planned
                .iter()
                .filter(|item| item.task.dest.exists())
                .map(|item| item.relative.clone()),
        );
    }

    if !installed.is_empty() {
        if let Err(error) = add_pack_paths(&root, &installed) {
            failures.push(ImportFailure {
                project_id: 0,
                file_id: 0,
                name: "整合包清单".into(),
                reason: format!("文件已补齐，但写入清单失败：{error}"),
            });
        }
    }
    failures
}

/// 失败项里能对应到 CurseForge 清单文件的那些（projectID / fileID）
pub(crate) fn failure_specs(failures: &[ImportFailure]) -> Vec<(i64, i64)> {
    failures
        .iter()
        .filter(|item| item.project_id > 0)
        .map(|item| (item.project_id, item.file_id))
        .collect()
}

/* ---------- 导入后的使用建议 ---------- */

/// 导入完成后的使用建议：按模组数量估算内存需求，对照这个实例实际生效的分配。
fn import_hints(cfg: &Config, instance_id: &str) -> Vec<String> {
    let effective = instance::apply(cfg, instance_id);
    let mods = instance::game_dir(cfg, instance_id).join("mods");
    memory_hint(
        count_mod_jars(&mods),
        effective.memory_auto,
        effective.memory_mb,
    )
    .into_iter()
    .collect()
}

/// 模组越多越吃内存。自动分配会随可用内存浮动，只做说明；
/// 手动分配低于建议值时才提醒，免得每次导入都弹一句废话。
fn memory_hint(mod_count: usize, memory_auto: bool, memory_mb: u32) -> Option<String> {
    let suggested_gb = match mod_count {
        150.. => 8,
        100.. => 6,
        60.. => 4,
        _ => return None,
    };
    if memory_auto {
        Some(format!(
            "共 {mod_count} 个模组，建议可用内存不低于 {suggested_gb} GB（当前为自动分配）"
        ))
    } else if memory_mb < suggested_gb * 1024 {
        Some(format!(
            "共 {mod_count} 个模组，建议在实例设置里把内存调到 {suggested_gb} GB（当前 {memory_mb} MB）"
        ))
    } else {
        None
    }
}

fn count_mod_jars(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    entry
                        .path()
                        .extension()
                        .map(|ext| ext.eq_ignore_ascii_case("jar"))
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
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

const MR_BASE: &str = "https://api.modrinth.com/v2";
/// Modrinth 批量查询单次最多接受的哈希数量
const MR_HASH_BATCH: usize = 100;

#[derive(Debug, Deserialize)]
struct MrVersionFile {
    #[serde(default)]
    hashes: HashMap<String, String>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    size: u64,
}

#[derive(Debug, Deserialize)]
struct MrVersionEntry {
    #[serde(default)]
    project_id: String,
    #[serde(default)]
    files: Vec<MrVersionFile>,
}

#[derive(Debug, Deserialize)]
struct MrProjectSide {
    #[serde(default)]
    id: String,
    #[serde(default)]
    client_side: String,
    #[serde(default)]
    server_side: String,
}

/// 只有内容目录顶层的文件才写进 files[]，其余（存档、配置等）继续放 overrides。
fn mrpack_candidate(path: &str) -> bool {
    let mut parts = path.split('/');
    let Some(dir) = parts.next() else {
        return false;
    };
    if !matches!(dir, "mods" | "resourcepacks" | "shaderpacks" | "datapacks") {
        return false;
    }
    let Some(file) = parts.next() else {
        return false;
    };
    !file.trim().is_empty() && parts.next().is_none()
}

async fn fetch_project_sides(
    client: &reqwest::Client,
    ids: &[String],
) -> HashMap<String, MrProjectSide> {
    let mut sides = HashMap::new();
    for chunk in ids.chunks(MR_HASH_BATCH) {
        let Ok(ids_param) = serde_json::to_string(chunk) else {
            continue;
        };
        let Ok(url) = reqwest::Url::parse_with_params(
            &format!("{MR_BASE}/projects"),
            &[("ids", ids_param.as_str())],
        ) else {
            continue;
        };
        let Ok(list) = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            download::get_json::<Vec<MrProjectSide>>(client, url.as_str(), false),
        )
        .await
        .unwrap_or_else(|_| Err(anyhow!("Modrinth 项目信息超时")))
        else {
            continue;
        };
        for side in list {
            if !side.id.is_empty() {
                sides.insert(side.id.clone(), side);
            }
        }
    }
    sides
}

/// 把实例里的内容文件按 SHA-1 反查 Modrinth，命中的写进 mrpack 的 files[]，
/// 这样别人用 Modrinth 生态打开时能识别出对应项目并继续更新。
/// 返回 (files 数组, 已归档的相对路径集合)；查询失败时整体回退到 overrides。
async fn modrinth_index_files(
    client: &reqwest::Client,
    files: &[(PathBuf, PathBuf)],
) -> (Vec<Value>, HashSet<String>) {
    // 计算 SHA-1 要完整读一遍文件，整合包动辄几个 GB，放阻塞线程池
    let owned = files.to_vec();
    let candidates = tokio::task::spawn_blocking(move || {
        let mut out: Vec<(String, String, u64)> = Vec::new();
        for (path, rel) in owned {
            let relative = sanitize_entry(&rel.to_string_lossy());
            if !mrpack_candidate(&relative) {
                continue;
            }
            let Some(hash) = sha1_file(&path) else {
                continue;
            };
            let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            out.push((hash, relative, size));
        }
        out
    })
    .await
    .unwrap_or_default();
    if candidates.is_empty() {
        return (Vec::new(), HashSet::new());
    }

    let mut versions: HashMap<String, MrVersionEntry> = HashMap::new();
    for chunk in candidates.chunks(MR_HASH_BATCH) {
        let hashes: Vec<String> = chunk.iter().map(|(hash, _, _)| hash.clone()).collect();
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            client
                .post(format!("{MR_BASE}/version_files"))
                .json(&serde_json::json!({ "hashes": hashes, "algorithm": "sha1" }))
                .send(),
        )
        .await;
        // 匹配失败时降级为全部放 overrides，不影响导出本身
        let Ok(Ok(response)) = response else {
            return (Vec::new(), HashSet::new());
        };
        if !response.status().is_success() {
            return (Vec::new(), HashSet::new());
        }
        match response.json::<HashMap<String, MrVersionEntry>>().await {
            Ok(batch) => versions.extend(batch),
            Err(_) => return (Vec::new(), HashSet::new()),
        }
    }

    // 项目侧的 client/server 标记用于生成 env；取不到就不写，按双端必需处理
    let mut ids: Vec<String> = versions
        .values()
        .map(|version| version.project_id.clone())
        .filter(|id| !id.is_empty())
        .collect();
    // 同一项目可能有多个文件版本，去重减少请求体
    ids.sort();
    ids.dedup();
    let sides = fetch_project_sides(client, &ids).await;

    let mut entries = Vec::new();
    let mut matched = HashSet::new();
    for (hash, relative, local_size) in candidates {
        let Some(version) = versions.get(&hash) else {
            continue;
        };
        let file = version
            .files
            .iter()
            .find(|file| {
                file.hashes
                    .get("sha1")
                    .map(|value| value.eq_ignore_ascii_case(&hash))
                    .unwrap_or(false)
            })
            .or_else(|| version.files.first());
        let Some(file) = file else {
            continue;
        };
        if file.url.is_empty() {
            continue;
        }
        let mut hashes_json = serde_json::Map::new();
        hashes_json.insert("sha1".into(), Value::String(hash.clone()));
        if let Some(sha512) = file.hashes.get("sha512").filter(|value| !value.is_empty()) {
            hashes_json.insert("sha512".into(), Value::String(sha512.clone()));
        }
        let mut entry = serde_json::json!({
            "path": relative.clone(),
            "hashes": hashes_json,
            "downloads": [file.url],
            "fileSize": if file.size > 0 { file.size } else { local_size },
        });
        if let Some(side) = sides.get(&version.project_id) {
            if !side.client_side.is_empty() && !side.server_side.is_empty() {
                entry["env"] = serde_json::json!({
                    "client": side.client_side,
                    "server": side.server_side,
                });
            }
        }
        entries.push(entry);
        matched.insert(relative);
    }
    (entries, matched)
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
    client: &reqwest::Client,
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
    // .mrpack 需要完整的 files[]：按 SHA-1 反查 Modrinth，命中的交给生态管理，
    // 未命中的（自改 Mod、存档、配置等）仍作为 overrides 打包
    let mut mr_files: Vec<Value> = Vec::new();
    let mut mr_matched: HashSet<String> = HashSet::new();
    if format == "mrpack" {
        progress.set_message("正在匹配 Modrinth 文件…");
        let (entries, matched) = modrinth_index_files(client, &files).await;
        mr_files = entries;
        mr_matched = matched;
    }
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
                "files": mr_files,
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
            if mr_matched.contains(&relative) {
                // 已写进 files[]，安装器会自己下载，不能再重复塞进 overrides
                let size = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
                progress.files_done.fetch_add(1, Ordering::Relaxed);
                progress.bytes_done.fetch_add(size, Ordering::Relaxed);
                continue;
            }
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

    let mut failures: Vec<ImportFailure> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
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
            let (parent, requested, note) = ensure_pack_loader(
                app,
                client,
                cfg,
                loader_plan(game_version, loader, loader_version, options),
                progress.clone(),
            )
            .await?;
            warnings.extend(note);

            write_instance_json(cfg, &instance_id, &parent)?;
            instance::save(
                cfg,
                &instance_id,
                &InstanceMeta {
                    display_name: Some(display_name.clone()),
                    description: format!(
                        "从 CurseForge 整合包导入 · {game_version} · {}",
                        loader_tag(loader)
                    ),
                    icon: "grass".into(),
                    category: "modpack".into(),
                    isolated: Some(true),
                    ..Default::default()
                },
            )?;

            let natives = install::natives_dir(cfg, &parent);
            if natives.exists() {
                let target = install::natives_dir(cfg, &instance_id);
                copy_dir_all(&natives, &target)?;
            }

            // 解析每个文件的下载地址。可选文件只在勾选了「安装可选文件」时才会进来，
            // 进来的都是用户要的，失败一律照实上报
            let specs: Vec<(i64, i64)> = files
                .iter()
                .cloned()
                .filter(|(_, _, required)| *required || options.install_optional)
                .map(|(project_id, file_id, _)| (project_id, file_id))
                .collect();
            let total_specs = specs.len();
            let project_ids: Vec<i64> = specs.iter().map(|(id, _)| *id).collect();
            download::begin_stage(
                app,
                &progress,
                format!("正在获取 {total_specs} 个文件地址…"),
            );
            let client_ref = client.clone();
            // API 源顺序跟随全局下载源设置：优先镜像时先试国内 MCIM
            let prefer_mirror = cfg.use_mirror();
            let mut stream = stream::iter(specs.into_iter().map(|(project_id, file_id)| {
                let client = client_ref.clone();
                async move {
                    let outcome =
                        curseforge::file_download(&client, project_id, file_id, prefer_mirror).await;
                    (project_id, file_id, outcome)
                }
            }))
            // 403 来自上游网络而非并发压力（PCL 同样高并发无此问题），保持原并发
            .buffer_unordered(8);
            let mut results = Vec::new();
            let mut resolved = 0usize;
            while let Some(result) = stream.next().await {
                results.push(result);
                resolved += 1;
                if resolved % 10 == 0 || resolved == total_specs {
                    progress.set_detail(format!("正在获取文件地址 {resolved}/{total_specs}…"));
                }
            }

            // 光影、资源包按项目大类放进对应目录：清单只给 projectID，
            // 一律塞 mods 会导致光影不生效、资源包列表里也看不到
            let class_ids = curseforge::project_class_ids(client, &project_ids, prefer_mirror).await;
            let root = instance::game_dir(cfg, &instance_id);
            let mut pack_paths: Vec<String> = Vec::new();
            let mut planned: Vec<Planned> = Vec::new();
            for (project_id, file_id, outcome) in results {
                let file = match outcome {
                    Ok(file) => file,
                    Err(err) => {
                        failures
                            .push(resolve_failure(client, project_id, file_id, &err).await);
                        continue;
                    }
                };
                let class_id = match class_ids.get(&file.project_id).copied() {
                    Some(id) => Some(id),
                    // 批量接口缺项时才逐个查，且只查非 jar 文件（Mod 一定是 jar）
                    None if !file.name.to_lowercase().ends_with(".jar") => {
                        curseforge::project_class_id(
                            client,
                            file.project_id,
                            prefer_mirror,
                        )
                        .await
                    }
                    None => None,
                };
                let item = curse_task(&file, file_id, class_id, &root);
                pack_paths.push(item.relative.clone());
                planned.push(item);
            }
            let tasks: Vec<FileTask> = planned.iter().map(|item| item.task.clone()).collect();
            if !tasks.is_empty() {
                download::begin_stage(app, &progress, "正在下载整合包内容…");
                let count = tasks.len() as u32;
                // 整合包动辄几百个文件，个别文件超时不该让整个导入失败：
                // 收集失败项，导入仍算完成，由下载面板提供「补下失败文件」
                let outcome = download::run_tasks_reporting(
                    app,
                    client,
                    tasks.clone(),
                    (cfg.concurrency as usize).clamp(1, download::MAX_CONCURRENCY),
                    cfg.use_mirror(),
                    progress.clone(),
                )
                .await?;
                installed_files += count.saturating_sub(outcome.failed.len() as u32);
                push_download_failures(&mut failures, &planned, outcome.failed);
            }
            download::begin_stage(app, &progress, "正在校验下载文件…");
            let broken =
                repair_broken_jars(client, cfg, &root, &tasks, progress.as_ref()).await;
            push_broken_files(&mut failures, &mut warnings, &planned, broken);

            download::begin_stage(app, &progress, "正在解压整合包覆盖文件…");
            let path_owned = path.to_string();
            let overrides_prefix = normalize_prefix(overrides);
            let dest = root.clone();
            let progress_clone = progress.clone();
            tokio::task::spawn_blocking(move || {
                extract_entries(
                    &path_owned,
                    &overrides_prefix,
                    &dest,
                    false,
                    &progress_clone,
                )
            })
            .await??;

            // 记录清单，导入后也能用新版整合包继续更新；失败只提示，不影响已导入的文件
            if let Err(error) = write_pack_record(
                &root,
                &parsed.preview,
                game_version,
                loader,
                requested.as_deref(),
                &pack_paths,
            ) {
                warnings.push(format!("写入整合包清单失败：{error}"));
            }

            (parent, game_version.clone(), loader.clone())
        }

        SourceData::Modrinth {
            game_version,
            loader,
            loader_version,
            files,
        } => {
            ensure_vanilla(app, client, cfg, game_version, progress.clone()).await?;
            let (parent, requested, note) = ensure_pack_loader(
                app,
                client,
                cfg,
                loader_plan(game_version, loader, loader_version, options),
                progress.clone(),
            )
            .await?;
            warnings.extend(note);

            write_instance_json(cfg, &instance_id, &parent)?;
            instance::save(
                cfg,
                &instance_id,
                &InstanceMeta {
                    display_name: Some(display_name.clone()),
                    description: format!(
                        "从 Modrinth 整合包导入 · {game_version} · {}",
                        loader_tag(loader)
                    ),
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
            let mut planned: Vec<Planned> = Vec::new();
            let mut pack_paths = Vec::new();
            for (file_path, url, sha1, size, optional) in files {
                // 可选文件也记入清单，避免下次更新时被当成「已移除」
                pack_paths.push(file_path.clone());
                if url.is_empty() {
                    if !optional || options.install_optional {
                        failures.push(ImportFailure {
                            project_id: 0,
                            file_id: 0,
                            name: file_path.clone(),
                            reason: "清单里没有下载地址".into(),
                        });
                    }
                    continue;
                }
                if *optional && !options.install_optional {
                    continue;
                }
                let Some(relative) = safe_relative(file_path) else {
                    continue;
                };
                planned.push(Planned {
                    task: FileTask {
                        url: url.clone(),
                        dest: root.join(relative),
                        sha1: sha1.clone(),
                        size: *size,
                        label: file_path.clone(),
                    },
                    // Modrinth 清单不带项目 ID，只能按路径补下
                    project_id: 0,
                    file_id: 0,
                    relative: file_path.clone(),
                });
            }
            let tasks: Vec<FileTask> = planned.iter().map(|item| item.task.clone()).collect();
            if !tasks.is_empty() {
                download::begin_stage(app, &progress, "正在下载整合包内容…");
                let count = tasks.len() as u32;
                // 整合包动辄几百个文件，个别文件超时不该让整个导入失败：
                // 收集失败项，导入仍算完成，由下载面板提供「补下失败文件」
                let outcome = download::run_tasks_reporting(
                    app,
                    client,
                    tasks.clone(),
                    (cfg.concurrency as usize).clamp(1, download::MAX_CONCURRENCY),
                    cfg.use_mirror(),
                    progress.clone(),
                )
                .await?;
                installed_files += count.saturating_sub(outcome.failed.len() as u32);
                push_download_failures(&mut failures, &planned, outcome.failed);
            }
            download::begin_stage(app, &progress, "正在校验下载文件…");
            let broken =
                repair_broken_jars(client, cfg, &root, &tasks, progress.as_ref()).await;
            push_broken_files(&mut failures, &mut warnings, &planned, broken);

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

            // 记录清单，导入后也能用新版整合包继续更新；失败只提示，不影响已导入的文件
            if let Err(error) = write_pack_record(
                &root,
                &parsed.preview,
                game_version,
                loader,
                requested.as_deref(),
                &pack_paths,
            ) {
                warnings.push(format!("写入整合包清单失败：{error}"));
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
            let broken =
                repair_broken_jars(client, cfg, &instance_root, &[], progress.as_ref()).await;
            push_broken_files(&mut failures, &mut warnings, &[], broken);

            // 通用整合包沿用父实例的版本与加载器，父实例可能是 Forge/NeoForge 实例；
            // 包里的加载器启动器装不了时如实记下，版本列表与启动前提示都靠它
            let (game_version, parent_loader) = install::instance_context(cfg, &parent)
                .unwrap_or_else(|_| (parent.clone(), "vanilla".to_string()));
            let loader = if preview.loader_supported {
                parent_loader
            } else {
                preview.loader.clone()
            };
            (parent, game_version, loader)
        }
    };

    // 收尾：解压清单里的地图 zip，并把数据包复制进已有存档
    let instance_root = instance::game_dir(cfg, &instance_id);
    let root_for_sync = instance_root.clone();
    match tokio::task::spawn_blocking(move || sync_content_dirs(&root_for_sync)).await {
        Ok(Ok(())) => {}
        // 解压大存档可能很慢，放到阻塞线程池，避免卡住界面
        Ok(Err(error)) => warnings.push(format!("整理存档内容失败：{error}")),
        Err(error) => warnings.push(format!("整理存档内容失败：{error}")),
    }

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

    // 解析阶段失败的文件没有下载地址，进不了下载面板的重试批次；
    // 记到任务名下，任务卡片上可以整批补下，也可以逐个重试
    add_pending_specs(&progress.job, &instance_id, failure_specs(&failures));

    let hints = import_hints(cfg, &instance_id);
    Ok(ImportResult {
        instance_id,
        display_name,
        installed_files,
        failures,
        warnings,
        hints,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        add_pack_paths, broken_jars, detect_loader, extract_entries, generic_loader,
        global_datapack_dir, infer_subdir, memory_hint, mmc_loader, mrpack_candidate,
        normalize_loader_version, old_pack_manifest, pack_subdir, parse_pack, plan_update,
        sha1_file, sync_content_dirs, unsupported_loader_warning, write_pack_record, PackPreview,
        UpdateFile,
    };
    use crate::download::ProgressState;
    use std::io::Write;
    use std::path::PathBuf;

    /// 在临时目录里打一个 zip，条目按「路径 → 内容」顺序写入
    fn make_zip_bytes(label: &str, entries: &[(&str, &[u8])]) -> PathBuf {
        let dir = temp_dir(label);
        let path = dir.join("pack.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for (name, body) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(body).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    fn make_zip(label: &str, entries: &[(&str, &str)]) -> PathBuf {
        let bytes: Vec<(&str, &[u8])> = entries.iter().map(|(name, body)| (*name, body.as_bytes())).collect();
        make_zip_bytes(label, &bytes)
    }

    /// 通用压缩包里识别出的加载器
    fn loader_of(path: &std::path::Path, prefix: &str) -> Option<(String, Option<String>)> {
        let mut archive = super::open_zip(path.to_str().unwrap()).unwrap();
        let names = super::zip_names(&mut archive);
        generic_loader(&mut archive, &names, prefix)
    }

    /// 清单里的加载器 ID 要拆对：`fabric-loader-0.15.11` 的版本是 `0.15.11`。
    /// 以前从第一个连字符切，会把 `loader-0.15.11` 当版本号去请求加载器配置，整包导入失败。
    #[test]
    fn splits_loader_id_and_version() {
        assert_eq!(
            detect_loader("forge-47.4.20"),
            ("forge".into(), Some("47.4.20".into()))
        );
        assert_eq!(
            detect_loader("fabric-loader-0.15.11"),
            ("fabric".into(), Some("0.15.11".into()))
        );
        assert_eq!(
            detect_loader("quilt-loader-0.21.0"),
            ("quilt".into(), Some("0.21.0".into()))
        );
        // 老包用过 minecraftforge 这个前缀
        assert_eq!(
            detect_loader("minecraftforge-14.23.5.2855"),
            ("forge".into(), Some("14.23.5.2855".into()))
        );
        assert_eq!(detect_loader("forge"), ("forge".into(), None));
        // 只写名字不写版本
        assert_eq!(detect_loader("fabric-loader"), ("fabric".into(), None));
        assert_eq!(detect_loader("quilt-loader"), ("quilt".into(), None));
        // 带游戏版本的全坐标要去掉前缀
        let (loader, version) = detect_loader("forge-1.20.1-47.4.20");
        assert_eq!(
            normalize_loader_version("1.20.1", &loader, version),
            Some("47.4.20".into())
        );
    }

    /// 认不出的加载器保留真名，不能悄悄当成原版
    #[test]
    fn keeps_unknown_loader_names() {
        assert_eq!(
            detect_loader("cleanroom-0.7.14-alpha"),
            ("cleanroom".into(), Some("0.7.14-alpha".into()))
        );
        assert_eq!(
            detect_loader("liteloader-1.12.2-SNAPSHOT"),
            ("liteloader".into(), Some("1.12.2-SNAPSHOT".into()))
        );
        assert_eq!(
            detect_loader("rift-1.0.4-1.13"),
            ("rift".into(), Some("1.0.4-1.13".into()))
        );
        assert!(!crate::install::is_supported_loader("cleanroom"));
        assert!(unsupported_loader_warning("cleanroom").contains("Cleanroom"));
    }

    /// GTO 形态的 CurseForge 包：Forge 47.4.20、169 个必需文件 + 若干可选，
    /// overrides 里带 config/openloader。解析计数要对，覆盖文件要原样落盘。
    #[test]
    fn parses_gto_shaped_pack_and_extracts_overrides() {
        let mut files = String::new();
        for index in 0..169 {
            files.push_str(&format!(
                r#"{{"projectID": 1000,"fileID": {},"required": true}},"#,
                500_000 + index
            ));
        }
        for index in 0..3 {
            files.push_str(&format!(
                r#"{{"projectID": 2000,"fileID": {},"required": false}},"#,
                600_000 + index
            ));
        }
        let manifest = format!(
            r#"{{"minecraft":{{"version":"1.20.1","modLoaders":[{{"id":"forge-47.4.20","primary":true}}]}},
"manifestType":"minecraftModpack","manifestVersion":1,
"name":"GregTech Odyssey","version":"0.5.6-beta","overrides":"overrides",
"files":[{}]}}"#,
            files.trim_end_matches(',')
        );
        // 老配置文件常见 GBK：「中文」= D6 D0 CE C4。必须按字节原样落盘，不能被当成文本改写
        let gbk_toml: &[u8] = b"# \xD6\xD0\xCE\xC4\r\n[legacy]\r\n";
        let path = make_zip_bytes(
            "gto",
            &[
                ("manifest.json", manifest.as_bytes()),
                ("overrides/config/forge-common.toml", gbk_toml),
                (
                    "overrides/config/openloader/resources/gto/data/gto/recipes/gregtech.json",
                    br#"{"type":"minecraft:crafting_shaped"}"#,
                ),
                ("overrides/config/gregtech.toml", b"ticks { }\n"),
                ("overrides/config/paxi/datapacks/gto.zip", b"PK\x05\x06"),
            ],
        );
        let parsed = parse_pack(path.to_str().unwrap(), false).unwrap();
        let preview = &parsed.preview;
        assert_eq!(preview.format, "curseforge");
        assert_eq!(preview.name, "GregTech Odyssey");
        assert_eq!(preview.version, "0.5.6-beta");
        assert_eq!(preview.game_version, "1.20.1");
        assert_eq!(preview.loader, "forge");
        assert_eq!(preview.loader_version.as_deref(), Some("47.4.20"));
        assert!(preview.loader_supported);
        assert_eq!(preview.file_count, 172);
        assert_eq!(preview.optional_count, 3);
        assert!(preview.warnings.is_empty(), "{:?}", preview.warnings);

        let dest = temp_dir("gto-out");
        let progress = ProgressState::new("install");
        let written =
            extract_entries(path.to_str().unwrap(), "overrides/", &dest, false, &progress).unwrap();
        assert_eq!(written, 4);
        let openloader = dest
            .join("config/openloader/resources/gto/data/gto/recipes/gregtech.json");
        assert!(openloader.is_file(), "config/openloader 必须落到实例目录");
        assert_eq!(
            std::fs::read(dest.join("config/forge-common.toml")).unwrap(),
            gbk_toml
        );
        assert!(dest.join("config/paxi/datapacks/gto.zip").is_file());
        assert!(!dest.join("manifest.json").exists(), "清单本身不能进实例目录");

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let _ = std::fs::remove_dir_all(&dest);
    }

    /// 用了启动器装不了的加载器：预览如实报出名字并说明，不归成原版
    #[test]
    fn reports_unsupported_loader_in_manifests() {
        let curse = make_zip(
            "cleanroom-cf",
            &[
                (
                    "manifest.json",
                    r#"{"minecraft":{"version":"1.12.2","modLoaders":[{"id":"cleanroom-0.7.14-alpha","primary":true}]},"name":"CR","version":"1","overrides":"overrides","files":[{"projectID":1,"fileID":2,"required":true}]}"#,
                ),
            ],
        );
        let preview = parse_pack(curse.to_str().unwrap(), false).unwrap().preview;
        assert_eq!(preview.loader, "cleanroom");
        assert!(!preview.loader_supported);
        assert!(preview.warnings.iter().any(|item| item.contains("Cleanroom")));

        let mrpack = make_zip(
            "liteloader-mr",
            &[(
                "modrinth.index.json",
                r#"{"name":"LL","versionId":"1.0","dependencies":{"minecraft":"1.12.2","liteloader":"1.12.2-SNAPSHOT"},"files":[]}"#,
            )],
        );
        let preview = parse_pack(mrpack.to_str().unwrap(), false).unwrap().preview;
        assert_eq!(preview.loader, "liteloader");
        assert!(!preview.loader_supported);
        assert_eq!(preview.loader_version.as_deref(), Some("1.12.2-SNAPSHOT"));

        let _ = std::fs::remove_dir_all(curse.parent().unwrap());
        let _ = std::fs::remove_dir_all(mrpack.parent().unwrap());
    }

    /// 通用压缩包也要认加载器：自带 versions/ 的看主类，Prism / MultiMC 实例看补丁与组件
    #[test]
    fn generic_zip_reports_embedded_loader() {
        let self_contained = make_zip(
            "generic-version",
            &[
                (
                    "versions/Cleanroom/Cleanroom.json",
                    r#"{"id":"Cleanroom","mainClass":"top.outlands.foundation.boot.Foundation","libraries":[{"name":"com.cleanroommc:cleanroom-loader:0.7.14-alpha"}]}"#,
                ),
                ("mods/a.jar", "jar"),
            ],
        );
        assert_eq!(
            loader_of(&self_contained, ""),
            Some(("cleanroom".into(), None))
        );

        // Cleanroom 的 MultiMC 实例沿用 net.minecraftforge 这个 uid，只能靠补丁里的主类区分
        let mmc = make_zip(
            "generic-mmc",
            &[
                (
                    "instance/mmc-pack.json",
                    r#"{"formatVersion":1,"components":[{"uid":"net.minecraft","version":"1.12.2"},{"uid":"org.lwjgl3","version":"3.4.3"},{"uid":"net.minecraftforge","version":"0.7.14-alpha"}]}"#,
                ),
                (
                    "instance/patches/net.minecraftforge.json",
                    r#"{"name":"Cleanroom","mainClass":"top.outlands.foundation.boot.Foundation","libraries":[{"name":"com.cleanroommc:cleanroom-loader:0.7.14-alpha"}]}"#,
                ),
            ],
        );
        assert_eq!(
            loader_of(&mmc, "instance/").map(|(loader, _)| loader),
            Some("cleanroom".into())
        );

        // 普通 Forge 实例：补丁是 launchwrapper + forge 库
        let forge = make_zip(
            "generic-forge",
            &[(
                "patches/net.minecraftforge.json",
                r#"{"mainClass":"net.minecraft.launchwrapper.Launch","libraries":[{"name":"net.minecraftforge:forge:1.12.2-14.23.5.2860"}]}"#,
            )],
        );
        assert_eq!(
            loader_of(&forge, "").map(|(loader, _)| loader),
            Some("forge".into())
        );

        // 没有补丁时退回组件表；对不上任何已知加载器的主类记成 unknown
        let components = serde_json::json!({"components":[{"uid":"net.minecraftforge","version":"14.23.5.2860"}]});
        assert_eq!(
            mmc_loader(&components),
            Some(("forge".into(), Some("14.23.5.2860".into())))
        );
        let weird = make_zip(
            "generic-unknown",
            &[(
                "versions/Weird/Weird.json",
                r#"{"id":"Weird","mainClass":"com.example.launch.Boot"}"#,
            )],
        );
        assert_eq!(
            loader_of(&weird, "").map(|(loader, _)| loader),
            Some("unknown".into())
        );

        for path in [self_contained, mmc, forge, weird] {
            let _ = std::fs::remove_dir_all(path.parent().unwrap());
        }
    }

    /// 内存建议按模组数量分档：小包不啰嗦，手动分配够了也不提
    #[test]
    fn memory_hint_scales_with_mod_count() {
        assert!(memory_hint(30, false, 1024).is_none());
        assert!(memory_hint(80, false, 2048).unwrap().contains("4 GB"));
        assert!(memory_hint(120, false, 8192).is_none(), "分配够了就不必再提");
        assert!(memory_hint(200, true, 0).unwrap().contains("8 GB"));
    }

    /// 补下成功的文件要追加进整合包清单，且不能重复
    #[test]
    fn appends_retried_files_to_pack_record() {
        let root = temp_dir("pack-add");
        std::fs::create_dir_all(&root).unwrap();
        write_pack_record(
            &root,
            &PackPreview::default(),
            "1.20.1",
            "forge",
            Some("47.4.20"),
            &["mods/a.jar".into()],
        )
        .unwrap();
        add_pack_paths(&root, &["mods/b.jar".into(), "mods/a.jar".into()]).unwrap();
        add_pack_paths(&root, &["mods/b.jar".into()]).unwrap();
        let record: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join(".docl-pack.json")).unwrap(),
        )
        .unwrap();
        let paths: Vec<&str> = record["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["path"].as_str())
            .collect();
        assert_eq!(paths, vec!["mods/a.jar", "mods/b.jar"]);
        // 不是整合包实例（没有清单）时静默跳过
        let plain = temp_dir("pack-plain");
        add_pack_paths(&plain, &["mods/b.jar".into()]).unwrap();
        assert!(!plain.join(".docl-pack.json").exists());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&plain);
    }

    /// CurseForge 清单只有 projectID，光影/资源包/数据包/地图要落到对应目录
    #[test]
    fn curse_class_ids_map_to_instance_dirs() {
        let none: Vec<String> = Vec::new();
        assert_eq!(pack_subdir(Some(6), "a.jar", &none), "mods");
        assert_eq!(pack_subdir(Some(12), "a.zip", &none), "resourcepacks");
        assert_eq!(pack_subdir(Some(6552), "a.zip", &none), "shaderpacks");
        assert_eq!(pack_subdir(Some(6945), "a.zip", &none), "datapacks");
        // 地图是 zip，收尾阶段会解压成存档文件夹
        assert_eq!(pack_subdir(Some(17), "map.zip", &none), "saves");
        // 未知类别走内容判断，拿不到清单时按 Mod 兜底
        assert_eq!(pack_subdir(Some(9999), "a.jar", &none), "mods");
    }

    /// 数据包与资源包都带 pack.mcmeta，靠 data/ 区分（PCL 只看 pack.mcmeta，会误判成资源包）
    #[test]
    fn infers_subdir_from_archive_modules() {
        let datapack = vec![
            "pack.mcmeta".to_string(),
            "assets".to_string(),
            "data".to_string(),
        ];
        assert_eq!(infer_subdir("pack.zip", &datapack), "datapacks");

        let resourcepack = vec!["pack.mcmeta".to_string(), "assets".to_string()];
        assert_eq!(infer_subdir("pack.zip", &resourcepack), "resourcepacks");

        let shader = vec!["shaders".to_string(), "pack.mcmeta".to_string()];
        assert_eq!(infer_subdir("shader.zip", &shader), "shaderpacks");

        let map = vec!["level.dat".to_string(), "region".to_string()];
        assert_eq!(infer_subdir("world.zip", &map), "saves");

        let jar = vec!["META-INF".to_string(), "mezz".to_string()];
        assert_eq!(infer_subdir("jei.jar", &jar), "mods");
    }

    /// 收尾：数据包复制进每个已有存档，没有存档时保留在实例根目录
    #[test]
    fn copies_datapacks_into_existing_worlds() {
        let root = temp_dir("datapack");
        std::fs::create_dir_all(root.join("datapacks")).unwrap();
        std::fs::write(root.join("datapacks").join("pack.zip"), b"dp").unwrap();

        // 没有存档：不创建任何目录
        sync_content_dirs(&root).unwrap();
        assert!(!root.join("saves").exists());

        let world = root.join("saves").join("world");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::write(world.join("level.dat"), b"level").unwrap();
        sync_content_dirs(&root).unwrap();
        assert!(world.join("datapacks").join("pack.zip").is_file());

        // 源码仍保留在实例根目录，便于之后新建存档时再复制
        assert!(root.join("datapacks").join("pack.zip").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 地图 zip 只解压一次：原包要保留（否则每次更新都会重下），重复同步不能生成副本
    #[test]
    fn extracts_map_zip_once_and_keeps_archive() {
        let root = temp_dir("map-sync");
        let saves = root.join("saves");
        std::fs::create_dir_all(&saves).unwrap();
        let archive = saves.join("Adventure.zip");
        let file = std::fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("Adventure/level.dat", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"level").unwrap();
        zip.finish().unwrap();

        sync_content_dirs(&root).unwrap();
        assert!(saves.join("Adventure").join("level.dat").is_file());
        assert!(archive.is_file(), "地图 zip 不能删，否则更新会重复下载");

        sync_content_dirs(&root).unwrap();
        assert!(!saves.join("Adventure (2)").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 装了 Open Loader / Paxi 时写它们的全局目录，不再逐个存档复制，避免重复加载
    #[test]
    fn uses_global_loader_dir_when_present() {
        let root = temp_dir("global-dp");
        std::fs::create_dir_all(root.join("mods")).unwrap();
        std::fs::create_dir_all(root.join("datapacks")).unwrap();
        std::fs::write(root.join("datapacks").join("pack.zip"), b"dp").unwrap();
        let world = root.join("saves").join("world");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::write(world.join("level.dat"), b"level").unwrap();

        // Open Loader 优先
        let openloader = root.join("mods").join("openloader-1.20.1-1.1.1.jar");
        std::fs::write(&openloader, b"jar").unwrap();
        sync_content_dirs(&root).unwrap();
        assert!(root.join("config").join("openloader").join("data").join("pack.zip").is_file());
        assert!(!world.join("datapacks").join("pack.zip").exists());

        // 去掉 Open Loader、换成 Paxi
        std::fs::remove_file(&openloader).unwrap();
        std::fs::write(
            root.join("mods").join("Paxi-1.20.1-Forge-4.0.jar"),
            b"jar",
        )
        .unwrap();
        sync_content_dirs(&root).unwrap();
        let paxi = root.join("config").join("paxi").join("datapacks");
        assert!(paxi.join("pack.zip").is_file());
        assert_eq!(global_datapack_dir(&root), Some(paxi));
        let _ = std::fs::remove_dir_all(&root);
    }

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

    /// 只更新内容变化的文件；旧清单里已被移除的文件要单独列出
    #[test]
    fn plans_pack_update_from_disk_state() {
        let root = temp_dir("update-plan");
        std::fs::create_dir_all(root.join("mods")).unwrap();
        std::fs::write(root.join("mods").join("keep.jar"), b"keep").unwrap();
        std::fs::write(root.join("mods").join("old.jar"), b"old").unwrap();
        std::fs::write(root.join("mods").join("gone.jar"), b"gone").unwrap();

        let keep_sha = sha1_file(&root.join("mods").join("keep.jar")).unwrap();
        let files: Vec<UpdateFile> = vec![
            (
                "mods/keep.jar".into(),
                "https://example.com/keep.jar".into(),
                Some(keep_sha),
                4,
                false,
            ),
            (
                "mods/old.jar".into(),
                "https://example.com/old.jar".into(),
                Some("0000000000000000000000000000000000000000".into()),
                10,
                false,
            ),
            (
                "mods/new.jar".into(),
                "https://example.com/new.jar".into(),
                Some("1111111111111111111111111111111111111111".into()),
                20,
                false,
            ),
            // 可选文件默认不下载，但要记入清单，避免被当成「已移除」
            (
                "mods/optional.jar".into(),
                "https://example.com/optional.jar".into(),
                None,
                5,
                true,
            ),
        ];
        let mut old_paths = std::collections::HashSet::new();
        for path in ["mods/keep.jar", "mods/old.jar", "mods/gone.jar"] {
            old_paths.insert(path.to_string());
        }
        let old = super::OldPack {
            name: "测试包".into(),
            version: "1.0".into(),
            paths: old_paths,
        };

        let plan = plan_update(&root, &files, Some(&old), false);
        assert_eq!(plan.added, 1);
        assert_eq!(plan.changed, 1);
        assert_eq!(plan.removed, vec!["mods/gone.jar".to_string()]);
        // 可选文件被跳过，不产生下载任务
        assert_eq!(plan.tasks.len(), 2);
        assert!(plan.paths.contains("mods/optional.jar"));
        assert_eq!(plan.download_bytes, 10 + 20);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// HMCL 的 modpack.cfg 也能作为旧清单来源
    #[test]
    fn reads_old_manifest_from_hmcl_pack_cfg() {
        let root = temp_dir("old-manifest");
        std::fs::write(
            root.join("modpack.cfg"),
            serde_json::json!({
                "type": "Modrinth",
                "name": "Fabulously Optimized",
                "version": "14.1.2",
                "manifest": {
                    "files": [
                        { "path": "mods/a.jar" },
                        { "path": "mods/b.jar" }
                    ]
                }
            })
            .to_string(),
        )
        .unwrap();

        let pack = old_pack_manifest(&root).unwrap();
        assert_eq!(pack.name, "Fabulously Optimized");
        assert_eq!(pack.version, "14.1.2");
        assert!(pack.paths.contains("mods/a.jar"));
        assert!(pack.paths.contains("mods/b.jar"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 只有内容目录顶层的文件适合交给 Modrinth 生态，其余留在 overrides
    #[test]
    fn only_top_level_content_files_go_to_modrinth_index() {
        for path in [
            "mods/sodium.jar",
            "resourcepacks/pack.zip",
            "shaderpacks/bsl.zip",
            "datapacks/data.zip",
        ] {
            assert!(mrpack_candidate(path), "{path} 应该写进 files[]");
        }
        for path in [
            "config/a.toml",
            "saves/world/level.dat",
            "mods/sub/nested.jar",
            "mods/",
        ] {
            assert!(!mrpack_candidate(path), "{path} 不应写进 files[]");
        }
    }

    /// 导入后写下的清单要能被下次更新识别（名称、版本、文件路径）
    #[test]
    fn writes_pack_record_for_next_update() {
        let root = temp_dir("pack-record");
        let paths = vec![
            "mods/b.jar".to_string(),
            "mods/a.jar".to_string(),
            "mods/a.jar".to_string(),
        ];
        write_pack_record(
            &root,
            &super::PackPreview {
                format: "curseforge".into(),
                name: "测试包".into(),
                version: "2.0".into(),
                ..Default::default()
            },
            "1.20.1",
            "forge",
            Some("47.2.0"),
            &paths,
        )
        .unwrap();

        let pack = old_pack_manifest(&root).unwrap();
        assert_eq!(pack.name, "测试包");
        assert_eq!(pack.version, "2.0");
        assert_eq!(pack.paths.len(), 2);
        assert!(pack.paths.contains("mods/a.jar"));
        assert!(pack.paths.contains("mods/b.jar"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
