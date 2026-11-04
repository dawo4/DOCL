//! Forge / NeoForge 原生安装器。
//!
//! 官方安装器依赖自身 JVM 联网（`--mirror` 只覆盖 Forge Maven，映射表和
//! 客户端补丁仍要直连 Mojang），网络一断就会出现「退出码 0 / 1 但清单残缺」
//! 的假成功。这里照官方安装器 2.x 的流程（ClientInstall + PostProcessors）
//! 把安装收进启动器：清单直接从安装器压缩包抽取写盘，依赖库走启动器自己的
//! 镜像下载管线，处理器（SRG 重映射、二进制补丁等）用本地 Java 逐个执行。

use crate::config::Config;
use crate::download::{self, FileTask, ProgressState};
use crate::install;
use crate::models::Library;
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::AppHandle;
use zip::ZipArchive;

/* ---------- install_profile.json 模型 ---------- */

#[derive(Deserialize)]
struct InstallProfile {
    #[serde(default)]
    version: String,
    #[serde(default)]
    minecraft: String,
    /// 安装器内嵌的完整版本清单在压缩包里的路径
    #[serde(default)]
    json: Option<String>,
    #[serde(default)]
    libraries: Vec<Library>,
    #[serde(default)]
    processors: Vec<Processor>,
    #[serde(default)]
    data: HashMap<String, DataFile>,
}

#[derive(Deserialize, Clone)]
struct DataFile {
    #[serde(default)]
    client: Option<String>,
    #[serde(default)]
    server: Option<String>,
}

#[derive(Deserialize, Clone)]
struct Processor {
    #[serde(default)]
    sides: Option<Vec<String>>,
    jar: String,
    #[serde(default)]
    classpath: Vec<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    outputs: HashMap<String, String>,
}

/* ---------- 入口 ---------- */

/// 安装器自带内嵌清单（version + minecraft + json）就走原生流程。
/// 1.13+ 的安装器带处理器，1.12 这类处理器为空、只需下载库 + 解内嵌 jar，
/// 交给官方安装器会卡在境外 maven 上（实测两分钟以上），一并接管。
pub fn supports_native(installer_path: &Path) -> bool {
    let Ok(bytes) = read_entry(installer_path, "install_profile.json") else {
        return false;
    };
    let Ok(profile) = serde_json::from_slice::<InstallProfile>(&bytes) else {
        return false;
    };
    !profile.version.is_empty() && !profile.minecraft.is_empty() && profile.json.is_some()
}

/// 安装器声明的版本目录名，用于回退官方安装器前清理半成品目录。
pub fn profile_version(installer_path: &Path) -> Option<String> {
    let bytes = read_entry(installer_path, "install_profile.json").ok()?;
    let profile: InstallProfile = serde_json::from_slice(&bytes).ok()?;
    if profile.version.is_empty() {
        None
    } else {
        Some(profile.version)
    }
}

/// 安装器包是否完整可用。缓存文件可能来自中断的下载，先校验再决定复不复用，
/// 否则会把坏包交给官方安装器，最终只看到退出码 1。
pub fn installer_usable(installer_path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(installer_path) else {
        return false;
    };
    let Ok(mut archive) = ZipArchive::new(file) else {
        return false;
    };
    let size = match archive.by_name("install_profile.json") {
        Ok(entry) => entry.size(),
        Err(_) => 0,
    };
    size > 64
}

pub async fn install(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    installer_path: &Path,
    loader: &str,
    game_version: &str,
    java_path: &str,
    progress: Arc<ProgressState>,
) -> Result<String> {
    let profile_bytes = read_entry(installer_path, "install_profile.json")?;
    let profile: InstallProfile =
        serde_json::from_slice(&profile_bytes).context("解析 install_profile.json 失败")?;
    if profile.version.is_empty() || profile.minecraft.is_empty() {
        bail!("install_profile.json 缺少版本信息");
    }
    if profile.minecraft != game_version {
        bail!(
            "安装器目标版本 {} 与当前游戏版本 {game_version} 不符",
            profile.minecraft
        );
    }

    let version_dir = cfg
        .game_root()
        .join("versions")
        .join(&profile.version);
    let result = install_inner(
        app,
        client,
        cfg,
        installer_path,
        loader,
        game_version,
        java_path,
        &profile,
        &version_dir,
        progress,
    )
    .await;
    if result.is_err() {
        // 只清加载器产物，版本隔离下 versions/<id> 还装着实例的 mods 与存档
        install::clear_loader_version_dir(&version_dir, &profile.version);
    }
    result
}

