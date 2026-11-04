use crate::config::Config;
use crate::curseforge;
use crate::download;
use crate::install;
use crate::instance;
use crate::mirror;
use crate::moddeps;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use zip::ZipArchive;

const BASE: &str = "https://api.modrinth.com/v2";
/// Modrinth 批量接口单次最多接受的哈希数量
const HASH_BATCH: usize = 100;

#[derive(Debug, Clone, Serialize)]
pub struct ModUpdate {
    /// 当前 jar 的文件名，用于替换
    pub file_name: String,
    pub mod_name: String,
    pub current_version: String,
    pub latest_version: String,
    pub latest_file: String,
    pub download_url: String,
    pub sha1: Option<String>,
    pub project_id: String,
    pub slug: String,
    pub icon_url: Option<String>,
    pub version_type: String,
    pub game_versions: Vec<String>,
    /// modrinth / curseforge
    pub source: String,
    pub project_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModUpdateReport {
    pub version_id: String,
    pub game_version: String,
    pub loader: String,
    pub scanned: u32,
    pub matched: u32,
    pub up_to_date: u32,
    pub updates: Vec<ModUpdate>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct LocalMeta {
    id: String,
    name: String,
    version: String,
}

struct LocalJar {
    file_name: String,
    path: PathBuf,
    sha1: String,
    meta: LocalMeta,
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
    hashes: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct MrVersion {
    #[serde(default)]
    project_id: String,
    #[serde(default)]
    version_number: String,
    #[serde(default)]
    version_type: String,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    files: Vec<MrFile>,
}

#[derive(Debug, Deserialize)]
struct MrProject {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    icon_url: Option<String>,
}

fn toml_value(line: &str, key: &str) -> Option<String> {
    let rest = line.strip_prefix(key)?.trim_start();
    let rest = rest.strip_prefix('=')?.trim();
    let rest = rest.trim_matches('"').trim_matches('\'');
    if rest.is_empty() || rest.starts_with("${") {
        None
    } else {
        Some(rest.to_string())
    }
}

/// 从 mods.toml / neoforge.mods.toml 的 [[mods]] 段里取字段。
fn parse_mods_toml(text: &str) -> LocalMeta {
    let mut meta = LocalMeta::default();
    let mut in_mods = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("[[") {
            in_mods = trimmed.replace(' ', "") == "[[mods]]";
            continue;
        }
        if !in_mods {
            continue;
        }
        if let Some(value) = toml_value(trimmed, "modId") {
            meta.id = value;
        }
        if let Some(value) = toml_value(trimmed, "displayName") {
            meta.name = value;
        }
        if let Some(value) = toml_value(trimmed, "version") {
            meta.version = value;
        }
    }
    meta
}

fn read_zip_text(archive: &mut ZipArchive<File>, name: &str) -> Option<String> {
    let mut file = archive.by_name(name).ok()?;
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn read_meta(path: &Path) -> LocalMeta {
    let Ok(file) = File::open(path) else {
        return LocalMeta::default();
    };
    let Ok(mut archive) = ZipArchive::new(file) else {
        return LocalMeta::default();
    };

    if let Some(text) = read_zip_text(&mut archive, "fabric.mod.json") {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            let meta = LocalMeta {
                id: value
                    .get("id")
                    .and_then(|item| item.as_str())
                    .unwrap_or_default()
                    .to_string(),
                name: value
                    .get("name")
                    .and_then(|item| item.as_str())
                    .unwrap_or_default()
                    .to_string(),
                version: value
                    .get("version")
                    .and_then(|item| item.as_str())
                    .unwrap_or_default()
                    .to_string(),
            };
            if !meta.id.is_empty() {
                return meta;
            }
        }
    }

    if let Some(text) = read_zip_text(&mut archive, "quilt.mod.json") {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            let loader = value.get("quilt_loader");
            let id = loader
                .and_then(|item| item.get("id"))
                .and_then(|item| item.as_str())
                .unwrap_or_default()
                .to_string();
            let version = loader
                .and_then(|item| item.get("version"))
                .and_then(|item| item.as_str())
                .unwrap_or_default()
                .to_string();
            let name = loader
                .and_then(|item| item.get("metadata"))
                .and_then(|item| item.get("name"))
                .and_then(|item| item.as_str())
                .unwrap_or_default()
                .to_string();
            if !id.is_empty() {
                return LocalMeta { id, name, version };
            }
        }
    }

    for name in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
        if let Some(text) = read_zip_text(&mut archive, name) {
            let meta = parse_mods_toml(&text);
            if !meta.id.is_empty() {
                return meta;
            }
        }
    }

