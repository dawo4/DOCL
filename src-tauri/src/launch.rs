use crate::auth::Account;
use crate::config::Config;
use crate::install;
use crate::java;
use crate::lan::ServerTarget;
use crate::models::*;
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

#[derive(Serialize, Clone)]
pub struct LogLine {
    pub stream: String,
    pub line: String,
}

#[derive(Serialize, Clone)]
pub struct GameExit {
    pub code: Option<i32>,
    pub pid: u32,
}

fn arg_values(list: &[ArgValue]) -> Vec<String> {
    let mut result = Vec::new();
    for arg in list {
        match arg {
            ArgValue::Plain(s) => result.push(s.clone()),
            ArgValue::Conditional { rules, value } => {
                if install::rules_allow(Some(rules)) {
                    match value {
                        RawArgValue::One(s) => result.push(s.clone()),
                        RawArgValue::Many(v) => result.extend(v.iter().cloned()),
                    }
                }
            }
        }
    }
    result
}

fn replace(template: &str, vars: &std::collections::HashMap<&str, String>) -> String {
    let mut out = template.to_string();
    for (key, value) in vars {
        out = out.replace(&format!("${{{key}}}"), value);
    }
    out
}

/// 按空格拆参数，支持用双引号包住带空格的路径。
fn split_args(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for ch in raw.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => current.push(ch),
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            None => current.push(ch),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// 内存管理预设，返回需要追加的 GC 参数。
fn memory_args(mode: &str, java_major: u32) -> Vec<&'static str> {
    match mode {
        "g1gc" => vec![
            "-XX:+UseG1GC",
            "-XX:+UnlockExperimentalVMOptions",
            "-XX:G1NewSizePercent=20",
            "-XX:G1ReservePercent=20",
            "-XX:MaxGCPauseMillis=50",
            "-XX:G1HeapRegionSize=32M",
        ],
        "zgc" => {
            if java_major >= 21 {
                vec!["-XX:+UseZGC", "-XX:+ZGenerational"]
            } else if java_major >= 15 {
                vec!["-XX:+UseZGC"]
            } else {
                vec!["-XX:+UseG1GC"]
            }
        }
        "custom" => Vec::new(),
        _ => vec!["-XX:+UseG1GC"],
    }
}

/// 启动参数与本地服务端共用的内存分配值。
/// 自动模式在启动瞬间重新读取系统内存，随当时的可用量浮动（参考 PCL）。
pub fn effective_memory_mb(cfg: &Config) -> u32 {
    if cfg.memory_auto {
        let info = crate::system::memory_info();
        crate::system::recommended_memory_mb(info.total_mb, info.available_mb)
    } else {
        cfg.memory_mb.max(512)
    }
}

/// 1.20 起支持 --quickPlayMultiplayer；26.x 等新版本号同样走快速加入。
fn supports_quick_play(game_version: &str) -> bool {
    let mut parts = game_version.split(['.', '-', '+']);
    let Some(major) = parts.next().and_then(|value| value.parse::<u32>().ok()) else {
        return false;
    };
    if major >= 21 {
        return true;
    }
    if major != 1 {
        return false;
    }
    parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .map(|minor| minor >= 20)
        .unwrap_or(false)
}

pub fn find_java(cfg: &Config, required: u32) -> Result<java::JavaInfo> {
    if let Some(path) = &cfg.java_path {
        if let Some(info) = java::inspect_java(std::path::Path::new(path), "手动指定") {
            // 手动指定优先，但版本严重不匹配时（例如 1.12 配 Java 21）
            // 游戏和 Forge 安装器都会直接崩，这种情况继续往下找匹配版本。
            // Java 8 时代的版本只认 8，更高版本才允许向上兼容。
            if info.major >= required && (required > 8 || info.major == required) {
                return Ok(info);
            }
        }
    }
    let detected = java::detect(cfg);
    if let Some(info) = java::pick_for_major(&detected, required) {
        return Ok(info);
    }
    Err(anyhow!(
        "LAUNCH_NO_JAVA:当前版本需要 Java {required}，本机未找到可用运行时"
    ))
}

