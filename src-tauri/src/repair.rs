use crate::config::Config;
use crate::download::{self, FileTask, ProgressState};
use crate::install;
use crate::instance;
use crate::models::{AssetIndexFile, DownloadRef, VersionDetail};
use crate::moddeps;
use anyhow::{Context, Result};
use serde::Serialize;
use std::io::Write;
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
    /// 从 mods 移到 mods-disabled 的解压残留目录
    pub quarantined_mods: Vec<String>,
    /// 因文件名非法而重命名的 Mod
    pub renamed_mods: Vec<String>,
    /// 自动安装的修复类 Mod
    pub installed_fixes: Vec<String>,
    /// 因文件已存在冲突而被定点清理的路径
    pub removed_paths: Vec<String>,
    /// 被移除的无效 -javaagent 参数
    pub removed_java_agents: Vec<String>,
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
    /// 崩溃证据行，用来定位具体是哪个配置文件出错
    #[serde(default)]
    pub evidence: Vec<String>,
}

/// 修复计划里的一项动作，带有会改动的具体文件，方便用户动手前确认。
#[derive(Debug, Default, Serialize)]
pub struct PreviewAction {
    pub id: String,
    pub title: String,
    pub detail: String,
    /// 相对游戏根目录的文件路径，最多列出 MAX_PREVIEW_FILES 项
    pub files: Vec<String>,
    /// 会改动/下载的文件总数（files 可能被截断）
    pub count: usize,
    /// 低 / 中
    pub risk: String,
}

#[derive(Debug, Default, Serialize)]
pub struct RepairPreview {
    pub actions: Vec<PreviewAction>,
    /// 不会改动文件、但需要提醒的问题
    pub issues: Vec<String>,
}

/// 单个动作最多列出的文件数，避免弹窗被超长列表撑爆。
const MAX_PREVIEW_FILES: usize = 40;

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
        let safe_id = safe_version_id(version_id);
        // 名称里带上游戏目录哈希：多个游戏文件夹下的同名版本不能共用备份
        let game_root = cfg.game_root();
        let dir_hash = short_path_hash(&game_root);
        Self {
            root: crate::config::data_dir()
                .join("backups")
                .join(format!("{safe_id}-{dir_hash}-{stamp}")),
            game_root,
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

    /// 记录一次重命名，还原时先把新名字改回旧名字。
    fn record_rename(&mut self, old: &Path, new: &Path) {
        let (Ok(old_rel), Ok(new_rel)) = (
            old.strip_prefix(&self.game_root),
            new.strip_prefix(&self.game_root),
        ) else {
            return;
        };
        if std::fs::create_dir_all(&self.root).is_err() {
            return;
        }
        let line = format!(
            "{}|{}\n",
            old_rel.to_string_lossy(),
            new_rel.to_string_lossy()
        );
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join("renames.txt"))
            .and_then(|mut file| file.write_all(line.as_bytes()));
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

/// 版本 id 转成安全的目录名片段（备份目录与匹配前缀共用）。
fn safe_version_id(version_id: &str) -> String {
    version_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

/// 游戏目录短哈希：区分不同游戏文件夹，避免同名版本的备份混在一起。
fn short_path_hash(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let text = path.to_string_lossy().to_lowercase();
    format!("{:x}", Sha256::digest(text.as_bytes()))
        .chars()
        .take(8)
        .collect()
}

fn restore_from_dir(cfg: &Config, dir: &Path) -> Result<usize> {
    let game_root = cfg.game_root();
    let mut count = 0usize;
    // 先撤销重命名，再把备份的文件拷回去
    restore_renames(dir, &game_root);
    restore_dir(dir, dir, &game_root, &mut count)?;
    Ok(count)
}

pub fn restore(cfg: &Config, version_id: &str) -> Result<usize> {
    let dir = latest_backup(version_id, &cfg.game_root())
        .ok_or_else(|| anyhow::anyhow!("没有找到「{version_id}」的修复备份"))?;
    restore_from_dir(cfg, &dir)
}

/// 修复历史里的一个备份。
#[derive(Debug, Serialize)]
pub struct BackupEntry {
    /// 备份目录名，按次还原时使用
    pub id: String,
    /// 备份创建时间（Unix 秒）
    pub created: u64,
    /// 备份的文件数
    pub files: usize,
    /// 改动过的文件示例（相对游戏根目录，最多 8 个）
    pub samples: Vec<String>,
}

/// 全部修复备份，新的在前。
pub fn backups(cfg: &Config, version_id: &str) -> Vec<BackupEntry> {
    let mut list: Vec<BackupEntry> = backup_dirs(version_id, &cfg.game_root())
        .into_iter()
        .map(|dir| {
            let id = dir
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            let created = id
                .rsplit('-')
                .next()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            let (files, samples) = count_backup_files(&dir);
            BackupEntry {
                id,
                created,
                files,
                samples,
            }
        })
        .collect();
    list.sort_by(|a, b| b.created.cmp(&a.created));
    list
}

/// 还原指定一次的备份；id 必须是当前版本、当前游戏目录下的备份目录名。
pub fn restore_at(cfg: &Config, version_id: &str, backup_id: &str) -> Result<usize> {
    if !backup_belongs(backup_id, version_id, &cfg.game_root())
        || backup_id.contains("..")
        || backup_id.contains('/')
        || backup_id.contains('\\')
    {
        anyhow::bail!("备份记录不存在：{backup_id}");
    }
    let dir = crate::config::data_dir().join("backups").join(backup_id);
    if !dir.is_dir() {
        anyhow::bail!("备份目录不存在：{backup_id}");
    }
    restore_from_dir(cfg, &dir)
}

/// 备份目录名必须是「版本前缀 + 目录哈希 + 纯数字时间戳」，
/// 否则 1.20.1 会误匹配到 1.20.1-forge 的备份。
/// 旧版本没有目录哈希，仍按「版本前缀 + 时间戳」兼容。
fn backup_belongs(name: &str, version_id: &str, game_root: &Path) -> bool {
    let safe_id = safe_version_id(version_id);
    let current_prefix = format!("{safe_id}-{}-", short_path_hash(game_root));
    if let Some(rest) = name.strip_prefix(&current_prefix) {
        return !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit());
    }
    let legacy_prefix = format!("{safe_id}-");
    name.strip_prefix(&legacy_prefix)
        .map(|rest| !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit()))
        .unwrap_or(false)
}