    LocalMeta::default()
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("读取文件失败：{}", path.display()))?;
    let mut hasher = Sha1::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_jars(dir: &Path) -> Vec<LocalJar> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut jars = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !file_name.to_lowercase().ends_with(".jar") {
            continue;
        }
        let Ok(sha1) = hash_file(&path) else {
            continue;
        };
        let meta = read_meta(&path);
        jars.push(LocalJar {
            file_name,
            path,
            sha1,
            meta,
        });
    }
    jars.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    jars
}

#[derive(Serialize)]
struct UpdateQuery<'a> {
    hashes: Vec<String>,
    algorithm: &'a str,
    loaders: Vec<String>,
    game_versions: Vec<String>,
}

async fn fetch_latest(
    client: &reqwest::Client,
    hashes: &[String],
    loader: &str,
    game_version: &str,
) -> Result<HashMap<String, MrVersion>> {
    let mut result = HashMap::new();
    for chunk in hashes.chunks(HASH_BATCH) {
        let body = UpdateQuery {
            hashes: chunk.to_vec(),
            algorithm: "sha1",
            loaders: vec![loader.to_string()],
            game_versions: vec![game_version.to_string()],
        };
        let response = client
            .post(format!("{BASE}/version_files/update"))
            .json(&body)
            .send()
            .await
            .context("连接 Modrinth 失败")?;
        if !response.status().is_success() {
            bail!("Modrinth 返回 HTTP {}", response.status());
        }
        let batch: HashMap<String, MrVersion> =
            response.json().await.context("解析 Modrinth 响应失败")?;
        result.extend(batch);
    }
    Ok(result)
}

async fn fetch_projects(
    client: &reqwest::Client,
    ids: &[String],
) -> HashMap<String, MrProject> {
    if ids.is_empty() {
        return HashMap::new();
    }
    let ids_param = match serde_json::to_string(ids) {
        Ok(value) => value,
        Err(_) => return HashMap::new(),
    };
    let Ok(url) = reqwest::Url::parse_with_params(
        &format!("{BASE}/projects"),
        &[("ids", ids_param.as_str())],
    ) else {
        return HashMap::new();
    };
    match download::get_json::<Vec<MrProject>>(client, url.as_str(), false).await {
        Ok(list) => list
            .into_iter()
            .filter(|item| !item.id.is_empty())
            .map(|item| (item.id.clone(), item))
            .collect(),
        Err(_) => HashMap::new(),
    }
}

const CF_MOD_CLASS: i64 = 6;
/// 单次检查最多尝试多少个未识别文件去 CurseForge 匹配，避免请求过多。
const CF_MATCH_LIMIT: usize = 12;

/// 从 jar 文件名里提取可用于搜索的项目名，去掉版本号与加载器后缀。
fn search_query(file_name: &str) -> String {
    let stem = file_name
        .trim_end_matches(".disabled")
        .trim_end_matches(".jar");
    let stop = [
        "fabric",
        "forge",
        "neoforge",
        "quilt",
        "minecraft",
        "mc",
        "universal",
        "client",
        "server",
        "alpha",
        "beta",
        "release",
        "shaders",
        "mod",
        "mods",
    ];
    let mut tokens: Vec<String> = Vec::new();
    for part in stem.split(|ch: char| matches!(ch, '-' | '_' | ' ' | '+' | '.')) {
        let lower = part.to_lowercase();
        if lower.chars().count() < 2 || stop.contains(&lower.as_str()) {
            continue;
        }
        let first = lower.chars().next().unwrap();
        if first.is_ascii_digit() {
            continue;
        }
        if lower.starts_with('v')
            && lower[1..]
                .chars()
                .all(|ch| ch.is_ascii_digit() || ch == '.')
        {
            continue;
        }
        if lower.starts_with("mc")
            && lower[2..]
                .chars()
                .next()
                .map(|ch| ch.is_ascii_digit())
                .unwrap_or(false)
        {
            continue;
        }
        tokens.push(part.to_string());
        if tokens.len() >= 3 {
            break;
        }
    }
    tokens.join(" ")
}