/* ---------- 安装流程 ---------- */

async fn install_inner(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    installer_path: &Path,
    loader: &str,
    game_version: &str,
    java_path: &str,
    profile: &InstallProfile,
    version_dir: &Path,
    progress: Arc<ProgressState>,
) -> Result<String> {
    let game_root = cfg.game_root();
    let libraries_dir = game_root.join("libraries");

    // 1) 官方安装器的第一步就是把内嵌清单写盘，照做——清单落地后，
    //    verify_loader_manifest 要查的核心库条目就完整了
    if version_dir.exists() {
        install::clear_loader_version_dir(version_dir, &profile.version);
    }
    std::fs::create_dir_all(version_dir)?;
    let json_entry = profile
        .json
        .clone()
        .unwrap_or_else(|| "/version.json".to_string());
    let manifest_bytes = read_entry(installer_path, &json_entry)
        .with_context(|| format!("读取安装器内嵌清单 {json_entry} 失败"))?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)
        .context("安装器内嵌清单解析失败")?;
    std::fs::write(
        version_dir.join(format!("{}.json", profile.version)),
        &manifest_bytes,
    )?;

    // 2) 依赖库 = 内嵌清单 + 处理器工具，全部交给启动器下载管线（带镜像）
    let mut libs: Vec<Library> = manifest
        .get("libraries")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default();
    libs.extend(profile.libraries.iter().cloned());

    let mut tasks: Vec<FileTask> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for lib in &libs {
        if !seen.insert(lib.name.clone()) {
            continue;
        }
        if install::library_is_native(lib) {
            continue;
        }
        let Some((dl, path)) = install::library_artifact(lib) else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        let dest = libraries_dir.join(path.replace('/', "\\"));
        // 已存在的库也要校验：0 字节或内容损坏的旧库会让处理器直接失败，
        // 而且官方安装器同样会跳过已存在文件，只报一个退出码 1
        if artifact_ok(&dest, &dl.sha1, dl.size) {
            continue;
        }
        if dest.exists() {
            let _ = std::fs::remove_file(&dest);
        }
        if dl.url.is_empty() {
            // 没有下载地址的库按官方行为从安装器内嵌文件解出
            if let Ok(bytes) = read_entry(installer_path, &format!("maven/{path}")) {
                if !dl.sha1.is_empty() && !sha1_hex(&bytes).eq_ignore_ascii_case(&dl.sha1) {
                    bail!("安装器内嵌库 {path} 校验失败，安装包可能已损坏");
                }
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&dest, &bytes)?;
            }
            continue;
        }
        tasks.push(FileTask {
            url: dl.url,
            dest,
            sha1: Some(dl.sha1),
            size: dl.size,
            label: lib.name.clone(),
        });
    }

    download::begin_stage(app, &progress, format!("正在下载 {loader} 依赖库…"));
    download::run_tasks(
        app,
        client,
        tasks,
        cfg.concurrency as usize,
        cfg.use_mirror(),
        progress.clone(),
    )
    .await?;

    // 3) 处理器数据表：[工件] 指向库目录，'字面量' 原样使用，其余从安装器解包
    let temp = std::env::temp_dir().join(format!("docl-forge-{}", profile.version));
    std::fs::create_dir_all(&temp)?;
    let vanilla_jar = game_root
        .join("versions")
        .join(&profile.minecraft)
        .join(format!("{}.jar", profile.minecraft));
    if !vanilla_jar.exists() {
        bail!("缺少原版客户端 {}，无法进行 {loader} 补丁", profile.minecraft);
    }

    let mut data: HashMap<String, String> = HashMap::new();
    for (key, entry) in &profile.data {
        let Some(value) = entry.client.as_deref().filter(|v| !v.is_empty()) else {
            continue;
        };
        let resolved = if value.starts_with('[') && value.ends_with(']') && value.len() >= 2 {
            let path = install::maven_path(&value[1..value.len() - 1])
                .ok_or_else(|| anyhow!("无法解析数据工件 {value}"))?;
            libraries_dir
                .join(path.replace('/', "\\"))
                .to_string_lossy()
                .to_string()
        } else if value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2 {
            value[1..value.len() - 1].to_string()
        } else {
            let name = value.trim_start_matches('/');
            let target = temp.join(name.replace('/', "\\"));
            let bytes = read_entry(installer_path, name)
                .with_context(|| format!("安装器内缺少数据文件 {name}"))?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&target, &bytes)?;
            target.to_string_lossy().to_string()
        };
        data.insert(key.clone(), resolved);
    }
    data.insert("SIDE".into(), "client".into());
    data.insert(
        "MINECRAFT_JAR".into(),
        vanilla_jar.to_string_lossy().to_string(),
    );
    data.insert("MINECRAFT_VERSION".into(), profile.minecraft.clone());
    data.insert("ROOT".into(), game_root.to_string_lossy().to_string());
    data.insert(
        "INSTALLER".into(),
        installer_path.to_string_lossy().to_string(),
    );
    data.insert(
        "LIBRARY_DIR".into(),
        libraries_dir.to_string_lossy().to_string(),
    );

    // 4) 按官方定义逐个运行处理器
    let processors: Vec<&Processor> = profile
        .processors
        .iter()
        .filter(|p| {
            p.sides
                .as_ref()
                .map_or(true, |sides| sides.iter().any(|side| side == "client"))
        })
        .collect();
    let total = processors.len() as u64;
    download::begin_stage(
        app,
        &progress,
        format!("正在运行 {loader} 处理器（共 {total} 步）…"),
    );
    progress.files_total.store(total, Ordering::Relaxed);
    progress.files_done.store(0, Ordering::Relaxed);
    progress.bytes_done.store(0, Ordering::Relaxed);
    progress.bytes_total.store(0, Ordering::Relaxed);
    download::emit_now(app, &progress);

    for (index, processor) in processors.iter().enumerate() {
        if progress.is_cancelled() {
            bail!("已取消 {loader} 安装");
        }
        progress.files_done.store(index as u64, Ordering::Relaxed);
        progress.set_message(format!(
            "正在运行 {loader} 处理器（{}/{total}）…",
            index + 1
        ));
        run_processor(
            cfg,
            java_path,
            &libraries_dir,
            &data,
            processor,
            &progress,
        )
        .await?;
    }
    let _ = std::fs::remove_dir_all(&temp);

    // Forge 1.17+ 的游戏本体是处理器产出的补丁版客户端，复制为版本 jar
    if let Some(patched) = data.get("PATCHED") {
        let source = PathBuf::from(patched);
        if source.exists() {
            std::fs::copy(&source, version_dir.join(format!("{}.jar", profile.version)))
                .with_context(|| format!("复制补丁客户端失败：{}", source.display()))?;
        }
    }

    // 5) 与官方安装器同级的完整性兜底
    install::verify_loader_manifest(cfg, &profile.version, loader, game_version)?;
    Ok(profile.version.clone())
}

