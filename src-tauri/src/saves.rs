use crate::config::Config;
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 一份存档备份。
#[derive(Debug, Clone, Serialize)]
pub struct WorldBackup {
    /// 存档文件夹名
    pub world: String,
    pub path: String,
    pub size: u64,
    /// 备份时间（Unix 秒）
    pub created: u64,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
}

fn saves_dir(cfg: &Config, version_id: &str) -> Result<PathBuf> {
    crate::instance::content_dir(cfg, version_id, "saves")
}

/// 解析到某个存档目录，同时校验目录名不会越出 saves。
pub fn world_dir(cfg: &Config, version_id: &str, folder: &str) -> Result<PathBuf> {
    let name = safe_world_name(folder)?;
    Ok(saves_dir(cfg, version_id)?.join(name))
}

/// 备份统一放在实例目录下的 backups/，不跟着 saves 被游戏扫描。
pub fn backups_dir(cfg: &Config, version_id: &str) -> Result<PathBuf> {
    let dir = crate::instance::game_dir(cfg, version_id).join("backups");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// 只接受 saves 下的一级文件夹名，挡掉路径穿越。
fn safe_world_name(folder: &str) -> Result<&str> {
    let name = folder.trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.starts_with('.')
        || name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|'])
    {
        bail!("非法的存档目录名：{folder}");
    }
    Ok(name)
}

fn world_from_backup_name(file: &str) -> String {
    let stem = file
        .strip_suffix(".zip")
        .or_else(|| file.strip_suffix(".ZIP"))
        .unwrap_or(file);
    match stem.rsplit_once("__") {
        Some((world, stamp)) if stamp.parse::<u64>().is_ok() => world.to_string(),
        _ => stem.to_string(),
    }
}

pub fn list_backups(cfg: &Config, version_id: &str) -> Result<Vec<WorldBackup>> {
    let dir = backups_dir(cfg, version_id)?;
    let mut list = Vec::new();
    for entry in std::fs::read_dir(&dir)?.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !file_name.to_lowercase().ends_with(".zip") {
            continue;
        }
        let created = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_secs())
            .unwrap_or(0);
        list.push(WorldBackup {
            world: world_from_backup_name(&file_name),
            path: path.to_string_lossy().to_string(),
            size: meta.len(),
            created,
        });
    }
    list.sort_by(|a, b| b.created.cmp(&a.created));
    Ok(list)
}

/// 把一个存档文件夹压成 zip，返回备份信息。
pub fn backup_world(cfg: &Config, version_id: &str, folder: &str) -> Result<WorldBackup> {
    let source = world_dir(cfg, version_id, folder)?;
    let name = source
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| folder.to_string());
    if !source.join("level.dat").is_file() {
        bail!("存档不存在或缺少 level.dat：{name}");
    }
    let dir = backups_dir(cfg, version_id)?;
    let path = dir.join(format!("{name}__{}.zip", now_secs()));
    write_zip_dir(&source, &path)?;
    let meta = std::fs::metadata(&path)?;
    let created = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_secs())
        .unwrap_or_else(now_secs);
    Ok(WorldBackup {
        world: name.to_string(),
        path: path.to_string_lossy().to_string(),
        size: meta.len(),
        created,
    })
}

/// 备份路径必须落在当前实例的 backups/ 里，避免前端传入任意路径。
fn valid_backup_path(cfg: &Config, version_id: &str, path: &str) -> Result<PathBuf> {
    let dir = std::fs::canonicalize(backups_dir(cfg, version_id)?)?;
    let file = std::fs::canonicalize(path).with_context(|| format!("备份文件不存在：{path}"))?;
    if !file.starts_with(&dir) {
        bail!("备份文件不在当前实例的备份目录内");
    }
    if !file.is_file() {
        bail!("备份路径不是文件");
    }
    Ok(file)
}

pub fn delete_backup(cfg: &Config, version_id: &str, path: &str) -> Result<()> {
    let file = valid_backup_path(cfg, version_id, path)?;
    std::fs::remove_file(&file).with_context(|| format!("删除备份失败：{}", file.display()))?;
    Ok(())
}