fn normalized(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

fn name_matches(query: &str, name: &str) -> bool {
    let name = normalized(name);
    let tokens: Vec<String> = query
        .split_whitespace()
        .map(normalized)
        .filter(|token| token.len() >= 3)
        .collect();
    if tokens.is_empty() {
        return false;
    }
    tokens.iter().any(|token| name.contains(token))
}

#[allow(clippy::type_complexity)]
async fn curseforge_updates(
    client: &reqwest::Client,
    game_version: &str,
    loader: &str,
    jars: &[&LocalJar],
) -> (Vec<ModUpdate>, u32, u32, HashSet<String>, Vec<String>) {
    let mut updates = Vec::new();
    let mut matched = 0u32;
    let mut up_to_date = 0u32;
    let mut handled = HashSet::new();
    let mut notes = Vec::new();

    for jar in jars.iter().take(CF_MATCH_LIMIT) {
        let query = search_query(&jar.file_name);
        if query.is_empty() {
            continue;
        }
        let Ok(result) = curseforge::search(
            client,
            CF_MOD_CLASS,
            &query,
            Some(game_version),
            Some(loader),
            None,
            0,
            5,
            2,
        )
        .await
        else {
            continue;
        };
        let Some(project) = result
            .items
            .into_iter()
            .find(|item| name_matches(&query, &item.name))
        else {
            continue;
        };
        let Ok(mod_id) = project.id.parse::<i64>() else {
            continue;
        };
        let Ok(files) =
            curseforge::files(client, mod_id, Some(game_version), Some(loader)).await
        else {
            continue;
        };
        let Some(file) = files.into_iter().next() else {
            continue;
        };
        handled.insert(jar.file_name.clone());
        matched += 1;
        if file.file_name.eq_ignore_ascii_case(&jar.file_name) {
            up_to_date += 1;
            continue;
        }
        let Some(url) = file
            .download_url
            .filter(|value| !value.trim().is_empty())
        else {
            notes.push(format!("{} 在 CurseForge 未开放第三方下载，已跳过", project.name));
            continue;
        };
        updates.push(ModUpdate {
            file_name: jar.file_name.clone(),
            mod_name: if jar.meta.name.is_empty() {
                project.name.clone()
            } else {
                jar.meta.name.clone()
            },
            current_version: jar.meta.version.clone(),
            latest_version: if file.name.is_empty() {
                file.file_name.clone()
            } else {
                file.name.clone()
            },
            latest_file: file.file_name.clone(),
            download_url: url,
            sha1: None,
            project_id: project.id.clone(),
            slug: project.slug.clone(),
            icon_url: if project.logo_url.is_empty() {
                None
            } else {
                Some(project.logo_url.clone())
            },
            version_type: match file.release_type {
                2 => "beta",
                3 => "alpha",
                _ => "release",
            }
            .into(),
            game_versions: file.game_versions,
            source: "curseforge".into(),
            project_url: format!(
                "https://www.curseforge.com/minecraft/mc-mods/{}",
                project.slug
            ),
        });
    }

    if jars.len() > CF_MATCH_LIMIT {
        notes.push(format!(
            "另有 {} 个未识别文件未尝试 CurseForge 匹配",
            jars.len() - CF_MATCH_LIMIT
        ));
    }
    (updates, matched, up_to_date, handled, notes)
}

pub async fn check(
    client: &reqwest::Client,
    cfg: &Config,
    version_id: &str,
) -> Result<ModUpdateReport> {
    let (game_version, loader) = install::instance_context(cfg, version_id)?;
    let dir = instance::content_dir(cfg, version_id, "mods")?;
    let jars = collect_jars(&dir);
    let mut report = ModUpdateReport {
        version_id: version_id.to_string(),
        game_version: game_version.clone(),
        loader: loader.clone(),
        scanned: jars.len() as u32,
        matched: 0,
        up_to_date: 0,
        updates: Vec::new(),
        notes: Vec::new(),
    };

    if jars.is_empty() {
        report.notes.push("mods 目录里还没有 jar 文件".into());
        return Ok(report);
    }
    if loader == "vanilla" {
        report
            .notes
            .push("当前版本没有安装加载器，Mod 更新检测已跳过".into());
        return Ok(report);
    }

    let hashes: Vec<String> = jars.iter().map(|jar| jar.sha1.clone()).collect();
    let latest = fetch_latest(client, &hashes, &loader, &game_version).await?;
    report.matched = jars
        .iter()
        .filter(|jar| latest.contains_key(&jar.sha1))
        .count() as u32;
    report.up_to_date = jars
        .iter()
        .filter(|jar| match latest.get(&jar.sha1) {
            Some(version) => version
                .files
                .iter()
                .find(|file| file.primary)
                .or_else(|| version.files.first())
                .and_then(|file| file.hashes.get("sha1"))
                .map(|sha| sha.eq_ignore_ascii_case(&jar.sha1))
                .unwrap_or(false),
            None => false,
        })
        .count() as u32;

    let mut project_ids: Vec<String> = latest
        .values()
        .map(|version| version.project_id.clone())
        .filter(|id| !id.is_empty())
        .collect();
    project_ids.sort();
    project_ids.dedup();
    let projects = fetch_projects(client, &project_ids).await;

    for jar in &jars {
        let Some(version) = latest.get(&jar.sha1) else {
            continue;
        };
        let file = version
            .files
            .iter()
            .find(|file| file.primary)
            .or_else(|| version.files.first());
        let Some(file) = file else {
            continue;
        };
        let latest_sha1 = file.hashes.get("sha1").cloned();
        if latest_sha1
            .as_deref()
            .map(|sha| sha.eq_ignore_ascii_case(&jar.sha1))
            .unwrap_or(false)
        {
            continue;
        }
        if file.url.is_empty() || file.filename.is_empty() {
            continue;
        }
        let project = projects.get(&version.project_id);
        let display_name = if !jar.meta.name.is_empty() {
            jar.meta.name.clone()
        } else if let Some(project) = project.filter(|item| !item.title.is_empty()) {
            project.title.clone()
        } else if !jar.meta.id.is_empty() {
            jar.meta.id.clone()
        } else {
            jar.path
                .file_stem()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| jar.file_name.clone())
        };

        report.updates.push(ModUpdate {
            file_name: jar.file_name.clone(),
            mod_name: display_name,
            current_version: jar.meta.version.clone(),
            latest_version: if version.version_number.is_empty() {
                "新版本".into()
            } else {
                version.version_number.clone()
            },
            latest_file: file.filename.clone(),
            download_url: file.url.clone(),
            sha1: latest_sha1,
            project_id: version.project_id.clone(),
            slug: project.map(|item| item.slug.clone()).unwrap_or_default(),
            icon_url: project.and_then(|item| item.icon_url.clone()),
            version_type: if version.version_type.is_empty() {
                "release".into()
            } else {
                version.version_type.clone()
            },
            game_versions: version.game_versions.clone(),
            source: "modrinth".into(),
            project_url: project
                .map(|item| {
                    if item.slug.is_empty() {
                        format!("https://modrinth.com/project/{}", version.project_id)
                    } else {
                        format!("https://modrinth.com/mod/{}", item.slug)
                    }
                })
                .unwrap_or_default(),
        });
    }
    let mut unknown: Vec<&LocalJar> = jars
        .iter()
        .filter(|jar| !latest.contains_key(&jar.sha1))
        .collect();

    if !unknown.is_empty() {
        let (cf_updates, cf_matched, cf_up_to_date, handled, cf_notes) =
            curseforge_updates(client, &game_version, &loader, &unknown).await;
        report.updates.extend(cf_updates);
        report.matched += cf_matched;
        report.up_to_date += cf_up_to_date;
        report.notes.extend(cf_notes);
        unknown.retain(|jar| !handled.contains(&jar.file_name));
    }

    report.updates.sort_by(|a, b| a.mod_name.to_lowercase().cmp(&b.mod_name.to_lowercase()));

    for jar in unknown.iter().take(8) {
        report.notes.push(format!(
            "未找到适配当前版本的更新：{}（可能已停止维护）",
            jar.file_name
        ));
    }
    if unknown.len() > 8 {
        report
            .notes
            .push(format!("另有 {} 个文件未识别", unknown.len() - 8));
    }

    // 同一个 mod 装了多份时，游戏可能在加载阶段直接报冲突
    for (id, files) in moddeps::duplicate_mods(&dir) {
        report.notes.push(format!(
            "重复模组 {id}：{} 个文件（{}）",
            files.len(),
            files
                .iter()
                .filter_map(|path| path.file_name().map(|name| name.to_string_lossy().to_string()))
                .take(3)
                .collect::<Vec<_>>()
                .join("、")
        ));
    }

    Ok(report)
}

