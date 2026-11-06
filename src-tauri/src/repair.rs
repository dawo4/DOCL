use crate::config::Config;
use crate::download::{self, FileTask, ProgressState};
use crate::install;
use crate::instance;
use crate::models::{DownloadRef, VersionDetail};
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
    /// 需要用户手动处理的问题
    pub issues: Vec<String>,
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
    progress: Arc<ProgressState>,
) -> Result<RepairReport> {
    let mut report = RepairReport::default();
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
        let (dups, missing) = tokio::task::spawn_blocking(move || {
            (
                moddeps::duplicate_mods(&scan_dir),
                moddeps::missing_requirements(&scan_dir, &loader_for_deps),
            )
        })
        .await?;
        report.missing_deps = missing;
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
}