/// 安装加载器时优先用版本要求的精确 Java 大版本。
/// 旧版 Forge 的处理器被更高版本 JDK 运行（例如 1.18 + Java 21）可能直接退出 1。
pub fn find_java_exact(cfg: &Config, required: u32) -> Option<java::JavaInfo> {
    if let Some(path) = &cfg.java_path {
        let info = java::inspect_java(&PathBuf::from(path), "手动指定");
        if let Some(info) = info {
            if info.major == required {
                return Some(info);
            }
        }
    }
    java::detect(cfg)
        .into_iter()
        .find(|info| info.major == required)
}

pub fn build(
    cfg: &Config,
    merged: &install::MergedDetail,
    version_id: &str,
    account: &Account,
    server: Option<&ServerTarget>,
) -> Result<(PathBuf, Vec<String>, Vec<String>)> {
    let detail = &merged.detail;
    let game_root = cfg.game_root();
    let game_dir = crate::instance::game_dir(cfg, version_id);
    std::fs::create_dir_all(&game_dir)?;

    let natives_dir = install::natives_dir(cfg, version_id);
    if !natives_dir.exists() {
        // PCL 等外部启动器导入的实例没走过安装流程，这里按需补建并解压本地库
        install::ensure_natives(cfg, version_id, &merged.detail)?;
    }

    let required = detail.required_java();
    let java = find_java(cfg, required)?;

    // Forge 1.17+ 引导参数用 ${version_name}.jar 把游戏 jar 排除出模块路径。
    // 隔离实例的 version_name 和 Forge 版本 id 不同，游戏 jar 必须以实例名
    // 落在实例目录里，否则它会被当成自动模块，和 fmlloader 找到的
    // minecraft 模块导出同一批包直接崩（ResolutionException）。
    // 注意官方清单没有 kind 字段，不能拿 kind 判断是否需要复制
    let mut jar_id = merged.jar_id.clone();
    // FML 11（NeoForge 26+）只要在 classpath 上看到 Minecraft 类就判定为开发环境，
    // 要求补丁 jar 带 Minecraft-Dists；生产模式下它自己按 libraryDirectory 定位
    // 补丁客户端与加载器本体，所以版本 jar 不能进 classpath。
    let fml_startup = detail
        .main_class
        .as_deref()
        .map(|name| name.starts_with("net.neoforged.fml.startup"))
        .unwrap_or(false);
    if fml_startup {
        jar_id = None;
    }
    if let Some(source_id) = jar_id.as_deref() {
        if source_id != version_id {
            let source = game_root
                .join("versions")
                .join(source_id)
                .join(format!("{source_id}.jar"));
            let dest = game_root
                .join("versions")
                .join(version_id)
                .join(format!("{version_id}.jar"));
            if source.exists() && !dest.exists() {
                let parent = dest
                    .parent()
                    .ok_or_else(|| anyhow!("无效的版本路径：{}", dest.display()))?;
                std::fs::create_dir_all(parent)?;
                std::fs::copy(&source, &dest)
                    .with_context(|| format!("复制游戏本体失败：{}", dest.display()))?;
            }
            if dest.exists() {
                jar_id = Some(version_id.to_string());
            }
        }
    }

    let classpath_entries = install::merged_classpath_entries(cfg, detail, jar_id.as_deref());
    let classpath = classpath_entries
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(";");

    let mut vars: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    vars.insert("natives_directory", natives_dir.to_string_lossy().to_string());
    vars.insert("launcher_name", "DOCL".into());
    vars.insert("launcher_version", env!("CARGO_PKG_VERSION").into());
    vars.insert("classpath", classpath.clone());
    vars.insert("classpath_separator", ";".into());
    vars.insert("auth_player_name", account.name.clone());
    vars.insert("version_name", detail.id.clone());
    vars.insert(
        "game_directory",
        game_dir.to_string_lossy().to_string(),
    );
    vars.insert(
        "assets_root",
        game_root.join("assets").to_string_lossy().to_string(),
    );
    vars.insert(
        "assets_index_name",
        detail
            .asset_index
            .as_ref()
            .map(|a| a.id.clone())
            .or_else(|| detail.assets.clone())
            .unwrap_or_else(|| "legacy".into()),
    );
    vars.insert("auth_uuid", account.uuid.clone());
    vars.insert("auth_access_token", account.access_token.clone());
    vars.insert("auth_session", account.access_token.clone());
    vars.insert(
        "user_type",
        match account.kind.as_str() {
            "microsoft" => "msa".into(),
            "authlib" => "mojang".into(),
            _ => "legacy".into(),
        },
    );
    vars.insert(
        "version_type",
        if detail.kind.is_empty() {
            "release".into()
        } else {
            detail.kind.clone()
        },
    );
    vars.insert("user_properties", "{}".into());
    vars.insert("clientid", String::new());
    vars.insert("auth_xuid", String::new());
    vars.insert("resolution_width", cfg.window_width.to_string());
    vars.insert("resolution_height", cfg.window_height.to_string());
    vars.insert("library_directory", game_root.join("libraries").to_string_lossy().to_string());

    let mut jvm_args: Vec<String> = Vec::new();
    if let Some(args) = &detail.arguments {
        for value in arg_values(&args.jvm) {
            jvm_args.push(replace(&value, &vars));
        }
    }
    if let Some(logging) = detail.logging.as_ref().and_then(|l| l.client.as_ref()) {
        let file_name = logging
            .file
            .url
            .rsplit('/')
            .next()
            .unwrap_or("log4j.xml")
            .to_string();
        let log_path = game_root.join("assets").join("log_configs").join(file_name);
        if log_path.exists() {
            vars.insert("path", log_path.to_string_lossy().to_string());
            jvm_args.push(replace(&logging.argument, &vars));
        }
    }

    let memory_mb = effective_memory_mb(cfg);
    let mut combined_jvm: Vec<String> = vec![
        format!("-Xmx{memory_mb}M"),
        format!("-Xms{}M", (memory_mb / 2).clamp(256, 2048)),
    ];
    for arg in memory_args(&cfg.memory_mode, java.major) {
        combined_jvm.push(arg.to_string());
    }
    combined_jvm.push("-Dfile.encoding=UTF-8".into());
    combined_jvm.push("-Dstdout.encoding=UTF-8".into());
    combined_jvm.push("-Dstderr.encoding=UTF-8".into());
    if !cfg.disable_launch_wrapper {
        combined_jvm.push("-Dminecraft.launcher.brand=DOCL".into());
        combined_jvm.push(format!(
            "-Dminecraft.launcher.version={}",
            env!("CARGO_PKG_VERSION")
        ));
    }
    // Java 24 起 sun.misc.Unsafe 内存访问默认告警/拒绝，LWJGL 需要显式放行
    if !cfg.disable_lwjgl_unsafe && java.major >= 24 {
        combined_jvm.push("--sun-misc-unsafe-memory-access=allow".into());
    }
    // 第三方皮肤站（LittleSkin 等）：把认证端点指向该站点
    if let (Some(root), Some(jar)) = (account.authlib_url.as_deref(), crate::authlib::injector_path()) {
        combined_jvm.push(format!("-javaagent:{}={root}", jar.display()));
        combined_jvm.push("-Dauthlibinjector.side=client".into());
        combined_jvm.push("-Dauthlibinjector.noShowServerName".into());
    }
    combined_jvm.extend(jvm_args);
    for custom in split_args(&cfg.java_args) {
        combined_jvm.push(custom);
    }
    if !combined_jvm.iter().any(|a| a.starts_with("-Djava.library.path")) {
        combined_jvm.push(format!(
            "-Djava.library.path={}",
            natives_dir.to_string_lossy()
        ));
    }
    if !combined_jvm.iter().any(|a| a == "-cp" || a == "-classpath") {
        combined_jvm.push("-cp".into());
        combined_jvm.push(classpath);
    }

    let mut game_args: Vec<String> = Vec::new();
    if let Some(args) = &detail.arguments {
        for value in arg_values(&args.game) {
            game_args.push(replace(&value, &vars));
        }
    } else if let Some(legacy) = &detail.minecraft_arguments {
        for value in legacy.split_whitespace() {
            game_args.push(replace(value, &vars));
        }
    }

    let mut command_args = combined_jvm;
    let main_class = detail
        .main_class
        .clone()
        .unwrap_or_else(|| "net.minecraft.client.main.Main".into());
    command_args.push(main_class);
    for extra in split_args(&cfg.game_args) {
        game_args.push(extra);
    }
    if cfg.fullscreen && !game_args.iter().any(|arg| arg == "--fullscreen") {
        game_args.push("--fullscreen".into());
    }
    if let Some(target) = server {
        let game_version = merged
            .chain
            .last()
            .cloned()
            .unwrap_or_else(|| version_id.to_string());
        if supports_quick_play(&game_version) {
            game_args.push("--quickPlayMultiplayer".into());
            game_args.push(target.address());
        } else {
            game_args.push("--server".into());
            game_args.push(target.host.clone());
            game_args.push("--port".into());
            game_args.push(target.port.to_string());
        }
    }
    command_args.extend(game_args);

    Ok((PathBuf::from(java.path), command_args, Vec::new()))
}

