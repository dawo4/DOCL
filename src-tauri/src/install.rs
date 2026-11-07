use crate::config::Config;
use crate::download::{self, FileTask, ProgressState};
use crate::launch;
use crate::models::*;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{copy, Write};
use std::path::{Path, PathBuf};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::UNIX_EPOCH;
use tauri::AppHandle;

const MANIFEST_URL: &str = "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Debug, Clone, Serialize)]
pub struct InstalledVersion {
    pub id: String,
    pub kind: String,
    pub jar_size: u64,
    pub has_jar: bool,
    pub last_modified: u64,
    pub java_major: u32,
    /// 游戏版本与加载器，自包含整合包也能显示真实值
    pub game_version: String,
    pub loader: String,
    /// 仅作为整合包父版本存在，不在版本页默认展示
    pub dependency: bool,
}

pub async fn fetch_manifest(client: &reqwest::Client, use_mirror: bool) -> Result<VersionManifest> {
    // 版本清单是版本页和安装的入口，用竞速请求避免偶发卡死
    download::get_json_race(client, MANIFEST_URL, use_mirror, None).await
}

/// 仅返回正式版版本号，供筛选下拉框在 CurseForge 接口不可用时兜底。
pub async fn manifest_version_ids(
    client: &reqwest::Client,
    use_mirror: bool,
) -> Result<Vec<String>> {
    let manifest = fetch_manifest(client, use_mirror).await?;
    let mut list: Vec<String> = manifest
        .versions
        .into_iter()
        .filter(|entry| entry.kind == "release")
        .map(|entry| entry.id)
        .collect();
    list.truncate(60);
    Ok(list)
}

pub fn list_installed(cfg: &Config) -> Vec<InstalledVersion> {
    let dir = cfg.game_root().join("versions");
    let mut result = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return result;
    };
    let versions: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|entry| {
            let id = entry.file_name().to_string_lossy().to_string();
            (id, entry.path())
        })
        .filter(|(id, path)| path.join(format!("{id}.json")).exists())
        .collect();

    // 整合包实例会继承原版与加载器版本，这些父版本在版本页默认折叠
    let mut dependencies: HashSet<String> = HashSet::new();
    for (id, _) in &versions {
        if crate::instance::get(cfg, id).category != "modpack" {
            continue;
        }
        if let Ok(merged) = resolve_detail(cfg, id) {
            for part in merged.chain {
                if &part != id {
                    dependencies.insert(part);
                }
            }
        }
    }

    for (id, path) in versions {
        let resolved = resolve_detail(cfg, &id).ok();
        let (kind, java_major) = match &resolved {
            Some(merged) => (merged.detail.kind.clone(), merged.detail.required_java()),
            None => ("unknown".into(), 8),
        };
        let (game_version, loader) = match &resolved {
            Some(merged) => context_from(&id, &merged.detail, &merged.chain),
            None => (id.clone(), "vanilla".to_string()),
        };
        let jar_path = match resolved.as_ref().and_then(|m| m.jar_id.as_ref()) {
            Some(jar_id) => dir.join(jar_id).join(format!("{jar_id}.jar")),
            None => path.join(format!("{id}.jar")),
        };
        let (jar_size, has_jar, modified) = match std::fs::metadata(&jar_path) {
            Ok(meta) => (
                meta.len(),
                true,
                meta.modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            ),
            Err(_) => (0, false, 0),
        };
        result.push(InstalledVersion {
            dependency: dependencies.contains(&id),
            id,
            kind,
            jar_size,
            has_jar,
            last_modified: modified,
            java_major,
            game_version,
            loader,
        });
    }
    result.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));
    result
}

/// 依赖版本只要还有别的版本在用就保留，避免误删共享的原版 / 加载器。
/// chain 含实例自身（chain[0] 就是 id），必须排除，否则"自己引用自己"恒真。
pub fn is_referenced(cfg: &Config, target: &str, excluded: &[String]) -> bool {
    let dir = cfg.game_root().join("versions");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            return false;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        if !dir.join(&id).join(format!("{id}.json")).exists() || excluded.contains(&id) {
            return false;
        }
        resolve_detail(cfg, &id)
            .map(|merged| {
                merged
                    .chain
                    .iter()
                    .any(|part| part == target && part != &id)
            })
            .unwrap_or(false)
    })
}

fn rule_matches(rule: &Rule) -> bool {
    if let Some(os) = &rule.os {
        if let Some(name) = &os.name {
            if name != "windows" {
                return false;
            }
        }
        if let Some(arch) = &os.arch {
            // 当前仅支持 64 位 JVM
            if arch == "x86" {
                return false;
            }
        }
    }
    if let Some(features) = &rule.features {
        for (key, expected) in features {
            let actual = match key.as_str() {
                "has_custom_resolution" => true,
                "is_demo_user" => false,
                "has_quick_plays_support" => false,
                "is_quick_play_singleplayer" => false,
                "is_quick_play_multiplayer" => false,
                "is_quick_play_realms" => false,
                _ => false,
            };
            if actual != *expected {
                return false;
            }
        }
    }
    true
}

pub fn rules_allow(rules: Option<&Vec<Rule>>) -> bool {
    match rules {
        None => true,
        Some(list) => {
            let mut allowed = false;
            for rule in list {
                if rule_matches(rule) {
                    allowed = rule.action == "allow";
                }
            }
            allowed
        }
    }
}

pub(crate) fn maven_parts(name: &str) -> Option<(String, String, String, Option<String>, String)> {
    let (coords, ext) = match name.split_once('@') {
        Some((c, e)) => (c, e.to_string()),
        None => (name, "jar".to_string()),
    };
    let parts: Vec<&str> = coords.split(':').collect();
    if parts.len() < 3 {
        return None;
    }
    let group = parts[0].to_string();
    let artifact = parts[1].to_string();
    let version = parts[2].to_string();
    let classifier = parts.get(3).map(|s| s.to_string());
    Some((group, artifact, version, classifier, ext))
}

pub(crate) fn maven_path(name: &str) -> Option<String> {
    let (group, artifact, version, classifier, ext) = maven_parts(name)?;
    let file = match classifier {
        Some(c) => format!("{artifact}-{version}-{c}.{ext}"),
        None => format!("{artifact}-{version}.{ext}"),
    };
    Some(format!(
        "{}/{}/{}/{}",
        group.replace('.', "/"),
        artifact,
        version,
        file
    ))
}

pub(crate) fn library_is_native(lib: &Library) -> bool {
    // 旧版清单用 natives 映射 + extract 表示需要解压；
    // 1.19 之后 natives 作为普通 classifier 库，直接进 classpath。
    lib.natives
        .as_ref()
        .map(|n| n.contains_key("windows"))
        .unwrap_or(false)
        || lib.extract.is_some()
}

pub(crate) fn library_artifact(lib: &Library) -> Option<(DownloadRef, String)> {
    if let Some(downloads) = &lib.downloads {
        if let Some(artifact) = &downloads.artifact {
            let path = artifact
                .path
                .clone()
                .or_else(|| maven_path(&lib.name))
                .unwrap_or_default();
            return Some((artifact.clone(), path));
        }
    }
    // Fabric / Quilt：name + url 形式
    if let Some(base) = &lib.url {
        let path = maven_path(&lib.name)?;
        let url = format!("{}/{}", base.trim_end_matches('/'), path);
        return Some((
            DownloadRef {
                sha1: lib.sha1.clone().unwrap_or_default(),
                size: lib.size.unwrap_or(0),
                url,
                path: Some(path.clone()),
            },
            path,
        ));
    }
    None
}

pub(crate) fn library_native_download(lib: &Library) -> Option<(DownloadRef, String)> {
    let downloads = lib.downloads.as_ref()?;
    let (group, artifact, version, name_classifier, _) = maven_parts(&lib.name)?;
    let classifier = lib
        .natives
        .as_ref()
        .and_then(|n| n.get("windows").cloned())
        .or_else(|| name_classifier.filter(|c| c.starts_with("natives")))?;
    if let Some(classifiers) = &downloads.classifiers {
        if let Some(dl) = classifiers.get(&classifier) {
            let file = format!("{artifact}-{version}-{classifier}.jar");
            let path = format!(
                "{}/{}/{}/{}",
                group.replace('.', "/"),
                artifact,
                version,
                file
            );
            return Some((dl.clone(), path));
        }
    }
    // 新版清单里 natives 直接作为 artifact 提供
    downloads.artifact.as_ref().map(|dl| {
        let path = dl
            .path
            .clone()
            .or_else(|| maven_path(&lib.name))
            .unwrap_or_default();
        (dl.clone(), path)
    })
}

pub(crate) fn extract_natives(jar: &Path, dest: &Path, exclude: &[String]) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let file = File::open(jar).with_context(|| format!("无法打开 {}", jar.display()))?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let rel = rel.to_path_buf();
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if exclude.iter().any(|ex| {
            let ex = ex.trim_end_matches('/');
            rel_str == ex || rel_str.starts_with(&format!("{ex}/"))
        }) {
            continue;
        }
        let out_path = dest.join(&rel);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = File::create(&out_path)?;
        copy(&mut entry, &mut out)?;
    }
    Ok(())
}