fn backup_dirs(version_id: &str, game_root: &Path) -> Vec<PathBuf> {
    let mut list: Vec<PathBuf> = std::fs::read_dir(crate::config::data_dir().join("backups"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .map(|name| backup_belongs(&name.to_string_lossy(), version_id, game_root))
                    .unwrap_or(false)
        })
        .collect();
    list.sort();
    list
}

fn latest_backup(version_id: &str, game_root: &Path) -> Option<PathBuf> {
    backup_dirs(version_id, game_root).pop()
}

/// 备份里的文件数与示例路径；renames.txt 只用于还原，不算改动文件。
fn count_backup_files(dir: &Path) -> (usize, Vec<String>) {
    fn walk(root: &Path, dir: &Path, count: &mut usize, samples: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, count, samples);
                continue;
            }
            if path.file_name().and_then(|name| name.to_str()) == Some("renames.txt") {
                continue;
            }
            *count += 1;
            if samples.len() < 8 {
                if let Ok(rel) = path.strip_prefix(root) {
                    samples.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    let mut count = 0usize;
    let mut samples = Vec::new();
    walk(dir, dir, &mut count, &mut samples);
    (count, samples)
}

/// 按 renames.txt 把修复时改过的文件名还原。
fn restore_renames(root: &Path, game_root: &Path) {
    let Ok(text) = std::fs::read_to_string(root.join("renames.txt")) else {
        return;
    };
    for line in text.lines() {
        let Some((old, new)) = line.split_once('|') else {
            continue;
        };
        let from = game_root.join(new);
        let to = game_root.join(old);
        if from.is_file() {
            if let Some(parent) = to.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::rename(&from, &to);
        }
    }
}

fn restore_dir(root: &Path, dir: &Path, game_root: &Path, count: &mut usize) -> Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            restore_dir(root, &path, game_root, count)?;
            continue;
        }
        // 重命名清单只用于还原，不应被拷回游戏目录
        if path.file_name().and_then(|name| name.to_str()) == Some("renames.txt") {
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

/// mods 目录下看起来像“被解压的 Mod”的文件夹。
fn extracted_mod_dirs(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter(|entry| {
            let path = entry.path();
            path.join("META-INF").is_dir()
                || path.join("fabric.mod.json").is_file()
                || path.join("quilt.mod.json").is_file()
                || path.join("mcmod.info").is_file()
        })
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            (!name.starts_with('.') && name != "mods-disabled").then_some(name)
        })
        .collect()
}

/// 把含空格、括号等字符的 Mod 文件名改成模块系统能接受的形式。
fn sanitize_mod_file_name(name: &str) -> Option<String> {
    if !name.to_ascii_lowercase().ends_with(".jar") {
        return None;
    }
    let stem = &name[..name.len() - 4];
    let mut cleaned = String::new();
    let mut last_underscore = false;
    for ch in stem.chars() {
        let keep = ch.is_ascii_alphanumeric()
            || matches!(ch, '.' | '-' | '_' | '+')
            || !ch.is_ascii();
        let out = if keep { ch } else { '_' };
        if out == '_' && last_underscore {
            continue;
        }
        last_underscore = out == '_';
        cleaned.push(out);
    }
    let cleaned = cleaned.trim_matches('_').to_string();
    if cleaned.is_empty() || cleaned == stem {
        None
    } else {
        Some(format!("{cleaned}.jar"))
    }
}

/// 按空格切分参数，引号内的空格保留在同一个 token 里。
fn split_command_line(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in input.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                current.push(ch);
            }
            ch if ch.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            ch => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// 去掉指向不存在文件的 -javaagent 参数，返回（新的参数串，被移除的项）。
fn strip_invalid_java_agents(args: &str) -> (String, Vec<String>) {
    let mut kept = Vec::new();
    let mut removed = Vec::new();
    for token in split_command_line(args) {
        let invalid = token
            .strip_prefix("-javaagent:")
            .map(|rest| {
                let path = rest.split('=').next().unwrap_or(rest).trim_matches('"');
                !path.is_empty() && !Path::new(path).is_file()
            })
            .unwrap_or(false);
        if invalid {
            removed.push(token);
        } else {
            kept.push(token);
        }
    }
    (kept.join(" "), removed)
}

