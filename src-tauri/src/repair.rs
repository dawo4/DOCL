use crate::config::Config;
use crate::download::{self, FileTask, ProgressState};
use crate::install;
use crate::instance;
use crate::models::{AssetIndexFile, DownloadRef, VersionDetail};
use crate::moddeps;
use anyhow::{Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::AppHandle;

#[derive(Debug, Default, Serialize)]
pub struct RepairReport {
    /// 版本清单是否被重新下载修复
    pub version_json: bool,
    /// 补回的文件数量（启动 jar + 依赖库）
    pub files_restored: usize,
    /// 仍然缺失或下载失败的文件
    pub files_failed: Vec<String>,
    /// 是否重新解压了本地库
    pub natives: bool,
    /// 被禁用的重复 mod（文件名）
    pub disabled_mods: Vec<String>,
    /// 因与当前游戏/加载器不兼容而被禁用的 mod（文件名）
    pub incompatible_mods: Vec<String>,
    /// 发现的重复 mod id
    pub duplicate_mods: Vec<String>,
    /// 缺少的必需前置
    pub missing_deps: Vec<String>,
    /// mods 目录里无法正常打开的 jar
    pub broken_mods: Vec<String>,
    /// 需要用户手动处理的问题
    pub issues: Vec<String>,
    /// 修复前的备份目录，None 表示没有改动任何文件
    pub backup_dir: Option<String>,
    /// 自动补装成功的前置
    pub deps_installed: Vec<String>,
    /// 自动切换后的 Java（含版本说明）
    pub java_switched: Option<String>,
    /// 是否重置了画质选项
    pub graphics_reset: bool,
    /// 补全的资源文件数量
    pub assets_restored: usize,
    /// 补全失败的资源文件数量
    pub assets_failed: usize,
    /// 是否清空过 config 目录
    pub config_reset: bool,
    /// 是否重装过加载器
    pub loader_reinstalled: bool,
}

/// 崩溃分析命中了哪些规则，决定要不要做针对性修复。
fn rule_hit(options: &RepairOptions, id: &str) -> bool {
    options.rules.iter().any(|item| item == id)
}

/// 一键修复的可选动作。默认全部关闭：下载量大或会改用户设置的操作必须显式选择。
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct RepairOptions {
    /// 自动补装缺失前置（联网下载 Mod）
    #[serde(default)]
    pub install_deps: bool,
    /// 把画质相关选项调低（会改 options.txt）
    #[serde(default)]
    pub reset_graphics: bool,
    /// 校验并补全资源文件（可能下载数百 MB）
    #[serde(default)]
    pub repair_assets: bool,
    /// 加载器损坏时重新安装（需要下载）
    #[serde(default)]
    pub reinstall_loader: bool,
    /// 崩溃分析命中的规则 id，用来做针对性修复
    #[serde(default)]
    pub rules: Vec<String>,
}

/// 修复前的文件备份：按「相对游戏根目录」的结构保存，还原时原样拷回。
struct BackupGuard {
    root: PathBuf,
    game_root: PathBuf,
    saved: usize,
}

impl BackupGuard {
    fn new(cfg: &Config, version_id: &str) -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_secs())
            .unwrap_or(0);
        let safe_id: String = version_id
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        Self {
            root: crate::config::data_dir()
                .join("backups")
                .join(format!("{safe_id}-{stamp}")),
            game_root: cfg.game_root(),
            saved: 0,
        }
    }

    /// 改动某个文件前先拷一份；文件不存在时跳过。
    /// 不在游戏根目录下的文件直接报错，由调用方跳过这次改动——
    /// 否则还原时会被写到错误的位置。
    fn save(&mut self, path: &Path) -> Result<()> {
        if !path.is_file() {
            return Ok(());
        }
        let rel = path.strip_prefix(&self.game_root).map_err(|_| {
            anyhow::anyhow!("文件不在游戏目录内：{}", path.display())
        })?;
        let dest = self.root.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(path, &dest)?;
        self.saved += 1;
        Ok(())
    }

    fn path(&self) -> Option<String> {
        (self.saved > 0).then(|| self.root.to_string_lossy().to_string())
    }
}

/// 递归备份一个目录，全部成功返回文件数；有一个失败就返回 None。
fn backup_tree(dir: &Path, backup: &mut BackupGuard, depth: u8) -> Option<usize> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut count = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                count += backup_tree(&path, backup, depth - 1)?;
            }
            continue;
        }
        backup.save(&path).ok()?;
        count += 1;
    }
    Some(count)
}