pub async fn install_version(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    version_id: &str,
    progress: Arc<ProgressState>,
) -> Result<InstalledVersion> {
    let use_mirror = cfg.use_mirror();
    let game_root = cfg.game_root();
    let version_dir = game_root.join("versions").join(version_id);
    tokio::fs::create_dir_all(&version_dir).await?;

    progress.set_message("正在获取版本信息…");
    let manifest = fetch_manifest(client, use_mirror).await?;
    let entry = manifest
        .versions
        .iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| anyhow!("版本 {version_id} 不存在"))?;

    let raw = download::get_bytes(client, &entry.url, use_mirror).await?;
    let detail: VersionDetail = serde_json::from_slice(&raw).context("解析版本 JSON 失败")?;
    if let Some(parent) = &detail.inherits_from {
        bail!("当前版本依赖 {parent}，原版安装暂不支持继承版本");
    }

    let version_json = version_dir.join(format!("{version_id}.json"));
    tokio::fs::write(&version_json, &raw).await?;

    // 1) 客户端 jar、依赖库、本地库
    let mut tasks: Vec<FileTask> = Vec::new();
    if let Some(client_dl) = detail.downloads.as_ref().and_then(|d| d.client.as_ref()) {
        tasks.push(FileTask {
            url: client_dl.url.clone(),
            dest: version_dir.join(format!("{version_id}.jar")),
            sha1: Some(client_dl.sha1.clone()),
            size: client_dl.size,
            label: format!("{version_id}.jar"),
        });
    }

    let mut native_jars: Vec<(PathBuf, Vec<String>)> = Vec::new();
    for lib in &detail.libraries {
        if !rules_allow(lib.rules.as_ref()) {
            continue;
        }
        let native = library_is_native(lib);
        let chosen = if native {
            library_native_download(lib)
        } else {
            library_artifact(lib)
        };
        if let Some((dl, path)) = chosen {
            if path.is_empty() {
                continue;
            }
            let dest = game_root.join("libraries").join(path.replace('/', "\\"));
            tasks.push(FileTask {
                url: dl.url,
                dest: dest.clone(),
                sha1: Some(dl.sha1),
                size: dl.size,
                label: dest
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
            });
            if native {
                let exclude = lib
                    .extract
                    .as_ref()
                    .and_then(|e| e.exclude.clone())
                    .unwrap_or_else(|| vec!["META-INF/".to_string()]);
                native_jars.push((dest, exclude));
            }
        }
    }

    progress.set_message("正在下载游戏核心与依赖库…");
    download::run_tasks(
        app,
        client,
        tasks,
        // 依赖库多为大文件，并发太高容易触发镜像限流
        (cfg.concurrency as usize).max(16),
        use_mirror,
        progress.clone(),
    )
    .await?;

    // 2) 资源索引与资源文件
    // 资源文件缺一两个只会少音效/贴图，不该让整个安装失败，失败项单独汇总
    let mut asset_failures: Vec<String> = Vec::new();
    if let Some(index_ref) = &detail.asset_index {
        progress.set_message("正在获取资源索引…");
        // 索引体积大且是后续全部下载的前置，卡在这里最常见，用竞速请求；
        // 补位放宽到 1.5 秒，避免正常网络下白白多下一份
        let raw = download::get_bytes_race(
            client,
            &index_ref.url,
            use_mirror,
            Some(&progress),
            3,
            1500,
        )
        .await?;
        let index: AssetIndexFile =
            serde_json::from_slice(&raw).context("解析资源索引失败")?;
        let index_path = game_root
            .join("assets")
            .join("indexes")
            .join(format!("{}.json", index_ref.id));
        if let Some(parent) = index_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&index_path, &raw).await?;

        let mut asset_tasks = Vec::new();
        for (name, obj) in &index.objects {
            if obj.hash.len() < 2 {
                continue;
            }
            let dest = game_root
                .join("assets")
                .join("objects")
                .join(&obj.hash[..2])
                .join(&obj.hash);
            asset_tasks.push(FileTask {
                url: format!(
                    "https://resources.download.minecraft.net/{}/{}",
                    &obj.hash[..2],
                    obj.hash
                ),
                dest,
                sha1: Some(obj.hash.clone()),
                size: obj.size,
                label: name.clone(),
            });
        }
        progress.set_message(format!("正在下载 {} 个资源文件…", asset_tasks.len()));
        let outcome = download::run_tasks_reporting(
            app,
            client,
            asset_tasks,
            // 资源文件是几千个小文件，并发 8 严重拖慢；镜像与官方源都支持 32+
            (cfg.concurrency as usize).max(32),
            use_mirror,
            progress.clone(),
        )
        .await?;
        if !outcome.failed.is_empty() {
            asset_failures = outcome
                .failed
                .into_iter()
                .map(|(label, _)| label)
                .collect();
        }
    }

    // 3) 日志配置
    if let Some(logging) = detail.logging.as_ref().and_then(|l| l.client.as_ref()) {
        let dest = game_root
            .join("assets")
            .join("log_configs")
            .join(&logging.file.url.rsplit('/').next().unwrap_or("log4j.xml"));
        let tasks = vec![FileTask {
            url: logging.file.url.clone(),
            dest,
            sha1: Some(logging.file.sha1.clone()),
            size: logging.file.size,
            label: "日志配置".into(),
        }];
        download::run_tasks(app, client, tasks, 1, use_mirror, progress.clone()).await?;
    }

    // 4) 解压旧版本地库；新版 natives 直接走 classpath
    progress.set_message("正在准备本地库…");
    let natives_dir = version_dir.join("natives");
    if natives_dir.exists() {
        let _ = std::fs::remove_dir_all(&natives_dir);
    }
    tokio::fs::create_dir_all(&natives_dir).await?;
    for sub in ["java", "jna", "lwjgl", "netty"] {
        tokio::fs::create_dir_all(natives_dir.join(sub)).await?;
    }
    if !native_jars.is_empty() {
        for (jar, exclude) in &native_jars {
            let jar = jar.clone();
            let jar_label = jar.clone();
            let dest = natives_dir.clone();
            let exclude = exclude.clone();
            tokio::task::spawn_blocking(move || extract_natives(&jar, &dest, &exclude))
                .await?
                .with_context(|| format!("解压本地库失败：{}", jar_label.display()))?;
        }
    }

    if asset_failures.is_empty() {
        progress.set_message("安装完成");
    } else {
        let sample = asset_failures
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("、");
        progress.set_message(format!(
            "安装完成，{} 个资源文件缺失（{}）。缺音效/贴图不影响启动，可稍后重新安装补齐",
            asset_failures.len(),
            sample
        ));
    }
    progress.active.store(false, Ordering::Relaxed);
    download::emit_now(app, &progress);

    let jar_path = version_dir.join(format!("{version_id}.jar"));
    let kind = detail.kind.clone();
    let java_major = detail.required_java();
    let (game_version, loader) = context_from(&version_id, &detail, &[version_id.to_string()]);
    Ok(InstalledVersion {
        dependency: false,
        id: version_id.to_string(),
        kind,
        jar_size: std::fs::metadata(&jar_path).map(|m| m.len()).unwrap_or(0),
        has_jar: jar_path.exists(),
        last_modified: std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        java_major,
        game_version,
        loader,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        clean_partial_loader_dir, context_from, failure_message, loader_version_from_id,
        official_installer_url,
    };
    use crate::models::VersionDetail;

    fn parse(value: serde_json::Value) -> VersionDetail {
        serde_json::from_value(value).unwrap()
    }

    /// 自包含整合包没有继承链：真实数据的 assets 是 `5`，
    /// 版本号只能靠 clientVersion 与库坐标还原
    #[test]
    fn self_contained_recovers_version_and_loader() {
        let detail = parse(serde_json::json!({
            "id": "Immersive Fight 4.2.1",
            "clientVersion": "1.20.1",
            "assets": "5",
            "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
            "libraries": [{ "name": "net.minecraftforge:fmlloader:1.20.1-47.4.6" }],
        }));
        let chain = vec!["Immersive Fight 4.2.1".to_string()];
        assert_eq!(
            context_from("Immersive Fight 4.2.1", &detail, &chain),
            ("1.20.1".to_string(), "forge".to_string())
        );
    }

    /// 没有 clientVersion 时退回库坐标里的版本号
    #[test]
    fn library_coordinate_yields_game_version() {
        let detail = parse(serde_json::json!({
            "id": "Pack",
            "assets": "5",
            "libraries": [{ "name": "net.minecraftforge:forge:1.12.2-14.23.5.2860" }],
        }));
        let chain = vec!["Pack".to_string()];
        assert_eq!(
            context_from("Pack", &detail, &chain),
            ("1.12.2".to_string(), "forge".to_string())
        );
    }

    /// 导入时写入的元信息优先级最高
    #[test]
    fn docl_meta_takes_precedence() {
        let detail = parse(serde_json::json!({
            "id": "Pack",
            "assets": "1.20.1",
            "_docl": { "game_version": "1.21.1", "loader": "neoforge" },
        }));
        let chain = vec!["Pack".to_string()];
        assert_eq!(
            context_from("Pack", &detail, &chain),
            ("1.21.1".to_string(), "neoforge".to_string())
        );
    }

    #[test]
    fn loader_version_from_standard_ids() {
        assert_eq!(
            loader_version_from_id("1.20.1-forge-47.4.28", "forge", "1.20.1").as_deref(),
            Some("47.4.28")
        );
        assert_eq!(
            loader_version_from_id("1.12.2-forge1.12.2-14.23.5.2860", "forge", "1.12.2")
                .as_deref(),
            Some("14.23.5.2860")
        );
        assert_eq!(
            loader_version_from_id("neoforge-21.1.259", "neoforge", "1.21.1").as_deref(),
            Some("21.1.259")
        );
        assert_eq!(
            loader_version_from_id("1.20.1-forge-47.1.106", "neoforge", "1.20.1").as_deref(),
            Some("47.1.106")
        );
        assert_eq!(
            loader_version_from_id("fabric-loader-0.16.9-1.21.1", "fabric", "1.21.1").as_deref(),
            Some("0.16.9")
        );
        assert_eq!(loader_version_from_id("自定义整合包", "forge", "1.20.1"), None);
    }

    #[test]
    fn official_installer_url_rewrites_mirror_hosts() {
        assert_eq!(
            official_installer_url(
                "forge",
                "https://bmclapi2.bangbang93.com/maven/net/minecraftforge/forge/1.20.1-47.4.28/forge-1.20.1-47.4.28-installer.jar"
            )
            .as_deref(),
            Some("https://maven.minecraftforge.net/net/minecraftforge/forge/1.20.1-47.4.28/forge-1.20.1-47.4.28-installer.jar")
        );
        assert_eq!(
            official_installer_url(
                "neoforge",
                "https://bmclapi2.bangbang93.com/maven/net/neoforged/neoforge/21.1.259/neoforge-21.1.259-installer.jar"
            )
            .as_deref(),
            Some("https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.259/neoforge-21.1.259-installer.jar")
        );
        assert_eq!(official_installer_url("forge", "https://example.com/x.jar"), None);
    }

    #[test]
    fn clearing_loader_files_keeps_instance_content() {
        let dir = std::env::temp_dir().join("docl-clear-loader-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pack.json"), b"{}").unwrap();
        std::fs::write(dir.join("pack.jar"), b"jar").unwrap();
        std::fs::create_dir_all(dir.join("natives")).unwrap();
        std::fs::write(dir.join("natives").join("x.dll"), b"x").unwrap();
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        std::fs::write(dir.join("mods").join("a.jar"), b"mod").unwrap();
        std::fs::create_dir_all(dir.join("saves").join("world")).unwrap();
        std::fs::write(dir.join("saves").join("world").join("level.dat"), b"data").unwrap();
        std::fs::create_dir_all(dir.join("config")).unwrap();
        std::fs::write(dir.join("config").join("a.toml"), b"cfg").unwrap();

        super::clear_loader_version_dir(&dir, "pack");
        assert!(!dir.join("pack.json").exists());
        assert!(!dir.join("pack.jar").exists());
        assert!(!dir.join("natives").exists());
        // 实例内容必须保留
        assert!(dir.join("mods").join("a.jar").exists());
        assert!(dir.join("saves").join("world").join("level.dat").exists());
        assert!(dir.join("config").join("a.toml").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clean_partial_loader_dir_only_removes_target() {
        let mut cfg = crate::config::Config::default();
        let root = std::env::temp_dir().join("docl-clean-partial-test");
        let _ = std::fs::remove_dir_all(&root);
        cfg.game_dir = root.to_string_lossy().to_string();
        let target = root.join("versions").join("1.20.1-forge-47.4.28");
        let other = root.join("versions").join("1.20.1");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(target.join("1.20.1-forge-47.4.28.json"), b"{}").unwrap();
        std::fs::create_dir_all(target.join("mods")).unwrap();
        std::fs::write(target.join("mods").join("a.jar"), b"mod").unwrap();

        clean_partial_loader_dir(&cfg, Some("1.20.1-forge-47.4.28"));
        assert!(!target.join("1.20.1-forge-47.4.28.json").exists());
        // 实例内容必须保留，只清理加载器清单
        assert!(target.join("mods").join("a.jar").exists());
        assert!(other.exists());

        clean_partial_loader_dir(&cfg, None);
        assert!(other.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn failure_message_carries_native_error_and_output() {
        let message = failure_message(
            "Forge 安装器退出码：1".into(),
            "processor failed",
            &None,
            &Some("原生处理器退出 1".into()),
        );
        assert!(message.contains("Forge 安装器退出码：1"));
        assert!(message.contains("安装器输出：processor failed"));
        assert!(message.contains("原生安装失败：原生处理器退出 1"));
        assert_eq!(failure_message("head".into(), "", &None, &None), "head");
    }

    /// PCL 生成的清单会把同一个库列两遍，重复路径会让 BootstrapLauncher 构建
    /// union 文件系统时抛 Duplicate key 直接启动失败，必须在 classpath 里折叠
    #[test]
    fn duplicate_libraries_are_collapsed() {
        let cfg = crate::config::Config::default();
        let detail = parse(serde_json::json!({
            "id": "Pack",
            "libraries": [
                { "name": "com.google.code.gson:gson:2.10.1" },
                { "name": "com.google.code.gson:gson:2.10.1" },
                { "name": "com.google.guava:guava:32.1.2-jre" },
            ],
        }));
        assert_eq!(super::library_file_paths(&cfg, &detail).len(), 2);
    }

    /// PCL / 加载器合并后可能同时留下 asm 9.6 和 9.8；Fabric 会因 classpath
    /// 上的重复类直接拒绝启动，所以必须只保留版本更高的那个。
    #[test]
    fn newer_duplicate_library_version_wins() {
        let cfg = crate::config::Config::default();
        let detail = parse(serde_json::json!({
            "id": "Pack",
            "libraries": [
                { "name": "org.ow2.asm:asm:9.6" },
                { "name": "org.ow2.asm:asm:9.8" },
                { "name": "org.ow2.asm:asm-analysis:9.8" },
            ],
        }));
        let paths = super::library_file_paths(&cfg, &detail);
        assert_eq!(paths.len(), 2);
        assert!(paths
            .iter()
            .any(|path| path.to_string_lossy().contains("asm-9.8.jar")));
        assert!(!paths
            .iter()
            .any(|path| path.to_string_lossy().contains("asm-9.6.jar")));
    }

    /// 有继承链时链根就是原版版本号
    #[test]
    fn chain_root_wins_over_assets() {
        let detail = parse(serde_json::json!({
            "id": "1.20.1-forge-47.2.0",
            "assets": "1.20.1",
        }));
        let chain = vec![
            "1.20.1-forge-47.2.0".to_string(),
            "1.20.1".to_string(),
        ];
        assert_eq!(
            context_from("1.20.1-forge-47.2.0", &detail, &chain),
            ("1.20.1".to_string(), "forge".to_string())
        );
    }
}

pub fn is_installed(cfg: &Config, version_id: &str) -> bool {
    cfg.game_root()
        .join("versions")
        .join(version_id)
        .join(format!("{version_id}.json"))
        .exists()
}

/// 供启动流程复用：读取已安装版本的详情
pub fn read_installed_detail(cfg: &Config, version_id: &str) -> Result<VersionDetail> {
    let path = cfg
        .game_root()
        .join("versions")
        .join(version_id)
        .join(format!("{version_id}.json"));
    let bytes = std::fs::read(&path)?;
    let detail = serde_json::from_slice::<VersionDetail>(&bytes)?;
    Ok(detail)
}

#[derive(Debug, Clone)]
pub struct MergedDetail {
    pub detail: VersionDetail,
    pub jar_id: Option<String>,
    /// 子版本 → 根版本
    pub chain: Vec<String>,
}

fn merge_child(base: &mut VersionDetail, child: &VersionDetail) {
    if !child.kind.is_empty() {
        base.kind = child.kind.clone();
    }
    if let Some(main_class) = &child.main_class {
        base.main_class = Some(main_class.clone());
    }
    if let Some(index) = &child.asset_index {
        base.asset_index = Some(index.clone());
    }
    if let Some(downloads) = &child.downloads {
        base.downloads = Some(downloads.clone());
    }
    if let Some(java) = &child.java_version {
        base.java_version = Some(java.clone());
    }
    if let Some(logging) = &child.logging {
        base.logging = Some(logging.clone());
    }
    if let Some(assets) = &child.assets {
        base.assets = Some(assets.clone());
    }
    if let Some(client) = &child.client_version {
        base.client_version = Some(client.clone());
    }
    if let Some(docl) = &child.docl {
        base.docl = Some(docl.clone());
    }

    for lib in &child.libraries {
        if let Some(pos) = base.libraries.iter().position(|l| l.name == lib.name) {
            base.libraries[pos] = lib.clone();
        } else {
            base.libraries.push(lib.clone());
        }
    }

    // 新式参数：子版本（ModLoader）的参数排在父版本之前，与官方 launch 语义一致
    let mut args = base.arguments.take().unwrap_or(Arguments {
        game: Vec::new(),
        jvm: Vec::new(),
    });
    if let Some(child_args) = &child.arguments {
        let mut game = child_args.game.clone();
        game.extend(args.game);
        let mut jvm = child_args.jvm.clone();
        jvm.extend(args.jvm);
        args.game = game;
        args.jvm = jvm;
    }
    if !args.game.is_empty() || !args.jvm.is_empty() {
        base.arguments = Some(args);
    }
    // 旧式参数：Forge 等清单给出的是完整超集（含自己的 tweakClass），
    // 必须整体覆盖父版本；保留父版本会丢 FML tweak class，
    // 游戏会以 VanillaTweaker 启动并报 ClassNotFoundException
    if let Some(args) = &child.minecraft_arguments {
        base.minecraft_arguments = Some(args.clone());
    }
}

pub fn resolve_detail(cfg: &Config, version_id: &str) -> Result<MergedDetail> {
    let mut chain: Vec<VersionDetail> = Vec::new();
    let mut visited: Vec<String> = Vec::new();
    let mut current = Some(version_id.to_string());
    let mut jar_id: Option<String> = None;

    while let Some(id) = current {
        if visited.contains(&id) {
            bail!("版本继承出现循环：{id}");
        }
        visited.push(id.clone());
        let detail = read_installed_detail(cfg, &id)?;
        if jar_id.is_none() {
            // 链上每一级先看自己的 jar（Forge 1.17+ 是处理器产出的补丁客户端），
            // 再看清单是否自带客户端下载；落到原版 jar 会让模块系统同时
            // 看到两份 Minecraft 类直接崩
            let own_jar = cfg
                .game_root()
                .join("versions")
                .join(&id)
                .join(format!("{id}.jar"));
            if own_jar.exists()
                || detail
                    .downloads
                    .as_ref()
                    .and_then(|d| d.client.as_ref())
                    .is_some()
            {
                jar_id = Some(id.clone());
            }
        }
        current = detail.inherits_from.clone();
        chain.push(detail);
    }

    let mut merged = chain
        .last()
        .cloned()
        .ok_or_else(|| anyhow!("版本 {version_id} 内容为空"))?;
    for child in chain.iter().rev().skip(1) {
        merge_child(&mut merged, child);
    }
    merged.id = version_id.to_string();
    merged.inherits_from = None;
    let ids = chain.into_iter().map(|d| d.id).collect();
    Ok(MergedDetail {
        detail: merged,
        jar_id,
        chain: ids,
    })
}

/// 解析 Maven 坐标（group:artifact:version[:classifier][@ext]），
/// 用于识别「同一个库的不同版本」，例如 asm 9.6 与 asm 9.8。
pub(crate) fn maven_key_version(name: &str) -> Option<(String, String)> {
    let coordinate = name.split('@').next().unwrap_or(name);
    let parts: Vec<&str> = coordinate.split(':').collect();
    if parts.len() < 3 || parts[0].is_empty() || parts[1].is_empty() || parts[2].is_empty() {
        return None;
    }
    let key = if parts.len() >= 4 && !parts[3].is_empty() {
        format!("{}:{}:{}", parts[0], parts[1], parts[3])
    } else {
        format!("{}:{}", parts[0], parts[1])
    };
    Some((key.to_ascii_lowercase(), parts[2].to_string()))
}

/// 比较 Maven 版本号：数字段按数值比，非数字段按字典序。
/// 只需要覆盖 asm 9.6 / 9.8 这类常见重复，不追求完整的 Maven 版本语义。
pub(crate) fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(value: &str) -> Vec<String> {
        value
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|part| !part.is_empty())
            .map(|part| part.to_ascii_lowercase())
            .collect()
    }
    let left_parts = parts(a);
    let right_parts = parts(b);
    for index in 0..left_parts.len().max(right_parts.len()) {
        let Some(left) = left_parts.get(index) else {
            return std::cmp::Ordering::Less;
        };
        let Some(right) = right_parts.get(index) else {
            return std::cmp::Ordering::Greater;
        };
        let order = match (left.parse::<u64>(), right.parse::<u64>()) {
            (Ok(left), Ok(right)) => left.cmp(&right),
            _ => left.cmp(right),
        };
        if order != std::cmp::Ordering::Equal {
            return order;
        }
    }
    std::cmp::Ordering::Equal
}

pub fn library_file_paths(cfg: &Config, detail: &VersionDetail) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut versions: Vec<Option<String>> = Vec::new();
    let mut chosen: HashMap<String, usize> = HashMap::new();
    let mut seen_paths: HashSet<String> = HashSet::new();
    for lib in &detail.libraries {
        if !rules_allow(lib.rules.as_ref()) || library_is_native(lib) {
            continue;
        }
        let path = lib
            .downloads
            .as_ref()
            .and_then(|d| d.artifact.as_ref())
            .and_then(|a| a.path.clone())
            .or_else(|| maven_path(&lib.name));
        if let Some(rel) = path {
            let full = cfg.game_root().join("libraries").join(rel.replace('/', "\\"));
            // 外部启动器生成的清单可能把同一个库列两遍（PCL 的整合包清单很常见），
            // 重复路径会让 BootstrapLauncher 构建 union 文件系统时因重复键直接崩溃：
            // Duplicate key .../gson-2.10.1.jar。这里按最终文件路径去重。
            let path_key = full.to_string_lossy().to_lowercase();
            if !seen_paths.insert(path_key.clone()) {
                continue;
            }
            let parsed = maven_key_version(&lib.name);
            let key = parsed
                .as_ref()
                .map(|(key, _)| key.clone())
                .unwrap_or_else(|| format!("path:{path_key}"));
            match chosen.get(&key).copied() {
                Some(index) => {
                    // PCL / 加载器合并时可能同时留下 asm 9.6 和 9.8，
                    // Fabric 的 verifyClasspath 发现重复类会直接拒绝启动，
                    // 这里折叠为版本更高的那一个。
                    let Some((_, version)) = &parsed else {
                        continue;
                    };
                    let newer = versions[index]
                        .as_deref()
                        .map(|current| {
                            compare_versions(version, current) == std::cmp::Ordering::Greater
                        })
                        .unwrap_or(false);
                    if newer {
                        paths[index] = full;
                        versions[index] = Some(version.clone());
                    }
                }
                None => {
                    chosen.insert(key, paths.len());
                    paths.push(full);
                    versions.push(parsed.map(|(_, version)| version));
                }
            }
        }
    }
    paths
}

