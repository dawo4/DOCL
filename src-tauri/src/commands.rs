use crate::auth::{self, Account, AccountView, DeviceCode};
use crate::authlib;
use crate::mcmod;
use crate::modmeta;
use crate::translate;
use crate::appupdate;
use crate::config::{self, Config};
use crate::crash;
use crate::curseforge;
use crate::download::{self, ProgressState};
use crate::hall;
use crate::install::{self, InstalledVersion, LoaderOption};
use crate::instance::{
    self, InstanceFile, InstanceInfo, InstanceMeta, InstancePaths, WorldSeed, WorldSummary,
};
use crate::java::{self, JavaInfo};
use crate::lan;
use crate::launch;
use crate::models::*;
use crate::moddeps;
use crate::modpack;
use crate::news;
use crate::pclimport;
use crate::ping::{self, ServerPingResult};
use crate::repair;
use crate::server;
use crate::skins;
use crate::system;
use crate::terracotta;
use crate::update;
use crate::vibrancy;
use crate::AppState;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, State};

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub fn open_folder(path: String) -> CmdResult<()> {
    let target = std::path::PathBuf::from(path.trim());
    if !target.exists() {
        return Err(format!("目录不存在：{}", target.display()));
    }
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(&target)
            .spawn()
            .map_err(err)?;
        return Ok(());
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open")
            .arg(&target)
            .spawn()
            .map_err(err)?;
        Ok(())
    }
}

#[tauri::command]
pub fn reveal_file(path: String) -> CmdResult<()> {
    let target = std::path::PathBuf::from(path.trim());
    if !target.exists() {
        return Err(format!("文件不存在：{}", target.display()));
    }
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg("/select,")
            .arg(&target)
            .spawn()
            .map_err(err)?;
        return Ok(());
    }
    #[cfg(not(windows))]
    {
        if let Some(parent) = target.parent() {
            std::process::Command::new("xdg-open")
                .arg(parent)
                .spawn()
                .map_err(err)?;
        }
        Ok(())
    }
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
pub fn system_memory() -> system::MemoryInfo {
    system::memory_info()
}

#[tauri::command]
pub async fn news_feed(
    state: State<'_, AppState>,
    force: Option<bool>,
) -> CmdResult<news::NewsFeed> {
    Ok(news::feed(&state.client, force.unwrap_or(false)).await)
}

#[tauri::command]
pub async fn news_article(
    state: State<'_, AppState>,
    path: String,
    wiki_title: Option<String>,
) -> CmdResult<String> {
    news::article(&state.client, &path, wiki_title.as_deref())
        .await
        .map_err(err)
}

#[tauri::command]
pub fn save_config(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    config: Config,
) -> CmdResult<Config> {
    let mut config = config;
    config.sync_game_dirs();
    // 限速设置立即生效（字节/秒；0 = 不限）
    download::set_speed_limit(config.speed_limit as u64 * 1024 * 1024);
    download::set_force_mirror(config.download_source == "mirror");
    // 毛玻璃开关变化时实时应用；系统不支持则回滚为关闭并照常保存，
    // 返回回滚后的配置让前端把开关弹回去
    if config.window_vibrancy != state.config.lock().unwrap().window_vibrancy {
        if vibrancy::apply(&window, config.window_vibrancy).is_err() {
            config.window_vibrancy = false;
        }
    }
    config::save(&config).map_err(err)?;
    std::fs::create_dir_all(&config.game_dir).map_err(err)?;
    *state.config.lock().unwrap() = config.clone();
    Ok(config)
}

/// 把当前设置导出成可读 JSON，方便迁移到另一台电脑。
#[tauri::command]
pub fn export_settings(state: State<'_, AppState>, path: String) -> CmdResult<()> {
    let config = state.config.lock().unwrap().clone();
    let text = serde_json::to_string_pretty(&config).map_err(err)?;
    std::fs::write(path.trim(), text).map_err(err)?;
    Ok(())
}

/// 导入设置并立即生效；账号、皮肤等数据不在配置内，不会被覆盖。
#[tauri::command]
pub fn import_settings(state: State<'_, AppState>, path: String) -> CmdResult<Config> {
    let bytes = std::fs::read(path.trim()).map_err(err)?;
    let mut config: Config = serde_json::from_slice(&bytes)
        .map_err(|e| format!("配置文件格式不正确：{e}"))?;
    config.sync_game_dirs();
    config::save(&config).map_err(err)?;
    *state.config.lock().unwrap() = config.clone();
    Ok(config)
}

#[tauri::command]
pub fn app_version() -> String {
    appupdate::CURRENT_VERSION.to_string()
}

/// 便携版会把配置、账号、实例索引放在 exe 同目录的 data 下。
#[tauri::command]
pub fn app_portable() -> bool {
    config::is_portable()
}

/// 读取 PCL 目录里的设置并返回预览，不写入任何内容。
#[tauri::command]
pub fn pcl_import_preview(path: String) -> CmdResult<pclimport::PclPreview> {
    pclimport::inspect(&path).map_err(err)
}

/// 应用 PCL 设置；只覆盖确实读到的字段。
#[tauri::command]
pub fn pcl_import_apply(state: State<'_, AppState>, path: String) -> CmdResult<Config> {
    let mut next = state.config.lock().unwrap().clone();
    pclimport::apply(&mut next, &path).map_err(err)?;
    config::save(&next).map_err(err)?;
    *state.config.lock().unwrap() = next.clone();
    Ok(next)
}

#[tauri::command]
pub async fn update_check(url: String) -> CmdResult<appupdate::UpdateInfo> {
    appupdate::check(&url).await.map_err(err)
}

#[tauri::command]
pub async fn update_download(
    app: AppHandle,
    url: String,
    version: String,
) -> CmdResult<String> {
    appupdate::download(&app, &url, &version)
        .await
        .map(|path| path.to_string_lossy().to_string())
        .map_err(err)
}

#[tauri::command]
pub fn update_install(app: AppHandle, path: String) -> CmdResult<()> {
    appupdate::install(&app, &path).map_err(err)
}

/// 便携版原地更新：解压、替换自身、重启。
#[tauri::command]
pub fn portable_update_install(app: AppHandle, path: String) -> CmdResult<()> {
    appupdate::portable_install(&app, &path).map_err(err)
}

#[derive(serde::Serialize)]
pub struct GameDirsView {
    pub dirs: Vec<config::GameDirEntry>,
    pub active: String,
    pub official: String,
}

#[tauri::command]
pub fn game_dirs(state: State<'_, AppState>) -> GameDirsView {
    let cfg = state.config.lock().unwrap().clone();
    GameDirsView {
        dirs: cfg.game_dirs.clone(),
        active: cfg.game_dir.clone(),
        official: config::default_game_dir().to_string_lossy().to_string(),
    }
}

/// 每个游戏目录下的版本数量；只数文件存在与否，不解析清单。
#[tauri::command]
pub fn game_dir_version_counts(
    state: State<'_, AppState>,
) -> std::collections::HashMap<String, u32> {
    let cfg = state.config.lock().unwrap().clone();
    let mut result = std::collections::HashMap::new();
    for entry in &cfg.game_dirs {
        let versions = std::path::Path::new(&entry.path).join("versions");
        let mut count = 0u32;
        if let Ok(items) = std::fs::read_dir(&versions) {
            for item in items.flatten() {
                if !item.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                let id = item.file_name().to_string_lossy().to_string();
                if item.path().join(format!("{id}.json")).exists() {
                    count += 1;
                }
            }
        }
        result.insert(entry.path.clone(), count);
    }
    result
}

/// 添加或切换目录；path 会先创建再写入配置。
#[tauri::command]
pub fn add_game_dir(state: State<'_, AppState>, path: String) -> CmdResult<Config> {
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err("目录不能为空".into());
    }
    let mut cfg = state.config.lock().unwrap().clone();
    std::fs::create_dir_all(&path).map_err(err)?;
    cfg.set_active_dir(&path);
    config::save(&cfg).map_err(err)?;
    *state.config.lock().unwrap() = cfg.clone();
    Ok(cfg)
}

#[tauri::command]
pub fn switch_game_dir(state: State<'_, AppState>, path: String) -> CmdResult<Config> {
    let mut cfg = state.config.lock().unwrap().clone();
    let exists = cfg
        .game_dirs
        .iter()
        .any(|entry| entry.path.eq_ignore_ascii_case(&path));
    if !exists {
        return Err("目录不在列表中".into());
    }
    cfg.set_active_dir(&path);
    config::save(&cfg).map_err(err)?;
    *state.config.lock().unwrap() = cfg.clone();
    Ok(cfg)
}