/// 按文件名关键字禁用模组（用于 OptiFine / ShadersMod 这类已知冲突）。
fn disable_matching_mods(
    cfg: &Config,
    version_id: &str,
    mods_dir: &Path,
    keywords: &[&str],
    backup: &mut BackupGuard,
    report: &mut RepairReport,
) {
    let Ok(entries) = std::fs::read_dir(mods_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_ascii_lowercase();
        if !lower.ends_with(".jar") || !keywords.iter().any(|key| lower.contains(key)) {
            continue;
        }
        if report.incompatible_mods.iter().any(|item| item == &name) {
            continue;
        }
        if backup.save(&path).is_err() {
            report.issues.push(format!("无法备份 {name}，已跳过禁用"));
            continue;
        }
        match instance::toggle_file(cfg, version_id, "mods", &name) {
            Ok(_) => report.incompatible_mods.push(name),
            Err(err) => report.issues.push(format!("无法禁用 {name}：{err}")),
        }
    }
}

pub fn restore(cfg: &Config, version_id: &str) -> Result<usize> {
    let dir = latest_backup(version_id)
        .ok_or_else(|| anyhow::anyhow!("没有找到「{version_id}」的修复备份"))?;
    let game_root = cfg.game_root();
    let mut count = 0usize;
    restore_dir(&dir, &dir, &game_root, &mut count)?;
    Ok(count)
}

fn restore_dir(root: &Path, dir: &Path, game_root: &Path, count: &mut usize) -> Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            restore_dir(root, &path, game_root, count)?;
            continue;
        }
        let rel = path.strip_prefix(root)?;
        let dest = game_root.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&path, &dest)?;
        // 修复时被禁用的文件会多出 .disabled，还原时一并清掉
        let _ = std::fs::remove_file(format!("{}.disabled", dest.to_string_lossy()));
        *count += 1;
    }
    Ok(())
}

fn latest_backup(version_id: &str) -> Option<PathBuf> {
    let safe_id: String = version_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let prefix = format!("{safe_id}-");
    let mut list: Vec<PathBuf> = std::fs::read_dir(crate::config::data_dir().join("backups"))
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .map(|name| name.to_string_lossy().starts_with(&prefix))
                    .unwrap_or(false)
        })
        .collect();
    list.sort();
    list.pop()
}

/// 把画质相关的几个键调低；只改已存在的键，不新增，避免污染不认识的选项。
fn reset_graphics(path: &Path) -> bool {
    const TARGETS: &[(&str, &str)] = &[
        ("renderDistance", "8"),
        ("graphicsMode", "0"),
        ("ao", "0"),
        ("entityShadows", "false"),
        ("particles", "2"),
        ("fancyGraphics", "false"),
    ];
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let mut changed = false;
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some((key, value)) = line.split_once(':') {
            if let Some((_, target)) = TARGETS.iter().find(|(name, _)| *name == key) {
                if value != *target {
                    changed = true;
                }
                out.push(format!("{key}:{target}"));
                continue;
            }
        }
        out.push(line.to_string());
    }
    if !changed {
        return false;
    }
    std::fs::write(path, out.join("\n")).is_ok()
}

/// 校验资源索引与对象文件，缺什么补什么。返回（补齐数，失败数）。
async fn repair_assets(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    detail: &VersionDetail,
    progress: Arc<ProgressState>,
) -> Result<(usize, usize)> {
    let Some(index) = detail.asset_index.as_ref() else {
        return Ok((0, 0));
    };
    let root = cfg.game_root().join("assets");
    let index_path = root.join("indexes").join(format!("{}.json", index.id));
    if needs_rebuild(&index_path, index.size) {
        progress.set_message("正在下载资源索引…");
        if let Some(parent) = index_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let task = FileTask {
            url: index.url.clone(),
            dest: index_path.clone(),
            sha1: Some(index.sha1.clone()),
            size: index.size,
            label: format!("{}.json", index.id),
        };
        download::run_tasks(app, client, vec![task], 1, cfg.use_mirror(), progress.clone()).await?;
    }

    let bytes = tokio::fs::read(&index_path).await?;
    let parsed: AssetIndexFile = serde_json::from_slice(&bytes).context("资源索引格式异常")?;
    let mut tasks: Vec<FileTask> = Vec::new();
    for object in parsed.objects.values() {
        if object.hash.len() < 2 {
            continue;
        }
        let prefix = &object.hash[..2];
        let dest = root.join("objects").join(prefix).join(&object.hash);
        if !needs_rebuild(&dest, object.size) {
            continue;
        }
        tasks.push(FileTask {
            url: format!(
                "https://resources.download.minecraft.net/{prefix}/{}",
                object.hash
            ),
            dest,
            sha1: Some(object.hash.clone()),
            size: object.size,
            label: object.hash.clone(),
        });
    }
    if tasks.is_empty() {
        return Ok((0, 0));
    }
    let total = tasks.len();
    progress.set_message(format!("正在补全 {total} 个资源文件…"));
    let outcome =
        download::run_tasks_reporting(app, client, tasks, 6, cfg.use_mirror(), progress.clone())
            .await?;
    let failed = outcome.failed.len();
    Ok((total.saturating_sub(failed), failed))
}