/// 从 FileAlreadyExistsException 的证据行里取出被占用的具体路径。
/// 只接受游戏目录内的路径，日志里带 .. 的路径直接跳过。
fn file_exists_paths(game_dir: &Path, evidence: &[String]) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for line in evidence {
        let Some(index) = line.find("FileAlreadyExistsException") else {
            continue;
        };
        let Some((_, tail)) = line[index..].split_once(':') else {
            continue;
        };
        let candidate = tail
            .trim()
            .trim_matches(|ch| matches!(ch, '\'' | '"' | ')' | ',' | ';'));
        if candidate.is_empty() || candidate.contains("..") {
            continue;
        }
        let path = Path::new(candidate);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            game_dir.join(path)
        };
        if !path.starts_with(game_dir) || !path.exists() {
            continue;
        }
        if !found.contains(&path) {
            found.push(path);
        }
    }
    found
}

/// 从崩溃证据里找出具体的 config 文件路径，避免整目录重置。
fn config_paths_from_evidence(game_dir: &Path, evidence: &[String]) -> Vec<PathBuf> {
    let config_dir = game_dir.join("config");
    let mut found: Vec<PathBuf> = Vec::new();
    for line in evidence {
        for token in line.split(|ch: char| {
            ch.is_whitespace() || matches!(ch, '"' | '\'' | '(' | ')' | ',' | '[' | ']')
        }) {
            let Some(index) = token
                .find("config/")
                .or_else(|| token.find("config\\"))
            else {
                continue;
            };
            let relative = &token[index..];
            if relative.contains("..") {
                continue;
            }
            let path = game_dir.join(relative.replace('/', "\\"));
            if !path.starts_with(&config_dir) || !path.exists() {
                continue;
            }
            if !found.contains(&path) {
                found.push(path);
            }
        }
    }
    found
}