/* ---------- 处理器执行 ---------- */

async fn run_processor(
    cfg: &Config,
    java_path: &str,
    libraries_dir: &Path,
    data: &HashMap<String, String>,
    processor: &Processor,
    progress: &Arc<ProgressState>,
) -> Result<()> {
    // 输出已存在且校验通过就直接跳过（官方同名缓存机制）
    let mut outputs: Vec<(PathBuf, String)> = Vec::new();
    for (key, sha_template) in &processor.outputs {
        let file = if key.starts_with('[') && key.ends_with(']') && key.len() >= 2 {
            let path = install::maven_path(&key[1..key.len() - 1])
                .ok_or_else(|| anyhow!("无法解析输出工件 {key}"))?;
            libraries_dir.join(path.replace('/', "\\"))
        } else {
            PathBuf::from(replace_tokens(data, key)?)
        };
        let sha1 = replace_tokens(data, sha_template)?;
        outputs.push((file, sha1));
    }
    if !outputs.is_empty()
        && outputs
            .iter()
            .all(|(file, sha1)| file_sha1(file).as_deref() == Some(sha1.as_str()))
    {
        return Ok(());
    }

    let jar_path = libraries_dir.join(
        install::maven_path(&processor.jar)
            .ok_or_else(|| anyhow!("无法解析处理器工件 {}", processor.jar))?
            .replace('/', "\\"),
    );
    if !jar_path.exists() {
        bail!("处理器依赖缺失：{}", processor.jar);
    }
    let main_class = jar_main_class(&jar_path)?;

    let mut classpath: Vec<String> = vec![jar_path.to_string_lossy().to_string()];
    for dep in &processor.classpath {
        let path = install::maven_path(dep)
            .ok_or_else(|| anyhow!("无法解析处理器依赖 {dep}"))?;
        let file = libraries_dir.join(path.replace('/', "\\"));
        if !file.exists() {
            bail!("处理器依赖缺失：{dep}");
        }
        classpath.push(file.to_string_lossy().to_string());
    }

    let mut args: Vec<String> = Vec::new();
    for arg in &processor.args {
        if arg.starts_with('[') && arg.ends_with(']') && arg.len() >= 2 {
            let path = install::maven_path(&arg[1..arg.len() - 1])
                .ok_or_else(|| anyhow!("无法解析参数工件 {arg}"))?;
            args.push(
                libraries_dir
                    .join(path.replace('/', "\\"))
                    .to_string_lossy()
                    .to_string(),
            );
        } else {
            args.push(replace_tokens(data, arg)?);
        }
    }

    let mut command = tokio::process::Command::new(java_path);
    // 个别处理器（如较新版本的 MOJMAP 下载）需要联网，代理配置与官方安装器一致
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
        .arg("-cp")
        .arg(classpath.join(";"))
        .arg(&main_class)
        .args(&args)
        .current_dir(cfg.game_root())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("启动处理器 {} 失败", processor.jar))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_task = tokio::spawn(async move {
        match stdout {
            Some(stream) => drain_to_string(stream).await,
            None => String::new(),
        }
    });
    let err_task = tokio::spawn(async move {
        match stderr {
            Some(stream) => drain_to_string(stream).await,
            None => String::new(),
        }
    });

    // 单个处理器几秒到几十秒，轮询等待以便响应取消
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if progress.is_cancelled() {
            let _ = child.kill().await;
            let _ = child.wait().await;
            bail!("已取消处理器 {}", processor.jar);
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill().await;
            let _ = child.wait().await;
            bail!("处理器 {} 运行超过 10 分钟，已停止", processor.jar);
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    };
    let stdout_text = out_task.await.unwrap_or_default();
    let stderr_text = err_task.await.unwrap_or_default();

    if !status.success() {
        let combined = format!("{stderr_text}{stdout_text}");
        let tail: String = {
            let chars: Vec<char> = combined.chars().collect();
            let start = chars.len().saturating_sub(600);
            chars[start..].iter().collect()
        };
        bail!(
            "处理器 {} 退出码 {}：{}",
            processor.jar,
            status.code().unwrap_or(-1),
            if tail.trim().is_empty() {
                "无输出".to_string()
            } else {
                tail
            }
        );
    }

    for (file, sha1) in &outputs {
        let actual = file_sha1(file)
            .ok_or_else(|| anyhow!("处理器 {} 未生成 {}", processor.jar, file.display()))?;
        if !actual.eq_ignore_ascii_case(sha1) {
            bail!(
                "处理器 {} 输出校验失败：{}（期望 {sha1}，实际 {actual}）",
                processor.jar,
                file.display()
            );
        }
    }
    Ok(())
}