#[tauri::command]
pub fn remove_game_dir(state: State<'_, AppState>, path: String) -> CmdResult<Config> {
    let mut cfg = state.config.lock().unwrap().clone();
    cfg.game_dirs
        .retain(|entry| !entry.path.eq_ignore_ascii_case(&path));
    if cfg.game_dirs.is_empty() {
        return Err("至少保留一个游戏目录".into());
    }
    if cfg.game_dir.eq_ignore_ascii_case(&path) {
        let next = cfg.game_dirs[0].path.clone();
        cfg.set_active_dir(&next);
    }
    cfg.sync_game_dirs();
    config::save(&cfg).map_err(err)?;
    *state.config.lock().unwrap() = cfg.clone();
    Ok(cfg)
}

#[tauri::command]
pub async fn fetch_versions(
    state: State<'_, AppState>,
    force: bool,
) -> CmdResult<Vec<VersionSummary>> {
    let cfg = state.config.lock().unwrap().clone();
    let manifest = if !force {
        state.manifest.lock().unwrap().clone()
    } else {
        None
    };
    let manifest = match manifest {
        Some(m) => m,
        None => {
            let m = install::fetch_manifest(&state.client, cfg.use_mirror())
                .await
                .map_err(err)?;
            *state.manifest.lock().unwrap() = Some(m.clone());
            m
        }
    };
    let installed: std::collections::HashSet<String> =
        install::list_installed(&cfg).into_iter().map(|v| v.id).collect();
    Ok(manifest
        .versions
        .into_iter()
        .map(|v| VersionSummary {
            installed: installed.contains(&v.id),
            id: v.id,
            kind: v.kind,
            release_time: v.release_time,
        })
        .collect())
}

#[tauri::command]
pub fn installed_versions(state: State<'_, AppState>) -> Vec<InstalledVersion> {
    let cfg = state.config.lock().unwrap().clone();
    install::list_installed(&cfg)
}

#[tauri::command]
pub async fn install_version(
    app: AppHandle,
    state: State<'_, AppState>,
    version_id: String,
) -> CmdResult<InstalledVersion> {
    let cfg = state.config.lock().unwrap().clone();
    {
        let guard = state.progress.lock().unwrap();
        if let Some(p) = guard.as_ref() {
            if p.active.load(Ordering::Relaxed) {
                return Err("已有下载任务正在进行".into());
            }
        }
    }
    let progress = ProgressState::new("install");
    progress.set_message(format!("准备安装 {version_id}…"));
    *state.progress.lock().unwrap() = Some(progress.clone());
    download::reset_failed_batch();

    let result = install::install_version(&app, &state.client, &cfg, &version_id, progress.clone())
        .await;

    progress.active.store(false, Ordering::Relaxed);
    download::emit_now(&app, &progress);
    *state.progress.lock().unwrap() = None;

    match result {
        Ok(version) => {
            *state.config.lock().unwrap() = {
                let mut c = cfg;
                c.selected_version = Some(version_id);
                let _ = config::save(&c);
                c
            };
            Ok(version)
        }
        Err(e) => Err(err(e)),
    }
}

#[tauri::command]
pub async fn loader_options(
    state: State<'_, AppState>,
    version_id: String,
) -> CmdResult<Vec<LoaderOption>> {
    let cfg = state.config.lock().unwrap().clone();
    Ok(install::loader_options(&state.client, &cfg, &version_id).await)
}

#[tauri::command]
pub async fn loader_versions(
    state: State<'_, AppState>,
    version_id: String,
    loader: String,
) -> CmdResult<Vec<install::LoaderVersion>> {
    let cfg = state.config.lock().unwrap().clone();
    install::loader_versions(&state.client, &cfg, &version_id, &loader)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn install_loader(
    app: AppHandle,
    state: State<'_, AppState>,
    version_id: String,
    loader: String,
    loader_version: Option<String>,
) -> CmdResult<InstalledVersion> {
    let cfg = state.config.lock().unwrap().clone();
    {
        let guard = state.progress.lock().unwrap();
        if let Some(p) = guard.as_ref() {
            if p.active.load(Ordering::Relaxed) {
                return Err("已有下载任务正在进行".into());
            }
        }
    }
    let progress = ProgressState::new("install");
    progress.set_message(format!("准备安装 {loader}…"));
    *state.progress.lock().unwrap() = Some(progress.clone());
    download::reset_failed_batch();

    let result = install::install_loader(
        &app,
        &state.client,
        &cfg,
        &version_id,
        &loader,
        loader_version.as_deref(),
        progress.clone(),
    )
    .await;

    progress.active.store(false, Ordering::Relaxed);
    download::emit_now(&app, &progress);
    *state.progress.lock().unwrap() = None;

    match result {
        Ok(version) => {
            let mut current = cfg;
            current.selected_version = Some(version.id.clone());
            let _ = config::save(&current);
            *state.config.lock().unwrap() = current;
            Ok(version)
        }
        Err(e) => Err(err(e)),
    }
}

#[tauri::command]
pub async fn reinstall_loader(
    app: AppHandle,
    state: State<'_, AppState>,
    version_id: String,
) -> CmdResult<InstalledVersion> {
    let cfg = state.config.lock().unwrap().clone();
    {
        let guard = state.progress.lock().unwrap();
        if let Some(p) = guard.as_ref() {
            if p.active.load(Ordering::Relaxed) {
                return Err("已有下载任务正在进行".into());
            }
        }
    }
    let progress = ProgressState::new("install");
    progress.set_message("准备重装加载器…");
    *state.progress.lock().unwrap() = Some(progress.clone());
    download::reset_failed_batch();

    let result =
        install::reinstall_loader(&app, &state.client, &cfg, &version_id, progress.clone()).await;

    progress.active.store(false, Ordering::Relaxed);
    download::emit_now(&app, &progress);
    *state.progress.lock().unwrap() = None;

    match result {
        Ok(version) => {
            let mut current = cfg;
            current.selected_version = Some(version.id.clone());
            let _ = config::save(&current);
            *state.config.lock().unwrap() = current;
            Ok(version)
        }
        Err(e) => Err(err(e)),
    }
}

#[tauri::command]
pub fn cancel_install(state: State<'_, AppState>) {
    if let Some(p) = state.progress.lock().unwrap().as_ref() {
        p.cancel.store(true, Ordering::Relaxed);
        p.active.store(false, Ordering::Relaxed);
    }
}

fn remove_version_files(cfg: &Config, version_id: &str) {
    let dir = install::resolve_version_dir(cfg, version_id);
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    let _ = instance::remove(cfg, version_id);
}

fn collect_version_ids(cfg: &Config) -> Vec<String> {
    let dir = cfg.game_root().join("versions");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|id| dir.join(id).join(format!("{id}.json")).exists())
        .collect()
}