pub fn merged_classpath_entries(
    cfg: &Config,
    detail: &VersionDetail,
    jar_id: Option<&str>,
) -> Vec<PathBuf> {
    let mut entries = library_file_paths(cfg, detail);
    if let Some(jar) = jar_id {
        let jar_path = cfg
            .game_root()
            .join("versions")
            .join(jar)
            .join(format!("{jar}.jar"));
        let key = jar_path.to_string_lossy().to_lowercase();
        let duplicated = entries
            .iter()
            .any(|p| p.to_string_lossy().to_lowercase() == key);
        if !duplicated {
            entries.push(jar_path);
        }
    }
    entries
}

pub async fn remove_version(cfg: &Config, version_id: &str) -> Result<()> {
    let dir = cfg.game_root().join("versions").join(version_id);
    if dir.exists() {
        tokio::fs::remove_dir_all(&dir).await?;
    }
    Ok(())
}

pub fn resolve_version_dir(cfg: &Config, version_id: &str) -> PathBuf {
    cfg.game_root().join("versions").join(version_id)
}

/// 清理加载器自身产物，保留实例内容（mods/saves/config/resourcepacks）。
/// 版本隔离开启时 versions/<id> 同时是实例游戏目录，绝不能整目录删除。
pub(crate) fn clear_loader_version_dir(dir: &Path, version_id: &str) {
    let _ = std::fs::remove_file(dir.join(format!("{version_id}.json")));
    let _ = std::fs::remove_file(dir.join(format!("{version_id}.jar")));
    let _ = std::fs::remove_dir_all(dir.join("natives"));
}

