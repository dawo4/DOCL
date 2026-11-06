use crate::config::{self, Config};
use crate::download::{self, FileTask, ProgressState};
use crate::models::*;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
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
    /// JDK 或 JRE；JDK 自带 javac，Forge 安装器兼容性更好
    pub kind: String,
    /// 发行版：Eclipse Temurin、Azul Zulu、Amazon Corretto 等
    pub vendor: String,
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
        kind: detect_kind(path).to_string(),
        vendor: detect_vendor(&text),
    })
}

/// 有些 OpenJDK 发行版不会在 `java -version` 里写厂商，只能靠特征串识别；
/// 识别不出来时退回 OpenJDK / 未知，不影响启动。
fn detect_vendor(text: &str) -> String {
    let lower = text.to_lowercase();
    const KNOWN: [(&str, &str); 11] = [
        ("temurin", "Eclipse Temurin"),
        ("adoptopenjdk", "AdoptOpenJDK"),
        ("zulu", "Azul Zulu"),
        ("corretto", "Amazon Corretto"),
        ("microsoft", "Microsoft OpenJDK"),
        ("liberica", "BellSoft Liberica"),
        ("graalvm", "GraalVM"),
        ("semeru", "IBM Semeru"),
        ("sapmachine", "SAP SapMachine"),
        ("dragonwell", "Alibaba Dragonwell"),
        ("java(tm) se runtime", "Oracle JDK"),
    ];
    for (needle, name) in KNOWN {
        if lower.contains(needle) {
            return name.to_string();
        }
    }
    if lower.contains("openjdk") {
        "OpenJDK".into()
    } else {
        "未知".into()
    }
}

/// JDK 自带 javac；只有运行时的是 JRE。
fn detect_kind(exe: &Path) -> &'static str {
    match exe.parent() {
        Some(bin) if bin.join("javac.exe").exists() => "JDK",
        _ => "JRE",
    }
}

/// 用户可能选到 java.exe、javaw.exe、bin 目录或 JDK/JRE 根目录，这里统一成 java.exe。
pub fn resolve_java_path(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim().trim_matches('"');
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    if path.is_dir() {
        for candidate in [path.join("bin").join("java.exe"), path.join("java.exe")] {
            if candidate.exists() {
                return Some(candidate);
            }
        }
        return None;
    }
    if !path.exists() {
        return None;
    }
    // 选到 javaw.exe 时换成同目录的 java.exe，后者才能输出 -version
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case("javaw.exe"))
        .unwrap_or(false)
    {
        let sibling = path.with_file_name("java.exe");
        if sibling.exists() {
            return Some(sibling);
        }
    }
    Some(path)
}