/// 无法正常打开的 mod jar：中央目录损坏时游戏会直接抛 ZipException。
fn broken_mod_jars(dir: &Path) -> Vec<String> {
    let mut broken = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return broken;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.to_lowercase().ends_with(".jar") {
            continue;
        }
        let Ok(file) = std::fs::File::open(&path) else {
            broken.push(name);
            continue;
        };
        if zip::ZipArchive::new(file).is_err() {
            broken.push(name);
        }
    }
    broken
}

fn version_json_path(cfg: &Config, version_id: &str) -> PathBuf {
    install::resolve_version_dir(cfg, version_id).join(format!("{version_id}.json"))
}

fn read_detail(cfg: &Config, version_id: &str) -> Option<VersionDetail> {
    let bytes = std::fs::read(version_json_path(cfg, version_id)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// 清单缺失或损坏时，从官方版本清单重新拉回（只对原版版本有效）。
async fn restore_version_json(
    client: &reqwest::Client,
    cfg: &Config,
    version_id: &str,
) -> Option<VersionDetail> {
    let use_mirror = cfg.use_mirror();
    let manifest = install::fetch_manifest(client, use_mirror).await.ok()?;
    let entry = manifest
        .versions
        .into_iter()
        .find(|item| item.id == version_id)?;
    let raw = download::get_bytes(client, &entry.url, use_mirror)
        .await
        .ok()?;
    let detail: VersionDetail = serde_json::from_slice(&raw).ok()?;
    let path = version_json_path(cfg, version_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    std::fs::write(&path, &raw).ok()?;
    Some(detail)
}

/// 文件不存在、为空、或大小与清单不符都算需要补下。
/// 按大小判断而不校验 sha1：整库校验太慢，且启动失败多半就是文件根本没下完。
fn needs_rebuild(path: &Path, size: u64) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return true;
    };
    if meta.len() == 0 {
        return true;
    }
    size > 0 && meta.len() != size
}

fn label_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn dir_has_files(dir: &Path, depth: u8) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            return true;
        }
        if depth > 0 && path.is_dir() && dir_has_files(&path, depth - 1) {
            return true;
        }
    }
    false
}

fn mtime(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_secs())
        .unwrap_or(0)
}

/// 1.17 起游戏跑在 Java 模块系统上，CustomSkinLoader 的 ForgeV1 / Legacy
/// 分支会和原版模块抢同一个包，直接抛 ResolutionException。
fn needs_modern_csl_check(game_version: &str, loader: &str) -> bool {
    let loader = loader.to_ascii_lowercase();
    if loader != "forge" && loader != "neoforge" {
        return false;
    }
    let mut parts = game_version.split('.');
    let major = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    let minor = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    major == 1 && minor >= 17
}

/// CustomSkinLoader 的 ForgeV1 / Legacy 分支只适用于 1.16 及以下。
fn is_legacy_customskinloader(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".jar")
        && lower.contains("customskinloader")
        && (lower.contains("forgev1") || lower.contains("legacy"))
}