fn remove_target(path: &Path, backup: &mut BackupGuard) -> Result<()> {
    if path.is_dir() {
        // 完整递归备份：只备份前两层就删除，深层文件会连同目录一起丢失
        if backup_tree(path, backup, u8::MAX).is_none() {
            anyhow::bail!("目录内有文件无法备份");
        }
        std::fs::remove_dir_all(path)?;
    } else if path.is_file() {
        backup.save(path)?;
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn stamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
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
    if !report.removed_paths.is_empty() {
        parts.push(format!("清理 {} 个冲突文件", report.removed_paths.len()));
    }
    if !report.removed_java_agents.is_empty() {
        parts.push(format!("移除 {} 个无效 -javaagent", report.removed_java_agents.len()));
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
    if needs_redownload(&dest, dl.size) {
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

/// jar 的中央目录读不出来时，游戏会在启动阶段直接抛 ZipException。
fn jar_is_broken(path: &Path) -> bool {
    let is_jar = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("jar"))
        .unwrap_or(false);
    if !is_jar {
        return false;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return true;
    };
    zip::ZipArchive::new(file).is_err()
}

/// 大小对得上也继续检查 jar 是否损坏；重下时会按清单里的 SHA-1 校验。
fn needs_redownload(path: &Path, size: u64) -> bool {
    needs_rebuild(path, size) || jar_is_broken(path)
}

/// 按版本清单挑出需要补下的依赖库，原生库同时记录解压信息。
fn collect_library_tasks(
    detail: &VersionDetail,
    game_root: &Path,
    tasks: &mut Vec<FileTask>,
    natives: &mut Vec<(PathBuf, Vec<String>)>,
) {
    for lib in &detail.libraries {
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
        push_library_task(tasks, natives, dest, &dl, exclude, native);
    }
}

/// 文件相对游戏根目录的展示路径。
fn rel_label(path: &Path, game_root: &Path) -> String {
    path.strip_prefix(game_root)
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
}

/// 截断文件列表，但保留总数。
fn preview_files(mut files: Vec<String>) -> (Vec<String>, usize) {
    let count = files.len();
    files.truncate(MAX_PREVIEW_FILES);
    (files, count)
}

fn push_action(
    out: &mut RepairPreview,
    id: &str,
    title: &str,
    detail: String,
    files: Vec<String>,
    risk: &str,
) {
    let (files, count) = preview_files(files);
    out.actions.push(PreviewAction {
        id: id.to_string(),
        title: title.to_string(),
        detail,
        files,
        count,
        risk: risk.to_string(),
    });
}

/// 只扫描、不改任何文件，生成带具体文件清单的修复计划。
pub fn preview(cfg: &Config, version_id: &str, options: &RepairOptions) -> RepairPreview {
    let mut out = RepairPreview::default();
    let game_root = cfg.game_root();

    let Some(own) = read_detail(cfg, version_id) else {
        push_action(
            &mut out,
            "version-json",
            "重新下载版本清单",
            "版本 json 缺失或已损坏，会从官方清单重新下载一份。".into(),
            vec![rel_label(&version_json_path(cfg, version_id), &game_root)],
            "低",
        );
        out.issues
            .push("版本清单不可用，其余修复项暂时无法预估".into());
        return out;
    };

    let Ok(merged) = install::resolve_detail(cfg, version_id) else {
        out.issues
            .push("版本结构异常，暂时无法生成修复计划，请先重装加载器".into());
        return out;
    };

    // Java：手动指定的运行时大版本或位数不对时会被切走
    if let Some(current) = instance::get(cfg, version_id)
        .java_path
        .filter(|path| !path.trim().is_empty())
    {
        let required = merged.detail.required_java();
        let current_major = crate::java::inspect_java(Path::new(&current), "手动指定")
            .map(|info| info.major);
        let bit_rule = rule_hit(options, "java-bitness-mismatch")
            || rule_hit(options, "java-32bit-lowmem")
            || rule_hit(options, "java-32bit-exact");
        let wrong_bits = bit_rule
            && crate::java::resolve_java_path(&current)
                .map(|exe| crate::preflight::is_32bit_java(&exe))
                .unwrap_or(false);
        if current_major != Some(required) || wrong_bits {
            push_action(
                &mut out,
                "java",
                "切换到匹配的 Java",
                if wrong_bits {
                    format!("当前是 32 位 Java，会换成 64 位的 Java {required}。")
                } else {
                    format!("游戏要求 Java {required}，会切换本机匹配的运行时。")
                },
                Vec::new(),
                "低",
            );
        }
    }

    // 无效 -javaagent：只删指向不存在文件的项
    if rule_hit(options, "java-agent-failed") {
        let mut removed = Vec::new();
        if let Some(args) = instance::get(cfg, version_id).java_args {
            removed = strip_invalid_java_agents(&args).1;
        }
        if !removed.is_empty() {
            push_action(
                &mut out,
                "java-agent",
                "清理无效的 Java 参数",
                "删掉指向不存在 jar 的 -javaagent 项，其余启动参数保持不动。".into(),
                removed,
                "低",
            );
        }
    }

    // 启动 jar 与依赖库
    let version_dir = install::resolve_version_dir(cfg, version_id);
    let jar_dest = version_dir.join(format!("{version_id}.jar"));
    let mut tasks: Vec<FileTask> = Vec::new();
    let mut natives: Vec<(PathBuf, Vec<String>)> = Vec::new();
    if let Some(dl) = own.downloads.as_ref().and_then(|item| item.client.as_ref()) {
        push_library_task(&mut tasks, &mut natives, jar_dest, dl, Vec::new(), false);
    }
    collect_library_tasks(&merged.detail, &game_root, &mut tasks, &mut natives);
    if !tasks.is_empty() {
        let files: Vec<String> = tasks
            .iter()
            .map(|task| rel_label(&task.dest, &game_root))
            .collect();
        push_action(
            &mut out,
            "missing-files",
            "补回缺失的游戏文件",
            format!(
                "启动 jar 与依赖库里有 {} 个文件缺失、大小不对或压缩包损坏，会按清单里的 SHA-1 重新下载。",
                files.len()
            ),
            files,
            "低",
        );
    }
    if !natives.is_empty()
        && (!tasks.is_empty() || !dir_has_files(&install::natives_dir(cfg, version_id), 2))
    {
        push_action(
            &mut out,
            "natives",
            "重新解压本地库",
            "把依赖库里的原生文件解压到实例目录，失败会导致启动时找不到 dll。".into(),
            Vec::new(),
            "低",
        );
    }

    // 画质重置（可选）
    if options.reset_graphics {
        let options_path = instance::game_dir(cfg, version_id).join("options.txt");
        if options_path.is_file() {
            push_action(
                &mut out,
                "graphics",
                "调低画质相关选项",
                "会修改渲染距离、阴影、粒子等键，其余选项保持不动。".into(),
                vec!["options.txt".into()],
                "中",
            );
        }
    }

    let isolated = instance::get(cfg, version_id)
        .isolated
        .unwrap_or(cfg.version_isolation);
    let (game_version, loader) = install::instance_context(cfg, version_id)
        .unwrap_or_else(|_| (version_id.to_string(), String::new()));

    // 预览只读：不要用 content_dir（会顺手创建 mods 目录）
    let mods_dir = instance::game_dir(cfg, version_id).join("mods");
    if mods_dir.is_dir() {
        // 重复 Mod：保留最新的一份，其余改成 .disabled
        let mut disable: Vec<String> = Vec::new();
        for (_id, files) in moddeps::duplicate_mods(&mods_dir) {
            let keep = files.iter().max_by_key(|path| mtime(path)).cloned();
            for file in files {
                if Some(&file) == keep.as_ref() {
                    continue;
                }
                if let Some(name) = file.file_name() {
                    disable.push(name.to_string_lossy().to_string());
                }
            }
        }
        if !disable.is_empty() {
            if isolated {
                push_action(
                    &mut out,
                    "duplicate-mods",
                    "禁用重复模组",
                    "同一 Mod 装了多份，会保留修改时间最新的一份，其余改名为 .disabled 并备份。".into(),
                    disable,
                    "中",
                );
            } else {
                out.issues.push(
                    "检测到重复模组，但该实例未启用版本隔离，修复不会自动禁用".into(),
                );
            }
        }

        let missing_deps = moddeps::missing_requirements(&mods_dir, &loader);
        if !missing_deps.is_empty() {
            if options.install_deps && isolated {
                push_action(
                    &mut out,
                    "missing-deps",
                    "补装缺失的前置 Mod",
                    "从 Modrinth 下载与当前游戏版本、加载器匹配的前置。".into(),
                    missing_deps,
                    "中",
                );
            } else if options.install_deps {
                out.issues.push(format!(
                    "缺少前置模组：{}；该实例未启用版本隔离，修复不会自动补装",
                    missing_deps.join("、")
                ));
            } else {
                out.issues.push(format!(
                    "缺少前置模组：{}（勾选「自动补装缺失前置」后可自动下载）",
                    missing_deps.join("、")
                ));
            }
        }

        let broken = broken_mod_jars(&mods_dir);
        if !broken.is_empty() {
            out.issues.push(format!(
                "这些 Mod 文件已损坏，需要删除后重新下载：{}",
                broken.join("、")
            ));
        }

        if needs_modern_csl_check(&game_version, &loader) {
            let candidates = legacy_customskinloader_files(&mods_dir);
            if !candidates.is_empty() {
                if isolated {
                    push_action(
                        &mut out,
                        "legacy-csl",
                        "禁用不兼容的 CustomSkinLoader",
                        "1.17 及以上无法加载 ForgeV1 / Legacy 分支，会改名并备份。".into(),
                        candidates,
                        "中",
                    );
                } else {
                    out.issues.push(format!(
                        "检测到不兼容的 {}，但该实例未启用版本隔离，修复不会自动禁用",
                        candidates.join("、")
                    ));
                }
            }
        }

        let mut keywords: Vec<&str> = Vec::new();
        if rule_hit(options, "optifine-sodium") {
            keywords.push("optifine");
        }
        if rule_hit(options, "shadersmod-optifine") {
            keywords.push("shadersmod");
        }
        if isolated && !keywords.is_empty() {
            let files: Vec<String> = std::fs::read_dir(&mods_dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|entry| entry.path().is_file())
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .filter(|name| {
                    let lower = name.to_ascii_lowercase();
                    lower.ends_with(".jar") && keywords.iter().any(|key| lower.contains(key))
                })
                .collect();
            if !files.is_empty() {
                push_action(
                    &mut out,
                    "render-conflict",
                    "禁用冲突的渲染 Mod",
                    "OptiFine 与光影/优化 Mod 同时安装会崩溃，会禁用其中一方并备份。".into(),
                    files,
                    "中",
                );
            }
        }

        if isolated {
            let extracted = extracted_mod_dirs(&mods_dir);
            if !extracted.is_empty() {
                push_action(
                    &mut out,
                    "extracted-mods",
                    "隔离解压残留目录",
                    "目录形式的 Mod 无法加载，会移动到 mods-disabled 保留现场。".into(),
                    extracted,
                    "中",
                );
            }
        }

        if isolated
            && (rule_hit(options, "mod-name")
                || rule_hit(options, "mod-filename")
                || rule_hit(options, "invalid-mod-name")
                || rule_hit(options, "mod-name-special-char")
                || rule_hit(options, "module-resolution"))
        {
            let files: Vec<String> = std::fs::read_dir(&mods_dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|entry| entry.path().is_file())
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .filter_map(|old| sanitize_mod_file_name(&old).map(|new| format!("{old} → {new}")))
                .collect();
            if !files.is_empty() {
                push_action(
                    &mut out,
                    "mod-file-name",
                    "修正非法的 Mod 文件名",
                    "文件名里的空格、括号等字符会让模块系统解析失败，会改名并备份。".into(),
                    files,
                    "中",
                );
            }
        }
    }

    // 配置损坏：优先只清证据里点名的文件
    if rule_hit(options, "night-config-bug")
        || rule_hit(options, "mod-config-corrupt")
        || rule_hit(options, "mod-config")
    {
        let game_dir = instance::game_dir(cfg, version_id);
        let targets = config_paths_from_evidence(&game_dir, &options.evidence);
        if !targets.is_empty() {
            push_action(
                &mut out,
                "config",
                "清理损坏的 Mod 配置",
                "只清理崩溃日志里点名的配置文件，删除前会备份。".into(),
                targets
                    .iter()
                    .map(|path| rel_label(path, &game_root))
                    .collect(),
                "中",
            );
        } else {
            out.issues
                .push("config 目录会被整体备份并清空（无法从日志定位具体文件）".into());
        }
    }

    // FileAlreadyExistsException：只清理日志点名的路径
    if rule_hit(options, "file-already-exists") {
        let game_dir = instance::game_dir(cfg, version_id);
        let targets = file_exists_paths(&game_dir, &options.evidence);
        if targets.is_empty() {
            out.issues
                .push("日志里没有定位到具体的已存在文件，需要按日志手动处理".into());
        } else {
            push_action(
                &mut out,
                "file-exists",
                "清理被占用的文件",
                "只清理日志点名的路径，删除前会备份。".into(),
                targets
                    .iter()
                    .map(|path| rel_label(path, &game_root))
                    .collect(),
                "中",
            );
        }
    }

    // 加载器重装（可选动作）
    if options.reinstall_loader
        && (rule_hit(options, "forge-incomplete")
            || rule_hit(options, "forge-incomplete-more")
            || rule_hit(options, "multiple-forge-in-json")
            || rule_hit(options, "repeat-installation")
            || rule_hit(options, "loader-incomplete"))
    {
        push_action(
            &mut out,
            "loader",
            "重装加载器",
            "只清理加载器产物并重新下载依赖，mods、存档与配置保留。".into(),
            Vec::new(),
            "中",
        );
    }

    // 修复类 Mod
    if isolated {
        let mut fixes: Vec<String> = Vec::new();
        if rule_hit(options, "nightconfig-bug") || rule_hit(options, "night-config-bug") {
            fixes.push("Night Config Fixes".into());
        }
        if rule_hit(options, "mixinbootstrap")
            || rule_hit(options, "install-mixinbootstrap")
            || rule_hit(options, "mixinbootstrap-missing")
        {
            fixes.push("MixinBootstrap".into());
        }
        if !fixes.is_empty() {
            push_action(
                &mut out,
                "fix-mods",
                "安装修复类 Mod",
                "按当前游戏版本与加载器自动挑选匹配的构建。".into(),
                fixes,
                "中",
            );
        }
    }

    // 资源补全只能开始后才知道需要下载多少
    if options.repair_assets {
        push_action(
            &mut out,
            "assets",
            "校验并补全资源文件",
            "逐个核对资源索引与对象文件，缺失的声音、语言文件会重新下载，具体数量在开始后才能确定。".into(),
            Vec::new(),
            "中",
        );
    }

    out
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
            || rule_hit(options, "java-32bit-lowmem")
            || rule_hit(options, "java-32bit-exact");
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

    // 无效 -javaagent：指向不存在的 jar 会让 JVM 启动阶段直接退出
    if rule_hit(options, "java-agent-failed") {
        let mut meta = instance::get(cfg, version_id);
        if let Some(args) = meta.java_args.clone() {
            let (next, removed) = strip_invalid_java_agents(&args);
            if !removed.is_empty() {
                progress.set_message("正在清理无效的 Java 参数…");
                meta.java_args = Some(next);
                match instance::save(cfg, version_id, &meta) {
                    Ok(()) => report.removed_java_agents = removed,
                    Err(err) => report
                        .issues
                        .push(format!("无法保存清理后的 Java 参数：{err}")),
                }
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
    collect_library_tasks(&merged.detail, &game_root, &mut tasks, &mut natives);

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

    // 解压残留：目录形式的“Mod”游戏无法加载，移到 mods-disabled 保留现场
    if isolated {
        if let Ok(mods_dir) = instance::content_dir(cfg, version_id, "mods") {
            let extracted = extracted_mod_dirs(&mods_dir);
            if !extracted.is_empty() {
                progress.set_message("正在隔离解压残留…");
                let target_root = instance::game_dir(cfg, version_id).join("mods-disabled");
                let _ = std::fs::create_dir_all(&target_root);
                for name in extracted {
                    let source = mods_dir.join(&name);
                    let target = target_root.join(format!("{name}-{}", stamp()));
                    match std::fs::rename(&source, &target) {
                        Ok(()) => report.quarantined_mods.push(name),
                        Err(err) => report
                            .issues
                            .push(format!("无法隔离解压残留 {name}：{err}")),
                    }
                }
            }
        }
    }

    // 非法文件名：只在崩溃规则明确指向模块名/文件名问题时重命名
    if isolated
        && (rule_hit(options, "mod-name")
            || rule_hit(options, "mod-filename")
            || rule_hit(options, "invalid-mod-name")
            || rule_hit(options, "mod-name-special-char")
            || rule_hit(options, "module-resolution"))
    {
        if let Ok(mods_dir) = instance::content_dir(cfg, version_id, "mods") {
            progress.set_message("正在修正非法的 Mod 文件名…");
            for entry in std::fs::read_dir(&mods_dir).into_iter().flatten().flatten() {
                let old = entry.file_name().to_string_lossy().to_string();
                let Some(new) = sanitize_mod_file_name(&old) else {
                    continue;
                };
                let source = entry.path();
                let target = mods_dir.join(&new);
                if target.exists() {
                    continue;
                }
                if backup.save(&source).is_err() {
                    report
                        .issues
                        .push(format!("无法备份 {old}，已跳过重命名"));
                    continue;
                }
                match std::fs::rename(&source, &target) {
                    Ok(()) => {
                        backup.record_rename(&source, &target);
                        report.renamed_mods.push(format!("{old} → {new}"));
                    }
                    Err(err) => report
                        .issues
                        .push(format!("重命名 {old} 失败：{err}")),
                }
            }
        }
    }

    // 已知修复类 Mod：按规则自动补装（只装与当前加载器/游戏版本匹配的构建）
    if isolated {
        if rule_hit(options, "nightconfig-bug") || rule_hit(options, "night-config-bug") {
            progress.set_message("正在安装 Night Config Fixes…");
            match moddeps::install_project(
                app,
                client,
                cfg,
                version_id,
                "night-config-fixes",
                "Night Config Fixes",
                progress.clone(),
            )
            .await
            {
                Ok(name) => report.installed_fixes.push(name),
                Err(err) => report
                    .issues
                    .push(format!("自动安装 Night Config Fixes 失败：{err}")),
            }
        }
        if rule_hit(options, "mixinbootstrap")
            || rule_hit(options, "install-mixinbootstrap")
            || rule_hit(options, "mixinbootstrap-missing")
        {
            progress.set_message("正在安装 MixinBootstrap…");
            match moddeps::install_project(
                app,
                client,
                cfg,
                version_id,
                "mixinbootstrap",
                "MixinBootstrap",
                progress.clone(),
            )
            .await
            {
                Ok(name) => report.installed_fixes.push(name),
                Err(err) => report
                    .issues
                    .push(format!("自动安装 MixinBootstrap 失败：{err}")),
            }
        }
    }

    // 配置损坏：优先只清掉证据里点名的文件，无法定位时再整体重置
    if rule_hit(options, "night-config-bug")
        || rule_hit(options, "mod-config-corrupt")
        || rule_hit(options, "mod-config")
    {
        progress.set_message("正在处理损坏的 Mod 配置…");
        let game_dir = instance::game_dir(cfg, version_id);
        let config_dir = game_dir.join("config");
        let targets = config_paths_from_evidence(&game_dir, &options.evidence);
        if !targets.is_empty() {
            let mut removed = 0usize;
            for path in &targets {
                match remove_target(path, &mut backup) {
                    Ok(()) => removed += 1,
                    Err(err) => report
                        .issues
                        .push(format!("无法清理配置 {}：{err}", path.display())),
                }
            }
            report.config_reset = removed > 0;
        } else if config_dir.is_dir() {
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

    // FileAlreadyExistsException：只清理日志点名的那个路径，避免误删其他文件
    if rule_hit(options, "file-already-exists") {
        progress.set_message("正在清理被占用的文件…");
        let game_dir = instance::game_dir(cfg, version_id);
        let targets = file_exists_paths(&game_dir, &options.evidence);
        if targets.is_empty() {
            report
                .issues
                .push("日志里没有找到可定位的已存在文件路径，请按日志手动删除".into());
        }
        for path in targets {
            match remove_target(&path, &mut backup) {
                Ok(()) => report.removed_paths.push(rel_label(&path, &cfg.game_root())),
                Err(err) => report
                    .issues
                    .push(format!("无法清理 {}：{err}", path.display())),
            }
        }
    }

    // 加载器损坏：可选重装（只清加载器产物，保留 mods 与存档）
    if options.reinstall_loader
        && (rule_hit(options, "forge-incomplete")
            || rule_hit(options, "forge-incomplete-more")
            || rule_hit(options, "multiple-forge-in-json")
            || rule_hit(options, "repeat-installation")
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

    #[test]
    fn sanitizes_invalid_mod_names() {
        assert_eq!(
            sanitize_mod_file_name("Cool Mod [1.20].jar").as_deref(),
            Some("Cool_Mod_1.20.jar")
        );
        assert_eq!(sanitize_mod_file_name("中文Mod.jar"), None);
        assert_eq!(sanitize_mod_file_name("normal-1.0.jar"), None);
        assert_eq!(sanitize_mod_file_name("notes.txt"), None);
    }

    #[test]
    fn detects_extracted_mod_directories() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("docl-repair-extracted-{stamp}"));
        std::fs::create_dir_all(dir.join("packA").join("META-INF")).unwrap();
        std::fs::create_dir_all(dir.join("packB")).unwrap();
        std::fs::write(dir.join("packB").join("fabric.mod.json"), b"{}").unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join("normal.jar"), b"x").unwrap();

        let mut found = extracted_mod_dirs(&dir);
        found.sort();
        assert_eq!(found, vec!["packA".to_string(), "packB".to_string()]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_reverses_renamed_mods() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let base = std::env::temp_dir().join(format!("docl-repair-rename-{stamp}"));
        let backup = base.join("backup");
        let game = base.join("game");
        let old_rel = Path::new("versions")
            .join("testpack")
            .join("mods")
            .join("Old Mod.jar");
        let new_rel = Path::new("versions")
            .join("testpack")
            .join("mods")
            .join("Old_Mod.jar");
        let new_path = game.join(&new_rel);
        std::fs::create_dir_all(new_path.parent().unwrap()).unwrap();
        std::fs::write(&new_path, b"x").unwrap();
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(
            backup.join("renames.txt"),
            format!("{}|{}\n", old_rel.display(), new_rel.display()),
        )
        .unwrap();

        restore_renames(&backup, &game);
        assert!(game.join(&old_rel).is_file());
        assert!(!new_path.exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn config_evidence_only_targets_config_files() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let game = std::env::temp_dir().join(format!("docl-repair-config-{stamp}"));
        std::fs::create_dir_all(game.join("config")).unwrap();
        std::fs::write(game.join("config").join("bad.toml"), b"x").unwrap();
        std::fs::write(game.join("secret.txt"), b"x").unwrap();

        let evidence = vec![
            "第 10 行：java.io.IOException: config/bad.toml".to_string(),
            "第 11 行：config/../secret.txt".to_string(),
            "第 12 行：没有路径".to_string(),
        ];
        let found = config_paths_from_evidence(&game, &evidence);
        assert_eq!(found, vec![game.join("config").join("bad.toml")]);

        let _ = std::fs::remove_dir_all(&game);
    }

    /// 预览列表会截断，但总数保持完整
    #[test]
    fn preview_truncates_file_lists_but_keeps_count() {
        let mut out = RepairPreview::default();
        let files: Vec<String> = (0..MAX_PREVIEW_FILES + 5)
            .map(|index| format!("mods/{index}.jar"))
            .collect();
        push_action(&mut out, "test", "测试", "说明".into(), files, "中");
        assert_eq!(out.actions.len(), 1);
        assert_eq!(out.actions[0].count, MAX_PREVIEW_FILES + 5);
        assert_eq!(out.actions[0].files.len(), MAX_PREVIEW_FILES);
        assert_eq!(out.actions[0].risk, "中");
    }

    /// 文件路径统一成 / 分隔，便于前端展示
    #[test]
    fn preview_rel_label_uses_forward_slashes() {
        let root = Path::new("C:/game");
        let path = root.join("versions").join("pack").join("pack.json");
        assert_eq!(rel_label(&path, root), "versions/pack/pack.json");
    }

    /// 清单缺失时，预览只给出重新下载清单这一项
    #[test]
    fn preview_without_version_json_reports_download() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let cfg = Config {
            game_dir: std::env::temp_dir()
                .join(format!("docl-preview-{stamp}"))
                .to_string_lossy()
                .to_string(),
            ..Config::default()
        };
        let preview = preview(&cfg, "docl-preview-missing", &RepairOptions::default());
        assert_eq!(preview.actions.len(), 1);
        assert_eq!(preview.actions[0].id, "version-json");
        assert!(!preview.issues.is_empty());
    }

    /// 备份目录里 renames.txt 不计入文件数，示例路径最多 8 个
    #[test]
    fn counts_backup_files_without_rename_manifest() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("docl-backup-count-{stamp}"));
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        std::fs::write(dir.join("mods").join("a.jar"), b"a").unwrap();
        std::fs::write(dir.join("renames.txt"), b"a|b").unwrap();
        let (files, samples) = count_backup_files(&dir);
        assert_eq!(files, 1);
        assert_eq!(samples, vec!["mods/a.jar".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 按次还原只接受当前版本的备份目录名
    #[test]
    fn restore_at_rejects_foreign_backup_ids() {
        let cfg = Config::default();
        assert!(restore_at(&cfg, "1.20.1", "../evil").is_err());
        assert!(restore_at(&cfg, "1.20.1", "1.20.2-forge-1").is_err());
        assert!(restore_at(&cfg, "1.20.1", "1.20.1/../evil").is_err());
        // 1.20.1 不能匹配到 1.20.1-forge 的备份
        assert!(restore_at(&cfg, "1.20.1", "1.20.1-forge-1700000000").is_err());
    }

    /// 备份目录名必须以纯数字时间戳结尾，且属于当前游戏目录
    #[test]
    fn backup_belongs_requires_numeric_suffix() {
        let root = Path::new("C:/games/.minecraft");
        let hash = short_path_hash(root);
        assert!(backup_belongs(
            &format!("1.20.1-{hash}-1700000000"),
            "1.20.1",
            root
        ));
        // 旧版本没有目录哈希的备份仍可识别
        assert!(backup_belongs("1.20.1-1700000000", "1.20.1", root));
        assert!(!backup_belongs("1.20.1-forge-1700000000", "1.20.1", root));
        assert!(!backup_belongs("1.20.1-", "1.20.1", root));
        assert!(!backup_belongs("1.20.1-abc", "1.20.1", root));
        // 其他游戏目录下同名版本的备份不能混用
        let other = Path::new("D:/other/.minecraft");
        assert!(!backup_belongs(
            &format!("1.20.1-{hash}-1700000000"),
            "1.20.1",
            other
        ));
    }

    /// FileAlreadyExistsException 只取游戏目录内、真实存在的路径
    #[test]
    fn file_exists_paths_only_accepts_inside_game_dir() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let game = std::env::temp_dir().join(format!("docl-file-exists-{stamp}"));
        std::fs::create_dir_all(game.join("config")).unwrap();
        let target = game.join("config").join("blocked.toml");
        std::fs::write(&target, b"x").unwrap();
        let outside = std::env::temp_dir().join(format!("docl-outside-{stamp}.txt"));
        std::fs::write(&outside, b"x").unwrap();

        let evidence = vec![
            "Caused by: java.nio.file.FileAlreadyExistsException: config/blocked.toml".to_string(),
            format!(
                "java.nio.file.FileAlreadyExistsException: {}",
                outside.display()
            ),
            "java.nio.file.FileAlreadyExistsException: config/../secret.txt".to_string(),
            "java.nio.file.FileAlreadyExistsException: config/missing.toml".to_string(),
        ];
        let found = file_exists_paths(&game, &evidence);
        assert_eq!(found, vec![target]);

        let _ = std::fs::remove_dir_all(&game);
        let _ = std::fs::remove_file(&outside);
    }

    /// 只移除指向不存在文件的 -javaagent，其余参数原样保留
    #[test]
    fn strips_only_invalid_java_agents() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("docl-javaagent-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let jar = dir.join("agent.jar");
        std::fs::write(&jar, b"jar").unwrap();
        let missing = dir.join("gone.jar");
        let args = format!(
            "-Xmx2G -javaagent:{} -javaagent:\"{}\"=opt -Dfoo=bar",
            jar.display(),
            missing.display()
        );
        let (next, removed) = strip_invalid_java_agents(&args);
        assert_eq!(removed.len(), 1);
        assert!(removed[0].contains("gone.jar"));
        assert!(next.contains(&jar.display().to_string()));
        assert!(next.contains("-Xmx2G"));
        assert!(next.contains("-Dfoo=bar"));
        assert!(!next.contains("gone.jar"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 大小正常但中央目录损坏的 jar 也要重下
    #[test]
    fn detects_broken_jar_archives() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("docl-broken-jar-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let broken = dir.join("broken.jar");
        std::fs::write(&broken, b"not a zip").unwrap();
        let text = dir.join("notes.txt");
        std::fs::write(&text, b"not a zip").unwrap();

        assert!(jar_is_broken(&broken));
        assert!(jar_is_broken(&dir.join("missing.jar")));
        assert!(!jar_is_broken(&text));
        assert!(needs_redownload(&broken, 9));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