#[tauri::command]
pub fn delete_version(state: State<'_, AppState>, version_id: String) -> CmdResult<Vec<String>> {
    let cfg = state.config.lock().unwrap().clone();

    let was_modpack = instance::get(&cfg, &version_id).category == "modpack";
    let chain: Vec<String> = if was_modpack {
        install::resolve_detail(&cfg, &version_id)
            .map(|merged| merged.chain)
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    remove_version_files(&cfg, &version_id);
    let mut removed = vec![version_id.clone()];

    // PCL 式体验：删整合包时把继承的原版 / 加载器一起带走，留一个孤儿都会占磁盘
    for parent in &chain {
        if parent == &version_id || removed.contains(parent) {
            continue;
        }
        if !install::is_referenced(&cfg, parent, &removed) {
            remove_version_files(&cfg, parent);
            removed.push(parent.clone());
        }
    }
    Ok(removed)
}

#[tauri::command]
pub fn instance_info(
    state: State<'_, AppState>,
    version_id: String,
) -> CmdResult<InstanceInfo> {
    let cfg = state.config.lock().unwrap().clone();
    instance::info(&cfg, &version_id).map_err(err)
}

#[tauri::command]
pub fn instance_metas(
    state: State<'_, AppState>,
) -> std::collections::HashMap<String, InstanceMeta> {
    let cfg = state.config.lock().unwrap().clone();
    instance::all(&cfg)
}

#[tauri::command]
pub fn instance_save_meta(
    state: State<'_, AppState>,
    version_id: String,
    meta: InstanceMeta,
) -> CmdResult<InstanceInfo> {
    let cfg = state.config.lock().unwrap().clone();
    instance::save(&cfg, &version_id, &meta).map_err(err)?;
    instance::info(&cfg, &version_id).map_err(err)
}

#[tauri::command]
pub fn instance_set_icon(
    state: State<'_, AppState>,
    version_id: String,
    icon: String,
) -> CmdResult<()> {
    let cfg = state.config.lock().unwrap().clone();
    instance::set_icon(&cfg, &version_id, &icon).map_err(err)
}

#[tauri::command]
pub fn instance_set_icon_image(
    state: State<'_, AppState>,
    version_id: String,
    source: String,
) -> CmdResult<String> {
    let cfg = state.config.lock().unwrap().clone();
    instance::set_icon_image(&cfg, &version_id, &source).map_err(err)
}

#[tauri::command]
pub fn instance_icon_image(
    state: State<'_, AppState>,
    version_id: String,
) -> Option<String> {
    let cfg = state.config.lock().unwrap().clone();
    instance::icon_image(&cfg, &version_id)
}

#[tauri::command]
pub fn instance_files(
    state: State<'_, AppState>,
    version_id: String,
    kind: String,
) -> CmdResult<Vec<InstanceFile>> {
    let cfg = state.config.lock().unwrap().clone();
    instance::list_files(&cfg, &version_id, &kind).map_err(err)
}

#[tauri::command]
pub fn instance_toggle_file(
    state: State<'_, AppState>,
    version_id: String,
    kind: String,
    name: String,
) -> CmdResult<()> {
    let cfg = state.config.lock().unwrap().clone();
    instance::toggle_file(&cfg, &version_id, &kind, &name).map_err(err)
}

#[tauri::command]
pub fn instance_delete_file(
    state: State<'_, AppState>,
    version_id: String,
    kind: String,
    name: String,
) -> CmdResult<()> {
    let cfg = state.config.lock().unwrap().clone();
    instance::delete_file(&cfg, &version_id, &kind, &name).map_err(err)
}

#[tauri::command]
pub fn instance_thumb(
    state: State<'_, AppState>,
    version_id: String,
    kind: String,
    name: String,
) -> CmdResult<Option<String>> {
    let cfg = state.config.lock().unwrap().clone();
    instance::thumb(&cfg, &version_id, &kind, &name).map_err(err)
}

/// 识别实例里已安装模组的来源（图标 + 项目链接），识别不到的不返回。
#[tauri::command]
pub async fn instance_mod_meta(
    state: State<'_, AppState>,
    version_id: String,
) -> CmdResult<Vec<modmeta::ModMeta>> {
    let cfg = state.config.lock().unwrap().clone();
    modmeta::lookup(&state.client, &cfg, &version_id)
        .await
        .map_err(err)
}

#[tauri::command]
pub fn instance_open_content(
    state: State<'_, AppState>,
    version_id: String,
    kind: String,
    name: String,
) -> CmdResult<()> {
    let cfg = state.config.lock().unwrap().clone();
    instance::open_content(&cfg, &version_id, &kind, &name).map_err(err)
}

#[tauri::command]
pub fn instance_import_files(
    state: State<'_, AppState>,
    version_id: String,
    kind: String,
    paths: Vec<String>,
) -> CmdResult<u32> {
    let cfg = state.config.lock().unwrap().clone();
    instance::import_paths(&cfg, &version_id, &kind, &paths).map_err(err)
}

#[tauri::command]
pub fn drop_inspect(paths: Vec<String>) -> Vec<crate::dropzone::DroppedItem> {
    crate::dropzone::inspect(&paths)
}

#[tauri::command]
pub async fn instance_import_mods(
    app: AppHandle,
    state: State<'_, AppState>,
    version_id: String,
    paths: Vec<String>,
) -> CmdResult<crate::moddeps::ModImportOutcome> {
    let cfg = state.config.lock().unwrap().clone();
    {
        let guard = state.progress.lock().unwrap();
        if let Some(p) = guard.as_ref() {
            if p.active.load(Ordering::Relaxed) {
                return Err("已有下载任务正在进行".into());
            }
        }
    }
    let progress = ProgressState::new("install");
    progress.set_message("正在导入模组…");
    *state.progress.lock().unwrap() = Some(progress.clone());

    let result = async {
        let imported = instance::import_paths(&cfg, &version_id, "mods", &paths)?;
        let mut report = crate::moddeps::DependencyReport::default();
        for path in &paths {
            let jar = std::path::Path::new(path);
            if !jar.is_file() || !path.to_lowercase().ends_with(".jar") {
                continue;
            }
            let part = crate::moddeps::install_missing(
                &app,
                &state.client,
                &cfg,
                &version_id,
                jar,
                progress.clone(),
            )
            .await;
            report.installed.extend(part.installed);
            report.failed.extend(part.failed);
            report.unresolved.extend(part.unresolved);
        }
        Ok::<_, anyhow::Error>(crate::moddeps::ModImportOutcome {
            imported,
            dependencies: report,
        })
    }
    .await;

    progress.active.store(false, Ordering::Relaxed);
    download::emit_now(&app, &progress);
    *state.progress.lock().unwrap() = None;

    result.map_err(err)
}

#[tauri::command]
pub fn instance_paths(
    state: State<'_, AppState>,
    version_id: String,
) -> InstancePaths {
    let cfg = state.config.lock().unwrap().clone();
    instance::paths(&cfg, &version_id)
}

#[tauri::command]
pub fn instance_worlds(state: State<'_, AppState>, version_id: String) -> Vec<WorldSummary> {
    let cfg = state.config.lock().unwrap().clone();
    instance::worlds(&cfg, &version_id)
}

#[tauri::command]
pub fn instance_world_seed(
    state: State<'_, AppState>,
    version_id: String,
    folder: String,
) -> CmdResult<WorldSeed> {
    let cfg = state.config.lock().unwrap().clone();
    instance::world_seed(&cfg, &version_id, &folder).map_err(err)
}

#[tauri::command]
pub async fn crash_analyze(
    state: State<'_, AppState>,
    version_id: String,
) -> CmdResult<crash::CrashReport> {
    let cfg = state.config.lock().unwrap().clone();
    // 会扫描 mods 目录、读大日志并检测 Java，放到阻塞线程池避免卡界面
    tokio::task::spawn_blocking(move || crash::analyze(&cfg, &version_id))
        .await
        .map_err(|error| format!("分析崩溃日志失败：{error}"))?
        .map_err(err)
}

/// 导出崩溃报告 ZIP（分析结果 + 相关日志 + 实例信息，凭据行已隐藏）。
#[tauri::command]
pub async fn crash_export(
    state: State<'_, AppState>,
    version_id: String,
) -> CmdResult<String> {
    let cfg = state.config.lock().unwrap().clone();
    tokio::task::spawn_blocking(move || crash::export(&cfg, &version_id))
        .await
        .map_err(|error| format!("导出崩溃报告失败：{error}"))?
        .map_err(err)
}

/// 进房前的 Mod / 版本一致性校验：房间广播了 mod 列表或版本时才对比。
#[derive(serde::Serialize)]
pub struct RoomModCheck {
    pub game_version: String,
    pub loader: String,
    /// 房间需要、当前实例没有的（文件名或 mod id）
    pub missing: Vec<String>,
    /// 当前实例有、房间没有列出的（仅房间给出文件名列表时统计）
    pub extra: Vec<String>,
    /// 需要提示给用户的版本/加载器不一致说明
    pub problems: Vec<String>,
}

#[tauri::command]
pub async fn room_mod_check(
    state: State<'_, AppState>,
    version_id: String,
    room_version: String,
    room_loader: String,
    room_mods: Vec<String>,
) -> CmdResult<RoomModCheck> {
    let cfg = state.config.lock().unwrap().clone();
    // 解压/读取几百个 jar 可能持续几秒，放到阻塞线程，避免卡住界面
    tokio::task::spawn_blocking(move || -> CmdResult<RoomModCheck> {
        let (game_version, loader) =
            install::instance_context(&cfg, &version_id).map_err(err)?;
        // 只读扫描，不顺手创建 mods 目录
        let dir = instance::game_dir(&cfg, &version_id).join("mods");

        let mut problems = Vec::new();
        if !room_version.trim().is_empty()
            && !room_version.trim().eq_ignore_ascii_case(&game_version)
        {
            problems.push(format!(
                "房间是 {}，当前实例是 {game_version}",
                room_version.trim()
            ));
        }
        if !room_loader.trim().is_empty() && !room_loader.trim().eq_ignore_ascii_case(&loader) {
            problems.push(format!(
                "房间使用 {}，当前实例是 {loader}",
                room_loader.trim()
            ));
        }

        let local_ids = moddeps::installed_ids(&dir);
        let mut local_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.ends_with(".jar") {
                    local_names.insert(name);
                }
            }
        }

        let mut missing = Vec::new();
        for item in &room_mods {
            let value = item.trim();
            if value.is_empty() {
                continue;
            }
            let lower = value.to_lowercase();
            let present = if lower.ends_with(".jar") {
                local_names.contains(&lower)
            } else {
                local_ids.contains(&lower)
            };
            if !present {
                missing.push(value.to_string());
            }
        }

        // 只有房间给出的全是文件名时才推断「本地多装了哪些」，
        // 否则（给的是 mod id）无法可靠反查每个 jar 的 id，容易误报
        let room_files: std::collections::HashSet<String> = room_mods
            .iter()
            .map(|item| item.trim().to_lowercase())
            .filter(|item| item.ends_with(".jar"))
            .collect();
        let extra = if !room_mods.is_empty() && room_files.len() == room_mods.len() {
            local_names
                .iter()
                .filter(|name| !room_files.contains(*name))
                .take(30)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        Ok(RoomModCheck {
            game_version,
            loader,
            missing,
            extra,
            problems,
        })
    })
    .await
    .map_err(|error| format!("进房校验失败：{error}"))?
}