pub(crate) fn clear_loader_version(cfg: &Config, version_id: &str) {
    clear_loader_version_dir(&resolve_version_dir(cfg, version_id), version_id);
}

pub fn natives_dir(cfg: &Config, version_id: &str) -> PathBuf {
    resolve_version_dir(cfg, version_id).join("natives")
}

/// 启动前的本地库自愈：PCL 等外部启动器导入的实例没有 natives 目录，
/// 这里按版本清单补建目录并解压本地库；新版清单没有 natives jar，空目录即可。
pub fn ensure_natives(cfg: &Config, version_id: &str, detail: &VersionDetail) -> Result<()> {
    let natives_dir = natives_dir(cfg, version_id);
    std::fs::create_dir_all(&natives_dir)?;
    for sub in ["java", "jna", "lwjgl", "netty"] {
        std::fs::create_dir_all(natives_dir.join(sub))?;
    }
    let game_root = cfg.game_root();
    for lib in &detail.libraries {
        if !rules_allow(lib.rules.as_ref()) || !library_is_native(lib) {
            continue;
        }
        let Some((_, path)) = library_native_download(lib) else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        let jar = game_root.join("libraries").join(path.replace('/', "\\"));
        if !jar.exists() {
            continue;
        }
        let exclude = lib
            .extract
            .as_ref()
            .and_then(|e| e.exclude.clone())
            .unwrap_or_else(|| vec!["META-INF/".to_string()]);
        extract_natives(&jar, &natives_dir, &exclude)
            .with_context(|| format!("解压本地库失败：{}", jar.display()))?;
    }
    Ok(())
}

/* ---------- 模组加载器 ---------- */

#[derive(Debug, Clone, Serialize)]
pub struct LoaderOption {
    pub loader: String,
    pub label: String,
    pub version: Option<String>,
    pub installed: bool,
    pub installed_id: Option<String>,
    pub supported: bool,
    pub note: String,
}

#[derive(Debug, Clone, Deserialize)]
struct BmclForgePromo {
    name: String,
    build: BmclForgeBuild,
}

#[derive(Debug, Clone, Deserialize)]
struct BmclForgeBuild {
    version: String,
}

async fn forge_recommended(
    client: &reqwest::Client,
    game_version: &str,
) -> Result<String> {
    let list: Vec<BmclForgePromo> = download::get_json(
        client,
        "https://bmclapi2.bangbang93.com/forge/promos",
        false,
    )
    .await?;
    let recommended = format!("{game_version}-recommended");
    let latest = format!("{game_version}-latest");
    list.iter()
        .find(|entry| entry.name == recommended)
        .or_else(|| list.iter().find(|entry| entry.name == latest))
        .map(|entry| entry.build.version.clone())
        .ok_or_else(|| anyhow!("Forge 没有适配 {game_version} 的版本"))
}

