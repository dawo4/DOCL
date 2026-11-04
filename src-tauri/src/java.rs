use crate::config::{self, Config};
use crate::download::{self, FileTask, ProgressState};
use crate::models::*;
use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tauri::AppHandle;
use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};
use winreg::RegKey;

const RUNTIME_ALL_URL: &str =
    "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

#[derive(Serialize, Clone, Debug)]
pub struct JavaInfo {
    pub path: String,
    pub version: String,
    pub major: u32,
    pub source: String,
}

fn hidden_command(path: &Path) -> Command {
    let mut cmd = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd
}

pub fn inspect_java(path: &Path, source: &str) -> Option<JavaInfo> {
    if !path.exists() {
        return None;
    }
    let output = hidden_command(path).arg("-version").output().ok()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let version = text
        .split('"')
        .nth(1)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".into());
    let major = if version.starts_with("1.") {
        version
            .split('.')
            .nth(1)
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(8)
    } else {
        version
            .split('.')
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0)
    };
    if major == 0 {
        return None;
    }
    Some(JavaInfo {
        path: path.to_string_lossy().to_string(),
        version,
        major,
        source: source.to_string(),
    })
}

fn find_java_in_dir(dir: &Path, source: &str, out: &mut Vec<JavaInfo>) {
    let direct = dir.join("bin").join("java.exe");
    if let Some(info) = inspect_java(&direct, source) {
        out.push(info);
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let candidate = entry.path().join("bin").join("java.exe");
        if let Some(info) = inspect_java(&candidate, source) {
            out.push(info);
        }
    }
}

fn registry_java_roots() -> Vec<(PathBuf, String)> {
    let mut roots = Vec::new();
    let subkeys = [
        "SOFTWARE\\JavaSoft\\Java Runtime Environment",
        "SOFTWARE\\JavaSoft\\Java Development Kit",
        "SOFTWARE\\Eclipse Adoptium\\JDK",
        "SOFTWARE\\Microsoft\\JDK",
        "SOFTWARE\\Azul Systems\\Zulu",
        "SOFTWARE\\BellSoft\\LibericaJDK",
    ];
    for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        for sub in subkeys {
            if let Ok(key) = hklm.open_subkey_with_flags(sub, KEY_READ | view) {
                for name in key.enum_keys().flatten() {
                    if let Ok(child) = key.open_subkey_with_flags(&name, KEY_READ | view) {
                        if let Ok(home) = child.get_value::<String, _>("JavaHome") {
                            roots.push((PathBuf::from(home), format!("注册表 {name}")));
                        }
                    }
                }
            }
        }
    }
    roots
}

fn common_java_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(pf) = std::env::var("ProgramFiles") {
        let base = PathBuf::from(pf);
        dirs.push(base.join("Java"));
        dirs.push(base.join("Eclipse Adoptium"));
        dirs.push(base.join("Microsoft"));
        dirs.push(base.join("Zulu"));
        dirs.push(base.join("BellSoft"));
        dirs.push(base.join("Amazon Corretto"));
    }
    if let Ok(pfx86) = std::env::var("ProgramFiles(x86)") {
        let base = PathBuf::from(pfx86);
        dirs.push(base.join("Java"));
        dirs.push(base.join("Eclipse Adoptium"));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join("Programs").join("Eclipse Adoptium"));
    }
    if let Ok(home) = std::env::var("USERPROFILE") {
        dirs.push(PathBuf::from(home).join(".jdks"));
    }
    dirs
}

fn where_java() -> Vec<PathBuf> {
    let output = hidden_command(Path::new("cmd"))
        .arg("/C")
        .arg("where java")
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| PathBuf::from(line.trim()))
        .filter(|p| p.exists())
        .collect()
}

pub fn detect(cfg: &Config) -> Vec<JavaInfo> {
    let mut found: Vec<JavaInfo> = Vec::new();
    let mut push = |info: JavaInfo| {
        let normalized = info.path.to_lowercase();
        if !found.iter().any(|j| j.path.to_lowercase() == normalized) {
            found.push(info);
        }
    };

    if let Some(path) = &cfg.java_path {
        if let Some(info) = inspect_java(Path::new(path), "手动指定") {
            push(info);
        }
    }
    if let Ok(home) = std::env::var("JAVA_HOME") {
        if let Some(info) = inspect_java(&Path::new(&home).join("bin").join("java.exe"), "JAVA_HOME") {
            push(info);
        }
    }

    // 内置 runtime 与用户自定义目录都扫，改了安装位置也不会漏掉此前装好的运行时
    let mut runtime_roots = vec![config::runtime_dir()];
    let custom_root = cfg.java_runtime_root();
    if custom_root != runtime_roots[0] {
        runtime_roots.push(custom_root);
    }
    for runtime_root in runtime_roots {
        if let Ok(entries) = std::fs::read_dir(&runtime_root) {
            for entry in entries.flatten() {
                let candidate = entry.path().join("bin").join("java.exe");
                if let Some(info) = inspect_java(&candidate, "启动器运行时") {
                    push(info);
                }
            }
        }
    }

    for (root, source) in registry_java_roots() {
        if let Some(info) = inspect_java(&root.join("bin").join("java.exe"), &source) {
            push(info);
        }
    }

    for dir in common_java_dirs() {
        let mut temp = Vec::new();
        find_java_in_dir(&dir, "常见安装目录", &mut temp);
        for info in temp {
            push(info);
        }
    }

    for path in where_java() {
        if let Some(info) = inspect_java(&path, "PATH") {
            push(info);
        }
    }

    found.sort_by(|a, b| b.major.cmp(&a.major));
    found
}