#[tauri::command]
pub async fn hall_rooms(
    state: State<'_, AppState>,
) -> CmdResult<Vec<hall::HallRoom>> {
    let url = state.config.lock().unwrap().hall_url.clone();
    hall::fetch(&state.client, &url).await.map_err(err)
}

#[tauri::command]
pub async fn mod_updates(
    state: State<'_, AppState>,
    version_id: String,
) -> CmdResult<update::ModUpdateReport> {
    let cfg = state.config.lock().unwrap().clone();
    update::check(&state.client, &cfg, &version_id)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn mod_update_install(
    state: State<'_, AppState>,
    version_id: String,
    url: String,
    file_name: String,
    old_name: String,
    sha1: Option<String>,
) -> CmdResult<String> {
    let cfg = state.config.lock().unwrap().clone();
    update::install_update(
        &state.client,
        &cfg,
        &version_id,
        &url,
        &file_name,
        &old_name,
        sha1,
    )
    .await
    .map_err(err)
}

#[tauri::command]
pub fn modpack_inspect(path: String) -> CmdResult<modpack::PackPreview> {
    modpack::inspect(&path).map_err(err)
}

#[tauri::command]
pub async fn modpack_download(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    file_name: String,
) -> CmdResult<String> {
    {
        let guard = state.progress.lock().unwrap();
        if let Some(p) = guard.as_ref() {
            if p.active.load(Ordering::Relaxed) {
                return Err("已有下载任务正在进行".into());
            }
        }
    }
    let progress = ProgressState::new("install");
    progress.set_message(format!("准备下载 {file_name}…"));
    *state.progress.lock().unwrap() = Some(progress.clone());

    let result = modpack::download(&app, &state.client, &url, &file_name, progress.clone()).await;

    progress.active.store(false, Ordering::Relaxed);
    download::emit_now(&app, &progress);
    *state.progress.lock().unwrap() = None;

    result.map_err(err)
}

#[tauri::command]
pub fn modpack_temp_remove(path: String) -> CmdResult<()> {
    modpack::temp_remove(&path).map_err(err)
}

#[tauri::command]
pub async fn modpack_import(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    options: modpack::ImportOptions,
) -> CmdResult<modpack::ImportResult> {
    let cfg = state.config.lock().unwrap().clone();
    {
        let guard = state.progress.lock().unwrap();
        if let Some(progress) = guard.as_ref() {
            if progress.active.load(Ordering::Relaxed) {
                return Err("已有下载任务正在进行".into());
            }
        }
    }
    let progress = ProgressState::new("install");
    progress.set_message("准备导入整合包…");
    *state.progress.lock().unwrap() = Some(progress.clone());
    download::reset_failed_batch();

    let result = modpack::import(&app, &state.client, &cfg, &path, &options, progress.clone()).await;

    progress.active.store(false, Ordering::Relaxed);
    download::emit_now(&app, &progress);
    *state.progress.lock().unwrap() = None;

    match result {
        Ok(imported) => {
            let mut current = cfg;
            current.selected_version = Some(imported.instance_id.clone());
            let _ = config::save(&current);
            *state.config.lock().unwrap() = current;
            Ok(imported)
        }
        Err(error) => Err(err(error)),
    }
}

/// 只重跑上一批失败的文件，不必重新导入整个整合包
#[derive(serde::Serialize)]
pub struct RetryDownloadsView {
    failed: Vec<String>,
}

#[tauri::command]
pub async fn retry_failed_downloads(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<RetryDownloadsView> {
    {
        let guard = state.progress.lock().unwrap();
        if let Some(progress) = guard.as_ref() {
            if progress.active.load(Ordering::Relaxed) {
                return Err("已有下载任务正在进行".into());
            }
        }
    }
    let Some(batch) = download::take_failed_batch() else {
        return Ok(RetryDownloadsView { failed: Vec::new() });
    };
    let progress = ProgressState::new("repair");
    progress.set_message("准备重试失败文件…");
    *state.progress.lock().unwrap() = Some(progress.clone());

    let result = download::run_tasks_reporting(
        &app,
        &state.client,
        batch.tasks,
        batch.concurrency,
        batch.use_mirror,
        progress.clone(),
    )
    .await;

    progress.active.store(false, Ordering::Relaxed);
    download::emit_now(&app, &progress);
    *state.progress.lock().unwrap() = None;

    let outcome = result.map_err(err)?;
    Ok(RetryDownloadsView {
        failed: outcome.failed.into_iter().map(|(label, _)| label).collect(),
    })
}

/// 本地游玩画像：各版本累计时长、启动次数与最近几次游戏
#[tauri::command]
pub fn play_stats() -> crate::stats::PlayStats {
    crate::stats::summary()
}

#[tauri::command]
pub fn modpack_export_preview(
    state: State<'_, AppState>,
    version_id: String,
) -> CmdResult<modpack::ExportPreview> {
    let cfg = state.config.lock().unwrap().clone();
    modpack::export_preview(&cfg, &version_id).map_err(err)
}

#[tauri::command]
pub async fn modpack_export(
    app: AppHandle,
    state: State<'_, AppState>,
    version_id: String,
    options: modpack::ExportOptions,
) -> CmdResult<modpack::ExportResult> {
    let cfg = state.config.lock().unwrap().clone();
    {
        let guard = state.progress.lock().unwrap();
        if let Some(progress) = guard.as_ref() {
            if progress.active.load(Ordering::Relaxed) {
                return Err("已有任务正在进行".into());
            }
        }
    }
    let progress = ProgressState::new("install");
    progress.set_message("准备导出整合包…");
    *state.progress.lock().unwrap() = Some(progress.clone());

    let result = modpack::export(&app, &cfg, &version_id, &options, progress.clone()).await;

    progress.active.store(false, Ordering::Relaxed);
    download::emit_now(&app, &progress);
    *state.progress.lock().unwrap() = None;

    result.map_err(err)
}

#[derive(serde::Serialize)]
pub struct InstanceContext {
    pub game_version: String,
    pub loader: String,
}

#[tauri::command]
pub fn instance_context(
    state: State<'_, AppState>,
    version_id: String,
) -> CmdResult<InstanceContext> {
    let cfg = state.config.lock().unwrap().clone();
    let (game_version, loader) =
        install::instance_context(&cfg, &version_id).map_err(err)?;
    Ok(InstanceContext {
        game_version,
        loader,
    })
}

#[derive(serde::Serialize)]
pub struct InstanceCheck {
    pub scanned: u32,
    pub broken: Vec<modpack::BrokenFile>,
}

/// 实例文件自检：扫描 mods / 资源包 / 光影里打不开的压缩包。
/// 磁盘 IO 可能较慢，放阻塞线程池避免卡住 UI。
#[tauri::command]
pub async fn check_instance_files(
    state: State<'_, AppState>,
    version_id: String,
) -> CmdResult<InstanceCheck> {
    let cfg = state.config.lock().unwrap().clone();
    let root = instance::game_dir(&cfg, &version_id);
    let (scanned, broken) = tokio::task::spawn_blocking(move || modpack::scan_broken_files(&root))
        .await
        .map_err(err)?;
    Ok(InstanceCheck { scanned, broken })
}

/// 一键自愈：修清单、补缺失文件、重解本地库、禁用重复模组。
#[tauri::command]
pub async fn repair_instance(
    app: AppHandle,
    state: State<'_, AppState>,
    version_id: String,
) -> CmdResult<repair::RepairReport> {
    let cfg = state.config.lock().unwrap().clone();
    {
        let guard = state.progress.lock().unwrap();
        if let Some(p) = guard.as_ref() {
            if p.active.load(Ordering::Relaxed) {
                return Err("已有下载任务正在进行".into());
            }
        }
    }
    let progress = ProgressState::new("repair");
    progress.set_message(format!("正在检查 {version_id}…"));
    *state.progress.lock().unwrap() = Some(progress.clone());
    download::reset_failed_batch();

    let result = repair::run(&app, &state.client, &cfg, &version_id, progress.clone()).await;

    progress.active.store(false, Ordering::Relaxed);
    download::emit_now(&app, &progress);
    *state.progress.lock().unwrap() = None;

    result.map_err(err)
}

#[tauri::command]
pub async fn cf_search(
    state: State<'_, AppState>,
    class_id: i64,
    query: String,
    game_version: Option<String>,
    loader: Option<String>,
    category_id: Option<i64>,
    index: u32,
    sort_field: Option<u32>,
) -> CmdResult<curseforge::CfSearchResult> {
    curseforge::search(
        &state.client,
        class_id,
        &query,
        game_version.as_deref(),
        loader.as_deref(),
        category_id,
        index,
        20,
        sort_field.unwrap_or(2),
    )
    .await
    .map_err(err)
}

/// 类型筛选下拉：CurseForge 分类列表。
#[tauri::command]
pub async fn cf_categories(
    state: State<'_, AppState>,
    class_id: i64,
) -> CmdResult<Vec<curseforge::CfCategoryView>> {
    curseforge::categories(&state.client, class_id).await.map_err(err)
}

#[tauri::command]
pub async fn cf_game_versions(
    state: State<'_, AppState>,
) -> CmdResult<Vec<String>> {
    match curseforge::game_versions(&state.client).await {
        Ok(list) if !list.is_empty() => Ok(list),
        // CurseForge 的版本接口在镜像上缺失，退回官方版本清单保证下拉框可用
        _ => {
            let use_mirror = state.config.lock().unwrap().use_mirror();
            install::manifest_version_ids(&state.client, use_mirror)
                .await
                .map_err(err)
        }
    }
}

#[tauri::command]
pub async fn cf_files(
    state: State<'_, AppState>,
    mod_id: String,
    game_version: Option<String>,
    loader: Option<String>,
) -> CmdResult<Vec<curseforge::CfFileView>> {
    let id: i64 = mod_id
        .parse()
        .map_err(|_| "无效的 CurseForge 项目 ID".to_string())?;
    curseforge::files(
        &state.client,
        id,
        game_version.as_deref(),
        loader.as_deref(),
    )
    .await
    .map_err(err)
}

#[tauri::command]
pub async fn modrinth_search(
    state: State<'_, AppState>,
    kind: String,
    query: String,
    game_version: Option<String>,
    loader: Option<String>,
    category: Option<String>,
    index: u32,
    sort_field: Option<u32>,
) -> CmdResult<curseforge::CfSearchResult> {
    crate::modrinth::search(
        &state.client,
        &kind,
        &query,
        game_version.as_deref(),
        loader.as_deref(),
        category.as_deref(),
        index,
        20,
        sort_field.unwrap_or(2),
    )
    .await
    .map_err(err)
}

/// 类型筛选下拉：Modrinth 分类标签。
#[tauri::command]
pub async fn modrinth_categories(
    state: State<'_, AppState>,
    kind: String,
) -> CmdResult<Vec<String>> {
    crate::modrinth::categories(&state.client, &kind).await.map_err(err)
}

#[tauri::command]
pub async fn modrinth_files(
    state: State<'_, AppState>,
    project_id: String,
    game_version: Option<String>,
    loader: Option<String>,
) -> CmdResult<Vec<curseforge::CfFileView>> {
    crate::modrinth::files(
        &state.client,
        &project_id,
        game_version.as_deref(),
        loader.as_deref(),
    )
    .await
    .map_err(err)
}

#[tauri::command]
pub async fn content_projects(
    state: State<'_, AppState>,
    source: String,
    ids: Vec<String>,
) -> CmdResult<Vec<curseforge::CfProject>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    if source == "modrinth" {
        crate::modrinth::projects(&state.client, &ids)
            .await
            .map_err(err)
    } else {
        let parsed: Vec<i64> = ids.iter().filter_map(|id| id.parse().ok()).collect();
        Ok(curseforge::projects(&state.client, &parsed).await)
    }
}

/// 取整合包配套服务端包的下载直链，供「下载服务端」一键使用。
#[tauri::command]
pub async fn cf_file_link(
    state: State<'_, AppState>,
    project_id: String,
    file_id: String,
) -> CmdResult<curseforge::FileLink> {
    let prefer_mirror = state.config.lock().unwrap().download_source != "official";
    curseforge::file_link(&state.client, &project_id, &file_id, prefer_mirror)
        .await
        .map_err(err)
}

/// 取一批项目的中文名（CurseForge / Modrinth），失败时前端保持英文原名。
#[tauri::command]
pub async fn project_translations(
    state: State<'_, AppState>,
    source: String,
    ids: Vec<String>,
) -> CmdResult<std::collections::HashMap<String, translate::TranslatedProject>> {
    if ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let source = if source == "modrinth" {
        "modrinth"
    } else {
        "curseforge"
    };
    Ok(translate::batch(&state.client, source, &ids).await)
}

/// 从 MC 百科补全项目中文名；关闭开关时直接返回空。
#[tauri::command]
pub async fn mcmod_names(
    state: State<'_, AppState>,
    queries: Vec<mcmod::NameQuery>,
) -> CmdResult<std::collections::HashMap<String, mcmod::ChineseName>> {
    if queries.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    if !state.config.lock().unwrap().mcmod_names {
        return Ok(std::collections::HashMap::new());
    }
    Ok(mcmod::batch(&state.client, queries).await)
}

#[tauri::command]
pub async fn cf_install_file(
    app: AppHandle,
    state: State<'_, AppState>,
    version_id: String,
    kind: String,
    url: String,
    file_name: String,
) -> CmdResult<curseforge::InstallOutcome> {
    let cfg = state.config.lock().unwrap().clone();
    {
        let guard = state.progress.lock().unwrap();
        if let Some(p) = guard.as_ref() {
            if p.active.load(Ordering::Relaxed) {
                return Err("已有下载任务正在进行".into());
            }
        }
    }
    let progress = ProgressState::new("install");
    progress.set_message(format!("准备下载 {file_name}…"));
    *state.progress.lock().unwrap() = Some(progress.clone());

    let result = curseforge::install_file(
        &app,
        &state.client,
        &cfg,
        &version_id,
        &kind,
        &url,
        &file_name,
        progress.clone(),
    )
    .await;

    progress.active.store(false, Ordering::Relaxed);
    download::emit_now(&app, &progress);
    *state.progress.lock().unwrap() = None;

    result.map_err(err)
}

#[tauri::command]
pub fn default_skin() -> String {
    skins::default_skin()
}

#[tauri::command]
pub fn default_skins() -> Vec<skins::DefaultSkinView> {
    skins::default_skins()
}

#[tauri::command]
pub fn get_accounts(state: State<'_, AppState>) -> Vec<AccountView> {
    // 每次从磁盘重新读取，确保微软登录完成后能立即看到新账号
    let accounts = auth::load_accounts();
    *state.accounts.lock().unwrap() = accounts.clone();
    accounts.iter().map(AccountView::from).collect()
}

#[tauri::command]
pub fn create_offline_account(
    state: State<'_, AppState>,
    name: String,
) -> CmdResult<AccountView> {
    let mut account = auth::create_offline(&name).map_err(err)?;
    if let Ok((path, model)) = skins::install_default(&account.id, &account.id) {
        account.skin_path = Some(path.to_string_lossy().to_string());
        account.skin_model = model;
    }
    {
        let mut accounts = state.accounts.lock().unwrap();
        accounts.retain(|a| a.id != account.id);
        accounts.push(account.clone());
        auth::save_accounts(&accounts).map_err(err)?;
    }
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.selected_account = Some(account.id.clone());
        config::save(&cfg).map_err(err)?;
    }
    Ok(AccountView::from(&account))
}