/// 让 Windows 为这个 java.exe 选择独立/高性能显卡。
#[cfg(windows)]
fn prefer_high_performance_gpu(java_path: &std::path::Path, app: &AppHandle) {
    use std::os::windows::process::CommandExt;
    let mut cmd = Command::new("reg");
    cmd.args([
        "add",
        r"HKCU\Software\Microsoft\DirectX\UserGpuPreferences",
        "/v",
        &java_path.to_string_lossy(),
        "/t",
        "REG_SZ",
        "/d",
        "GpuPreference=2;",
        "/f",
    ])
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .creation_flags(0x08000000);
    match cmd.status() {
        Ok(status) if status.success() => {
            let _ = app.emit(
                "game-log",
                LogLine {
                    stream: "system".into(),
                    line: "已为当前 Java 写入高性能显卡偏好".into(),
                },
            );
        }
        _ => {
            let _ = app.emit(
                "game-log",
                LogLine {
                    stream: "system".into(),
                    line: "写入高性能显卡偏好失败，可在显卡控制面板手动设置".into(),
                },
            );
        }
    }
}

#[cfg(not(windows))]
fn prefer_high_performance_gpu(_java_path: &std::path::Path, _app: &AppHandle) {}

/// 按配置在启动游戏前执行命令，工作目录为游戏目录。
fn run_pre_launch(app: &AppHandle, cfg: &Config, game_dir: &std::path::Path) {
    let command = cfg.pre_launch.trim();
    if command.is_empty() {
        return;
    }
    let _ = app.emit(
        "game-log",
        LogLine {
            stream: "system".into(),
            line: format!("执行启动前命令：{command}"),
        },
    );
    let mut cmd = Command::new("cmd");
    cmd.args(["/C", command])
        .current_dir(game_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let output = match cmd.output() {
        Ok(output) => output,
        Err(err) => {
            let _ = app.emit(
                "game-log",
                LogLine {
                    stream: "system".into(),
                    line: format!("启动前命令执行失败：{err}"),
                },
            );
            return;
        }
    };
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let _ = app.emit(
            "game-log",
            LogLine {
                stream: "stdout".into(),
                line: line.to_string(),
            },
        );
    }
    for line in String::from_utf8_lossy(&output.stderr).lines() {
        let _ = app.emit(
            "game-log",
            LogLine {
                stream: "stderr".into(),
                line: line.to_string(),
            },
        );
    }
    if !output.status.success() {
        let _ = app.emit(
            "game-log",
            LogLine {
                stream: "system".into(),
                line: format!("启动前命令返回非零退出码：{:?}", output.status.code()),
            },
        );
    }
}