fn summary(report: &RepairReport) -> String {
    let mut parts: Vec<String> = Vec::new();
    if report.version_json {
        parts.push("版本清单已修复".into());
    }
    if report.files_restored > 0 {
        parts.push(format!("补回 {} 个文件", report.files_restored));
    }
    if report.natives {
        parts.push("本地库已重新解压".into());
    }
    if !report.disabled_mods.is_empty() {
        parts.push(format!("禁用 {} 个重复 mod", report.disabled_mods.len()));
    }
    if !report.incompatible_mods.is_empty() {
        parts.push(format!(
            "禁用 {} 个不兼容 mod",
            report.incompatible_mods.len()
        ));
    }
    if !report.missing_deps.is_empty() {
        parts.push(format!("缺 {} 个前置", report.missing_deps.len()));
    }
    if !report.broken_mods.is_empty() {
        parts.push(format!("{} 个 Mod 文件损坏", report.broken_mods.len()));
    }
    if !report.deps_installed.is_empty() {
        parts.push(format!("补装 {} 个前置", report.deps_installed.len()));
    }
    if report.java_switched.is_some() {
        parts.push("已切换 Java".into());
    }
    if report.graphics_reset {
        parts.push("画质已调低".into());
    }
    if report.assets_restored > 0 {
        parts.push(format!("补全 {} 个资源文件", report.assets_restored));
    }
    if report.config_reset {
        parts.push("已重置 Mod 配置".into());
    }
    if report.loader_reinstalled {
        parts.push("已重装加载器".into());
    }
    if !report.issues.is_empty() || !report.files_failed.is_empty() {
        parts.push("仍有问题需要处理".into());
    }
    if parts.is_empty() {
        "检查完成，没有发现问题".into()
    } else {
        parts.join("，")
    }
}

fn push_library_task(
    tasks: &mut Vec<FileTask>,
    natives: &mut Vec<(PathBuf, Vec<String>)>,
    dest: PathBuf,
    dl: &DownloadRef,
    exclude: Vec<String>,
    native: bool,
) {
    if needs_rebuild(&dest, dl.size) {
        tasks.push(FileTask {
            url: dl.url.clone(),
            dest: dest.clone(),
            sha1: Some(dl.sha1.clone()),
            size: dl.size,
            label: label_of(&dest),
        });
    }
    if native {
        natives.push((dest, exclude));
    }
}