fn safe_file_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name == ".."
        || !name.to_lowercase().ends_with(".jar")
    {
        bail!("无效的 Mod 文件名");
    }
    Ok(name.to_string())
}

/// 下载并替换单个 Mod，返回新文件路径。
pub async fn install_update(
    client: &reqwest::Client,
    cfg: &Config,
    version_id: &str,
    url: &str,
    file_name: &str,
    old_name: &str,
    expected_sha1: Option<String>,
) -> Result<String> {
    let dir = instance::content_dir(cfg, version_id, "mods")?;
    fs::create_dir_all(&dir)?;
    let file_name = safe_file_name(file_name)?;
    let target = dir.join(&file_name);
    let tmp = dir.join(format!("{file_name}.download"));

    // 换源与续传交给公共下载器：CDN 断流时不再让单个 Mod 更新卡住
    let mut urls = vec![mirror::rewrite(url), url.to_string()];
    urls.dedup();
    let mut ignore = |_: u64, _: u64| {};
    download::download_stream(client, &urls, &tmp, &mut ignore).await?;

    let mut hasher = Sha1::new();
    download::hash_existing(&tmp, &mut hasher).await?;
    let actual = format!("{:x}", hasher.finalize());
    if let Some(expected) = expected_sha1.filter(|value| !value.is_empty()) {
        if !expected.eq_ignore_ascii_case(&actual) {
            let _ = fs::remove_file(&tmp);
            bail!("文件校验失败（SHA1 不匹配）：{file_name}");
        }
    }

    if target.exists() {
        fs::remove_file(&target).context("删除旧文件失败，游戏可能正在运行")?;
    }
    tokio::fs::rename(&tmp, &target).await?;

    if !old_name.is_empty() && old_name != file_name {
        if let Ok(old) = safe_file_name(old_name) {
            let old_path = dir.join(old);
            if old_path.exists() {
                let _ = fs::remove_file(old_path);
            }
        }
    }

    Ok(target.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    #[test]
    fn parses_forge_mods_toml() {
        let text = r#"
modLoader="javafml"
loaderVersion="[47,)"

[[mods]]
modId="jei"
version="15.2.0.27"
displayName="Just Enough Items"

[[dependencies.jei]]
modId="forge"
"#;
        let meta = parse_mods_toml(text);
        assert_eq!(meta.id, "jei");
        assert_eq!(meta.name, "Just Enough Items");
        assert_eq!(meta.version, "15.2.0.27");
    }

    #[test]
    fn reads_fabric_meta_from_zip() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                "fabric.mod.json",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer
            .write_all(br#"{"id":"sodium","name":"Sodium","version":"0.5.8"}"#)
            .unwrap();
        let bytes = writer.finish().unwrap().into_inner();

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("docl-mod-{stamp}.jar"));
        fs::write(&path, bytes).unwrap();
        let meta = read_meta(&path);
        let _ = fs::remove_file(&path);
        assert_eq!(meta.id, "sodium");
        assert_eq!(meta.name, "Sodium");
        assert_eq!(meta.version, "0.5.8");
    }

    #[test]
    fn derives_search_query_from_file_name() {
        assert_eq!(search_query("sodium-fabric-0.6.13+mc1.21.3.jar"), "sodium");
        assert_eq!(
            search_query("jei-1.20.1-forge-15.2.0.27.jar"),
            "jei"
        );
        assert_eq!(search_query("create-1.20.1-0.5.1-f.jar"), "create");
        assert!(name_matches("jei", "Just Enough Items (JEI)"));
        assert!(!name_matches("jei", "JourneyMap"));
    }

    #[test]
    fn rejects_unsafe_update_names() {
        assert!(safe_file_name("sodium-0.5.8.jar").is_ok());
        assert!(safe_file_name("../evil.jar").is_err());
        assert!(safe_file_name("sub/evil.jar").is_err());
        assert!(safe_file_name("readme.txt").is_err());
    }
}