pub fn spawn(
    app: &AppHandle,
    cfg: &Config,
    version_id: &str,
    account: &Account,
    server: Option<&ServerTarget>,
) -> Result<u32> {
    let merged = install::resolve_detail(cfg, version_id)?;
    let game_dir = crate::instance::game_dir(cfg, version_id);
    let (java_path, args, _) = build(cfg, &merged, version_id, account, server)?;

    let _ = app.emit(
        "game-log",
        LogLine {
            stream: "system".into(),
            line: format!("使用 Java：{}", java_path.display()),
        },
    );
    let _ = app.emit(
        "game-log",
        LogLine {
            stream: "system".into(),
            line: format!(
                "启动参数：{} 个，游戏目录：{}",
                args.len(),
                game_dir.display()
            ),
        },
    );

    let mut cmd = Command::new(&java_path);
    cmd.current_dir(&game_dir)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    if cfg.prefer_gpu {
        prefer_high_performance_gpu(&java_path, app);
    }
    run_pre_launch(app, cfg, &game_dir);

    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let stats_version = version_id.to_string();
    let mut child = cmd.spawn()?;
    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // 把游戏输出同时落盘：进程被外部终止时也能留下现场
    let log_path = crate::config::data_dir().join("logs").join("game.log");
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let logger = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
        .ok()
        .map(|file| Arc::new(Mutex::new(BufWriter::new(file))));
    if let Some(logger) = &logger {
        if let Ok(mut writer) = logger.lock() {
            let _ = writeln!(
                writer,
                "=== DOCL 启动 {version_id} · Java {} · pid {pid} ===",
                java_path.display()
            );
            let _ = writer.flush();
        }
    }

    if let Some(stdout) = stdout {
        let app = app.clone();
        let logger = logger.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(logger) = &logger {
                    if let Ok(mut writer) = logger.lock() {
                        let _ = writeln!(writer, "{line}");
                        let _ = writer.flush();
                    }
                }
                let _ = app.emit(
                    "game-log",
                    LogLine {
                        stream: "stdout".into(),
                        line,
                    },
                );
            }
        });
    }
    if let Some(stderr) = stderr {
        let app = app.clone();
        let logger = logger.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Some(logger) = &logger {
                    if let Ok(mut writer) = logger.lock() {
                        let _ = writeln!(writer, "[stderr] {line}");
                        let _ = writer.flush();
                    }
                }
                let _ = app.emit(
                    "game-log",
                    LogLine {
                        stream: "stderr".into(),
                        line,
                    },
                );
            }
        });
    }

    let app = app.clone();
    std::thread::spawn(move || {
        let status = child.wait();
        let code = status.as_ref().ok().and_then(|s| s.code());
        if let Some(logger) = &logger {
            if let Ok(mut writer) = logger.lock() {
                let _ = writeln!(writer, "=== 游戏退出：代码 {code:?}（pid {pid}）===");
                let _ = writer.flush();
            }
        }
        // 先记录本次游玩时长，再通知前端刷新，否则版本页会读到旧数据
        let ended = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(started_at);
        crate::stats::record(&stats_version, started_at, ended.saturating_sub(started_at));
        let _ = app.emit("game-exit", GameExit { code, pid });
    });

    Ok(pid)
}

pub fn kill(pid: u32) -> Result<()> {
    let mut cmd = Command::new("taskkill");
    cmd.args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd.status()?;
    Ok(())
}