/// 手动添加的 Java：校验能启动并读出版本，失败时给出可操作的提示。
pub fn inspect_manual(raw: &str) -> Result<JavaInfo> {
    let path = resolve_java_path(raw).ok_or_else(|| {
        anyhow!("找不到 Java：{raw}\n请选择 JDK/JRE 的安装目录，或其中的 bin\\java.exe")
    })?;
    inspect_java(&path, "手动指定")
        .ok_or_else(|| anyhow!("无法运行 {}，它可能不是有效的 Java 运行时", path.display()))
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
    use super::{detect_vendor, pick_for_major, JavaInfo};

    fn java(major: u32) -> JavaInfo {
        JavaInfo {
            path: format!("java{major}"),
            version: format!("{major}.0.0"),
            major,
            source: "test".into(),
            kind: "JDK".into(),
            vendor: "Eclipse Temurin".into(),
        }
    }

    #[test]
    fn vendor_from_release_text() {
        let temurin = "openjdk version \"17.0.8\"\nOpenJDK Runtime Environment Temurin-17.0.8+7 (build 17.0.8+7)";
        assert_eq!(detect_vendor(temurin), "Eclipse Temurin");
        let zulu = "openjdk version \"21.0.2\"\nOpenJDK Runtime Environment Zulu21.32+17-CA (build 21.0.2+13)";
        assert_eq!(detect_vendor(zulu), "Azul Zulu");
        let oracle = "java version \"1.8.0_202\"\nJava(TM) SE Runtime Environment (build 1.8.0_202-b08)";
        assert_eq!(detect_vendor(oracle), "Oracle JDK");
        let plain = "openjdk version \"17.0.8\"\nOpenJDK Runtime Environment (build 17.0.8+7)";
        assert_eq!(detect_vendor(plain), "OpenJDK");
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

/// 列出 Mojang 官方运行时目录里可下载的大版本，供下载下拉使用。
pub async fn available_runtime_majors(cfg: &Config) -> Result<Vec<u32>> {
    let client = crate::http::client();
    let all: RuntimeAll = download::get_json(&client, RUNTIME_ALL_URL, cfg.use_mirror()).await?;
    let windows = all
        .windows_x64
        .ok_or_else(|| anyhow!("官方未提供 Windows x64 Java 运行时"))?;
    let mut majors = Vec::new();
    for (component, entries) in &windows {
        // minecraft-java-exe 是启动器外壳，不是可直接运行游戏的 JRE
        if !(component.starts_with("jre-") || component.starts_with("java-runtime")) {
            continue;
        }
        if component.contains("snapshot") {
            continue;
        }
        for entry in entries {
            let major = parse_major(&entry.version.name);
            if major > 0 && !majors.contains(&major) {
                majors.push(major);
            }
        }
    }
    majors.sort_unstable();
    Ok(majors)
}

/// 第三方发行版固定提供这些大版本；Mojang 官方目录另行动态读取。
pub fn vendor_majors(source: &str) -> Vec<u32> {
    match source {
        "temurin" | "zulu" => vec![8, 11, 16, 17, 21, 25],
        "corretto" => vec![8, 11, 17, 21, 25],
        _ => Vec::new(),
    }
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

#[derive(Deserialize)]
struct TemurinAsset {
    binary: TemurinBinary,
}

#[derive(Deserialize)]
struct TemurinBinary {
    package: TemurinPackage,
}

#[derive(Deserialize)]
struct TemurinPackage {
    link: String,
    #[serde(default)]
    checksum: Option<String>,
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Deserialize)]
struct ZuluPackage {
    #[serde(default)]
    download_url: String,
}

struct RuntimeZip {
    url: String,
    size: u64,
    sha256: Option<String>,
    label: String,
    dir_name: String,
}

/// 按发行版安装：Mojang 走清单下载，Temurin / Zulu / Corretto 走各自官方 zip。
pub async fn install_runtime_vendor(
    app: &AppHandle,
    cfg: &Config,
    source: &str,
    major: u32,
    progress: Arc<ProgressState>,
) -> Result<JavaInfo> {
    match source {
        "temurin" => install_temurin(app, cfg, major, progress).await,
        "zulu" => install_zulu(app, cfg, major, progress).await,
        "corretto" => install_corretto(app, cfg, major, progress).await,
        _ => install_runtime(app, cfg, major, progress).await,
    }
}

async fn install_temurin(
    app: &AppHandle,
    cfg: &Config,
    major: u32,
    progress: Arc<ProgressState>,
) -> Result<JavaInfo> {
    let client = crate::http::client();
    progress.set_message(format!("正在查询 Eclipse Temurin Java {major}…"));
    let url = format!(
        "https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture=x64&image_type=jdk&os=windows&vendor=eclipse"
    );
    let assets: Vec<TemurinAsset> = download::get_json(&client, &url, cfg.use_mirror()).await?;
    let asset = assets
        .first()
        .ok_or_else(|| anyhow!("Eclipse Temurin 没有 Java {major} 的 Windows x64 JDK"))?;
    install_zip_runtime(
        app,
        cfg,
        progress,
        RuntimeZip {
            url: asset.binary.package.link.clone(),
            size: asset.binary.package.size.unwrap_or(0),
            sha256: asset.binary.package.checksum.clone().filter(|v| !v.is_empty()),
            label: format!("Temurin Java {major}"),
            dir_name: format!("temurin-jdk-{major}"),
        },
    )
    .await
}

async fn install_zulu(
    app: &AppHandle,
    cfg: &Config,
    major: u32,
    progress: Arc<ProgressState>,
) -> Result<JavaInfo> {
    let client = crate::http::client();
    progress.set_message(format!("正在查询 Azul Zulu Java {major}…"));
    let url = format!(
        "https://api.azul.com/metadata/v1/zulu/packages/?java_version={major}&os=windows&arch=x64&java_package_type=jdk&archive_type=zip&release_status=ga&availability_types=CA&latest=true"
    );
    let response = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .context("查询 Azul Zulu 下载地址失败")?;
    let packages: Vec<ZuluPackage> = response.json().await.context("解析 Azul Zulu 返回失败")?;
    let package = packages
        .iter()
        .find(|item| !item.download_url.is_empty())
        .ok_or_else(|| anyhow!("Azul Zulu 没有 Java {major} 的 Windows x64 JDK"))?;
    let size = remote_size(&client, &package.download_url).await;
    install_zip_runtime(
        app,
        cfg,
        progress,
        RuntimeZip {
            url: package.download_url.clone(),
            size,
            sha256: None,
            label: format!("Zulu Java {major}"),
            dir_name: format!("zulu-jdk-{major}"),
        },
    )
    .await
}

async fn install_corretto(
    app: &AppHandle,
    cfg: &Config,
    major: u32,
    progress: Arc<ProgressState>,
) -> Result<JavaInfo> {
    if !vendor_majors("corretto").contains(&major) {
        bail!("Amazon Corretto 没有 Java {major} 的 Windows x64 JDK");
    }
    let client = crate::http::client();
    progress.set_message(format!("正在查询 Amazon Corretto Java {major}…"));
    let url = format!(
        "https://corretto.aws/downloads/latest/amazon-corretto-{major}-x64-windows-jdk.zip"
    );
    let size = remote_size(&client, &url).await;
    install_zip_runtime(
        app,
        cfg,
        progress,
        RuntimeZip {
            url,
            size,
            sha256: None,
            label: format!("Corretto Java {major}"),
            dir_name: format!("corretto-jdk-{major}"),
        },
    )
    .await
}

async fn remote_size(client: &reqwest::Client, url: &str) -> u64 {
    client
        .head(url)
        .send()
        .await
        .ok()
        .and_then(|response| response.content_length())
        .unwrap_or(0)
}

async fn install_zip_runtime(
    app: &AppHandle,
    cfg: &Config,
    progress: Arc<ProgressState>,
    zip: RuntimeZip,
) -> Result<JavaInfo> {
    let client = crate::http::client();
    let root = cfg.java_runtime_root();
    tokio::fs::create_dir_all(&root).await?;
    let zip_path = root.join(format!(".{}.zip", zip.dir_name));
    let _ = tokio::fs::remove_file(&zip_path).await;

    progress.set_message(format!("正在下载 {}…", zip.label));
    let task = FileTask {
        url: zip.url,
        dest: zip_path.clone(),
        sha1: None,
        size: zip.size,
        label: zip.label.clone(),
    };
    download::run_tasks(
        app,
        &client,
        vec![task],
        cfg.concurrency as usize,
        cfg.use_mirror(),
        progress.clone(),
    )
    .await?;

    if let Some(expected) = zip.sha256 {
        let path = zip_path.clone();
        tokio::task::spawn_blocking(move || verify_sha256(&path, &expected))
            .await
            .map_err(|e| anyhow!("校验 Java 压缩包失败：{e}"))??;
    }

    progress.set_message(format!("正在解压 {}…", zip.label));
    let dest = root.join(&zip.dir_name);
    let archive = zip_path.clone();
    let target = dest.clone();
    tokio::task::spawn_blocking(move || extract_runtime_zip(&archive, &target))
        .await
        .map_err(|e| anyhow!("解压 Java 运行时失败：{e}"))??;
    let _ = tokio::fs::remove_file(&zip_path).await;

    let java_path = dest.join("bin").join("java.exe");
    let info = inspect_java(&java_path, "启动器运行时")
        .ok_or_else(|| anyhow!("安装完成，但无法运行 {}", java_path.display()))?;
    progress.set_message("Java 运行时安装完成");
    Ok(info)
}

fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    if format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected) {
        return Ok(());
    }
    let _ = std::fs::remove_file(path);
    bail!("Java 压缩包校验失败（SHA256 不匹配）")
}