fn xml_versions(text: &str) -> Vec<String> {
    text.split("<version>")
        .skip(1)
        .filter_map(|chunk| chunk.split("</version>").next())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn version_key(value: &str) -> Vec<u64> {
    value
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct LoaderVersion {
    pub version: String,
    pub recommended: bool,
}

async fn fetch_maven_metadata(
    client: &reqwest::Client,
    path: &str,
    official_base: &str,
) -> Result<Vec<String>> {
    let mirrors = [
        format!("https://bmclapi2.bangbang93.com/maven/{path}"),
        format!("{official_base}{path}"),
    ];
    for url in mirrors {
        if let Ok(bytes) = download::get_bytes(client, &url, false).await {
            let text = String::from_utf8_lossy(&bytes);
            let versions = xml_versions(&text);
            if !versions.is_empty() {
                return Ok(versions);
            }
        }
    }
    bail!("无法获取加载器版本列表")
}

pub(crate) fn forge_installer_for(game_version: &str, version: &str) -> (String, String, String) {
    let coordinate = format!("{game_version}-{version}");
    let file = format!("forge-{coordinate}-installer.jar");
    (
        version.to_string(),
        format!("https://bmclapi2.bangbang93.com/maven/net/minecraftforge/forge/{coordinate}/{file}"),
        file,
    )
}

pub(crate) fn neoforge_installer_for(game_version: &str, version: &str) -> (String, String, String) {
    if game_version == "1.20.1" {
        let coordinate = format!("1.20.1-{version}");
        let file = format!("forge-{coordinate}-installer.jar");
        return (
            version.to_string(),
            format!("https://bmclapi2.bangbang93.com/maven/net/neoforged/forge/{coordinate}/{file}"),
            file,
        );
    }
    let file = format!("neoforge-{version}-installer.jar");
    (
        version.to_string(),
        format!("https://bmclapi2.bangbang93.com/maven/net/neoforged/neoforge/{version}/{file}"),
        file,
    )
}

async fn neoforge_installer(
    client: &reqwest::Client,
    game_version: &str,
) -> Result<(String, String, String)> {
    if game_version == "1.20.1" {
        let versions = fetch_maven_metadata(
            client,
            "net/neoforged/forge/maven-metadata.xml",
            "https://maven.neoforged.net/releases/",
        )
        .await?;
        let pick = versions
            .iter()
            .filter(|v| v.starts_with("1.20.1-") && !v.contains("beta"))
            .max_by_key(|v| version_key(v))
            .ok_or_else(|| anyhow!("NeoForge 没有适配 1.20.1 的版本"))?
            .clone();
        let short = pick.trim_start_matches("1.20.1-").to_string();
        return Ok(neoforge_installer_for(game_version, &short));
    }

    let prefix = game_version
        .strip_prefix("1.")
        .unwrap_or(game_version)
        .to_string();
    let mut versions = fetch_maven_metadata(
        client,
        "net/neoforged/neoforge/maven-metadata.xml",
        "https://maven.neoforged.net/releases/",
    )
    .await?;
    versions.retain(|v| {
        v.starts_with(&format!("{prefix}."))
            && !v.contains('-')
            && v.split('.').all(|part| part.chars().all(|c| c.is_ascii_digit()))
    });
    let pick = versions
        .into_iter()
        .max_by_key(|v| version_key(v))
        .ok_or_else(|| anyhow!("NeoForge 没有适配 {game_version} 的版本"))?;
    Ok(neoforge_installer_for(game_version, &pick))
}

async fn forge_installer(
    client: &reqwest::Client,
    game_version: &str,
) -> Result<(String, String, String)> {
    let version = forge_recommended(client, game_version).await?;
    Ok(forge_installer_for(game_version, &version))
}

/// 从已解析的版本信息推断「游戏版本 + 加载器」。
///
/// 自包含整合包合并后没有继承链，`chain.last()` 只能是包名，因此按以下顺序兜底：
/// 1. 导入时写进 json 的 `_docl`；
/// 2. 继承链的链根（原版版本号）；
/// 3. Forge / NeoForge 安装器留下的 `clientVersion`；
/// 4. 库坐标里的版本号（如 `fmlloader:1.20.1-47.4.6`）；
/// 5. 资源索引名（部分整合包的 `assets` 是 `5` 这类旧索引，只有形如版本号才采用）。
pub fn context_from(version_id: &str, detail: &VersionDetail, chain: &[String]) -> (String, String) {
    if let Some(meta) = &detail.docl {
        if !meta.game_version.is_empty() {
            return (meta.game_version.clone(), meta.loader.clone());
        }
    }

    let game_version = chain
        .last()
        .cloned()
        .filter(|value| value != version_id)
        .or_else(|| {
            detail
                .client_version
                .as_deref()
                .filter(|value| looks_like_game_version(value))
                .map(str::to_string)
        })
        .or_else(|| game_version_from_libraries(detail))
        .or_else(|| {
            detail
                .assets
                .as_deref()
                .filter(|value| looks_like_game_version(value))
                .map(str::to_string)
        })
        .unwrap_or_else(|| version_id.to_string());

    (game_version, detect_loader(version_id, detail))
}

/// 资源索引名形如 `1.20.1`；`pre-1.6`、`legacy`、`5` 这类不算。
fn looks_like_game_version(value: &str) -> bool {
    value.contains('.') && value.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// Forge 的 `fmlloader:1.20.1-47.4.6`、`forge:1.12.2-14.23.5` 自带原版版本号。
fn game_version_from_libraries(detail: &VersionDetail) -> Option<String> {
    for lib in &detail.libraries {
        let lower = lib.name.to_ascii_lowercase();
        if lower.starts_with("net.minecraftforge:forge:")
            || lower.starts_with("net.minecraftforge:fmlloader:")
        {
            let version = lib.name.rsplit(':').next()?;
            let candidate = version.split('-').next()?;
            if looks_like_game_version(candidate) {
                return Some(candidate.to_string());
            }
        }
    }
    None
}

/// 版本 id 优先（DOCL 自己安装的加载器版本 id 带名字），
/// 自包含实例的 id 是包名，改看库坐标与主类。
fn detect_loader(version_id: &str, detail: &VersionDetail) -> String {
    let lower = version_id.to_ascii_lowercase();
    if lower.contains("neoforge") {
        return "neoforge".into();
    }
    if lower.contains("forge") {
        return "forge".into();
    }
    if lower.contains("quilt") {
        return "quilt".into();
    }
    if lower.contains("fabric") {
        return "fabric".into();
    }
    for lib in &detail.libraries {
        let name = lib.name.to_ascii_lowercase();
        if name.starts_with("net.neoforged:") {
            return "neoforge".into();
        }
        if name.starts_with("net.minecraftforge:") {
            return "forge".into();
        }
        if name.starts_with("org.quiltmc:quilt-loader") {
            return "quilt".into();
        }
        if name.starts_with("net.fabricmc:fabric-loader") {
            return "fabric".into();
        }
    }
    if let Some(main) = &detail.main_class {
        let main = main.to_ascii_lowercase();
        if main.contains("bootstraplauncher") {
            return "forge".into();
        }
        if main.contains("quilt") {
            return "quilt".into();
        }
        if main.contains("knot") {
            return "fabric".into();
        }
    }
    "vanilla".into()
}

pub fn instance_context(cfg: &Config, version_id: &str) -> Result<(String, String)> {
    let merged = resolve_detail(cfg, version_id)?;
    Ok(context_from(version_id, &merged.detail, &merged.chain))
}

pub async fn loader_versions(
    client: &reqwest::Client,
    cfg: &Config,
    game_version: &str,
    loader: &str,
) -> Result<Vec<LoaderVersion>> {
    if loader == "fabric" || loader == "quilt" {
        let url = loader_list_url(loader, game_version)
            .ok_or_else(|| anyhow!("不支持的加载器：{loader}"))?;
        let list: Vec<FabricLoaderEntry> =
            download::get_json(client, &url, cfg.use_mirror()).await?;
        let recommended = list
            .iter()
            .find(|entry| entry.loader.stable)
            .map(|entry| entry.loader.version.clone());
        let versions = list
            .into_iter()
            .map(|entry| LoaderVersion {
                recommended: Some(&entry.loader.version) == recommended.as_ref(),
                version: entry.loader.version,
            })
            .collect();
        return Ok(versions);
    }

    if loader == "forge" {
        let all = fetch_maven_metadata(
            client,
            "net/minecraftforge/forge/maven-metadata.xml",
            "https://maven.minecraftforge.net/",
        )
        .await?;
        let prefix = format!("{game_version}-");
        let recommended = forge_recommended(client, game_version).await.ok();
        let mut versions: Vec<String> = all
            .into_iter()
            .filter_map(|coordinate| {
                coordinate
                    .strip_prefix(&prefix)
                    .map(|value| value.to_string())
            })
            .collect();
        versions.sort_by_key(|value| std::cmp::Reverse(version_key(value)));
        versions.truncate(80);
        return Ok(versions
            .into_iter()
            .map(|version| LoaderVersion {
                recommended: Some(&version) == recommended.as_ref(),
                version,
            })
            .collect());
    }

    if loader == "neoforge" {
        let recommended = neoforge_installer(client, game_version)
            .await
            .ok()
            .map(|(version, _, _)| version);
        let mut versions: Vec<String> = if game_version == "1.20.1" {
            fetch_maven_metadata(
                client,
                "net/neoforged/forge/maven-metadata.xml",
                "https://maven.neoforged.net/releases/",
            )
            .await?
            .into_iter()
            .filter(|v| v.starts_with("1.20.1-") && !v.contains("beta"))
            .map(|v| v.trim_start_matches("1.20.1-").to_string())
            .collect()
        } else {
            let prefix = game_version
                .strip_prefix("1.")
                .unwrap_or(game_version)
                .to_string();
            fetch_maven_metadata(
                client,
                "net/neoforged/neoforge/maven-metadata.xml",
                "https://maven.neoforged.net/releases/",
            )
            .await?
            .into_iter()
            .filter(|v| {
                v.starts_with(&format!("{prefix}."))
                    && !v.contains('-')
                    && v.split('.').all(|part| part.chars().all(|c| c.is_ascii_digit()))
            })
            .collect()
        };
        versions.sort_by_key(|value| std::cmp::Reverse(version_key(value)));
        versions.truncate(80);
        return Ok(versions
            .into_iter()
            .map(|version| LoaderVersion {
                recommended: Some(&version) == recommended.as_ref(),
                version,
            })
            .collect());
    }

    bail!("不支持的加载器：{loader}")
}

/// 安装器中途被打断时仍会返回成功码，但写出的清单缺少核心库条目，
/// 启动阶段会抛 module ResolutionException，所以装完必须校验一次。
pub(crate) fn verify_loader_manifest(
    cfg: &Config,
    id: &str,
    loader: &str,
    game_version: &str,
) -> Result<()> {
    let path = cfg
        .game_root()
        .join("versions")
        .join(id)
        .join(format!("{id}.json"));
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("安装器已结束，但没有生成 {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&text)?;
    let names: Vec<&str> = value
        .get("libraries")
        .and_then(|list| list.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|lib| lib.get("name").and_then(|name| name.as_str()))
                .collect()
        })
        .unwrap_or_default();

    let mut missing = Vec::new();
    // 1.17 起核心库改为 fmlloader 引导 + 处理器产出的补丁客户端当版本 jar，
    // 清单里不再有 forge universal 与 net.minecraft:client 条目
    let split_client = game_version
        .split('.')
        .nth(1)
        .and_then(|value| value.parse::<u32>().ok())
        .map(|minor| minor >= 17)
        .unwrap_or(false);

    if loader == "neoforge" {
        if !names.iter().any(|name| {
            name.contains("neoforged") || name.starts_with("net.minecraftforge:fmlloader:")
        }) {
            missing.push("NeoForge 引导库");
        }
    } else if split_client {
        if !names
            .iter()
            .any(|name| name.starts_with("net.minecraftforge:fmlloader:"))
        {
            missing.push("Forge 引导库");
        }
    } else if !names
        .iter()
        .any(|name| name.starts_with("net.minecraftforge:forge:"))
    {
        missing.push("Forge 核心库");
    }
    if split_client
        && !cfg
            .game_root()
            .join("versions")
            .join(id)
            .join(format!("{id}.jar"))
            .exists()
    {
        missing.push("补丁版客户端");
    }
    if missing.is_empty() {
        return Ok(());
    }

    // 半成品清单留着只会让游戏静默退出，清掉加载器产物让用户重装；
    // 实例的 mods/存档不能被一起删掉
    clear_loader_version(cfg, id);
    bail!(
        "{loader} 安装器提前结束，清单缺少{}，已清理加载器文件。请重新安装 {loader}",
        missing.join("、")
    );
}

/// 安装器见到已存在的版本目录会跳过重写直接返回成功码，残缺清单会一直
/// 留在盘上。所以装前先体检：完整的同名加载器直接复用，残缺的当场删掉。
fn check_existing_loader(
    cfg: &Config,
    game_version: &str,
    loader: &str,
    requested: Option<&str>,
) -> Option<String> {
    let mut reuse: Option<String> = None;
    for (name, id) in installed_loaders(cfg, game_version) {
        if name != loader {
            continue;
        }
        let version_matches = requested.map(|v| id.contains(v)).unwrap_or(true);
        if verify_loader_manifest(cfg, &id, loader, game_version).is_ok() {
            if version_matches && reuse.is_none() {
                reuse = Some(id);
            }
            continue;
        }
        clear_loader_version(cfg, &id);
    }
    reuse
}

/// 加载器装好后的统一收尾：复制父版本 natives、汇总版本信息
fn finish_loader_install(
    cfg: &Config,
    game_version: &str,
    target: &str,
) -> Result<InstalledVersion> {
    if let Some(parent_natives) = {
        let path = natives_dir(cfg, game_version);
        path.exists().then_some(path)
    } {
        let dest = natives_dir(cfg, target);
        if !dest.exists() {
            std::fs::create_dir_all(&dest)?;
            copy_dir_all(&parent_natives, &dest)?;
        }
    }
    let detail = resolve_detail(cfg, target)?;
    let jar_size = detail
        .jar_id
        .as_ref()
        .and_then(|jar_id| {
            std::fs::metadata(
                cfg.game_root()
                    .join("versions")
                    .join(jar_id)
                    .join(format!("{jar_id}.jar")),
            )
            .ok()
        })
        .map(|m| m.len())
        .unwrap_or(0);
    let (game_version, loader) = context_from(&target, &detail.detail, &detail.chain);
    Ok(InstalledVersion {
        dependency: false,
        id: target.to_string(),
        kind: "modded".into(),
        jar_size,
        has_jar: true,
        last_modified: std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        java_major: detail.detail.required_java(),
        game_version,
        loader,
    })
}

/// 加载器安装日志：安装器的 stdout/stderr 原样留档，
/// 失败时把日志路径带回错误，用户可以直接发回排查。
struct InstallLog {
    path: PathBuf,
    file: Arc<std::sync::Mutex<std::fs::File>>,
}

impl InstallLog {
    fn create(
        loader: &str,
        game_version: &str,
        java_path: &str,
        installer: &Path,
        native_error: Option<&str>,
    ) -> Option<Self> {
        let dir = crate::config::data_dir().join("logs");
        std::fs::create_dir_all(&dir).ok()?;
        let stamp = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_secs();
        let path = dir.join(format!("loader-{loader}-{game_version}-{stamp}.log"));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()?;
        let _ = writeln!(file, "loader: {loader}");
        let _ = writeln!(file, "game_version: {game_version}");
        let _ = writeln!(file, "java: {java_path}");
        let _ = writeln!(file, "installer: {}", installer.display());
        if let Some(err) = native_error {
            let _ = writeln!(file, "native_error: {err}");
        }
        let _ = writeln!(
            file,
            "------------------------------------------------------------"
        );
        Some(Self {
            path,
            file: Arc::new(std::sync::Mutex::new(file)),
        })
    }

    fn writer(&self) -> Arc<std::sync::Mutex<std::fs::File>> {
        self.file.clone()
    }

    fn append(&self, text: &str) {
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(file, "{text}");
        }
    }
}

fn append_log(file: &Option<Arc<std::sync::Mutex<std::fs::File>>>, text: &str) {
    if let Some(file) = file {
        if let Ok(mut file) = file.lock() {
            let _ = writeln!(file, "{text}");
        }
    }
}

/// 失败时带回原生安装原因和日志路径，避免只剩「退出码 1」。
fn extra_context(log: &Option<InstallLog>, native_error: &Option<String>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(err) = native_error {
        parts.push(format!("原生安装失败：{err}"));
    }
    if let Some(log) = log {
        parts.push(format!("完整日志：{}", log.path.display()));
    }
    parts.join("；")
}

fn failure_message(
    headline: String,
    detail: &str,
    log: &Option<InstallLog>,
    native_error: &Option<String>,
) -> String {
    let mut parts = vec![headline];
    if !detail.is_empty() {
        parts.push(format!("安装器输出：{detail}"));
    }
    let extra = extra_context(log, native_error);
    if !extra.is_empty() {
        parts.push(extra);
    }
    parts.join("。")
}

/// 安装器退出前最后一段输出往往没有换行符，补记一次避免丢失关键报错。
fn flush_tail(
    pending: &str,
    progress: &ProgressState,
    tail: &std::sync::Mutex<Vec<String>>,
    log_file: &Option<Arc<std::sync::Mutex<std::fs::File>>>,
) {
    let text = pending.rsplit(['\r', '\n']).next().unwrap_or("").trim();
    if text.is_empty() {
        return;
    }
    progress.set_detail(text.chars().take(120).collect::<String>());
    append_log(log_file, text);
    if let Ok(mut log) = tail.lock() {
        log.push(text.chars().take(160).collect::<String>());
        if log.len() > 8 {
            log.remove(0);
        }
    }
}

/// 安装器下载地址的官方备用源（镜像地址被改写前的原始主机）
fn official_installer_url(loader: &str, mirror_url: &str) -> Option<String> {
    let rest = mirror_url.strip_prefix("https://bmclapi2.bangbang93.com/maven/")?;
    Some(if loader == "neoforge" {
        format!("https://maven.neoforged.net/releases/{rest}")
    } else {
        format!("https://maven.minecraftforge.net/{rest}")
    })
}

/// 确保安装器缓存完整可用：坏包直接删掉重下，镜像失败再走官方源。
/// 只看文件存在就复用会把损坏的安装器交给官方流程，最终只报退出码 1。
async fn ensure_installer(
    app: &AppHandle,
    client: &reqwest::Client,
    loader: &str,
    mirror_url: &str,
    file_name: &str,
    progress: &Arc<ProgressState>,
) -> Result<PathBuf> {
    let cache = crate::config::data_dir().join("installers");
    tokio::fs::create_dir_all(&cache).await?;
    let path = cache.join(file_name);
    if crate::forge_native::installer_usable(&path) {
        return Ok(path);
    }
    let _ = tokio::fs::remove_file(&path).await;
    let part = PathBuf::from(format!("{}.part", path.display()));
    let _ = tokio::fs::remove_file(&part).await;

    let mut candidates = vec![mirror_url.to_string()];
    if let Some(official) = official_installer_url(loader, mirror_url) {
        candidates.push(official);
    }

    let mut last_err: Option<anyhow::Error> = None;
    for (index, url) in candidates.iter().enumerate() {
        progress.files_total.store(1, Ordering::Relaxed);
        progress.files_done.store(0, Ordering::Relaxed);
        progress.bytes_done.store(0, Ordering::Relaxed);
        progress.bytes_total.store(0, Ordering::Relaxed);
        download::begin_stage(
            app,
            progress,
            if index == 0 {
                format!("正在下载 {loader} 安装器…")
            } else {
                format!("安装器校验失败，改用官方源重下…")
            },
        );

        let mut on_progress = |done: u64, total: u64| {
            progress.bytes_done.store(done, Ordering::Relaxed);
            progress.bytes_total.store(total, Ordering::Relaxed);
            download::emit_now(app, progress);
        };
        match download::download_stream(client, std::slice::from_ref(url), &path, &mut on_progress)
            .await
        {
            Ok(()) => {
                if crate::forge_native::installer_usable(&path) {
                    progress.files_done.store(1, Ordering::Relaxed);
                    download::emit_now(app, progress);
                    return Ok(path);
                }
                let _ = tokio::fs::remove_file(&path).await;
                last_err = Some(anyhow!("{url} 返回的安装器包不完整"));
            }
            Err(err) => {
                let _ = tokio::fs::remove_file(&path).await;
                let _ = tokio::fs::remove_file(&part).await;
                last_err = Some(err.context(format!("下载 {loader} 安装器失败：{url}")));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("下载 {loader} 安装器失败")))
}

/// 清掉安装器声明版本对应的加载器产物：残缺清单留着会让下一次安装直接复用。
fn clean_partial_loader_dir(cfg: &Config, profile_id: Option<&str>) {
    let Some(id) = profile_id else {
        return;
    };
    clear_loader_version(cfg, id);
}

struct OfficialRun {
    success: bool,
    code: i32,
    detail: String,
}

/// 官方安装器单次执行。输出同时写入日志和进度详情，返回值只描述本次结果，
/// 哪次重试、为何失败交给调用方决定。
async fn run_official_installer(
    app: &AppHandle,
    cfg: &Config,
    installer_path: &Path,
    java_path: &str,
    progress: &Arc<ProgressState>,
    log: Option<&InstallLog>,
) -> Result<OfficialRun> {
    // Forge 安装器会检查目标目录里的启动器配置文件，没有就直接报错退出
    let profiles = cfg.game_root().join("launcher_profiles.json");
    if !profiles.exists() {
        let _ = std::fs::write(
            &profiles,
            br#"{"profiles":{},"settings":{"language":"zh_cn"},"version":3}"#,
        );
    }

    let mut command = tokio::process::Command::new(java_path);
    // JVM 参数必须写在 -jar 之前，否则会被当成安装器自己的参数直接报错退出
    if let Some(proxy) = crate::http::usable_system_proxy() {
        let address = proxy
            .trim_start_matches("http://")
            .trim_start_matches("https://");
        if let Some((host, port)) = address.rsplit_once(':') {
            command
                .arg(format!("-Dhttp.proxyHost={host}"))
                .arg(format!("-Dhttp.proxyPort={port}"))
                .arg(format!("-Dhttps.proxyHost={host}"))
                .arg(format!("-Dhttps.proxyPort={port}"));
        }
    }
    command
        .arg("-jar")
        .arg(installer_path)
        .arg("--installClient")
        .arg(cfg.game_root());
    if cfg.use_mirror() {
        command
            .arg("--mirror")
            .arg("https://bmclapi2.bangbang93.com/maven");
    }
    command
        .current_dir(cfg.game_root())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().context("启动 Forge/NeoForge 安装器失败")?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    // 保留最近几行安装器输出，失败时直接写进报错里
    let tail: Arc<std::sync::Mutex<Vec<String>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));

    // 安装器用 \r 刷新进度条而不是换行，按行读会一直读不到东西
    let read_stream = |stream: Option<tokio::process::ChildStdout>,
                       progress: Arc<ProgressState>,
                       tail: Arc<std::sync::Mutex<Vec<String>>>,
                       log_file: Option<Arc<std::sync::Mutex<std::fs::File>>>| async move {
        if let Some(mut stream) = stream {
            use tokio::io::AsyncReadExt;
            let mut buffer = [0u8; 4096];
            let mut pending = String::new();
            loop {
                match stream.read(&mut buffer).await {
                    Ok(0) => break,
                    Ok(read) => {
                        pending.push_str(&String::from_utf8_lossy(&buffer[..read]));
                        if let Some(position) = pending.rfind(['\r', '\n']) {
                            let last = pending[..position].rsplit(['\r', '\n']).next().unwrap_or("");
                            let text = last.trim();
                            if !text.is_empty() {
                                progress.set_detail(text.chars().take(120).collect::<String>());
                                append_log(&log_file, text);
                                if let Ok(mut log) = tail.lock() {
                                    log.push(text.chars().take(160).collect::<String>());
                                    if log.len() > 8 {
                                        log.remove(0);
                                    }
                                }
                            }
                            pending = pending[position + 1..].to_string();
                        }
                        if pending.len() > 4096 {
                            pending.clear();
                        }
                    }
                    Err(_) => break,
                }
            }
            flush_tail(&pending, &progress, &tail, &log_file);
        }
    };
    let read_err = |stream: Option<tokio::process::ChildStderr>,
                    progress: Arc<ProgressState>,
                    tail: Arc<std::sync::Mutex<Vec<String>>>,
                    log_file: Option<Arc<std::sync::Mutex<std::fs::File>>>| async move {
        if let Some(mut stream) = stream {
            use tokio::io::AsyncReadExt;
            let mut buffer = [0u8; 4096];
            let mut pending = String::new();
            loop {
                match stream.read(&mut buffer).await {
                    Ok(0) => break,
                    Ok(read) => {
                        pending.push_str(&String::from_utf8_lossy(&buffer[..read]));
                        if let Some(position) = pending.rfind(['\r', '\n']) {
                            let last = pending[..position].rsplit(['\r', '\n']).next().unwrap_or("");
                            let text = last.trim();
                            if !text.is_empty() {
                                progress.set_detail(text.chars().take(120).collect::<String>());
                                append_log(&log_file, text);
                                if let Ok(mut log) = tail.lock() {
                                    log.push(text.chars().take(160).collect::<String>());
                                    if log.len() > 8 {
                                        log.remove(0);
                                    }
                                }
                            }
                            pending = pending[position + 1..].to_string();
                        }
                        if pending.len() > 4096 {
                            pending.clear();
                        }
                    }
                    Err(_) => break,
                }
            }
            flush_tail(&pending, &progress, &tail, &log_file);
        }
    };

    let out_log = log.map(|value| value.writer());
    let err_log = log.map(|value| value.writer());
    let out_task = tokio::spawn(read_stream(stdout, progress.clone(), tail.clone(), out_log));
    let err_task = tokio::spawn(read_err(stderr, progress.clone(), tail.clone(), err_log));

    let started = std::time::Instant::now();
    let deadline = started + std::time::Duration::from_secs(900);
    let mut last_tick = started;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if progress.is_cancelled() {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = out_task.await;
            let _ = err_task.await;
            bail!("已取消安装");
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = out_task.await;
            let _ = err_task.await;
            bail!("安装器运行超过 15 分钟，已停止");
        }
        // 安装器首次运行要下载依赖且几乎不打印进度，定期刷新已用时间，
        // 避免界面长时间静止被当成卡死
        if last_tick.elapsed() >= std::time::Duration::from_secs(10) {
            last_tick = std::time::Instant::now();
            progress.set_detail(format!(
                "安装器运行中，已用 {} 秒…",
                started.elapsed().as_secs()
            ));
            download::emit_now(app, progress);
        }
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    };
    let _ = out_task.await;
    let _ = err_task.await;
    let detail = tail.lock().map(|log| log.join("；")).unwrap_or_default();
    Ok(OfficialRun {
        success: status.success(),
        code: status.code().unwrap_or(-1),
        detail,
    })
}