/// 读取第三方皮肤站元数据（设置页填地址后校验用）。
#[tauri::command]
pub async fn authlib_meta(
    state: State<'_, AppState>,
    root: String,
) -> CmdResult<authlib::ServerMeta> {
    authlib::meta(&state.client, &root).await.map_err(err)
}

/// LittleSkin 等第三方皮肤站登录：账号密码换 accessToken，并保存为 authlib 账号。
#[tauri::command]
pub async fn authlib_login(
    state: State<'_, AppState>,
    root: String,
    username: String,
    password: String,
) -> CmdResult<AccountView> {
    let login = authlib::authenticate(&state.client, &root, &username, &password)
        .await
        .map_err(err)?;
    let account = Account {
        id: format!("authlib-{}", login.uuid),
        kind: "authlib".into(),
        name: login.name,
        uuid: login.uuid,
        access_token: login.access_token,
        refresh_token: login.client_token,
        expires_at: u64::MAX,
        // 不写本地默认皮肤，否则会盖住皮肤站上的角色皮肤
        skin_path: None,
        skin_model: "classic".into(),
        authlib_url: Some(root.trim().trim_end_matches('/').to_string()),
    };
    {
        let mut accounts = state.accounts.lock().unwrap();
        accounts.retain(|a| a.id != account.id);
        accounts.push(account.clone());
        auth::save_accounts(&accounts).map_err(err)?;
    }
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.selected_account = Some(account.id.clone());
        config::save(&cfg).map_err(err)?;
    }
    Ok(AccountView::from(&account))
}