pub async fn run(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    version_id: &str,
    options: &RepairOptions,
    progress: Arc<ProgressState>,
) -> Result<RepairReport> {
    let mut report = RepairReport::default();
    let mut backup = BackupGuard::new(cfg, version_id);
    let use_mirror = cfg.use_mirror();
    let game_root = cfg.game_root();
    let version_dir = install::resolve_version_dir(cfg, version_id);

    // 1) 版本清单：损坏或缺失时重新拉一份原版 JSON
    progress.set_message("正在检查版本清单…");
    let mut own = read_detail(cfg, version_id);
    if own.is_none() {
        if let Some(fresh) = restore_version_json(client, cfg, version_id).await {
            report.version_json = true;
            own = Some(fresh);
        }
    }
    let Some(own) = own else {
        report.issues.push(
            "版本清单缺失或已损坏，且不在官方清单里（加载器版本需要重新安装）".into(),
        );
        progress.set_message(summary(&report));
        return Ok(report);
    };

    // 父版本不在本地时，合并必然失败，直接给出可读原因
    if let Some(parent) = own.inherits_from.clone() {
        if read_detail(cfg, &parent).is_none() {
            report
                .issues
                .push(format!("依赖的父版本 {parent} 不存在，请先安装它"));
            progress.set_message(summary(&report));
            return Ok(report);
        }
    }

    let merged = match install::resolve_detail(cfg, version_id) {
        Ok(value) => value,
        Err(err) => {
            report.issues.push(format!("版本结构异常：{err}"));
            progress.set_message(summary(&report));
            return Ok(report);
        }
    };

    // Java：实例手动指定的运行时与要求不符时，自动换成本机匹配的版本
    if let Some(current) = instance::get(cfg, version_id)
        .java_path
        .filter(|path| !path.trim().is_empty())
    {
        let required = merged.detail.required_java();
        let current_major = crate::java::inspect_java(std::path::Path::new(&current), "手动指定")
            .map(|info| info.major);
        // 必须精确匹配：Java 过高（要求 8 用 17、要求 17 用 21）同样会崩
        let wrong_major = current_major != Some(required);
        // 崩溃规则指出位数不对时，即使大版本一致也要换成 64 位
        let bit_rule = rule_hit(options, "java-bitness-mismatch")
            || rule_hit(options, "java-32bit-lowmem");
        let wrong_bits = bit_rule
            && crate::java::resolve_java_path(&current)
                .map(|exe| crate::preflight::is_32bit_java(&exe))
                .unwrap_or(false);
        if wrong_major || wrong_bits {
            progress.set_message("正在匹配可用的 Java…");
            let detected = crate::java::detect(cfg);
            // 位数不对时优先同大版本的 64 位运行时
            let prefer64 = wrong_bits
                .then(|| {
                    detected.iter().find(|info| {
                        info.major == required
                            && crate::java::resolve_java_path(&info.path)
                                .map(|exe| !crate::preflight::is_32bit_java(&exe))
                                .unwrap_or(false)
                    })
                })
                .flatten()
                .cloned();
            let picked = prefer64.or_else(|| crate::java::pick_for_major(&detected, required));
            match picked {
                Some(info) if info.path != current => {
                    let mut meta = instance::get(cfg, version_id);
                    meta.java_path = Some(info.path.clone());
                    if instance::save(cfg, version_id, &meta).is_ok() {
                        report.java_switched = Some(format!("Java {} · {}", info.major, info.path));
                    }
                }
                Some(_) => {}
                None => report.issues.push(format!(
                    "当前 Java 不满足游戏要求的 Java {required}，本机也没有找到匹配的运行时，请到「设置 → Java」安装"
                )),
            }
        }
    }

    // 画质重置（可选）：只改已存在的键，不新增
    if options.reset_graphics {
        progress.set_message("正在重置画质选项…");
        let options_path = instance::game_dir(cfg, version_id).join("options.txt");
        if options_path.is_file() && backup.save(&options_path).is_err() {
            report.issues.push("无法备份 options.txt，已跳过画质重置".into());
        } else {
            report.graphics_reset = reset_graphics(&options_path);
        }
    }

    progress.set_message("正在核对游戏核心与依赖库…");

    // 2) 启动 jar：只有版本自身清单带 client 下载时才补，
    // 加载器的补丁 jar 由安装器产出，拿原版 jar 顶上会直接崩
    let mut tasks: Vec<FileTask> = Vec::new();
    let mut natives: Vec<(PathBuf, Vec<String>)> = Vec::new();
    let jar_dest = version_dir.join(format!("{version_id}.jar"));
    if let Some(dl) = own.downloads.as_ref().and_then(|item| item.client.as_ref()) {
        push_library_task(&mut tasks, &mut natives, jar_dest.clone(), dl, Vec::new(), false);
    } else if own.inherits_from.is_some() && !jar_dest.exists() {
        report
            .issues
            .push("加载器生成的启动 jar 缺失，请重新安装该加载器版本".into());
    }

    // 3) 依赖库：与安装流程一致的判定（规则、native、maven 路径）
    for lib in &merged.detail.libraries {
        if !install::rules_allow(lib.rules.as_ref()) {
            continue;
        }
        let native = install::library_is_native(lib);
        let chosen = if native {
            install::library_native_download(lib)
        } else {
            install::library_artifact(lib)
        };
        let Some((dl, path)) = chosen else {
            continue;
        };
        if path.is_empty() || dl.url.is_empty() {
            continue;
        }
        let dest = game_root.join("libraries").join(path.replace('/', "\\"));
        let exclude = lib
            .extract
            .as_ref()
            .and_then(|item| item.exclude.clone())
            .unwrap_or_else(|| vec!["META-INF/".to_string()]);
        push_library_task(&mut tasks, &mut natives, dest, &dl, exclude, native);
    }

    let total = tasks.len();
    if total > 0 {
        progress.set_message(format!("正在补下 {total} 个缺失文件…"));
        let outcome = download::run_tasks_reporting(
            app,
            client,
            tasks,
            (cfg.concurrency as usize).max(8),
            use_mirror,
            progress.clone(),
        )
        .await?;
        report.files_restored = total.saturating_sub(outcome.failed.len());
        report.files_failed = outcome
            .failed
            .into_iter()
            .map(|(label, _)| label)
            .collect();
    }

    // 4) 本地库解压：目录为空，或刚才补过依赖库时重解一遍
    let natives_dir = install::natives_dir(cfg, version_id);
    if !natives.is_empty() && (total > 0 || !dir_has_files(&natives_dir, 2)) {
        progress.set_message("正在重新解压本地库…");
        tokio::fs::create_dir_all(&natives_dir).await?;
        for (jar, exclude) in natives {
            if !jar.exists() {
                continue;
            }
            let target = natives_dir.clone();
            let label = jar.display().to_string();
            tokio::task::spawn_blocking(move || install::extract_natives(&jar, &target, &exclude))
                .await
                .with_context(|| format!("解压本地库失败：{label}"))??;
        }
        report.natives = true;
    }

    // 5) 模组：重复 id 禁用多余的那份，缺失前置只提示（避免乱装）
    progress.set_message("正在检查模组冲突…");
    // 未启用版本隔离时 mods 目录属于整个游戏根目录，自动禁用会影响所有版本
    let isolated = instance::get(cfg, version_id)
        .isolated
        .unwrap_or(cfg.version_isolation);
    let (game_version, loader) = install::instance_context(cfg, version_id)
        .unwrap_or_else(|_| (version_id.to_string(), String::new()));
    if let Ok(mods_dir) = instance::content_dir(cfg, version_id, "mods") {
        let scan_dir = mods_dir.clone();
        let loader_for_deps = loader.clone();
        let (dups, missing, broken) = tokio::task::spawn_blocking(move || {
            (
                moddeps::duplicate_mods(&scan_dir),
                moddeps::missing_requirements(&scan_dir, &loader_for_deps),
                broken_mod_jars(&scan_dir),
            )
        })
        .await?;
        report.missing_deps = missing;
        report.broken_mods = broken;
        if !report.broken_mods.is_empty() {
            report.issues.push(format!(
                "这些 Mod 文件已损坏，建议删除后重新下载：{}",
                report.broken_mods.join("、")
            ));
        }
        // 自动补前置是可选动作，默认只提示
        if options.install_deps && !report.missing_deps.is_empty() && !isolated {
            // 与禁用重复 Mod 保持一致：共享 mods 目录不能偷偷塞文件
            report.issues.push(
                "未启用版本隔离，自动补装会写入共享 mods 目录，已跳过；请手动确认或开启版本隔离后再试"
                    .into(),
            );
        } else if options.install_deps && !report.missing_deps.is_empty() {
            progress.set_message("正在补齐缺失前置…");
            let pending = report.missing_deps.clone();
            let deps = moddeps::install_requirements(
                app,
                client,
                cfg,
                version_id,
                &pending,
                progress.clone(),
            )
            .await;
            report.deps_installed = deps.installed.clone();
            let mut remaining = deps.failed.clone();
            remaining.extend(deps.unresolved.iter().cloned());
            report.missing_deps = remaining;
        }
        if isolated {
            for (id, files) in dups {
                report.duplicate_mods.push(id);
                // 保留修改时间最新的那份，其余禁用（.jar → .jar.disabled）
                let keep = files.iter().max_by_key(|path| mtime(path)).cloned();
                for file in files {
                    if Some(&file) == keep.as_ref() {
                        continue;
                    }
                    let Some(name) =
                        file.file_name().map(|value| value.to_string_lossy().to_string())
                    else {
                        continue;
                    };
                    if backup.save(&file).is_err() {
                        report
                            .issues
                            .push(format!("无法备份 {name}，已跳过禁用"));
                        continue;
                    }
                    match instance::toggle_file(cfg, version_id, "mods", &name) {
                        Ok(_) => report.disabled_mods.push(name),
                        Err(err) => report
                            .issues
                            .push(format!("无法禁用 {name}：{err}")),
                    }
                }
            }
        } else {
            for (id, _) in dups {
                report.duplicate_mods.push(id);
            }
            if !report.duplicate_mods.is_empty() {
                report.issues.push(
                    "该实例未启用版本隔离，重复模组会影响同目录所有版本，未自动禁用".into(),
                );
            }
        }
    }

    // 6) 已知不兼容：1.17+ 的 Forge/NeoForge 实例里，CustomSkinLoader 的
    // ForgeV1 / Legacy 分支会和模块系统冲突，直接禁用并告知用户
    if needs_modern_csl_check(&game_version, &loader) {
        if let Ok(mods_dir) = instance::content_dir(cfg, version_id, "mods") {
            progress.set_message("正在检查已知不兼容模组…");
            let candidates = legacy_customskinloader_files(&mods_dir);
            if isolated {
                for name in candidates {
                    if backup.save(&mods_dir.join(&name)).is_err() {
                        report
                            .issues
                            .push(format!("无法备份 {name}，已跳过禁用"));
                        continue;
                    }
                    match instance::toggle_file(cfg, version_id, "mods", &name) {
                        Ok(_) => report.incompatible_mods.push(name),
                        Err(err) => report
                            .issues
                            .push(format!("无法禁用不兼容模组 {name}：{err}")),
                    }
                }
            } else if !candidates.is_empty() {
                report.issues.push(format!(
                    "检测到不兼容的 {}：1.17 及以上无法加载 CustomSkinLoader 的 ForgeV1 / Legacy 版本；该实例未启用版本隔离，未自动禁用",
                    candidates.join("、")
                ));
            }
        }
    }

    // 已知渲染冲突：按崩溃规则直接禁用其中一方
    if isolated {
        let mut keywords: Vec<&str> = Vec::new();
        if rule_hit(options, "optifine-sodium") {
            keywords.push("optifine");
        }
        if rule_hit(options, "shadersmod-optifine") {
            keywords.push("shadersmod");
        }
        if !keywords.is_empty() {
            progress.set_message("正在处理渲染冲突模组…");
            if let Ok(mods_dir) = instance::content_dir(cfg, version_id, "mods") {
                disable_matching_mods(
                    cfg,
                    version_id,
                    &mods_dir,
                    &keywords,
                    &mut backup,
                    &mut report,
                );
            }
        }
    }

    // 配置损坏：备份后清空 config，让 Mod 重新生成；备份不完整就不动
    if rule_hit(options, "night-config-bug") || rule_hit(options, "mod-config-corrupt") {
        progress.set_message("正在重置 Mod 配置…");
        let config_dir = instance::game_dir(cfg, version_id).join("config");
        if config_dir.is_dir() {
            match backup_tree(&config_dir, &mut backup, 2) {
                None => report
                    .issues
                    .push("config 目录有文件无法备份，已跳过重置".into()),
                Some(0) => {}
                Some(_) => match std::fs::remove_dir_all(&config_dir) {
                    Ok(()) => report.config_reset = true,
                    Err(err) => report
                        .issues
                        .push(format!("无法清理 config 目录：{err}")),
                },
            }
        } else {
            report
                .issues
                .push("没有找到 config 目录，可能该实例不使用独立配置".into());
        }
    }

    // 加载器损坏：可选重装（只清加载器产物，保留 mods 与存档）
    if options.reinstall_loader
        && (rule_hit(options, "forge-incomplete")
            || rule_hit(options, "multiple-forge-in-json")
            || rule_hit(options, "loader-incomplete"))
    {
        progress.set_message("正在重装加载器…");
        match install::reinstall_loader(app, client, cfg, version_id, progress.clone()).await {
            Ok(_) => report.loader_reinstalled = true,
            Err(err) => report.issues.push(format!("重装加载器失败：{err}")),
        }
    }

    // 资源文件补全（可选，可能下载数百 MB）
    if options.repair_assets {
        progress.set_message("正在校验资源文件…");
        match repair_assets(app, client, cfg, &merged.detail, progress.clone()).await {
            Ok((restored, failed)) => {
                report.assets_restored = restored;
                report.assets_failed = failed;
                if failed > 0 {
                    report
                        .issues
                        .push(format!("有 {failed} 个资源文件下载失败，可以稍后再试一次"));
                }
            }
            Err(err) => report.issues.push(format!("补全资源文件失败：{err}")),
        }
    }

    // 7) 修完再自检一次，只把阻断级问题抛给用户；警告项留给崩溃分析页
    progress.set_message("正在复查修复结果…");
    for issue in crate::preflight::check(cfg, version_id).issues {
        if issue.blocking {
            report
                .issues
                .push(format!("复查未通过：{}。{}", issue.title, issue.detail));
        }
    }
    report.backup_dir = backup.path();

    progress.set_message(summary(&report));
    Ok(report)
}