async fn install_forge_like(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    game_version: &str,
    loader: &str,
    requested: Option<&str>,
    progress: Arc<ProgressState>,
) -> Result<InstalledVersion> {
    // 清单完整的同名加载器不必重装，直接复用（重复导入整合包时秒过）
    if let Some(existing) = check_existing_loader(cfg, game_version, loader, requested) {
        download::begin_stage(app, &progress, format!("{loader} 已安装，直接复用"));
        progress.files_total.store(1, Ordering::Relaxed);
        progress.files_done.store(1, Ordering::Relaxed);
        download::emit_now(app, &progress);
        return finish_loader_install(cfg, game_version, &existing);
    }

    let required = read_installed_detail(cfg, game_version)?.required_java();
    // 安装器优先精确大版本：手动指定 Java 21 跑 1.18 的处理器很容易退出 1
    let java = match launch::find_java_exact(cfg, required) {
        Some(info) => info,
        None => launch::find_java(cfg, required)?,
    };

    progress.set_message(format!("正在获取 {loader} 安装器地址…"));
    let (_, url, file_name) = if let Some(version) = requested {
        if loader == "neoforge" {
            neoforge_installer_for(game_version, version)
        } else {
            forge_installer_for(game_version, version)
        }
    } else if loader == "neoforge" {
        neoforge_installer(client, game_version).await?
    } else {
        forge_installer(client, game_version).await?
    };

    let installer_path = ensure_installer(app, client, loader, &url, &file_name, &progress).await?;

    // 带内嵌清单的安装器优先原生安装：库走启动器镜像，不受安装器自身联网限制。
    // 原生流程失败时回退官方安装器，避免个别清单差异导致装不上；
    // 原生错误必须保留下来，否则最终只剩一个没有上下文的「退出码 1」。
    let mut native_error: Option<String> = None;
    if crate::forge_native::supports_native(&installer_path) {
        match crate::forge_native::install(
            app,
            client,
            cfg,
            &installer_path,
            loader,
            game_version,
            &java.path,
            progress.clone(),
        )
        .await
        {
            Ok(target) => return finish_loader_install(cfg, game_version, &target),
            Err(err) => {
                let text = format!("{err:#}");
                progress.set_message(format!("原生安装未完成，改用官方安装器：{text}"));
                download::emit_now(app, &progress);
                native_error = Some(text);
            }
        }
    }

    // 回退官方安装器前先清掉半成品版本目录，避免它继续复用残缺清单
    let profile_id = crate::forge_native::profile_version(&installer_path);
    clean_partial_loader_dir(cfg, profile_id.as_deref());

    let log = InstallLog::create(
        loader,
        game_version,
        &java.path,
        &installer_path,
        native_error.as_deref(),
    );

    let mut last_failure: Option<String> = None;
    for attempt in 0..2 {
        progress.files_total.store(1, Ordering::Relaxed);
        progress.files_done.store(0, Ordering::Relaxed);
        // 上一阶段（整合包下载）的字节数不能带进来，否则进度会直接显示 100%
        progress.bytes_done.store(0, Ordering::Relaxed);
        progress.bytes_total.store(0, Ordering::Relaxed);
        download::begin_stage(
            app,
            &progress,
            if attempt == 0 {
                format!("正在运行 {loader} 安装器（可能需要几分钟）…")
            } else {
                format!("首次未完成，清理残留后重试 {loader} 安装器…")
            },
        );

        let run = run_official_installer(
            app,
            cfg,
            &installer_path,
            &java.path,
            &progress,
            log.as_ref(),
        )
        .await?;
        if let Some(log) = &log {
            log.append(&format!("attempt {} exit code: {}", attempt + 1, run.code));
        }

        if !run.success {
            let message = failure_message(
                format!("{loader} 安装器退出码：{}", run.code),
                &run.detail,
                &log,
                &native_error,
            );
            if attempt == 0 {
                last_failure = Some(message);
                clean_partial_loader_dir(cfg, profile_id.as_deref());
                continue;
            }
            return Err(anyhow!(message));
        }

        progress.files_done.store(1, Ordering::Relaxed);
        progress.set_message(format!("{loader} 安装完成"));
        download::emit_now(app, &progress);

        let Some(target) = installed_loaders(cfg, game_version)
            .into_iter()
            .find(|(name, _)| name == loader)
            .map(|(_, id)| id)
        else {
            let message = failure_message(
                format!("安装器已结束，但没有找到 {loader} 版本"),
                &run.detail,
                &log,
                &native_error,
            );
            if attempt == 0 {
                last_failure = Some(message);
                clean_partial_loader_dir(cfg, profile_id.as_deref());
                continue;
            }
            return Err(anyhow!(message));
        };

        // 安装器中途退出也会返回成功码，但清单不完整，启动时会直接崩。
        // 校验失败时把安装器最后几行输出、原生失败原因和日志路径一并带回
        if let Err(err) = verify_loader_manifest(cfg, &target, loader, game_version) {
            let message =
                failure_message(format!("{err:#}"), &run.detail, &log, &native_error);
            if attempt == 0 {
                last_failure = Some(message);
                clean_partial_loader_dir(cfg, profile_id.as_deref());
                continue;
            }
            return Err(anyhow!(message));
        }

        if let Some(log) = log {
            // Windows 上文件句柄未关闭时删不掉，先取出路径并释放句柄
            let path = log.path.clone();
            drop(log);
            let _ = std::fs::remove_file(path);
        }
        return finish_loader_install(cfg, game_version, &target);
    }

    Err(anyhow!(
        last_failure.unwrap_or_else(|| format!("{loader} 安装失败"))
    ))
}