/// 还原备份：先解到临时目录，再整体搬进 saves；
/// 同名存档已存在时先改名为「旧存档 时间戳」，不直接覆盖。
pub fn restore_backup(cfg: &Config, version_id: &str, path: &str) -> Result<String> {
    let file = valid_backup_path(cfg, version_id, path)?;
    let fallback = world_from_backup_name(
        file.file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_default()
            .as_str(),
    );
    import_archive(cfg, version_id, &file, &fallback, true)
}

/// 导入外部存档压缩包（.zip / .mcworld）。
pub fn import_world(cfg: &Config, version_id: &str, zip_path: &str) -> Result<String> {
    let file = PathBuf::from(zip_path);
    if !file.is_file() {
        bail!("找不到存档压缩包：{zip_path}");
    }
    let fallback = file
        .file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "导入的存档".to_string());
    import_archive(cfg, version_id, &file, &fallback, false)
}

fn import_archive(
    cfg: &Config,
    version_id: &str,
    archive: &Path,
    fallback_name: &str,
    keep_old: bool,
) -> Result<String> {
    let saves = saves_dir(cfg, version_id)?;
    let staging = saves.join(format!(".import-{}", uuid::Uuid::new_v4().simple()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging).ok();
    }
    let extracted = extract_zip(archive, &staging);
    if let Err(err) = extracted {
        std::fs::remove_dir_all(&staging).ok();
        return Err(err);
    }
    let result = place_world(&saves, &staging, fallback_name, keep_old);
    std::fs::remove_dir_all(&staging).ok();
    result
}

fn place_world(
    saves: &Path,
    staging: &Path,
    fallback_name: &str,
    keep_old: bool,
) -> Result<String> {
    let root = find_world_root(staging)?;
    let raw_name = if root == staging {
        fallback_name.to_string()
    } else {
        root.file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| fallback_name.to_string())
    };
    let name = clean_world_name(&raw_name, fallback_name);
    let dest = saves.join(&name);
    if dest.exists() {
        if !keep_old {
            bail!("存档「{name}」已存在，请先改名或删除旧存档");
        }
        let aside = saves.join(format!("{name} (旧存档 {})", now_secs()));
        std::fs::rename(&dest, &aside)
            .with_context(|| format!("无法移开旧存档：{}", dest.display()))?;
    }
    std::fs::rename(&root, &dest)
        .with_context(|| format!("无法写入存档：{}", dest.display()))?;
    Ok(name)
}

fn clean_world_name(name: &str, fallback: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect();
    let cleaned = cleaned
        .trim()
        .trim_end_matches(['.', ' '])
        .to_string();
    if cleaned.is_empty() || cleaned.starts_with('.') {
        fallback.to_string()
    } else {
        cleaned
    }
}

