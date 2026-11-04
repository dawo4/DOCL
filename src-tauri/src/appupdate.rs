use crate::config;
use crate::http;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub available: bool,
    pub url: String,
    /// 便携版压缩包地址，留空表示只发安装包
    pub portable_url: String,
    pub notes: String,
}

#[derive(Deserialize)]
struct Manifest {
    version: String,
    url: String,
    #[serde(default)]
    portable_url: String,
    #[serde(default)]
    notes: String,
}

#[derive(Clone, Serialize)]
pub struct UpdateProgress {
    pub downloaded: u64,
    pub total: u64,
}

fn version_numbers(raw: &str) -> Vec<u64> {
    raw.split(|c: char| !c.is_ascii_digit())
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
}

/// 逐段比较版本号，1.10.0 大于 1.9.2。
fn is_newer(latest: &str, current: &str) -> bool {
    let newer = version_numbers(latest);
    let older = version_numbers(current);
    for index in 0..newer.len().max(older.len()) {
        let left = *newer.get(index).unwrap_or(&0);
        let right = *older.get(index).unwrap_or(&0);
        if left != right {
            return left > right;
        }
    }
    false
}

fn validate_url(url: &str) -> Result<&str> {
    let url = url.trim();
    if url.is_empty() {
        bail!("还没有配置更新清单地址");
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        bail!("更新清单地址必须以 http:// 或 https:// 开头");
    }
    Ok(url)
}

pub async fn check(url: &str) -> Result<UpdateInfo> {
    let url = validate_url(url)?;
    let manifest: Manifest = http::client()
        .get(url)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .context("请求更新清单失败")?
        .error_for_status()
        .context("更新清单返回了错误状态")?
        .json()
        .await
        .context("更新清单格式不正确")?;
    if manifest.version.trim().is_empty() || manifest.url.trim().is_empty() {
        bail!("更新清单缺少 version 或 url 字段");
    }
    Ok(UpdateInfo {
        current: CURRENT_VERSION.to_string(),
        available: is_newer(&manifest.version, CURRENT_VERSION),
        latest: manifest.version,
        url: manifest.url,
        portable_url: manifest.portable_url,
        notes: manifest.notes,
    })
}

fn download_name(url: &str, version: &str) -> String {
    let last = url
        .rsplit(['/', '?'])
        .next()
        .map(|name| name.trim().to_string())
        .unwrap_or_default();
    if (last.ends_with(".exe") || last.ends_with(".msi") || last.ends_with(".zip"))
        && last
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return last;
    }
    let version = version
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("DOCL-{version}-setup.exe")
}

pub async fn download(app: &AppHandle, url: &str, version: &str) -> Result<PathBuf> {
    let url = validate_url(url)?;
    let dir = config::data_dir().join("updates");
    tokio::fs::create_dir_all(&dir).await?;
    let target = dir.join(download_name(url, version));

    let client = http::client();
    let urls = vec![url.to_string()];
    let mut on_progress = |downloaded: u64, total: u64| {
        let _ = app.emit(
            "update-progress",
            UpdateProgress {
                downloaded,
                total,
            },
        );
    };
    crate::download::download_stream(&client, &urls, &target, &mut on_progress).await?;
    Ok(target)
}

/// 启动完整安装包，随后关闭启动器让安装程序接管。
pub fn install(app: &AppHandle, path: &str) -> Result<()> {
    let target = PathBuf::from(path);
    if !target.is_file() {
        bail!("安装包不存在：{path}");
    }
    std::process::Command::new(&target)
        .spawn()
        .context("启动安装程序失败")?;
    app.exit(0);
    Ok(())
}

/// 便携版原地更新：解压更新包、替换自身、重启。
/// 运行中的 exe 允许改名不允许删除，因此先备份为 .old 再落新文件，
/// 落新失败立即回滚；.old 由新进程启动时清理。
pub fn portable_install(app: &AppHandle, zip_path: &str) -> Result<()> {
    if !crate::config::is_portable() {
        bail!("仅便携版支持原地更新");
    }
    let current = std::env::current_exe().context("无法定位当前程序")?;
    let backup = current.with_extension("exe.old");
    let zip_file =
        std::fs::File::open(zip_path).with_context(|| format!("无法读取更新包：{zip_path}"))?;
    let mut archive = zip::ZipArchive::new(zip_file).context("更新包格式不正确")?;

    // 解压到临时目录，确认包内有新的 DOCL.exe 再动当前程序
    let stage_dir = crate::config::data_dir().join("updates").join("pending");
    let _ = std::fs::remove_dir_all(&stage_dir);
    std::fs::create_dir_all(&stage_dir)?;
    archive.extract(&stage_dir).context("解压更新包失败")?;
    let staged = stage_dir.join("DOCL.exe");
    if !staged.is_file() {
        let _ = std::fs::remove_dir_all(&stage_dir);
        bail!("更新包里没有找到 DOCL.exe");
    }

    std::fs::rename(&current, &backup).context("备份当前程序失败")?;
    if let Err(err) = std::fs::copy(&staged, &current) {
        let _ = std::fs::rename(&backup, &current);
        let _ = std::fs::remove_dir_all(&stage_dir);
        bail!("替换程序失败：{err}");
    }
    let _ = std::fs::remove_dir_all(&stage_dir);

    std::process::Command::new(&current)
        .spawn()
        .context("重启失败")?;
    app.exit(0);
    Ok(())
}