/* ---------- 工具函数 ---------- */

async fn drain_to_string<R: tokio::io::AsyncRead + Unpin>(mut stream: R) -> String {
    use tokio::io::AsyncReadExt;
    let mut buffer = Vec::new();
    let _ = stream.read_to_end(&mut buffer).await;
    String::from_utf8_lossy(&buffer).to_string()
}

fn read_entry(zip_path: &Path, name: &str) -> Result<Vec<u8>> {
    let file =
        std::fs::File::open(zip_path).with_context(|| format!("无法打开 {}", zip_path.display()))?;
    let mut archive = ZipArchive::new(file).context("安装器压缩包损坏或不可读")?;
    let name = name.trim_start_matches('/');
    let mut entry = archive
        .by_name(name)
        .with_context(|| format!("安装器内缺少 {name}"))?;
    let mut buffer = Vec::new();
    entry.read_to_end(&mut buffer)?;
    Ok(buffer)
}

fn jar_main_class(jar_path: &Path) -> Result<String> {
    let file = std::fs::File::open(jar_path)?;
    let mut archive = ZipArchive::new(file)?;
    let mut text = String::new();
    if let Ok(mut entry) = archive.by_name("META-INF/MANIFEST.MF") {
        entry.read_to_string(&mut text)?;
    }
    // MANIFEST 的值可能折行（续行以空格开头），先拼回一行再找
    let mut unfolded = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(' ') {
            unfolded.push_str(rest);
        } else {
            if !unfolded.is_empty() {
                unfolded.push('\n');
            }
            unfolded.push_str(line);
        }
    }
    for line in unfolded.lines() {
        if line
            .get(..11)
            .map(|head| head.eq_ignore_ascii_case("Main-Class:"))
            .unwrap_or(false)
        {
            let value = line[11..].trim();
            if !value.is_empty() {
                return Ok(value.to_string());
            }
        }
    }
    bail!("{} 缺少 Main-Class", jar_path.display())
}

