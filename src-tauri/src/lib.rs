mod auth;
mod authlib;
mod appupdate;
mod commands;
mod config;
mod crash;
mod curseforge;
mod download;

mod dropzone;
mod forge_native;
mod hall;
mod http;
mod install;
mod instance;
mod java;
mod lan;
mod launch;
mod mirror;
mod mcmod;
mod modpack;
mod modmeta;
mod moddeps;
mod modrinth;
mod models;
mod news;
mod pclimport;
mod ping;
mod server;
mod skins;
mod system;
mod terracotta;
mod translate;
mod update;
mod vibrancy;

use std::sync::Mutex;
use tauri::Manager;

pub struct AppState {
    pub config: Mutex<config::Config>,
    pub client: reqwest::Client,
    pub progress: Mutex<Option<std::sync::Arc<download::ProgressState>>>,
    pub manifest: Mutex<Option<models::VersionManifest>>,
    pub child_pid: Mutex<Option<u32>>,
    pub accounts: Mutex<Vec<auth::Account>>,
}

pub fn run() {
    // 清理上次便携更新的备份；旧进程退出后才能删掉，失败留给下次
    if config::is_portable() {
        if let Ok(current) = std::env::current_exe() {
            let _ = std::fs::remove_file(current.with_extension("exe.old"));
        }
    }
    let cfg = config::load();
    // 恢复上次的下载限速设置
    download::set_speed_limit(cfg.speed_limit as u64 * 1024 * 1024);
    // 「仅镜像」时强制镜像优先，其余走按主机的智能排序
    download::set_force_mirror(cfg.download_source == "mirror");
    let accounts = auth::load_accounts();
    lan::start();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let window = app.get_webview_window("main");
            // 按配置恢复毛玻璃；失败静默跳过，设置页开关可以重试
            if cfg.window_vibrancy {
                if let Some(window) = &window {
                    if let Err(e) = vibrancy::enable(window) {
                        eprintln!("vibrancy: {e}");
                    }
                }
            }
            // 记住窗口大小：启动时恢复，关闭时保存。用逻辑像素，随 DPI 缩放一致
            if let Some(window) = window {
                if cfg.remember_window && cfg.launcher_window_w >= 400 && cfg.launcher_window_h >= 300 {
                    let _ = window.set_size(tauri::LogicalSize::new(
                        cfg.launcher_window_w,
                        cfg.launcher_window_h,
                    ));
                }
                let saver = window.clone();
                window.on_window_event(move |event| {
                    if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                        // 最大化/全屏时尺寸不代表用户想要的窗口大小，保留上一次的值
                        if saver.is_maximized().unwrap_or(false) || saver.is_fullscreen().unwrap_or(false) {
                            return;
                        }
                        let mut cfg = config::load();
                        if cfg.remember_window {
                            if let Ok(size) = saver.inner_size() {
                                let scale = saver.scale_factor().unwrap_or(1.0);
                                cfg.launcher_window_w = (size.width as f64 / scale).round() as u32;
                                cfg.launcher_window_h = (size.height as f64 / scale).round() as u32;
                                let _ = config::save(&cfg);
                            }
                        }
                    }
                });
            }
            Ok(())
        })
        .manage(AppState {
            config: Mutex::new(cfg),
            client: http::client(),
            progress: Mutex::new(None),
            manifest: Mutex::new(None),
            child_pid: Mutex::new(None),
            accounts: Mutex::new(accounts),
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::system_memory,
            commands::news_feed,
            commands::news_article,
            commands::save_config,
            commands::export_settings,
            commands::import_settings,
            commands::app_version,
            commands::app_portable,
            commands::pcl_import_preview,
            commands::pcl_import_apply,
            commands::update_check,
            commands::update_download,
            commands::update_install,
            commands::portable_update_install,
            commands::open_folder,
            commands::cf_categories,
            commands::modrinth_categories,
            commands::reveal_file,
            commands::game_dirs,
            commands::game_dir_version_counts,
            commands::add_game_dir,
            commands::switch_game_dir,
            commands::remove_game_dir,
            commands::fetch_versions,
            commands::installed_versions,
            commands::install_version,
            commands::cancel_install,
            commands::loader_options,
            commands::loader_versions,
            commands::install_loader,
            commands::delete_version,
            commands::instance_info,
            commands::instance_metas,
            commands::instance_save_meta,
            commands::instance_set_icon,
            commands::instance_set_icon_image,
            commands::instance_icon_image,
            commands::instance_files,
            commands::instance_toggle_file,
            commands::instance_delete_file,
            commands::instance_thumb,
            commands::instance_mod_meta,
            commands::instance_open_content,
            commands::instance_import_files,
            commands::drop_inspect,
            commands::instance_import_mods,
            commands::instance_paths,
            commands::instance_worlds,
            commands::instance_world_seed,
            commands::modpack_inspect,
            commands::modpack_download,
            commands::modpack_temp_remove,
            commands::modpack_import,
            commands::modpack_export_preview,
            commands::modpack_export,
            commands::instance_context,
            commands::check_instance_files,
            commands::crash_analyze,
            commands::crash_export,
            commands::mod_updates,
            commands::mod_update_install,
            commands::cf_search,
            commands::cf_game_versions,
            commands::cf_files,
            commands::modrinth_search,
            commands::modrinth_files,
            commands::content_projects,
            commands::project_translations,
            commands::mcmod_names,
            commands::cf_file_link,
            commands::cf_install_file,
            commands::default_skin,
            commands::default_skins,
            commands::get_accounts,
            commands::create_offline_account,
            commands::remove_account,
            commands::select_account,
            commands::get_account_skin,
            commands::save_account_skin,
            commands::clear_account_skin,
            commands::export_account_skin,
            commands::upload_account_skin,
            commands::skin_library_list,
            commands::skin_library_import,
            commands::skin_library_save,
            commands::skin_library_from_account,
            commands::skin_library_from_player,
            commands::skin_library_rename,
            commands::skin_library_remove,
            commands::skin_library_apply,
            commands::account_capes,
            commands::cape_set_active,
            commands::ms_login_begin,
            commands::authlib_meta,
            commands::authlib_login,
            commands::detect_java,
            commands::install_java,
            commands::delete_java,
            commands::launch_version,
            commands::launch_server,
            commands::lan_rooms,
            commands::hall_rooms,
            commands::server_versions,
            commands::server_status,
            commands::server_start,
            commands::server_pack_import,
            commands::server_pack_remove,
            commands::server_command,
            commands::server_stop,
            commands::terracotta_prepare,
            commands::terracotta_status,
            commands::terracotta_host,
            commands::terracotta_join,
            commands::terracotta_reset,
            commands::terracotta_shutdown,
            commands::remember_server,
            commands::forget_server,
            commands::server_ping,
            commands::kill_game,
            commands::import_background,
            commands::background_data_url,
            commands::pick_background_random,
        ])
        .build(tauri::generate_context!())
        .expect("构建启动器失败");

    app.run(|_app, event| {
        if let tauri::RunEvent::Exit = event {
            terracotta::shutdown_blocking();
        }
    });
}