#[derive(Debug, Clone, Deserialize)]
struct FabricLoaderEntry {
    loader: FabricLoaderVersion,
}

#[derive(Debug, Clone, Deserialize)]
struct FabricLoaderVersion {
    version: String,
    #[serde(default)]
    stable: bool,
}

fn loader_list_url(loader: &str, game_version: &str) -> Option<String> {
    match loader {
        "fabric" => Some(format!(
            "https://meta.fabricmc.net/v2/versions/loader/{game_version}"
        )),
        "quilt" => Some(format!(
            "https://meta.quiltmc.org/v3/versions/loader/{game_version}"
        )),
        _ => None,
    }
}

fn loader_profile_url(loader: &str, game_version: &str, loader_version: &str) -> Option<String> {
    match loader {
        "fabric" => Some(format!(
            "https://meta.fabricmc.net/v2/versions/loader/{game_version}/{loader_version}/profile/json"
        )),
        "quilt" => Some(format!(
            "https://meta.quiltmc.org/v3/versions/loader/{game_version}/{loader_version}/profile/json"
        )),
        _ => None,
    }
}

async fn pick_loader_version(
    client: &reqwest::Client,
    cfg: &Config,
    loader: &str,
    game_version: &str,
) -> Result<String> {
    if loader == "forge" {
        return forge_recommended(client, game_version).await;
    }
    if loader == "neoforge" {
        let (version, _, _) = neoforge_installer(client, game_version).await?;
        return Ok(version);
    }
    let url = loader_list_url(loader, game_version)
        .ok_or_else(|| anyhow!("不支持的加载器：{loader}"))?;
    let list: Vec<FabricLoaderEntry> =
        download::get_json(client, &url, cfg.use_mirror()).await?;
    let stable = list.iter().find(|entry| entry.loader.stable);
    let pick = stable.or_else(|| list.first());
    pick.map(|entry| entry.loader.version.clone())
        .ok_or_else(|| anyhow!("{loader} 没有适配 {game_version} 的加载器版本"))
}