#[tauri::command]
pub fn remove_account(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let mut accounts = state.accounts.lock().unwrap();
    accounts.retain(|a| a.id != id);
    auth::save_accounts(&accounts).map_err(err)?;
    let mut cfg = state.config.lock().unwrap();
    if cfg.selected_account.as_deref() == Some(id.as_str()) {
        cfg.selected_account = accounts.first().map(|a| a.id.clone());
        let _ = config::save(&cfg);
    }
    Ok(())
}

#[tauri::command]
pub fn select_account(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let mut cfg = state.config.lock().unwrap();
    cfg.selected_account = Some(id);
    config::save(&cfg).map_err(err)
}

#[derive(serde::Serialize)]
pub struct AccountSkinView {
    pub data_url: String,
    pub model: String,
    pub source: String,
}

fn find_account(account_id: &str) -> CmdResult<Account> {
    auth::load_accounts()
        .into_iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".into())
}

#[tauri::command]
pub async fn get_account_skin(
    state: State<'_, AppState>,
    account_id: String,
) -> CmdResult<AccountSkinView> {
    let account = find_account(&account_id)?;
    let skin = skins::load_skin(&state.client, &account).await;
    // 皮肤站/官方档案里带的模型（classic/slim）同步回账号，
    // 否则各处以登录时写死的 classic 渲染，纤细皮肤的手臂会整条取错图
    if skin.source != "local" && !skin.model.is_empty() && skin.model != account.skin_model {
        let mut accounts = auth::load_accounts();
        if let Some(item) = accounts.iter_mut().find(|item| item.id == account_id) {
            item.skin_model = skin.model.clone();
            if auth::save_accounts(&accounts).is_ok() {
                *state.accounts.lock().unwrap() = accounts;
            }
        }
    }
    Ok(AccountSkinView {
        data_url: skin.data_url,
        model: skin.model,
        source: skin.source,
    })
}