#[cfg(test)]
mod tests {
    use super::{pick_for_major, JavaInfo};

    fn java(major: u32) -> JavaInfo {
        JavaInfo {
            path: format!("java{major}"),
            version: format!("{major}.0.0"),
            major,
            source: "test".into(),
        }
    }

    #[test]
    fn exact_major_wins_over_newer_runtime() {
        // 安装器优先精确大版本：有 Java 17 时不能拿 21 去跑 1.18 的处理器
        let list = vec![java(21), java(17), java(8)];
        assert_eq!(pick_for_major(&list, 17).map(|info| info.major), Some(17));
        assert_eq!(pick_for_major(&list, 8).map(|info| info.major), Some(8));
        // 没有精确版本时向上兼容，但 Java 8 时代不允许高版本顶替
        assert_eq!(pick_for_major(&list, 16).map(|info| info.major), Some(17));
        assert!(pick_for_major(&[java(21)], 8).is_none());
    }
}

pub fn pick_for_major(list: &[JavaInfo], required: u32) -> Option<JavaInfo> {
    if let Some(exact) = list.iter().find(|j| j.major == required) {
        return Some(exact.clone());
    }
    // Java 8 时代的版本（1.12 及更早）不能拿高版本 JDK 顶替，
    // Forge 安装器和游戏都会直接崩，宁可报缺失也不误选
    if required <= 8 {
        return None;
    }
    let mut newer: Vec<&JavaInfo> = list.iter().filter(|j| j.major > required).collect();
    newer.sort_by_key(|j| j.major);
    newer.first().map(|info| (*info).clone())
}

fn parse_major(version: &str) -> u32 {
    // Mojang 运行时的版本号有三种写法：21.0.7 / 1.8.0_51 / 8u51-cacert462b08
    let trimmed = version.trim();
    let digits: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
    let first = digits.parse::<u32>().unwrap_or(0);
    if first == 1 {
        // 旧式 1.8.0_xxx，主版本在第二段
        return trimmed
            .split('.')
            .nth(1)
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
    }
    first
}

pub async fn install_runtime(
    app: &AppHandle,
    cfg: &Config,
    major: u32,
    progress: Arc<ProgressState>,
) -> Result<JavaInfo> {
    let client = crate::http::client();
    progress.set_message(format!("正在查询 Java {major} 运行时…"));
    let all: RuntimeAll = download::get_json(&client, RUNTIME_ALL_URL, cfg.use_mirror()).await?;
    let windows = all
        .windows_x64
        .ok_or_else(|| anyhow!("官方未提供 Windows x64 Java 运行时"))?;

    let mut best: Option<(String, String)> = None;
    for (component, entries) in &windows {
        for entry in entries {
            if parse_major(&entry.version.name) != major {
                continue;
            }
            if best.is_none() {
                best = Some((component.clone(), entry.manifest.url.clone()));
            }
        }
    }
    let Some((component, manifest_url)) = best else {
        bail!("未找到适配 Java {major} 的官方运行时");
    };

    progress.set_message(format!("正在下载 Java {major} 运行时…"));
    let manifest: RuntimeManifest =
        download::get_json(&client, &manifest_url, cfg.use_mirror()).await?;
    let install_dir = cfg.java_runtime_root().join(&component);
    tokio::fs::create_dir_all(&install_dir).await?;

    let mut tasks: Vec<FileTask> = Vec::new();
    for (path, file) in &manifest.files {
        match file.kind.as_str() {
            "directory" => {
                let _ = tokio::fs::create_dir_all(install_dir.join(path.replace('/', "\\"))).await;
            }
            "file" => {
                if let Some(downloads) = &file.downloads {
                    tasks.push(FileTask {
                        url: downloads.raw.url.clone(),
                        dest: install_dir.join(path.replace('/', "\\")),
                        sha1: Some(downloads.raw.sha1.clone()),
                        size: downloads.raw.size,
                        label: path.clone(),
                    });
                }
            }
            _ => {}
        }
    }

    download::run_tasks(
        app,
        &client,
        tasks,
        cfg.concurrency as usize,
        cfg.use_mirror(),
        progress.clone(),
    )
    .await?;

    let java_path = install_dir.join("bin").join("java.exe");
    progress.set_message("Java 运行时安装完成");
    progress.active.store(false, std::sync::atomic::Ordering::Relaxed);
    download::emit_now(app, &progress);

    inspect_java(&java_path, "启动器运行时")
        .ok_or_else(|| anyhow!("运行时安装完成，但未能启动 java.exe"))
}