/// 官方 Util.replaceTokens 的移植：{KEY} 换成数据项，'字面量' 去引号，\ 转义
fn replace_tokens(data: &HashMap<String, String>, value: &str) -> Result<String> {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        let current = chars[index];
        if current == '\\' {
            index += 1;
            if index >= chars.len() {
                bail!("非法转义：{value}");
            }
            out.push(chars[index]);
        } else if current == '{' || current == '\'' {
            let mut key = String::new();
            let mut closed = false;
            let mut scan = index + 1;
            while scan < chars.len() {
                let c = chars[scan];
                if c == '\\' {
                    scan += 1;
                    if scan >= chars.len() {
                        bail!("非法转义：{value}");
                    }
                    key.push(chars[scan]);
                } else if (current == '{' && c == '}') || (current == '\'' && c == '\'') {
                    closed = true;
                    break;
                } else {
                    key.push(c);
                }
                scan += 1;
            }
            if !closed {
                bail!("未闭合的标记：{value}");
            }
            if current == '\'' {
                out.push_str(&key);
            } else {
                let token = data
                    .get(&key)
                    .ok_or_else(|| anyhow!("缺少数据项 {key}：{value}"))?;
                out.push_str(token);
            }
            index = scan;
        } else {
            out.push(current);
        }
        index += 1;
    }
    Ok(out)
}

/// 已存在的文件是否可信：空文件、大小不符、SHA1 不符都要重新下载。
fn artifact_ok(path: &Path, sha1: &str, size: u64) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if meta.len() == 0 {
        return false;
    }
    if size > 0 && meta.len() != size {
        return false;
    }
    if sha1.is_empty() {
        return true;
    }
    file_sha1(path)
        .map(|actual| actual.eq_ignore_ascii_case(sha1))
        .unwrap_or(false)
}

fn file_sha1(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha1::new();
    let mut buffer = [0u8; 65536];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Some(format!("{:x}", hasher.finalize()))
}

fn sha1_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::{artifact_ok, installer_usable, profile_version};
    use std::io::Write;

    fn write_zip(path: &std::path::Path, entries: &[(&str, &str)]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for (name, body) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn installer_usable_requires_valid_profile_entry() {
        let dir = std::env::temp_dir().join("docl-installer-usable-test");
        std::fs::create_dir_all(&dir).unwrap();
        let valid = dir.join("valid.jar");
        let _ = std::fs::remove_file(&valid);
        write_zip(
            &valid,
            &[(
                "install_profile.json",
                r#"{"version":"1.20.1-forge-47.4.28","minecraft":"1.20.1","json":"/version.json"}"#,
            )],
        );
        assert!(installer_usable(&valid));
        assert_eq!(
            profile_version(&valid).as_deref(),
            Some("1.20.1-forge-47.4.28")
        );

        let no_profile = dir.join("no-profile.jar");
        let _ = std::fs::remove_file(&no_profile);
        write_zip(&no_profile, &[("version.json", "{}")]);
        assert!(!installer_usable(&no_profile));
        assert!(profile_version(&no_profile).is_none());

        let corrupt = dir.join("corrupt.jar");
        std::fs::write(&corrupt, b"not a zip").unwrap();
        assert!(!installer_usable(&corrupt));
        assert!(profile_version(&corrupt).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn existing_artifact_requires_size_and_hash() {
        let dir = std::env::temp_dir().join("docl-forge-artifact-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lib.jar");
        let _ = std::fs::remove_file(&path);
        assert!(!artifact_ok(&path, "", 0));

        std::fs::write(&path, b"").unwrap();
        assert!(!artifact_ok(&path, "", 0));

        std::fs::write(&path, b"abc").unwrap();
        assert!(artifact_ok(&path, "", 3));
        assert!(!artifact_ok(&path, "", 5));
        assert!(!artifact_ok(
            &path,
            "0000000000000000000000000000000000000000",
            3
        ));
        assert!(artifact_ok(
            &path,
            "a9993e364706816aba3e25717850c26c9cd0d89d",
            3
        ));
        let _ = std::fs::remove_file(&path);
    }
}