#[tauri::command]
pub fn save_account_skin(
    state: State<'_, AppState>,
    account_id: String,
    png_base64: String,
    model: String,
) -> CmdResult<AccountView> {
    let path = skins::save_local(&account_id, &png_base64).map_err(err)?;
    let mut accounts = auth::load_accounts();
    let account = accounts
        .iter_mut()
        .find(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    account.skin_path = Some(path.to_string_lossy().to_string());
    account.skin_model = model;
    let view = AccountView::from(&*account);
    auth::save_accounts(&accounts).map_err(err)?;
    *state.accounts.lock().unwrap() = accounts;
    Ok(view)
}

#[tauri::command]
pub fn clear_account_skin(
    state: State<'_, AppState>,
    account_id: String,
) -> CmdResult<AccountView> {
    let mut accounts = auth::load_accounts();
    let account = accounts
        .iter_mut()
        .find(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    if let Some(path) = account.skin_path.take() {
        let _ = std::fs::remove_file(path);
    }
    account.skin_model = "classic".into();
    let view = AccountView::from(&*account);
    auth::save_accounts(&accounts).map_err(err)?;
    *state.accounts.lock().unwrap() = accounts;
    Ok(view)
}

#[tauri::command]
pub fn export_account_skin(account_id: String, target_path: String) -> CmdResult<()> {
    let account = find_account(&account_id)?;
    skins::export_bytes(&account, &target_path).map_err(err)
}

#[tauri::command]
pub async fn upload_account_skin(
    state: State<'_, AppState>,
    account_id: String,
    png_base64: String,
    model: String,
) -> CmdResult<AccountView> {
    let mut account = find_account(&account_id)?;
    skins::upload_skin(&state.client, &account, &png_base64, &model)
        .await
        .map_err(err)?;
    let path = skins::save_local(&account_id, &png_base64).map_err(err)?;
    account.skin_path = Some(path.to_string_lossy().to_string());
    account.skin_model = model;
    let mut accounts = auth::load_accounts();
    if let Some(slot) = accounts.iter_mut().find(|a| a.id == account_id) {
        *slot = account.clone();
    }
    auth::save_accounts(&accounts).map_err(err)?;
    *state.accounts.lock().unwrap() = accounts;
    Ok(AccountView::from(&account))
}

#[tauri::command]
pub fn skin_library_list() -> Vec<skins::LibraryItemView> {
    skins::library_views()
}

#[tauri::command]
pub fn skin_library_import(
    name: String,
    model: String,
    path: String,
) -> CmdResult<skins::LibraryItemView> {
    let bytes = std::fs::read(path.trim()).map_err(err)?;
    let entry = skins::library_add(&name, &model, bytes, "import").map_err(err)?;
    skins::library_views()
        .into_iter()
        .find(|item| item.id == entry.id)
        .ok_or_else(|| "皮肤入库失败".into())
}

#[tauri::command]
pub fn skin_library_save(
    name: String,
    model: String,
    png_base64: String,
) -> CmdResult<skins::LibraryItemView> {
    let bytes = skins::decode_and_normalize(&png_base64).map_err(err)?;
    let entry = skins::library_add(&name, &model, bytes, "editor").map_err(err)?;
    skins::library_views()
        .into_iter()
        .find(|item| item.id == entry.id)
        .ok_or_else(|| "皮肤入库失败".into())
}

#[tauri::command]
pub async fn skin_library_from_account(
    state: State<'_, AppState>,
    name: String,
    account_id: String,
) -> CmdResult<skins::LibraryItemView> {
    let account = find_account(&account_id)?;
    let skin = skins::load_skin(&state.client, &account).await;
    let bytes = skins::decode_and_normalize(&skin.data_url).map_err(err)?;
    let entry = skins::library_add(&name, &skin.model, bytes, "account").map_err(err)?;
    skins::library_views()
        .into_iter()
        .find(|item| item.id == entry.id)
        .ok_or_else(|| "皮肤入库失败".into())
}

#[tauri::command]
pub async fn skin_library_from_player(
    state: State<'_, AppState>,
    name: String,
) -> CmdResult<skins::LibraryItemView> {
    let (player, bytes, model) = skins::player_skin(&state.client, &name)
        .await
        .map_err(err)?;
    let entry = skins::library_add(&player, &model, bytes, "player").map_err(err)?;
    skins::library_views()
        .into_iter()
        .find(|item| item.id == entry.id)
        .ok_or_else(|| "皮肤入库失败".into())
}

#[tauri::command]
pub fn skin_library_rename(id: String, name: String) -> CmdResult<()> {
    skins::library_rename(&id, &name).map_err(err)
}

#[tauri::command]
pub fn skin_library_remove(id: String) -> CmdResult<()> {
    skins::library_remove(&id).map_err(err)
}

#[tauri::command]
pub async fn skin_library_apply(
    state: State<'_, AppState>,
    id: String,
    account_id: String,
    upload: bool,
) -> CmdResult<AccountView> {
    let library = skins::load_library();
    let entry = library
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| "皮肤不存在".to_string())?;
    let bytes = skins::library_bytes(&id).map_err(err)?;
    let data_url = skins::to_data_url(&bytes, "image/png");
    let mut account = find_account(&account_id)?;
    if upload {
        skins::upload_skin(&state.client, &account, &data_url, &entry.model)
            .await
            .map_err(err)?;
    }
    let path = skins::save_local(&account_id, &data_url).map_err(err)?;
    account.skin_path = Some(path.to_string_lossy().to_string());
    account.skin_model = entry.model.clone();
    let mut accounts = auth::load_accounts();
    if let Some(slot) = accounts.iter_mut().find(|a| a.id == account_id) {
        *slot = account.clone();
    }
    auth::save_accounts(&accounts).map_err(err)?;
    *state.accounts.lock().unwrap() = accounts;
    Ok(AccountView::from(&account))
}

#[tauri::command]
pub async fn account_capes(
    state: State<'_, AppState>,
    account_id: String,
) -> CmdResult<Vec<skins::CapeView>> {
    let account = find_account(&account_id)?;
    skins::list_capes(&state.client, &account)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn cape_set_active(
    state: State<'_, AppState>,
    account_id: String,
    cape_id: Option<String>,
) -> CmdResult<()> {
    let account = find_account(&account_id)?;
    skins::set_cape(&state.client, &account, cape_id.as_deref())
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn ms_login_begin(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<DeviceCode> {
    let code = auth::begin_device_code(&state.client).await.map_err(err)?;
    let client = state.client.clone();
    let app_clone = app.clone();
    let code_clone = code.clone();
    tokio::spawn(async move {
        match auth::poll_device_login(app_clone.clone(), client, code_clone).await {
            Ok(account) => {
                // 由前端刷新账号列表
                let _ = app_clone.emit("ms-login-account", AccountView::from(&account));
            }
            Err(e) => {
                let _ = app_clone.emit(
                    "ms-login",
                    serde_json::json!({ "ok": false, "message": e.to_string() }),
                );
            }
        }
    });
    Ok(code)
}

#[tauri::command]
pub fn detect_java(state: State<'_, AppState>) -> Vec<JavaInfo> {
    let cfg = state.config.lock().unwrap().clone();
    java::detect(&cfg)
}

/// 手动添加 Java：接受 java.exe、javaw.exe 或 JDK/JRE 安装目录。
/// 校验通过后立即设为优先使用的运行时，自动检测不到的自定义安装也能用。
#[tauri::command]
pub fn add_java(state: State<'_, AppState>, path: String) -> CmdResult<JavaInfo> {
    let info = java::inspect_manual(&path).map_err(err)?;
    let mut cfg = state.config.lock().unwrap();
    cfg.java_path = Some(info.path.clone());
    config::save(&cfg).map_err(err)?;
    Ok(info)
}

#[tauri::command]
pub async fn java_runtime_majors(
    state: State<'_, AppState>,
    source: Option<String>,
) -> CmdResult<Vec<u32>> {
    let source = source.as_deref().unwrap_or("mojang");
    if source == "mojang" {
        let cfg = state.config.lock().unwrap().clone();
        return java::available_runtime_majors(&cfg).await.map_err(err);
    }
    Ok(java::vendor_majors(source))
}

#[tauri::command]
pub async fn install_java(
    app: AppHandle,
    state: State<'_, AppState>,
    major: u32,
    source: Option<String>,
) -> CmdResult<JavaInfo> {
    let cfg = state.config.lock().unwrap().clone();
    let progress = ProgressState::new("java");
    progress.set_message(format!("准备安装 Java {major}…"));
    *state.progress.lock().unwrap() = Some(progress.clone());

    let source = source.unwrap_or_else(|| "mojang".into());
    let result =
        java::install_runtime_vendor(&app, &cfg, &source, major, progress.clone()).await;
    progress.active.store(false, Ordering::Relaxed);
    download::emit_now(&app, &progress);
    *state.progress.lock().unwrap() = None;

    match result {
        Ok(info) => {
            let mut cfg = state.config.lock().unwrap();
            cfg.java_path = Some(info.path.clone());
            let _ = config::save(&cfg);
            Ok(info)
        }
        Err(e) => Err(err(e)),
    }
}

/// 删除启动器下载的 Java 运行时。
/// 只允许删运行时目录内的组件，系统/PATH/手动指定的 Java 一律拒绝，避免误删。
#[tauri::command]
pub async fn delete_java(state: State<'_, AppState>, path: String) -> CmdResult<bool> {
    let root = state.config.lock().unwrap().java_runtime_root();
    let exe = std::path::PathBuf::from(&path);
    let target = exe
        .parent()
        .and_then(|bin| bin.parent())
        .map(|dir| dir.to_path_buf())
        .ok_or_else(|| "无效的 Java 路径".to_string())?;

    let canonical_root = std::fs::canonicalize(&root).unwrap_or(root);
    let canonical_target = std::fs::canonicalize(&target)
        .map_err(|_| "Java 目录不存在或无法访问".to_string())?;
    if canonical_target == canonical_root || !canonical_target.starts_with(&canonical_root) {
        return Err("只能删除启动器下载的 Java 运行时".to_string());
    }

    tokio::fs::remove_dir_all(&canonical_target)
        .await
        .map_err(|e| format!("删除失败：{e}"))?;

    // 手动指定的正是被删的这个时回退到自动选择
    let mut cfg = state.config.lock().unwrap();
    if cfg.java_path.as_deref() == Some(path.as_str()) {
        cfg.java_path = None;
        let _ = config::save(&cfg);
    }
    Ok(true)
}

async fn resolve_account(app: &AppHandle, state: &AppState, cfg: &Config) -> Account {
    let accounts = state.accounts.lock().unwrap().clone();
    let selected = cfg
        .selected_account
        .as_ref()
        .and_then(|id| accounts.iter().find(|a| &a.id == id).cloned())
        .or_else(|| accounts.first().cloned())
        .unwrap_or_else(|| auth::create_offline("Steve").unwrap());

    let mut account: Account = selected;
    if account.kind == "authlib" {
        // 第三方皮肤站：先续期，再预热 authlib-injector
        if let Some(root) = account.authlib_url.clone() {
            match authlib::refresh(
                &state.client,
                &root,
                &account.access_token,
                &account.refresh_token,
            )
            .await
            {
                Ok(login) => {
                    account.name = login.name;
                    account.uuid = login.uuid;
                    account.access_token = login.access_token;
                    account.refresh_token = login.client_token;
                    let mut list = state.accounts.lock().unwrap();
                    if let Some(slot) = list.iter_mut().find(|a| a.id == account.id) {
                        *slot = account.clone();
                    }
                    let _ = auth::save_accounts(&list);
                }
                Err(e) => {
                    let _ = app.emit(
                        "game-log",
                        launch::LogLine {
                            stream: "system".into(),
                            line: format!(
                                "第三方账号续期失败：{e}。若进服被踢，请到账号页重新登录。"
                            ),
                        },
                    );
                }
            }
        }
        authlib::prepare(&state.client, &account).await;
        if authlib::injector_path().is_none() {
            let _ = app.emit(
                "game-log",
                launch::LogLine {
                    stream: "system".into(),
                    line: "未能下载 authlib-injector，第三方登录可能无法生效".into(),
                },
            );
        }
        return account;
    }
    if account.kind == "microsoft" {
        match auth::refresh(&state.client, &account).await {
            Ok(refreshed) => {
                account = refreshed;
                let mut list = state.accounts.lock().unwrap();
                if let Some(slot) = list.iter_mut().find(|a| a.id == account.id) {
                    *slot = account.clone();
                } else {
                    list.push(account.clone());
                }
                let _ = auth::save_accounts(&list);
            }
            Err(e) => {
                let _ = app.emit(
                    "game-log",
                    launch::LogLine {
                        stream: "system".into(),
                        line: format!("账号刷新失败：{e}"),
                    },
                );
            }
        }
    }
    account
}

#[tauri::command]
pub async fn launch_version(
    app: AppHandle,
    state: State<'_, AppState>,
    version_id: String,
) -> CmdResult<u32> {
    // 版本级启动设置覆盖全局配置
    let cfg = instance::apply(&state.config.lock().unwrap().clone(), &version_id);
    let account = resolve_account(&app, &state, &cfg).await;
    let pid = launch::spawn(&app, &cfg, &version_id, &account, None).map_err(err)?;
    *state.child_pid.lock().unwrap() = Some(pid);
    Ok(pid)
}

#[tauri::command]
pub async fn launch_server(
    app: AppHandle,
    state: State<'_, AppState>,
    version_id: String,
    address: String,
) -> CmdResult<u32> {
    let target = lan::parse_address(&address).map_err(err)?;
    let cfg = instance::apply(&state.config.lock().unwrap().clone(), &version_id);
    let account = resolve_account(&app, &state, &cfg).await;
    let pid = launch::spawn(&app, &cfg, &version_id, &account, Some(&target)).map_err(err)?;
    *state.child_pid.lock().unwrap() = Some(pid);
    Ok(pid)
}

#[tauri::command]
pub fn lan_rooms() -> lan::LanView {
    lan::rooms()
}

#[tauri::command]
pub fn server_versions(state: State<'_, AppState>) -> Vec<server::ServerVersionOption> {
    let cfg = state.config.lock().unwrap().clone();
    server::versions(&cfg)
}

#[tauri::command]
pub fn server_status() -> Option<server::ServerSnapshot> {
    server::status()
}

#[tauri::command]
pub async fn server_pack_import(
    state: State<'_, AppState>,
    path: String,
    name: String,
    slug: String,
    game_version: Option<String>,
    loader: Option<String>,
) -> CmdResult<server::ServerPack> {
    let cfg = state.config.lock().unwrap().clone();
    // 解压可能是几百 MB，放到阻塞线程池，避免卡住界面
    tokio::task::spawn_blocking(move || {
        server::import_pack(
            &cfg,
            &path,
            &name,
            &slug,
            game_version.as_deref(),
            loader.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("导入服务端包失败：{e}"))?
    .map_err(err)
}

#[tauri::command]
pub async fn server_pack_remove(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let cfg = state.config.lock().unwrap().clone();
    // 服务端包可能有几百 MB，删除放到阻塞线程池，避免卡住界面
    tokio::task::spawn_blocking(move || server::remove_pack(&cfg, &id))
        .await
        .map_err(|e| format!("删除服务端包失败：{e}"))?
        .map_err(err)
}

#[tauri::command]
pub async fn server_start(
    state: State<'_, AppState>,
    version_id: String,
    options: server::ServerOptions,
) -> CmdResult<server::ServerSnapshot> {
    let cfg = instance::apply(&state.config.lock().unwrap().clone(), &version_id);
    // 服务端整合包按解压目录直接启动
    if server::load_packs()
        .iter()
        .any(|pack| pack.id == version_id)
    {
        return server::start_pack(&state.client, &cfg, &version_id, &options)
            .await
            .map_err(err);
    }
    server::start(&state.client, &cfg, &version_id, &options)
        .await
        .map_err(err)
}

#[tauri::command]
pub fn server_command(command: String) -> CmdResult<()> {
    server::send(&command).map_err(err)
}

#[tauri::command]
pub fn server_stop() -> CmdResult<()> {
    server::stop().map_err(err)
}

#[tauri::command]
pub async fn terracotta_prepare(state: State<'_, AppState>) -> CmdResult<terracotta::TcStatus> {
    terracotta::prepare(&state.client).await.map_err(err)
}

#[tauri::command]
pub async fn terracotta_status() -> terracotta::TcStatus {
    terracotta::status().await
}

#[tauri::command]
pub async fn terracotta_host(
    state: State<'_, AppState>,
    room: Option<String>,
    player: Option<String>,
) -> CmdResult<()> {
    let nodes = terracotta::parse_nodes(&state.config.lock().unwrap().terracotta_nodes);
    terracotta::host(room, player, &nodes).await.map_err(err)
}

#[tauri::command]
pub async fn terracotta_join(
    state: State<'_, AppState>,
    room: String,
    player: Option<String>,
) -> CmdResult<()> {
    let nodes = terracotta::parse_nodes(&state.config.lock().unwrap().terracotta_nodes);
    terracotta::join(room, player, &nodes).await.map_err(err)
}

#[tauri::command]
pub async fn terracotta_reset() -> CmdResult<()> {
    terracotta::reset().await.map_err(err)
}

#[tauri::command]
pub async fn terracotta_shutdown() -> CmdResult<()> {
    terracotta::shutdown().await.map_err(err)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
}

#[tauri::command]
pub fn remember_server(state: State<'_, AppState>, address: String) -> CmdResult<Config> {
    let target = lan::parse_address(&address).map_err(err)?;
    let normalized = target.address();
    let mut cfg = state.config.lock().unwrap();
    cfg.recent_servers
        .retain(|entry| entry.address != normalized);
    cfg.recent_servers.insert(
        0,
        config::ServerEntry {
            address: normalized,
            last_used: now_secs(),
        },
    );
    cfg.recent_servers.truncate(12);
    config::save(&cfg).map_err(err)?;
    Ok(cfg.clone())
}

#[tauri::command]
pub fn forget_server(state: State<'_, AppState>, address: String) -> CmdResult<Config> {
    let mut cfg = state.config.lock().unwrap();
    cfg.recent_servers.retain(|entry| entry.address != address);
    config::save(&cfg).map_err(err)?;
    Ok(cfg.clone())
}

#[tauri::command]
pub async fn server_ping(address: String) -> CmdResult<ServerPingResult> {
    let target = lan::parse_address(&address).map_err(err)?;
    ping::ping(&target, 5000).await.map_err(err)
}

#[tauri::command]
pub fn kill_game(state: State<'_, AppState>) -> CmdResult<()> {
    if let Some(pid) = *state.child_pid.lock().unwrap() {
        launch::kill(pid).map_err(err)?;
    }
    Ok(())
}

#[derive(serde::Serialize)]
pub struct BackgroundInfo {
    pub path: String,
    pub data_url: String,
}

fn image_mime(path: &std::path::Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

fn encode_background(path: &std::path::Path) -> CmdResult<String> {
    let meta = std::fs::metadata(path).map_err(err)?;
    if meta.len() > 24 * 1024 * 1024 {
        return Err("背景图片过大（上限 24 MB）".into());
    }
    let mime = image_mime(path).ok_or_else(|| "仅支持 PNG / JPG / WEBP / BMP / GIF 图片".to_string())?;
    let bytes = std::fs::read(path).map_err(err)?;
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:{mime};base64,{encoded}"))
}

#[tauri::command]
pub fn import_background(path: String) -> CmdResult<BackgroundInfo> {
    let source = std::path::PathBuf::from(&path);
    encode_background(&source)?;
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_lowercase();
    let dir = crate::config::data_dir();
    std::fs::create_dir_all(&dir).map_err(err)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let target = dir.join(format!("background-{stamp}.{ext}"));
    std::fs::copy(&source, &target).map_err(err)?;
    let data_url = encode_background(&target)?;
    Ok(BackgroundInfo {
        path: target.to_string_lossy().to_string(),
        data_url,
    })
}

#[tauri::command]
pub fn background_data_url(path: String) -> CmdResult<String> {
    encode_background(std::path::Path::new(&path))
}

/// 从文件夹随机挑一张背景图，返回可直接编码的路径（PCL 式：每次启动换一张）。
#[tauri::command]
pub fn pick_background_random(dir: String) -> CmdResult<Option<String>> {
    let entries = std::fs::read_dir(&dir)
        .map_err(|e| format!("无法读取背景文件夹：{e}"))?;
    let images: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            // 超过 24 MB 的图片编码会失败，直接排除，避免随机到它就变成空白背景
            path.is_file()
                && image_mime(path).is_some()
                && std::fs::metadata(path)
                    .map(|meta| meta.len() <= 24 * 1024 * 1024)
                    .unwrap_or(false)
        })
        .collect();
    if images.is_empty() {
        return Ok(None);
    }
    // 用当前纳秒对长度取模选图：足够随机且不引入额外依赖
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize ^ d.as_secs() as usize)
        .unwrap_or(0);
    Ok(Some(images[nanos % images.len()].to_string_lossy().to_string()))
}