fn installed_loaders(cfg: &Config, game_version: &str) -> Vec<(String, String)> {
    let dir = cfg.game_root().join("versions");
    let mut result = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return result;
    };
    for entry in entries.flatten() {
        let id = entry.file_name().to_string_lossy().to_string();
        let json = entry.path().join(format!("{id}.json"));
        let Ok(bytes) = std::fs::read(&json) else {
            continue;
        };
        let Ok(detail) = serde_json::from_slice::<VersionDetail>(&bytes) else {
            continue;
        };
        if detail.inherits_from.as_deref() != Some(game_version) {
            continue;
        }
        let loader = if id.starts_with("fabric-loader") {
            "fabric"
        } else if id.starts_with("quilt-loader") {
            "quilt"
        } else if id.contains("neoforge") {
            "neoforge"
        } else if id.contains("forge") {
            "forge"
        } else {
            continue;
        };
        result.push((loader.to_string(), id));
    }
    result
}

pub async fn loader_options(
    _client: &reqwest::Client,
    cfg: &Config,
    game_version: &str,
) -> Vec<LoaderOption> {
    let installed = installed_loaders(cfg, game_version);
    let loaders = [
        ("fabric", "Fabric", true),
        ("quilt", "Quilt", true),
        ("forge", "Forge", true),
        ("neoforge", "NeoForge", true),
        ("optifine", "OptiFine", false),
    ];
    let mut result = Vec::new();
    for (loader, label, supported) in loaders {
        let found = installed.iter().find(|(name, _)| name == loader);
        let note = if !supported {
            "暂未支持"
        } else if found.is_some() {
            "已安装"
        } else {
            "选择版本安装"
        };
        result.push(LoaderOption {
            loader: loader.to_string(),
            label: label.to_string(),
            version: None,
            installed: found.is_some(),
            installed_id: found.map(|(_, id)| id.clone()),
            supported,
            note: note.to_string(),
        });
    }
    result
}

fn copy_dir_all(source: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(source)?.flatten() {
        let target = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

pub async fn install_loader(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    game_version: &str,
    loader: &str,
    requested: Option<&str>,
    progress: Arc<ProgressState>,
) -> Result<InstalledVersion> {
    let use_mirror = cfg.use_mirror();
    if !is_installed(cfg, game_version) {
        progress.set_message(format!("正在安装原版 {game_version}…"));
        install_version(app, client, cfg, game_version, progress.clone()).await?;
    }

    if loader == "forge" || loader == "neoforge" {
        return install_forge_like(
            app,
            client,
            cfg,
            game_version,
            loader,
            requested,
            progress,
        )
        .await;
    }

    progress.set_message(format!("正在获取 {loader} 加载器版本…"));
    let loader_version = match requested {
        Some(version) => version.to_string(),
        None => pick_loader_version(client, cfg, loader, game_version).await?,
    };
    let profile_url = loader_profile_url(loader, game_version, &loader_version)
        .ok_or_else(|| anyhow!("不支持的加载器：{loader}"))?;
    let raw = download::get_bytes(client, &profile_url, use_mirror).await?;
    let profile: VersionDetail =
        serde_json::from_slice(&raw).context("解析加载器配置失败")?;
    let loader_id = profile.id.clone();
    let version_dir = cfg.game_root().join("versions").join(&loader_id);
    tokio::fs::create_dir_all(&version_dir).await?;
    tokio::fs::write(version_dir.join(format!("{loader_id}.json")), &raw).await?;

    // 复制父版本 natives，加载器沿用原版的本地库
    if let Some(parent_id) = profile.inherits_from.clone() {
        let parent_natives = natives_dir(cfg, &parent_id);
        let target = natives_dir(cfg, &loader_id);
        if parent_natives.exists() {
            tokio::fs::create_dir_all(&target).await?;
            let source = parent_natives.clone();
            let dest = target.clone();
            tokio::task::spawn_blocking(move || copy_dir_all(&source, &dest))
                .await??;
        }
    }

    let merged = resolve_detail(cfg, &loader_id)?;
    let mut tasks: Vec<FileTask> = Vec::new();
    for lib in &merged.detail.libraries {
        if !rules_allow(lib.rules.as_ref()) || library_is_native(lib) {
            continue;
        }
        if let Some((dl, path)) = library_artifact(lib) {
            let dest = cfg.game_root().join("libraries").join(path.replace('/', "\\"));
            if !dest.exists() {
                tasks.push(FileTask {
                    url: dl.url,
                    dest,
                    sha1: Some(dl.sha1),
                    size: dl.size,
                    label: lib.name.clone(),
                });
            }
        }
    }

    if !tasks.is_empty() {
        progress.set_message(format!("正在下载 {loader} 依赖库…"));
        download::run_tasks(
            app,
            client,
            tasks,
            cfg.concurrency as usize,
            use_mirror,
            progress.clone(),
        )
        .await?;
    }

    progress.set_message("加载器安装完成");
    progress.active.store(false, Ordering::Relaxed);
    download::emit_now(app, &progress);

    let java_major = merged.detail.required_java();
    let jar_size = merged
        .jar_id
        .as_ref()
        .and_then(|jar_id| {
            std::fs::metadata(
                cfg.game_root()
                    .join("versions")
                    .join(jar_id)
                    .join(format!("{jar_id}.jar")),
            )
            .ok()
        })
        .map(|m| m.len())
        .unwrap_or(0);

    let (game_version, loader) = context_from(&loader_id, &merged.detail, &merged.chain);
    Ok(InstalledVersion {
        dependency: false,
        id: loader_id,
        kind: "modded".into(),
        jar_size,
        has_jar: true,
        last_modified: std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        java_major,
        game_version,
        loader,
    })
}

/// 从标准加载器版本目录名里取回加载器版本号，用于一键重装。
fn loader_version_from_id(version_id: &str, loader: &str, game_version: &str) -> Option<String> {
    // 版本号必须以数字开头，避免把 "MyForgePack" 之类整合包名当成加载器版本
    let clean = |rest: &str| {
        let rest = rest.trim_start_matches('-');
        let rest = rest
            .strip_prefix(&format!("{game_version}-"))
            .unwrap_or(rest);
        rest.starts_with(|c: char| c.is_ascii_digit())
            .then(|| rest.to_string())
    };
    match loader {
        "forge" | "neoforge" => {
            version_id.split_once("forge").and_then(|(_, rest)| clean(rest))
        }
        "fabric" | "quilt" => {
            let rest = version_id.strip_prefix(&format!("{loader}-loader-"))?;
            let rest = rest
                .strip_suffix(&format!("-{game_version}"))
                .unwrap_or(rest);
            rest.starts_with(|c: char| c.is_ascii_digit())
                .then(|| rest.to_string())
        }
        _ => None,
    }
}

/// 一键自愈的「重装加载器」：删掉损坏的版本目录，按原加载器和版本重新安装。
pub async fn reinstall_loader(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    version_id: &str,
    progress: Arc<ProgressState>,
) -> Result<InstalledVersion> {
    let (game_version, loader) = instance_context(cfg, version_id)
        .with_context(|| format!("无法识别实例 {version_id} 的游戏版本与加载器"))?;
    if !matches!(loader.as_str(), "forge" | "neoforge" | "fabric" | "quilt") {
        bail!("{loader} 版本不需要重装加载器");
    }
    let Some(loader_version) = loader_version_from_id(version_id, &loader, &game_version) else {
        bail!("这个实例不是标准加载器目录（可能是整合包），请重新导入整合包");
    };

    // 只清加载器产物，保留实例的 mods/存档；否则 check_existing_loader 会复用残留清单
    clear_loader_version(cfg, version_id);

    progress.set_message(format!("正在重装 {loader} {loader_version}…"));
    install_loader(
        app,
        client,
        cfg,
        &game_version,
        &loader,
        Some(&loader_version),
        progress,
    )
    .await
}