fn extract_runtime_zip(zip_path: &Path, dest: &Path) -> Result<()> {
    let temp = dest.with_extension("extract");
    if temp.exists() {
        std::fs::remove_dir_all(&temp)?;
    }
    std::fs::create_dir_all(&temp)?;
    extract_zip_all(zip_path, &temp)?;

    let runtime_root = find_runtime_root(&temp)
        .ok_or_else(|| anyhow!("Java 压缩包里没有 bin\\java.exe"))?;
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }
    std::fs::rename(&runtime_root, dest)
        .with_context(|| format!("无法把 Java 运行时移动到 {}", dest.display()))?;
    let _ = std::fs::remove_dir_all(&temp);
    Ok(())
}

fn extract_zip_all(zip_path: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(zip_path)
        .with_context(|| format!("无法打开 Java 压缩包：{}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("Java 压缩包损坏或格式不支持")?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let target = dest.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&target)?;
        std::io::copy(&mut entry, &mut out)?;
    }
    Ok(())
}

/// 压缩包一般带一层版本目录（jdk-21.0.13+9 等），找到真正包含 bin 的那一层。
fn find_runtime_root(dir: &Path) -> Option<PathBuf> {
    if dir.join("bin").join("java.exe").exists() {
        return Some(dir.to_path_buf());
    }
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|entry| entry.path().join("bin").join("java.exe").exists())
        .map(|entry| entry.path())
}