/// 压缩包里的存档根：根目录的 level.dat，或子目录 / 再深一层里的 level.dat。
fn find_world_root(staging: &Path) -> Result<PathBuf> {
    if staging.join("level.dat").is_file() {
        return Ok(staging.to_path_buf());
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(staging)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    for dir in &dirs {
        if dir.join("level.dat").is_file() {
            return Ok(dir.clone());
        }
    }
    for dir in &dirs {
        let Ok(nested) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut inner: Vec<PathBuf> = nested
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir() && path.join("level.dat").is_file())
            .collect();
        inner.sort();
        if let Some(found) = inner.into_iter().next() {
            return Ok(found);
        }
    }
    Err(anyhow!("压缩包里没有找到存档（缺少 level.dat）"))
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<()> {
    let file = File::open(archive).with_context(|| format!("无法打开压缩包：{}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file).context("压缩包损坏或格式不支持")?;
    std::fs::create_dir_all(dest)?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        if relative.to_string_lossy().starts_with("__MACOSX") {
            continue;
        }
        let target = dest.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = File::create(&target)?;
        std::io::copy(&mut entry, &mut out)?;
    }
    Ok(())
}

fn write_zip_dir(source: &Path, dest: &Path) -> Result<()> {
    let file = File::create(dest).with_context(|| format!("无法创建备份：{}", dest.display()))?;
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    add_dir(&mut writer, source, source, options)?;
    writer.finish()?;
    Ok(())
}

fn add_dir(
    writer: &mut zip::ZipWriter<File>,
    root: &Path,
    dir: &Path,
    options: zip::write::SimpleFileOptions,
) -> Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        let relative = path.strip_prefix(root)?;
        let name = relative.to_string_lossy().replace('\\', "/");
        if path.is_dir() {
            writer.add_directory(format!("{name}/"), options)?;
            add_dir(writer, root, &path, options)?;
            continue;
        }
        writer.start_file(name, options)?;
        let mut file = File::open(&path)?;
        std::io::copy(&mut file, writer)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{clean_world_name, find_world_root, world_from_backup_name};
    use std::io::Read;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("docl-saves-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_backup_world_name() {
        assert_eq!(world_from_backup_name("我的世界__1730000000.zip"), "我的世界");
        assert_eq!(world_from_backup_name("world.zip"), "world");
    }

    #[test]
    fn finds_world_root_at_staging_and_child() {
        let root = temp_dir("root");
        std::fs::write(root.join("level.dat"), b"data").unwrap();
        assert_eq!(find_world_root(&root).unwrap(), root);
        std::fs::remove_dir_all(&root).ok();

        let root = temp_dir("child");
        let world = root.join("存档");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::write(world.join("level.dat"), b"data").unwrap();
        assert_eq!(find_world_root(&root).unwrap(), world);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rejects_archive_without_level_dat() {
        let root = temp_dir("empty");
        std::fs::write(root.join("readme.txt"), b"note").unwrap();
        assert!(find_world_root(&root).is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn sanitizes_world_folder_names() {
        assert_eq!(clean_world_name("a:b/c*?", "fallback"), "a_b_c__");
        assert_eq!(clean_world_name("  .hidden ", "fallback"), "fallback");
        assert_eq!(clean_world_name("  ", "fallback"), "fallback");
    }

    /// 备份 → 改动 → 还原：应恢复旧内容，并把被覆盖的存档改名保留
    #[test]
    fn backup_and_restore_roundtrip() {
        use crate::config::Config;

        let root = temp_dir("roundtrip");
        let mut cfg = Config::default();
        cfg.game_dir = root.to_string_lossy().to_string();
        cfg.version_isolation = false;

        let world = root.join("saves").join("world");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::write(world.join("level.dat"), b"v1").unwrap();

        let backup = super::backup_world(&cfg, "1.21.1", "world").unwrap();
        std::fs::write(world.join("level.dat"), b"v2").unwrap();

        let name = super::restore_backup(&cfg, "1.21.1", &backup.path).unwrap();
        assert_eq!(name, "world");
        let restored = std::fs::read(world.join("level.dat")).unwrap();
        assert_eq!(restored, b"v1");

        let kept = std::fs::read_dir(root.join("saves"))
            .unwrap()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().contains("旧存档"));
        assert!(kept, "被覆盖的存档应改名保留");

        std::fs::remove_dir_all(&root).ok();
    }

    /// 备份路径不在实例 backups 目录内时必须拒绝删除
    #[test]
    fn delete_backup_rejects_outside_path() {
        use crate::config::Config;

        let root = temp_dir("guard");
        let mut cfg = Config::default();
        cfg.game_dir = root.to_string_lossy().to_string();
        cfg.version_isolation = false;
        std::fs::create_dir_all(root.join("backups")).unwrap();
        let outside = root.join("outside.zip");
        std::fs::write(&outside, b"not a backup").unwrap();

        let result = super::delete_backup(
            &cfg,
            "1.21.1",
            &outside.to_string_lossy().to_string(),
        );
        assert!(result.is_err());
        assert!(outside.exists());

        std::fs::remove_dir_all(&root).ok();
    }

    /// 备份 zip 里应包含 level.dat 与 region 目录
    #[test]
    fn backup_zip_contains_world_files() {        let root = temp_dir("zip");
        let world = root.join("world");
        std::fs::create_dir_all(world.join("region")).unwrap();
        std::fs::write(world.join("level.dat"), b"level").unwrap();
        std::fs::write(world.join("region").join("r.0.0.mca"), b"chunk").unwrap();
        let zip = root.join("backup.zip");
        super::write_zip_dir(&world, &zip).unwrap();

        let file = std::fs::File::open(&zip).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut names: Vec<String> = Vec::new();
        for index in 0..archive.len() {
            names.push(archive.by_index(index).unwrap().name().to_string());
        }
        assert!(names.iter().any(|name| name == "level.dat"));
        assert!(names.iter().any(|name| name == "region/r.0.0.mca"));

        let mut level = archive.by_name("level.dat").unwrap();
        let mut content = String::new();
        level.read_to_string(&mut content).unwrap();
        assert_eq!(content, "level");
    }
}