/// mods 目录里启用的 CustomSkinLoader ForgeV1 / Legacy jar。
fn legacy_customskinloader_files(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| is_legacy_customskinloader(name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_or_mismatched_size_needs_rebuild() {
        let dir = std::env::temp_dir().join("docl-repair-test");
        std::fs::create_dir_all(&dir).unwrap();
        let empty = dir.join("empty.jar");
        std::fs::write(&empty, b"").unwrap();
        assert!(needs_rebuild(&empty, 0));

        let full = dir.join("full.jar");
        std::fs::write(&full, b"abc").unwrap();
        assert!(!needs_rebuild(&full, 3));
        assert!(needs_rebuild(&full, 5));
        assert!(needs_rebuild(&dir.join("missing.jar"), 0));
    }

    #[test]
    fn summary_lists_actions() {
        let mut report = RepairReport::default();
        assert_eq!(summary(&report), "检查完成，没有发现问题");
        report.files_restored = 2;
        report.disabled_mods = vec!["a.jar".into()];
        report.incompatible_mods = vec!["CustomSkinLoader_ForgeV1-14.17.2.jar".into()];
        assert!(summary(&report).contains("补回 2 个文件"));
        assert!(summary(&report).contains("禁用 1 个重复 mod"));
        assert!(summary(&report).contains("禁用 1 个不兼容 mod"));
    }

    /// 只有 CustomSkinLoader 的旧分支会被判定为不兼容
    #[test]
    fn flags_legacy_customskinloader_files() {
        assert!(is_legacy_customskinloader(
            "CustomSkinLoader_ForgeV1-14.17.2.jar"
        ));
        assert!(is_legacy_customskinloader(
            "CustomSkinLoader_ForgeLegacy-14.17.2.jar"
        ));
        assert!(!is_legacy_customskinloader(
            "CustomSkinLoader_ForgeV2-14.17.2.jar"
        ));
        assert!(!is_legacy_customskinloader(
            "CustomSkinLoader_ForgeV1-14.17.2.jar.disabled"
        ));
        assert!(!is_legacy_customskinloader("radon-1.0.jar"));
    }

    /// 1.16 及以下用 ForgeV1 是正常的，只有 1.17+ 的 Forge/NeoForge 才会冲突
    #[test]
    fn legacy_csl_only_conflicts_on_modern_forge() {
        assert!(needs_modern_csl_check("1.18.2", "forge"));
        assert!(needs_modern_csl_check("1.17", "neoforge"));
        assert!(!needs_modern_csl_check("1.16.5", "forge"));
        assert!(!needs_modern_csl_check("1.18.2", "fabric"));
        assert!(!needs_modern_csl_check("1.18.2", ""));
    }

    /// 备份树按相对游戏目录还原，并清掉修复时产生的 .disabled
    #[test]
    fn restores_files_from_backup_tree() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let base = std::env::temp_dir().join(format!("docl-repair-restore-{stamp}"));
        let backup = base.join("backup");
        let game = base.join("game");
        let rel = Path::new("versions").join("testpack").join("mods").join("foo.jar");
        std::fs::create_dir_all(backup.join(&rel).parent().unwrap()).unwrap();
        std::fs::write(backup.join(&rel), b"original").unwrap();
        std::fs::create_dir_all(game.join(&rel).parent().unwrap()).unwrap();
        std::fs::write(format!("{}.disabled", game.join(&rel).display()), b"x").unwrap();

        let mut count = 0usize;
        restore_dir(&backup, &backup, &game, &mut count).unwrap();
        assert_eq!(count, 1);
        assert_eq!(std::fs::read(game.join(&rel)).unwrap(), b"original");
        assert!(!Path::new(&format!("{}.disabled", game.join(&rel).display())).exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 游戏目录外的文件不能备份，否则还原会写到错误位置
    #[test]
    fn backup_rejects_files_outside_game_root() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let base = std::env::temp_dir().join(format!("docl-repair-outside-{stamp}"));
        std::fs::create_dir_all(&base).unwrap();
        let outside = base.join("outside.jar");
        std::fs::write(&outside, b"x").unwrap();

        let mut guard = BackupGuard {
            root: base.join("backup"),
            game_root: base.join("game"),
            saved: 0,
        };
        assert!(guard.save(&outside).is_err());
        assert!(guard.path().is_none(), "没有成功备份就不该给还原入口");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 画质重置只改已知键，不动其它设置；已经是目标值时不再写盘
    #[test]
    fn reset_graphics_lowers_existing_keys_only() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("docl-repair-graphics-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("options.txt");
        std::fs::write(
            &path,
            "renderDistance:16\nfov:70\nentityShadows:true\ngraphicsMode:1",
        )
        .unwrap();

        assert!(reset_graphics(&path));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("renderDistance:8"));
        assert!(text.contains("entityShadows:false"));
        assert!(text.contains("fov:70"), "不认识的键要保持原样");
        assert!(!reset_graphics(&path), "已经是目标值不应重复写");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 不是合法 zip 的 jar 会被标记为损坏，其它文件忽略
    #[test]
    fn detects_broken_mod_jars() {
        use std::io::Write;

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("docl-repair-broken-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();

        {
            let file = std::fs::File::create(dir.join("good.jar")).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file("a.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"ok").unwrap();
            zip.finish().unwrap();
        }
        std::fs::write(dir.join("bad.jar"), b"not a zip").unwrap();
        std::fs::write(dir.join("note.txt"), b"ignore").unwrap();

        let broken = broken_mod_jars(&dir);
        assert_eq!(broken, vec!["bad.jar".to_string()]);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
